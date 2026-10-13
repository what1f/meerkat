use lankide_harness::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    HarnessAvailability, HarnessCapability, HarnessConfig, HarnessError, HarnessErrorCode,
    HarnessFactory, HarnessFuture, HarnessInfo, HarnessSession, ProbeResult, SessionEntry,
    SessionHealth, SessionRequest, SteerReceipt, ToolResultReceipt, ToolResultRequest,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, Weak, mpsc as std_mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[derive(Clone, Debug)]
pub struct ProcessHarnessSpec {
    pub arguments: Vec<String>,
    pub bind_timeout: Duration,
    pub cancel_grace: Duration,
    pub close_grace: Duration,
    pub pid_journal: Option<PathBuf>,
}

impl Default for ProcessHarnessSpec {
    fn default() -> Self {
        Self {
            arguments: Vec::new(),
            bind_timeout: Duration::from_secs(5),
            cancel_grace: Duration::from_millis(250),
            close_grace: Duration::from_millis(250),
            pid_journal: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProcessHarnessFactory {
    info: HarnessInfo,
    spec: ProcessHarnessSpec,
}

impl ProcessHarnessFactory {
    pub fn new(info: HarnessInfo, spec: ProcessHarnessSpec) -> Self {
        Self { info, spec }
    }

    fn validate_config(&self, config: &HarnessConfig) -> Result<(), HarnessError> {
        config.validate()?;
        if config.harness_id != self.info.harness_id {
            return Err(HarnessError::invalid_config(format!(
                "factory {} cannot run harness {}",
                self.info.harness_id, config.harness_id
            )));
        }
        Ok(())
    }

    fn open_session(
        &self,
        request: SessionRequest,
        resume: bool,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        self.validate_config(&request.config)?;
        if resume
            && request
                .native_session_id
                .as_deref()
                .unwrap_or("")
                .is_empty()
        {
            return Err(HarnessError::new(
                HarnessErrorCode::InvalidConfig,
                "resume requires native_session_id",
            ));
        }
        if !resume && request.native_session_id.is_some() {
            return Err(HarnessError::new(
                HarnessErrorCode::InvalidConfig,
                "start does not accept native_session_id",
            ));
        }

        let mut peer = spawn_peer(
            &request.config,
            &self.spec,
            Some(&request.working_directory),
        )?;
        let command = if resume {
            PeerCommand::Resume {
                request: request.clone(),
            }
        } else {
            PeerCommand::Start {
                request: request.clone(),
            }
        };
        if let Err(error) = write_json_line(&mut peer.stdin, &command) {
            peer.reap(true);
            return Err(error);
        }

        let first = match peer.lines.recv_timeout(self.spec.bind_timeout) {
            Ok(Ok(line)) => line,
            Ok(Err(error)) => {
                peer.reap(true);
                return Err(HarnessError::new(
                    HarnessErrorCode::BindFailed,
                    format!("stdout before session bind: {error}"),
                ));
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                peer.reap(true);
                return Err(HarnessError::new(
                    HarnessErrorCode::Timeout,
                    "timed out waiting for native session bind",
                ));
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                peer.reap(true);
                return Err(HarnessError::new(
                    HarnessErrorCode::BindFailed,
                    "process exited before native session bind",
                ));
            }
        };
        let bound: BindMessage = serde_json::from_str(&first).map_err(|error| {
            peer.reap(true);
            HarnessError::new(
                HarnessErrorCode::BindFailed,
                format!("invalid bind response: {error}"),
            )
        })?;
        if bound.message_type != "bound" || bound.native_session_id.trim().is_empty() {
            peer.reap(true);
            return Err(HarnessError::new(
                HarnessErrorCode::BindFailed,
                "peer did not bind a native session",
            ));
        }
        if resume && request.native_session_id.as_deref() != Some(bound.native_session_id.as_str())
        {
            peer.reap(true);
            return Err(HarnessError::new(
                HarnessErrorCode::BindFailed,
                "peer resumed a different native session",
            ));
        }

        let inner = Arc::new(ProcessSessionInner {
            entry: Mutex::new(SessionEntry {
                harness_id: self.info.harness_id.clone(),
                native_session_id: bound.native_session_id,
                config_fingerprint: request.config.config_fingerprint.clone(),
                health: SessionHealth::Healthy,
            }),
            stdin: Mutex::new(Some(peer.stdin)),
            child: Mutex::new(Some(peer.child)),
            commands: Mutex::new(()),
            manager: Mutex::new(ManagerState { active: None }),
            cancel_grace: self.spec.cancel_grace,
            close_grace: self.spec.close_grace,
            pid_journal: self.spec.pid_journal.clone(),
        });
        spawn_dispatcher(Arc::downgrade(&inner), peer.lines);
        Ok(Box::new(ProcessHarnessSession { inner }))
    }
}

impl HarnessFactory for ProcessHarnessFactory {
    fn info(&self) -> &HarnessInfo {
        &self.info
    }

    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            self.validate_config(config)?;
            let mut peer = spawn_peer(config, &self.spec, None)?;
            write_json_line(
                &mut peer.stdin,
                &PeerCommand::Probe {
                    config: config.clone(),
                },
            )?;
            let line = match peer.lines.recv_timeout(self.spec.bind_timeout) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    peer.reap(true);
                    return Err(HarnessError::new(
                        HarnessErrorCode::Unavailable,
                        format!("probe stdout failed: {error}"),
                    ));
                }
                Err(_) => {
                    peer.reap(true);
                    return Err(HarnessError::new(
                        HarnessErrorCode::Timeout,
                        "probe response timeout",
                    ));
                }
            };
            let response: ProbeMessage = serde_json::from_str(&line).map_err(|error| {
                HarnessError::new(
                    HarnessErrorCode::InvalidEvent,
                    format!("invalid probe response: {error}"),
                )
            })?;
            peer.reap(false);
            if response.message_type != "probe_result" {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidEvent,
                    "expected probe_result",
                ));
            }
            Ok(ProbeResult {
                info: self.info.clone(),
                availability: response.availability,
                version: response.version,
                capabilities: response.capabilities,
                reason: response.reason,
            })
        })
    }

    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open_session(request, false) })
    }

    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open_session(request, true) })
    }
}

