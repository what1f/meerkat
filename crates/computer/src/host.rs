use crate::STANDARD_HARNESS_IDS;
use crate::resources::{agent_directory, download_resource, replace_resource_links};
use crate::state::{
    AssignmentLifecycle, PersistedAssignment, PersistedEvent, PersistedSession, PersistedState,
    SessionRecovery, assignment_key,
};
use lankide_harness::{
    AgentEvent, AgentEventKind, ExecuteRequest, HarnessAvailability as NativeHarnessAvailability,
    HarnessConfig as NativeHarnessConfig, HarnessErrorCode, HarnessFactory, HarnessSession,
    ModelRef, ProbeResult, SessionRequest, ToolDefinition as NativeToolDefinition, ToolResultError,
    ToolResultRequest, ToolResultResource,
};
use lankide_protocol::{
    ActiveAttempt, AgentRunContext, AssignExecution, AssignmentResult, ComputerFrame,
    ComputerHeartbeat, ComputerHello, DEFAULT_MAX_MESSAGE_BYTES, DeliveryResult, ExecutionEvent,
    ExecutionEventKind, ExecutionRef, HarnessAvailability, HarnessInfo, RuntimeFrame,
    SessionAction, ToolCall, computer_frame, runtime_frame,
};
use prost::Message;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock, mpsc, watch};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

const COMPLETION_ACKNOWLEDGEMENT: &str = "OK";
const MAX_COMPLETION_CORRECTIONS: usize = 3;
const COMPLETION_PROTOCOL_CORRECTION: &str = r#"<system-reminder>
上一条回复不符合结束协议。需要回复或执行操作时，请使用相应工具；不要重复已成功执行的操作。确认本轮全部完成后，只回复 OK。
</system-reminder>"#;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ComputerConfig {
    pub computer_id: String,
    pub instance_id: String,
    pub workspace_root: String,
    pub reconnect: ReconnectPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial: Duration,
    pub maximum: Duration,
    pub multiplier: u32,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(50),
            maximum: Duration::from_secs(5),
            multiplier: 2,
        }
    }
}

pub struct RegisteredHarness {
    pub factory: Arc<dyn HarnessFactory>,
    pub probe_config: NativeHarnessConfig,
}

impl RegisteredHarness {
    pub fn new<H: HarnessFactory + 'static>(factory: H, probe_config: NativeHarnessConfig) -> Self {
        Self {
            factory: Arc::new(factory),
            probe_config,
        }
    }
}

#[derive(Debug)]
pub enum ComputerError {
    Io(std::io::Error),
    Transport(lankide_protocol::TransportError),
    Rpc(tonic::Status),
    Harness(lankide_harness::HarnessError),
    InvalidAssignment(String),
    InvalidHarnessRegistration(String),
    Resource(String),
    PathOutsideAgentHome,
    PersistenceFailed,
    Join(String),
}

impl fmt::Display for ComputerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "computer_io: {error}"),
            Self::Transport(error) => write!(formatter, "computer_transport: {error}"),
            Self::Rpc(error) => write!(formatter, "computer_rpc: {error}"),
            Self::Harness(error) => write!(formatter, "computer_harness: {error}"),
            Self::InvalidAssignment(error) => write!(formatter, "invalid_assignment: {error}"),
            Self::InvalidHarnessRegistration(id) => {
                write!(formatter, "invalid_harness_registration: {id}")
            }
            Self::Resource(error) => write!(formatter, "resource: {error}"),
            Self::PathOutsideAgentHome => formatter.write_str("path_outside_agent_home"),
            Self::PersistenceFailed => formatter.write_str("computer_state_persistence_failed"),
            Self::Join(error) => write!(formatter, "computer_join: {error}"),
        }
    }
}

impl std::error::Error for ComputerError {}

impl From<std::io::Error> for ComputerError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<lankide_protocol::TransportError> for ComputerError {
    fn from(value: lankide_protocol::TransportError) -> Self {
        Self::Transport(value)
    }
}
impl From<tonic::Status> for ComputerError {
    fn from(value: tonic::Status) -> Self {
        Self::Rpc(value)
    }
}
impl From<lankide_harness::HarnessError> for ComputerError {
    fn from(value: lankide_harness::HarnessError) -> Self {
        Self::Harness(value)
    }
}

pub struct Computer {
    capability_cli: Option<PathBuf>,
    config: ComputerConfig,
    harnesses: HashMap<String, RegisteredHarness>,
}

impl Computer {
    pub fn new(config: ComputerConfig) -> Self {
        Self {
            config,
            capability_cli: crate::capability_cli::discover(),
            harnesses: HashMap::new(),
        }
    }

    pub fn with_capability_cli(mut self, path: impl Into<PathBuf>) -> Self {
        self.capability_cli = Some(path.into());
        self
    }

    pub fn config(&self) -> &ComputerConfig {
        &self.config
    }

    pub fn register_harness(&mut self, harness: RegisteredHarness) -> Result<(), ComputerError> {
        let harness_id = harness.factory.info().harness_id.clone();
        if !STANDARD_HARNESS_IDS.contains(&harness_id.as_str())
            || harness_id != harness.probe_config.harness_id
        {
            return Err(ComputerError::InvalidHarnessRegistration(harness_id));
        }
        harness.probe_config.validate_executable()?;
        self.harnesses.insert(harness_id, harness);
        Ok(())
    }

    pub fn harness_count(&self) -> usize {
        self.harnesses.len()
    }

    /// Runs until the supplied shutdown future completes.
    pub async fn run<F>(
        self,
        socket_path: impl Into<PathBuf>,
        shutdown: F,
    ) -> Result<(), ComputerError>
    where
        F: Future<Output = ()> + Send,
    {
        let handle = self.spawn(socket_path).await?;
        tokio::select! {
            result = handle.join() => result,
            _ = shutdown => {
                handle.shutdown("computer_shutdown").await;
                handle.join().await
            }
        }
    }

