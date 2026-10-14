use crate::HARNESS_ID;
use crate::mcp::{McpCall, McpServer};
use lankide_harness::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    HarnessAvailability, HarnessCapability, HarnessConfig, HarnessError, HarnessErrorCode,
    HarnessFactory, HarnessFuture, HarnessInfo, HarnessSession, ProbeResult, SessionEntry,
    SessionHealth, SessionRequest, SteerReceipt, ToolResultReceipt, ToolResultRequest,
};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc as std_mpsc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Clone, Debug)]
pub struct ClaudeCodeHarness {
    info: HarnessInfo,
    pid_journal: Option<PathBuf>,
}

impl ClaudeCodeHarness {
    pub fn new() -> Self {
        Self {
            info: HarnessInfo::new(HARNESS_ID, "Claude Code")
                .expect("valid static Claude Code Harness info"),
            pid_journal: None,
        }
    }

    pub fn with_pid_journal(mut self, path: impl Into<PathBuf>) -> Self {
        self.pid_journal = Some(path.into());
        self
    }

    fn validate(&self, config: &HarnessConfig) -> Result<(), HarnessError> {
        config.validate()?;
        if config.harness_id != HARNESS_ID {
            return Err(HarnessError::invalid_config(
                "Claude Code requires harness_id=claude_code",
            ));
        }
        if !matches!(
            config.thinking_level.as_str(),
            "off" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            return Err(HarnessError::invalid_config(
                "unsupported Claude Code thinking level",
            ));
        }
        Ok(())
    }

    fn open(
        &self,
        request: SessionRequest,
        resume: bool,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        self.validate(&request.config)?;
        match (resume, request.native_session_id.as_deref()) {
            (true, None | Some("")) => {
                return Err(HarnessError::invalid_config(
                    "resume requires native_session_id",
                ));
            }
            (false, Some(_)) => {
                return Err(HarnessError::invalid_config(
                    "start does not accept native_session_id",
                ));
            }
            _ => {}
        }
        let native_session_id = request.native_session_id.clone().unwrap_or_else(new_uuid);
        if !valid_uuid(&native_session_id) {
            return Err(HarnessError::invalid_config(
                "Claude Code native_session_id must be a UUID",
            ));
        }
        let paths = SessionPaths::prepare(&request)?;
        if resume && !session_artifact_exists(&paths.config_dir, &native_session_id) {
            return Err(harness_error(
                HarnessErrorCode::BindFailed,
                "Claude Code session artifact is unavailable for resume",
            ));
        }
        let mcp = McpServer::start(&paths.artifact_dir, request.tools.clone())?;
        let (mcp_path, calls, mcp_stop) = mcp.into_parts();
        let inner = Arc::new(Inner {
            entry: Mutex::new(SessionEntry {
                harness_id: HARNESS_ID.to_owned(),
                native_session_id,
                config_fingerprint: request.config.config_fingerprint.clone(),
                health: SessionHealth::Healthy,
            }),
            request,
            paths,
            mcp_path,
            mcp_stop: Mutex::new(Some(mcp_stop)),
            child: Mutex::new(None),
            stdin: Mutex::new(None),
            active: Mutex::new(None),
            pending_tools: Mutex::new(HashMap::new()),
            applied_tools: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            pid_journal: self.pid_journal.clone(),
        });
        spawn_tool_dispatch(Arc::downgrade(&inner), calls);
        Ok(Box::new(ClaudeCodeSession { inner }))
    }
}

