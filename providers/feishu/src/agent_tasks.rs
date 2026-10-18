use crate::client::{Auth, Client as FeishuClient};
use crate::dto::{TaskMember, TaskRecord, TasklistRecord};
use crate::http::{ClientError, Transport};
use crate::mention::{
    BARE_MENTION, MentionTarget, extract_agent_ids, has_bare_mention, render_card_markdown,
};
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

pub(crate) type SharedClient = FeishuClient<Arc<dyn Transport>>;

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

消息中的提及使用 @名称[id]。需要提及成员时保留完整标记，不要猜测未出现过的 ID。发送普通消息使用 message_send；向人类团队成员发起私信时提供 recipient_id（成员 ID），不需要已有会话。回复具体消息时增加 reply_to，并确保该消息属于目标会话。复杂长文通过 content_file 提供；本地图片和文件先用 lankide upload --file PATH 上传，再将返回的完整资源对象放入 message_send 的 attachments 数组（resource_id、file_name、media_type、size_bytes、sha256）。Markdown 本地路径不支持。

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
        .and_then(Value::as_bool)
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
    action_lock: Arc<tokio::sync::Mutex<()>>,
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
    .map(|operation| {
        definition(
            operation,
            client.clone(),
            auth,
            access.clone(),
            context.clone(),
            action_lock.clone(),
        )
    })
    .collect()
}