    pub async fn spawn(
        self,
        socket_path: impl Into<PathBuf>,
    ) -> Result<ComputerHandle, ComputerError> {
        let socket_path = socket_path.into();
        let root = expanded_root(&self.config.workspace_root)?;
        std::fs::create_dir_all(&root)?;
        let state_path = root.join(".computer/state.json");
        let mut persisted = PersistedState::load(&state_path)?;
        for session in persisted.sessions.values_mut() {
            if session.recovery == SessionRecovery::Interrupted {
                session.recovery = SessionRecovery::Interrupted;
            }
        }
        persisted.save(&state_path)?;
        let pending_events = persisted
            .pending_events
            .iter()
            .map(|(key, events)| {
                (
                    key.clone(),
                    events
                        .iter()
                        .map(|(sequence, event)| (*sequence, restore_event(event)))
                        .collect(),
                )
            })
            .collect();
        let mut defaults = HashMap::new();
        let mut factories = HashMap::new();
        let mut probes = HashMap::new();
        for (harness_id, registration) in self.harnesses {
            let mut probe = match registration.factory.probe(&registration.probe_config).await {
                Ok(probe) => probe,
                Err(error) => ProbeResult {
                    info: registration.factory.info().clone(),
                    availability: NativeHarnessAvailability::Unavailable,
                    version: None,
                    capabilities: Default::default(),
                    reason: Some(redact_diagnostic(
                        &error.to_string(),
                        &registration.probe_config,
                    )),
                },
            };
            probe.reason = probe
                .reason
                .map(|reason| redact_diagnostic(&reason, &registration.probe_config));
            defaults.insert(harness_id.clone(), registration.probe_config);
            factories.insert(harness_id.clone(), registration.factory);
            probes.insert(harness_id, probe);
        }
        let (stop_tx, stop_rx) = watch::channel(None);
        let inner = Arc::new(Inner {
            config: self.config,
            capability_cli: self.capability_cli,
            root,
            socket_path,
            state_path,
            factories,
            defaults,
            probes,
            state: Mutex::new(persisted),
            starting: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            assignment_tasks: Mutex::new(Vec::new()),
            pending_events: Mutex::new(pending_events),
            outstanding_tools: Mutex::new(HashSet::new()),
            deferred_tool_results: Mutex::new(HashMap::new()),
            connection: RwLock::new(None),
            connected: AtomicBool::new(false),
            persistence_failed: AtomicBool::new(false),
            stop_tx,
        });
        let handle = ComputerHandle {
            inner: inner.clone(),
            join: Arc::new(Mutex::new(None)),
        };
        let task = tokio::spawn(run_loop(inner, stop_rx));
        *handle.join.lock().await = Some(task);
        Ok(handle)
    }
}

async fn run_loop(
    inner: Arc<Inner>,
    mut stop_rx: watch::Receiver<Option<String>>,
) -> Result<(), ComputerError> {
    let mut delay = inner.config.reconnect.initial;
    loop {
        // A reconnect backoff may already have consumed the watch notification.
        // Check the value as well as awaiting future notifications.
        if stop_rx.borrow().is_some() {
            let reason = stop_rx
                .borrow()
                .clone()
                .unwrap_or_else(|| "computer_shutdown".into());
            ComputerHandle::cancel_inner(&inner, &reason).await;
            return if inner.persistence_failed.load(Ordering::Acquire) {
                Err(ComputerError::PersistenceFailed)
            } else {
                Ok(())
            };
        }
        tokio::select! {
            _ = stop_rx.changed() => continue,
            result = run_connection(inner.clone()) => {
                *inner.connection.write().await = None;
                let established = inner.connected.swap(false, Ordering::AcqRel);
                if let Err(error) = result {
                    if matches!(error, ComputerError::InvalidAssignment(_)) {
                        return Err(error);
                    }
                }
                if stop_rx.borrow().is_some() {
                    continue;
                }
                    if established {
                        delay = inner.config.reconnect.initial;
                    }
                    let sleep_delay = delay;
                    tokio::select! {
                        _ = stop_rx.changed() => {}
                        _ = tokio::time::sleep(sleep_delay) => {
                            if !established {
                                delay = std::cmp::min(
                                    inner.config.reconnect.maximum,
                                    delay.saturating_mul(inner.config.reconnect.multiplier),
                                );
                            }
                        }
                    }
            }
        }
    }
}

#[derive(Clone)]
pub struct ComputerHandle {
    inner: Arc<Inner>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<Result<(), ComputerError>>>>>,
}

impl ComputerHandle {
    pub fn instance_id(&self) -> &str {
        &self.inner.config.instance_id
    }

    pub async fn active_count(&self) -> usize {
        self.inner.active.lock().await.len()
    }

    pub async fn status(&self) -> ComputerStatus {
        ComputerStatus {
            connected: self.inner.connected.load(Ordering::Acquire),
            active_attempts: active_attempts(&self.inner).await,
        }
    }

    pub async fn shutdown(&self, reason: impl Into<String>) {
        let reason = reason.into();
        Self::cancel_inner(&self.inner, &reason).await;
        let _ = self.inner.stop_tx.send(Some(reason));
    }

    pub async fn join(&self) -> Result<(), ComputerError> {
        // Keep the handle in its slot while awaiting: run() selects this future
        // against shutdown and must be able to await it again after cancellation.
        let mut slot = self.join.lock().await;
        let result = match slot.as_mut() {
            Some(task) => task
                .await
                .map_err(|error| ComputerError::Join(error.to_string()))?,
            None => Ok(()),
        };
        slot.take();
        result
    }

