use std::fmt;

/// Stable failures returned by checked domain boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DomainError {
    InvalidIdentifier { kind: &'static str, value: String },
    InvalidPositiveId { kind: &'static str, value: i64 },
    InvalidTimestamp { value: i64 },
    TimestampOverflow,
    InvalidScheduleInterval { minutes: u32 },
}

impl DomainError {
    /// Machine-stable error code suitable for protocol translation.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidIdentifier { .. } => "invalid_identifier",
            Self::InvalidPositiveId { .. } => "invalid_positive_id",
            Self::InvalidTimestamp { .. } => "invalid_timestamp",
            Self::TimestampOverflow => "timestamp_overflow",
            Self::InvalidScheduleInterval { .. } => "invalid_schedule_interval",
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for DomainError {}
