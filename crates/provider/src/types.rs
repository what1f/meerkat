use crate::{ProviderError, ProviderFuture};
use lankide_core::{AgentId, AttentionLevel, ChannelAbility, ChannelId, MessageId, ProviderId};
use lankide_protocol::ResourceRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, fmt, sync::Arc};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub name: String,
    pub description: String,
    pub version: String,
    pub icon: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderOnboarding {
    pub step: String,
    pub status: ProviderOnboardingStatus,
    pub title: String,
    pub instruction: String,
    pub fields: Vec<ProviderOnboardingField>,
    pub opaque_payload: Value,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    /// Provider-owned Agent configuration persisted by Runtime only after the
    /// external onboarding flow has completed successfully.
    pub agent_config_updates: Value,
}

impl ProviderOnboarding {
    pub fn ready(opaque_payload: Value) -> Self {
        Self {
            step: "configuration".into(),
            status: ProviderOnboardingStatus::Ready,
            title: String::new(),
            instruction: String::new(),
            fields: Vec::new(),
            opaque_payload,
            error_code: None,
            error_message: None,
            agent_config_updates: Value::Object(Default::default()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderOnboardingStatus {
    ActionRequired,
    Verifying,
    Ready,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderOnboardingField {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub required: bool,
    pub value: String,
    pub help: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelDefinition {
    pub id: ChannelId,
    pub name: String,
    pub default_attention: ChannelDefaultAttention,
    pub attention_levels: BTreeSet<AttentionLevel>,
    pub abilities: BTreeSet<ChannelAbility>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelDefaultAttention {
    DirectMessage,
    Follow,
    Mention,
    Mute,
}

impl ChannelDefinition {
    pub fn new(
        id: ChannelId,
        name: impl Into<String>,
        default_attention: ChannelDefaultAttention,
        attention: impl IntoIterator<Item = AttentionLevel>,
        abilities: impl IntoIterator<Item = ChannelAbility>,
    ) -> Result<Self, ProviderError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ProviderError::InvalidInput {
                field: "channel_name",
            });
        }
        Ok(Self {
            id,
            name,
            default_attention,
            attention_levels: attention.into_iter().collect(),
            abilities: abilities.into_iter().collect(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventSenderType {
    Human,
    Agent,
    Bot,
    System,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSender {
    pub id: String,
    pub kind: EventSenderType,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderEvent {
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub sender: EventSender,
    pub kind: ProviderEventKind,
    pub provider_data: Value,
}
impl ProviderEvent {
    /// Stable identity supplied by the Provider adapter for replay deduplication.
    pub fn event_id(&self) -> Option<&str> {
        self.provider_data
            .get("event_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
    }

    /// Original event time normalized to milliseconds by this common seam.
    pub fn occurred_at_millis(&self) -> Option<i64> {
        self.provider_data
            .get("occurred_at_ms")
            .and_then(Value::as_i64)
            .or_else(|| {
                self.provider_data
                    .get("occurred_at_s")
                    .and_then(Value::as_i64)
                    .and_then(|seconds| seconds.checked_mul(1_000))
            })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderEventKind {
    Message(MessageEvent),
    Reaction(ReactionEvent),
    Custom(CustomEvent),
}
#[derive(Clone, Debug, PartialEq)]
pub struct MessageEvent {
    pub provider_message_id: String,
    pub content: String,
    pub reply_to_provider_message_id: Option<String>,
    pub mentioned_agent_ids: Vec<AgentId>,
    pub attachments: Vec<ResourceRef>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactionAction {
    Add,
    Remove,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionEvent {
    pub target_provider_message_id: String,
    pub sender: EventSender,
    pub reaction: String,
    pub action: ReactionAction,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CustomEvent {
    pub reason: String,
    pub target_agent_ids: Vec<AgentId>,
    pub subject_id: String,
    pub subject_name: String,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JsonSchema(Value);
impl JsonSchema {
    pub fn new(value: Value) -> Result<Self, ProviderError> {
        let Some(schema) = value.as_object() else {
            return Err(ProviderError::InvalidSchema { location: "root" });
        };
        if schema.get("type").is_some_and(|v| !v.is_string())
            || schema
                .get("required")
                .is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
            || schema.get("properties").is_some_and(|v| !v.is_object())
        {
            return Err(ProviderError::InvalidSchema { location: "root" });
        }
        Ok(Self(value))
    }
    pub fn value(&self) -> &Value {
        &self.0
    }
    pub fn validate(&self, value: &Value) -> Result<(), ProviderError> {
        validate(&self.0, value)
    }
}
fn validate(schema: &Value, value: &Value) -> Result<(), ProviderError> {
    let object = schema
        .as_object()
        .ok_or(ProviderError::InvalidSchema { location: "root" })?;
    let matches = match object.get("type").and_then(Value::as_str) {
        None => true,
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("number") => value.is_number(),
        Some("null") => value.is_null(),
        Some(_) => return Err(ProviderError::InvalidSchema { location: "type" }),
    };
    if !matches {
        return Err(ProviderError::SchemaMismatch { location: "value" });
    }
    if let Some(instance) = value.as_object() {
        if object
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|key| !instance.contains_key(key))
            })
        {
            return Err(ProviderError::SchemaMismatch {
                location: "required",
            });
        }
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            for (key, child_schema) in properties {
                if let Some(child) = instance.get(key) {
                    validate(child_schema, child)?;
                }
            }
        }
    }
    Ok(())
}

pub trait InvocationHandler: Send + Sync {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value>;
}

/// Read-only metadata for an interface registered by an initialized Provider.
/// Invocation handlers and Provider state deliberately remain inside the Host.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct InterfaceCatalogEntry {
    pub id: String,
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Clone)]
pub struct InterfaceDefinition {
    pub id: String,
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
    pub handler: Arc<dyn InvocationHandler>,
}
#[derive(Clone)]
pub struct ToolDefinition {
    pub id: String,
    pub description: String,
    pub input_schema: JsonSchema,
    pub output_schema: JsonSchema,
    pub handler: Arc<dyn InvocationHandler>,
}
#[derive(Clone)]
pub struct CliCommandDefinition {
    pub id: String,
    pub description: String,
    pub handler: Arc<dyn InvocationHandler>,
}
macro_rules! definition_debug {
    ($name:ty, $($field:ident),+) => { impl fmt::Debug for $name { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct(stringify!($name)); $(d.field(stringify!($field), &self.$field);)+ d.finish_non_exhaustive()
    } } };
}
definition_debug!(InterfaceDefinition, id, input_schema, output_schema);
definition_debug!(ToolDefinition, id, description, input_schema, output_schema);
definition_debug!(CliCommandDefinition, id, description);

#[derive(Clone, Debug, PartialEq)]
pub struct MessageAttachment {
    pub resource: ResourceRef,
    pub caption: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendMessageStatus {
    Sent,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMessageResult {
    pub status: SendMessageStatus,
    pub provider_message_id: Option<String>,
}
impl SendMessageResult {
    pub fn sent(id: impl Into<String>) -> Result<Self, ProviderError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(ProviderError::InvalidInput {
                field: "provider_message_id",
            });
        }
        Ok(Self {
            status: SendMessageStatus::Sent,
            provider_message_id: Some(id),
        })
    }
    pub const fn unknown() -> Self {
        Self {
            status: SendMessageStatus::Unknown,
            provider_message_id: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SendMessageSpec {
    pub channel_id: ChannelId,
    pub chat_id: String,
    pub message_id: MessageId,
    pub idempotency_key: String,
    pub content: String,
    pub reply_to: Option<MessageId>,
    pub mentions: Vec<AgentId>,
    pub attachments: Vec<MessageAttachment>,
    pub force: bool,
    pub provider_agent_config: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RecallMessageSpec {
    pub channel_id: ChannelId,
    pub chat_id: String,
    pub message_id: MessageId,
    pub provider_message_id: String,
    pub provider_agent_config: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EditMessageSpec {
    pub channel_id: ChannelId,
    pub chat_id: String,
    pub message_id: MessageId,
    pub provider_message_id: String,
    pub content: String,
    pub provider_agent_config: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AutoReactionSpec {
    pub auto_reaction: String,
    pub channel_id: ChannelId,
    pub chat_id: String,
    pub trigger_messages: Vec<MessageId>,
    pub provider_agent_config: Value,
}

/// A platform reaction identifier supported by this Provider.
#[derive(Clone, Debug)]
pub struct ReactionDefinition {
    pub id: String,
    pub name: String,
    pub emoji: String,
}
