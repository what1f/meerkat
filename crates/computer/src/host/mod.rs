mod assignment;
mod connection;
mod control;
mod events;
mod failure;
mod liveness;
mod records;

use self::connection::run_loop;
use self::control::active_attempts;
use self::events::restore_event;
use crate::STANDARD_HARNESS_IDS;
use crate::state::{PersistedState, SessionRecovery};
use lankide_harness::{
    HarnessAvailability as NativeHarnessAvailability, HarnessConfig as NativeHarnessConfig,
    HarnessFactory, HarnessSession, ProbeResult,
};
use lankide_protocol::{ActiveAttempt, AgentRunContext, ComputerFrame, ExecutionEvent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock, mpsc, watch};

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
    Link(lankide_protocol::link::LinkError),
    ProtocolVersionUnsupported,
    Harness(lankide_harness::HarnessError),
    InvalidAssignment(String),
    InvalidHarnessRegistration(String),
    Resource(String),
    ResourceIntegrityMismatch,
    PathOutsideAgentHome,
    PersistenceFailed,
    Join(String),
}

impl fmt::Display for ComputerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "computer_io: {error}"),
            Self::Transport(error) => write!(formatter, "computer_transport: {error}"),
            Self::ProtocolVersionUnsupported => formatter.write_str("protocol_version_unsupported"),
            Self::Link(error) => write!(formatter, "computer_link: {error}"),
            Self::Rpc(error) => write!(formatter, "computer_rpc: {error}"),
            Self::Harness(error) => write!(formatter, "computer_harness: {error}"),
            Self::InvalidAssignment(error) => write!(formatter, "invalid_assignment: {error}"),
            Self::InvalidHarnessRegistration(id) => {
                write!(formatter, "invalid_harness_registration: {id}")
            }
            Self::ResourceIntegrityMismatch => formatter.write_str("resource_integrity_mismatch"),
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
impl From<lankide_protocol::link::LinkError> for ComputerError {
    fn from(error: lankide_protocol::link::LinkError) -> Self {
        match error {
            lankide_protocol::link::LinkError::Closed(
                lankide_protocol::link::CloseReason::ProtocolVersionUnsupported,
            ) => Self::ProtocolVersionUnsupported,
            error => Self::Link(error),
        }
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

/// Local Harness diagnostics policy. Each record has one previous segment.
#[derive(Clone, Debug)]
pub struct OutputRecordPolicy {
    pub max_bytes: u64,
    pub retention: Duration,
}

impl Default for OutputRecordPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 4 * 1024 * 1024,
            retention: Duration::from_secs(7 * 24 * 60 * 60),
        }
    }
}

/// Limits for a single attempt, independent of connection heartbeats.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionLiveness {
    pub idle_timeout: Duration,
    pub total_budget: Duration,
    pub cancel_grace: Duration,
}
impl Default for ExecutionLiveness {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(10 * 60),
            total_budget: Duration::from_secs(2 * 60 * 60),
            cancel_grace: Duration::from_secs(2),
        }
    }
}

pub struct Computer {
    liveness: ExecutionLiveness,
    output_records: OutputRecordPolicy,
    capability_cli: Option<PathBuf>,
    link_endpoint: Option<(String, String)>,
    config: ComputerConfig,
    harnesses: HashMap<String, RegisteredHarness>,
}

impl Computer {
    pub fn new(config: ComputerConfig) -> Self {
        Self {
            link_endpoint: None,
            config,
            liveness: ExecutionLiveness::default(),
            output_records: OutputRecordPolicy::default(),
            capability_cli: crate::capability_cli::discover(),
            harnesses: HashMap::new(),
        }
    }

    /// Configure local Pi records and startup cleanup. The size limit applies
    /// to each of the current and previous segments; minimum size is 256 bytes.
    /// Native session history is never expired by this policy.
    pub fn with_output_record_policy(mut self, policy: OutputRecordPolicy) -> Self {
        self.output_records = policy;
        self
    }

    pub fn with_capability_cli(mut self, path: impl Into<PathBuf>) -> Self {
        self.capability_cli = Some(path.into());
        self
    }

    pub fn with_execution_liveness(mut self, policy: ExecutionLiveness) -> Self {
        self.liveness = policy;
        self
    }

    pub fn with_link_endpoint(mut self, url: String, secret: String) -> Self {
        self.link_endpoint = Some((url, secret));
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
        if self.output_records.max_bytes < 256 {
            return Err(ComputerError::InvalidAssignment(
                "invalid_output_record_limit".into(),
            ));
        }
        records::expire(&root, self.output_records.retention)?;
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
            liveness: self.liveness,
            config: self.config,
            output_records: self.output_records,
            capability_cli: self.capability_cli,
            root,
            socket_path,
            link_endpoint: self.link_endpoint,
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
        let cancellations: Vec<_> = inner
            .active
            .lock()
            .await
            .values()
            .map(|active| {
                active.cancel.send_replace(Some(Cancellation {
                    reason: reason.to_owned(),
                    force_close: false,
                }));
                active.terminated.clone()
            })
            .collect();
        for terminated in cancellations {
            liveness::wait_for_stop(terminated, inner.liveness.cancel_grace).await;
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

#[derive(Clone)]
struct Cancellation {
    reason: String,
    force_close: bool,
}

struct ActiveExecution {
    terminated: watch::Receiver<Option<lankide_harness::AgentEventKind>>,
    cancel: watch::Sender<Option<Cancellation>>,
    progress: watch::Sender<tokio::time::Instant>,
    agent_key: String,
    capabilities: std::collections::BTreeSet<lankide_harness::HarnessCapability>,
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
    liveness: ExecutionLiveness,
    output_records: OutputRecordPolicy,
    capability_cli: Option<PathBuf>,
    config: ComputerConfig,
    root: PathBuf,
    socket_path: PathBuf,
    link_endpoint: Option<(String, String)>,
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

fn redact_diagnostic(value: &str, config: &NativeHarnessConfig) -> String {
    config
        .environment
        .values()
        .filter(|secret| !secret.is_empty())
        .fold(value.to_owned(), |message, secret| {
            message.replace(secret, "[redacted]")
        })
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
