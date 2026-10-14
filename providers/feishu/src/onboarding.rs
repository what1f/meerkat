use crate::{
    Auth, Credentials, Domain, FeishuClient, FeishuRegistrar, RegistrationPoll, ReqwestTransport,
    Transport, authorization::standard_capability_gaps, authorization_url,
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
        ) && (payload
            .get("device_code")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
            || payload.get("phase").and_then(Value::as_str) == Some("permissions")))
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

pub async fn agent_onboarding(
    context: AgentOnboardingContext,
    action: String,
    payload: Value,
) -> Result<ProviderOnboarding, ProviderError> {
    agent_onboarding_with_transport(context, action, payload, Arc::new(ReqwestTransport::new()))
        .await
}

pub(crate) async fn agent_onboarding_with_transport(
    context: AgentOnboardingContext,
    action: String,
    payload: Value,
    transport: Arc<dyn Transport>,
) -> Result<ProviderOnboarding, ProviderError> {
    if context
        .provider_agent_config
        .get("mode")
        .and_then(Value::as_str)
        != Some("new")
    {
        return existing_agent_onboarding(context, action, payload, transport).await;
    }
    if action == "get" {
        if let Some(saved) = context.storage().get(&pending_state_key(&context)).await?
            && let Ok(saved_payload) = serde_json::from_slice::<Value>(&saved)
            && saved_payload.get("phase").and_then(Value::as_str) == Some("permissions")
        {
            return Ok(onboarding(
                "permissions",
                ProviderOnboardingStatus::ActionRequired,
                saved_payload,
            ));
        }
        return Ok(onboarding(
            "create_app",
            ProviderOnboardingStatus::ActionRequired,
            json!({}),
        ));
    }
    if payload.get("phase").and_then(Value::as_str) == Some("permissions") {
        return poll_permissions(context, payload, transport).await;
    }
    if action == "start" {
        let registration =
            tokio::task::spawn_blocking(|| FeishuRegistrar::new(ReqwestTransport::new()).start())
                .await
                .map_err(|_| external("feishu_registration_start"))?
                .map_err(|_| external("feishu_registration_start"))?;
        return Ok(onboarding(
            "create_app",
            ProviderOnboardingStatus::ActionRequired,
            json!({
                "device_code": registration.device_code,
                "verification_uri": registration.verification_uri,
                "interval_seconds": registration.interval.as_secs(),
                "expires_in_seconds": registration.expires_in.as_secs(),
            }),
        ));
    }
    let device_code = payload
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or(ProviderError::InvalidInput {
            field: "device_code",
        })?
        .to_owned();
    let result = tokio::task::spawn_blocking(move || {
        FeishuRegistrar::new(ReqwestTransport::new()).poll(&device_code)
    })
    .await
    .map_err(|_| external("feishu_registration_poll"))?
    .map_err(|_| external("feishu_registration_poll"))?;
    match result {
        RegistrationPoll::Pending { .. } => Ok(onboarding(
            "create_app",
            ProviderOnboardingStatus::Verifying,
            payload,
        )),
        RegistrationPoll::Complete(credential) => {
            context
                .write_agent_secret("app_secret", SecretValue::new(credential.app_secret()))
                .await?;
            let link = authorization_url(&Domain::Feishu, &credential.app_id)
                .map_err(|_| external("feishu_authorization_link"))?;
            let permissions_payload = json!({
                "phase": "permissions",
                "app_id": credential.app_id,
                "creator_open_id": credential.creator_open_id,
                "domain": "feishu",
                "verification_uri": link,
                "interval_seconds": 5,
            });
            context
                .storage()
                .put(
                    &pending_state_key(&context),
                    serde_json::to_vec(&permissions_payload)
                        .map_err(|_| external("feishu_authorization_state"))?,
                )
                .await?;
            Ok(onboarding(
                "permissions",
                ProviderOnboardingStatus::ActionRequired,
                permissions_payload,
            ))
        }
        RegistrationPoll::Denied => Ok(ProviderOnboarding {
            error_code: Some("access_denied".into()),
            error_message: Some("飞书未确认创建应用。".into()),
            ..onboarding("create_app", ProviderOnboardingStatus::Error, payload)
        }),
        RegistrationPoll::Expired => Ok(ProviderOnboarding {
            error_code: Some("expired".into()),
            error_message: Some("本次确认已过期。".into()),
            ..onboarding("create_app", ProviderOnboardingStatus::Error, json!({}))
        }),
    }
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
    let creator_open_id = payload
        .get("creator_open_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
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
    let inspected_creator_open_id = creator_open_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        let client = FeishuClient::new(transport, Credentials::app(inspected_app_id, app_secret))?
            .with_domain(domain);
        let capabilities = client.published_capabilities(Auth::Tenant)?;
        if !contains_standard_capabilities(&capabilities.tenant_scopes, &capabilities.events) {
            return Ok::<_, crate::ClientError>(None);
        }
        let bot = client.bot_info(Auth::Tenant)?;
        let creator = if inspected_creator_open_id.trim().is_empty() {
            None
        } else {
            Some(client.get_user(&inspected_creator_open_id, Auth::Tenant)?)
        };
        Ok(Some((bot.open_id, creator)))
    })
    .await
    .map_err(|_| external("feishu_authorization_inspection"))?;
    match result {
        Ok(Some((bot_open_id, creator))) => {
            let creator_person_id = if let Some(creator) = creator {
                let (external_id_type, external_id) = if !creator.union_id.trim().is_empty() {
                    ("union_id", creator.union_id.trim())
                } else if !creator.user_id.trim().is_empty() {
                    ("user_id", creator.user_id.trim())
                } else {
                    ("open_id", creator.open_id.trim())
                };
                let mut person = context.storage().get_or_create_person(external_id).await?;
                let mut state = person
                    .provider_state
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                state.insert("external_id_type".to_owned(), json!(external_id_type));
                let mut open_ids = state
                    .remove("open_ids_by_app")
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                if !creator.open_id.trim().is_empty() {
                    open_ids.insert(app_id.clone(), json!(creator.open_id));
                }
                state.insert("open_ids_by_app".to_owned(), Value::Object(open_ids));
                let next = Value::Object(state);
                if next != person.provider_state {
                    person = context
                        .storage()
                        .compare_and_set_person_state(
                            &person.person_id,
                            person.state_revision,
                            next,
                        )
                        .await?;
                }
                Some(person.person_id.to_string())
            } else {
                None
            };
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
        Ok(None) | Err(_) => Ok(onboarding(
            "permissions",
            ProviderOnboardingStatus::Verifying,
            payload,
        )),
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
    let capabilities = tokio::task::spawn_blocking(move || {
        FeishuClient::new(
            inspected_transport,
            Credentials::app(inspected_app_id, inspected_app_secret),
        )?
        .with_domain(inspected_domain)
        .published_capabilities(Auth::Tenant)
    })
    .await
    .map_err(|_| external("feishu_authorization_inspection"))?
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
            let update = tokio::task::spawn_blocking(move || {
                FeishuClient::new(transport, Credentials::app(update_app_id, app_secret))?
                    .with_domain(update_domain)
                    .update_authorization(&missing_scopes, &missing_events, Auth::Tenant)
            })
            .await;
            if !matches!(update, Ok(Ok(_))) {
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
