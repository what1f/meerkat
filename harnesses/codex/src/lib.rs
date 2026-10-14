#![forbid(unsafe_code)]

//! Codex Harness adapter.

use lankide_harness::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    HarnessAvailability, HarnessCapability, HarnessConfig, HarnessError, HarnessErrorCode,
    HarnessFactory, HarnessFuture, HarnessInfo, HarnessSession, ProbeResult, SessionEntry,
    SessionHealth, SessionRequest, SteerReceipt, ToolResultReceipt, ToolResultRequest,
};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_stream::wrappers::UnboundedReceiverStream;

const RUNTIME_SOCKET_ENV: &str = "LANKIDE_RUNTIME_SOCKET";
const HARNESS_ID: &str = "codex";
const RPC_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
pub struct CodexHarness {
    info: HarnessInfo,
    pid_journal: Option<PathBuf>,
}

impl CodexHarness {
    pub fn new() -> Self {
        Self {
            info: HarnessInfo::new(HARNESS_ID, "Codex").expect("static Codex Harness info"),
            pid_journal: None,
        }
    }

    pub fn with_pid_journal(mut self, path: impl Into<PathBuf>) -> Self {
        self.pid_journal = Some(path.into());
        self
    }

    fn validate_config(&self, config: &HarnessConfig) -> Result<(), HarnessError> {
        config.validate()?;
        if config.harness_id != HARNESS_ID {
            return Err(HarnessError::invalid_config(
                "Codex harness_id must be codex",
            ));
        }
        if config
            .environment
            .get(RUNTIME_SOCKET_ENV)
            .is_some_and(|socket| !Path::new(socket).is_absolute())
        {
            return Err(HarnessError::invalid_config(
                "LANKIDE_RUNTIME_SOCKET must be absolute",
            ));
        }
        Ok(())
    }
}

impl Default for CodexHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessFactory for CodexHarness {
    fn info(&self) -> &HarnessInfo {
        &self.info
    }

    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            config.validate_executable()?;
            let version_output = Command::new(&config.cli_path)
                .arg("--version")
                .envs(&config.environment)
                .output();
            let (availability, version, reason) = match version_output {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    let app_server = Command::new(&config.cli_path)
                        .args(["app-server", "--help"])
                        .envs(&config.environment)
                        .output();
                    match app_server {
                        Ok(help) if help.status.success() => {
                            (HarnessAvailability::Available, Some(version), None)
                        }
                        Ok(_) => (
                            HarnessAvailability::Unavailable,
                            Some(version),
                            Some("Codex CLI does not provide a usable app-server".to_owned()),
                        ),
                        Err(error) => (
                            HarnessAvailability::Unavailable,
                            Some(version),
                            Some(format!("Codex app-server probe failed: {error}")),
                        ),
                    }
                }
                Ok(_) => (
                    HarnessAvailability::Unavailable,
                    None,
                    Some("Codex CLI version probe failed".to_owned()),
                ),
                Err(error) => (
                    HarnessAvailability::Unavailable,
                    None,
                    Some(format!("Codex CLI is unavailable: {error}")),
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
                &["app-server", "--listen", "stdio://"],
            )?;
            let result = async {
                process.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"lankide-catalog","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}})).await?;
                let initialized = process.receive(|v| v["id"] == 1).await?;
                if initialized.get("error").is_some() { return Err(harness_error(HarnessErrorCode::InvalidEvent, "Codex model discovery initialization failed")); }
                process.send(json!({"method":"initialized","params":{}})).await?;
                let mut models = Vec::new();
                let mut cursor: Option<String> = None;
                let mut cursors = BTreeSet::new();
                for request_id in 2..102 {
                    process.send(json!({"id":request_id,"method":"model/list","params":{"limit":100,"cursor":cursor}})).await?;
                    let response = process.receive(|v| v["id"] == request_id).await?;
                    let page = response.pointer("/result/data").and_then(Value::as_array).ok_or_else(|| harness_error(HarnessErrorCode::InvalidEvent, "Codex did not return a model directory"))?;
                    for model in page {
                        if model["hidden"] == true { continue; }
                        let id = model["model"].as_str().filter(|v| !v.is_empty()).ok_or_else(|| harness_error(HarnessErrorCode::InvalidEvent, "Codex returned a model without an ID"))?;
                        models.push(lankide_harness::NativeModel { id: id.into(), name: model["displayName"].as_str().unwrap_or(id).into(), description: model["description"].as_str().unwrap_or_default().into(), thinking_levels: model["supportedReasoningEfforts"].as_array().into_iter().flatten().filter_map(|v| v["reasoningEffort"].as_str()).map(str::to_owned).collect() });
                    }
                    cursor = response.pointer("/result/nextCursor").and_then(Value::as_str).map(str::to_owned);
                    match &cursor {
                        None => return Ok(models),
                        Some(value) if !cursors.insert(value.clone()) => break,
                        _ => {}
                    }
                }
                Err(harness_error(HarnessErrorCode::InvalidEvent, "Codex model directory pagination did not finish"))
            }.await;
            process.close().await;
            result
        })
    }

    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move {
            self.validate_config(&request.config)?;
            CodexSession::open(request, false, self.pid_journal.clone()).await
        })
    }

    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move {
            self.validate_config(&request.config)?;
            if request
                .native_session_id
                .as_deref()
                .is_none_or(str::is_empty)
            {
                return Err(HarnessError::invalid_config(
                    "Codex resume requires a native thread ID",
                ));
            }
            CodexSession::open(request, true, self.pid_journal.clone()).await
        })
    }
}

