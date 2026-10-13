use crate::{DomainError, UnixMillis};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AttentionLevel {
    Follow,
    Schedule,
    Mention,
}

impl AttentionLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Schedule => "schedule",
            Self::Mention => "mention",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleInterval {
    Minutes5,
    Minutes15,
    Minutes30,
    Hour1,
    Hours2,
    Hours4,
    Hours8,
    Hours24,
}

impl ScheduleInterval {
    pub const fn minutes(self) -> u32 {
        match self {
            Self::Minutes5 => 5,
            Self::Minutes15 => 15,
            Self::Minutes30 => 30,
            Self::Hour1 => 60,
            Self::Hours2 => 120,
            Self::Hours4 => 240,
            Self::Hours8 => 480,
            Self::Hours24 => 1_440,
        }
    }

    pub fn next_boundary(self, now: UnixMillis) -> Result<UnixMillis, DomainError> {
        let interval = i64::from(self.minutes()) * 60_000;
        let periods = now
            .get()
            .checked_div(interval)
            .and_then(|value| value.checked_add(1))
            .ok_or(DomainError::TimestampOverflow)?;
        let boundary = periods
            .checked_mul(interval)
            .ok_or(DomainError::TimestampOverflow)?;
        UnixMillis::new(boundary)
    }
}

impl TryFrom<u32> for ScheduleInterval {
    type Error = DomainError;

    fn try_from(minutes: u32) -> Result<Self, Self::Error> {
        match minutes {
            5 => Ok(Self::Minutes5),
            15 => Ok(Self::Minutes15),
            30 => Ok(Self::Minutes30),
            60 => Ok(Self::Hour1),
            120 => Ok(Self::Hours2),
            240 => Ok(Self::Hours4),
            480 => Ok(Self::Hours8),
            1_440 => Ok(Self::Hours24),
            _ => Err(DomainError::InvalidScheduleInterval { minutes }),
        }
    }
}