#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum PeerCommand {
    Probe { config: HarnessConfig },
    Start { request: SessionRequest },
    Resume { request: SessionRequest },
    Execute { request: ExecuteRequest },
    Steer { delivery_id: String, input: String },
    ToolResult { request: ToolResultRequest },
    Cancel { reason: String },
    Close,
}

#[derive(Deserialize)]
struct ProbeMessage {
    #[serde(rename = "type")]
    message_type: String,
    availability: HarnessAvailability,
    version: Option<String>,
    capabilities: std::collections::BTreeSet<HarnessCapability>,
    reason: Option<String>,
}

#[derive(Deserialize)]
struct BindMessage {
    #[serde(rename = "type")]
    message_type: String,
    native_session_id: String,
}

struct SpawnedPeer {
    child: Child,
    stdin: ChildStdin,
    lines: std_mpsc::Receiver<Result<String, std::io::Error>>,
    pid_journal: Option<PathBuf>,
}

impl SpawnedPeer {
    fn reap(&mut self, force: bool) {
        if force {
            let _ = self.child.kill();
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        record_pid(self.pid_journal.as_deref(), "reaped", self.child.id());
    }
}

fn spawn_peer(
    config: &HarnessConfig,
    spec: &ProcessHarnessSpec,
    working_directory: Option<&Path>,
) -> Result<SpawnedPeer, HarnessError> {
    let mut command = Command::new(&config.cli_path);
    command
        .args(&spec.arguments)
        .envs(&config.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = working_directory {
        command.current_dir(path);
    }
    let mut child = command.spawn().map_err(|error| {
        HarnessError::new(
            HarnessErrorCode::SpawnFailed,
            format!("failed to spawn harness CLI: {error}"),
        )
    })?;
    let pid = child.id();
    record_pid(spec.pid_journal.as_deref(), "spawned", pid);
    let stdin = child.stdin.take().ok_or_else(|| {
        HarnessError::new(HarnessErrorCode::SpawnFailed, "harness stdin unavailable")
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        HarnessError::new(HarnessErrorCode::SpawnFailed, "harness stdout unavailable")
    })?;
    let (sender, lines) = std_mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    while line.ends_with(['\n', '\r']) {
                        line.pop();
                    }
                    if sender.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });
    Ok(SpawnedPeer {
        child,
        stdin,
        lines,
        pid_journal: spec.pid_journal.clone(),
    })
}