pub(crate) async fn task_event(
    context: &lankide_provider::InitContext,
    client: Arc<SharedClient>,
    auth: Auth,
    event: &crate::normalize::NormalizedTaskEvent,
) -> Result<Option<(lankide_provider::EventSender, lankide_provider::CustomEvent)>, ProviderError> {
    use lankide_provider::{CustomEvent, EventSender, EventSenderType};
    let (_, identity) = crate::identity::load(context.storage()).await?;
    let raw: Value =
        serde_json::from_slice(&event.raw).map_err(|_| task_error("read_task_event", false))?;
    let app_id = raw["header"]["app_id"].as_str().unwrap_or_default();
    let roster = context.storage().team_members().await?;
    let directory = MemberDirectory::from_roster(
        &identity,
        &roster,
        app_id,
        context.storage(),
        client.clone(),
        auth,
    )
    .await?;
    let task = match client.get_task(&event.task_id, "open_id", auth).await {
        Ok(task) => task,
        // Task events reach every subscribed App, but an App can only read
        // tasks it may access. Those events are not addressed to it.
        Err(ClientError::Api {
            code: 1470403 | 1470404,
            ..
        }) => return Ok(None),
        Err(error) => return Err(task_error("read_task_event", error.is_retryable())),
    };
    let mut targets = directory
        .agents
        .iter()
        .filter(|(_, app)| task_member_ids(&task).contains(&("app".to_owned(), (*app).clone())))
        .filter_map(|(id, _)| lankide_core::AgentId::new(id).ok())
        .collect::<Vec<_>>();
    let action = event
        .event_types
        .iter()
        .map(|kind| match kind.as_str() {
            "task_create" => "创建了任务",
            "task_completed_update" => {
                if is_completed(&task) {
                    "完成了任务"
                } else {
                    "重新打开了任务"
                }
            }
            "task_assignees_update" => "更新了负责人",
            "task_followers_update" => "更新了关注人",
            "task_comment_create" | "task_comment_reply" => "评论了任务",
            "task_comment_update" => "更新了评论",
            "task_comment_delete" => "删除了评论",
            "task_deleted" => "删除了任务",
            "task_summary_update" => "更新了标题",
            "task_desc_update" => "更新了描述",
            "task_start_due_update" => "更新了日期",
            _ => "更新了任务",
        })
        .collect::<Vec<_>>()
        .join("、");
    let key = event
        .comment_id
        .as_ref()
        .map(|id| format!("task_comment/{id}"))
        .unwrap_or_else(|| format!("task_action/{}", event.task_id));
    let local = context
        .storage()
        .get(&key)
        .await?
        .map(|bytes| serde_json::from_slice::<Value>(&bytes))
        .transpose()
        .map_err(|_| task_error("read_task_action", false))?;
    let comment_created = event
        .event_types
        .iter()
        .any(|kind| matches!(kind.as_str(), "task_comment_create" | "task_comment_reply"));
    let local = local.filter(|value| {
        if event.comment_id.is_some() {
            return comment_created;
        }
        let version = value["updated_at"]
            .as_str()
            .and_then(|value| value.parse::<i64>().ok());
        let event_time = raw["header"]["create_time"]
            .as_str()
            .and_then(|value| value.parse::<i64>().ok());
        value["updated_at"].as_str() == Some(task.updated_at.as_str())
            && version
                .zip(event_time)
                .is_some_and(|(version, time)| time >= version)
    });
    let comment = if let Some(id) = &event.comment_id {
        if event
            .event_types
            .iter()
            .any(|kind| kind == "task_comment_delete")
        {
            None
        } else {
            Some(
                client
                    .get_task_comment(id, auth)
                    .await
                    .map_err(|_| task_error("read_task_comment", true))?,
            )
        }
    } else {
        None
    };
    let member = comment
        .as_ref()
        .filter(|_| comment_created)
        .and_then(|comment| comment.creator.as_ref())
        .or_else(|| {
            event
                .event_types
                .iter()
                .any(|kind| kind == "task_create")
                .then_some(task.creator.as_ref())
                .flatten()
        });
    // A creator is authoritative for creation, but is not the actor of later edits.
    let actor_id = member
        .and_then(|member| directory.stable_id(member))
        .or_else(|| local.as_ref().and_then(|value| value["actor_id"].as_str()));
    let sender = if let Some(id) = actor_id {
        let agent = identity.agents.get(id);
        EventSender {
            id: id.to_owned(),
            kind: if agent.is_some() {
                EventSenderType::Agent
            } else {
                EventSenderType::Human
            },
            name: agent
                .map(|agent| agent.display_name.clone())
                .filter(|name| !name.is_empty())
                .or_else(|| {
                    member
                        .map(|member| member.name.clone())
                        .filter(|name| !name.is_empty())
                })
                .unwrap_or_else(|| id.to_owned()),
        }
    } else {
        EventSender {
            id: "feishu:task".into(),
            kind: EventSenderType::System,
            name: "飞书任务".into(),
        }
    };
    let content = local
        .as_ref()
        .and_then(|value| value["content"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            comment
                .as_ref()
                .map(|comment| {
                    let author_app = comment
                        .creator
                        .as_ref()
                        .filter(|member| normalized_member_type(member) == "app")
                        .map(|member| member.id.as_str())
                        .unwrap_or(app_id);
                    normalize_comment_mentions(&comment.content, author_app, &directory)
                })
                .unwrap_or_else(|| event.content.clone())
        });
    let mentioned_agent_ids = extract_agent_ids(&content)
        .into_iter()
        .filter(|id| identity.agents.contains_key(id))
        .filter_map(|id| lankide_core::AgentId::new(id).ok())
        .filter(|id| id.as_str() != sender.id)
        .collect::<Vec<_>>();
    for id in &mentioned_agent_ids {
        if !targets.contains(id) {
            targets.push(id.clone());
        }
    }
    targets.retain(|id| id.as_str() != sender.id);
    let mut title = format!("{}{}：{}", sender.name, action, task.summary);
    if !content.trim().is_empty() {
        title.push_str("\n");
        title.push_str(&content);
    }
    Ok(Some((
        sender,
        CustomEvent {
            reason: "task".into(),
            target_agent_ids: targets,
            mentioned_agent_ids,
            subject_id: event.task_id.clone(),
            subject_name: title,
            data: serde_json::to_value(event).map_err(|_| task_error("read_task_event", false))?,
        },
    )))
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
                    "due":due_schema()
                }
            }),
            Self::Update => json!({
                "type":"object",
                "required":["task_id"],
                "properties":{
                    "task_id":{"type":"string"},
                    "summary":{"type":"string"},
                    "description":{"type":"string"},
                    "due":due_schema()
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
    context: AgentRuntimeContext,
    action_lock: Arc<tokio::sync::Mutex<()>>,
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
            context,
            action_lock,
        }),
    })
}

