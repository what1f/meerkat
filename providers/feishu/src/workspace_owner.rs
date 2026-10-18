use crate::{
    console,
    http::{Method, Request, Transport},
};
use lankide_provider::{
    InitContext, InvocationHandler, ProviderError, ProviderFuture, ProviderStorage,
    ProviderTeamMember, ResourceSaver, ResourceUpload, SecretValue, SecretWriter, TeamRole,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct OwnerHandler {
    pub authorization_lock: Arc<tokio::sync::Mutex<()>>,
    pub context: InitContext,
    pub transport: Arc<dyn Transport>,
}

pub(crate) fn external(error: crate::ClientError) -> ProviderError {
    error.provider_error(false)
}

pub(crate) async fn user_info(
    transport: &dyn Transport,
    base: &str,
    token: &str,
) -> Result<Value, crate::ClientError> {
    let response = transport
        .execute(
            Request::new(Method::Get, format!("{base}/open-apis/authen/v1/user_info"))
                .header("Authorization", format!("Bearer {token}")),
        )
        .await?;
    console::passport_data(&response, "user_info")
}

pub(crate) async fn profile(
    storage: &dyn ProviderStorage,
    saver: &dyn ResourceSaver,
    transport: &dyn Transport,
    info: &Value,
) -> Result<Value, ProviderError> {
    let external_id = ["union_id", "user_id", "open_id"]
        .into_iter()
        .find_map(|key| {
            info.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
        .ok_or(ProviderError::InvalidInput {
            field: "owner_identity",
        })?;
    let person = storage.get_or_create_person(external_id).await?;
    let mut owner = json!({"state":"ready","person_id":person.person_id});
    if let Some(name) = info.get("name").and_then(Value::as_str) {
        owner["name"] = json!(name);
    }
    if let Some(url) = info
        .get("avatar_url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let response = transport
            .execute(Request::new(Method::Get, url))
            .await
            .map_err(|e| external(e.into()))?;
        crate::http::validate_success(&response, "owner_avatar").map_err(external)?;
        let media_type = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "image/jpeg".into());
        owner["avatar"] = resource_json(
            saver
                .save(ResourceUpload {
                    file_name: "owner.jpg".into(),
                    media_type,
                    bytes: response.body,
                })
                .await?,
        );
    }
    Ok(owner)
}

pub(crate) async fn save_grant(
    context: &lankide_provider::AgentOnboardingContext,
    transport: &dyn Transport,
    grant: &console::UserGrant,
    app_id: &str,
    base: &str,
    app_secret: &str,
) -> Result<(), ProviderError> {
    let owner = profile(
        context.storage(),
        &OnboardingSaver(context),
        transport,
        &grant.info,
    )
    .await?;
    save_authorization(
        context.storage(),
        context.workspace_secret_writer(),
        owner,
        grant,
        app_id,
        base,
        app_secret,
    )
    .await
}

async fn save_authorization(
    storage: &dyn ProviderStorage,
    writer: &dyn SecretWriter,
    owner: Value,
    grant: &console::UserGrant,
    app_id: &str,
    base: &str,
    app_secret: &str,
) -> Result<(), ProviderError> {
    writer
        .write(
            "feishu_owner_refresh_token",
            SecretValue::new(grant.refresh_token()),
        )
        .await?;
    writer
        .write(
            "feishu_owner_access_token",
            SecretValue::new(grant.access_token()),
        )
        .await?;
    writer
        .write("feishu_owner_app_secret", SecretValue::new(app_secret))
        .await?;
    let mut state = storage.workspace_state().await?;
    state["owner"] = owner;
    state["owner_app_id"] = json!(app_id);
    state["owner_api_base"] = json!(base);
    state["owner_open_id"] = json!(grant.open_id);
    state["owner_tenant_key"] = grant.info.get("tenant_key").cloned().unwrap_or(Value::Null);
    storage.put_workspace_state(state).await?;
    storage.management_changed().await
}

/// Only authentication failures imply that signing in again can repair the call.
/// Scope, resource permissions, network and rate-limit errors do not change the owner.
pub(crate) fn requires_sign_in(error: &crate::ClientError) -> bool {
    match error {
        crate::ClientError::HttpStatus {
            status: 401 | 403, ..
        } => true,
        crate::ClientError::Api {
            code: 99991668,
            message,
            ..
        } => !message.contains("user access token not supported"),
        crate::ClientError::Api { code, .. } => {
            matches!(code, 99991661 | 99991663 | 99991671 | 99991677 | 99991679)
        }
        _ => false,
    }
}

pub(crate) async fn authorization_ready(
    context: &InitContext,
    state: &Value,
) -> Result<bool, ProviderError> {
    if state["owner"]["state"] != "ready"
        || [
            "owner_app_id",
            "owner_api_base",
            "owner_tenant_key",
            "owner_open_id",
        ]
        .iter()
        .any(|key| nonempty(&state[*key]).is_none())
        || nonempty(&state["owner"]["person_id"]).is_none()
    {
        return Ok(false);
    }
    for key in ["feishu_owner_app_secret", "feishu_owner_access_token"] {
        match context.secret_reader().read(key).await {
            Ok(secret) if !secret.expose_secret().trim().is_empty() => {}
            Ok(_) | Err(ProviderError::SecretAccessDenied) => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

pub(crate) async fn needs_sign_in(storage: &dyn ProviderStorage) -> Result<Value, ProviderError> {
    let mut state = storage.workspace_state().await?;
    let mut owner = state.get("owner").cloned().unwrap_or_else(|| json!({}));
    if owner["state"] != "needs_sign_in" {
        owner["state"] = json!("needs_sign_in");
        state["owner"] = owner.clone();
        storage.put_workspace_state(state).await?;
        storage.management_changed().await?;
    }
    Ok(owner)
}

/// Called while holding the workspace authorization lock. The rotated refresh
/// token is persisted before exposing the new access token to another call.
pub(crate) async fn refresh_authorization(
    context: &InitContext,
    transport: &dyn Transport,
) -> Result<(), ProviderError> {
    let state = context.storage().workspace_state().await?;
    let refresh = match context
        .secret_reader()
        .read("feishu_owner_refresh_token")
        .await
    {
        Ok(value) if !value.expose_secret().trim().is_empty() => value,
        Ok(_) | Err(ProviderError::SecretAccessDenied) => {
            needs_sign_in(context.storage()).await?;
            return Err(ProviderError::AuthorizationRequired);
        }
        Err(error) => return Err(error),
    };
    let secret = context
        .secret_reader()
        .read("feishu_owner_app_secret")
        .await?;
    let app = nonempty(&state["owner_app_id"]).ok_or(ProviderError::AuthorizationRequired)?;
    let base = nonempty(&state["owner_api_base"]).ok_or(ProviderError::AuthorizationRequired)?;
    match console::refresh_user_token(
        transport,
        base,
        app,
        secret.expose_secret(),
        refresh.expose_secret(),
    )
    .await
    {
        Ok((access, refresh)) => {
            context
                .secret_writer()
                .write("feishu_owner_refresh_token", SecretValue::new(refresh))
                .await?;
            context
                .secret_writer()
                .write("feishu_owner_access_token", SecretValue::new(access))
                .await
        }
        Err(crate::ClientError::Api {
            code: 20026 | 20037 | 20064 | 20073 | 99991669,
            ..
        }) => {
            needs_sign_in(context.storage()).await?;
            Err(ProviderError::AuthorizationRequired)
        }
        Err(error) => Err(external(error)),
    }
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.trim().is_empty())
}

impl InvocationHandler for OwnerHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let _guard = self.authorization_lock.lock().await;
            let state = self.context.storage().workspace_state().await?;
            if !authorization_ready(&self.context, &state).await? {
                return match input.get("action").and_then(Value::as_str) {
                    Some("get" | "refresh") => needs_sign_in(self.context.storage()).await,
                    _ => Err(ProviderError::InvalidInput { field: "action" }),
                };
            }
            let owner = state["owner"].clone();
            match input.get("action").and_then(Value::as_str) {
                Some("get") => Ok(owner),
                Some("refresh") => {
                    let mut token = self
                        .context
                        .secret_reader()
                        .read("feishu_owner_access_token")
                        .await?;
                    let base = state.get("owner_api_base").and_then(Value::as_str).ok_or(
                        ProviderError::InvalidInput {
                            field: "owner_api_base",
                        },
                    )?;
                    let mut info =
                        user_info(self.transport.as_ref(), base, token.expose_secret()).await;
                    if info.as_ref().is_err_and(requires_sign_in) {
                        match refresh_authorization(&self.context, self.transport.as_ref()).await {
                            Ok(()) => {
                                token = self
                                    .context
                                    .secret_reader()
                                    .read("feishu_owner_access_token")
                                    .await?;
                                info =
                                    user_info(self.transport.as_ref(), base, token.expose_secret())
                                        .await;
                            }
                            Err(ProviderError::AuthorizationRequired) => {
                                return needs_sign_in(self.context.storage()).await;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    match info {
                        Ok(info) => {
                            let id = ["union_id", "user_id", "open_id"]
                                .into_iter()
                                .find_map(|key| {
                                    info.get(key)
                                        .and_then(Value::as_str)
                                        .filter(|s| !s.is_empty())
                                })
                                .ok_or(ProviderError::InvalidInput {
                                    field: "owner_identity",
                                })?;
                            let person = self.context.storage().get_or_create_person(id).await?;
                            if owner.get("person_id").and_then(Value::as_str)
                                != Some(person.person_id.as_str())
                            {
                                return Err(ProviderError::InvalidInput {
                                    field: "owner_identity",
                                });
                            }
                            let saver = ContextSaver(&self.context);
                            let owner = profile(
                                self.context.storage(),
                                &saver,
                                self.transport.as_ref(),
                                &info,
                            )
                            .await?;
                            let mut state = self.context.storage().workspace_state().await?;
                            state["owner"] = owner.clone();
                            self.context.storage().put_workspace_state(state).await?;
                            self.context.storage().management_changed().await?;
                            Ok(owner)
                        }
                        Err(error) => Err(external(error)),
                    }
                }
                _ => Err(ProviderError::InvalidInput { field: "action" }),
            }
        })
    }
}
struct ContextSaver<'a>(&'a InitContext);
impl ResourceSaver for ContextSaver<'_> {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, lankide_core::ResourceRef> {
        Box::pin(async move { self.0.save_resource(upload).await })
    }
}