impl Default for ClaudeCodeHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessFactory for ClaudeCodeHarness {
    fn info(&self) -> &HarnessInfo {
        &self.info
    }

    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            config.validate_executable()?;
            let output = Command::new(&config.cli_path)
                .arg("--version")
                .envs(&config.environment)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            let (availability, version, reason) = match output {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    let help = Command::new(&config.cli_path)
                        .arg("--help")
                        .envs(&config.environment)
                        .stdin(Stdio::null())
                        .stderr(Stdio::null())
                        .output();
                    match help {
                        Ok(help) if help.status.success() => {
                            let text = String::from_utf8_lossy(&help.stdout);
                            let flags = [
                                "--output-format",
                                "--input-format",
                                "--session-id",
                                "--resume",
                                "--mcp-config",
                                "--system-prompt",
                            ];
                            if flags.iter().all(|flag| text.contains(flag)) {
                                (
                                    HarnessAvailability::Available,
                                    (!version.is_empty()).then_some(version),
                                    None,
                                )
                            } else {
                                (HarnessAvailability::Unavailable, (!version.is_empty()).then_some(version), Some("Claude Code CLI lacks the required noninteractive protocol".to_owned()))
                            }
                        }
                        Ok(_) => (
                            HarnessAvailability::Unavailable,
                            (!version.is_empty()).then_some(version),
                            Some("Claude Code --help failed".to_owned()),
                        ),
                        Err(error) => (
                            HarnessAvailability::Unavailable,
                            (!version.is_empty()).then_some(version),
                            Some(format!("Claude Code capability probe failed: {error}")),
                        ),
                    }
                }
                Ok(_) => (
                    HarnessAvailability::Unavailable,
                    None,
                    Some("Claude Code --version failed".to_owned()),
                ),
                Err(error) => (
                    HarnessAvailability::Unavailable,
                    None,
                    Some(format!("Claude Code CLI unavailable: {error}")),
                ),
            };
            Ok(ProbeResult {
                info: self.info.clone(),
                availability,
                version,
                capabilities: BTreeSet::from([
                    HarnessCapability::SystemPrompt,
                    HarnessCapability::Streaming,
                    HarnessCapability::Resume,
                    HarnessCapability::Steer,
                    HarnessCapability::Cancel,
                    HarnessCapability::Tools,
                ]),
                reason,
            })
        })
    }

    fn models<'a>(
        &'a self,
        config: &'a HarnessConfig,
    ) -> HarnessFuture<'a, Vec<lankide_harness::NativeModel>> {
        Box::pin(async move {
            let mut process = lankide_harness::NativeCatalogProcess::spawn(
                config,
                &[
                    "--print",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--no-session-persistence",
                    "--mcp-config",
                    r#"{"mcpServers":{}}"#,
                ],
            )?;
            let result = async {
                process.send(json!({"type":"control_request","request_id":"catalog","request":{"subtype":"initialize"}})).await?;
                let response = process.receive(|v| v["type"] == "control_response" && v["response"]["request_id"] == "catalog").await?;
                let models = response.pointer("/response/response/models").and_then(Value::as_array).ok_or_else(|| harness_error(HarnessErrorCode::InvalidEvent, "Claude Code did not return a model directory"))?;
                models.iter().map(|model| {
                    let id = model["value"].as_str().filter(|v| !v.is_empty()).ok_or_else(|| harness_error(HarnessErrorCode::InvalidEvent, "Claude Code returned a model without an ID"))?;
                    Ok(lankide_harness::NativeModel { id: id.into(), name: model["displayName"].as_str().unwrap_or(id).into(), description: model["description"].as_str().unwrap_or_default().into(), thinking_levels: model["supportedEffortLevels"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect() })
                }).collect()
            }.await;
            process.close().await;
            result
        })
    }

    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open(request, false) })
    }

    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open(request, true) })
    }
}

struct ClaudeCodeSession {
    inner: Arc<Inner>,
}

struct Inner {
    entry: Mutex<SessionEntry>,
    request: SessionRequest,
    paths: SessionPaths,
    mcp_path: PathBuf,
    mcp_stop: Mutex<Option<crate::mcp::McpStopHandle>>,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    active: Mutex<Option<Execution>>,
    pending_tools: Mutex<HashMap<String, std_mpsc::Sender<ToolResultRequest>>>,
    applied_tools: Mutex<HashMap<String, String>>,
    closed: AtomicBool,
    pid_journal: Option<PathBuf>,
}

struct Execution {
    sender: mpsc::UnboundedSender<Result<AgentEvent, HarnessError>>,
    sequence: u64,
    final_output: String,
}

#[derive(Clone)]
struct SessionPaths {
    config_dir: PathBuf,
    artifact_dir: PathBuf,
}