    async fn cancel_inner(inner: &Inner, reason: &str) {
        for starting in inner.starting.lock().await.values() {
            let _ = starting.cancel.send(Some(reason.to_owned()));
        }
        let sessions: Vec<_> = inner
            .active
            .lock()
            .await
            .values()
            .map(|active| active.session.clone())
            .collect();
        for session in sessions {
            let _ = session.cancel(reason.to_owned()).await;
            let _ = session.close().await;
        }
        let tasks = std::mem::take(&mut *inner.assignment_tasks.lock().await);
        for mut task in tasks {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputerStatus {
    pub connected: bool,
    pub active_attempts: Vec<ActiveAttempt>,
}

struct ActiveExecution {
    agent_key: String,
    session: Arc<dyn HarnessSession>,
    agent_home: PathBuf,
    context: AgentRunContext,
    accepting_tool_results: bool,
    last_event_sequence: u64,
}

struct StartingExecution {
    assignment_key: String,
    cancel: watch::Sender<Option<String>>,
}

struct Inner {
    capability_cli: Option<PathBuf>,
    config: ComputerConfig,
    root: PathBuf,
    socket_path: PathBuf,
    state_path: PathBuf,
    factories: HashMap<String, Arc<dyn HarnessFactory>>,
    defaults: HashMap<String, NativeHarnessConfig>,
    probes: HashMap<String, ProbeResult>,
    state: Mutex<PersistedState>,
    starting: Mutex<HashMap<String, StartingExecution>>,
    active: Mutex<HashMap<String, ActiveExecution>>,
    assignment_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pending_events: Mutex<HashMap<String, BTreeMap<u64, ExecutionEvent>>>,
    // These two maps are the Computer-owned routing point for Runtime results.
    // Delivery into HarnessSession awaits the shared Harness contract method.
    outstanding_tools: Mutex<HashSet<(String, String)>>,
    deferred_tool_results: Mutex<HashMap<(String, String), lankide_protocol::ToolResult>>,
    connection: RwLock<Option<mpsc::Sender<ComputerFrame>>>,
    connected: AtomicBool,
    persistence_failed: AtomicBool,
    stop_tx: watch::Sender<Option<String>>,
}

async fn run_connection(inner: Arc<Inner>) -> Result<(), ComputerError> {
    let mut client = lankide_protocol::connect_uds(
        &inner.socket_path,
        lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES,
    )
    .await?;
    let (tx, rx) = mpsc::channel(64);
    tx.send(ComputerFrame {
        frame: Some(computer_frame::Frame::Hello(make_hello(&inner).await)),
    })
    .await
    .map_err(|_| ComputerError::InvalidAssignment("computer_stream_closed".into()))?;
    let mut inbound = client
        .connect_computer(ReceiverStream::new(rx))
        .await?
        .into_inner();
    let first = inbound.next().await.ok_or_else(|| {
        ComputerError::InvalidAssignment("runtime_stream_closed_before_hello".into())
    })??;
    let heartbeat_interval = match first.frame {
        Some(runtime_frame::Frame::Hello(hello))
            if hello.protocol_version == lankide_protocol::PROTOCOL_VERSION =>
        {
            Duration::from_millis(hello.heartbeat_interval_ms.max(1))
        }
        _ => {
            return Err(ComputerError::InvalidAssignment(
                "invalid_runtime_hello".into(),
            ));
        }
    };
    *inner.connection.write().await = Some(tx.clone());
    inner.connected.store(true, Ordering::Release);
    publish_inventory(&inner).await;
    replay_pending(&inner, &tx).await;
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut stop_rx = inner.stop_tx.subscribe();
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                tx.send(ComputerFrame {
                    frame: Some(computer_frame::Frame::Heartbeat(ComputerHeartbeat {
                        computer_id: inner.config.computer_id.clone(),
                        instance_id: inner.config.instance_id.clone(),
                        sent_at: now_millis(),
                        active_attempts: active_attempts(&inner).await,
                    })),
                }).await.map_err(|_| ComputerError::InvalidAssignment("computer_stream_closed".into()))?;
            }
            changed = stop_rx.changed() => {
                let _ = changed;
                return Ok(());
            }
            frame = inbound.next() => match frame {
                Some(frame) => dispatch(inner.clone(), frame?).await?,
                None => return Ok(()),
            }
        }
    }
}

async fn make_hello(inner: &Inner) -> ComputerHello {
    let harnesses = STANDARD_HARNESS_IDS
        .iter()
        .map(|id| {
            let factory = inner.factories.get(*id);
            let probe = inner.probes.get(*id);
            HarnessInfo {
                model_catalog: None,
                model_catalog_error: String::new(),
                harness_id: (*id).to_owned(),
                display_name: factory
                    .map(|factory| factory.info().display_name.clone())
                    .unwrap_or_else(|| match *id {
                        "pi" => "Pi".into(),
                        "codex" => "Codex".into(),
                        _ => "Claude Code".into(),
                    }),
                availability: probe
                    .map(|probe| match probe.availability {
                        NativeHarnessAvailability::Available => HarnessAvailability::Available,
                        NativeHarnessAvailability::Unavailable => HarnessAvailability::Unavailable,
                        NativeHarnessAvailability::Unknown => HarnessAvailability::Unknown,
                    })
                    .unwrap_or(HarnessAvailability::Unavailable)
                    .into(),
                version: probe
                    .and_then(|probe| probe.version.clone())
                    .unwrap_or_default(),
                reason: probe
                    .and_then(|probe| probe.reason.clone())
                    .unwrap_or_else(|| {
                        if factory.is_some() {
                            String::new()
                        } else {
                            "not installed".into()
                        }
                    }),
            }
        })
        .collect();
    ComputerHello {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        computer_id: inner.config.computer_id.clone(),
        instance_id: inner.config.instance_id.clone(),
        computer_version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        harnesses,
    }
}

async fn publish_inventory(inner: &Inner) {
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::InventoryChanged(
                lankide_protocol::InventoryChanged {
                    harnesses: make_hello(inner).await.harnesses,
                },
            )),
        },
    )
    .await;
}

async fn dispatch(inner: Arc<Inner>, frame: RuntimeFrame) -> Result<(), ComputerError> {
    match frame.frame {
        Some(runtime_frame::Frame::AssignExecution(assign)) => {
            let task_inner = inner.clone();
            let task = tokio::spawn(async move {
                handle_assignment(task_inner, assign).await;
            });
            let mut tasks = inner.assignment_tasks.lock().await;
            tasks.retain(|task| !task.is_finished());
            tasks.push(task);
        }
        Some(runtime_frame::Frame::DeliverSteer(steer)) => {
            handle_steer(&inner, steer).await;
        }
        Some(runtime_frame::Frame::CancelExecution(cancel)) => {
            handle_cancel(&inner, cancel.execution, cancel.reason).await;
        }
        Some(runtime_frame::Frame::DeleteAgentFiles(request)) => {
            handle_delete_agent_files(&inner, request).await;
        }
        Some(runtime_frame::Frame::ExecutionEventAck(ack)) => {
            if let Some(execution) = ack.execution {
                let key = assignment_key(execution.execution_id, execution.attempt);
                let mut pending = inner.pending_events.lock().await;
                if let Some(events) = pending.get_mut(&key) {
                    events.retain(|sequence, event| {
                        *sequence > ack.max_saved_sequence || terminal_protocol_event(event.kind)
                    });
                    if events.is_empty() {
                        pending.remove(&key);
                    }
                }
                drop(pending);
                let mut state = inner.state.lock().await;
                if let Some(events) = state.pending_events.get_mut(&key) {
                    events.retain(|sequence, event| {
                        *sequence > ack.max_saved_sequence || terminal_protocol_event(event.kind)
                    });
                    if events.is_empty() {
                        state.pending_events.remove(&key);
                    }
                    state.save(&inner.state_path)?;
                }
            }
        }
        Some(runtime_frame::Frame::ToolResult(result)) => {
            route_tool_result(&inner, result).await;
        }
        Some(runtime_frame::Frame::ExecutionCommitted(committed)) => {
            if let Some(execution) = committed.execution {
                let key = assignment_key(execution.execution_id, execution.attempt);
                inner.pending_events.lock().await.remove(&key);
                let mut state = inner.state.lock().await;
                state.pending_events.remove(&key);
                state.accepted_assignments.remove(&key);
                state.save(&inner.state_path)?;
            }
        }
        Some(runtime_frame::Frame::Hello(_)) => {}
        Some(runtime_frame::Frame::Shutdown(shutdown)) => {
            ComputerHandle::cancel_inner(&inner, &shutdown.reason).await;
            let _ = inner.stop_tx.send(Some(shutdown.reason));
        }
        None => {
            return Err(ComputerError::InvalidAssignment(
                "unknown_runtime_frame".into(),
            ));
        }
    }
    Ok(())
}

