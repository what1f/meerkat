use crate::client::{Auth, Client as FeishuClient};
use crate::dto::{TaskMember, TaskRecord, TasklistRecord};
use crate::http::Transport;
use crate::mention::{MentionTarget, extract_agent_ids, render_card_markdown};
use crate::tasks::{
    Comment, CreateTask, ListRequest, MemberIdentity, TaskMemberInput, TaskPatch, TasklistMember,
};
use lankide_provider::{
    AgentRuntimeContext, InvocationHandler, JsonSchema, ProviderError, ProviderFuture,
    ProviderTeamMember, TeamRole, ToolDefinition,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

type SharedClient = FeishuClient<Arc<dyn Transport>>;

const DEFAULT_TASKLIST_NAME: &str = "团队任务";

const IM_PROMPT: &str = r#"## 飞书 IM

飞书原生表情通常比通用 emoji 更有表现力，优先使用飞书原生表情。需要确认完整表情目录、含义或使用场景时，使用 feishu_reaction_catalog。

- 调侃或玩笑语气：GLANCE（看）、SLIGHT（白眼）、TONGUE（吐舌）、SMIRK（坏笑）。
- 赞同：OK、THUMBSUP、JIAYI（+1）、CheckMark、Yes、SMART、WITTY、PRAISE。
- 不赞同：No、ThumbsDown、CrossMark、MinusOne（-1）。
- 阴阳怪气或无奈：ERROR、NOSEPICK、SPITBLOOD、EMBARRASSED、WAVE、LOOKDOWN、Shrug、SMILE。
- 笑：LOL、BLUSH、TRICK、LAUGH。哭或难受：CRY、SOB、ENOUGH、TEARS。
- 协作：Get（已收到）、THANKS（感谢）、FISTBUMP（碰拳）、DONE（完成）、LGTM（我看行）、OnIt（在做了）、OneSecond（稍等）、YouAreTheBest（称赞）、SALUTE（致意）、Typing（处理中）。
- 其他常用：WHAT、Lemon、FACEPALM、THINKING、ClownFace、SHOCKED、DIZZY、PROUD、YEAH。

能用一个表情完整表达态度时，直接使用 feishu_reaction_add；有额外信息需要补充时，可以先回应表情再发送文字。当 Get 已经承担全部语义时，不要再发一条“已收到”。OK、CrossMark、MinusOne、DONE、LGTM 等更适合在具体消息下直接表态，不要生硬地插入正文。

消息中的提及使用 @名称[id]。需要提及成员时保留完整标记，不要猜测未出现过的 ID。发送普通消息使用 message_send；回复具体消息时增加 reply_to，并确保该消息属于目标会话。复杂长文通过 content_file 提供；本地图片和文件使用 Markdown 图片或链接引用。

把自己当成团队里真实的一员：当前讨论需要你的观点、有人在问你，或你掌握了能推动事情的信息时才发言；只是看到了但插话没有价值时，不发送消息。回应某一句具体消息且引用关系能帮助理解上下文时才使用 reply_to，不要为了形式感给每条消息都加引用。message_send 返回 sent 才表示已发送；返回 held 时没有发出去。"#;

const TASK_PROMPT: &str = r#"## 飞书任务职责

任务系统已启用。使用 feishu_task_list 查看分配给你或你关注的任务，使用 feishu_task_get 读取完整任务及评论。

创建任务时使用 feishu_task_create。你和 Team Lead 会自动成为负责人；assignee_ids 是额外负责人，follower_ids 是可以读取和评论但不负责完成任务的协作者。不要为了让全员看见任务而把所有成员都加入。

只有负责人可以更新或完成任务。使用 feishu_task_update 更新内容，使用 feishu_task_complete 完成任务；至少提供一个需要修改的字段。

需要增加或移除负责人、关注人时，分别使用 feishu_task_member_add、feishu_task_member_remove。成员使用团队成员 ID，角色使用 assignee 或 follower。

任务评论不是 IM 发言。在任务中记录进展或讨论任务时使用 feishu_task_comment；需要分页读取评论时使用 feishu_task_comments。工具失败时，不要假装操作已经完成。"#;

pub(crate) fn task_system_enabled(config: &Value) -> bool {
    config
        .get("task_system_enabled")
        .or_else(|| {
            config
                .get("provider")
                .and_then(|value| value.get("task_system_enabled"))
        })
        .and_then(Value::as_bool)
        .or_else(|| {
            config
                .get("task_system")
                .or_else(|| {
                    config
                        .get("provider")
                        .and_then(|value| value.get("task_system"))
                })
                .and_then(|value| value.get("enabled"))
                .and_then(Value::as_bool)
        })
        .unwrap_or(false)
}

pub(crate) fn system_prompt(context: &AgentRuntimeContext) -> String {
    if task_system_enabled(&context.workspace_config) {
        format!("{IM_PROMPT}\n\n{TASK_PROMPT}")
    } else {
        IM_PROMPT.to_owned()
    }
}

pub(crate) async fn tools(
    context: &AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
) -> Result<Vec<ToolDefinition>, ProviderError> {
    if !task_system_enabled(&context.workspace_config) {
        return Ok(Vec::new());
    }
    let access = TaskAccess::from_context(context, client.clone(), auth).await?;
    [
        TaskOperation::List,
        TaskOperation::Get,
        TaskOperation::Create,
        TaskOperation::Update,
        TaskOperation::AddMembers,
        TaskOperation::RemoveMembers,
        TaskOperation::Complete,
        TaskOperation::Comments,
        TaskOperation::Comment,
    ]
    .into_iter()
    .map(|operation| definition(operation, client.clone(), auth, access.clone()))
    .collect()
}

pub(crate) fn target_agents_for_task(
    directory: &crate::identity::IdentityDirectory,
    client: &SharedClient,
    auth: Auth,
    task_id: &str,
) -> Vec<lankide_core::AgentId> {
    let directory = MemberDirectory::from_identity(directory);
    let Ok(task) = client.get_task(task_id, "open_id", auth) else {
        return Vec::new();
    };
    let member_ids = task_member_ids(&task);
    directory
        .agents
        .into_iter()
        .filter(|(_, app_id)| member_ids.contains(&("app".to_owned(), app_id.clone())))
        .filter_map(|(agent_id, _)| lankide_core::AgentId::new(agent_id).ok())
        .collect()
}

#[derive(Clone, Copy)]
enum TaskOperation {
    List,
    Get,
    Create,
    Update,
    AddMembers,
    RemoveMembers,
    Complete,
    Comments,
    Comment,
}

impl TaskOperation {
    fn id(self) -> &'static str {
        match self {
            Self::List => "feishu_task_list",
            Self::Get => "feishu_task_get",
            Self::Create => "feishu_task_create",
            Self::Update => "feishu_task_update",
            Self::AddMembers => "feishu_task_member_add",
            Self::RemoveMembers => "feishu_task_member_remove",
            Self::Complete => "feishu_task_complete",
            Self::Comments => "feishu_task_comments",
            Self::Comment => "feishu_task_comment",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::List => "查看分配给你或你关注的飞书任务",
            Self::Get => "查看一个飞书任务",
            Self::Create => "创建飞书任务",
            Self::Update => "更新由你负责的飞书任务",
            Self::AddMembers => "增加飞书任务的负责人或关注人",
            Self::RemoveMembers => "移除飞书任务的负责人或关注人",
            Self::Complete => "完成由你负责的飞书任务",
            Self::Comments => "分页查看一个飞书任务的评论",
            Self::Comment => "在一个飞书任务中发表评论",
        }
    }

    fn input_schema(self) -> Value {
        match self {
            Self::List => json!({
                "type":"object",
                "properties":{
                    "completed":{"type":"boolean"},
                    "page_token":{"type":"string"},
                    "page_size":{"type":"integer"}
                }
            }),
            Self::Get | Self::Complete => json!({
                "type":"object",
                "required":["task_id"],
                "properties":{"task_id":{"type":"string"}}
            }),
            Self::Comments => json!({
                "type":"object","required":["task_id"],
                "properties":{"task_id":{"type":"string"},"page_token":{"type":"string"},"page_size":{"type":"integer"}}
            }),
            Self::Comment => json!({
                "type":"object","required":["task_id","content"],
                "properties":{"task_id":{"type":"string"},"content":{"type":"string"},"reply_to_comment_id":{"type":"string"}}
            }),
            Self::Create => json!({
                "type":"object",
                "required":["summary"],
                "properties":{
                    "summary":{"type":"string"},
                    "description":{"type":"string"},
                    "assignee_ids":{"type":"array"},
                    "follower_ids":{"type":"array"},
                    "due":{"type":"object"}
                }
            }),
            Self::Update => json!({
                "type":"object",
                "required":["task_id"],
                "properties":{
                    "task_id":{"type":"string"},
                    "summary":{"type":"string"},
                    "description":{"type":"string"},
                    "due":{"type":"object"}
                }
            }),
            Self::AddMembers | Self::RemoveMembers => json!({
                "type":"object",
                "required":["task_id","member_ids","role"],
                "properties":{
                    "task_id":{"type":"string"},
                    "member_ids":{"type":"array","items":{"type":"string"},"minItems":1},
                    "role":{"type":"string","enum":["assignee","follower"]}
                }
            }),
        }
    }
}