fn resource_json(r: lankide_core::ResourceRef) -> Value {
    json!({"resource_id":r.resource_id,"file_name":r.file_name,"media_type":r.media_type,"size_bytes":r.size_bytes,"sha256":r.sha256})
}

pub(crate) async fn switch_account(
    context: &InitContext,
    transport: Arc<dyn Transport>,
    action: &str,
    lock: &tokio::sync::Mutex<()>,
    authorization_lock: &tokio::sync::Mutex<()>,
) -> Result<Value, ProviderError> {
    if action == "switch_start" {
        owner_app(context, &context.storage().workspace_state().await?).await?;
    }
    let (candidate, login) = console::switch_login(
        transport.clone(),
        context.secret_reader(),
        context.secret_writer(),
        action,
        lock,
    )
    .await?;
    match login {
        console::Login::Required => Ok(json!({"state":"required"})),
        console::Login::Waiting { qr_code, scanned } => Ok(json!({
            "state": if scanned { "scanned" } else { "waiting" },
            "qr_code": qr_code,
            "poll_interval_seconds": console::LOGIN_POLL_INTERVAL_SECONDS,
        })),
        console::Login::Ready(session) => {
            let state = context.storage().workspace_state().await?;
            let binding = owner_app(context, &state).await?;
            let mut console = console::DeveloperConsole::new(transport.clone(), session);
            if !console::switch_is_current(context.secret_reader(), &candidate).await? {
                return Ok(json!({"state":"required"}));
            }
            console
                .repair_user_authorization(&binding.app)
                .await
                .map_err(|error| error.provider_error(true))?;
            if !console::switch_is_current(context.secret_reader(), &candidate).await? {
                return Ok(json!({"state":"required"}));
            }
            let authorization = console
                .authorize_user(&binding.app, binding.secret.expose_secret(), &binding.base)
                .await;
            if matches!(
                &authorization,
                Err(crate::ClientError::Api { code: 20029, .. })
            ) {
                console
                    .log_authorization_diagnostics(&binding.app, context.logger())
                    .await;
            }
            let grant = authorization.map_err(|error| {
                    let configuration = matches!(&error, crate::ClientError::MissingField { operation, field }
                        if operation == "user_token" && field == "refresh_token");
                    error.provider_error(configuration)
                })?;
            let tenant =
                nonempty(&grant.info["tenant_key"]).ok_or(ProviderError::InvalidInput {
                    field: "owner_tenant",
                })?;
            if binding
                .tenant
                .as_deref()
                .is_some_and(|expected| expected != tenant)
            {
                return Err(ProviderError::InvalidInput {
                    field: "owner_tenant",
                });
            }
            // Initial binding must be the recorded App creator. Existing owners may
            // still switch to another account in the same tenant, as before.
            if let Some((person, open_id)) = &binding.creator {
                let id = ["union_id", "user_id", "open_id"]
                    .into_iter()
                    .find_map(|key| nonempty(&grant.info[key]));
                if id != Some(person.as_str()) || grant.open_id != *open_id {
                    return Err(ProviderError::InvalidInput {
                        field: "owner_identity",
                    });
                }
            }
            let owner = profile(
                context.storage(),
                &ContextSaver(context),
                transport.as_ref(),
                &grant.info,
            )
            .await?;
            let _authorization_guard = authorization_lock.lock().await;
            let _guard = lock.lock().await;
            if !console::switch_is_current(context.secret_reader(), &candidate).await? {
                return Ok(json!({"state":"required"}));
            }
            save_authorization(
                context.storage(),
                context.secret_writer(),
                owner,
                &grant,
                &binding.app,
                &binding.base,
                binding.secret.expose_secret(),
            )
            .await?;
            console::save_session(context.secret_writer(), &console.into_session()).await?;
            console::finish_switch(context.secret_writer()).await?;
            Ok(json!({"state":"ready"}))
        }
    }
}

