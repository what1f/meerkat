//! Feishu Open API and event wire objects.
//!
//! These objects intentionally retain provider terminology.  Conversion to
//! the shared Provider event contract belongs in a later thin adapter; keeping
//! the wire shape here means that a newer SDK does not force Runtime to know
//! about Feishu's `open_id`, `union_id`, CardKit or Task payloads.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;

/// Feishu callback payloads use both an empty string and JSON `null` for
/// optional identifiers.  Keep the provider-local DTO ergonomic (`String`)
/// while accepting both wire representations; normalization applies the
/// required-field checks after decoding.
fn nullable_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// Mirrors the local TypeScript SDK's optional arrays and Go adapter's
/// `encoding/json` behavior for a wire-level `null` slice.
fn nullable_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

/// Mirrors the local SDK's optional nested objects and Go's zero-value decode
/// when a callback contains a `null` object.
fn nullable_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Identity {
    #[serde(default, deserialize_with = "nullable_string")]
    pub open_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub union_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub user_id: String,
}

impl Identity {
    pub fn preferred(&self) -> Option<(&'static str, &str)> {
        if !self.open_id.trim().is_empty() {
            Some(("open_id", self.open_id.trim()))
        } else if !self.union_id.trim().is_empty() {
            Some(("union_id", self.union_id.trim()))
        } else if !self.user_id.trim().is_empty() {
            Some(("user_id", self.user_id.trim()))
        } else {
            None
        }
    }