async fn handle_delete_agent_files(inner: &Inner, request: lankide_protocol::DeleteAgentFiles) {
    let result = async {
        let agent = request
            .agent
            .as_ref()
            .ok_or_else(|| ComputerError::InvalidAssignment("missing_agent".into()))?;
        let key = format!("{}/{}", agent.workspace_id, agent.agent_id);
        if let Some(starting) = inner.starting.lock().await.get(&key) {
            let _ = starting.cancel.send(Some("Agent was deleted".into()));
        }
        let sessions = inner
            .active
            .lock()
            .await
            .iter()
            .filter(|(_, active)| active.agent_key == key)
            .map(|(execution, active)| (execution.clone(), active.session.clone()))
            .collect::<Vec<_>>();
        for (_, session) in &sessions {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                session.cancel("Agent was deleted".into()),
            )
            .await;
            let _ = session.close().await;
        }
        let mut active = inner.active.lock().await;
        for (execution, _) in sessions {
            active.remove(&execution);
        }
        drop(active);
        let directory = agent_directory(&inner.root, &agent.workspace_id, &agent.agent_id)?;
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ComputerError::Io(error)),
        }
    }
    .await;
    let (success, error_code, error_message) = match result {
        Ok(()) => (true, String::new(), String::new()),
        Err(error) => (false, error_code(&error).into(), public_error(&error)),
    };
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::DeleteAgentFilesResult(
                lankide_protocol::DeleteAgentFilesResult {
                    request_id: request.request_id,
                    success,
                    error_code,
                    error_message,
                },
            )),
        },
    )
    .await;
}

async fn handle_assignment(inner: Arc<Inner>, assign: AssignExecution) {
    let execution = match assign.execution.clone() {
        Some(value) if value.execution_id > 0 && value.attempt > 0 => value,
        _ => return,
    };
    let key = assignment_key(execution.execution_id, execution.attempt);
    let agent_key = assign
        .workspace
        .as_ref()
        .zip(assign.agent.as_ref())
        .and_then(|(workspace, profile)| profile.agent.as_ref().map(|agent| (workspace, agent)))
        .filter(|(workspace, agent)| agent.workspace_id == workspace.workspace_id)
        .map(|(workspace, agent)| format!("{}/{}", workspace.workspace_id, agent.agent_id));
    let remembered = inner
        .state
        .lock()
        .await
        .accepted_assignments
        .contains_key(&key);
    let mut cancel = None;
    let result = if let Some(agent_key) = agent_key.as_ref().filter(|_| !remembered) {
        let (cancel_tx, mut cancel_rx) = watch::channel(None);
        let mut starting = inner.starting.lock().await;
        let active = inner.active.lock().await;
        if starting.contains_key(agent_key)
            || active.values().any(|value| &value.agent_key == agent_key)
        {
            Err(ComputerError::InvalidAssignment("agent_busy".into()))
        } else {
            starting.insert(
                agent_key.clone(),
                StartingExecution {
                    assignment_key: key.clone(),
                    cancel: cancel_tx,
                },
            );
            drop(active);
            drop(starting);
            cancel = Some(agent_key.clone());
            tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    let _ = changed;
                    Err(ComputerError::InvalidAssignment("assignment_cancelled".into()))
                }
                result = prepare_assignment(inner.clone(), assign, execution.clone(), key.clone()) => result,
            }
        }
    } else {
        prepare_assignment(inner.clone(), assign, execution.clone(), key.clone()).await
    };
    if let Some(agent_key) = cancel {
        let mut starting = inner.starting.lock().await;
        if starting
            .get(&agent_key)
            .is_some_and(|value| value.assignment_key == key)
        {
            starting.remove(&agent_key);
        }
    }
    if let Err(error) = result {
        send_frame(
            &inner,
            ComputerFrame {
                frame: Some(computer_frame::Frame::AssignmentResult(AssignmentResult {
                    execution: Some(execution),
                    accepted: false,
                    error_code: error_code(&error).into(),
                    error_message: public_error(&error),
                })),
            },
        )
        .await;
    }
}