struct TaskHandler {
    context: AgentRuntimeContext,
    action_lock: Arc<tokio::sync::Mutex<()>>,
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
            let _guard = self.action_lock.lock().await;
            let result = execute(operation, &client, auth, &access, input.clone())
                .await
                .or_else(|error| Ok(tool_error(&error)))?;
            if result["ok"] == true
                && matches!(
                    operation,
                    TaskOperation::Create
                        | TaskOperation::Update
                        | TaskOperation::AddMembers
                        | TaskOperation::RemoveMembers
                        | TaskOperation::Complete
                        | TaskOperation::Comment
                )
            {
                let (key, updated_at) = if let Some(id) = result["comment_id"].as_str() {
                    (Some(format!("task_comment/{id}")), Value::Null)
                } else if let Some(id) = result["task"]["task_id"].as_str() {
                    (
                        Some(format!("task_action/{id}")),
                        result["task"]["updated_at"].clone(),
                    )
                } else {
                    (None, Value::Null)
                };
                if let Some(key) = key {
                    self.context
                        .storage()
                        .put(
                            &key,
                            serde_json::to_vec(&json!({
                                "actor_id": self.context.agent_id, "updated_at": updated_at,
                                "content":input.get("content"), "operation":operation.id()
                            }))
                            .map_err(|_| task_error("record_task_action", false))?,
                        )
                        .await?;
                }
            }
            Ok(result)
        })
    }
}

async fn execute(
    operation: TaskOperation,
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: Value,
) -> Result<Value, ProviderError> {
    match operation {
        TaskOperation::List => list_tasks(client, auth, access, &input).await,
        TaskOperation::Get => get_task(client, auth, access, &input).await,
        TaskOperation::Create => create_task(client, auth, access, &input).await,
        TaskOperation::Update => update_task(client, auth, access, &input, false).await,
        TaskOperation::AddMembers => change_task_members(client, auth, access, &input, true).await,
        TaskOperation::RemoveMembers => {
            change_task_members(client, auth, access, &input, false).await
        }
        TaskOperation::Complete => update_task(client, auth, access, &input, true).await,
        TaskOperation::Comments => list_comments(client, auth, access, &input).await,
        TaskOperation::Comment => create_comment(client, auth, access, &input).await,
    }
}

async fn list_tasks(
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
        .await
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

async fn get_task(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let task = client
        .get_task(&task_id, "open_id", auth)
        .await
        .map_err(|_| task_error("feishu_task_get", false))?;
    if !access.can_read(&task) {
        return Ok(denied(
            "not_task_member",
            "你不是这个任务的负责人或关注人。",
        ));
    }
    let comments = client
        .list_task_comments(&task_id, None, 50, auth)
        .await
        .map_err(|_| task_error("feishu_task_comments", true))?;
    Ok(json!({
        "ok":true,
        "task":task_json(&task, access),
        "comments":comments.items.iter().map(|comment| comment_json(comment, &access.directory)).collect::<Vec<_>>(),
        "comments_next_page_token":comments.next_page_token,
        "comments_has_more":comments.has_more,
    }))
}

async fn list_comments(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let task = client
        .get_task(&task_id, "open_id", auth)
        .await
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
        .await
        .map_err(|_| task_error("feishu_task_comments", true))?;
    Ok(json!({
        "ok":true,
        "comments":page.items.iter().map(|comment| comment_json(comment, &access.directory)).collect::<Vec<_>>(),
        "next_page_token":page.next_page_token,
        "has_more":page.has_more,
    }))
}

async fn create_comment(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let task = client
        .get_task(&task_id, "open_id", auth)
        .await
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
        .await
        .map_err(|_| task_error("feishu_task_comment", false))?;
    Ok(json!({"ok":true,"comment_id":comment_id}))
}

async fn create_task(
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
        due: normalized_due(input)?,
        assignees,
        followers,
        tasklist_guid: Some(access.tasklist_id.clone()),
        client_token: Some(uuid::Uuid::new_v4().to_string()),
        extra: Value::Null,
    };
    let created = client
        .create_task(&task, "open_id", auth)
        .await
        .map_err(|_| task_error("feishu_task_create", false))?;
    let verified = client
        .get_task(&created.guid, "open_id", auth)
        .await
        .map_err(|_| task_error("feishu_task_verify", true))?;
    if !task_contains_members(&verified, &task.assignees, true)
        || !task_contains_members(&verified, &task.followers, false)
        || !task_in_tasklist(&verified, &access.tasklist_id)
    {
        return Err(task_error("feishu_task_verify", false));
    }
    Ok(json!({"ok":true,"task":task_json(&verified, access)}))
}