struct CodexSession {
    inner: Arc<SessionInner>,
}

struct SessionInner {
    stderr_path: PathBuf,
    thinking_level: String,
    entry: Mutex<SessionEntry>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pending_rpc: Mutex<HashMap<String, oneshot::Sender<Result<Value, HarnessError>>>>,
    execution: Mutex<Option<ExecutionState>>,
    tool_calls: Mutex<HashMap<String, PendingToolCall>>,
    applied_tools: Mutex<HashMap<String, String>>,
    next_rpc_id: AtomicU64,
    pid: u32,
    pid_journal: Option<PathBuf>,
}

struct ExecutionState {
    sender: mpsc::UnboundedSender<Result<AgentEvent, HarnessError>>,
    sequence: u64,
    turn_id: Option<String>,
    final_output: String,
    cancel_reason: Option<String>,
}

struct PendingToolCall {
    rpc_id: Value,
    tool_name: String,
}

impl CodexSession {
    async fn open(
        request: SessionRequest,
        resume: bool,
        pid_journal: Option<PathBuf>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        let codex_home = request
            .config
            .environment
            .get("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| request.working_directory.join(".lankide/codex"));
        lankide_harness::prepare_config_overlay(
            &codex_home,
            lankide_harness::user_config_directory("CODEX_HOME", ".codex").as_deref(),
            &["auth.json", "config.toml"],
        )?;
        lankide_harness::prepare_native_skills(
            &request.working_directory,
            &request.working_directory.join(".agents/skills"),
        )?;
        let stderr_path = codex_home.join(format!(
            "app-server-{}.stderr.log",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_path)
            .map_err(|error| {
                harness_error(
                    HarnessErrorCode::Io,
                    format!("open Codex stderr artifact: {error}"),
                )
            })?;
        let mut command = Command::new(&request.config.cli_path);
        command
            .args(["app-server", "--listen", "stdio://"])
            .envs(&request.config.environment)
            .env("CODEX_HOME", &codex_home)
            .current_dir(&request.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        let mut child = command.spawn().map_err(|error| {
            harness_error(
                HarnessErrorCode::SpawnFailed,
                format!("failed to spawn Codex app-server: {error}"),
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            harness_error(HarnessErrorCode::SpawnFailed, "Codex stdin unavailable")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            harness_error(HarnessErrorCode::SpawnFailed, "Codex stdout unavailable")
        })?;
        let pid = child.id();
        record_pid(pid_journal.as_deref(), "spawned", pid);
        let inner = Arc::new(SessionInner {
            stderr_path,
            thinking_level: request.config.thinking_level.clone(),
            entry: Mutex::new(SessionEntry {
                harness_id: HARNESS_ID.to_owned(),
                native_session_id: String::new(),
                config_fingerprint: request.config.config_fingerprint.clone(),
                health: SessionHealth::Starting,
            }),
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(Some(child)),
            pending_rpc: Mutex::new(HashMap::new()),
            execution: Mutex::new(None),
            tool_calls: Mutex::new(HashMap::new()),
            applied_tools: Mutex::new(HashMap::new()),
            next_rpc_id: AtomicU64::new(1),
            pid,
            pid_journal,
        });
        spawn_reader(Arc::downgrade(&inner), stdout);

        inner
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "lankide",
                        "title": "Lankide",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "experimentalApi": true,
                        "requestAttestation": false
                    }
                }),
            )
            .await?;
        inner.notify("initialized", json!({}))?;

        let dynamic_tools = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema
                })
            })
            .collect::<Vec<_>>();
        let method = if resume {
            "thread/resume"
        } else {
            "thread/start"
        };
        let mut params = json!({
            "model": request.config.model.model_id,
            "cwd": request.working_directory,
            "approvalPolicy": "never",
            "sandbox": match request.permission_policy {
                lankide_harness::PermissionPolicy::FullAccess => "danger-full-access",
            },
            "developerInstructions": request.system_prompt,
            "dynamicTools": dynamic_tools
        });
        if resume {
            params["threadId"] = Value::String(
                request
                    .native_session_id
                    .clone()
                    .expect("resume ID validated"),
            );
        }
        let result = inner.request(method, params).await?;
        let native_session_id = nested_string(&result, &["thread", "id"])
            .ok_or_else(|| {
                harness_error(
                    HarnessErrorCode::BindFailed,
                    format!("Codex {method} returned no thread ID"),
                )
            })?
            .to_owned();
        if resume && request.native_session_id.as_deref() != Some(&native_session_id) {
            inner.reap(true);
            return Err(harness_error(
                HarnessErrorCode::BindFailed,
                "Codex resumed a different thread",
            ));
        }
        *inner.entry.lock().expect("entry lock") = SessionEntry {
            harness_id: HARNESS_ID.to_owned(),
            native_session_id,
            config_fingerprint: request.config.config_fingerprint,
            health: SessionHealth::Healthy,
        };
        Ok(Box::new(Self { inner }))
    }
}

