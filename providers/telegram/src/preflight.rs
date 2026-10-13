//! Fail-closed guard for Telegram live mutations.
//!
//! The new provider must not race the legacy Desktop/daemon consumer for the
//! same Bot or database.  The probe itself is supplied by testkit/host code;
//! this module provides the typed, non-bypassable result that a live adapter
//! can require before its first write or polling call.

use std::fmt;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LegacyConsumerState {
    pub desktop_running: bool,
    pub launch_agent_loaded: bool,
    pub daemon_running: bool,
    pub database_open_handles: usize,
}

impl LegacyConsumerState {
    pub fn is_clear(&self) -> bool {
        !self.desktop_running
            && !self.launch_agent_loaded
            && !self.daemon_running
            && self.database_open_handles == 0
    }
    pub fn blocking_reasons(&self) -> Vec<&'static str> {
        let mut reasons = Vec::new();
        if self.desktop_running {
            reasons.push("legacy Desktop process is running");
        }
        if self.launch_agent_loaded {
            reasons.push("legacy com.lankide.daemon LaunchAgent is loaded");
        }
        if self.daemon_running {
            reasons.push("legacy daemon process is running");
        }
        if self.database_open_handles > 0 {
            reasons.push("Telegram database has open handles");
        }
        reasons
    }
}

pub trait LegacyConsumerProbe {
    fn inspect(&self) -> Result<LegacyConsumerState, ProbeError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationPermit {
    run_id: String,
}

impl MutationPermit {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn marker(&self) -> String {
        format!("[lankide-e2e:{}]", self.run_id)
    }

    /// Recheck the same four legacy-consumer conditions immediately before a
    /// mutation.  Hosts should call this again after polling and before every
    /// live write; a permit never turns a later blocked probe into success.
    pub fn recheck<P: LegacyConsumerProbe>(&self, probe: &P) -> Result<(), GuardError> {
        let state = probe.inspect().map_err(GuardError::Probe)?;
        if state.is_clear() {
            Ok(())
        } else {
            Err(GuardError::Blocked(state))
        }
    }
}

pub fn require_clear<P: LegacyConsumerProbe>(
    probe: &P,
    run_id: impl Into<String>,
) -> Result<MutationPermit, GuardError> {
    let run_id = run_id.into();
    if run_id.trim().is_empty() {
        return Err(GuardError::MissingRunID);
    }
    let state = probe.inspect().map_err(GuardError::Probe)?;
    if !state.is_clear() {
        return Err(GuardError::Blocked(state));
    }
    Ok(MutationPermit { run_id })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeError {
    Unavailable(String),
    Failed(String),
}
impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for ProbeError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardError {
    MissingRunID,
    Probe(ProbeError),
    Blocked(LegacyConsumerState),
}
impl fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRunID => {
                formatter.write_str("Telegram live mutation requires an E2E run ID")
            }
            Self::Probe(error) => write!(
                formatter,
                "Telegram legacy-consumer preflight failed: {error}"
            ),
            Self::Blocked(state) => write!(
                formatter,
                "Telegram live mutation blocked by legacy consumer state: {}",
                state.blocking_reasons().join(", ")
            ),
        }
    }
}
impl std::error::Error for GuardError {}
