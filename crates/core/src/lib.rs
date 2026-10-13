#![forbid(unsafe_code)]

//! Stable, persistence-independent domain vocabulary for Lankide.
//!
//! Every aggregate in this crate is built through a checked public boundary.
//! Storage, transports, Providers, Harnesses, and UI code are deliberately not
//! dependencies of this crate.

mod collaboration;
mod error;
mod identity;
mod markdown_mention;
mod unread;

pub use collaboration::{
    Agent, AgentChatCursor, Attachment, AttentionLevel, AttentionSetting, Channel, ChannelAbility,
    Chat, Mention, Message, MessageAuthor, MessageAuthorType, MessageStatus, MessageTimeline,
    Reply, ScheduleInterval, Workspace, WorkspaceStatus,
};
pub use error::DomainError;
pub use identity::{
    AgentId, ChannelId, ChatId, ExecutionId, MessageId, PersonId, ProviderId, ResourceId,
    SnapshotId, UnixMillis, WorkspaceId,
};
pub use markdown_mention::{MarkdownMention, markdown_mention, markdown_mentions};
pub use unread::{
    ExecutionInput, TriggerReason, UnreadChatInput, UnreadChatSnapshot, UnreadSnapshot,
    UnreadSnapshotInput,
};

pub const CRATE_NAME: &str = "lankide-core";

/// Canonical Tool names are portable across every supported Harness.
pub fn is_canonical_tool_name(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