impl SessionPaths {
    fn prepare(request: &SessionRequest) -> Result<Self, HarnessError> {
        let config_dir = request
            .config
            .environment
            .get("LANKIDE_CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                request
                    .working_directory
                    .join(".lankide/claude-code")
                    .join(stable_component(&request.config.config_fingerprint))
            });
        let artifact_dir = request
            .config
            .environment
            .get("LANKIDE_CLAUDE_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| config_dir.join("artifacts"));
        let source = request
            .config
            .environment
            .get("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| lankide_harness::user_config_directory("CLAUDE_CONFIG_DIR", ".claude"));
        lankide_harness::prepare_config_overlay(
            &config_dir,
            source.as_deref(),
            &["settings.json", ".credentials.json", ".claude.json"],
        )?;
        lankide_harness::prepare_native_skills(
            &request.working_directory,
            &request.working_directory.join(".claude/skills"),
        )?;
        fs::create_dir_all(&artifact_dir).map_err(io_error("create Claude artifact dir"))?;
        Ok(Self {
            config_dir,
            artifact_dir,
        })
    }
}

impl HarnessSession for ClaudeCodeSession {
    fn process_group_id(&self) -> Option<u32> {
        self.inner
            .child
            .lock()
            .expect("child lock")
            .as_ref()
            .map(Child::id)
    }

    fn entry(&self) -> SessionEntry {
        self.inner.entry.lock().expect("entry lock").clone()
    }

    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async move {
            if self.inner.closed.load(Ordering::Acquire)
                || self.entry().health != SessionHealth::Healthy
            {
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "Claude Code session is not healthy",
                ));
            }
            let (sender, receiver) = mpsc::unbounded_channel();
            {
                let mut active = self.inner.active.lock().expect("active lock");
                if active.is_some() {
                    return Err(harness_error(
                        HarnessErrorCode::InvalidState,
                        "Claude Code session already has an active turn",
                    ));
                }
                let mut execution = Execution {
                    sender,
                    sequence: 0,
                    final_output: String::new(),
                };
                emit(&mut execution, AgentEventKind::Started);
                emit(
                    &mut execution,
                    AgentEventKind::NativeSessionBound {
                        native_session_id: self.entry().native_session_id,
                    },
                );
                *active = Some(execution);
            }
            if let Err(error) = self.inner.spawn_turn(request.input) {
                self.inner.fail(error.clone());
                return Err(error);
            }
            Ok(Box::pin(UnboundedReceiverStream::new(receiver)) as AgentEventStream)
        })
    }

    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async move {
            if self.inner.active.lock().expect("active lock").is_none() {
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "Claude Code has no active turn",
                ));
            }
            self.inner.write_input(&input)?;
            Ok(SteerReceipt {
                delivery_id,
                accepted: true,
            })
        })
    }

    fn tool_result<'a>(
        &'a self,
        request: ToolResultRequest,
    ) -> HarnessFuture<'a, ToolResultReceipt> {
        Box::pin(async move {
            let fingerprint = serde_json::to_string(&request).map_err(|error| {
                harness_error(HarnessErrorCode::InvalidConfig, error.to_string())
            })?;
            if let Some(previous) = self
                .inner
                .applied_tools
                .lock()
                .expect("applied lock")
                .get(&request.tool_call_id)
                .cloned()
            {
                if previous == fingerprint {
                    return Ok(ToolResultReceipt {
                        tool_call_id: request.tool_call_id,
                        already_applied: true,
                    });
                }
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "conflicting Claude Code tool result",
                ));
            }
            let sender = self
                .inner
                .pending_tools
                .lock()
                .expect("pending lock")
                .remove(&request.tool_call_id)
                .ok_or_else(|| {
                    harness_error(
                        HarnessErrorCode::InvalidState,
                        "Claude Code tool call was not observed",
                    )
                })?;
            sender.send(request.clone()).map_err(|_| {
                harness_error(
                    HarnessErrorCode::StreamInterrupted,
                    "Claude Code MCP request ended",
                )
            })?;
            self.inner
                .applied_tools
                .lock()
                .expect("applied lock")
                .insert(request.tool_call_id.clone(), fingerprint);
            self.inner.emit(AgentEventKind::ToolFinished {
                tool_call_id: request.tool_call_id.clone(), success: request.success,
                output: json!({"content_markdown":request.content_markdown,"structured_content":request.structured_content,"resources":request.resources,"error":request.error}),
            });
            Ok(ToolResultReceipt {
                tool_call_id: request.tool_call_id,
                already_applied: false,
            })
        })
    }

    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async move {
            let mut execution = self
                .inner
                .active
                .lock()
                .expect("active lock")
                .take()
                .ok_or_else(|| {
                    harness_error(
                        HarnessErrorCode::InvalidState,
                        "Claude Code has no active turn",
                    )
                })?;
            emit(&mut execution, AgentEventKind::Cancelled { reason });
            self.inner.reap(true);
            self.inner
                .pending_tools
                .lock()
                .expect("pending lock")
                .clear();
            Ok(CancelReceipt { accepted: true })
        })
    }

    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            if self.inner.closed.swap(true, Ordering::AcqRel) {
                return Ok(CloseReceipt {
                    already_closed: true,
                });
            }
            if let Some(mut execution) = self.inner.active.lock().expect("active lock").take() {
                emit(
                    &mut execution,
                    AgentEventKind::Cancelled {
                        reason: "Claude Code session closed".to_owned(),
                    },
                );
            }
            self.inner.reap(true);
            self.inner
                .pending_tools
                .lock()
                .expect("pending lock")
                .clear();
            if let Some(handle) = self.inner.mcp_stop.lock().expect("mcp lock").take() {
                handle.stop();
            }
            self.inner.entry.lock().expect("entry lock").health = SessionHealth::Closed;
            Ok(CloseReceipt {
                already_closed: false,
            })
        })
    }
}

