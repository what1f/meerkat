use crate::{
    client::{Auth, Client, Credentials},
    dto::TaskRecord,
    http::Transport,
    tasks::{CreateTask, ListRequest, MemberIdentity, TaskPatch},
};
use lankide_provider::{InitContext, InvocationHandler, ProviderError, ProviderFuture};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct TasksHandler {
    pub context: InitContext,
    pub transport: Arc<dyn Transport>,
}
fn error(e: crate::ClientError) -> ProviderError {
    ProviderError::External {
        operation: "feishu_workspace_tasks",
        retryable: e.is_retryable(),
    }
}
fn authorization_error(error: ProviderError) -> ProviderError {
    match error {
        ProviderError::SecretAccessDenied => ProviderError::AuthorizationRequired,
        other => other,
    }
}
fn text<'a>(v: &'a Value, k: &'static str) -> Result<&'a str, ProviderError> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or(ProviderError::InvalidInput { field: k })
}
fn item(task: TaskRecord, directory: &crate::identity::IdentityDirectory) -> Value {
    let mut result = json!({"id":task.guid,"text":task.summary,"completed":!task.completed_at.is_empty()&&task.completed_at!="0"});
    if let Some(member) = task
        .assignees
        .iter()
        .chain(task.members.iter().filter(|m| m.role == "assignee"))
        .next()
    {
        let mut assignee = json!({});
        if !member.name.is_empty() {
            assignee["name"] = json!(member.name);
        }
        if let Some((id, agent)) = directory.agents.iter().find(|(_, a)| {
            a.app_id == member.id
                || a.open_ids_by_app
                    .values()
                    .any(|open_id| open_id == &member.id)
        }) {
            assignee["agent_id"] = json!(id);
            if !assignee["name"].is_string() && !agent.display_name.is_empty() {
                assignee["name"] = json!(agent.display_name);
            }
        }
        if assignee.as_object().is_some_and(|a| !a.is_empty()) {
            result["assignee"] = assignee;
        }
    }
    result
}
impl InvocationHandler for TasksHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let action = text(&input, "action")?;
            let enabled = crate::agent_tasks::task_system_enabled(self.context.workspace_config());
            if !enabled {
                if action == "list" {
                    return Ok(json!({"enabled":false,"items":[]}));
                }
                return Err(ProviderError::InvalidInput {
                    field: "task_system_enabled",
                });
            }
            let state = self.context.storage().workspace_state().await?;
            // A saved tasklist alone does not grant access to the owner's tasks.
            // Do not treat an unconnected account as a malformed list request.
            for field in ["owner_app_id", "owner_api_base"] {
                if text(&state, field).is_err() {
                    return Err(ProviderError::AuthorizationRequired);
                }
            }
            let directory = crate::identity::IdentityDirectory::from_workspace_state(&state)?;
            let app_id = text(&state, "owner_app_id")?;
            let secret = self
                .context
                .secret_reader()
                .read("feishu_owner_app_secret")
                .await
                .map_err(authorization_error)?;
            let token = self
                .context
                .secret_reader()
                .read("feishu_owner_access_token")
                .await
                .map_err(authorization_error)?;
            let domain = crate::client::Domain::Custom(text(&state, "owner_api_base")?.into());
            let client = Client::new(
                self.transport.clone(),
                Credentials::app(app_id, secret.expose_secret())
                    .with_user_access_token(token.expose_secret()),
            )
            .map_err(error)?
            .with_domain(domain);
            let guid = crate::agent_tasks::join_team_tasklist_as_lead(
                self.context.storage(),
                self.context.workspace_config(),
                &client,
                app_id,
            )
            .await?;
            let guid = guid.as_str();
            match action {
                "list" => {
                    client
                        .subscribe_task_events("open_id", Auth::User)
                        .await
                        .map_err(error)?;
                    let list = client
                        .get_tasklist(guid, "open_id", Auth::User)
                        .await
                        .map_err(error)?;
                    let mut request = ListRequest {
                        task_type: None,
                        ..Default::default()
                    };
                    let mut items = Vec::new();
                    loop {
                        let page = client
                            .list_tasklist_tasks(guid, &request, Auth::User)
                            .await
                            .map_err(error)?;
                        items.extend(page.items.into_iter().map(|task| item(task, &directory)));
                        if !page.has_more {
                            break;
                        }
                        request.page_token = page.next_page_token;
                        if request.page_token.is_none() {
                            break;
                        }
                    }
                    Ok(json!({"enabled":true,"name":list.name,"items":items}))
                }
                "create" => {
                    let task = client
                        .create_task(
                            &CreateTask {
                                summary: text(&input, "text")?.trim().into(),
                                tasklist_guid: Some(guid.into()),
                                assignees: vec![MemberIdentity::user(text(
                                    &state,
                                    "owner_open_id",
                                )?)],
                                ..Default::default()
                            },
                            "open_id",
                            Auth::User,
                        )
                        .await
                        .map_err(error)?;
                    self.context.storage().management_changed().await?;
                    Ok(item(task, &directory))
                }
                "set_completed" => {
                    let id = text(&input, "id")?;
                    let completed = input
                        .get("completed")
                        .and_then(Value::as_bool)
                        .ok_or(ProviderError::InvalidInput { field: "completed" })?;
                    let task = client
                        .get_task(id, "open_id", Auth::User)
                        .await
                        .map_err(error)?;
                    if task.tasklist_guid != guid
                        && !task.tasklists.iter().any(|l| l.tasklist_guid == guid)
                    {
                        return Err(ProviderError::InvalidInput { field: "id" });
                    }
                    let updated = client
                        .update_task(
                            id,
                            &TaskPatch {
                                status: Some(if completed { "completed" } else { "todo" }.into()),
                                ..Default::default()
                            },
                            "open_id",
                            Auth::User,
                        )
                        .await
                        .map_err(error)?;
                    self.context.storage().management_changed().await?;
                    Ok(item(updated, &directory))
                }
                _ => Err(ProviderError::InvalidInput { field: "action" }),
            }
        })
    }
}
