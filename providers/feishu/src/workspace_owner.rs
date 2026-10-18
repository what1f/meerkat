use crate::{
    Credentials, FeishuClient, console,
    http::{Method, Request, Transport},
};
use lankide_provider::{
    InitContext, InvocationHandler, ProviderError, ProviderFuture, ProviderStorage,
    ProviderTeamMember, ResourceSaver, ResourceUpload,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct OwnerHandler {
    pub profile_lock: Arc<tokio::sync::Mutex<()>>,
    pub context: InitContext,
    pub transport: Arc<dyn Transport>,
}

pub(crate) fn external(error: crate::ClientError) -> ProviderError {
    error.provider_error(false)
}

pub(crate) fn check_account(
    state: &Value,
    info: &Value,
    switching: bool,
) -> Result<(), ProviderError> {
    if state["console_tenant_id"]
        .as_str()
        .is_some_and(|v| Some(v) != info["tenantId"].as_str())
    {
        return Err(ProviderError::InvalidInput {
            field: "owner_tenant",
        });
    }
    if !switching
        && state["console_user_id"]
            .as_str()
            .is_some_and(|v| Some(v) != info["id"].as_str())
    {
        return Err(ProviderError::InvalidInput {
            field: "owner_identity",
        });
    }
    Ok(())
}

pub(crate) async fn profile(
    saver: &dyn ResourceSaver,
    transport: &dyn Transport,
    info: &Value,
    person: Option<&str>,
) -> Result<Value, ProviderError> {
    let mut owner = json!({"state":"ready", "name":info["name"]});
    if let Some(id) = person {
        owner["person_id"] = json!(id);
    }
    if let Some(url) = info["avatar"].as_str().filter(|v| !v.is_empty()) {
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
        let r = saver
            .save(ResourceUpload {
                file_name: "owner.jpg".into(),
                media_type,
                bytes: response.body,
            })
            .await?;
        owner["avatar"] = json!({"resource_id":r.resource_id,"file_name":r.file_name,"media_type":r.media_type,"size_bytes":r.size_bytes,"sha256":r.sha256});
    }
    Ok(owner)
}

pub(crate) async fn save_profile(
    storage: &dyn ProviderStorage,
    info: &Value,
    owner: Value,
) -> Result<(), ProviderError> {
    let mut state = storage.workspace_state().await?;
    state["owner"] = owner;
    state["console_user_id"] = info["id"].clone();
    state["console_tenant_id"] = info["tenantId"].clone();
    storage.put_workspace_state(state).await?;
    storage.management_changed().await
}

/// Only applications/me supplies the open identity. Console IDs never enter the person directory.
pub(crate) async fn bind_creator(
    storage: &dyn ProviderStorage,
    client: &FeishuClient<Arc<dyn Transport>>,
    app: &str,
) -> Result<(String, String), ProviderError> {
    let union = client
        .application_creator("union_id")
        .await
        .map_err(external)?;
    let open = client
        .application_creator("open_id")
        .await
        .map_err(external)?;
    let mut person = storage.get_or_create_person(&union).await?;
    let mut state = person.provider_state.clone();
    state["external_id_type"] = json!("union_id");
    state["open_ids_by_app"][app] = json!(open);
    person = storage
        .compare_and_set_person_state(&person.person_id, person.state_revision, state)
        .await?;
    Ok((person.person_id.to_string(), open))
}

pub(crate) struct ContextSaver<'a>(pub &'a InitContext);
impl ResourceSaver for ContextSaver<'_> {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, lankide_core::ResourceRef> {
        Box::pin(async move { self.0.save_resource(upload).await })
    }
}
pub(crate) struct OnboardingSaver<'a>(pub &'a lankide_provider::AgentOnboardingContext);
impl ResourceSaver for OnboardingSaver<'_> {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, lankide_core::ResourceRef> {
        Box::pin(async move { self.0.save_resource(upload).await })
    }
}

impl InvocationHandler for OwnerHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let _guard = self.profile_lock.lock().await;
            let state = self.context.storage().workspace_state().await?;
            let owner = state
                .get("owner")
                .cloned()
                .unwrap_or_else(|| json!({"state":"unbound"}));
            match input["action"].as_str() {
                Some("get") => Ok(owner),
                Some("refresh") => {
                    let Some(session) = console::saved_session(
                        self.transport.clone(),
                        self.context.secret_reader(),
                    )
                    .await?
                    else {
                        return Ok(owner);
                    };
                    let mut console =
                        console::DeveloperConsole::new(self.transport.clone(), session);
                    let info = console.profile().await.map_err(external)?;
                    check_account(&state, &info, false)?;
                    let next = profile(
                        &ContextSaver(&self.context),
                        self.transport.as_ref(),
                        &info,
                        owner["person_id"].as_str(),
                    )
                    .await?;
                    console::save_session(self.context.secret_writer(), &console.into_session())
                        .await?;
                    save_profile(self.context.storage(), &info, next.clone()).await?;
                    Ok(next)
                }
                _ => Err(ProviderError::InvalidInput { field: "action" }),
            }
        })
    }
}

pub(crate) async fn switch_account(
    context: &InitContext,
    transport: Arc<dyn Transport>,
    action: &str,
    lock: &tokio::sync::Mutex<()>,
    profile_lock: &tokio::sync::Mutex<()>,
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
        console::Login::Waiting { qr_code, scanned } => Ok(
            json!({"state":if scanned {"scanned"} else {"waiting"}, "qr_code":qr_code,"poll_interval_seconds":console::LOGIN_POLL_INTERVAL_SECONDS}),
        ),
        console::Login::Ready(session) => {
            let _profile_guard = profile_lock.lock().await;
            let state = context.storage().workspace_state().await?;
            let mut console = console::DeveloperConsole::new(transport.clone(), session);
            let info = console.profile().await.map_err(external)?;
            check_account(&state, &info, true)?;
            let person = if state["console_user_id"] == info["id"] {
                state["owner"]["person_id"].as_str().map(str::to_owned)
            } else {
                let mut binding = None;
                for member in context.storage().team_members().await? {
                    let ProviderTeamMember::Agent { agent_id } = member else {
                        continue;
                    };
                    let config = context.storage().agent_config(&agent_id).await?;
                    let Some(app) = config["app_id"].as_str() else {
                        continue;
                    };
                    match console.verify_creator(app, &info).await {
                        Ok(()) => {}
                        Err(crate::ClientError::InvalidInput(_)) => continue,
                        Err(error) => return Err(external(error)),
                    }
                    let secret = context
                        .secret_reader()
                        .read(&format!("agent:{agent_id}:app_secret"))
                        .await?;
                    let domain =
                        crate::adapter::configured_domain(&config, context.workspace_config())?;
                    let client = FeishuClient::new(
                        transport.clone(),
                        Credentials::app(app, secret.expose_secret()),
                    )
                    .map_err(external)?
                    .with_domain(domain);
                    binding = Some(bind_creator(context.storage(), &client, app).await?.0);
                    break;
                }
                Some(binding.ok_or(ProviderError::InvalidInput {
                    field: "owner_identity",
                })?)
            };
            let owner = profile(
                &ContextSaver(context),
                transport.as_ref(),
                &info,
                person.as_deref(),
            )
            .await?;
            let _guard = lock.lock().await;
            if !console::switch_is_current(context.secret_reader(), &candidate).await? {
                return Ok(json!({"state":"required"}));
            }
            save_profile(context.storage(), &info, owner).await?;
            console::save_session(context.secret_writer(), &console.into_session()).await?;
            console::finish_switch(context.secret_writer()).await?;
            Ok(json!({"state":"ready"}))
        }
    }
}