fn definition(
    operation: TaskOperation,
    client: Arc<SharedClient>,
    auth: Auth,
    access: TaskAccess,
) -> Result<ToolDefinition, ProviderError> {
    Ok(ToolDefinition {
        id: operation.id().to_owned(),
        description: operation.description().to_owned(),
        input_schema: JsonSchema::new(operation.input_schema())?,
        output_schema: JsonSchema::new(json!({"type":"object","required":["ok"]}))?,
        handler: Arc::new(TaskHandler {
            operation,
            client,
            auth,
            access,
        }),
    })
}

struct TaskHandler {
    operation: TaskOperation,
    client: Arc<SharedClient>,
    auth: Auth,
    access: TaskAccess,
}

impl InvocationHandler for TaskHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let operation = self.operation;
        let client = self.client.clone();
        let auth = self.auth;
        let access = self.access.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                execute(operation, &client, auth, &access, input)
                    .or_else(|error| Ok(tool_error(&error)))
            })
            .await
            .map_err(|_| ProviderError::External {
                operation: "feishu_task",
                retryable: true,
            })?
        })
    }
}

fn execute(
    operation: TaskOperation,
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: Value,
) -> Result<Value, ProviderError> {
    match operation {
        TaskOperation::List => list_tasks(client, auth, access, &input),
        TaskOperation::Get => get_task(client, auth, access, &input),
        TaskOperation::Create => create_task(client, auth, access, &input),
        TaskOperation::Update => update_task(client, auth, access, &input, false),
        TaskOperation::AddMembers => change_task_members(client, auth, access, &input, true),
        TaskOperation::RemoveMembers => change_task_members(client, auth, access, &input, false),
        TaskOperation::Complete => update_task(client, auth, access, &input, true),
        TaskOperation::Comments => list_comments(client, auth, access, &input),
        TaskOperation::Comment => create_comment(client, auth, access, &input),
    }
}

