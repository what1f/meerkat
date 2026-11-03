mod assignment;
mod cli_versions;
mod connection;
mod control;
mod events;
mod failure;
mod files;
mod harness_test;
mod liveness;
mod proxy;
mod records;
mod sessions;

use self::connection::run_loop;
use self::control::active_attempts;
use self::events::restore_event;
use crate::state::{PersistedState, SessionRecovery};
use meerkat_harness::{
    HarnessAvailability as NativeHarnessAvailability, HarnessConfig as NativeHarnessConfig,
    HarnessFactory, HarnessSession, ProbeResult,
};
use meerkat_protocol::{ActiveAttempt, AgentRunContext, ExecutionEvent, MachineFrame};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock, mpsc, watch};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MachineConfig {
    pub machine_id: String,
    pub instance_id: String,
    pub workspace_root: String,
    pub reconnect: ReconnectPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ReconnectPolicy {
    pub initial: Duration,
    pub maximum: Duration,
    pub multiplier: u32,
    pub healthy_duration: Duration,
    pub handshake_timeout: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(50),
            maximum: Duration::from_secs(5),
            multiplier: 2,
            healthy_duration: Duration::from_secs(60),
            handshake_timeout: Duration::from_secs(10),
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
pub enum MachineError {
    Io(std::io::Error),
    Transport(meerkat_protocol::TransportError),
    Rpc(meerkat_protocol::Status),
    Link(meerkat_protocol::link::LinkError),
    ProtocolVersionUnsupported,
    Harness(meerkat_harness::HarnessError),
    InvalidAssignment(String),
    InvalidHarnessRegistration(String),
    Resource(String),
    ResourceIntegrityMismatch,
    PathOutsideAgentHome,
    PersistenceFailed,
    Join(String),
}

impl fmt::Display for MachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "machine_io: {error}"),
            Self::Transport(error) => write!(formatter, "machine_transport: {error}"),
            Self::ProtocolVersionUnsupported => formatter.write_str("protocol_version_unsupported"),
            Self::Link(error) => write!(formatter, "machine_link: {error}"),
            Self::Rpc(error) => write!(formatter, "machine_rpc: {error}"),
            Self::Harness(error) => write!(formatter, "machine_harness: {error}"),
            Self::InvalidAssignment(error) => write!(formatter, "invalid_assignment: {error}"),
            Self::InvalidHarnessRegistration(id) => {
                write!(formatter, "invalid_harness_registration: {id}")
            }
            Self::ResourceIntegrityMismatch => formatter.write_str("resource_integrity_mismatch"),
            Self::Resource(error) => write!(formatter, "resource: {error}"),
            Self::PathOutsideAgentHome => formatter.write_str("path_outside_agent_home"),
            Self::PersistenceFailed => formatter.write_str("machine_state_persistence_failed"),
            Self::Join(error) => write!(formatter, "machine_join: {error}"),
        }
    }
}

impl std::error::Error for MachineError {}

