use crate::Transport;
use crate::{Credentials, PollRequest, ReqwestTransport, TelegramClient};
use lankide_provider::{
    AgentOnboardingContext, ProviderError, ProviderOnboarding, ProviderOnboardingStatus,
    SecretValue,
};
use serde_json::{Value, json};
use std::sync::Arc;

fn is_stale_managed_bot_candidate(error: &crate::ClientError) -> bool {
    match error {
        crate::ClientError::InvalidInput(_) => true,
        crate::ClientError::Api { code, .. } => matches!(*code, 400 | 403),
        _ => false,
    }
}

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
        ) && payload
            .get("bot_username")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()))
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

async fn save(
    context: &AgentOnboardingContext,
    value: &ProviderOnboarding,
) -> Result<(), ProviderError> {
    let mut state = context.storage().agent_state(&context.agent_id).await?;
    let object = state.as_object_mut().ok_or(ProviderError::InvalidInput {
        field: "agent_provider_state",
    })?;
    object.insert(
        "onboarding".into(),
        json!({
            "step": value.step,
            "status": match value.status {
                ProviderOnboardingStatus::ActionRequired => "action_required",
                ProviderOnboardingStatus::Verifying => "verifying",
                ProviderOnboardingStatus::Ready => "ready",
                ProviderOnboardingStatus::Error => "error",
            },
            "payload": value.opaque_payload,
            "error_code": value.error_code,
            "error_message": value.error_message,
        }),
    );
    context
        .storage()
        .put_agent_state(&context.agent_id, state)
        .await
}

async fn load(
    context: &AgentOnboardingContext,
) -> Result<Option<ProviderOnboarding>, ProviderError> {
    let state = context.storage().agent_state(&context.agent_id).await?;
    let Some(value) = state.get("onboarding") else {
        return Ok(None);
    };
    let status = match value.get("status").and_then(Value::as_str) {
        Some("action_required") => ProviderOnboardingStatus::ActionRequired,
        Some("verifying") => ProviderOnboardingStatus::Verifying,
        Some("ready") => ProviderOnboardingStatus::Ready,
        Some("error") => ProviderOnboardingStatus::Error,
        _ => return Ok(None),
    };
    let payload = value.get("payload").cloned().unwrap_or_else(|| json!({}));
    Ok(Some(ProviderOnboarding {
        step: value
            .get("step")
            .and_then(Value::as_str)
            .unwrap_or("create_bot")
            .into(),
        status,
        error_code: value
            .get("error_code")
            .and_then(Value::as_str)
            .map(str::to_owned),
        error_message: value
            .get("error_message")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..onboarding("create_bot", status, payload)
    }))
}

pub async fn agent_onboarding(
    context: AgentOnboardingContext,
    action: String,
    supplied_payload: Value,
) -> Result<ProviderOnboarding, ProviderError> {
    agent_onboarding_with_transport(
        context,
        action,
        supplied_payload,
        Arc::new(ReqwestTransport::new()),
    )
    .await
}