fn list_tasks(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let request = ListRequest {
        completed: input.get("completed").and_then(Value::as_bool),
        page_token: text(input, "page_token"),
        page_size: input
            .get("page_size")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(50)
            .clamp(1, 100),
        task_type: Some("my_tasks".to_owned()),
        user_id_type: "open_id".to_owned(),
    };
    let page = client
        .list_tasklist_tasks(&access.tasklist_id, &request, auth)
        .map_err(|_| task_error("feishu_task_list", true))?;
    let tasks = page
        .items
        .iter()
        .filter(|task| access.can_read(task))
        .map(|task| task_json(task, access))
        .collect::<Vec<_>>();
    Ok(json!({
        "ok": true,
        "tasks": tasks,
        "next_page_token": page.next_page_token,
        "has_more": page.has_more,
    }))
}

fn get_task(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let task = client
        .get_task(&task_id, "open_id", auth)
        .map_err(|_| task_error("feishu_task_get", false))?;
    if !access.can_read(&task) {
        return Ok(denied(
            "not_task_member",
            "你不是这个任务的负责人或关注人。",
        ));
    }
    let comments = client
        .list_task_comments(&task_id, None, 50, auth)
        .map_err(|_| task_error("feishu_task_comments", true))?;
    Ok(json!({
        "ok":true,
        "task":task_json(&task, access),
        "comments":comments.items.iter().map(|comment| comment_json(comment, &access.directory)).collect::<Vec<_>>(),
        "comments_next_page_token":comments.next_page_token,
        "comments_has_more":comments.has_more,
    }))
}

fn list_comments(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let task = client
        .get_task(&task_id, "open_id", auth)
        .map_err(|_| task_error("feishu_task_get", false))?;
    if !access.can_read(&task) {
        return Ok(denied(
            "not_task_member",
            "你不是这个任务的负责人或关注人。",
        ));
    }
    let page = client
        .list_task_comments(
            &task_id,
            input.get("page_token").and_then(Value::as_str),
            input.get("page_size").and_then(Value::as_u64).unwrap_or(50) as usize,
            auth,
        )
        .map_err(|_| task_error("feishu_task_comments", true))?;
    Ok(json!({
        "ok":true,
        "comments":page.items.iter().map(|comment| comment_json(comment, &access.directory)).collect::<Vec<_>>(),
        "next_page_token":page.next_page_token,
        "has_more":page.has_more,
    }))
}

fn create_comment(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let task = client
        .get_task(&task_id, "open_id", auth)
        .map_err(|_| task_error("feishu_task_get", false))?;
    if !access.can_read(&task) {
        return Ok(denied(
            "not_task_member",
            "你不是这个任务的负责人或关注人。",
        ));
    }
    let comment_id = client
        .create_task_comment(
            &Comment {
                task_guid: task_id,
                content: access.render_comment(&required_text(input, "content")?)?,
                reply_to_comment_id: text(input, "reply_to_comment_id"),
            },
            "open_id",
            auth,
        )
        .map_err(|_| task_error("feishu_task_comment", false))?;
    Ok(json!({"ok":true,"comment_id":comment_id}))
}

