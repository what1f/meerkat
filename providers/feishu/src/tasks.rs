//! Provider-defined Feishu Task v2 request shapes.
//!
//! Task is intentionally not part of the common Message/Channel contract.
//! These DTOs are enough for a thin adapter to build task tools and custom
//! events while preserving the Task API's app/user identity semantics.

use crate::mention::extract_agent_ids;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const TASK_SUBSCRIPTION_PATH: &str = "/open-apis/task/v2/task_v2/task_subscription";
pub const TASKS_PATH: &str = "/open-apis/task/v2/tasks";
pub const COMMENTS_PATH: &str = "/open-apis/task/v2/comments";
pub const TASKLISTS_PATH: &str = "/open-apis/task/v2/tasklists";

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct MemberIdentity {
    pub external_id: String,
    #[serde(rename = "type")]
    pub identity_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct TasklistMember {
    pub external_id: String,
    #[serde(rename = "type")]
    pub identity_type: String,
    pub role: String,
}

/// A typed member for Task v2's member operations. Task members use
/// business roles (`assignee`/`follower`), which are intentionally separate
/// from a Tasklist's transport ACL roles (`editor`/`viewer`).
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct TaskMemberInput {
    pub external_id: String,
    #[serde(rename = "type")]
    pub identity_type: String,
    pub role: String,
}

impl TaskMemberInput {
    pub fn new(identity: MemberIdentity, role: impl Into<String>) -> Self {
        Self {
            external_id: identity.external_id,
            identity_type: identity.identity_type,
            role: role.into(),
        }
    }

    pub fn assignee(identity: MemberIdentity) -> Self {
        Self::new(identity, "assignee")
    }

    pub fn follower(identity: MemberIdentity) -> Self {
        Self::new(identity, "follower")
    }

    pub(crate) fn as_json(&self) -> Option<Value> {
        (!self.external_id.trim().is_empty()
            && !self.identity_type.trim().is_empty()
            && !self.role.trim().is_empty())
        .then(|| {
            json!({
                "id": self.external_id.trim(),
                "type": self.identity_type.trim(),
                "role": self.role.trim(),
            })
        })
    }
}

impl TasklistMember {
    pub fn new(identity: MemberIdentity, role: impl Into<String>) -> Self {
        Self {
            external_id: identity.external_id,
            identity_type: identity.identity_type,
            role: role.into(),
        }
    }

    pub fn editor(identity: MemberIdentity) -> Self {
        Self::new(identity, "editor")
    }

    pub fn viewer(identity: MemberIdentity) -> Self {
        Self::new(identity, "viewer")
    }

    pub(crate) fn as_json(&self) -> Option<Value> {
        (!self.external_id.trim().is_empty()
            && !self.identity_type.trim().is_empty()
            && !self.role.trim().is_empty())
        .then(|| {
            json!({
                "id": self.external_id.trim(),
                "type": self.identity_type.trim(),
                "role": self.role.trim(),
            })
        })
    }
}

impl MemberIdentity {
    pub fn app(external_id: impl Into<String>) -> Self {
        Self {
            external_id: external_id.into(),
            identity_type: "app".to_owned(),
        }
    }
    pub fn user(external_id: impl Into<String>) -> Self {
        Self {
            external_id: external_id.into(),
            identity_type: "user".to_owned(),
        }
    }

    fn is_valid(&self) -> bool {
        !self.external_id.trim().is_empty() && !self.identity_type.trim().is_empty()
    }

    fn as_json(&self, role: &str) -> Option<Value> {
        self.is_valid().then(|| {
            json!({
                "id": self.external_id.trim(),
                "type": self.identity_type.trim(),
                "role": role,
            })
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ListRequest {
    pub completed: Option<bool>,
    pub page_token: Option<String>,
    pub page_size: usize,
    pub task_type: Option<String>,
    pub user_id_type: String,
}

impl Default for ListRequest {
    fn default() -> Self {
        Self {
            completed: None,
            page_token: None,
            page_size: 100,
            task_type: Some("my_tasks".to_owned()),
            user_id_type: "open_id".to_owned(),
        }
    }
}

impl ListRequest {
    pub fn query(&self) -> Vec<(String, String)> {
        let mut result = vec![
            (
                "page_size".to_owned(),
                if self.page_size == 0 {
                    "100".to_owned()
                } else {
                    self.page_size.clamp(1, 100).to_string()
                },
            ),
            (
                "user_id_type".to_owned(),
                if self.user_id_type.trim().is_empty() {
                    "open_id".to_owned()
                } else {
                    self.user_id_type.trim().to_owned()
                },
            ),
        ];
        if let Some(completed) = self.completed {
            result.push(("completed".to_owned(), completed.to_string()));
        }
        if let Some(token) = self
            .page_token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            result.push(("page_token".to_owned(), token.trim().to_owned()));
        }
        if let Some(task_type) = self
            .task_type
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            result.push(("type".to_owned(), task_type.trim().to_owned()));
        }
        result
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct TaskPatch {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub due: Option<Value>,
    pub extra: Value,
}

impl TaskPatch {
    pub fn is_empty(&self) -> bool {
        self.summary.is_none()
            && self.description.is_none()
            && self.status.is_none()
            && self.due.is_none()
            && !self
                .extra
                .as_object()
                .is_some_and(|value| !value.is_empty())
    }

    pub fn to_json(&self) -> Value {
        let mut task = serde_json::Map::new();
        let mut update_fields = Vec::new();
        if let Some(value) = self.summary.as_deref() {
            task.insert("summary".to_owned(), Value::String(value.to_owned()));
            update_fields.push(Value::String("summary".to_owned()));
        }
        if let Some(value) = self.description.as_deref() {
            task.insert("description".to_owned(), Value::String(value.to_owned()));
            update_fields.push(Value::String("description".to_owned()));
        }
        if let Some(value) = self.status.as_deref() {
            let completed_at = if matches!(value, "completed" | "done") {
                now_millis().to_string()
            } else {
                "0".to_owned()
            };
            task.insert("completed_at".to_owned(), Value::String(completed_at));
            update_fields.push(Value::String("completed_at".to_owned()));
        }
        if let Some(value) = &self.due {
            task.insert("due".to_owned(), value.clone());
            update_fields.push(Value::String("due".to_owned()));
        }
        if let Value::Object(extra) = &self.extra {
            for (key, value) in extra {
                task.insert(key.clone(), value.clone());
                if !update_fields
                    .iter()
                    .any(|field| field.as_str() == Some(key.as_str()))
                {
                    update_fields.push(Value::String(key.clone()));
                }
            }
        }
        json!({ "task": Value::Object(task), "update_fields": update_fields })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct CreateTask {
    pub summary: String,
    pub description: String,
    pub due: Option<Value>,
    pub assignee_ids: Vec<String>,
    pub collaborator_ids: Vec<String>,
    /// Typed Task identities.  Use `MemberIdentity::app` for an Agent App and
    /// `MemberIdentity::user` for a human; the legacy `*_ids` fields remain
    /// user identities for callers that already have provider IDs.
    pub assignees: Vec<MemberIdentity>,
    pub followers: Vec<MemberIdentity>,
    pub tasklist_guid: Option<String>,
    /// Feishu's idempotency key for create retries.  The live E2E adapter can
    /// use its run-scoped operation ID here without exposing it as provider
    /// persistence state.
    pub client_token: Option<String>,
    pub extra: Value,
}

impl CreateTask {
    pub fn to_json(&self) -> Value {
        let mut body = json!({ "summary": self.summary, "description": self.description });
        if let Some(due) = &self.due {
            body["due"] = due.clone();
        }
        let mut members = Vec::new();
        for id in &self.assignee_ids {
            let identity = MemberIdentity::user(id);
            if let Some(member) = identity.as_json("assignee") {
                members.push(member);
            }
        }
        for id in &self.collaborator_ids {
            let identity = MemberIdentity::user(id);
            if let Some(member) = identity.as_json("follower") {
                members.push(member);
            }
        }
        for identity in &self.assignees {
            if let Some(member) = identity.as_json("assignee") {
                members.push(member);
            }
        }
        for identity in &self.followers {
            if let Some(member) = identity.as_json("follower") {
                members.push(member);
            }
        }
        if !members.is_empty() {
            body["members"] = Value::Array(members);
        }
        if let Some(tasklist_guid) = &self.tasklist_guid {
            body["tasklists"] = json!([{ "tasklist_guid": tasklist_guid }]);
        }
        if let Some(client_token) = self
            .client_token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["client_token"] = Value::String(client_token.trim().to_owned());
        }
        if let Value::Object(extra) = &self.extra {
            for (key, value) in extra {
                body[key] = value.clone();
            }
        }
        body
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub struct Comment {
    pub task_guid: String,
    pub content: String,
    pub reply_to_comment_id: Option<String>,
}

impl Comment {
    pub fn to_json(&self) -> Value {
        let mut body = json!({
            "resource_type": "task",
            "resource_id": self.task_guid,
            "content": self.content
        });
        if let Some(id) = &self.reply_to_comment_id {
            body["reply_to_comment_id"] = Value::String(id.clone());
        }
        body
    }

    pub fn mentioned_agent_ids(&self) -> Vec<String> {
        extract_agent_ids(&self.content)
    }
}

fn now_millis() -> i128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as i128)
        .unwrap_or_default()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskEventReason {
    Assigned,
    Updated,
    Completed,
    Commented,
    Deleted,
    AccessChanged,
    Unknown(String),
}

impl TaskEventReason {
    pub fn from_event_types(types: &[String]) -> Self {
        if types
            .iter()
            .any(|value| value == "task_create" || value == "task_assignees_update")
        {
            return Self::Assigned;
        }
        if types.iter().any(|value| value == "task_completed_update") {
            return Self::Completed;
        }
        if types.iter().any(|value| value == "task_deleted") {
            return Self::Deleted;
        }
        if types.iter().any(|value| value == "task_comment_create") {
            return Self::Commented;
        }
        if types.iter().any(|value| {
            matches!(
                value.as_str(),
                "task_update"
                    | "task_summary_update"
                    | "task_description_update"
                    | "task_due_update"
                    | "task_start_update"
                    | "task_repeat_rule_update"
                    | "task_tasklist_update"
                    | "task_progress_update"
                    | "task_status_update"
            )
        }) {
            return Self::Updated;
        }
        if types.iter().any(|value| {
            matches!(
                value.as_str(),
                "task_member_add" | "task_member_remove" | "task_members_update"
            )
        }) {
            return Self::AccessChanged;
        }
        Self::Unknown(types.join(","))
    }
}