impl Inner {
    fn spawn_turn(self: &Arc<Self>, input: String) -> Result<(), HarnessError> {
        // A result event is terminal for the previous CLI invocation, but its
        // OS process can take a moment to exit after stdin closes. Reap it
        // before installing the next invocation's handle.
        self.reap(false);
        let entry = self.entry.lock().expect("entry lock").clone();
        let is_resume = session_artifact_exists(&self.paths.config_dir, &entry.native_session_id);
        let stderr_path = self
            .paths
            .artifact_dir
            .join(format!("claude-{}.stderr.log", unique()));
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_path)
            .map_err(io_error("open Claude stderr artifact"))?;
        let mut command = Command::new(&self.request.config.cli_path);
        command
            .args([
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--input-format",
                "stream-json",
                "--model",
                self.request.config.model.model_id.as_str(),
                "--permission-mode",
                match self.request.permission_policy {
                    lankide_harness::PermissionPolicy::FullAccess => "bypassPermissions",
                },
                "--mcp-config",
            ])
            .arg(&self.mcp_path);
        command.args(["--system-prompt", self.request.system_prompt.as_str()]);
        if self.request.config.thinking_level == "off" {
            command.env("CLAUDE_CODE_DISABLE_THINKING", "1");
        } else {
            command
                .args(["--effort", self.request.config.thinking_level.as_str()])
                .env("CLAUDE_CODE_DISABLE_THINKING", "0");
        }
        if is_resume {
            command.args(["--resume", entry.native_session_id.as_str()]);
        } else {
            command.args(["--session-id", entry.native_session_id.as_str()]);
        }
        command
            .envs(&self.request.config.environment)
            .env_remove("LANKIDE_RUNTIME_SOCKET")
            .env_remove("LANKIDE_RUNTIME_URL")
            .env("CLAUDE_CONFIG_DIR", &self.paths.config_dir)
            .current_dir(&self.request.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|error| {
            let mut failure = harness_error(
                HarnessErrorCode::SpawnFailed,
                format!("failed to spawn Claude Code CLI: {error}"),
            );
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) {
                failure.failure.category = lankide_harness::FailureCategory::Configuration;
                failure.failure.retryable = false;
            }
            failure
        })?;
        let pid = child.id();
        record_pid(self.pid_journal.as_deref(), "spawned", pid);
        let stdin = child.stdin.take().ok_or_else(|| {
            harness_error(
                HarnessErrorCode::SpawnFailed,
                "Claude Code stdin unavailable",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            harness_error(
                HarnessErrorCode::SpawnFailed,
                "Claude Code stdout unavailable",
            )
        })?;
        *self.stdin.lock().expect("stdin lock") = Some(stdin);
        *self.child.lock().expect("child lock") = Some(child);
        if let Err(error) = self.write_input(&input) {
            self.reap(true);
            return Err(error);
        }
        spawn_reader(Arc::downgrade(self), stdout, pid, stderr_path);
        Ok(())
    }

    fn write_input(&self, input: &str) -> Result<(), HarnessError> {
        let message = json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":input}]}});
        let mut guard = self.stdin.lock().expect("stdin lock");
        let stdin = guard.as_mut().ok_or_else(|| {
            harness_error(
                HarnessErrorCode::InvalidState,
                "Claude Code stdin is closed",
            )
        })?;
        serde_json::to_writer(&mut *stdin, &message).map_err(|error| {
            harness_error(HarnessErrorCode::Io, format!("write Claude input: {error}"))
        })?;
        stdin
            .write_all(b"\n")
            .map_err(io_error("write Claude input"))?;
        stdin.flush().map_err(io_error("flush Claude input"))
    }

    fn emit(&self, kind: AgentEventKind) {
        if let Some(execution) = self.active.lock().expect("active lock").as_mut() {
            emit(execution, kind);
        }
    }

    fn fail(&self, error: HarnessError) {
        if let Some(mut execution) = self.active.lock().expect("active lock").take() {
            emit(
                &mut execution,
                AgentEventKind::Failed {
                    failure: Some(error.failure.clone()),
                    code: format!("{:?}", error.code),
                    message: error.message,
                },
            );
        }
        self.entry.lock().expect("entry lock").health = SessionHealth::Broken;
    }

    fn reap(&self, force: bool) {
        self.stdin.lock().expect("stdin lock").take();
        if let Some(mut child) = self.child.lock().expect("child lock").take() {
            if force || child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
            record_pid(self.pid_journal.as_deref(), "reaped", child.id());
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.reap(true);
        if let Some(handle) = self.mcp_stop.lock().expect("mcp lock").take() {
            handle.stop();
        }
    }
}