fn create_task(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let summary = required_text(input, "summary")?;
    let mut assignee_ids = vec![access.lead_id.clone(), access.agent_id.clone()];
    assignee_ids.extend(string_array(input, "assignee_ids")?);
    deduplicate(&mut assignee_ids);
    let mut follower_ids = string_array(input, "follower_ids")?;
    deduplicate(&mut follower_ids);
    if let Some(duplicate) = assignee_ids
        .iter()
        .find(|id| follower_ids.iter().any(|follower| follower == *id))
    {
        return Ok(denied(
            "duplicate_task_role",
            &format!("成员 {duplicate} 不能同时是负责人和关注人。"),
        ));
    }
    let assignees = access.directory.resolve_members(&assignee_ids)?;
    let followers = access.directory.resolve_members(&follower_ids)?;
    let task = CreateTask {
        summary,
        description: text(input, "description").unwrap_or_default(),
        due: input.get("due").cloned(),
        assignee_ids: Vec::new(),
        collaborator_ids: Vec::new(),
        assignees,
        followers,
        tasklist_guid: Some(access.tasklist_id.clone()),
        client_token: Some(uuid::Uuid::new_v4().to_string()),
        extra: Value::Null,
    };
    let created = client
        .create_task(&task, "open_id", auth)
        .map_err(|_| task_error("feishu_task_create", false))?;
    let verified = client
        .get_task(&created.guid, "open_id", auth)
        .map_err(|_| task_error("feishu_task_verify", true))?;
    if !task_contains_members(&verified, &task.assignees, true)
        || !task_contains_members(&verified, &task.followers, false)
        || !task_in_tasklist(&verified, &access.tasklist_id)
    {
        return Err(task_error("feishu_task_verify", false));
    }
    Ok(json!({"ok":true,"task":task_json(&verified, access)}))
}

fn update_task(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
    complete: bool,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let current = client
        .get_task(&task_id, "open_id", auth)
        .map_err(|_| task_error("feishu_task_get", false))?;
    if !access.can_read(&current) {
        return Ok(denied(
            "not_task_member",
            "你不是这个任务的负责人或关注人。",
        ));
    }
    if !access.can_write(&current) {
        return Ok(denied(
            "not_assignee",
            "只有任务负责人可以更新或完成这个任务。",
        ));
    }
    let patch = TaskPatch {
        summary: (!complete).then(|| text(input, "summary")).flatten(),
        description: (!complete).then(|| text(input, "description")).flatten(),
        status: complete.then(|| "completed".to_owned()),
        due: (!complete).then(|| input.get("due").cloned()).flatten(),
        extra: Value::Null,
    };
    if !complete && patch.is_empty() {
        return Ok(denied("empty_update", "请提供要更新的任务内容。"));
    }
    if complete && is_completed(&current) {
        return Ok(json!({"ok":true,"task":task_json(&current, access)}));
    }
    let updated = client
        .update_task(&task_id, &patch, "open_id", auth)
        .map_err(|_| task_error("feishu_task_update", false))?;
    Ok(json!({"ok":true,"task":task_json(&updated, access)}))
}

fn change_task_members(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
    add: bool,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let role = required_text(input, "role")?;
    let assignee_role = match role.as_str() {
        "assignee" => true,
        "follower" => false,
        _ => return Err(ProviderError::InvalidInput { field: "role" }),
    };
    let mut member_ids = string_array(input, "member_ids")?;
    deduplicate(&mut member_ids);
    if member_ids.is_empty() {
        return Err(ProviderError::InvalidInput {
            field: "member_ids",
        });
    }
    let current = client
        .get_task(&task_id, "open_id", auth)
        .map_err(|_| task_error("feishu_task_get", false))?;
    if !access.can_read(&current) {
        return Ok(denied(
            "not_task_member",
            "你不是这个任务的负责人或关注人。",
        ));
    }
    if !access.can_write(&current) {
        return Ok(denied("not_assignee", "只有任务负责人可以调整任务成员。"));
    }
    let identities = access.directory.resolve_members(&member_ids)?;
    if add
        && let Some((member_id, _)) = member_ids
            .iter()
            .zip(&identities)
            .find(|(_, identity)| task_contains_member(&current, identity, !assignee_role))
    {
        return Ok(denied(
            "duplicate_task_role",
            &format!("成员 {member_id} 已经是任务的另一个角色。"),
        ));
    }
    let changes = identities
        .into_iter()
        .filter(|identity| task_contains_member(&current, identity, assignee_role) != add)
        .map(|identity| {
            if assignee_role {
                TaskMemberInput::assignee(identity)
            } else {
                TaskMemberInput::follower(identity)
            }
        })
        .collect::<Vec<_>>();
    if changes.is_empty() {
        return Ok(json!({"ok":true,"task":task_json(&current, access)}));
    }
    let updated = if add {
        client.add_task_members(&task_id, &changes, "open_id", auth)
    } else {
        client.remove_task_members(&task_id, &changes, "open_id", auth)
    }
    .map_err(|_| {
        task_error(
            if add {
                "feishu_task_member_add"
            } else {
                "feishu_task_member_remove"
            },
            false,
        )
    })?;
    Ok(json!({"ok":true,"task":task_json(&updated, access)}))
}