async fn prepare_assignment(
    inner: Arc<Inner>,
    assign: AssignExecution,
    execution: ExecutionRef,
    key: String,
) -> Result<(), ComputerError> {
    let fingerprint = assignment_fingerprint(&assign);
    let remembered_assignment = inner
        .state
        .lock()
        .await
        .accepted_assignments
        .get(&key)
        .cloned();
    if let Some(remembered) = remembered_assignment {
        if remembered.fingerprint != fingerprint {
            return Err(ComputerError::InvalidAssignment(
                "assignment_conflict".into(),
            ));
        }
        send_frame(
            &inner,
            ComputerFrame {
                frame: Some(computer_frame::Frame::AssignmentResult(AssignmentResult {
                    execution: Some(execution.clone()),
                    accepted: true,
                    error_code: String::new(),
                    error_message: String::new(),
                })),
            },
        )
        .await;
        replay_assignment_pending(&inner, &key).await;
        return Ok(());
    }
    let workspace = assign
        .workspace
        .as_ref()
        .ok_or_else(|| ComputerError::InvalidAssignment("missing_workspace".into()))?;
    let profile = assign
        .agent
        .as_ref()
        .ok_or_else(|| ComputerError::InvalidAssignment("missing_agent".into()))?;
    let agent = profile
        .agent
        .as_ref()
        .ok_or_else(|| ComputerError::InvalidAssignment("missing_agent_ref".into()))?;
    if agent.workspace_id != workspace.workspace_id {
        return Err(ComputerError::InvalidAssignment(
            "agent_workspace_mismatch".into(),
        ));
    }
    let agent_key = format!("{}/{}", workspace.workspace_id, agent.agent_id);
    {
        let active = inner.active.lock().await;
        if active.values().any(|value| value.agent_key == agent_key) {
            return Err(ComputerError::InvalidAssignment("agent_busy".into()));
        }
    }
    let wire_config = assign
        .harness
        .as_ref()
        .ok_or_else(|| ComputerError::InvalidAssignment("missing_harness".into()))?;
    if !STANDARD_HARNESS_IDS.contains(&wire_config.harness_id.as_str()) {
        return Err(ComputerError::InvalidAssignment(
            "unsupported_harness".into(),
        ));
    }
    if Path::new(&wire_config.executable_path)
        .file_name()
        .and_then(|v| v.to_str())
        == Some("dcc")
        && wire_config.harness_id != "claude_code"
    {
        return Err(ComputerError::InvalidAssignment(
            "dcc_requires_claude_code".into(),
        ));
    }
    let factory = inner
        .factories
        .get(&wire_config.harness_id)
        .cloned()
        .ok_or_else(|| ComputerError::InvalidAssignment("harness_unavailable".into()))?;
    let directive = assign
        .session
        .as_ref()
        .ok_or_else(|| ComputerError::InvalidAssignment("missing_session".into()))?;
    let delivery = assign
        .initial_delivery
        .as_ref()
        .ok_or_else(|| ComputerError::InvalidAssignment("missing_delivery".into()))?;
    let agent_home = agent_directory(&inner.root, &workspace.workspace_id, &agent.agent_id)?;
    std::fs::create_dir_all(&agent_home)?;
    let agent_home = agent_home.canonicalize()?;
    let context = AgentRunContext {
        execution: Some(execution.clone()),
        execution_token: assign.execution_token.clone(),
    };
    let mut local_resources = Vec::new();
    for resource in assign.resources.iter().chain(delivery.resources.iter()) {
        if local_resources
            .iter()
            .any(|(id, _)| *id == resource.resource_id)
        {
            continue;
        }
        let path = download_resource(
            &inner.socket_path,
            resource,
            context.clone(),
            &agent_home.join("resources"),
        )
        .await?;
        local_resources.push((resource.resource_id, path));
    }
    let mut input = replace_resource_links(&delivery.rendered_markdown, &local_resources);
    if let Some(file) = input.strip_prefix("content_file:") {
        let path = crate::resolve_content_file(&agent_home, file)?;
        input = std::fs::read_to_string(path)?;
    }
    let default_config = inner
        .defaults
        .get(&wire_config.harness_id)
        .ok_or_else(|| ComputerError::InvalidAssignment("harness_unavailable".into()))?;
    let mut native_config =
        native_config(wire_config, &directive.config_fingerprint, default_config)?;
    // Execution credentials belong to this attempt, never to editable Agent configuration.
    native_config.environment.extend([
        (
            "LANKIDE_RUNTIME_SOCKET".into(),
            inner.socket_path.display().to_string(),
        ),
        (
            "LANKIDE_EXECUTION_ID".into(),
            execution.execution_id.to_string(),
        ),
        (
            "LANKIDE_EXECUTION_ATTEMPT".into(),
            execution.attempt.to_string(),
        ),
        (
            "LANKIDE_EXECUTION_TOKEN".into(),
            assign.execution_token.clone(),
        ),
        (
            "LANKIDE_AGENT_HOME".into(),
            agent_home.display().to_string(),
        ),
    ]);
    let mut system_prompt = assign.system_prompt.clone();
    crate::capability_cli::inject(
        inner.capability_cli.as_deref(),
        &mut native_config,
        &assign.cli_commands,
        &mut system_prompt,
    )?;
    let tools = assign
        .tools
        .iter()
        .map(|tool| {
            Ok(NativeToolDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: serde_json::from_str(&tool.input_schema_json)
                    .map_err(|_| ComputerError::InvalidAssignment("invalid_tool_schema".into()))?,
                output_schema: serde_json::from_str(&tool.output_schema_json)
                    .map_err(|_| ComputerError::InvalidAssignment("invalid_tool_schema".into()))?,
            })
        })
        .collect::<Result<Vec<_>, ComputerError>>()?;
    let session_key = format!("{agent_key}/{}", directive.logical_session_id);
    let remembered = inner.state.lock().await.sessions.get(&session_key).cloned();
    let requested_native = directive.native_session_id.clone().or_else(|| {
        remembered
            .as_ref()
            .filter(|entry| {
                entry.harness_id == wire_config.harness_id
                    && entry.config_fingerprint == directive.config_fingerprint
            })
            .map(|entry| entry.native_session_id.clone())
    });
    let resume = SessionAction::try_from(directive.action).ok()
        == Some(SessionAction::ResumeOrStart)
        && requested_native.is_some();
    let request = SessionRequest {
        config: native_config,
        working_directory: agent_home.clone(),
        system_prompt,
        tools,
        native_session_id: if resume {
            requested_native.clone()
        } else {
            None
        },
    };
    let session = if resume {
        match factory.resume(request.clone()).await {
            Ok(session) => session,
            Err(error) if error.code == HarnessErrorCode::InvalidState => {
                factory
                    .start(SessionRequest {
                        native_session_id: None,
                        ..request
                    })
                    .await?
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        factory.start(request).await?
    };
    let session: Arc<dyn HarnessSession> = session.into();
    let entry = session.entry();
    let stream = match session.execute(ExecuteRequest { input }).await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = session.close().await;
            return Err(error.into());
        }
    };
    {
        let mut state = inner.state.lock().await;
        state.accepted_assignments.insert(
            key.clone(),
            PersistedAssignment {
                fingerprint,
                lifecycle: AssignmentLifecycle::Active,
            },
        );
        state.sessions.insert(
            session_key.clone(),
            PersistedSession {
                harness_id: entry.harness_id,
                native_session_id: entry.native_session_id,
                config_fingerprint: entry.config_fingerprint,
                recovery: SessionRecovery::Interrupted,
            },
        );
        state.save(&inner.state_path)?;
    }
    inner.active.lock().await.insert(
        key.clone(),
        ActiveExecution {
            agent_key,
            session: session.clone(),
            agent_home: agent_home.clone(),
            context: context.clone(),
            accepting_tool_results: true,
            last_event_sequence: 0,
        },
    );
    send_frame(
        &inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::AssignmentResult(AssignmentResult {
                execution: Some(execution.clone()),
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        },
    )
    .await;
    send_frame(
        &inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::DeliveryResult(DeliveryResult {
                execution: Some(execution.clone()),
                delivery_id: delivery.delivery_id,
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        },
    )
    .await;
    tokio::spawn(run_events(
        inner,
        key,
        session_key,
        execution,
        context,
        agent_home,
        session,
        stream,
    ));
    Ok(())
}

fn native_config(
    config: &lankide_protocol::HarnessConfig,
    fingerprint: &str,
    defaults: &NativeHarnessConfig,
) -> Result<NativeHarnessConfig, ComputerError> {
    let mut environment = defaults.environment.clone();
    environment.extend(
        config
            .environment_variables
            .iter()
            .map(|entry| (entry.name.clone(), entry.value.clone())),
    );
    let result = NativeHarnessConfig {
        harness_id: config.harness_id.clone(),
        model: ModelRef::new(config.model.clone(), config.model.clone()),
        thinking_level: config.thinking_level.clone(),
        cli_path: if config.executable_path.trim().is_empty() {
            defaults.cli_path.clone()
        } else {
            PathBuf::from(&config.executable_path)
        },
        environment,
        config_fingerprint: fingerprint.to_owned(),
    };
    result.validate()?;
    Ok(result)
}

async fn run_events(
    inner: Arc<Inner>,
    key: String,
    session_key: String,
    execution: ExecutionRef,
    context: AgentRunContext,
    agent_home: PathBuf,
    session: Arc<dyn HarnessSession>,
    mut stream: lankide_harness::AgentEventStream,
) {
    let mut last_sequence = 0_u64;
    let mut stream_sequence = 0_u64;
    let mut corrections = 0_usize;
    let mut terminal_event = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(mut event) if event.sequence > stream_sequence => {
                stream_sequence = event.sequence;
                if let AgentEventKind::Completed { final_output } = &event.kind
                    && final_output != COMPLETION_ACKNOWLEDGEMENT
                {
                    if corrections >= MAX_COMPLETION_CORRECTIONS {
                        terminal_event = Some(synthetic_event(
                            last_sequence + 1,
                            AgentEventKind::Failed {
                                code: "completion_protocol".into(),
                                message: "Agent did not finish this run correctly".into(),
                            },
                        ));
                        break;
                    }
                    corrections += 1;
                    match session
                        .execute(ExecuteRequest {
                            input: COMPLETION_PROTOCOL_CORRECTION.into(),
                        })
                        .await
                    {
                        Ok(next_stream) => {
                            stream = next_stream;
                            stream_sequence = 0;
                            continue;
                        }
                        Err(error) => {
                            terminal_event = Some(synthetic_event(
                                last_sequence + 1,
                                AgentEventKind::Failed {
                                    code: format!("{:?}", error.code).to_lowercase(),
                                    message: error.message,
                                },
                            ));
                            break;
                        }
                    }
                }

                last_sequence += 1;
                event.sequence = last_sequence;
                if let Some(active) = inner.active.lock().await.get_mut(&key) {
                    active.last_event_sequence = last_sequence;
                }
                if let AgentEventKind::ToolStarted {
                    tool_call_id,
                    tool_name,
                    arguments,
                } = &event.kind
                {
                    match prepare_tool_arguments(tool_name, arguments, &agent_home) {
                        Ok(arguments) => {
                            inner
                                .outstanding_tools
                                .lock()
                                .await
                                .insert((key.clone(), tool_call_id.clone()));
                            send_frame(
                                &inner,
                                ComputerFrame {
                                    frame: Some(computer_frame::Frame::ToolCall(ToolCall {
                                        execution: Some(execution.clone()),
                                        tool_call_id: tool_call_id.clone(),
                                        tool_name: tool_name.clone(),
                                        arguments_json: arguments.to_string(),
                                        execution_token: context.execution_token.clone(),
                                    })),
                                },
                            )
                            .await;
                        }
                        Err((code, message)) => {
                            let _ = session
                                .tool_result(ToolResultRequest {
                                    tool_call_id: tool_call_id.clone(),
                                    success: false,
                                    content_markdown: String::new(),
                                    structured_content: serde_json::json!({}),
                                    resources: Vec::new(),
                                    error: Some(ToolResultError {
                                        code: code.into(),
                                        message: message.into(),
                                        retryable: false,
                                    }),
                                })
                                .await;
                        }
                    }
                }
                if let AgentEventKind::ToolFinished { tool_call_id, .. } = &event.kind {
                    inner
                        .outstanding_tools
                        .lock()
                        .await
                        .remove(&(key.clone(), tool_call_id.clone()));
                }
                if event.is_terminal() {
                    terminal_event = Some(event);
                    break;
                }
                if let Err(error) = emit_event(&inner, &key, &execution, &event).await {
                    stop_after_persistence_failure(&inner, &error);
                    return;
                }
            }
            Ok(_) => {
                terminal_event = Some(synthetic_event(
                    last_sequence + 1,
                    AgentEventKind::Failed {
                        code: "invalid_sequence".into(),
                        message: "Harness event sequence was not monotonic".into(),
                    },
                ));
                break;
            }
            Err(error) => {
                terminal_event = Some(synthetic_event(
                    last_sequence + 1,
                    AgentEventKind::Failed {
                        code: format!("{:?}", error.code).to_lowercase(),
                        message: error.message,
                    },
                ));
                break;
            }
        }
    }
    let terminal_event = terminal_event.unwrap_or_else(|| {
        synthetic_event(
            last_sequence + 1,
            AgentEventKind::Failed {
                code: "stream_interrupted".into(),
                message: "Harness stream ended before a terminal event".into(),
            },
        )
    });
    let _ = session.close().await;
    // Persist and enqueue the terminal event before removing the execution
    // from the active inventory. Otherwise a heartbeat can report the attempt
    // missing while Runtime is still waiting to commit its terminal event.
    if let Err(error) = emit_event(&inner, &key, &execution, &terminal_event).await {
        stop_after_persistence_failure(&inner, &error);
        return;
    }
    inner.active.lock().await.remove(&key);
    inner
        .outstanding_tools
        .lock()
        .await
        .retain(|(execution_key, _)| execution_key != &key);
    inner
        .deferred_tool_results
        .lock()
        .await
        .retain(|(execution_key, _), _| execution_key != &key);
    let mut state = inner.state.lock().await;
    if let Some(assignment) = state.accepted_assignments.get_mut(&key) {
        assignment.lifecycle = AssignmentLifecycle::Terminal;
    }
    if let Some(entry) = state.sessions.get_mut(&session_key) {
        entry.recovery = SessionRecovery::Ready;
    }
    if let Err(error) = state.save(&inner.state_path) {
        stop_after_persistence_failure(&inner, &error);
    }
}

