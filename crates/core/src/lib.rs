#![forbid(unsafe_code)]

//! Stable, persistence-independent domain vocabulary for Meerkat.
//!
//! Every aggregate in this crate is built through a checked public boundary.
//! Storage, transports, Providers, Harnesses, and UI code are deliberately not
//! dependencies of this crate.

mod attention;
mod channel;
mod error;
mod identity;
mod markdown_mention;
mod message;
mod resource;
mod team;

pub use attention::{AttentionLevel, ScheduleInterval};
pub use channel::ChannelAbility;
pub use error::DomainError;
pub use identity::{
    AgentId, ChannelId, ExecutionId, MessageId, PersonId, ProviderId, ResourceId, UnixMillis,
    WorkspaceId,
};
pub use markdown_mention::{markdown_mention, markdown_mentions};
pub use message::{MessageAuthorType, ReactionAction};
pub use resource::ResourceRef;
pub use team::TeamRole;

pub const CRATE_NAME: &str = "meerkat-core";
pub const PROVIDER_CAPABILITY_TASK_SYSTEM: &str = "task_system";

/// Canonical Tool names are portable across every supported Harness.
pub fn is_canonical_tool_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A short chat notice, selected from the triggering message's language.
pub fn agent_failure_notice(trigger: &str) -> &'static str {
    if trigger
        .chars()
        .any(|c| matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0x20000..=0x323af))
    {
        "我未能处理这条消息，请负责人在 Meerkat 中检查。"
    } else {
        "I couldn’t handle this message; please ask my owner to check Meerkat."
    }
}