    pub fn aliases(&self) -> Vec<ExternalIdentity> {
        [
            ("open_id", self.open_id.trim()),
            ("union_id", self.union_id.trim()),
            ("user_id", self.user_id.trim()),
        ]
        .into_iter()
        .filter(|(_, id)| !id.is_empty())
        .map(|(kind, id)| ExternalIdentity {
            kind: kind.to_owned(),
            id: id.to_owned(),
        })
        .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ExternalIdentity {
    #[serde(default, deserialize_with = "nullable_string")]
    pub kind: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct EventHeader {
    #[serde(default, deserialize_with = "nullable_string")]
    pub app_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub event_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub event_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub create_time: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub tenant_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct EventMention {
    #[serde(default, deserialize_with = "nullable_string")]
    pub key: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub id: Identity,
    #[serde(default, deserialize_with = "nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub mentioned_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct EventMessage {
    #[serde(default, deserialize_with = "nullable_string")]
    pub message_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub root_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub parent_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub thread_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub message_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub content: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub create_time: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub update_time: String,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub mentions: Vec<EventMention>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct EventSender {
    #[serde(default, deserialize_with = "nullable_string")]
    pub sender_type: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub sender_id: Identity,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageEventBody {
    #[serde(default, deserialize_with = "nullable_default")]
    pub sender: EventSender,
    #[serde(default, deserialize_with = "nullable_default")]
    pub message: EventMessage,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct FeishuMessageEvent {
    #[serde(default, deserialize_with = "nullable_default")]
    pub header: EventHeader,
    #[serde(default, deserialize_with = "nullable_default")]
    pub event: MessageEventBody,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionType {
    #[serde(default, deserialize_with = "nullable_string")]
    pub emoji_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionEventBody {
    #[serde(default, deserialize_with = "nullable_string")]
    pub message_id: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub reaction_type: ReactionType,
    #[serde(default, deserialize_with = "nullable_string")]
    pub operator_type: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub user_id: Identity,
    #[serde(default, deserialize_with = "nullable_string")]
    pub app_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub action_time: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct FeishuReactionEvent {
    #[serde(default, deserialize_with = "nullable_default")]
    pub header: EventHeader,
    #[serde(default, deserialize_with = "nullable_default")]
    pub event: ReactionEventBody,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ChatUpdatedBody {
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_id: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub after_change: ChatChange,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ChatChange {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct FeishuChatUpdatedEvent {
    #[serde(default, deserialize_with = "nullable_default")]
    pub header: EventHeader,
    #[serde(default, deserialize_with = "nullable_default")]
    pub event: ChatUpdatedBody,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct ApiResponse<T> {
    #[serde(default)]
    pub code: i64,
    #[serde(default, deserialize_with = "nullable_string")]
    pub msg: String,
    #[serde(default)]
    pub data: Option<T>,
}

#[derive(Clone, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TokenResponse {
    #[serde(default, deserialize_with = "nullable_string")]
    pub tenant_access_token: String,
    #[serde(default)]
    pub expire: i64,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenResponse")
            .field("tenant_access_token", &"<redacted>")
            .field("expire", &self.expire)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageResult {
    #[serde(default, deserialize_with = "nullable_string")]
    pub message_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessagePageResponse {
    #[serde(default, deserialize_with = "nullable_vec")]
    pub items: Vec<MessageRecord>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub page_token: String,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub message_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub root_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub parent_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub thread_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub msg_type: String,
    #[serde(default)]
    pub body: Option<MessageBody>,
    #[serde(default)]
    pub sender: Option<MessageRecordSender>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub create_time: String,
    /// Some Feishu deployments return `data.items[0]` for this endpoint,
    /// while others return the message object directly.
    #[serde(default)]
    pub items: Vec<MessageRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageBody {
    #[serde(default, deserialize_with = "nullable_string")]
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageRecordSender {
    #[serde(default, deserialize_with = "nullable_string")]
    pub id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub id_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub sender_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ImageResult {
    #[serde(default, deserialize_with = "nullable_string")]
    pub image_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct FileResult {
    #[serde(default, deserialize_with = "nullable_string")]
    pub file_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionResult {
    #[serde(default, deserialize_with = "nullable_string")]
    pub reaction_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionOperator {
    #[serde(default, deserialize_with = "nullable_string")]
    pub operator_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub operator_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub reaction_id: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub reaction_type: ReactionType,
    #[serde(default, deserialize_with = "nullable_default")]
    pub operator: ReactionOperator,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionPage {
    #[serde(default, deserialize_with = "nullable_vec")]
    pub items: Vec<ReactionRecord>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub page_token: String,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ChatRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub chat_mode: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct UserRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub open_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub union_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub user_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub nickname: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub description: String,
    #[serde(default, deserialize_with = "nullable_default")]
    pub avatar: UserAvatar,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct UserAvatar {
    #[serde(default, deserialize_with = "nullable_string")]
    pub avatar_origin: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub avatar_640: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub avatar_240: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub avatar_72: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct BotRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub open_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub app_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct BotInfoResponse {
    #[serde(default, deserialize_with = "nullable_default")]
    pub bot: BotRecord,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct DownloadedResource {
    pub data: Vec<u8>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub content_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub file_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub guid: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub summary: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub description: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub status: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub completed_at: String,
    #[serde(default)]
    pub creator: Option<TaskMember>,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub assignees: Vec<TaskMember>,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub collaborators: Vec<TaskMember>,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub members: Vec<TaskMember>,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub tasklists: Vec<TasklistRef>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub tasklist_guid: String,
    #[serde(default)]
    pub due: Option<Value>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub created_at: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub updated_at: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub url: String,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub custom_fields: Vec<Value>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskMember {
    #[serde(default, deserialize_with = "nullable_string")]
    pub id: String,
    #[serde(
        rename = "type",
        alias = "id_type",
        default,
        deserialize_with = "nullable_string"
    )]
    pub id_type: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub role: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TasklistRef {
    #[serde(default, deserialize_with = "nullable_string")]
    pub tasklist_guid: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub section_guid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TasklistRecord {
    #[serde(default, deserialize_with = "nullable_string")]
    pub guid: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub name: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub description: String,
    #[serde(default)]
    pub owner: Option<TaskMember>,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub members: Vec<TaskMember>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub url: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub created_at: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub updated_at: String,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskComment {
    #[serde(alias = "id", default, deserialize_with = "nullable_string")]
    pub comment_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub content: String,
    #[serde(default)]
    pub creator: Option<TaskMember>,
    #[serde(alias = "created_at", default, deserialize_with = "nullable_string")]
    pub create_time: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskPageResponse {
    #[serde(default, deserialize_with = "nullable_vec")]
    pub items: Vec<TaskRecord>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub page_token: String,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskResult {
    #[serde(default)]
    pub task: Option<TaskRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TasklistResult {
    #[serde(default)]
    pub tasklist: Option<TasklistRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TasklistPageResponse {
    #[serde(default, deserialize_with = "nullable_vec")]
    pub items: Vec<TasklistRecord>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub page_token: String,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskCommentPageResponse {
    #[serde(default, deserialize_with = "nullable_vec")]
    pub items: Vec<TaskComment>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub page_token: String,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskCommentResult {
    #[serde(default)]
    pub comment: Option<TaskComment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct TaskEventBody {
    #[serde(default, deserialize_with = "nullable_vec")]
    pub event_types: Vec<String>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub task_guid: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub comment_id: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct FeishuTaskEvent {
    #[serde(default, deserialize_with = "nullable_default")]
    pub header: EventHeader,
    #[serde(default, deserialize_with = "nullable_default")]
    pub event: TaskEventBody,
}