fn prepare_tool_arguments(
    tool_name: &str,
    arguments: &serde_json::Value,
    agent_home: &Path,
) -> Result<serde_json::Value, (&'static str, &'static str)> {
    if tool_name != "message_send" {
        return Ok(arguments.clone());
    }
    let mut object = arguments.as_object().cloned().ok_or((
        "invalid_tool_arguments",
        "Message arguments must be an object",
    ))?;
    let Some(content_file) = object.remove("content_file") else {
        return Ok(serde_json::Value::Object(object));
    };
    let content_file = content_file
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or(("content_file_invalid", "Content file path is invalid"))?;
    let path = crate::resolve_content_file(agent_home, content_file)
        .map_err(|_| ("content_file_unreadable", "Content file could not be read"))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|_| ("content_file_unreadable", "Content file could not be read"))?;
    if metadata.len() > DEFAULT_MAX_MESSAGE_BYTES as u64 {
        return Err(("content_file_too_large", "Content file is too large"));
    }
    let bytes = std::fs::read(path)
        .map_err(|_| ("content_file_unreadable", "Content file could not be read"))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| ("content_file_invalid_utf8", "Content file must be UTF-8"))?;
    object.insert("content".into(), serde_json::Value::String(content));
    Ok(serde_json::Value::Object(object))
}

fn stop_after_persistence_failure(inner: &Inner, error: &std::io::Error) {
    eprintln!("Computer stopped because its execution state could not be saved: {error}");
    inner.persistence_failed.store(true, Ordering::Release);
    let _ = inner.stop_tx.send(Some("state_persistence_failed".into()));
}

