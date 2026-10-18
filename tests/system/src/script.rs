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
    OfflineEvents {
        count: usize,
        bytes: usize,
        text: String,
    },
    ReplyBeforeTerminal {
        text: String,
    },
    DeferredPrivateReply {
        text: String,
    },
    LinkReply {
        text: String,
    },
    /// Emits a capability request and terminal result without waiting for a response.
    /// Used to replay a Machine's buffered output after authorization changes.
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
    Greeting,
    HangOnStart,
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
    ProxyScenario {
        binary: String,
    },
    FailedProviderTool {
        via_cli: bool,
        rpc_error: bool,
        text: String,
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
    models: Option<Vec<NativeModel>>,
    controls: PathBuf,
}
impl ScriptedHarness {
    pub fn new(journal: PathBuf) -> Self {
        Self {
            models: None,
            controls: std::env::var_os("LANKIDE_SYSTEM_CONTROL_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| journal.parent().unwrap().join("scripted-members")),
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
    pub fn with_models(mut self, models: Vec<NativeModel>) -> Self {
        self.models = Some(models);
        self
    }
    fn prepare(&self, mut request: SessionRequest) -> Result<SessionRequest, HarnessError> {
        // Machine's public working directory is workspace/agents/member. Use that
        // identity; browser-created members need no editable test configuration.
        let home = &request.working_directory;
        let agent = home
            .file_name()
            .ok_or_else(|| HarnessError::new(HarnessErrorCode::InvalidConfig, "member home"))?;
        let workspace = home
            .ancestors()
            .nth(2)
            .and_then(|p| p.file_name())
            .ok_or_else(|| HarnessError::new(HarnessErrorCode::InvalidConfig, "workspace home"))?;
        let dir = self.controls.join(workspace).join(agent);
        std::fs::create_dir_all(&dir)
            .map_err(|e| HarnessError::new(HarnessErrorCode::InvalidConfig, e.to_string()))?;
        let env = &mut request.config.environment;
        for (key, file) in [
            ("LANKIDE_SYSTEM_INPUTS", "inputs.jsonl"),
            ("LANKIDE_SYSTEM_CONTROLS", "controls.jsonl"),
            ("LANKIDE_SYSTEM_STEERS", "steers.jsonl"),
            ("LANKIDE_SYSTEM_RELEASE", "release"),
        ] {
            env.entry(key.into())
                .or_insert_with(|| dir.join(file).display().to_string());
        }
        env.entry("LANKIDE_SYSTEM_SCRIPT".into())
            .or_insert_with(|| serde_json::to_string(&Script::reply("hi")).unwrap());
        env.insert(
            "LANKIDE_SYSTEM_HOLD".into(),
            dir.join("hold").display().to_string(),
        );
        Ok(request)
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
    fn models<'a>(&'a self, _config: &'a HarnessConfig) -> HarnessFuture<'a, Vec<NativeModel>> {
        Box::pin(async move {
            if let Some(models) = &self.models {
                return Ok(models.clone());
            }
            Ok(vec![
                NativeModel {
                    id: "scripted".into(),
                    name: "Scripted".into(),
                    description: String::new(),
                    thinking_levels: vec!["low".into(), "high".into()],
                },
                NativeModel {
                    id: "short".into(),
                    name: "Short".into(),
                    description: String::new(),
                    thinking_levels: vec!["medium".into()],
                },
                NativeModel {
                    id: "plain".into(),
                    name: "Plain".into(),
                    description: String::new(),
                    thinking_levels: vec![],
                },
                // The same name from two sources, and enough models to fill a picker column.
                NativeModel {
                    id: "north/twin".into(),
                    name: "Twin Flash Preview With A Very Long Name".into(),
                    description: String::new(),
                    thinking_levels: vec![
                        "low".into(),
                        "medium".into(),
                        "high".into(),
                        "max".into(),
                    ],
                },
                NativeModel {
                    id: "south/twin".into(),
                    name: "Twin Flash Preview With A Very Long Name".into(),
                    description: String::new(),
                    thinking_levels: vec!["low".into()],
                },
            ]
            .into_iter()
            .chain((0..8).map(|n| NativeModel {
                id: format!("filler-{n}"),
                name: format!("Filler {n}"),
                description: String::new(),
                thinking_levels: vec![],
            }))
            .collect())
        })
    }
    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        let request = match self.prepare(request) {
            Ok(request) => request,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        if let Some(error) = Self::spawn_failure(&request) {
            Box::pin(async move { Err(error) })
        } else {
            self.process.start(request)
        }
    }
    fn start_before<'a>(
        &'a self,
        request: SessionRequest,
        deadline: std::time::Instant,
    ) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        if let Some(error) = Self::spawn_failure(&request) {
            Box::pin(async move { Err(error) })
        } else {
            self.process.start_before(request, deadline)
        }
    }
    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        let request = match self.prepare(request) {
            Ok(request) => request,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
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