fn write_json_line<T: Serialize>(stdin: &mut ChildStdin, value: &T) -> Result<(), HarnessError> {
    serde_json::to_writer(&mut *stdin, value).map_err(|error| {
        HarnessError::new(
            HarnessErrorCode::Io,
            format!("failed to encode peer command: {error}"),
        )
    })?;
    stdin.write_all(b"\n").map_err(|error| {
        HarnessError::new(
            HarnessErrorCode::Io,
            format!("failed to write peer command: {error}"),
        )
    })?;
    stdin.flush().map_err(|error| {
        HarnessError::new(
            HarnessErrorCode::Io,
            format!("failed to flush peer command: {error}"),
        )
    })
}

struct ProcessHarnessSession {
    inner: Arc<ProcessSessionInner>,
}

struct ProcessSessionInner {
    entry: Mutex<SessionEntry>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    commands: Mutex<()>,
    manager: Mutex<ManagerState>,
    cancel_grace: Duration,
    close_grace: Duration,
    pid_journal: Option<PathBuf>,
}

struct ManagerState {
    active: Option<ActiveExecution>,
}

struct ActiveExecution {
    sender: mpsc::Sender<Result<AgentEvent, HarnessError>>,
    next_sequence: u64,
    observed_tool_calls: BTreeSet<String>,
    applied_tool_results: BTreeMap<String, ToolResultRequest>,
    cancelling: bool,
}

impl ProcessSessionInner {
    fn write(&self, command: &PeerCommand) -> Result<(), HarnessError> {
        let mut guard = self.stdin.lock().expect("stdin mutex poisoned");
        let stdin = guard.as_mut().ok_or_else(|| {
            HarnessError::new(HarnessErrorCode::InvalidState, "session stdin is closed")
        })?;
        write_json_line(stdin, command)
    }

    fn health(&self) -> SessionHealth {
        self.entry.lock().expect("entry mutex poisoned").health
    }

    fn set_health(&self, health: SessionHealth) {
        self.entry.lock().expect("entry mutex poisoned").health = health;
    }

    fn force_cancel(&self, reason: String) {
        let mut manager = self.manager.lock().expect("manager mutex poisoned");
        if let Some(active) = manager.active.take() {
            let event = AgentEvent {
                sequence: active.next_sequence,
                kind: AgentEventKind::Cancelled { reason },
            };
            thread::spawn(move || {
                let _ = active.sender.blocking_send(Ok(event));
            });
            self.set_health(SessionHealth::Cancelled);
            drop(manager);
            if let Some(pid) = self
                .child
                .lock()
                .expect("child mutex poisoned")
                .as_ref()
                .map(Child::id)
            {
                record_pid(self.pid_journal.as_deref(), "forced", pid);
            }
            self.kill_and_wait();
        }
    }

    fn kill_and_wait(&self) {
        self.stdin.lock().expect("stdin mutex poisoned").take();
        if let Some(mut child) = self.child.lock().expect("child mutex poisoned").take() {
            let pid = child.id();
            let _ = child.kill();
            let _ = child.wait();
            record_pid(self.pid_journal.as_deref(), "reaped", pid);
        }
    }