impl HarnessSession for CodexSession {
    fn entry(&self) -> SessionEntry {
        self.inner.entry.lock().expect("entry lock").clone()
    }

    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async move {
            let health = self.inner.entry.lock().expect("entry lock").health;
            if health != SessionHealth::Healthy {
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "Codex session is not healthy",
                ));
            }
            let (sender, receiver) = mpsc::unbounded_channel();
            {
                let mut active = self.inner.execution.lock().expect("execution lock");
                if active.is_some() {
                    return Err(harness_error(
                        HarnessErrorCode::InvalidState,
                        "Codex session already has an active turn",
                    ));
                }
                let mut state = ExecutionState {
                    sender,
                    sequence: 0,
                    turn_id: None,
                    final_output: String::new(),
                    cancel_reason: None,
                };
                emit(&mut state, AgentEventKind::Started);
                emit(
                    &mut state,
                    AgentEventKind::NativeSessionBound {
                        native_session_id: self.entry().native_session_id,
                    },
                );
                *active = Some(state);
            }
            let thread_id = self.entry().native_session_id;
            let result = self
                .inner
                .request(
                    "turn/start",
                    json!({
                        "threadId": thread_id,
                        "input": [{"type": "text", "text": request.input, "text_elements": []}],
                        "effort": self.inner.thinking_level
                    }),
                )
                .await;
            match result {
                Ok(value) => {
                    if let Some(turn_id) = nested_string(&value, &["turn", "id"]) {
                        if let Some(active) = self
                            .inner
                            .execution
                            .lock()
                            .expect("execution lock")
                            .as_mut()
                        {
                            active.turn_id = Some(turn_id.to_owned());
                        }
                    }
                }
                Err(error) => {
                    self.inner.fail_execution(error.clone());
                    return Err(error);
                }
            }
            Ok(Box::pin(UnboundedReceiverStream::new(receiver)) as AgentEventStream)
        })
    }

    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async move {
            let (thread_id, turn_id) = self.inner.active_ids()?;
            self.inner
                .request(
                    "turn/steer",
                    json!({
                        "threadId": thread_id,
                        "expectedTurnId": turn_id,
                        "input": [{"type": "text", "text": input, "text_elements": []}]
                    }),
                )
                .await?;
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
                .expect("applied tools lock")
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
                    "conflicting result for an applied Codex tool call",
                ));
            }
            let pending = self
                .inner
                .tool_calls
                .lock()
                .expect("tool calls lock")
                .remove(&request.tool_call_id)
                .ok_or_else(|| {
                    harness_error(
                        HarnessErrorCode::InvalidState,
                        "Codex tool call was not observed",
                    )
                })?;
            let output = json!({
                "content_markdown": request.content_markdown,
                "structured_content": request.structured_content,
                "resources": request.resources,
                "error": request.error
            });
            self.inner.write(json!({
                "id": pending.rpc_id,
                "result": {
                    "contentItems": [{"type": "inputText", "text": output.to_string()}],
                    "success": request.success
                }
            }))?;
            self.inner
                .applied_tools
                .lock()
                .expect("applied tools lock")
                .insert(request.tool_call_id.clone(), fingerprint);
            Ok(ToolResultReceipt {
                tool_call_id: request.tool_call_id,
                already_applied: false,
            })
        })
    }

    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async move {
            let (thread_id, turn_id) = self.inner.active_ids()?;
            if let Some(active) = self
                .inner
                .execution
                .lock()
                .expect("execution lock")
                .as_mut()
            {
                active.cancel_reason = Some(reason);
            }
            self.inner
                .request(
                    "turn/interrupt",
                    json!({"threadId": thread_id, "turnId": turn_id}),
                )
                .await?;
            Ok(CancelReceipt { accepted: true })
        })
    }

    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            let mut entry = self.inner.entry.lock().expect("entry lock");
            if entry.health == SessionHealth::Closed {
                return Ok(CloseReceipt {
                    already_closed: true,
                });
            }
            drop(entry);
            if let Some(mut active) = self.inner.execution.lock().expect("execution lock").take() {
                emit(
                    &mut active,
                    AgentEventKind::Cancelled {
                        reason: "Codex session closed".to_owned(),
                    },
                );
            }
            self.inner.reap(true);
            entry = self.inner.entry.lock().expect("entry lock");
            entry.health = SessionHealth::Closed;
            Ok(CloseReceipt {
                already_closed: false,
            })
        })
    }
}