async fn emit_event(
    inner: &Inner,
    key: &str,
    execution: &ExecutionRef,
    event: &AgentEvent,
) -> std::io::Result<()> {
    let (kind, payload) = protocol_event(event);
    let event = ExecutionEvent {
        execution: Some(execution.clone()),
        sequence: event.sequence,
        occurred_at: now_millis(),
        kind: kind.into(),
        payload,
    };
    let persisted = PersistedEvent {
        execution_id: execution.execution_id,
        attempt: execution.attempt,
        sequence: event.sequence,
        occurred_at: event.occurred_at,
        kind: event.kind,
        payload: event.payload.clone(),
    };
    let mut state = inner.state.lock().await;
    state
        .pending_events
        .entry(key.to_owned())
        .or_default()
        .insert(event.sequence, persisted);
    state.save(&inner.state_path)?;
    drop(state);
    inner
        .pending_events
        .lock()
        .await
        .entry(key.to_owned())
        .or_default()
        .insert(event.sequence, event.clone());

    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::ExecutionEvent(event)),
        },
    )
    .await;
    Ok(())
}

fn restore_event(event: &PersistedEvent) -> ExecutionEvent {
    ExecutionEvent {
        execution: Some(ExecutionRef {
            execution_id: event.execution_id,
            attempt: event.attempt,
        }),
        sequence: event.sequence,
        occurred_at: event.occurred_at,
        kind: event.kind,
        payload: event.payload.clone(),
    }
}

fn protocol_event(event: &AgentEvent) -> (ExecutionEventKind, Vec<u8>) {
    let kind = match &event.kind {
        AgentEventKind::Started => ExecutionEventKind::Started,
        AgentEventKind::NativeSessionBound { .. } => ExecutionEventKind::NativeSessionBound,
        AgentEventKind::AssistantDelta { .. } => ExecutionEventKind::AssistantDelta,
        AgentEventKind::ReasoningDelta { .. } => ExecutionEventKind::ReasoningDelta,
        AgentEventKind::ToolStarted { .. } => ExecutionEventKind::ToolStarted,
        AgentEventKind::ToolFinished { .. } => ExecutionEventKind::ToolFinished,
        AgentEventKind::Usage { .. } => ExecutionEventKind::Usage,
        AgentEventKind::Warning { .. } => ExecutionEventKind::Warning,
        AgentEventKind::Completed { .. } => ExecutionEventKind::Completed,
        AgentEventKind::Failed { .. } => ExecutionEventKind::Failed,
        AgentEventKind::Cancelled { .. } => ExecutionEventKind::Cancelled,
    };
    (
        kind,
        serde_json::to_vec(&event.kind).unwrap_or_else(|_| b"{}".to_vec()),
    )
}

fn terminal_protocol_event(kind: i32) -> bool {
    matches!(
        ExecutionEventKind::try_from(kind).ok(),
        Some(
            ExecutionEventKind::Completed
                | ExecutionEventKind::Failed
                | ExecutionEventKind::Cancelled
        )
    )
}

fn synthetic_event(sequence: u64, kind: AgentEventKind) -> AgentEvent {
    AgentEvent { sequence, kind }
}

async fn handle_steer(inner: &Inner, steer: lankide_protocol::DeliverSteer) {
    let Some(execution) = steer.execution else {
        return;
    };
    let Some(delivery) = steer.delivery else {
        return;
    };
    let key = assignment_key(execution.execution_id, execution.attempt);
    let session = inner
        .active
        .lock()
        .await
        .get(&key)
        .map(|active| active.session.clone());
    let result = match session {
        Some(session) => {
            session
                .steer(delivery.delivery_id.to_string(), delivery.rendered_markdown)
                .await
        }
        None => Err(lankide_harness::HarnessError::new(
            lankide_harness::HarnessErrorCode::InvalidState,
            "execution is not active",
        )),
    };
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::DeliveryResult(DeliveryResult {
                execution: Some(execution),
                delivery_id: delivery.delivery_id,
                accepted: result.as_ref().is_ok_and(|receipt| receipt.accepted),
                error_code: result
                    .as_ref()
                    .err()
                    .map(|error| format!("{:?}", error.code).to_lowercase())
                    .unwrap_or_default(),
                error_message: result.err().map(|error| error.message).unwrap_or_default(),
            })),
        },
    )
    .await;
}

async fn handle_cancel(inner: &Inner, execution: Option<ExecutionRef>, reason: String) {
    let Some(execution) = execution else { return };
    let key = assignment_key(execution.execution_id, execution.attempt);
    if let Some(starting) = inner
        .starting
        .lock()
        .await
        .values()
        .find(|starting| starting.assignment_key == key)
    {
        let _ = starting.cancel.send(Some(reason.clone()));
    }
    let session = inner.active.lock().await.get_mut(&key).map(|active| {
        active.accepting_tool_results = false;
        active.session.clone()
    });
    if let Some(session) = session {
        let _ = tokio::time::timeout(Duration::from_secs(2), session.cancel(reason)).await;
    }
}