    fn graceful_close(&self) {
        self.stdin.lock().expect("stdin mutex poisoned").take();
        let deadline = Instant::now() + self.close_grace;
        loop {
            let exited = {
                let mut guard = self.child.lock().expect("child mutex poisoned");
                match guard.as_mut() {
                    Some(child) => child.try_wait().ok().flatten().is_some(),
                    None => true,
                }
            };
            if exited || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.kill_and_wait();
    }
}

impl Drop for ProcessSessionInner {
    fn drop(&mut self) {
        if let Ok(stdin) = self.stdin.get_mut() {
            stdin.take();
        }
        if let Ok(child) = self.child.get_mut()
            && let Some(mut child) = child.take()
        {
            let pid = child.id();
            let _ = child.kill();
            let _ = child.wait();
            record_pid(self.pid_journal.as_deref(), "reaped", pid);
        }
    }
}

impl HarnessSession for ProcessHarnessSession {
    fn entry(&self) -> SessionEntry {
        self.inner
            .entry
            .lock()
            .expect("entry mutex poisoned")
            .clone()
    }

    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async move {
            let _command = self.inner.commands.lock().expect("command mutex poisoned");
            if self.inner.health() != SessionHealth::Healthy {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "session is not healthy",
                ));
            }
            let (sender, receiver) = mpsc::channel(32);
            {
                let mut manager = self.inner.manager.lock().expect("manager mutex poisoned");
                if manager.active.is_some() {
                    return Err(HarnessError::new(
                        HarnessErrorCode::InvalidState,
                        "an execution is already active",
                    ));
                }
                manager.active = Some(ActiveExecution {
                    sender,
                    next_sequence: 1,
                    observed_tool_calls: BTreeSet::new(),
                    applied_tool_results: BTreeMap::new(),
                    cancelling: false,
                });
            }
            if let Err(error) = self.inner.write(&PeerCommand::Execute { request }) {
                self.inner
                    .manager
                    .lock()
                    .expect("manager mutex poisoned")
                    .active
                    .take();
                self.inner.set_health(SessionHealth::Broken);
                return Err(error);
            }
            Ok(Box::pin(ReceiverStream::new(receiver)) as AgentEventStream)
        })
    }

    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async move {
            let _command = self.inner.commands.lock().expect("command mutex poisoned");
            let manager = self.inner.manager.lock().expect("manager mutex poisoned");
            let Some(active) = manager.active.as_ref() else {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "steer requires an active execution",
                ));
            };
            if active.cancelling {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "session cancellation is already in progress",
                ));
            }
            self.inner.write(&PeerCommand::Steer {
                delivery_id: delivery_id.clone(),
                input,
            })?;
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
            let _command = self.inner.commands.lock().expect("command mutex poisoned");
            let mut manager = self.inner.manager.lock().expect("manager mutex poisoned");
            let active = manager.active.as_mut().ok_or_else(|| {
                HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "tool result requires an active execution",
                )
            })?;
            if active.cancelling {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "session cancellation is already in progress",
                ));
            }
            if !active.observed_tool_calls.contains(&request.tool_call_id) {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    format!(
                        "tool call {} was not observed in this session execution",
                        request.tool_call_id
                    ),
                ));
            }
            if let Some(applied) = active.applied_tool_results.get(&request.tool_call_id) {
                if applied == &request {
                    return Ok(ToolResultReceipt {
                        tool_call_id: request.tool_call_id,
                        already_applied: true,
                    });
                }
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    format!("conflicting result for tool call {}", request.tool_call_id),
                ));
            }
            self.inner.write(&PeerCommand::ToolResult {
                request: request.clone(),
            })?;
            active
                .applied_tool_results
                .insert(request.tool_call_id.clone(), request.clone());
            Ok(ToolResultReceipt {
                tool_call_id: request.tool_call_id,
                already_applied: false,
            })
        })
    }

    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async move {
            let _command = self.inner.commands.lock().expect("command mutex poisoned");
            if self.inner.health() == SessionHealth::Cancelled {
                return Ok(CancelReceipt { accepted: false });
            }
            if self.inner.health() == SessionHealth::Closed {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "session is closed",
                ));
            }
            let mut manager = self.inner.manager.lock().expect("manager mutex poisoned");
            let Some(active) = manager.active.as_mut() else {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "cancel requires an active execution",
                ));
            };
            if active.cancelling {
                return Ok(CancelReceipt { accepted: false });
            }
            self.inner.write(&PeerCommand::Cancel {
                reason: reason.clone(),
            })?;
            active.cancelling = true;
            drop(manager);
            let weak = Arc::downgrade(&self.inner);
            let grace = self.inner.cancel_grace;
            thread::spawn(move || {
                thread::sleep(grace);
                if let Some(inner) = weak.upgrade() {
                    inner.force_cancel(reason);
                }
            });
            Ok(CancelReceipt { accepted: true })
        })
    }

    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            let _command = self.inner.commands.lock().expect("command mutex poisoned");
            if self.inner.health() == SessionHealth::Closed {
                return Ok(CloseReceipt {
                    already_closed: true,
                });
            }
            if self
                .inner
                .manager
                .lock()
                .expect("manager mutex poisoned")
                .active
                .is_some()
            {
                self.inner.force_cancel("session closed".to_owned());
            } else {
                let _ = self.inner.write(&PeerCommand::Close);
                self.inner.graceful_close();
            }
            self.inner.set_health(SessionHealth::Closed);
            Ok(CloseReceipt {
                already_closed: false,
            })
        })
    }
}

