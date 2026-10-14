use lankide_harness::*;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[path = "../../../crates/harness/tests/support/process.rs"]
mod process_support;
use process_support::{ProcessHarnessFactory, ProcessHarnessSpec};

/// Scenarios select behaviour through each Agent's normal Harness environment.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "behaviour", rename_all = "snake_case")]
pub enum Script {
    Reply {
        text: String,
        delay_ms: u64,
        deltas: Vec<String>,
        held: bool,
    },
    LinkReply {
        text: String,
    },
    /// Emits a capability request and terminal result without waiting for a response.
    /// Used to replay a Computer's buffered output after authorization changes.
    BufferedReply {
        text: String,
    },
    FailSequence {
        texts: Vec<String>,
        reply: String,
    },
    FailSpawn {
        text: String,
    },
    FailMidTurn {
        text: String,
    },
    Hang,
    HangOnce {
        text: String,
    },
    Progress {
        duration_ms: u64,
        interval_ms: u64,
    },
    SpawnChild {
        text: String,
        delay_ms: u64,
    },
    Capability {
        arguments: Vec<String>,
        text: String,
    },
}
impl Script {
    pub fn reply(text: impl Into<String>) -> Self {
        Self::Reply {
            text: text.into(),
            delay_ms: 0,
            deltas: Vec::new(),
            held: false,
        }
    }
    /// Keep the first reply pending until the scenario calls `release_run`.
    pub fn held(mut self) -> Self {
        if let Self::Reply { held, .. } = &mut self {
            *held = true;
        }
        self
    }
    pub fn after(mut self, delay: Duration) -> Self {
        match &mut self {
            Self::Reply { delay_ms, .. } | Self::SpawnChild { delay_ms, .. } => {
                *delay_ms = delay.as_millis() as u64
            }
            _ => panic!("delay requires a reply script"),
        }
        self
    }
    pub fn streaming(mut self, deltas: impl IntoIterator<Item = String>) -> Self {
        if let Self::Reply { deltas: events, .. } = &mut self {
            *events = deltas.into_iter().collect();
        }
        self
    }
}

pub(crate) struct ScriptedHarness {
    process: ProcessHarnessFactory,
}
impl ScriptedHarness {
    pub fn new(journal: PathBuf) -> Self {
        Self {
            process: ProcessHarnessFactory::new(
                HarnessInfo::new("pi", "Scripted process").expect("harness id"),
                ProcessHarnessSpec {
                    pid_journal: Some(journal),
                    cancel_grace: Duration::from_millis(100),
                    close_grace: Duration::from_millis(100),
                    ..Default::default()
                },
            ),
        }
    }
    fn spawn_failure(request: &SessionRequest) -> Option<HarnessError> {
        let script = request.config.environment.get("LANKIDE_SYSTEM_SCRIPT")?;
        if let Script::FailSpawn { text } = serde_json::from_str(script).ok()? {
            let mut error = HarnessError::new(HarnessErrorCode::SpawnFailed, text.clone());
            error.failure = scripted_failure(&text);
            Some(error)
        } else {
            None
        }
    }
}
impl HarnessFactory for ScriptedHarness {
    fn info(&self) -> &HarnessInfo {
        self.process.info()
    }
    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        self.process.probe(config)
    }
    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        if let Some(error) = Self::spawn_failure(&request) {
            Box::pin(async move { Err(error) })
        } else {
            self.process.start(request)
        }
    }
    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        if let Some(error) = Self::spawn_failure(&request) {
            Box::pin(async move { Err(error) })
        } else {
            self.process.resume(request)
        }
    }
}

pub fn scripted_failure(text: &str) -> HarnessFailure {
    let category = if text.contains("model not found") {
        FailureCategory::Configuration
    } else if text.contains("rate limit") {
        FailureCategory::RateLimit
    } else if text.contains("ownership conflict") {
        FailureCategory::Conflict
    } else if text.contains("transient") {
        FailureCategory::Transient
    } else if text.contains("authentication") {
        FailureCategory::Authentication
    } else if text.contains("timeout") {
        FailureCategory::Timeout
    } else if text.contains("cancelled") {
        FailureCategory::Cancelled
    } else {
        FailureCategory::Agent
    };
    HarnessFailure {
        code: "scripted_failure".into(),
        category,
        retryable: matches!(
            category,
            FailureCategory::RateLimit | FailureCategory::Conflict
        ),
        detail: text.into(),
    }
}