impl SessionInner {
    async fn request(&self, method: &str, params: Value) -> Result<Value, HarnessError> {
        let id = self.next_rpc_id.fetch_add(1, Ordering::Relaxed);
        let key = id.to_string();
        let (sender, receiver) = oneshot::channel();
        self.pending_rpc
            .lock()
            .expect("pending RPC lock")
            .insert(key.clone(), sender);
        if let Err(error) = self.write(json!({"method": method, "id": id, "params": params})) {
            self.pending_rpc
                .lock()
                .expect("pending RPC lock")
                .remove(&key);
            return Err(error);
        }
        match timeout(RPC_TIMEOUT, receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => Err(harness_error(
                HarnessErrorCode::StreamInterrupted,
                "Codex RPC stream ended",
            )),
            Err(_) => {
                self.pending_rpc
                    .lock()
                    .expect("pending RPC lock")
                    .remove(&key);
                Err(harness_error(
                    HarnessErrorCode::Timeout,
                    "Codex RPC timed out",
                ))
            }
        }
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), HarnessError> {
        self.write(json!({"method": method, "params": params}))
    }

    fn write(&self, value: Value) -> Result<(), HarnessError> {
        let mut guard = self.stdin.lock().expect("stdin lock");
        let stdin = guard.as_mut().ok_or_else(|| {
            harness_error(HarnessErrorCode::InvalidState, "Codex session is closed")
        })?;
        serde_json::to_writer(&mut *stdin, &value).map_err(|error| {
            harness_error(HarnessErrorCode::Io, format!("write Codex RPC: {error}"))
        })?;
        stdin.write_all(b"\n").map_err(|error| {
            harness_error(HarnessErrorCode::Io, format!("write Codex RPC: {error}"))
        })?;
        stdin.flush().map_err(|error| {
            harness_error(HarnessErrorCode::Io, format!("flush Codex RPC: {error}"))
        })
    }

    fn active_ids(&self) -> Result<(String, String), HarnessError> {
        let thread_id = self
            .entry
            .lock()
            .expect("entry lock")
            .native_session_id
            .clone();
        let turn_id = self
            .execution
            .lock()
            .expect("execution lock")
            .as_ref()
            .and_then(|state| state.turn_id.clone())
            .ok_or_else(|| {
                harness_error(HarnessErrorCode::InvalidState, "Codex has no active turn")
            })?;
        Ok((thread_id, turn_id))
    }