fn spawn_tool_dispatch(inner: Weak<Inner>, calls: std_mpsc::Receiver<McpCall>) {
    thread::spawn(move || {
        while let Ok(call) = calls.recv() {
            let Some(inner) = inner.upgrade() else { return };
            inner
                .pending_tools
                .lock()
                .expect("pending lock")
                .insert(call.call_id.clone(), call.response);
            inner.emit(AgentEventKind::ToolStarted {
                tool_call_id: call.call_id,
                tool_name: call.name,
                arguments: call.arguments,
            });
        }
    });
}

fn spawn_reader(
    inner: Weak<Inner>,
    stdout: std::process::ChildStdout,
    _pid: u32,
    stderr_path: PathBuf,
) {
    thread::spawn(move || {
        let mut saw_terminal = false;
        for line in BufReader::new(stdout).lines() {
            let Some(inner) = inner.upgrade() else { return };
            match line {
                Ok(line) => match serde_json::from_str::<Value>(&line) {
                    Ok(value) => saw_terminal |= handle_event(&inner, value),
                    Err(error) => {
                        inner.fail(harness_error(
                            HarnessErrorCode::InvalidEvent,
                            format!("invalid Claude stream-json event: {error}"),
                        ));
                        break;
                    }
                },
                Err(error) => {
                    inner.fail(harness_error(
                        HarnessErrorCode::StreamInterrupted,
                        format!("Claude stdout failed: {error}"),
                    ));
                    break;
                }
            }
        }
        if let Some(inner) = inner.upgrade() {
            if !saw_terminal && inner.active.lock().expect("active lock").is_some() {
                inner.fail(harness_error(
                    HarnessErrorCode::StreamInterrupted,
                    inner
                        .child
                        .lock()
                        .expect("child lock")
                        .as_mut()
                        .map(|child| {
                            process_exit_detail(
                                &stderr_path,
                                child,
                                "Claude stdout ended before result",
                            )
                        })
                        .unwrap_or_else(|| "Claude stdout ended before result".into()),
                ));
            }
            inner.stdin.lock().expect("stdin lock").take();
        }
    });
}