pub(crate) async fn agent_onboarding_with_transport(
    context: AgentOnboardingContext,
    action: String,
    supplied_payload: Value,
    transport: Arc<dyn Transport>,
) -> Result<ProviderOnboarding, ProviderError> {
    if context
        .provider_agent_config
        .get("mode")
        .and_then(Value::as_str)
        != Some("new")
    {
        return Ok(ProviderOnboarding::ready(supplied_payload));
    }
    if action == "get" {
        if let Some(value) = load(&context).await? {
            return Ok(value);
        }
        let value = onboarding(
            "create_bot",
            ProviderOnboardingStatus::ActionRequired,
            json!({}),
        );
        save(&context, &value).await?;
        return Ok(value);
    }

    let manager_token = context
        .workspace_secrets()
        .read("manager_bot_token")
        .await?
        .expose_secret()
        .to_owned();
    let manager_account_id = manager_token
        .split_once(':')
        .map(|(id, _)| id.to_owned())
        .ok_or(ProviderError::InvalidInput {
            field: "manager_bot_token",
        })?;
    let username = context
        .provider_agent_config
        .get("bot_username")
        .or_else(|| context.provider_agent_config.get("provider_handle"))
        .and_then(Value::as_str)
        .ok_or(ProviderError::InvalidInput {
            field: "bot_username",
        })?
        .to_owned();

    let endpoint = crate::adapter::api_endpoint(&context.workspace_config)?;
    if action == "start" {
        let token = manager_token.clone();
        let username_for_start = username.clone();
        let display_name = context.name.clone();
        let registration = (async move {
            let client = TelegramClient::new(
                transport.clone(),
                Credentials::new(manager_account_id, token),
            )?
            .with_domain(endpoint);
            let manager = client.verify_credentials().await?;
            client.managed_bot_registration(&manager, &username_for_start, &display_name)
        })
        .await
        .map_err(|_| external("telegram_registration_start"))?;
        let value = onboarding(
            "create_bot",
            ProviderOnboardingStatus::ActionRequired,
            json!({
                "verification_uri": registration.verification_uri(),
                "offset": 0,
                // Keep the normalized identifier used to build the link.  Polls
                // must continue watching the Bot selected at the start of this
                // onboarding session, even if the Agent profile changes later.
                "bot_username": registration.username,
            }),
        );
        save(&context, &value).await?;
        return Ok(value);
    }

    let payload = if supplied_payload
        .as_object()
        .is_some_and(|object| object.is_empty())
    {
        load(&context)
            .await?
            .map(|value| value.opaque_payload)
            .unwrap_or_else(|| json!({}))
    } else {
        supplied_payload
    };
    let username = payload
        .get("bot_username")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or(username);
    let manager_username = context
        .storage()
        .workspace_state()
        .await?
        .get("manager_bot_username")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let offset = payload.get("offset").and_then(Value::as_i64).unwrap_or(0);
    let token = manager_token;
    let manager_username_for_poll = manager_username;
    let username_for_poll = username.clone();
    let display_name = context.name.clone();
    let poll_transport = transport.clone();
    let poll = (async move {
        let client =
            TelegramClient::new(poll_transport, Credentials::new(manager_account_id, token))?
                .with_domain(endpoint);
        let registration = if let Some(manager_username) = manager_username_for_poll {
            client.managed_bot_registration_for_manager(
                &manager_username,
                &username_for_poll,
                &display_name,
            )?
        } else {
            let manager = client.verify_credentials().await?;
            client.managed_bot_registration(&manager, &username_for_poll, &display_name)?
        };
        let mut request = PollRequest::managed_bot();
        request.offset = offset;
        request.timeout_seconds = 1;
        let updates = client.get_updates(&request).await?;
        let next_offset = updates
            .iter()
            .map(|update| update.update_id + 1)
            .max()
            .unwrap_or(offset);
        let mut credential = None;
        for update in &updates {
            let Some(managed) = update.managed_bot.as_ref() else {
                continue;
            };
            if !registration.matches(&managed.bot) {
                continue;
            }
            match client.managed_bot_credential(update).await {
                Ok(value) => {
                    credential = Some(value);
                    break;
                }
                // The Manager Bot may still have older events for the same
                // username after a Bot was removed.  Those events cannot
                // yield a credential; continue with the rest of this batch.
                Err(error) if is_stale_managed_bot_candidate(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok::<_, crate::ClientError>((credential, next_offset))
    })
    .await
    .map_err(|_| external("telegram_registration_poll"))?;

    if let Some(credential) = poll.0 {
        let state = context.storage().workspace_state().await?;
        if state
            .get("owner_user_id")
            .and_then(Value::as_i64)
            .is_none_or(|id| id == credential.creator.id)
        {
            let mut state = state;
            let person = context
                .storage()
                .get_or_create_person(&credential.creator_user_id)
                .await?;
            let mut owner = json!({"state":"ready","person_id":person.person_id,"name":credential.creator.display_name()});
            let token = context
                .workspace_secrets()
                .read("manager_bot_token")
                .await?;
            let manager = TelegramClient::new(
                transport.clone(),
                Credentials::new("manager", token.expose_secret()),
            )
            .map_err(|_| external("telegram_owner"))?
            .with_domain(crate::adapter::api_endpoint(&context.workspace_config)?);
            if let Some(photo) = manager
                .user_profile_photo(credential.creator.id)
                .await
                .map_err(|_| external("telegram_owner_photo"))?
            {
                owner["avatar"] = resource_json(
                    context
                        .save_resource(lankide_provider::ResourceUpload {
                            file_name: "owner.jpg".into(),
                            media_type: photo.content_type,
                            bytes: photo.data,
                        })
                        .await?,
                );
            }
            state["owner"] = owner;
            state["owner_user_id"] = json!(credential.creator.id);
            context.storage().put_workspace_state(state).await?;
            context.storage().management_changed().await?;
        }
        context
            .write_agent_secret("bot_token", SecretValue::new(credential.token()))
            .await?;
        let mut provider_state = context.storage().agent_state(&context.agent_id).await?;
        let object = provider_state
            .as_object_mut()
            .ok_or(ProviderError::InvalidInput {
                field: "agent_provider_state",
            })?;
        object.insert("account_id".into(), json!(credential.account_id.clone()));
        object.insert("bot_username".into(), json!(credential.username.clone()));
        object.insert(
            "creator_user_id".into(),
            json!(credential.creator_user_id.clone()),
        );
        context
            .storage()
            .put_agent_state(&context.agent_id, provider_state)
            .await?;
        let mut value = onboarding("ready", ProviderOnboardingStatus::Ready, json!({}));
        value.agent_config_updates = json!({
            "mode": "existing",
            "bot_username": credential.username,
            "provider_handle": credential.username,
        });
        save(&context, &value).await?;
        Ok(value)
    } else {
        let mut next_payload = payload;
        next_payload["offset"] = json!(poll.1);
        let value = onboarding(
            "create_bot",
            ProviderOnboardingStatus::Verifying,
            next_payload,
        );
        save(&context, &value).await?;
        Ok(value)
    }
}

fn resource_json(r: lankide_core::ResourceRef) -> Value {
    json!({"resource_id":r.resource_id,"file_name":r.file_name,"media_type":r.media_type,"size_bytes":r.size_bytes,"sha256":r.sha256})
}
