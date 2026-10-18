use crate::{
    Auth, Credentials, Domain, FeishuClient, Transport,
    authorization::standard_capability_gaps,
    authorization_url,
    console::{self, DeveloperConsole, Login},
    contains_standard_capabilities,
};
use lankide_provider::{
    AgentOnboardingContext, ProviderError, ProviderOnboarding, ProviderOnboardingStatus,
    SecretValue,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn external(operation: &'static str) -> ProviderError {
    ProviderError::External {
        operation,
        retryable: true,
    }
}

fn onboarding(step: &str, status: ProviderOnboardingStatus, payload: Value) -> ProviderOnboarding {
    ProviderOnboarding {
        step: step.into(),
        status,
        verification_uri: payload
            .get("verification_uri")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        poll_interval_seconds: (matches!(
            status,
            ProviderOnboardingStatus::ActionRequired | ProviderOnboardingStatus::Verifying
        ) && matches!(
            payload.get("phase").and_then(Value::as_str),
            Some("permissions" | "console_login")
        ))
        .then(|| {
            payload
                .get("interval_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .clamp(1, 30) as u32
        }),
        opaque_payload: payload,
        error_code: None,
        error_message: None,
        agent_config_updates: Value::Object(Default::default()),
    }
}

fn pending_state_key(context: &AgentOnboardingContext) -> String {
    format!("__agent_onboarding/{}", context.agent_id.as_str())
}

pub(crate) async fn agent_onboarding_with_transport(
    context: AgentOnboardingContext,
    action: String,
    payload: Value,
    transport: Arc<dyn Transport>,
    flow: &mut console::LoginFlow,
) -> Result<ProviderOnboarding, ProviderError> {
    if context
        .provider_agent_config
        .get("mode")
        .and_then(Value::as_str)
        != Some("new")
    {
        return existing_agent_onboarding(context, action, payload, transport).await;
    }
    if action == "get" || action == "start" {
        if let Some(saved) = context.storage().get(&pending_state_key(&context)).await?
            && let Ok(saved_payload) = serde_json::from_slice::<Value>(&saved)
            && saved_payload.get("phase").and_then(Value::as_str) == Some("permissions")
        {
            if action == "start" {
                return poll_permissions(context, saved_payload, transport).await;
            }
            return Ok(onboarding(
                "permissions",
                ProviderOnboardingStatus::ActionRequired,
                saved_payload,
            ));
        }
        if action == "get" {
            return Ok(onboarding(
                "create_app",
                ProviderOnboardingStatus::ActionRequired,
                json!({}),
            ));
        }
    }
    if payload.get("phase").and_then(Value::as_str) == Some("permissions") {
        return poll_permissions(context, payload, transport).await;
    }
    if action == "start" || payload["phase"] == "console_login" {
        if !matches!(configured_domain(&context)?, Domain::Feishu) {
            return Err(ProviderError::InvalidInput { field: "domain" });
        }
        return console_onboarding(&context, &action, transport, flow).await;
    }
    Err(ProviderError::InvalidInput { field: "action" })
}

/// Creates the Agent App through the developer console, asking the user to
/// sign in with a fresh, one-use QR session.
async fn console_onboarding(
    context: &AgentOnboardingContext,
    action: &str,
    transport: Arc<dyn Transport>,
    flow: &mut console::LoginFlow,
) -> Result<ProviderOnboarding, ProviderError> {
    let login = if let Some(session) = flow.take() {
        Ok(Login::Ready(session))
    } else {
        flow.advance(
            transport.as_ref(),
            if action == "start" { "start" } else { "poll" },
        )
        .await
    };
    let login = match login {
        Ok(login) => login,
        Err(error) => return Err(error),
    };
    let session = match login {
        Login::Ready(session) => session,
        Login::Waiting { qr_code, scanned } => {
            return Ok(onboarding(
                "console_login",
                if scanned {
                    ProviderOnboardingStatus::Verifying
                } else {
                    ProviderOnboardingStatus::ActionRequired
                },
                json!({
                    "phase": "console_login",
                    "qr_code": qr_code,
                    "interval_seconds": console::LOGIN_POLL_INTERVAL_SECONDS,
                }),
            ));
        }
    };
    let mut developer_console = DeveloperConsole::new(transport.clone(), session);
    let tenant_id = match developer_console.tenant_id().await {
        Ok(id) => id,
        Err(e) => return Err(e.provider_error(false)),
    };
    let created = developer_console
        .create_agent_app(&context.name)
        .await
        .map_err(|e| e.provider_error(false))?;
    drop(developer_console);
    async {
        context
            .write_agent_secret("app_secret", SecretValue::new(created.app_secret()))
            .await?;
        let permissions_payload = json!({
            "phase": "permissions",
            "app_id": created.app_id,
            "creator_open_id": "",
            "tenant_id": tenant_id,
            "domain": "feishu",
            "interval_seconds": 2,
        });
        context
            .storage()
            .put(
                &pending_state_key(context),
                serde_json::to_vec(&permissions_payload)
                    .map_err(|_| external("feishu_authorization_state"))?,
            )
            .await?;
        Ok(onboarding(
            "permissions",
            ProviderOnboardingStatus::Verifying,
            permissions_payload,
        ))
    }
    .await
}

async fn poll_permissions(
    context: AgentOnboardingContext,
    payload: Value,
    transport: Arc<dyn Transport>,
) -> Result<ProviderOnboarding, ProviderError> {
    let app_id = payload
        .get("app_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ProviderError::InvalidInput { field: "app_id" })?
        .to_owned();
    let secret_name = format!("agent:{}:app_secret", context.agent_id.as_str());
    let app_secret = context
        .workspace_secrets()
        .read(&secret_name)
        .await?
        .expose_secret()
        .to_owned();
    let domain = configured_domain(&context)?;
    let inspected_app_id = app_id.clone();
    let inspected_transport = transport.clone();
    let inspected_secret = app_secret.clone();
    let inspected_domain = domain.clone();
    let result = (async move {
        let client = FeishuClient::new(
            inspected_transport,
            Credentials::app(inspected_app_id, inspected_secret),
        )?
        .with_domain(inspected_domain);
        let capabilities = client.published_capabilities(Auth::Tenant).await?;
        if !contains_standard_capabilities(&capabilities.tenant_scopes, &capabilities.events) {
            return Ok::<_, crate::ClientError>(None);
        }
        let bot = client.bot_info(Auth::Tenant).await?;
        Ok(Some(bot.open_id))
    })
    .await;
    match result {
        Ok(Some(bot_open_id)) => {
            let client =
                FeishuClient::new(transport.clone(), Credentials::app(&app_id, &app_secret))
                    .map_err(|e| e.provider_error(false))?
                    .with_domain(domain.clone());
            let union = client
                .application_creator("union_id")
                .await
                .map_err(|e| e.provider_error(false))?;
            let creator_open_id = client
                .application_creator("open_id")
                .await
                .map_err(|e| e.provider_error(false))?;
            let mut person = context.storage().get_or_create_person(&union).await?;
            let mut identity = person.provider_state.clone();
            identity["external_id_type"] = json!("union_id");
            identity["open_ids_by_app"][&app_id] = json!(creator_open_id);
            person = context
                .storage()
                .compare_and_set_person_state(&person.person_id, person.state_revision, identity)
                .await?;
            let profile = client.get_user(&creator_open_id, Auth::Tenant).await.ok();
            let name = profile
                .as_ref()
                .map(|p| p.name.as_str())
                .filter(|v| !v.is_empty());
            let mut avatar = None;
            if let Some(url) = profile
                .as_ref()
                .map(|p| p.avatar.avatar_origin.as_str())
                .filter(|v| !v.is_empty())
            {
                if let Ok(response) = transport
                    .execute(crate::http::Request::new(crate::http::Method::Get, url))
                    .await
                {
                    if (200..300).contains(&response.status) {
                        avatar = context
                            .save_resource(lankide_provider::ResourceUpload {
                                file_name: "member.jpg".into(),
                                media_type: response
                                    .headers
                                    .iter()
                                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or_else(|| "image/jpeg".into()),
                                bytes: response.body,
                            })
                            .await
                            .ok();
                    }
                }
            }
            let tenant = payload["tenant_id"]
                .as_str()
                .filter(|v| !v.is_empty())
                .ok_or(ProviderError::InvalidInput {
                    field: "team_enterprise",
                })?;
            context
                .storage()
                .join_human_team(&person.person_id, Some(tenant), name, avatar.as_ref())
                .await?;
            let creator_person_id = Some(person.person_id.to_string());
            if !creator_open_id.is_empty() {
                crate::agent_tasks::initialize_tasklist(
                    &context,
                    transport.clone(),
                    &app_id,
                    &app_secret,
                    domain,
                )
                .await?;
            }
            context
                .storage()
                .put(&pending_state_key(&context), b"{}".to_vec())
                .await?;
            let mut value = onboarding("ready", ProviderOnboardingStatus::Ready, json!({}));
            value.agent_config_updates = json!({
                "mode": "existing",
                "app_id": app_id,
                "provider_handle": app_id,
                "creator_open_id": creator_open_id,
                "bot_open_id": bot_open_id,
                "creator_person_id": creator_person_id,
                "domain": "feishu",
            });
            Ok(value)
        }
        Ok(None) => Ok(onboarding(
            "permissions",
            ProviderOnboardingStatus::Verifying,
            payload,
        )),
        Err(error) if error.is_retryable() => Ok(onboarding(
            "permissions",
            ProviderOnboardingStatus::Verifying,
            payload,
        )),
        Err(_) => Ok(ProviderOnboarding {
            error_code: Some("inspection_failed".into()),
            error_message: Some("无法读取飞书应用状态。".into()),
            ..onboarding("permissions", ProviderOnboardingStatus::Error, payload)
        }),
    }
}

async fn existing_agent_onboarding(
    context: AgentOnboardingContext,
    action: String,
    payload: Value,
    transport: Arc<dyn Transport>,
) -> Result<ProviderOnboarding, ProviderError> {
    let app_id = config_text(&context.provider_agent_config, "app_id")
        .or_else(|| config_text(&context.provider_agent_config, "provider_handle"))
        .ok_or(ProviderError::InvalidInput { field: "app_id" })?;
    let domain = configured_domain(&context)?;
    let secret_name = format!("agent:{}:app_secret", context.agent_id.as_str());
    let app_secret = context
        .workspace_secrets()
        .read(&secret_name)
        .await?
        .expose_secret()
        .trim()
        .to_owned();
    if app_secret.is_empty() {
        return Err(ProviderError::InvalidInput {
            field: "app_secret",
        });
    }

    let inspected_app_id = app_id.clone();
    let inspected_domain = domain.clone();
    let inspected_transport = transport.clone();
    let inspected_app_secret = app_secret.clone();
    let capabilities = (async move {
        FeishuClient::new(
            inspected_transport,
            Credentials::app(inspected_app_id, inspected_app_secret),
        )?
        .with_domain(inspected_domain)
        .published_capabilities(Auth::Tenant)
        .await
    })
    .await
    .map_err(|_| external("feishu_authorization_inspection"))?;
    let (missing_scopes, missing_events) =
        standard_capability_gaps(&capabilities.tenant_scopes, &capabilities.events);
    if missing_scopes.is_empty() && missing_events.is_empty() {
        return Ok(ProviderOnboarding::ready(json!({})));
    }

    let submitted = payload
        .get("submitted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let manual_confirmation = payload
        .get("manual_confirmation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut state = json!({
        "phase": "permissions",
        "flow": "authorization_update",
        "app_id": app_id.clone(),
        "missing_scopes": missing_scopes,
        "missing_events": missing_events,
        "submitted": submitted,
        "interval_seconds": 5,
    });
    if manual_confirmation {
        state["verification_uri"] = Value::String(
            authorization_url(&domain, &app_id)
                .map_err(|_| external("feishu_authorization_link"))?,
        );
        state["manual_confirmation"] = Value::Bool(true);
    }
    match action.as_str() {
        "get" => Ok(onboarding(
            "authorization_update",
            ProviderOnboardingStatus::ActionRequired,
            state,
        )),
        "start" => {
            let missing_scopes = state["missing_scopes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let missing_events = state["missing_events"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let update_app_id = app_id.clone();
            let update_domain = domain.clone();
            let update = (async move {
                FeishuClient::new(transport, Credentials::app(update_app_id, app_secret))?
                    .with_domain(update_domain)
                    .update_authorization(&missing_scopes, &missing_events, Auth::Tenant)
                    .await
            })
            .await;
            if !matches!(update, Ok(_)) {
                let link = authorization_url(&domain, &app_id)
                    .map_err(|_| external("feishu_authorization_link"))?;
                state["verification_uri"] = Value::String(link);
                state["manual_confirmation"] = Value::Bool(true);
                return Ok(onboarding(
                    "authorization_update",
                    ProviderOnboardingStatus::ActionRequired,
                    state,
                ));
            }
            let mut state = state;
            state["submitted"] = Value::Bool(true);
            Ok(onboarding(
                "authorization_update",
                ProviderOnboardingStatus::Verifying,
                state,
            ))
        }
        "poll" if submitted => Ok(onboarding(
            "authorization_update",
            ProviderOnboardingStatus::Verifying,
            state,
        )),
        "poll" => Ok(onboarding(
            "authorization_update",
            ProviderOnboardingStatus::ActionRequired,
            state,
        )),
        _ => Err(ProviderError::InvalidInput { field: "action" }),
    }
}

fn config_text(config: &Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn configured_domain(context: &AgentOnboardingContext) -> Result<Domain, ProviderError> {
    crate::adapter::configured_domain(&context.provider_agent_config, &context.workspace_config)
}
