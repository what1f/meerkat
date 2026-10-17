use crate::{
    console,
    http::{Method, Request, Transport},
};
use lankide_provider::{
    InitContext, InvocationHandler, ProviderError, ProviderFuture, ProviderStorage, ResourceSaver,
    ResourceUpload, SecretValue,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct OwnerHandler {
    pub context: InitContext,
    pub transport: Arc<dyn Transport>,
}

pub(crate) fn external(error: crate::ClientError) -> ProviderError {
    ProviderError::External {
        operation: "feishu_workspace_owner",
        retryable: error.is_retryable(),
    }
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
    context
        .workspace_secret_writer()
        .write(
            "feishu_owner_access_token",
            SecretValue::new(grant.access_token()),
        )
        .await?;
    context
        .workspace_secret_writer()
        .write("feishu_owner_app_secret", SecretValue::new(app_secret))
        .await?;
    let mut state = context.storage().workspace_state().await?;
    state["owner"] = owner;
    state["owner_app_id"] = json!(app_id);
    state["owner_api_base"] = json!(base);
    state["owner_open_id"] = json!(grant.open_id);
    state["owner_tenant_key"] = grant.info.get("tenant_key").cloned().unwrap_or(Value::Null);
    context.storage().put_workspace_state(state).await?;
    context.storage().management_changed().await
}

impl InvocationHandler for OwnerHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let state = self.context.storage().workspace_state().await?;
            let owner = state
                .get("owner")
                .cloned()
                .unwrap_or_else(|| json!({"state":"unbound"}));
            match input.get("action").and_then(Value::as_str) {
                Some("get") => Ok(owner),
                Some("refresh") if owner.get("person_id").is_none() => Ok(owner),
                Some("refresh") => {
                    let token = self
                        .context
                        .secret_reader()
                        .read("feishu_owner_access_token")
                        .await?;
                    let base = state.get("owner_api_base").and_then(Value::as_str).ok_or(
                        ProviderError::InvalidInput {
                            field: "owner_api_base",
                        },
                    )?;
                    match user_info(self.transport.as_ref(), base, token.expose_secret()).await {
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
                        Err(error) if error.is_retryable() => Err(external(error)),
                        Err(
                            crate::ClientError::Api { .. }
                            | crate::ClientError::HttpStatus {
                                status: 401 | 403, ..
                            },
                        ) => {
                            let mut owner = owner;
                            owner["state"] = json!("reauth_required");
                            let mut state = state;
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
) -> Result<Value, ProviderError> {
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
            let field = |key: &'static str| {
                state
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or(ProviderError::InvalidInput { field: key })
            };
            let app = field("owner_app_id")?;
            let base = field("owner_api_base")?;
            let tenant = field("owner_tenant_key")?;
            let secret = context
                .secret_reader()
                .read("feishu_owner_app_secret")
                .await?;
            let mut console = console::DeveloperConsole::new(transport.clone(), session);
            let grant = console
                .authorize_user(app, secret.expose_secret(), base)
                .await
                .map_err(external)?;
            if grant.info.get("tenant_key").and_then(Value::as_str) != Some(tenant) {
                return Err(ProviderError::InvalidInput {
                    field: "owner_tenant",
                });
            }
            let owner = profile(
                context.storage(),
                &ContextSaver(context),
                transport.as_ref(),
                &grant.info,
            )
            .await?;
            let _guard = lock.lock().await;
            if !console::switch_is_current(context.secret_reader(), &candidate).await? {
                return Ok(json!({"state":"required"}));
            }
            context
                .secret_writer()
                .write(
                    "feishu_owner_access_token",
                    SecretValue::new(grant.access_token()),
                )
                .await?;
            console::save_session(context.secret_writer(), &console.into_session()).await?;
            let mut state = context.storage().workspace_state().await?;
            state["owner"] = owner;
            state["owner_open_id"] = json!(grant.open_id);
            context.storage().put_workspace_state(state).await?;
            console::finish_switch(context.secret_writer()).await?;
            context.storage().management_changed().await?;
            Ok(json!({"state":"ready"}))
        }
    }
}

struct OnboardingSaver<'a>(&'a lankide_provider::AgentOnboardingContext);
impl ResourceSaver for OnboardingSaver<'_> {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, lankide_core::ResourceRef> {
        Box::pin(async move { self.0.save_resource(upload).await })
    }
}
