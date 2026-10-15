//! Pure Feishu event normalization.
//!
//! No Runtime or Provider trait types are used here.  The result is a stable
//! provider-local envelope that a later adapter can map into the shared
//! `ProviderEvent` while retaining all external IDs and attachment metadata.

use crate::dto::{
    ChatUpdatedBody, EventMessage, EventSender, FeishuChatUpdatedEvent, FeishuMessageEvent,
    FeishuReactionEvent, FeishuTaskEvent, Identity,
};
use crate::mention::inbound_marker;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum SenderKind {
    Human,
    Agent,
    Bot,
    System,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedSender {
    pub external_id: String,
    pub sender_id: String,
    pub kind: SenderKind,
    pub display_name: String,
    pub aliases: Vec<(String, String)>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedMention {
    pub external_id: String,
    pub aliases: Vec<(String, String)>,
    pub agent_id: Option<String>,
    pub display_name: String,
    pub kind: SenderKind,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedAttachment {
    pub kind: String,
    pub key: String,
    pub name: String,
    pub content_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedMessage {
    pub provider_message_id: String,
    pub chat_id: String,
    pub channel_id: String,
    pub chat_type: String,
    pub content: String,
    pub message_type: String,
    pub reply_to_provider_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub sender: NormalizedSender,
    pub mentions: Vec<NormalizedMention>,
    pub mentioned_agent_ids: Vec<String>,
    pub attachments: Vec<NormalizedAttachment>,
    pub occurred_at_ms: Option<i64>,
    pub tenant_key: String,
    pub app_id: String,
    pub event_id: String,
    pub correlation_key: String,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedReaction {
    pub action: ReactionAction,
    pub target_provider_message_id: String,
    pub chat_id: Option<String>,
    pub reaction: String,
    pub sender: NormalizedSender,
    pub occurred_at_ms: Option<i64>,
    pub tenant_key: String,
    pub app_id: String,
    pub event_id: String,
    pub raw: Vec<u8>,
}

pub use lankide_core::ReactionAction;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedChatUpdate {
    pub chat_id: String,
    pub name: String,
    pub tenant_key: String,
    pub event_id: String,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedTaskEvent {
    pub task_id: String,
    pub event_types: Vec<String>,
    pub comment_id: Option<String>,
    pub content: String,
    pub tenant_key: String,
    pub event_id: String,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum NormalizedEvent {
    Message(NormalizedMessage),
    Reaction(NormalizedReaction),
    ChatUpdated(NormalizedChatUpdate),
    Task(NormalizedTaskEvent),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizationConfig {
    /// External bot IDs (open_id/union_id/user_id/app_id) to internal Agent ID.
    pub agent_by_external_id: HashMap<String, String>,
    /// The current App's identity; self-authored reactions must not wake it.
    pub app_id: String,
}

impl NormalizationConfig {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into().trim().to_owned(),
            agent_by_external_id: HashMap::new(),
        }
    }

    pub fn with_agent(
        mut self,
        external_id: impl Into<String>,
        agent_id: impl Into<String>,
    ) -> Self {
        let external_id = external_id.into();
        let agent_id = agent_id.into();
        if !external_id.trim().is_empty() && !agent_id.trim().is_empty() {
            self.agent_by_external_id
                .insert(external_id.trim().to_owned(), agent_id.trim().to_owned());
        }
        self
    }

    fn resolve_agent(&self, identity: &Identity) -> Option<String> {
        [&identity.open_id, &identity.union_id, &identity.user_id]
            .into_iter()
            .find_map(|value| self.agent_by_external_id.get(value.trim()).cloned())
    }
}

pub fn normalize_event(
    raw: &[u8],
    config: &NormalizationConfig,
) -> Result<NormalizedEvent, NormalizeError> {
    let header = serde_json::from_slice::<serde_json::Value>(raw)
        .map_err(|error| NormalizeError::Malformed(error.to_string()))?
        .get("header")
        .cloned()
        .unwrap_or(Value::Null);
    let event_type = header
        .get("event_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "im.message.receive_v1" => normalize_message(raw, config).map(NormalizedEvent::Message),
        "im.message.reaction.created_v1" | "im.message.reaction.deleted_v1" => {
            normalize_reaction(raw, config).map(NormalizedEvent::Reaction)
        }
        "im.chat.updated_v1" => normalize_chat_update(raw).map(NormalizedEvent::ChatUpdated),
        value if value.starts_with("task.") => normalize_task(raw).map(NormalizedEvent::Task),
        value => Err(NormalizeError::UnsupportedEvent(value.to_owned())),
    }
}

pub fn normalize_message(
    raw: &[u8],
    config: &NormalizationConfig,
) -> Result<NormalizedMessage, NormalizeError> {
    let envelope: FeishuMessageEvent = serde_json::from_slice(raw)
        .map_err(|error| NormalizeError::Malformed(error.to_string()))?;
    let header = &envelope.header;
    let message = &envelope.event.message;
    if header.event_type != "im.message.receive_v1" {
        return Err(NormalizeError::UnsupportedEvent(header.event_type.clone()));
    }
    if message.message_id.trim().is_empty() || message.chat_id.trim().is_empty() {
        return Err(NormalizeError::Incomplete(
            "message_id and chat_id are required".to_owned(),
        ));
    }
    if header.app_id.trim().is_empty() {
        return Err(NormalizeError::Incomplete(
            "header.app_id is required".to_owned(),
        ));
    }
    if !config.app_id.trim().is_empty() && config.app_id.trim() != header.app_id.trim() {
        return Err(NormalizeError::AppMismatch {
            expected: config.app_id.trim().to_owned(),
            actual: header.app_id.trim().to_owned(),
        });
    }

    let mention_result = normalize_mentions(message, config);
    let (content, attachments) = normalize_content(message, &mention_result.replacements)?;
    let sender = normalize_sender(&envelope.event.sender, config);
    let channel_id = if message.chat_type == "p2p" {
        "private_chat"
    } else {
        "group_chat"
    };
    // The same message can be delivered through more than one App callback.
    // The message ID is the stable provider identity; Feishu's callback
    // event_id is retained in `raw` but must not split deduplication keys.
    let event_id = format!("im.message.receive:{}", message.message_id.trim());
    let occurred_at_ms = parse_timestamp_ms(
        non_empty(&message.create_time).or_else(|| non_empty(&header.create_time)),
    );
    Ok(NormalizedMessage {
        provider_message_id: message.message_id.trim().to_owned(),
        chat_id: message.chat_id.trim().to_owned(),
        channel_id: channel_id.to_owned(),
        chat_type: message.chat_type.trim().to_owned(),
        content,
        message_type: message.message_type.trim().to_owned(),
        reply_to_provider_message_id: non_empty(&message.parent_id).map(str::to_owned),
        thread_id: non_empty(&message.thread_id).map(str::to_owned),
        sender,
        mentions: mention_result.mentions,
        mentioned_agent_ids: mention_result.agent_ids,
        attachments,
        occurred_at_ms,
        tenant_key: header.tenant_key.trim().to_owned(),
        app_id: header.app_id.trim().to_owned(),
        event_id,
        correlation_key: format!("feishu:message:{}", message.message_id.trim()),
        raw: raw.to_vec(),
    })
}

pub fn normalize_reaction(
    raw: &[u8],
    config: &NormalizationConfig,
) -> Result<NormalizedReaction, NormalizeError> {
    let envelope: FeishuReactionEvent = serde_json::from_slice(raw)
        .map_err(|error| NormalizeError::Malformed(error.to_string()))?;
    let header = &envelope.header;
    let event = &envelope.event;
    let action = match header.event_type.as_str() {
        "im.message.reaction.created_v1" => ReactionAction::Add,
        "im.message.reaction.deleted_v1" => ReactionAction::Remove,
        value => return Err(NormalizeError::UnsupportedEvent(value.to_owned())),
    };
    if header.app_id.trim().is_empty()
        || event.message_id.trim().is_empty()
        || event.reaction_type.emoji_type.trim().is_empty()
    {
        return Err(NormalizeError::Incomplete(
            "reaction app_id, message_id and emoji_type are required".to_owned(),
        ));
    }
    if !config.app_id.trim().is_empty() && config.app_id.trim() != header.app_id.trim() {
        return Err(NormalizeError::AppMismatch {
            expected: config.app_id.trim().to_owned(),
            actual: header.app_id.trim().to_owned(),
        });
    }
    match event.operator_type.trim() {
        "user" if event.user_id.preferred().is_none() => {
            return Err(NormalizeError::Incomplete(
                "user reaction has no operator identity".to_owned(),
            ));
        }
        "app" if event.app_id.trim().is_empty() => {
            return Err(NormalizeError::Incomplete(
                "app reaction has no app_id".to_owned(),
            ));
        }
        "user" | "app" => {}
        value => {
            return Err(NormalizeError::Incomplete(format!(
                "unsupported reaction operator type {value}"
            )));
        }
    }
    let identity = if event.operator_type == "app" {
        Identity {
            open_id: String::new(),
            union_id: String::new(),
            user_id: event.app_id.clone(),
        }
    } else {
        event.user_id.clone()
    };
    let sender = normalize_sender_with_identity(&identity, &event.operator_type, config, "");
    let action_time = non_empty(&event.action_time).or_else(|| non_empty(&header.create_time));
    // Feishu may deliver one human reaction through several Agent Apps.  Use
    // the stable union/user alias for the event identity so those deliveries
    // deduplicate in the thin adapter; using open_id here would split them.
    let actor_id = if event.operator_type == "user" {
        non_empty(&event.user_id.union_id)
            .or_else(|| non_empty(&event.user_id.user_id))
            .or_else(|| non_empty(&event.user_id.open_id))
            .unwrap_or_default()
    } else {
        non_empty(&event.app_id).unwrap_or_default()
    };
    let event_id = format!(
        "{}:{}:{}:{}:{}:{}",
        header.event_type,
        event.message_id.trim(),
        event.operator_type.trim(),
        actor_id,
        event.reaction_type.emoji_type.trim(),
        action_time.unwrap_or_default()
    );
    Ok(NormalizedReaction {
        action,
        target_provider_message_id: event.message_id.trim().to_owned(),
        chat_id: None,
        reaction: event.reaction_type.emoji_type.trim().to_owned(),
        sender,
        occurred_at_ms: parse_timestamp_ms(action_time),
        tenant_key: header.tenant_key.trim().to_owned(),
        app_id: header.app_id.trim().to_owned(),
        event_id,
        raw: raw.to_vec(),
    })
}

pub fn normalize_chat_update(raw: &[u8]) -> Result<NormalizedChatUpdate, NormalizeError> {
    let envelope: FeishuChatUpdatedEvent = serde_json::from_slice(raw)
        .map_err(|error| NormalizeError::Malformed(error.to_string()))?;
    let body: &ChatUpdatedBody = &envelope.event;
    if envelope.header.event_type != "im.chat.updated_v1" || body.chat_id.trim().is_empty() {
        return Err(NormalizeError::Incomplete(
            "chat update event_type and chat_id are required".to_owned(),
        ));
    }
    let name = body
        .after_change
        .name
        .as_deref()
        .and_then(non_empty)
        .unwrap_or(&body.chat_id);
    Ok(NormalizedChatUpdate {
        chat_id: body.chat_id.trim().to_owned(),
        name: name.trim().to_owned(),
        tenant_key: envelope.header.tenant_key.trim().to_owned(),
        event_id: non_empty(&envelope.header.event_id)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("im.chat.updated:{}", body.chat_id)),
        raw: raw.to_vec(),
    })
}

pub fn normalize_task(raw: &[u8]) -> Result<NormalizedTaskEvent, NormalizeError> {
    let mut envelope: FeishuTaskEvent = serde_json::from_slice(raw)
        .map_err(|error| NormalizeError::Malformed(error.to_string()))?;
    if !envelope.header.event_type.starts_with("task.") {
        return Err(NormalizeError::UnsupportedEvent(
            envelope.header.event_type.clone(),
        ));
    }
    if envelope.event.task_guid.trim().is_empty() {
        return Err(NormalizeError::Incomplete(
            "task_guid is required".to_owned(),
        ));
    }
    if envelope.header.event_type == "task.task.comment.updated_v1" {
        envelope.event.event_types = vec![
            match envelope.event.obj_type {
                1 => "task_comment_create",
                2 => "task_comment_reply",
                3 => "task_comment_update",
                4 => "task_comment_delete",
                _ => {
                    return Err(NormalizeError::Incomplete(
                        "comment obj_type is required".into(),
                    ));
                }
            }
            .to_owned(),
        ];
    }
    let event_id = canonical_task_event_id(&envelope);
    Ok(NormalizedTaskEvent {
        task_id: envelope.event.task_guid.trim().to_owned(),
        event_types: envelope.event.event_types.clone(),
        comment_id: non_empty(&envelope.event.comment_id).map(str::to_owned),
        content: envelope.event.content.clone(),
        tenant_key: envelope.header.tenant_key.trim().to_owned(),
        event_id,
        raw: raw.to_vec(),
    })
}

fn canonical_task_event_id(envelope: &FeishuTaskEvent) -> String {
    let task_id = envelope.event.task_guid.trim();
    if let Some(comment_id) = non_empty(&envelope.event.comment_id) {
        return match envelope.event.obj_type {
            1 | 2 => format!("task.comment:{comment_id}:created"),
            4 => format!("task.comment:{comment_id}:deleted"),
            _ => format!(
                "task.comment:{comment_id}:updated:{}",
                envelope.header.create_time
            ),
        };
    }
    if envelope
        .event
        .event_types
        .iter()
        .any(|value| value == "task_create")
    {
        return format!("task.create:{task_id}");
    }
    let mut event_types = envelope.event.event_types.clone();
    event_types.sort();
    event_types.dedup();
    let event_types = event_types.join(",");
    if let Some(create_time) = non_empty(&envelope.header.create_time) {
        return format!("task.change:{task_id}:{create_time}:{event_types}");
    }
    if let Some(provider_event_id) = non_empty(&envelope.header.event_id) {
        return provider_event_id.to_owned();
    }
    format!("task.change:{task_id}:{event_types}")
}

#[derive(Default)]
struct MentionNormalization {
    mentions: Vec<NormalizedMention>,
    agent_ids: Vec<String>,
    replacements: HashMap<String, String>,
}

fn normalize_mentions(
    message: &EventMessage,
    config: &NormalizationConfig,
) -> MentionNormalization {
    let mut result = MentionNormalization::default();
    let mut seen_agents = HashSet::new();
    for mention in &message.mentions {
        let external_id = mention
            .id
            .preferred()
            .map(|(_, id)| id.to_owned())
            .unwrap_or_default();
        let mentioned_type = mention.mentioned_type.to_ascii_lowercase();
        let agent_id = matches!(mentioned_type.as_str(), "bot" | "app")
            .then(|| config.resolve_agent(&mention.id))
            .flatten();
        let kind = match mentioned_type.as_str() {
            "user" => SenderKind::Human,
            "bot" | "app" if agent_id.is_some() => SenderKind::Agent,
            "bot" | "app" => SenderKind::Bot,
            _ => SenderKind::Unknown,
        };
        let display_name = if mention.name.trim().is_empty() {
            match kind {
                SenderKind::Human => "飞书成员",
                SenderKind::Agent | SenderKind::Bot => "Agent",
                SenderKind::System | SenderKind::Unknown => "成员",
            }
            .to_owned()
        } else {
            mention.name.trim().to_owned()
        };
        if let Some(agent_id) = &agent_id {
            if seen_agents.insert(agent_id.clone()) {
                result.agent_ids.push(agent_id.clone());
            }
            if let Some(marker) = inbound_marker(&display_name, agent_id) {
                result.replacements.insert(mention.key.clone(), marker);
            }
        } else if kind == SenderKind::Human && !mention.key.trim().is_empty() {
            if let Some(marker) = inbound_marker(&display_name, &external_id) {
                result.replacements.insert(mention.key.clone(), marker);
            }
        } else if matches!(kind, SenderKind::Bot | SenderKind::Unknown)
            && !mention.key.trim().is_empty()
            && !display_name.is_empty()
        {
            result
                .replacements
                .insert(mention.key.clone(), format!("@{display_name}"));
        }
        result.mentions.push(NormalizedMention {
            external_id,
            aliases: mention
                .id
                .aliases()
                .into_iter()
                .map(|alias| (alias.kind, alias.id))
                .collect(),
            agent_id,
            display_name,
            kind,
        });
    }
    result
}

fn normalize_sender(sender: &EventSender, config: &NormalizationConfig) -> NormalizedSender {
    normalize_sender_with_identity(&sender.sender_id, &sender.sender_type, config, "")
}

fn normalize_sender_with_identity(
    identity: &Identity,
    sender_type: &str,
    config: &NormalizationConfig,
    display_name: &str,
) -> NormalizedSender {
    let external_id = identity
        .preferred()
        .map(|(_, id)| id.to_owned())
        .unwrap_or_default();
    let agent_id = config.resolve_agent(identity);
    let kind = match sender_type {
        "user" => SenderKind::Human,
        "bot" | "app" if agent_id.is_some() => SenderKind::Agent,
        "bot" | "app" => SenderKind::Bot,
        "system" => SenderKind::System,
        _ => SenderKind::Unknown,
    };
    let sender_id = agent_id.unwrap_or_else(|| external_id.clone());
    NormalizedSender {
        external_id,
        sender_id,
        kind,
        display_name: display_name.trim().to_owned(),
        aliases: identity
            .aliases()
            .into_iter()
            .map(|alias| (alias.kind, alias.id))
            .collect(),
    }
}

fn normalize_content(
    message: &EventMessage,
    replacements: &HashMap<String, String>,
) -> Result<(String, Vec<NormalizedAttachment>), NormalizeError> {
    let raw = message.content.trim();
    if raw.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let parsed = serde_json::from_str::<Value>(raw).ok();
    let mut attachments = Vec::new();
    let content = match message.message_type.as_str() {
        "text" => parsed
            .as_ref()
            .and_then(|value| value.get("text"))
            .and_then(Value::as_str)
            .unwrap_or(raw)
            .to_owned(),
        "image" => {
            let key = parsed
                .as_ref()
                .and_then(|value| value.get("image_key"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !key.is_empty() {
                attachments.push(NormalizedAttachment {
                    kind: "image".to_owned(),
                    key: key.to_owned(),
                    name: "image".to_owned(),
                    content_type: "image/*".to_owned(),
                });
            }
            String::new()
        }
        "file" | "audio" | "media" => {
            let key = parsed
                .as_ref()
                .and_then(|value| value.get("file_key"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let name = parsed
                .as_ref()
                .and_then(|value| value.get("file_name"))
                .and_then(Value::as_str)
                .unwrap_or("attachment");
            if !key.is_empty() {
                attachments.push(NormalizedAttachment {
                    kind: message.message_type.clone(),
                    key: key.to_owned(),
                    name: name.to_owned(),
                    content_type: "application/octet-stream".to_owned(),
                });
            }
            String::new()
        }
        "folder" | "sticker" => {
            let key = parsed
                .as_ref()
                .and_then(|value| value.get("file_key"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !key.is_empty() {
                attachments.push(NormalizedAttachment {
                    kind: message.message_type.clone(),
                    key: key.to_owned(),
                    name: parsed
                        .as_ref()
                        .and_then(|value| value.get("file_name"))
                        .and_then(Value::as_str)
                        .unwrap_or(if message.message_type == "folder" {
                            "folder"
                        } else {
                            "sticker"
                        })
                        .to_owned(),
                    content_type: if message.message_type == "sticker" {
                        "image/*".to_owned()
                    } else {
                        "application/octet-stream".to_owned()
                    },
                });
            }
            String::new()
        }
        "post" | "interactive" => flatten_structured_content(
            parsed.as_ref().unwrap_or(&Value::String(raw.to_owned())),
            &mut attachments,
        ),
        _ => raw.to_owned(),
    };
    let mut result = content;
    for (key, replacement) in replacements {
        if !key.is_empty() {
            result = result.replace(key, replacement);
        }
    }
    Ok((result.trim().to_owned(), attachments))
}

fn flatten_structured_content(
    value: &Value,
    attachments: &mut Vec<NormalizedAttachment>,
) -> String {
    let mut pieces = Vec::new();
    fn walk(value: &Value, pieces: &mut Vec<String>, attachments: &mut Vec<NormalizedAttachment>) {
        match value {
            Value::Array(items) => items
                .iter()
                .for_each(|item| walk(item, pieces, attachments)),
            Value::Object(object) => {
                // Older CardKit payloads wrap the actual card in a
                // `user_dsl` JSON string.  Decode that wrapper before walking
                // the common card tree so interactive messages do not lose
                // their markdown or attachment nodes.
                if let Some(user_dsl) = object.get("user_dsl") {
                    match user_dsl {
                        Value::String(encoded) => {
                            if let Ok(decoded) = serde_json::from_str::<Value>(encoded) {
                                walk(&decoded, pieces, attachments);
                                return;
                            }
                        }
                        value if !value.is_null() => {
                            walk(value, pieces, attachments);
                            return;
                        }
                        _ => {}
                    }
                }
                let tag = object
                    .get("tag")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match tag {
                    "markdown" | "lark_md" | "plain_text" | "text" => {
                        if let Some(text) = object
                            .get("content")
                            .or_else(|| object.get("text"))
                            .and_then(Value::as_str)
                        {
                            pieces.push(text.to_owned());
                        }
                    }
                    "a" => {
                        let text = object
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let href = object
                            .get("href")
                            .or_else(|| object.get("url"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        pieces.push(if href.is_empty() {
                            text.to_owned()
                        } else {
                            format!("[{text}]({href})")
                        });
                    }
                    "at" => pieces.push(format!(
                        "@{}",
                        object
                            .get("user_name")
                            .or_else(|| object.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("成员")
                    )),
                    "img" | "image" => {
                        if let Some(key) = object
                            .get("img_key")
                            .or_else(|| object.get("image_key"))
                            .and_then(Value::as_str)
                        {
                            attachments.push(NormalizedAttachment {
                                kind: "image".to_owned(),
                                key: key.to_owned(),
                                name: object
                                    .get("alt")
                                    .and_then(Value::as_str)
                                    .unwrap_or("image")
                                    .to_owned(),
                                content_type: "image/*".to_owned(),
                            });
                        }
                    }
                    "file" => {
                        if let Some(key) = object.get("file_key").and_then(Value::as_str) {
                            attachments.push(NormalizedAttachment {
                                kind: "file".to_owned(),
                                key: key.to_owned(),
                                name: object
                                    .get("file_name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("attachment")
                                    .to_owned(),
                                content_type: "application/octet-stream".to_owned(),
                            });
                        }
                    }
                    "hr" => pieces.push("---".to_owned()),
                    "code_block" => pieces.push(format!(
                        "```\n{}\n```",
                        object
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    )),
                    _ => {
                        for key in [
                            "zh_cn", "en_us", "ja_jp", "title", "body", "elements", "content",
                            "fields", "text",
                        ] {
                            if let Some(child) = object.get(key) {
                                walk(child, pieces, attachments);
                            }
                        }
                    }
                }
            }
            Value::String(text) => pieces.push(text.clone()),
            _ => {}
        }
    }
    walk(value, &mut pieces, attachments);
    pieces.join("\n\n").trim().to_owned()
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_timestamp_ms(value: Option<&str>) -> Option<i64> {
    let value = value?.trim();
    let timestamp = value.parse::<i64>().ok()?;
    if timestamp > 10_000_000_000 {
        Some(timestamp)
    } else {
        Some(timestamp * 1000)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NormalizeError {
    Malformed(String),
    UnsupportedEvent(String),
    Incomplete(String),
    AppMismatch { expected: String, actual: String },
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => write!(formatter, "malformed Feishu event: {message}"),
            Self::UnsupportedEvent(event) => write!(formatter, "unsupported Feishu event {event}"),
            Self::Incomplete(message) => write!(formatter, "incomplete Feishu event: {message}"),
            Self::AppMismatch { expected, actual } => write!(
                formatter,
                "Feishu event app_id {actual} does not match configured app {expected}"
            ),
        }
    }
}

impl std::error::Error for NormalizeError {}
