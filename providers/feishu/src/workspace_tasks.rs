use crate::{
    client::{Auth, Client, Credentials},
    dto::TaskRecord,
    http::Transport,
    tasks::ListRequest,
};
use lankide_provider::{InitContext, InvocationHandler, ProviderError, ProviderFuture};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct TasksHandler {
    pub context: InitContext,
    pub transport: Arc<dyn Transport>,
}
fn error(e: crate::ClientError) -> ProviderError {
    e.provider_error(false)
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
        Box::pin(async move { self.run(input).await })
    }
}
impl TasksHandler {
    async fn run(&self, input: Value) -> Result<Value, ProviderError> {
        if text(&input, "action")? != "list" {
            return Err(ProviderError::InvalidInput { field: "action" });
        }
        if !crate::agent_tasks::task_system_enabled(self.context.workspace_config()) {
            return Ok(json!({"enabled":false,"items":[]}));
        }
        let state = self.context.storage().workspace_state().await?;
        let directory = crate::identity::IdentityDirectory::from_workspace_state(&state)?;
        let guid = match state["tasklist_guid"].as_str().filter(|v| !v.is_empty()) {
            Some(guid) => guid,
            None => {
                return Err(ProviderError::InvalidInput {
                    field: "tasklist_guid",
                });
            }
        };
        // The list's creating App is the explicit reader. Never pick the first roster member.
        let app_id = text(&state, "tasklist_reader_app_id")?;
        let mut selected = None;
        for member in self.context.storage().team_members().await? {
            let lankide_provider::ProviderTeamMember::Agent { agent_id } = member else {
                continue;
            };
            let config = self.context.storage().agent_config(&agent_id).await?;
            if config["app_id"] == app_id {
                selected = Some((agent_id, config));
                break;
            }
        }
        let (id, config) = selected.ok_or(ProviderError::NotFound {
            entity: "tasklist_reader",
        })?;
        let secret = self
            .context
            .secret_reader()
            .read(&format!("agent:{id}:app_secret"))
            .await?;
        let client = Client::new(
            self.transport.clone(),
            Credentials::app(app_id, secret.expose_secret()),
        )
        .map_err(error)?
        .with_domain(crate::adapter::configured_domain(
            &config,
            self.context.workspace_config(),
        )?);
        let list = client
            .get_tasklist(guid, "open_id", Auth::Tenant)
            .await
            .map_err(error)?;
        let mut request = ListRequest {
            task_type: None,
            ..Default::default()
        };
        let mut items = Vec::new();
        loop {
            let page = client
                .list_tasklist_tasks(guid, &request, Auth::Tenant)
                .await
                .map_err(error)?;
            items.extend(page.items.into_iter().map(|task| item(task, &directory)));
            if !page.has_more {
                break;
            }
            let next = page.next_page_token.ok_or(ProviderError::InvalidInput {
                field: "page_token",
            })?;
            if request.page_token.as_ref() == Some(&next) {
                return Err(ProviderError::InvalidInput {
                    field: "page_token",
                });
            }
            request.page_token = Some(next);
        }
        Ok(json!({"enabled":true,"name":list.name,"items":items}))
    }
}