impl From<std::io::Error> for MachineError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<meerkat_protocol::TransportError> for MachineError {
    fn from(value: meerkat_protocol::TransportError) -> Self {
        Self::Transport(value)
    }
}
impl From<meerkat_protocol::link::LinkError> for MachineError {
    fn from(error: meerkat_protocol::link::LinkError) -> Self {
        match error {
            meerkat_protocol::link::LinkError::Closed(
                meerkat_protocol::link::CloseReason::ProtocolVersionUnsupported,
            ) => Self::ProtocolVersionUnsupported,
            error => Self::Link(error),
        }
    }
}
impl From<meerkat_protocol::Status> for MachineError {
    fn from(value: meerkat_protocol::Status) -> Self {
        Self::Rpc(value)
    }
}
impl From<meerkat_harness::HarnessError> for MachineError {
    fn from(value: meerkat_harness::HarnessError) -> Self {
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

pub struct Machine {
    session_idle_timeout: Duration,
    liveness: ExecutionLiveness,
    output_records: OutputRecordPolicy,
    capability_cli: Option<PathBuf>,
    link_endpoint: Option<(String, String)>,
    cert_sha256: Option<String>,
    device_credential: String,
    config: MachineConfig,
    harnesses: HashMap<String, RegisteredHarness>,
}

impl Machine {
    pub fn new(config: MachineConfig) -> Self {
        Self {
            session_idle_timeout: sessions::DEFAULT_IDLE_TIMEOUT,
            link_endpoint: None,
            cert_sha256: None,
            device_credential: String::new(),
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

    pub fn with_session_idle_timeout(mut self, timeout: Duration) -> Self {
        self.session_idle_timeout = timeout;
        self
    }

    pub fn with_device_credential(mut self, credential: String) -> Self {
        self.device_credential = credential;
        self
    }

    pub fn with_certificate_pin(mut self, pin: Option<String>) -> Self {
        self.cert_sha256 = pin;
        self
    }
    pub fn with_link_endpoint(mut self, url: String, secret: String) -> Self {
        self.device_credential = secret.clone();
        self.link_endpoint = Some((url, secret));
        self
    }

    pub fn config(&self) -> &MachineConfig {
        &self.config
    }

    pub fn register_harness(&mut self, harness: RegisteredHarness) -> Result<(), MachineError> {
        let harness_id = harness.factory.info().harness_id.clone();
        if harness_id.is_empty() || harness_id != harness.probe_config.harness_id {
            return Err(MachineError::InvalidHarnessRegistration(harness_id));
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
        runtime_run: impl Into<PathBuf>,
        shutdown: F,
    ) -> Result<(), MachineError>
    where
        F: Future<Output = ()> + Send,
    {
        let handle = self.spawn(runtime_run).await?;
        tokio::select! {
            result = handle.join() => result,
            _ = shutdown => {
                handle.shutdown("machine_shutdown").await;
                handle.join().await
            }
        }
    }

    pub async fn spawn(
        self,
        runtime_run: impl Into<PathBuf>,
    ) -> Result<MachineHandle, MachineError> {
        let runtime_run = runtime_run.into();
        let root = expanded_root(&self.config.workspace_root)?;
        std::fs::create_dir_all(&root)?;
        if self.output_records.max_bytes < 256 {
            return Err(MachineError::InvalidAssignment(
                "invalid_output_record_limit".into(),
            ));
        }
        records::expire(&root, self.output_records.retention)?;
        let state_path = root.join(".machine/state.json");
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
        let mut model_catalogs = HashMap::new();
        let cli_versions = cli_versions::CliVersions::load(root.join(".machine/cli-versions.json"));
        for (harness_id, registration) in self.harnesses {
            let mut probe = match cli_versions
                .probe(registration.factory.as_ref(), &registration.probe_config)
                .await
            {
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
            let models = tokio::time::timeout(
                Duration::from_secs(5),
                registration.factory.models(&registration.probe_config),
            )
            .await;
            let catalog = match models {
                Ok(Ok(models)) => (
                    Some(meerkat_protocol::ModelCatalog {
                        models: models
                            .into_iter()
                            .map(|model| meerkat_protocol::ModelCatalogItem {
                                available: true,
                                model_id: model.id,
                                display_name: model.name,
                                description: model.description,
                                supported_thinking_levels: model.thinking_levels,
                            })
                            .collect(),
                    }),
                    String::new(),
                ),
                Ok(Err(error)) => (
                    None,
                    redact_diagnostic(&error.to_string(), &registration.probe_config),
                ),
                Err(_) => (None, "Model discovery timed out".into()),
            };
            model_catalogs.insert(harness_id.clone(), catalog);
            defaults.insert(harness_id.clone(), registration.probe_config);
            factories.insert(harness_id.clone(), registration.factory);
            probes.insert(harness_id, probe);
        }
        let (stop_tx, stop_rx) = watch::channel(None);
        let mut resource_http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60));
        if let Some(pin) = self.cert_sha256.as_deref() {
            resource_http =
                resource_http.use_preconfigured_tls(meerkat_protocol::tls::client_config(pin)?);
        }
        let resource_http = resource_http
            .build()
            .map_err(|error| MachineError::Resource(error.to_string()))?;
        let inner = Arc::new(Inner {
            cli_versions,
            session_idle_timeout: self.session_idle_timeout,
            idle_sessions: Mutex::new(HashMap::new()),
            resource_http,
            liveness: self.liveness,
            config: self.config,
            output_records: self.output_records,
            capability_cli: self.capability_cli,
            root,
            runtime_run,
            link_endpoint: self.link_endpoint,
            cert_sha256: self.cert_sha256,
            device_credential: self.device_credential,
            state_path,
            factories,
            defaults,
            probes,
            model_catalogs,
            state: Mutex::new(persisted),
            starting: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            assignment_tasks: Mutex::new(Vec::new()),
            pending_events: Mutex::new(pending_events),
            outstanding_tools: Mutex::new(HashSet::new()),
            deferred_tool_results: Mutex::new(HashMap::new()),
            capability_calls: Mutex::new(HashMap::new()),
            connection: RwLock::new(None),
            connected: watch::channel(false).0,
            persistence_failed: AtomicBool::new(false),
            stop_tx,
        });
        let handle = MachineHandle {
            inner: inner.clone(),
            join: Arc::new(Mutex::new(None)),
        };
        tokio::spawn(cli_versions::watch(Arc::downgrade(&inner)));
        let task = tokio::spawn(run_loop(inner, stop_rx));
        *handle.join.lock().await = Some(task);
        Ok(handle)
    }
}

#[derive(Clone)]
pub struct MachineHandle {
    inner: Arc<Inner>,
    join: Arc<Mutex<Option<tokio::task::JoinHandle<Result<(), MachineError>>>>>,
}

impl MachineHandle {
    pub fn instance_id(&self) -> &str {
        &self.inner.config.instance_id
    }

    pub async fn active_count(&self) -> usize {
        self.inner.active.lock().await.len()
    }

    pub async fn status(&self) -> MachineStatus {
        let connected = *self.inner.connected.borrow();
        MachineStatus {
            connected,
            active_attempts: active_attempts(&self.inner).await,
        }
    }

    /// Wait until this Machine has an authenticated Runtime connection.
    pub async fn wait_until_connected(&self) {
        let mut connected = self.inner.connected.subscribe();
        let _ = connected.wait_for(|value| *value).await;
    }

    pub async fn shutdown(&self, reason: impl Into<String>) {
        let reason = reason.into();
        Self::cancel_inner(&self.inner, &reason).await;
        let _ = self.inner.stop_tx.send(Some(reason));
    }

    pub async fn join(&self) -> Result<(), MachineError> {
        // Keep the handle in its slot while awaiting: run() selects this future
        // against shutdown and must be able to await it again after cancellation.
        let mut slot = self.join.lock().await;
        let result = match slot.as_mut() {
            Some(task) => task
                .await
                .map_err(|error| MachineError::Join(error.to_string()))?,
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
        sessions::close_all(inner).await;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineStatus {
    pub connected: bool,
    pub active_attempts: Vec<ActiveAttempt>,
}

#[derive(Clone)]
struct Cancellation {
    reason: String,
    force_close: bool,
}

struct ActiveExecution {
    session_identity: String,
    terminated: watch::Receiver<Option<meerkat_harness::AgentEventKind>>,
    cancel: watch::Sender<Option<Cancellation>>,
    progress: watch::Sender<tokio::time::Instant>,
    agent_key: String,
    capabilities: std::collections::BTreeSet<meerkat_harness::HarnessCapability>,
    session: Arc<dyn HarnessSession>,
    agent_home: PathBuf,
    context: AgentRunContext,
    delivery_lock: Arc<tokio::sync::Mutex<()>>,
    accepting_deliveries: bool,
    accepting_tool_results: bool,
    last_event_sequence: u64,
}

struct StartingExecution {
    assignment_key: String,
    cancel: watch::Sender<Option<String>>,
}

struct Inner {
    cli_versions: cli_versions::CliVersions,
    session_idle_timeout: Duration,
    idle_sessions: Mutex<HashMap<String, sessions::IdleSession>>,
    resource_http: reqwest::Client,
    liveness: ExecutionLiveness,
    output_records: OutputRecordPolicy,
    capability_cli: Option<PathBuf>,
    config: MachineConfig,
    root: PathBuf,
    runtime_run: PathBuf,
    link_endpoint: Option<(String, String)>,
    cert_sha256: Option<String>,
    device_credential: String,
    state_path: PathBuf,
    factories: HashMap<String, Arc<dyn HarnessFactory>>,
    defaults: HashMap<String, NativeHarnessConfig>,
    probes: HashMap<String, ProbeResult>,
    model_catalogs: HashMap<String, (Option<meerkat_protocol::ModelCatalog>, String)>,
    state: Mutex<PersistedState>,
    starting: Mutex<HashMap<String, StartingExecution>>,
    active: Mutex<HashMap<String, ActiveExecution>>,
    assignment_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pending_events: Mutex<HashMap<String, BTreeMap<u64, ExecutionEvent>>>,
    // These two maps are the Machine-owned routing point for Runtime results.
    // Delivery into HarnessSession awaits the shared Harness contract method.
    outstanding_tools: Mutex<HashSet<(String, String)>>,
    deferred_tool_results: Mutex<HashMap<(String, String), meerkat_protocol::ToolResult>>,
    capability_calls: Mutex<HashMap<String, proxy::PendingCall>>,
    connection: RwLock<Option<mpsc::Sender<MachineFrame>>>,
    connected: watch::Sender<bool>,
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

fn expanded_root(value: &str) -> Result<PathBuf, MachineError> {
    if let Some(rest) = value.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| MachineError::InvalidAssignment("home_directory_unavailable".into()))?;
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