fn spawn_dispatcher(
    weak: Weak<ProcessSessionInner>,
    lines: std_mpsc::Receiver<Result<String, std::io::Error>>,
) {
    thread::spawn(move || {
        while let Ok(line) = lines.recv() {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            match line {
                Ok(line) => dispatch_line(&inner, &line),
                Err(error) => fail_active(
                    &inner,
                    HarnessError::new(
                        HarnessErrorCode::StreamInterrupted,
                        format!("harness stdout failed: {error}"),
                    ),
                ),
            }
        }
        if let Some(inner) = weak.upgrade()
            && !matches!(
                inner.health(),
                SessionHealth::Closed | SessionHealth::Cancelled
            )
        {
            fail_active(
                &inner,
                HarnessError::new(
                    HarnessErrorCode::StreamInterrupted,
                    "harness event stream ended before a terminal event",
                ),
            );
        }
    });
}

fn dispatch_line(inner: &ProcessSessionInner, line: &str) {
    let event: AgentEvent = match serde_json::from_str(line) {
        Ok(event) => event,
        Err(error) => {
            fail_active(
                inner,
                HarnessError::new(
                    HarnessErrorCode::InvalidEvent,
                    format!("invalid harness event: {error}"),
                ),
            );
            return;
        }
    };
    let mut manager = inner.manager.lock().expect("manager mutex poisoned");
    let Some(active) = manager.active.as_mut() else {
        return;
    };
    if event.sequence != active.next_sequence {
        let expected = active.next_sequence;
        drop(manager);
        fail_active(
            inner,
            HarnessError::new(
                HarnessErrorCode::InvalidEvent,
                format!(
                    "non-monotonic harness event sequence: expected {expected}, got {}",
                    event.sequence
                ),
            ),
        );
        return;
    }
    active.next_sequence += 1;
    if let AgentEventKind::ToolStarted { tool_call_id, .. } = &event.kind {
        active.observed_tool_calls.insert(tool_call_id.clone());
    }
    let terminal = event.is_terminal();
    let cancelled = matches!(event.kind, AgentEventKind::Cancelled { .. });
    let sender = active.sender.clone();
    let _ = sender.blocking_send(Ok(event));
    if terminal {
        manager.active.take();
        drop(manager);
        inner.set_health(if cancelled {
            SessionHealth::Cancelled
        } else {
            SessionHealth::Healthy
        });
    }
}

fn fail_active(inner: &ProcessSessionInner, error: HarnessError) {
    let active = inner
        .manager
        .lock()
        .expect("manager mutex poisoned")
        .active
        .take();
    if let Some(active) = active {
        let _ = active.sender.blocking_send(Err(error));
    }
    inner.set_health(SessionHealth::Broken);
}

fn record_pid(path: Option<&Path>, action: &str, pid: u32) {
    let Some(path) = path else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{action}\t{pid}");
    }
}
