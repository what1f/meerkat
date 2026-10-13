use std::fmt;

/// Stable failures returned by checked domain boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DomainError {
    InvalidIdentifier {
        kind: &'static str,
        value: String,
    },
    InvalidPositiveId {
        kind: &'static str,
        value: i64,
    },
    InvalidTimestamp {
        value: i64,
    },
    TimestampOverflow,
    EmptyField {
        field: &'static str,
    },
    InvalidAttachmentHash,
    CrossWorkspaceReference {
        entity: &'static str,
        expected_workspace_id: String,
        actual_workspace_id: String,
    },
    ProviderMismatch {
        expected_provider_id: String,
        actual_provider_id: String,
    },
    ChannelMismatch {
        expected_channel_id: String,
        actual_channel_id: String,
    },
    ChatMismatch {
        expected_chat_id: String,
        actual_chat_id: String,
    },
    AgentMismatch {
        expected_agent_id: String,
        actual_agent_id: String,
    },
    UnsupportedAttention {
        channel_id: String,
        level: &'static str,
    },
    MissingScheduleInterval,
    UnexpectedScheduleInterval {
        level: &'static str,
    },
    InvalidScheduleInterval {
        minutes: u32,
    },
    CursorRegression {
        current_message_id: i64,
        proposed_message_id: i64,
    },
    MessageOrderRegression {
        previous_message_id: i64,
        proposed_message_id: i64,
    },
    DuplicateChat {
        chat_id: String,
    },
    MentionedMessageNotUnread {
        message_id: i64,
    },
    EmptyUnreadSnapshot,
    EmptyTriggerReasons,
    TriggerOutsideSnapshot {
        chat_id: String,
        message_id: Option<i64>,
    },
}

impl DomainError {
    /// Machine-stable error code suitable for protocol translation.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidIdentifier { .. } => "invalid_identifier",
            Self::InvalidPositiveId { .. } => "invalid_positive_id",
            Self::InvalidTimestamp { .. } => "invalid_timestamp",
            Self::TimestampOverflow => "timestamp_overflow",
            Self::EmptyField { .. } => "empty_field",
            Self::InvalidAttachmentHash => "invalid_attachment_hash",
            Self::CrossWorkspaceReference { .. } => "cross_workspace_reference",
            Self::ProviderMismatch { .. } => "provider_mismatch",
            Self::ChannelMismatch { .. } => "channel_mismatch",
            Self::ChatMismatch { .. } => "chat_mismatch",
            Self::AgentMismatch { .. } => "agent_mismatch",
            Self::UnsupportedAttention { .. } => "unsupported_attention",
            Self::MissingScheduleInterval => "missing_schedule_interval",
            Self::UnexpectedScheduleInterval { .. } => "unexpected_schedule_interval",
            Self::InvalidScheduleInterval { .. } => "invalid_schedule_interval",
            Self::CursorRegression { .. } => "cursor_regression",
            Self::MessageOrderRegression { .. } => "message_order_regression",
            Self::DuplicateChat { .. } => "duplicate_chat",
            Self::MentionedMessageNotUnread { .. } => "mentioned_message_not_unread",
            Self::EmptyUnreadSnapshot => "empty_unread_snapshot",
            Self::EmptyTriggerReasons => "empty_trigger_reasons",
            Self::TriggerOutsideSnapshot { .. } => "trigger_outside_snapshot",
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DomainError {}