async fn route_tool_result(inner: &Inner, result: lankide_protocol::ToolResult) {
    let Some(execution) = result.execution.clone() else {
        return;
    };
    let execution_key = assignment_key(execution.execution_id, execution.attempt);
    let route_key = (execution_key.clone(), result.tool_call_id.clone());
    let active = inner.active.lock().await.get(&execution_key).map(|active| {
        (
            active.session.clone(),
            active.agent_home.clone(),
            active.context.clone(),
            active.accepting_tool_results,
        )
    });
    let Some((session, agent_home, context, accepting)) = active else {
        send_tool_result_receipt(
            inner,
            &execution,
            &result.tool_call_id,
            false,
            "execution_not_active",
            "Execution is no longer active",
        )
        .await;
        return;
    };
    if !accepting {
        send_tool_result_receipt(
            inner,
            &execution,
            &result.tool_call_id,
            false,
            "execution_cancelling",
            "Execution is being cancelled",
        )
        .await;
        return;
    }
    if !inner.outstanding_tools.lock().await.contains(&route_key) {
        send_tool_result_receipt(
            inner,
            &execution,
            &result.tool_call_id,
            false,
            "unknown_tool_call",
            "Tool call is not owned by this execution",
        )
        .await;
        return;
    }
    let mut local_resources = Vec::new();
    for resource in &result.resources {
        match download_resource(
            &inner.socket_path,
            resource,
            context.clone(),
            &agent_home.join("tool-results"),
        )
        .await
        {
            Ok(local_path) => local_resources.push(ToolResultResource {
                local_path,
                file_name: resource.file_name.clone(),
                media_type: resource.media_type.clone(),
                size_bytes: resource.size_bytes,
                sha256: resource.sha256.clone(),
            }),
            Err(_) => {
                send_tool_result_receipt(
                    inner,
                    &execution,
                    &result.tool_call_id,
                    false,
                    "tool_resource_invalid",
                    "Tool result resource could not be verified",
                )
                .await;
                return;
            }
        }
    }
    let local_links = local_resources
        .iter()
        .zip(result.resources.iter())
        .map(|(local, resource)| (resource.resource_id, local.local_path.clone()))
        .collect::<Vec<_>>();
    let structured_content = match serde_json::from_str(&result.structured_content_json) {
        Ok(value) => value,
        Err(_) => {
            send_tool_result_receipt(
                inner,
                &execution,
                &result.tool_call_id,
                false,
                "invalid_tool_result_json",
                "Tool result data is invalid",
            )
            .await;
            return;
        }
    };
    let request = ToolResultRequest {
        tool_call_id: result.tool_call_id.clone(),
        success: result.success,
        content_markdown: replace_resource_links(&result.content_markdown, &local_links),
        structured_content,
        resources: local_resources,
        error: result.error.as_ref().map(|error| ToolResultError {
            code: error.code.clone(),
            message: error.message.clone(),
            retryable: error.retryable,
        }),
    };
    match session.tool_result(request).await {
        Ok(receipt) => {
            inner
                .deferred_tool_results
                .lock()
                .await
                .insert(route_key, result);
            send_tool_result_receipt(
                inner,
                &execution,
                &receipt.tool_call_id,
                true,
                "",
                if receipt.already_applied {
                    "already_applied"
                } else {
                    "applied"
                },
            )
            .await;
        }
        Err(error) => {
            send_tool_result_receipt(
                inner,
                &execution,
                &result.tool_call_id,
                false,
                "tool_result_rejected",
                &error.message,
            )
            .await;
        }
    }
}

async fn send_tool_result_receipt(
    inner: &Inner,
    execution: &ExecutionRef,
    tool_call_id: &str,
    success: bool,
    error_code: &str,
    message: &str,
) {
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::ResourceTransferResult(
                lankide_protocol::ResourceTransferResult {
                    transfer_id: format!(
                        "tool_result:{}:{}:{tool_call_id}",
                        execution.execution_id, execution.attempt
                    ),
                    success,
                    error_code: error_code.to_owned(),
                    error_message: message.to_owned(),
                },
            )),
        },
    )
    .await;
}

async fn replay_pending(inner: &Inner, sender: &mpsc::Sender<ComputerFrame>) {
    let pending = inner.pending_events.lock().await;
    for events in pending.values() {
        for event in events.values() {
            let _ = sender
                .send(ComputerFrame {
                    frame: Some(computer_frame::Frame::ExecutionEvent(event.clone())),
                })
                .await;
        }
    }
}

async fn replay_assignment_pending(inner: &Inner, key: &str) {
    let events = inner
        .pending_events
        .lock()
        .await
        .get(key)
        .cloned()
        .unwrap_or_default();
    for event in events.into_values() {
        send_frame(
            inner,
            ComputerFrame {
                frame: Some(computer_frame::Frame::ExecutionEvent(event)),
            },
        )
        .await;
    }
}

async fn active_attempts(inner: &Inner) -> Vec<ActiveAttempt> {
    let active = inner.active.lock().await;
    let mut attempts = active
        .iter()
        .filter_map(|(key, value)| {
            let (execution_id, attempt) = key.split_once(':')?;
            Some(ActiveAttempt {
                execution: Some(ExecutionRef {
                    execution_id: execution_id.parse().ok()?,
                    attempt: attempt.parse().ok()?,
                }),
                last_event_sequence: value.last_event_sequence,
                harness_process_state: "active".into(),
            })
        })
        .map(|attempt| {
            let execution = attempt.execution.as_ref().expect("active attempt");
            ((execution.execution_id, execution.attempt), attempt)
        })
        .collect::<BTreeMap<_, _>>();
    drop(active);

    // A terminal Harness event remains an active Runtime attempt until Runtime
    // acknowledges the commit. Include that durable outbox in heartbeats so a
    // normal completion is never mistaken for a lost Computer process.
    for (key, events) in inner.pending_events.lock().await.iter() {
        let Some((execution_id, attempt)) = key.split_once(':') else {
            continue;
        };
        let (Ok(execution_id), Ok(attempt)) = (execution_id.parse(), attempt.parse()) else {
            continue;
        };
        attempts
            .entry((execution_id, attempt))
            .or_insert_with(|| ActiveAttempt {
                execution: Some(ExecutionRef {
                    execution_id,
                    attempt,
                }),
                last_event_sequence: events.keys().next_back().copied().unwrap_or_default(),
                harness_process_state: "terminal_pending_commit".into(),
            });
    }
    attempts.into_values().collect()
}

fn assignment_fingerprint(assign: &AssignExecution) -> String {
    let mut stable = assign.clone();
    stable.execution_token.clear();
    let digest = Sha256::digest(stable.encode_to_vec());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn redact_diagnostic(value: &str, config: &NativeHarnessConfig) -> String {
    config
        .environment
        .values()
        .filter(|secret| !secret.is_empty())
        .fold(value.to_owned(), |message, secret| {
            message.replace(secret, "[redacted]")
        })
}

async fn send_frame(inner: &Inner, frame: ComputerFrame) {
    if let Some(sender) = inner.connection.read().await.clone() {
        let _ = sender.send(frame).await;
    }
}

fn error_code(error: &ComputerError) -> &'static str {
    match error {
        ComputerError::InvalidAssignment(value) if value == "agent_busy" => "agent_busy",
        ComputerError::InvalidAssignment(value) if value == "duplicate_assignment" => {
            "duplicate_assignment"
        }
        ComputerError::InvalidAssignment(value) if value == "assignment_conflict" => {
            "assignment_conflict"
        }
        ComputerError::Resource(_) => "resource_error",
        ComputerError::PathOutsideAgentHome => "path_outside_agent_home",
        ComputerError::Harness(_) => "harness_error",
        _ => "invalid_assignment",
    }
}

fn public_error(error: &ComputerError) -> String {
    match error {
        ComputerError::InvalidAssignment(value) => value.clone(),
        ComputerError::PathOutsideAgentHome => "Content file is outside the Agent directory".into(),
        ComputerError::Resource(_) => "A required resource could not be prepared".into(),
        ComputerError::Harness(_) => "The selected Harness could not start".into(),
        _ => "The execution could not be prepared".into(),
    }
}

fn expanded_root(value: &str) -> Result<PathBuf, ComputerError> {
    if let Some(rest) = value.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| ComputerError::InvalidAssignment("home_directory_unavailable".into()))?;
        Ok(PathBuf::from(home).join(rest))
    } else {
        Ok(PathBuf::from(value))
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