struct OwnerApp {
    app: String,
    base: String,
    tenant: Option<String>,
    secret: SecretValue,
    creator: Option<(String, String)>,
}

async fn owner_app(context: &InitContext, state: &Value) -> Result<OwnerApp, ProviderError> {
    let owner_id = nonempty(&state["owner"]["person_id"]);
    if owner_id.is_some()
        && let (Some(app), Some(base), Some(tenant)) = (
            nonempty(&state["owner_app_id"]),
            nonempty(&state["owner_api_base"]),
            nonempty(&state["owner_tenant_key"]),
        )
    {
        match context
            .secret_reader()
            .read("feishu_owner_app_secret")
            .await
        {
            Ok(secret) if !secret.expose_secret().trim().is_empty() => {
                return Ok(OwnerApp {
                    app: app.into(),
                    base: base.into(),
                    tenant: Some(tenant.into()),
                    secret,
                    creator: None,
                });
            }
            Ok(_) | Err(ProviderError::SecretAccessDenied) => {}
            Err(error) => return Err(error),
        }
    }
    let roster = context.storage().team_members().await?;
    let person = if let Some(id) = owner_id {
        Some(
            context
                .storage()
                .person(&lankide_core::PersonId::new(id).map_err(|_| {
                    ProviderError::InvalidInput {
                        field: "owner_identity",
                    }
                })?)
                .await?,
        )
    } else {
        roster.iter().find_map(|member| match member {
            ProviderTeamMember::Human(person) if person.team_role == TeamRole::Lead => {
                Some(person.clone())
            }
            _ => None,
        })
    }
    .ok_or(ProviderError::OwnerAppUnavailable)?;
    let directory = crate::identity::IdentityDirectory::from_workspace_state(state)?;
    let mut agents: Vec<_> = roster
        .into_iter()
        .filter_map(|member| match member {
            ProviderTeamMember::Agent { agent_id } => Some(agent_id),
            _ => None,
        })
        .collect();
    agents.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for id in agents {
        let config = context.storage().agent_config(&id).await?;
        let Some(identity) = directory.agents.get(id.as_str()) else {
            continue;
        };
        let (Some(app), Some(open_id)) = (
            nonempty(&config["app_id"]),
            nonempty(&config["creator_open_id"]),
        ) else {
            continue;
        };
        if config["creator_person_id"].as_str() != Some(person.person_id.as_str())
            || identity.app_id != app
            || context.storage().agent_state(&id).await?["app_id"] != app
            || person.provider_state["open_ids_by_app"][app].as_str() != Some(open_id)
        {
            continue;
        }
        let person_tenant = nonempty(&person.provider_state["tenant_key"]);
        let app_tenant = (!identity.tenant_key.is_empty()).then_some(identity.tenant_key.as_str());
        let saved_tenant = nonempty(&state["owner_tenant_key"]);
        let tenants: Vec<_> = [saved_tenant, person_tenant, app_tenant]
            .into_iter()
            .flatten()
            .collect();
        if tenants.iter().any(|t| Some(*t) != tenants.first().copied()) {
            continue;
        }
        let domain = match config["domain"].as_str().unwrap_or("feishu") {
            "feishu" => crate::client::Domain::Feishu,
            "lark" => crate::client::Domain::Lark,
            _ => continue,
        };
        let secret = match context
            .secret_reader()
            .read(&format!("agent:{id}:app_secret"))
            .await
        {
            Ok(secret) if !secret.expose_secret().trim().is_empty() => secret,
            Ok(_) | Err(ProviderError::SecretAccessDenied) => continue,
            Err(error) => return Err(error),
        };
        return Ok(OwnerApp {
            app: app.into(),
            base: domain.base_url().into(),
            tenant: tenants.first().map(|s| (*s).to_owned()),
            secret,
            creator: (owner_id.is_none() || tenants.is_empty())
                .then(|| (person.external_id.clone(), open_id.into())),
        });
    }
    Err(ProviderError::OwnerAppUnavailable)
}

struct OnboardingSaver<'a>(&'a lankide_provider::AgentOnboardingContext);
impl ResourceSaver for OnboardingSaver<'_> {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, lankide_core::ResourceRef> {
        Box::pin(async move { self.0.save_resource(upload).await })
    }
}