#[derive(Clone)]
struct TaskAccess {
    agent_id: String,
    app_id: String,
    lead_id: String,
    tasklist_id: String,
    directory: MemberDirectory,
}

impl TaskAccess {
    async fn from_context(
        context: &AgentRuntimeContext,
        client: Arc<SharedClient>,
        auth: Auth,
    ) -> Result<Self, ProviderError> {
        let (mut workspace_state, identity_directory) =
            crate::identity::load(context.storage()).await?;
        let app_id = identity_directory
            .agent_app_id(&context.agent_id)
            .map(str::to_owned)
            .ok_or(ProviderError::InvalidInput {
                field: "task_agent_app_id",
            })?;
        let team_members = context.storage().team_members().await?;
        let lead_id = team_members
            .iter()
            .filter_map(|member| match member {
                ProviderTeamMember::Human(person) => Some(person),
                ProviderTeamMember::Agent { .. } => None,
            })
            .find(|person| person.team_role == TeamRole::Lead)
            .map(|person| person.person_id.to_string())
            .ok_or(ProviderError::NotFound {
                entity: "team_lead",
            })?;
        let directory = MemberDirectory::from_roster(
            &identity_directory,
            &team_members,
            &app_id,
            context.storage(),
            client.clone(),
            auth,
        )
        .await?;
        let saved_tasklist_id = text(&workspace_state, "tasklist_guid").or_else(|| {
            workspace_state
                .get("task_system")
                .and_then(|value| text(value, "tasklist_guid"))
        });
        let configured_name = tasklist_name(&context.workspace_config);
        let desired_members = directory.tasklist_members();
        let tasklist_id = ensure_tasklist(
            client,
            auth,
            saved_tasklist_id.as_deref(),
            configured_name.as_deref(),
            &desired_members,
        )
        .await?;
        if saved_tasklist_id.as_deref() != Some(tasklist_id.as_str()) {
            workspace_state["tasklist_guid"] = Value::String(tasklist_id.clone());
            context
                .storage()
                .put_workspace_state(workspace_state)
                .await?;
        }
        Ok(Self {
            agent_id: context.agent_id.to_string(),
            app_id,
            lead_id,
            tasklist_id,
            directory,
        })
    }

    fn can_read(&self, task: &TaskRecord) -> bool {
        task_has_member(task, &self.app_id, false) || task_has_member(task, &self.app_id, true)
    }

    fn can_write(&self, task: &TaskRecord) -> bool {
        task_has_member(task, &self.app_id, true)
    }

