use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PersistedSession {
    pub harness_id: String,
    pub native_session_id: String,
    pub config_fingerprint: String,
    pub recovery: SessionRecovery,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionRecovery {
    Ready,
    Interrupted,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct PersistedState {
    pub sessions: BTreeMap<String, PersistedSession>,
    #[serde(default)]
    pub accepted_assignments: BTreeMap<String, PersistedAssignment>,
    #[serde(default)]
    pub pending_events: BTreeMap<String, BTreeMap<u64, PersistedEvent>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PersistedAssignment {
    pub fingerprint: String,
    pub lifecycle: AssignmentLifecycle,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssignmentLifecycle {
    Active,
    Terminal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PersistedEvent {
    pub execution_id: i64,
    pub attempt: u32,
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: i32,
    pub payload: Vec<u8>,
}

impl PersistedState {
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "state_path_has_no_parent")
        })?;
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(
            &temporary,
            serde_json::to_vec(self)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )?;
        fs::rename(temporary, path)
    }
}

pub(crate) fn assignment_key(execution_id: i64, attempt: u32) -> String {
    format!("{execution_id}:{attempt}")
}