    fn fail_execution(&self, error: HarnessError) {
        if let Some(mut active) = self.execution.lock().expect("execution lock").take() {
            emit(
                &mut active,
                AgentEventKind::Failed {
                    failure: Some(error.failure.clone()),
                    code: format!("{:?}", error.code),
                    message: error.message,
                },
            );
        }
    }

    fn reap(&self, force: bool) {
        self.stdin.lock().expect("stdin lock").take();
        let mut child = self.child.lock().expect("child lock").take();
        if let Some(child) = child.as_mut() {
            if force || child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
            record_pid(self.pid_journal.as_deref(), "reaped", self.pid);
        }
    }
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        self.reap(true);
    }
}

fn spawn_reader(inner: Weak<SessionInner>, stdout: std::process::ChildStdout) {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Some(inner) = inner.upgrade() else {
                return;
            };
            match line {
                Ok(line) => match serde_json::from_str::<Value>(&line) {
                    Ok(message) => dispatch_message(&inner, message),
                    Err(error) => inner.fail_execution(harness_error(
                        HarnessErrorCode::InvalidEvent,
                        format!("invalid Codex app-server event: {error}"),
                    )),
                },
                Err(error) => {
                    inner.fail_execution(harness_error(
                        HarnessErrorCode::StreamInterrupted,
                        format!("Codex stdout failed: {error}"),
                    ));
                    return;
                }
            }
        }
        if let Some(inner) = inner.upgrade() {
            let detail = inner
                .child
                .lock()
                .expect("child lock")
                .as_mut()
                .map(|child| {
                    process_exit_detail(&inner.stderr_path, child, "Codex app-server closed stdout")
                })
                .unwrap_or_else(|| "Codex app-server closed stdout".into());
            inner.fail_execution(harness_error(
                HarnessErrorCode::StreamInterrupted,
                detail.clone(),
            ));
            let pending = std::mem::take(&mut *inner.pending_rpc.lock().expect("pending RPC lock"));
            for (_, sender) in pending {
                let _ = sender.send(Err(harness_error(
                    HarnessErrorCode::StreamInterrupted,
                    detail.clone(),
                )));
            }
        }
    });
}