async fn update_task(
    client: &SharedClient,
    auth: Auth,
    access: &TaskAccess,
    input: &Value,
    complete: bool,
) -> Result<Value, ProviderError> {
    let task_id = required_text(input, "task_id")?;
    let current = client
        .get_task(&task_id, "open_id", auth)
        .await
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
        due: if complete {
            None
        } else {
            normalized_due(input)?
        },
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
        .await
        .map_err(|_| task_error("feishu_task_update", false))?;
    Ok(json!({"ok":true,"task":task_json(&updated, access)}))
}

async fn change_task_members(
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
        .await
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
        client
            .add_task_members(&task_id, &changes, "open_id", auth)
            .await
    } else {
        client
            .remove_task_members(&task_id, &changes, "open_id", auth)
            .await
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
        let (_, identity_directory) = crate::identity::load(context.storage()).await?;
        let app_id = identity_directory
            .agent_app_id(&context.agent_id)
            .map(str::to_owned)
            .ok_or(ProviderError::InvalidInput {
                field: "task_agent_app_id",
            })?;
        let team_members = context.storage().team_members().await?;
        let lead_id = team_lead_id(&team_members)?;
        let directory = MemberDirectory::from_roster(
            &identity_directory,
            &team_members,
            &app_id,
            context.storage(),
            client.clone(),
            auth,
        )
        .await?;
        let tasklist_id = ensure_team_tasklist(
            context.storage(),
            &context.workspace_config,
            &directory,
            &lead_id,
            client,
            auth,
        )
        .await?;
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
        if has_bare_mention(
            content,
            self.directory
                .mention_targets
                .values()
                .map(|target| target.display_name.as_str()),
        ) {
            return Err(ProviderError::InvalidInput {
                field: BARE_MENTION,
            });
        }
        let ids = extract_agent_ids(content);
        let targets =
            ids.iter()
                .map(|id| {
                    self.directory.mention_targets.get(id).cloned().ok_or(
                        ProviderError::InvalidInput {
                            field: "content: 无法提及该成员，请先确认飞书身份",
                        },
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
        Ok(render_card_markdown(content, &targets).0)
    }
}

fn team_lead_id(team_members: &[ProviderTeamMember]) -> Result<String, ProviderError> {
    team_members
        .iter()
        .filter_map(|member| match member {
            ProviderTeamMember::Human(person) => Some(person),
            ProviderTeamMember::Agent { .. } => None,
        })
        .find(|person| person.team_role == TeamRole::Lead)
        .map(|person| person.person_id.to_string())
        .ok_or(ProviderError::NotFound {
            entity: "team_lead",
        })
}

fn owner_task_error(operation: &'static str, error: ClientError) -> ProviderError {
    if crate::workspace_owner::requires_sign_in(&error) {
        ProviderError::AuthorizationRequired
    } else {
        task_error(operation, error.is_retryable())
    }
}

/// Joins `app_id` to the Team Tasklist as the team lead, creating the list
/// under the lead's own identity when the workspace has none yet.
pub(crate) async fn join_team_tasklist_as_lead(
    storage: &dyn lankide_provider::ProviderStorage,
    workspace_config: &Value,
    lead_client: &SharedClient,
    app_id: &str,
) -> Result<String, ProviderError> {
    let mut workspace_state = storage.workspace_state().await?;
    let saved_guid = text(&workspace_state, "tasklist_guid");
    let agent = [TasklistMember::editor(MemberIdentity::app(app_id))];
    let saved = match saved_guid.as_deref() {
        Some(guid) => match lead_client.get_tasklist(guid, "open_id", Auth::User).await {
            Ok(tasklist) => Some(tasklist),
            Err(ClientError::Api { code: 1470404, .. }) => None,
            Err(error) => {
                return Err(owner_task_error("feishu_tasklist_access", error));
            }
        },
        None => None,
    };
    let tasklist = match saved {
        Some(tasklist) => {
            let missing = missing_tasklist_members(&tasklist, &agent);
            if !missing.is_empty() {
                lead_client
                    .add_tasklist_members(&tasklist.guid, &missing, "open_id", Auth::User)
                    .await
                    .map_err(|error| owner_task_error("feishu_tasklist_members", error))?;
            }
            tasklist
        }
        None => {
            let name =
                tasklist_name(workspace_config).unwrap_or_else(|| DEFAULT_TASKLIST_NAME.to_owned());
            lead_client
                .create_tasklist(&name, &agent, "open_id", Auth::User)
                .await
                .map_err(|error| owner_task_error("feishu_tasklist_create", error))?
        }
    };
    if tasklist.guid.trim().is_empty() {
        return Err(task_error("feishu_tasklist_get", false));
    }
    if saved_guid.as_deref() != Some(tasklist.guid.as_str()) {
        workspace_state["tasklist_guid"] = Value::String(tasklist.guid.clone());
        storage.put_workspace_state(workspace_state).await?;
    }
    Ok(tasklist.guid)
}

async fn ensure_team_tasklist(
    storage: &dyn lankide_provider::ProviderStorage,
    workspace_config: &Value,
    directory: &MemberDirectory,
    lead_id: &str,
    client: Arc<SharedClient>,
    auth: Auth,
) -> Result<String, ProviderError> {
    let mut workspace_state = storage.workspace_state().await?;
    let saved_tasklist_id = text(&workspace_state, "tasklist_guid");
    let lead = directory
        .humans
        .get(lead_id)
        .cloned()
        .map(MemberIdentity::user)
        .ok_or(ProviderError::NotFound {
            entity: "team_lead",
        })?;
    let tasklist_id = ensure_tasklist(
        client,
        auth,
        saved_tasklist_id.as_deref(),
        tasklist_name(workspace_config).as_deref(),
        &directory.tasklist_members(),
        &lead,
        &directory.current_app_id,
    )
    .await?;
    if saved_tasklist_id.as_deref() != Some(tasklist_id.as_str()) {
        workspace_state["tasklist_guid"] = Value::String(tasklist_id.clone());
        storage.put_workspace_state(workspace_state).await?;
    }
    Ok(tasklist_id)
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
                    .map(TasklistMember::editor),
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
    owner: &MemberIdentity,
    caller_app_id: &str,
) -> Result<String, ProviderError> {
    let saved_guid = saved_guid
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let saved_tasklist = match saved_guid.as_deref() {
        Some(guid) => match client.get_tasklist(guid, "open_id", auth).await {
            Ok(tasklist) => Some(tasklist),
            Err(ClientError::Api { code: 1470404, .. }) => None,
            // Another member can manage the list; never replace the team's
            // list just because this App is not in it yet.
            Err(error) => {
                return Err(task_error("feishu_tasklist_access", error.is_retryable()));
            }
        },
        None => None,
    };
    let mut tasklist = if let Some(tasklist) = saved_tasklist {
        tasklist
    } else {
        let name = configured_name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEFAULT_TASKLIST_NAME);
        let mut matches = Vec::new();
        let mut page_token = None;
        loop {
            let page = client
                .list_tasklists(page_token.as_deref(), 100, "open_id", auth)
                .await
                .map_err(|error| task_error("feishu_tasklist_list", error.is_retryable()))?;
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
                .create_tasklist(name, desired_members, "open_id", auth)
                .await
                .map_err(|error| task_error("feishu_tasklist_create", error.is_retryable()))?,
            1 => matches.remove(0),
            _ => return Err(task_error("feishu_tasklist_ambiguous", false)),
        }
    };
    if tasklist.guid.trim().is_empty() {
        return Err(task_error("feishu_tasklist_get", false));
    }
    if let Some(name) = configured_name.filter(|value| !value.trim().is_empty())
        && tasklist.name.trim() != name
    {
        tasklist = client
            .patch_tasklist(&tasklist.guid, Some(name), "open_id", auth)
            .await
            .map_err(|error| task_error("feishu_tasklist_update", error.is_retryable()))?;
    }
    let missing = missing_tasklist_members(&tasklist, desired_members);
    if !missing.is_empty() {
        tasklist = client
            .add_tasklist_members(&tasklist.guid, &missing, "open_id", auth)
            .await
            .map_err(|error| task_error("feishu_tasklist_members", error.is_retryable()))?;
    }
    // The list belongs to the team lead; Agents work in it as editors. Only
    // the owning App can hand it over.
    let owned_by_caller = tasklist.owner.as_ref().is_some_and(|current| {
        normalized_member_type(current) == "app" && current.id == caller_app_id
    });
    if owned_by_caller {
        client
            .transfer_tasklist_owner(&tasklist.guid, owner, "open_id", auth)
            .await
            .map_err(|error| task_error("feishu_tasklist_owner", error.is_retryable()))?;
    }
    Ok(tasklist.guid)
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
        "updated_at": task.updated_at,
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
    if let ProviderError::InvalidInput { field } = error {
        return denied(error.code(), &format!("任务参数无效：{field}。"));
    }
    let message = match error {
        ProviderError::SchemaMismatch { .. } => "任务参数无效。",
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

fn due_schema() -> Value {
    json!({"description":"截止日期：YYYY-MM-DD，或 {timestamp: 毫秒字符串, is_all_day: 布尔值}",
        "oneOf":[{"type":"string","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"},
        {"type":"object","required":["timestamp","is_all_day"],"additionalProperties":false,
        "properties":{"timestamp":{"type":"string","pattern":"^[0-9]+$"},"is_all_day":{"type":"boolean"}}}]})
}
fn normalized_due(input: &Value) -> Result<Option<Value>, ProviderError> {
    let Some(value) = input.get("due") else {
        return Ok(None);
    };
    let invalid = || ProviderError::InvalidInput {
        field: "due: 请使用 YYYY-MM-DD，或 {timestamp: 毫秒字符串, is_all_day: 布尔值}",
    };
    if let Some(date) = value.as_str() {
        let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| invalid())?;
        if parsed.format("%Y-%m-%d").to_string() != date {
            return Err(invalid());
        }
        return Ok(Some(
            json!({"timestamp":parsed.and_hms_opt(0,0,0).ok_or_else(invalid)?.and_utc().timestamp_millis().to_string(),"is_all_day":true}),
        ));
    }
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() != 2
        || value["timestamp"]
            .as_str()
            .and_then(|v| v.parse::<u64>().ok())
            .is_none()
        || !value["is_all_day"].is_boolean()
    {
        return Err(invalid());
    }
    Ok(Some(value.clone()))
}