    fn render_comment(&self, content: &str) -> Result<String, ProviderError> {
        if native_at_start(content).is_some() {
            return Err(ProviderError::InvalidInput {
                field: "content: 提及请使用 @名称[成员ID]",
            });
        }
        let ids = extract_agent_ids(content);
        let targets =
            ids.iter()
                .map(|id| {
                    self.directory.mention_targets.get(id).cloned().ok_or(
                        ProviderError::InvalidInput {
                            field: "task_comment_mention",
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
        Ok(render_card_markdown(content, &targets).0)
    }
}

#[derive(Clone, Default)]
struct MemberDirectory {
    agents: BTreeMap<String, String>,
    humans: BTreeMap<String, String>,
    mention_targets: BTreeMap<String, MentionTarget>,
    inbound_mentions: BTreeMap<(String, String), MentionTarget>,
    current_app_id: String,
}

impl MemberDirectory {
    fn from_identity(identity: &crate::identity::IdentityDirectory) -> Self {
        let mut directory = Self::default();
        for (id, identity) in &identity.agents {
            if !identity.app_id.trim().is_empty() {
                directory.agents.insert(id.clone(), identity.app_id.clone());
            }
        }
        directory
    }

    async fn from_roster(
        identity: &crate::identity::IdentityDirectory,
        team_members: &[ProviderTeamMember],
        current_app_id: &str,
        storage: &dyn lankide_provider::ProviderStorage,
        client: Arc<SharedClient>,
        auth: Auth,
    ) -> Result<Self, ProviderError> {
        let mut directory = Self::default();
        directory.current_app_id = current_app_id.to_owned();
        for member in team_members {
            let ProviderTeamMember::Agent { agent_id } = member else {
                continue;
            };
            let Some(agent) = identity.agents.get(agent_id.as_str()) else {
                continue;
            };
            for (app_id, open_id) in &agent.open_ids_by_app {
                directory.inbound_mentions.insert(
                    (app_id.clone(), open_id.clone()),
                    MentionTarget::new(agent_id.as_str(), open_id, &agent.display_name),
                );
            }
            if !agent.app_id.trim().is_empty() {
                directory
                    .agents
                    .insert(agent_id.to_string(), agent.app_id.clone());
            }
            if let Some(open_id) = agent
                .open_ids_by_app
                .get(current_app_id)
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                directory.mention_targets.insert(
                    agent_id.to_string(),
                    MentionTarget::new(agent_id.as_str(), open_id, &agent.display_name),
                );
            }
        }
        for person in team_members.iter().filter_map(|member| match member {
            ProviderTeamMember::Human(person) => Some(person),
            ProviderTeamMember::Agent { .. } => None,
        }) {
            let open_id = crate::identity::person_open_id(
                storage,
                person.clone(),
                current_app_id,
                client.clone(),
                auth,
            )
            .await?;
            let person_id = person.person_id.to_string();
            if let Some(aliases) = person
                .provider_state
                .get("open_ids_by_app")
                .and_then(Value::as_object)
            {
                for (app_id, value) in aliases {
                    if let Some(open_id) = value.as_str() {
                        directory.inbound_mentions.insert(
                            (app_id.clone(), open_id.to_owned()),
                            MentionTarget::new(&person_id, open_id, "成员"),
                        );
                    }
                }
            }
            directory.inbound_mentions.insert(
                (current_app_id.to_owned(), open_id.clone()),
                MentionTarget::new(&person_id, &open_id, "成员"),
            );
            directory.humans.insert(person_id.clone(), open_id.clone());
            directory.mention_targets.insert(
                person_id.clone(),
                MentionTarget::new(person_id, open_id, ""),
            );
        }
        Ok(directory)
    }

    fn tasklist_members(&self) -> Vec<TasklistMember> {
        self.agents
            .values()
            .cloned()
            .map(MemberIdentity::app)
            .map(TasklistMember::editor)
            .chain(
                self.humans
                    .values()
                    .cloned()
                    .map(MemberIdentity::user)
                    .map(TasklistMember::viewer),
            )
            .collect()
    }

    fn resolve_members(&self, ids: &[String]) -> Result<Vec<MemberIdentity>, ProviderError> {
        ids.iter()
            .map(|id| {
                if let Some(app_id) = self.agents.get(id) {
                    return Ok(MemberIdentity::app(app_id.clone()));
                }
                if let Some(open_id) = self.humans.get(id) {
                    return Ok(MemberIdentity::user(open_id.clone()));
                }
                Err(ProviderError::InvalidInput {
                    field: "task_member_id",
                })
            })
            .collect()
    }

    fn stable_id(&self, member: &TaskMember) -> Option<&str> {
        match normalized_member_type(member) {
            "app" => self
                .agents
                .iter()
                .find_map(|(id, external)| (external == &member.id).then_some(id.as_str())),
            "user" => self
                .humans
                .iter()
                .find_map(|(id, external)| (external == &member.id).then_some(id.as_str())),
            _ => None,
        }
    }
}

async fn ensure_tasklist(
    client: Arc<SharedClient>,
    auth: Auth,
    saved_guid: Option<&str>,
    configured_name: Option<&str>,
    desired_members: &[TasklistMember],
) -> Result<String, ProviderError> {
    let saved_guid = saved_guid.map(str::to_owned);
    let configured_name = configured_name.map(str::to_owned);
    let desired_members = desired_members.to_vec();
    tokio::task::spawn_blocking(move || {
        let saved_tasklist = if let Some(guid) = saved_guid
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            client.get_tasklist(guid, "open_id", auth).ok()
        } else {
            None
        };
        let mut tasklist = if let Some(tasklist) = saved_tasklist {
            tasklist
        } else {
            // A replacement Agent App cannot read a Tasklist owned only by a
            // deleted App. Discover or create the Team Tasklist available to
            // the current verified App and persist that GUID through the
            // caller, instead of making IM execution unavailable as well.
            let name = configured_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(DEFAULT_TASKLIST_NAME);
            let mut matches = Vec::new();
            let mut page_token = None;
            loop {
                let page = client
                    .list_tasklists(page_token.as_deref(), 100, "open_id", auth)
                    .map_err(|_| task_error("feishu_tasklist_list", true))?;
                matches.extend(
                    page.items
                        .into_iter()
                        .filter(|item| item.name.trim() == name),
                );
                if !page.has_more {
                    break;
                }
                let next = page
                    .next_page_token
                    .ok_or_else(|| task_error("feishu_tasklist_list", false))?;
                if page_token.as_deref() == Some(next.as_str()) {
                    return Err(task_error("feishu_tasklist_list", false));
                }
                page_token = Some(next);
            }
            match matches.len() {
                0 => client
                    .create_tasklist(name, &desired_members, "open_id", auth)
                    .map_err(|_| task_error("feishu_tasklist_create", false))?,
                1 => matches.remove(0),
                _ => return Err(task_error("feishu_tasklist_ambiguous", false)),
            }
        };
        if let Some(name) = configured_name
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            && tasklist.name.trim() != name
        {
            tasklist = client
                .patch_tasklist(&tasklist.guid, Some(name), "open_id", auth)
                .map_err(|_| task_error("feishu_tasklist_update", false))?;
        }
        if tasklist.guid.trim().is_empty() {
            return Err(task_error("feishu_tasklist_get", false));
        }
        let missing = missing_tasklist_members(&tasklist, &desired_members);
        if !missing.is_empty() {
            client
                .add_tasklist_members(&tasklist.guid, &missing, "open_id", auth)
                .map_err(|_| task_error("feishu_tasklist_members", false))?;
        }
        Ok(tasklist.guid)
    })
    .await
    .map_err(|_| task_error("feishu_tasklist", true))?
}

fn missing_tasklist_members(
    tasklist: &TasklistRecord,
    desired: &[TasklistMember],
) -> Vec<TasklistMember> {
    let mut current = BTreeMap::new();
    if let Some(owner) = &tasklist.owner {
        current.insert(
            (normalized_member_type(owner).to_owned(), owner.id.clone()),
            "owner".to_owned(),
        );
    }
    for member in &tasklist.members {
        current.insert(
            (normalized_member_type(member).to_owned(), member.id.clone()),
            member.role.clone(),
        );
    }
    desired
        .iter()
        .filter(|member| {
            let role = current.get(&(
                member.identity_type.trim().to_owned(),
                member.external_id.trim().to_owned(),
            ));
            !matches!(role.map(String::as_str), Some("owner"))
                && role.map(String::as_str) != Some(member.role.as_str())
                && !(member.role == "viewer" && matches!(role.map(String::as_str), Some("editor")))
        })
        .cloned()
        .collect()
}

fn tasklist_name(config: &Value) -> Option<String> {
    text(config, "task_list_name")
        .or_else(|| {
            config
                .get("provider")
                .and_then(|value| text(value, "task_list_name"))
        })
        .or_else(|| {
            config
                .get("task_system")
                .and_then(|value| text(value, "name"))
        })
}

fn task_contains_members(
    task: &TaskRecord,
    expected: &[MemberIdentity],
    assignee_role: bool,
) -> bool {
    expected.iter().all(|identity| {
        let mut members: Box<dyn Iterator<Item = &TaskMember>> = if assignee_role {
            Box::new(assignees(task))
        } else {
            Box::new(followers(task))
        };
        members.any(|member| {
            normalized_member_type(member) == identity.identity_type
                && member.id == identity.external_id
        })
    })
}

fn task_contains_member(task: &TaskRecord, expected: &MemberIdentity, assignee_role: bool) -> bool {
    let mut members: Box<dyn Iterator<Item = &TaskMember>> = if assignee_role {
        Box::new(assignees(task))
    } else {
        Box::new(followers(task))
    };
    members.any(|member| {
        normalized_member_type(member) == expected.identity_type
            && member.id == expected.external_id
    })
}

fn task_in_tasklist(task: &TaskRecord, tasklist_id: &str) -> bool {
    task.tasklist_guid == tasklist_id
        || task
            .tasklists
            .iter()
            .any(|tasklist| tasklist.tasklist_guid == tasklist_id)
}

fn deduplicate(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn task_json(task: &TaskRecord, access: &TaskAccess) -> Value {
    let assignee_ids = assignees(task)
        .filter_map(|member| access.directory.stable_id(member).map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let follower_ids = followers(task)
        .filter_map(|member| access.directory.stable_id(member).map(str::to_owned))
        .collect::<BTreeSet<_>>();
    json!({
        "task_id": task.guid,
        "summary": task.summary,
        "description": task.description,
        "status": if is_completed(task) { "completed" } else { "open" },
        "due": task.due,
        "assignee_ids": assignee_ids,
        "follower_ids": follower_ids,
        "can_update": access.can_write(task),
    })
}

fn comment_json(comment: &crate::dto::TaskComment, directory: &MemberDirectory) -> Value {
    let creator = comment.creator.as_ref().and_then(|member| {
        directory.stable_id(member).map(|id| {
            json!({
                "type": normalized_member_type(member),
                "id": id,
            })
        })
    });
    let app_id = comment
        .creator
        .as_ref()
        .filter(|member| normalized_member_type(member) == "app")
        .map(|member| member.id.as_str())
        .unwrap_or(&directory.current_app_id);
    json!({
        "comment_id":comment.comment_id,
        "content":normalize_comment_mentions(&comment.content, app_id, directory),
        "creator":creator,
        "created_at":comment.create_time,
    })
}

fn native_at_start(content: &str) -> Option<usize> {
    content
        .to_ascii_lowercase()
        .match_indices("<at")
        .find_map(|(index, _)| {
            content
                .as_bytes()
                .get(index + 3)
                .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/'))
                .then_some(index)
        })
}

// Task comments retain the author's App-scoped mention IDs on readback.
fn normalize_comment_mentions(content: &str, app_id: &str, directory: &MemberDirectory) -> String {
    let mut remaining = content;
    let mut result = String::new();
    while let Some(start) = native_at_start(remaining) {
        result.push_str(&remaining[..start]);
        let tag = &remaining[start..];
        let Some(end) = tag.find('>') else {
            result.push_str("@未识别成员");
            return result;
        };
        let attributes = tag[3..end].trim();
        let id = attributes
            .strip_prefix("id")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .map(str::trim)
            .map(|value| value.trim_matches(['\"', '\'']));
        let target = id.and_then(|id| {
            directory
                .inbound_mentions
                .get(&(app_id.to_owned(), id.to_owned()))
        });
        let marker = target.and_then(|target| {
            let name = if target.display_name.trim().is_empty() {
                "成员"
            } else {
                &target.display_name
            };
            lankide_core::markdown_mention(name, &target.agent_id)
        });
        result.push_str(marker.as_deref().unwrap_or("@未识别成员"));
        let tail = &tag[end + 1..];
        remaining = if let Some(close) = tail.to_ascii_lowercase().find("</at>") {
            &tail[close + 5..]
        } else {
            tail
        };
    }
    result.push_str(remaining);
    result
}

fn task_has_member(task: &TaskRecord, app_id: &str, assignee_only: bool) -> bool {
    let matches =
        |member: &TaskMember| normalized_member_type(member) == "app" && member.id == app_id;
    if assignees(task).any(matches) {
        return true;
    }
    !assignee_only && followers(task).any(matches)
}

fn assignees(task: &TaskRecord) -> impl Iterator<Item = &TaskMember> {
    task.assignees.iter().chain(
        task.members
            .iter()
            .filter(|member| member.role == "assignee"),
    )
}

fn followers(task: &TaskRecord) -> impl Iterator<Item = &TaskMember> {
    task.collaborators.iter().chain(
        task.members
            .iter()
            .filter(|member| matches!(member.role.as_str(), "follower" | "collaborator")),
    )
}

fn task_member_ids(task: &TaskRecord) -> BTreeSet<(String, String)> {
    assignees(task)
        .chain(followers(task))
        .map(|member| (normalized_member_type(member).to_owned(), member.id.clone()))
        .collect()
}

fn normalized_member_type(member: &TaskMember) -> &str {
    match member.id_type.as_str() {
        "app" | "APP" => "app",
        "user" | "USER" => "user",
        value => value,
    }
}

fn is_completed(task: &TaskRecord) -> bool {
    (!task.completed_at.trim().is_empty() && task.completed_at != "0")
        || matches!(task.status.as_str(), "completed" | "done")
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_text(value: &Value, key: &'static str) -> Result<String, ProviderError> {
    text(value, key).ok_or(ProviderError::InvalidInput { field: key })
}

fn string_array(value: &Value, key: &'static str) -> Result<Vec<String>, ProviderError> {
    let Some(values) = value.get(key) else {
        return Ok(Vec::new());
    };
    values
        .as_array()
        .ok_or(ProviderError::InvalidInput { field: key })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or(ProviderError::InvalidInput { field: key })
        })
        .collect()
}

fn denied(code: &str, message: &str) -> Value {
    json!({"ok":false,"error":{"code":code,"message":message}})
}

fn tool_error(error: &ProviderError) -> Value {
    let message = match error {
        ProviderError::InvalidInput { .. } | ProviderError::SchemaMismatch { .. } => {
            "任务参数无效。"
        }
        ProviderError::NotFound { .. } => "没有找到这个任务。",
        _ => "飞书任务操作未完成，请稍后重试。",
    };
    denied(error.code(), message)
}

fn task_error(operation: &'static str, retryable: bool) -> ProviderError {
    ProviderError::External {
        operation,
        retryable,
    }
}