fn dispatch_message(inner: &Arc<SessionInner>, message: Value) {
    if message.get("method").is_none() {
        let key = rpc_id_key(message.get("id"));
        if let Some(sender) = inner
            .pending_rpc
            .lock()
            .expect("pending RPC lock")
            .remove(&key)
        {
            let response = if let Some(error) = message.get("error") {
                Err(harness_error(
                    HarnessErrorCode::InvalidEvent,
                    format!("Codex RPC error: {}", error),
                ))
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(response);
        }
        return;
    }

    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    if message.get("id").is_some() {
        if method == "item/tool/call" {
            let call_id = params
                .get("callId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let tool_name = params
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if !call_id.is_empty() && !tool_name.is_empty() {
                inner.tool_calls.lock().expect("tool calls lock").insert(
                    call_id.clone(),
                    PendingToolCall {
                        rpc_id: message["id"].clone(),
                        tool_name: tool_name.clone(),
                    },
                );
                with_execution(inner, |active| {
                    emit(
                        active,
                        AgentEventKind::ToolStarted {
                            tool_call_id: call_id,
                            tool_name,
                            arguments: params.get("arguments").cloned().unwrap_or(Value::Null),
                        },
                    );
                });
                return;
            }
        }
        let _ = inner.write(json!({
            "id": message["id"],
            "error": {"code": -32601, "message": "unsupported Codex app-server request"}
        }));
        return;
    }

    let event_thread = params.get("threadId").and_then(Value::as_str);
    let native_id = inner
        .entry
        .lock()
        .expect("entry lock")
        .native_session_id
        .clone();
    if event_thread.is_some_and(|id| !native_id.is_empty() && id != native_id) {
        return;
    }
    match method {
        "turn/started" => {
            let turn_id = nested_string(&params, &["turn", "id"]).map(str::to_owned);
            with_execution(inner, |active| active.turn_id = turn_id);
        }
        "item/agentMessage/delta" => {
            if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                with_execution(inner, |active| {
                    active.final_output.push_str(delta);
                    emit(
                        active,
                        AgentEventKind::AssistantDelta {
                            text: delta.to_owned(),
                        },
                    );
                });
            }
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                with_execution(inner, |active| {
                    emit(
                        active,
                        AgentEventKind::ReasoningDelta {
                            text: delta.to_owned(),
                        },
                    );
                });
            }
        }
        "thread/tokenUsage/updated" => {
            let usage = params.get("tokenUsage").and_then(|value| value.get("last"));
            if let Some(usage) = usage {
                with_execution(inner, |active| {
                    emit(
                        active,
                        AgentEventKind::Usage {
                            input_tokens: usage
                                .get("inputTokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                            output_tokens: usage
                                .get("outputTokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                        },
                    );
                });
            }
        }
        "item/completed" => {
            let item = &params["item"];
            match item.get("type").and_then(Value::as_str) {
                Some("agentMessage") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        with_execution(inner, |active| {
                            if active.final_output.is_empty() {
                                active.final_output = text.to_owned();
                                emit(
                                    active,
                                    AgentEventKind::AssistantDelta {
                                        text: text.to_owned(),
                                    },
                                );
                            } else {
                                active.final_output = text.to_owned();
                            }
                        });
                    }
                }
                Some("dynamicToolCall") => {
                    let call_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                    let pending_name = inner
                        .tool_calls
                        .lock()
                        .expect("tool calls lock")
                        .get(call_id)
                        .map(|pending| pending.tool_name.clone());
                    let _ = pending_name;
                    with_execution(inner, |active| {
                        emit(
                            active,
                            AgentEventKind::ToolFinished {
                                tool_call_id: call_id.to_owned(),
                                success: item
                                    .get("success")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                output: item.get("contentItems").cloned().unwrap_or(Value::Null),
                            },
                        );
                    });
                }
                _ => {}
            }
        }
        "warning" | "configWarning" | "deprecationNotice" => {
            let warning = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex warning")
                .to_owned();
            with_execution(inner, |active| {
                emit(active, AgentEventKind::Warning { message: warning });
            });
        }
        "error" => {
            let retrying = params.get("willRetry").and_then(Value::as_bool) == Some(true);
            if !retrying {
                let message = nested_string(&params, &["error", "message"])
                    .or_else(|| params.get("message").and_then(Value::as_str))
                    .unwrap_or("Codex turn failed")
                    .to_owned();
                with_execution(inner, |active| {
                    emit(active, AgentEventKind::Warning { message });
                });
            }
        }
        "turn/completed" => finish_turn(inner, &params),
        _ => {}
    }
}

fn finish_turn(inner: &Arc<SessionInner>, params: &Value) {
    let mut active = inner.execution.lock().expect("execution lock").take();
    let Some(mut active) = active.take() else {
        return;
    };
    let turn = params.get("turn").unwrap_or(&Value::Null);
    let event_turn_id = turn.get("id").and_then(Value::as_str);
    if active
        .turn_id
        .as_deref()
        .zip(event_turn_id)
        .is_some_and(|(expected, actual)| expected != actual)
    {
        *inner.execution.lock().expect("execution lock") = Some(active);
        return;
    }
    match turn
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed")
    {
        "completed" => {
            let output = active.final_output.clone();
            emit(
                &mut active,
                AgentEventKind::Completed {
                    final_output: output,
                },
            );
        }
        "interrupted" => {
            inner.entry.lock().expect("entry lock").health = SessionHealth::Cancelled;
            let reason = active
                .cancel_reason
                .clone()
                .unwrap_or_else(|| "Codex turn interrupted".to_owned());
            emit(&mut active, AgentEventKind::Cancelled { reason });
        }
        _ => {
            let message = nested_string(turn, &["error", "message"])
                .unwrap_or("Codex turn failed")
                .to_owned();
            emit(
                &mut active,
                AgentEventKind::Failed {
                    failure: None,
                    code: "codex_turn_failed".to_owned(),
                    message,
                },
            );
        }
    }
}

fn with_execution(inner: &Arc<SessionInner>, operation: impl FnOnce(&mut ExecutionState)) {
    if let Some(active) = inner.execution.lock().expect("execution lock").as_mut() {
        operation(active);
    }
}

fn emit(active: &mut ExecutionState, kind: AgentEventKind) {
    let kind = classified_event(kind);
    active.sequence += 1;
    let _ = active.sender.send(Ok(AgentEvent {
        sequence: active.sequence,
        kind,
    }));
}

fn nested_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in keys {
        current = current.get(*key)?;
    }
    current.as_str()
}

fn rpc_id_key(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

fn record_pid(path: Option<&Path>, state: &str, pid: u32) {
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
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