fn handle_event(inner: &Arc<Inner>, value: Value) -> bool {
    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "system" if value.get("subtype").and_then(Value::as_str) == Some("init") => {
            let got = value
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if got != inner.entry.lock().expect("entry lock").native_session_id {
                inner.fail(harness_error(
                    HarnessErrorCode::BindFailed,
                    "Claude bound a different session",
                ));
            }
        }
        "stream_event" => {
            if let Some(delta) = value.pointer("/event/delta") {
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if let Some(execution) =
                                inner.active.lock().expect("active lock").as_mut()
                            {
                                execution.final_output.push_str(text);
                                emit(
                                    execution,
                                    AgentEventKind::AssistantDelta {
                                        text: text.to_owned(),
                                    },
                                );
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            inner.emit(AgentEventKind::ReasoningDelta {
                                text: text.to_owned(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "result" => {
            let got = value
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let expected = inner
                .entry
                .lock()
                .expect("entry lock")
                .native_session_id
                .clone();
            if let Some(mut execution) = inner.active.lock().expect("active lock").take() {
                if let Some(result) = value
                    .get("result")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    execution.final_output = result.to_owned();
                }
                if got != expected {
                    emit(
                        &mut execution,
                        AgentEventKind::Failed {
                            failure: None,
                            code: "BindFailed".to_owned(),
                            message: "Claude result used a different session".to_owned(),
                        },
                    );
                } else if value.get("is_error").and_then(Value::as_bool) == Some(true) {
                    let errors = value
                        .get("errors")
                        .and_then(Value::as_array)
                        .map(|errors| {
                            errors
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    let message = if !errors.is_empty() {
                        errors
                    } else if let Some(error) = value
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .or_else(|| value.get("error").and_then(Value::as_str))
                    {
                        error.to_owned()
                    } else if !execution.final_output.is_empty() {
                        execution.final_output.clone()
                    } else {
                        value
                            .get("subtype")
                            .and_then(Value::as_str)
                            .unwrap_or("Claude Code execution failed")
                            .to_owned()
                    };
                    emit(
                        &mut execution,
                        AgentEventKind::Failed {
                            failure: None,
                            code: value
                                .get("api_error_status")
                                .and_then(Value::as_str)
                                .unwrap_or("ClaudeCodeError")
                                .to_owned(),
                            message,
                        },
                    );
                } else {
                    if let Some(usage) = value.get("usage") {
                        emit(
                            &mut execution,
                            AgentEventKind::Usage {
                                input_tokens: usage
                                    .get("input_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0),
                                output_tokens: usage
                                    .get("output_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0),
                            },
                        );
                    }
                    let final_output = execution.final_output.clone();
                    emit(&mut execution, AgentEventKind::Completed { final_output });
                }
            }
            inner.stdin.lock().expect("stdin lock").take();
            return true;
        }
        _ => {}
    }
    false
}

fn session_artifact_exists(root: &Path, session_id: &str) -> bool {
    let target = format!("{session_id}.jsonl");
    let mut paths = vec![root.to_path_buf()];
    while let Some(path) = paths.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                paths.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some(&target) {
                return true;
            }
        }
    }
    false
}

fn stable_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(96)
        .collect::<String>();
    if value.is_empty() {
        "default".to_owned()
    } else {
        value
    }
}

fn new_uuid() -> String {
    let mut value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        ^ ((unique() as u128) << 64);
    value = (value & !(0xf_u128 << 76)) | (4_u128 << 76);
    value = (value & !(0x3_u128 << 62)) | (0x2_u128 << 62);
    let text = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &text[..8],
        &text[8..12],
        &text[12..16],
        &text[16..20],
        &text[20..]
    )
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, c)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn unique() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ NEXT.fetch_add(1, Ordering::Relaxed)
        ^ ((std::process::id() as u64) << 32)
}

fn emit(execution: &mut Execution, kind: AgentEventKind) {
    let kind = classified_event(kind);
    execution.sequence += 1;
    let _ = execution.sender.send(Ok(AgentEvent {
        sequence: execution.sequence,
        kind,
    }));
}

fn io_error(context: &'static str) -> impl FnOnce(std::io::Error) -> HarnessError {
    move |error| harness_error(HarnessErrorCode::Io, format!("{context}: {error}"))
}

fn record_pid(path: Option<&Path>, state: &str, pid: u32) {
    let Some(path) = path else { return };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let line = format!("{state}\t{pid}\n");
        let _ = file.write_all(line.as_bytes());
    }
}

fn classify_failure(code: String, detail: String) -> lankide_harness::HarnessFailure {
    use lankide_harness::FailureCategory;
    let text = detail.to_ascii_lowercase();
    let category = if (text.contains("model")
        && (text.contains("not found")
            || text.contains("not available")
            || text.contains("not supported")))
        || [
            "model not found",
            "unknown model",
            "invalid model",
            "does not exist",
            "no such file",
            "not found (os error 2)",
            "command not found",
            "enoent",
        ]
        .iter()
        .any(|value| text.contains(value))
    {
        FailureCategory::Configuration
    } else if [
        "unauthorized",
        "authentication",
        "authenticate",
        "invalid bearer token",
        "token expired",
        "no api key",
        "missing api key",
        "invalid api key",
        "invalid_api_key",
        "api key not",
        "not logged in",
        "login required",
        "401",
        "forbidden",
        "403",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        FailureCategory::Authentication
    } else if [
        "rate limit",
        "rate_limit",
        "rate-limit",
        "usage limit",
        "usage_limit",
        "resource_exhausted",
        "exceeded your current quota",
        "too many requests",
        "429",
        "quota exceeded",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        FailureCategory::RateLimit
    } else if text.contains("cancelled") || text.contains("canceled") || text.contains("aborted") {
        FailureCategory::Cancelled
    } else if text.contains("timeout") || text.contains("timed out") {
        FailureCategory::Timeout
    } else if [
        "exited",
        "exit status",
        "process exit",
        "stream",
        "connection",
        "503",
        "502",
        "temporarily unavailable",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        FailureCategory::Transient
    } else {
        FailureCategory::Agent
    };
    lankide_harness::HarnessFailure {
        code,
        category,
        retryable: matches!(
            category,
            FailureCategory::RateLimit | FailureCategory::Transient | FailureCategory::Timeout
        ),
        detail,
    }
}

fn harness_error(code: HarnessErrorCode, message: impl Into<String>) -> HarnessError {
    let mut error = HarnessError::new(code, message);
    error.failure = classify_failure(error.failure.code.clone(), error.message.clone());
    // Unknown transport and configuration failures keep the contract default.
    if error.failure.category == lankide_harness::FailureCategory::Agent {
        let default = HarnessError::new(code, error.message.clone());
        error.failure.category = default.failure.category;
        error.failure.retryable = default.failure.retryable;
    }
    error
}

fn classified_event(mut kind: AgentEventKind) -> AgentEventKind {
    if let AgentEventKind::Failed {
        code,
        message,
        failure,
    } = &mut kind
    {
        if failure.is_none() {
            *failure = Some(classify_failure(code.clone(), message.clone()));
        }
    }
    kind
}

fn process_exit_detail(stderr_path: &Path, child: &mut Child, fallback: &str) -> String {
    let status = child
        .try_wait()
        .ok()
        .flatten()
        .map(|status| status.to_string());
    let stderr = std::fs::read_to_string(stderr_path).unwrap_or_default();
    if stderr.trim().is_empty() {
        status
            .map(|status| format!("{fallback}; process exit: {status}"))
            .unwrap_or_else(|| fallback.to_owned())
    } else {
        match status {
            Some(status) => format!("{stderr}\nprocess exit: {status}"),
            None => stderr,
        }
    }
}
