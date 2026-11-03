use super::*;
use meerkat_core::PersonId;
use meerkat_protocol::{AgentRef, EnvironmentVariable, SessionAction};
use meerkat_provider::{
    OperationProgress, OperationUpdate, ProviderAgentFile, ProviderFuture as SdkFuture,
    ProviderLogger, ProviderMessage, ProviderPersonIdentity, ProviderResource, ProviderStorage,
    ResourceSaver, ResourceUpload, SecretReader, SecretValue as ProviderSecret,
};
use meerkat_storage::{AgentRecord, AttentionRecord, PersonRecord};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

const APP_NEW: u8 = 0;
const APP_READY: u8 = 1;
const APP_RUNNING: u8 = 2;
const APP_STOPPED: u8 = 3;

#[derive(Clone)]
pub struct RuntimeApplication {
    pub(crate) inner: Arc<ApplicationInner>,
}

pub(crate) struct ApplicationInner {
    pub(crate) pairing_validity_ms: AtomicI64,
    pub(crate) machine_registration: tokio::sync::Mutex<()>,
    runtime: StdMutex<Option<Arc<Runtime>>>,
    factories: ProviderFactoryRegistry,
    state: AtomicU8,
    sequence: AtomicU64,
    events: broadcast::Sender<meerkat_protocol::ManagementEvent>,
    recovery: StdMutex<Option<RecoveryReport>>,
    machine_connected: AtomicBool,
    broker: MachineBroker,
    resources: StdMutex<Option<Arc<dyn ResourceStore>>>,
    profiles: AgentProfileFiles,
    workspace_tasks: StdMutex<BTreeMap<WorkspaceId, JoinHandle<()>>>,
    execution_tasks: StdMutex<BTreeMap<String, JoinHandle<()>>>,
    shutting_down: AtomicBool,
    scheduler_started: AtomicBool,
    background_tasks: StdMutex<tokio::task::JoinSet<()>>,
    shutdown_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationReadiness {
    pub management_ready: bool,
    pub providers_started: bool,
    pub machine_connected: bool,
    pub state: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub recovering_execution_ids: Vec<i64>,
    pub unknown_send_keys: Vec<String>,
    pub actions: Vec<RecoveryAction>,
    pub transitioned_executions: usize,
    pub transitioned_sends: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    WaitForMachine { execution_id: i64 },
    ReconcileProviderSend { request_key: String },
}

impl RuntimeApplication {
    pub(crate) async fn stop_agent_execution(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<(), RuntimeError> {
        let key = format!("{workspace_id}:{agent_id}");
        if let Some(task) = self
            .inner
            .execution_tasks
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(&key)
        {
            task.abort();
        }
        let runtime = self.runtime()?;
        for execution_id in runtime
            .database()
            .active_execution_ids_for_agent(workspace_id, agent_id)
            .await?
        {
            self.inner
                .broker
                .cancel_execution(execution_id, "Agent was deleted")?;
            runtime
                .database()
                .cancel_execution(execution_id, now_millis())
                .await?;
        }
        Ok(())
    }
    pub async fn open(
        config: RuntimeConfig,
        factories: ProviderFactoryRegistry,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_shutdown_timeout(config, factories, Duration::from_secs(30)).await
    }

    /// Sets the maximum wait for Provider receivers to release storage on shutdown.
    pub async fn open_with_shutdown_timeout(
        config: RuntimeConfig,
        factories: ProviderFactoryRegistry,
        shutdown_timeout: Duration,
    ) -> Result<Self, RuntimeError> {
        let profiles = AgentProfileFiles::new(&config.database.path);
        let resource_root = config
            .database
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("resources");
        let runtime = Arc::new(
            tokio::task::spawn_blocking(move || Runtime::new(config))
                .await
                .map_err(|error| RuntimeError::Application(error.to_string()))??,
        );
        let broker = MachineBroker::with_database(runtime.database());
        let resources: Arc<dyn ResourceStore> = Arc::new(LocalResourceStore::with_database(
            runtime.database(),
            resource_root,
        ));
        let application = Self {
            inner: Arc::new(ApplicationInner {
                pairing_validity_ms: AtomicI64::new(600_000),
                machine_registration: tokio::sync::Mutex::new(()),
                runtime: StdMutex::new(Some(runtime)),
                factories,
                state: AtomicU8::new(APP_NEW),
                sequence: AtomicU64::new(0),
                events: broadcast::channel(256).0,
                recovery: StdMutex::new(None),
                machine_connected: AtomicBool::new(false),
                broker,
                resources: StdMutex::new(Some(resources)),
                profiles,
                workspace_tasks: StdMutex::new(BTreeMap::new()),
                execution_tasks: StdMutex::new(BTreeMap::new()),
                shutting_down: AtomicBool::new(false),
                scheduler_started: AtomicBool::new(false),
                background_tasks: StdMutex::new(tokio::task::JoinSet::new()),
                shutdown_timeout,
            }),
        };
        application
            .runtime()?
            .database()
            .observe_activity(Arc::downgrade(&application.inner));
        application.load_persisted_workspaces().await?;
        application.inner.state.store(APP_READY, Ordering::Release);
        Ok(application)
    }

    pub async fn start(&self) -> Result<(), RuntimeError> {
        let runtime = self.runtime()?;
        self.start_scheduler()?;
        self.inner.state.store(APP_RUNNING, Ordering::Release);
        for workspace in runtime.database().list_workspace_records().await? {
            if workspace.active {
                match runtime.resume_workspace(&workspace.id, now_millis()).await {
                    Ok(()) => {
                        runtime
                            .database()
                            .set_workspace_setup(
                                &workspace.id,
                                meerkat_storage::WorkspaceSetupStatus::Ready,
                                None,
                                now_millis(),
                            )
                            .await?;
                        self.start_workspace_loop(workspace.id)?;
                    }
                    Err(error) => {
                        log::error!(
                            "[runtime] workspace={} Provider startup failed: {error}",
                            workspace.id
                        );
                        runtime
                            .database()
                            .set_workspace_setup(
                                &workspace.id,
                                meerkat_storage::WorkspaceSetupStatus::Error,
                                Some("provider_setup_failed"),
                                now_millis(),
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn recover(&self, now: i64) -> Result<RecoveryReport, RuntimeError> {
        let runtime = self.runtime()?;
        let state = runtime.database().recover_startup(now).await?;
        let mut report = RecoveryReport {
            recovering_execution_ids: Vec::new(),
            unknown_send_keys: Vec::new(),
            actions: Vec::new(),
            transitioned_executions: state.transitioned_executions,
            transitioned_sends: state.transitioned_sends,
        };
        for execution in state.executions {
            let execution_id = execution.execution.execution_id;
            report.recovering_execution_ids.push(execution_id);
            report
                .actions
                .push(RecoveryAction::WaitForMachine { execution_id });
        }
        for send in state.sends {
            let request_key = send.attempt.request_key;
            report.unknown_send_keys.push(request_key.clone());
            report
                .actions
                .push(RecoveryAction::ReconcileProviderSend { request_key });
        }
        *self
            .inner
            .recovery
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)? = Some(report.clone());
        Ok(report)
    }

    pub fn recovery_report(&self) -> Result<Option<RecoveryReport>, RuntimeError> {
        Ok(self
            .inner
            .recovery
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .clone())
    }

    pub fn readiness(&self) -> ApplicationReadiness {
        let state = self.inner.state.load(Ordering::Acquire);
        ApplicationReadiness {
            management_ready: matches!(state, APP_READY | APP_RUNNING),
            providers_started: state == APP_RUNNING,
            machine_connected: self.inner.machine_connected.load(Ordering::Acquire),
            state: match state {
                APP_NEW => "starting",
                APP_READY => "ready",
                APP_RUNNING => "running",
                APP_STOPPED => "stopped",
                _ => "error",
            },
        }
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.begin_shutdown().await?;
        self.finish_shutdown().await
    }

    pub async fn begin_shutdown(&self) -> Result<(), RuntimeError> {
        self.inner.shutting_down.store(true, Ordering::Release);
        let tasks = {
            let mut tasks = self
                .inner
                .workspace_tasks
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            std::mem::take(&mut *tasks)
                .into_values()
                .collect::<Vec<_>>()
        };
        for task in &tasks {
            task.abort();
        }
        for task in tasks {
            let _ = task.await;
        }
        let execution_tasks = {
            let mut tasks = self
                .inner
                .execution_tasks
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            std::mem::take(&mut *tasks)
                .into_values()
                .collect::<Vec<_>>()
        };
        for task in &execution_tasks {
            task.abort();
        }
        for task in execution_tasks {
            let _ = task.await;
        }
        self.inner.broker.shutdown()?;
        let mut background_tasks = {
            let mut tasks = self
                .inner
                .background_tasks
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            std::mem::take(&mut *tasks)
        };
        background_tasks.abort_all();
        while background_tasks.join_next().await.is_some() {}
        let runtime = self.runtime()?;
        runtime.shutdown().await?;
        self.inner.machine_connected.store(false, Ordering::Release);
        drop(runtime);
        Ok(())
    }

    pub async fn finish_shutdown(&self) -> Result<(), RuntimeError> {
        let runtime = self
            .inner
            .runtime
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .take()
            .ok_or(RuntimeError::ApplicationStopped)?;
        let resources = self
            .inner
            .resources
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .take();
        drop(resources);
        Arc::try_unwrap(runtime)
            .map_err(|_| RuntimeError::RuntimeStillShared)?
            .close(self.inner.shutdown_timeout)
            .await?;
        self.inner.state.store(APP_STOPPED, Ordering::Release);
        Ok(())
    }

    pub(crate) fn spawn_background(
        &self,
        future: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<(), RuntimeError> {
        let mut tasks = self
            .inner
            .background_tasks
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::ApplicationStopped);
        }
        while tasks.try_join_next().is_some() {}
        tasks.spawn(future);
        Ok(())
    }

    pub fn subscribe_management(&self) -> broadcast::Receiver<meerkat_protocol::ManagementEvent> {
        self.inner.events.subscribe()
    }

    pub fn machine_broker(&self) -> MachineBroker {
        self.inner.broker.clone()
    }

    pub fn resource_store(&self) -> Result<Arc<dyn ResourceStore>, RuntimeError> {
        self.inner
            .resources
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .clone()
            .ok_or(RuntimeError::ApplicationStopped)
    }

    pub(crate) fn agent_profiles(&self) -> &AgentProfileFiles {
        &self.inner.profiles
    }

    pub(crate) fn set_machine_connected(&self, connected: bool) {
        self.inner
            .machine_connected
            .store(connected, Ordering::Release);
    }

    pub(crate) fn factories(&self) -> ProviderFactoryRegistry {
        self.inner.factories.clone()
    }

    pub(crate) fn start_workspace_loop(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), RuntimeError> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::ApplicationStopped);
        }
        self.start_scheduler()?;
        self.runtime()?.database().wake();
        let mut tasks = self
            .inner
            .workspace_tasks
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        if tasks.contains_key(&workspace_id) {
            return Ok(());
        }
        let application = self.clone();
        let task_workspace = workspace_id.clone();
        tasks.insert(
            workspace_id,
            tokio::spawn(async move {
                application.consume_workspace(task_workspace).await;
            }),
        );
        Ok(())
    }

    pub(crate) async fn stop_workspace_loop(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<(), RuntimeError> {
        let task = self
            .inner
            .workspace_tasks
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(workspace_id);
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }

    /// Configure the lifetime of newly issued resource and capability tokens.
    pub fn set_execution_token_lifetime(
        &self,
        duration: std::time::Duration,
    ) -> Result<(), RuntimeError> {
        self.runtime()?.set_execution_token_lifetime(duration)
    }

    pub(crate) fn runtime(&self) -> Result<Arc<Runtime>, RuntimeError> {
        self.inner
            .runtime
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .clone()
            .ok_or(RuntimeError::ApplicationStopped)
    }

    async fn load_persisted_workspaces(&self) -> Result<(), RuntimeError> {
        let runtime = self.runtime()?;
        for workspace in runtime.database().list_workspace_records().await? {
            if self
                .attach_workspace(&runtime, workspace.clone())
                .await
                .is_err()
            {
                runtime
                    .database()
                    .set_workspace_setup(
                        &workspace.id,
                        meerkat_storage::WorkspaceSetupStatus::Error,
                        Some("provider_setup_failed"),
                        now_millis(),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn consume_workspace(&self, workspace_id: WorkspaceId) {
        let Ok(runtime) = self.runtime() else {
            return;
        };
        loop {
            if self.inner.shutting_down.load(Ordering::Acquire) {
                break;
            }
            match runtime.next_provider_event(&workspace_id).await {
                Ok(Some(event)) => {
                    match runtime
                        .accept_provider_event(&workspace_id, event, now_millis())
                        .await
                    {
                        Ok(CoordinatedEvent::Message {
                            steer_deliveries, ..
                        }) => {
                            for delivery in steer_deliveries {
                                if let Err(error) = self.inner.broker.deliver_steer(delivery).await
                                {
                                    log::error!(
                                        "[runtime] workspace={workspace_id} steer delivery failed: {error}"
                                    );
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => log::error!(
                            "[runtime] workspace={workspace_id} event acceptance failed: {error}"
                        ),
                    }
                }
                // None means the Provider event channel has closed.
                Ok(None) | Err(RuntimeError::WorkspacePaused(_)) => break,
                Err(error) => {
                    log::error!("[runtime] workspace={workspace_id} event receive failed: {error}");
                    break;
                }
            }
        }
    }

    fn start_scheduler(&self) -> Result<(), RuntimeError> {
        if self.inner.scheduler_started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let application = self.clone();
        self.spawn_background(async move {
            application.schedule_work().await;
        })
    }

    async fn schedule_work(&self) {
        let Ok(runtime) = self.runtime() else {
            return;
        };
        let changes = runtime.database().changes();
        loop {
            let changed = changes.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            for (workspace, agent) in runtime.database().take_management_changes() {
                self.emit_management(
                    if agent.is_some() {
                        meerkat_protocol::ManagementEventKind::AgentUpdated
                    } else {
                        meerkat_protocol::ManagementEventKind::WorkspaceUpdated
                    },
                    &workspace,
                    agent.as_ref(),
                    now_millis(),
                );
            }
            let next_due = match self.schedule_ready_work(&runtime).await {
                Ok(due) => due,
                Err(error) => {
                    log::error!("[runtime] scheduling failed: {error}");
                    Some(now_millis().saturating_add(1000))
                }
            };
            if let Some(due) = next_due {
                let delay = Duration::from_millis(due.saturating_sub(now_millis()).max(0) as u64);
                tokio::select! {
                    _ = &mut changed => {},
                    _ = tokio::time::sleep(delay) => {},
                }
            } else {
                changed.await;
            }
        }
    }

    async fn schedule_ready_work(
        &self,
        runtime: &Arc<Runtime>,
    ) -> Result<Option<i64>, RuntimeError> {
        // Lease expiry is global and remains active with no receiving Workspace.
        let mut next_due = runtime.database().next_attempt_lease_at().await?;
        if next_due.is_some_and(|due| due <= now_millis()) {
            self.inner.broker.expire_leases(now_millis()).await?;
            next_due = runtime.database().next_attempt_lease_at().await?;
        }
        for workspace in runtime.database().list_workspace_records().await? {
            let workspace_id = workspace.id;
            if !runtime.is_workspace_active(&workspace_id).unwrap_or(false) {
                continue;
            }
            let agents = runtime.database().list_agent_records(&workspace_id).await?;
            for agent in agents {
                let connected = self.inner.broker.can_dispatch(
                    &agent.machine_id,
                    &workspace_id,
                    &agent.agent_id,
                );
                let task_key = format!("{}:{}", workspace_id, agent.agent_id);
                let should_start = self
                    .inner
                    .execution_tasks
                    .lock()
                    .map(|mut tasks| {
                        tasks.retain(|_, task| !task.is_finished());
                        !tasks.contains_key(&task_key)
                    })
                    .unwrap_or(false);
                if !should_start {
                    continue;
                }
                let due = runtime
                    .database()
                    .next_agent_work_at(&workspace_id, &agent.agent_id, connected)
                    .await?;
                let Some(due) = due else {
                    continue;
                };
                if due > now_millis() {
                    next_due = Some(next_due.map_or(due, |current: i64| current.min(due)));
                    continue;
                }
                let executor = self.inner.broker.executor(agent.machine_id);
                let execution_runtime = runtime.clone();
                let execution_application = self.clone();
                let execution_workspace = workspace_id.clone();
                let execution_agent = agent.agent_id.clone();
                let mut tasks = self
                    .inner
                    .execution_tasks
                    .lock()
                    .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
                let task = tokio::spawn(async move {
                    let previous_pause = execution_runtime
                        .database()
                        .agent_pause_reason(&execution_workspace, &execution_agent)
                        .await
                        .ok()
                        .flatten();
                    let result = if connected {
                        execution_runtime
                            .execute_ready(
                                &execution_workspace,
                                &execution_agent,
                                &executor,
                                now_millis(),
                            )
                            .await
                    } else {
                        execution_runtime
                            .apply_pending_failure_policies(&execution_workspace, &execution_agent)
                            .await
                            .map(|_| ExecutionProgress::Idle)
                    };
                    let changed = matches!(&result,Ok(progress) if !matches!(progress,ExecutionProgress::Idle))
                        || execution_runtime
                            .database()
                            .agent_pause_reason(&execution_workspace, &execution_agent)
                            .await
                            .ok()
                            .flatten()
                            != previous_pause;
                    let retry_after_error = result.is_err();
                    if let Err(error) = result {
                        log::error!(
                            "[runtime] workspace={execution_workspace} agent={execution_agent} execution failed: {error}"
                        );
                    }
                    if changed {
                        execution_application.emit_management(
                            meerkat_protocol::ManagementEventKind::AgentUpdated,
                            &execution_workspace,
                            Some(&execution_agent),
                            now_millis(),
                        );
                    }
                    if let Err(error) = execution_runtime
                        .finish_paused_workspace(&execution_workspace)
                        .await
                    {
                        log::error!(
                            "[runtime] workspace={execution_workspace} pause completion failed: {error}"
                        );
                    }
                    // A failed Provider operation has no due time in the inbox.
                    // Back off that operation before making the Agent eligible again.
                    if retry_after_error {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    if let Ok(mut tasks) = execution_application.inner.execution_tasks.lock() {
                        tasks.remove(&format!("{}:{}", execution_workspace, execution_agent));
                    }
                    execution_runtime.database().wake();
                });
                tasks.insert(task_key, task);
            }
        }
        Ok(next_due)
    }

    pub(crate) async fn attach_workspace(
        &self,
        runtime: &Arc<Runtime>,
        workspace: WorkspaceRecord,
    ) -> Result<(), RuntimeError> {
        let factory = self.inner.factories.factory(&workspace.provider_id)?;
        let services = workspace_services(runtime.database(), self.resource_store()?, &workspace);
        let factory_workspace = workspace.clone();
        runtime
            .register_provider_instance_factory(
                workspace.id.clone(),
                Arc::new(move || factory.create(&factory_workspace, services.clone())),
            )
            .await?;
        for agent in runtime.database().list_agent_records(&workspace.id).await? {
            let config = agent_runtime_config(&workspace, agent)?;
            runtime.configure_agent(&workspace.id, config).await?;
        }
        Ok(())
    }

    pub(crate) fn emit_machine_catalog_changed(&self) {
        let sequence = self.inner.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.inner.events.send(meerkat_protocol::ManagementEvent {
            sequence,
            occurred_at: now_millis(),
            kind: meerkat_protocol::ManagementEventKind::MachineCatalogChanged,
            workspace: None,
            agent: None,
        });
    }

    pub(crate) fn emit_management(
        &self,
        kind: meerkat_protocol::ManagementEventKind,
        workspace: &WorkspaceId,
        agent: Option<&AgentId>,
        occurred_at: i64,
    ) {
        let sequence = self.inner.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.inner.events.send(meerkat_protocol::ManagementEvent {
            sequence,
            occurred_at,
            kind,
            workspace: Some(WorkspaceRef {
                workspace_id: workspace.to_string(),
            }),
            agent: agent.map(|id| AgentRef {
                workspace_id: workspace.to_string(),
                agent_id: id.to_string(),
            }),
        });
    }
}

pub(crate) fn agent_runtime_config(
    workspace: &WorkspaceRecord,
    agent: AgentRecord,
) -> Result<AgentRuntimeConfig, RuntimeError> {
    let attention = agent
        .attention
        .iter()
        .map(|(channel, policy)| {
            let policy = match policy {
                AttentionRecord::Mention => AttentionPolicy::Mention,
                AttentionRecord::Always => AttentionPolicy::Always,
                AttentionRecord::Mute => AttentionPolicy::Mute,
                AttentionRecord::Schedule { interval_minutes } => AttentionPolicy::Schedule(
                    ScheduleInterval::try_from(*interval_minutes)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                ),
            };
            Ok((channel.clone(), policy))
        })
        .collect::<Result<BTreeMap<_, _>, RuntimeError>>()?;
    let config_fingerprint = lowercase_sha256(
        serde_json::json!({
            "harness": agent.harness_id,
            "model": agent.model,
            "thinking": agent.thinking,
            "cli": agent.cli_path,
            "environment": agent.env,
            "launch_arguments": agent.launch_arguments,
        })
        .to_string()
        .as_bytes(),
    );
    let session = SessionDirective {
        logical_session_id: agent
            .session_config
            .get("logical_session_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(agent.agent_id.as_str())
            .to_owned(),
        action: SessionAction::ResumeOrStart as i32,
        native_session_id: agent
            .session_config
            .get("native_session_id")
            .and_then(serde_json::Value::as_str)
            .filter(|_| {
                agent
                    .session_config
                    .get("native_session_fingerprint")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|id| id == config_fingerprint)
                    && agent
                        .session_config
                        .get("native_session_harness")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(|id| id == agent.harness_id)
                    && agent
                        .session_config
                        .get("native_session_machine")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(|id| id == agent.machine_id)
            })
            .map(str::to_owned),
        config_fingerprint: config_fingerprint.clone(),
    };
    Ok(AgentRuntimeConfig {
        agent_id: agent.agent_id.clone(),
        provider_sender_id: agent
            .session_config
            .get("provider_sender_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(agent.agent_id.as_str())
            .to_owned(),
        attention,
        auto_reaction: agent.auto_reaction.clone(),
        agent_config: serde_json::json!({
            "id": agent.agent_id,
            "name": agent.name,
            "role": agent.role,
            "bio": agent.bio,
            "soul": agent.soul,
        }),
        provider_agent_config: agent.provider_agent_config,
        agent_profile: AgentRuntimeProfile {
            agent: Some(AgentRef {
                workspace_id: workspace.id.to_string(),
                agent_id: agent.agent_id.to_string(),
            }),
            name: agent.name,
            role: agent.role,
            bio: agent.bio,
            soul: agent.soul,
        },
        machine_id: agent.machine_id,
        harness: HarnessConfig {
            harness_id: agent.harness_id,
            executable_path: agent.cli_path.unwrap_or_default(),
            launch_arguments: agent.launch_arguments,
            model: agent.model,
            thinking_level: agent.thinking,
            environment_variables: agent
                .env
                .into_iter()
                .map(|(name, value)| EnvironmentVariable { name, value })
                .collect(),
        },
        session,
        execution_token: agent
            .session_config
            .get("execution_token")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

pub(crate) fn workspace_services(
    storage: Database,
    resources: Arc<dyn ResourceStore>,
    workspace: &WorkspaceRecord,
) -> HostServices {
    workspace_services_with_secrets(storage, resources, workspace, BTreeMap::new())
}

pub(crate) fn workspace_services_with_secrets(
    storage: Database,
    resources: Arc<dyn ResourceStore>,
    workspace: &WorkspaceRecord,
    staged_secrets: BTreeMap<String, ProviderSecret>,
) -> HostServices {
    let adapter = Arc::new(WorkspaceServices {
        storage,
        workspace_id: workspace.id.clone(),
        namespace: workspace.provider_id.to_string(),
        provider_instance_id: workspace.provider_id.clone(),
        resources,
        staged_secrets: Arc::new(staged_secrets),
    });
    HostServices {
        secret_reader: adapter.clone(),
        secret_writer: adapter.clone(),
        storage: adapter.clone(),
        logger: adapter.clone(),
        resource_saver: adapter.clone(),
        operation_progress: adapter,
        event_capacity: 256,
        invocation_timeout: std::time::Duration::from_secs(30),
    }
}

struct WorkspaceServices {
    storage: Database,
    workspace_id: WorkspaceId,
    namespace: String,
    provider_instance_id: ProviderId,
    resources: Arc<dyn ResourceStore>,
    staged_secrets: Arc<BTreeMap<String, ProviderSecret>>,
}
impl SecretReader for WorkspaceServices {
    fn read<'a>(&'a self, name: &'a str) -> SdkFuture<'a, ProviderSecret> {
        Box::pin(async move {
            if let Some(value) = self.staged_secrets.get(name) {
                return Ok(value.clone());
            }
            self.storage
                .read_secret(&self.workspace_id, name)
                .await
                .map_err(provider_storage_error)?
                .map(|value| ProviderSecret::new(value.expose_secret()))
                .ok_or(ProviderError::SecretAccessDenied)
        })
    }
}
impl meerkat_provider::SecretWriter for WorkspaceServices {
    fn write<'a>(&'a self, name: &'a str, value: ProviderSecret) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            if let Some(scoped) = name.strip_prefix("agent:")
                && let Some((agent_id, secret_name)) = scoped.split_once(':')
            {
                let agent_id = AgentId::new(agent_id)
                    .map_err(|_| ProviderError::InvalidInput { field: "agent_id" })?;
                return self
                    .storage
                    .put_agent_secret(
                        &self.workspace_id,
                        &agent_id,
                        secret_name,
                        meerkat_storage::SecretValue::new(value.expose_secret()),
                        now_millis(),
                    )
                    .await
                    .map_err(provider_storage_error);
            }
            self.storage
                .put_secret(
                    &self.workspace_id,
                    name,
                    meerkat_storage::SecretValue::new(value.expose_secret()),
                    now_millis(),
                )
                .await
                .map(|_| ())
                .map_err(provider_storage_error)
        })
    }
}
impl ProviderStorage for WorkspaceServices {
    fn management_changed(&self) -> SdkFuture<'_, ()> {
        Box::pin(async move {
            self.storage
                .management_changed(self.workspace_id.clone(), None);
            Ok(())
        })
    }

    fn get<'a>(&'a self, key: &'a str) -> SdkFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            self.storage
                .provider_kv_get(&self.workspace_id, &self.namespace, key)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn put<'a>(&'a self, key: &'a str, value: Vec<u8>) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            self.storage
                .provider_kv_put(
                    &self.workspace_id,
                    &self.namespace,
                    key,
                    &value,
                    now_millis(),
                )
                .await
                .map_err(provider_storage_error)
        })
    }
    fn workspace_state<'a>(&'a self) -> SdkFuture<'a, serde_json::Value> {
        Box::pin(async move {
            self.storage
                .workspace_provider_state(&self.workspace_id)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn put_workspace_state<'a>(&'a self, state: serde_json::Value) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            self.storage
                .put_workspace_provider_state(&self.workspace_id, &state)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn agent_config<'a>(&'a self, agent_id: &'a AgentId) -> SdkFuture<'a, serde_json::Value> {
        Box::pin(async move {
            self.storage
                .agent_record(&self.workspace_id, agent_id)
                .await
                .map(|agent| agent.provider_agent_config)
                .map_err(|_| ProviderError::External {
                    operation: "provider_storage",
                    retryable: true,
                })
        })
    }
    fn agent_state<'a>(&'a self, agent_id: &'a AgentId) -> SdkFuture<'a, serde_json::Value> {
        Box::pin(async move {
            self.storage
                .agent_provider_state(&self.workspace_id, agent_id)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn put_agent_state<'a>(
        &'a self,
        agent_id: &'a AgentId,
        state: serde_json::Value,
    ) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            self.storage
                .put_agent_provider_state(&self.workspace_id, agent_id, &state)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn message<'a>(&'a self, message_id: MessageId) -> SdkFuture<'a, ProviderMessage> {
        Box::pin(async move {
            let message = self
                .storage
                .provider_message(&self.workspace_id, message_id.get())
                .await
                .map_err(provider_storage_error)?
                .ok_or(ProviderError::InvalidInput {
                    field: "message_id",
                })?;
            Ok(ProviderMessage {
                message_id,
                provider_message_id: message.provider_message_id,
                channel_id: message.channel_id,
                chat_id: message.provider_chat_id,
            })
        })
    }
    fn resource<'a>(
        &'a self,
        resource: &'a meerkat_core::ResourceRef,
    ) -> SdkFuture<'a, ProviderResource> {
        Box::pin(async move {
            let resource_id =
                ResourceId::new(resource.resource_id).map_err(|_| ProviderError::InvalidInput {
                    field: "resource_id",
                })?;
            let stored = self
                .resources
                .read(&self.workspace_id, resource_id)
                .await
                .map_err(|_| ProviderError::External {
                    operation: "read_runtime_resource",
                    retryable: false,
                })?;
            let stored_resource = meerkat_core::ResourceRef::from(stored.resource);
            if stored_resource != *resource {
                return Err(ProviderError::InvalidInput { field: "resource" });
            }
            Ok(ProviderResource {
                resource: stored_resource,
                bytes: stored.bytes,
            })
        })
    }
    fn get_or_create_person<'a>(
        &'a self,
        external_id: &'a str,
    ) -> SdkFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.storage
                .get_or_create_person(
                    &self.workspace_id,
                    &self.provider_instance_id,
                    external_id,
                    now_millis(),
                )
                .await
                .map(person_identity)
                .map_err(provider_storage_error)
        })
    }
    fn join_human_team<'a>(
        &'a self,
        person_id: &'a PersonId,
        tenant_id: Option<&'a str>,
        name: Option<&'a str>,
        avatar: Option<&'a meerkat_core::ResourceRef>,
    ) -> SdkFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            let person = self
                .storage
                .join_human_team(
                    &self.workspace_id,
                    person_id,
                    tenant_id,
                    name,
                    avatar,
                    now_millis(),
                )
                .await
                .map_err(|error| {
                    if matches!(&error, StorageError::Conflict(v) if v == "team_enterprise") {
                        ProviderError::InvalidInput {
                            field: "team_enterprise",
                        }
                    } else {
                        provider_storage_error(error)
                    }
                })?;
            self.management_changed().await?;
            Ok(person_identity(person))
        })
    }
    fn claim_human_profile_sync<'a>(
        &'a self,
        person_id: &'a PersonId,
        force: bool,
        now: i64,
    ) -> SdkFuture<'a, bool> {
        Box::pin(async move {
            self.storage
                .claim_human_profile_sync(&self.workspace_id, person_id, force, now)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn human_profile<'a>(
        &'a self,
        person_id: &'a PersonId,
    ) -> SdkFuture<'a, (Option<String>, Option<meerkat_core::ResourceRef>)> {
        Box::pin(async move {
            self.storage
                .human_profile(&self.workspace_id, person_id)
                .await
                .map_err(provider_storage_error)
        })
    }
    fn update_human_profile<'a>(
        &'a self,
        person_id: &'a PersonId,
        name: Option<&'a str>,
        avatar: Option<&'a meerkat_core::ResourceRef>,
        signature: Option<&'a str>,
    ) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            let changed = self
                .storage
                .update_human_profile(
                    &self.workspace_id,
                    person_id,
                    name,
                    avatar,
                    signature,
                    now_millis(),
                )
                .await
                .map_err(provider_storage_error)?;
            if changed {
                self.management_changed().await?;
            }
            Ok(())
        })
    }
    fn person<'a>(&'a self, person_id: &'a PersonId) -> SdkFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.storage
                .person(&self.workspace_id, person_id)
                .await
                .map_err(provider_storage_error)?
                .map(person_identity)
                .ok_or(ProviderError::NotFound { entity: "person" })
        })
    }
    fn team_members(&self) -> SdkFuture<'_, Vec<meerkat_provider::ProviderTeamMember>> {
        Box::pin(async move {
            let humans = self
                .storage
                .list_human_team_members(&self.workspace_id)
                .await
                .map_err(provider_storage_error)?;
            let agents = self
                .storage
                .list_agents(&self.workspace_id)
                .await
                .map_err(provider_storage_error)?;
            Ok(humans
                .into_iter()
                .map(person_identity)
                .map(meerkat_provider::ProviderTeamMember::Human)
                .chain(agents.into_iter().map(|agent| {
                    meerkat_provider::ProviderTeamMember::Agent { agent_id: agent.id }
                }))
                .collect())
        })
    }
    fn compare_and_set_person_state<'a>(
        &'a self,
        person_id: &'a PersonId,
        expected_revision: i64,
        state: serde_json::Value,
    ) -> SdkFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.storage
                .compare_and_set_person_state(
                    &self.workspace_id,
                    person_id,
                    expected_revision,
                    &state,
                    now_millis(),
                )
                .await
                .map(person_identity)
                .map_err(provider_storage_error)
        })
    }
    fn save_agent_file<'a>(&'a self, upload: ResourceUpload) -> SdkFuture<'a, ProviderAgentFile> {
        Box::pin(async move {
            let resource = self
                .resources
                .save(&self.workspace_id, upload)
                .await
                .map_err(|_| ProviderError::External {
                    operation: "save_agent_file",
                    retryable: false,
                })?;
            let resource_id =
                ResourceId::new(resource.resource_id).map_err(|_| ProviderError::InvalidInput {
                    field: "resource_id",
                })?;
            let metadata = self
                .storage
                .resource_metadata(&self.workspace_id, resource_id)
                .await
                .map_err(provider_storage_error)?
                .ok_or(ProviderError::NotFound { entity: "resource" })?;
            Ok(ProviderAgentFile {
                resource: resource.into(),
                path: metadata.storage_ref,
            })
        })
    }
}

fn person_identity(record: PersonRecord) -> ProviderPersonIdentity {
    ProviderPersonIdentity {
        person_id: record.person_id,
        external_id: record.external_id,
        team_role: record.team_role,
        provider_state: record.provider_identity_state,
        state_revision: record.provider_state_revision,
    }
}
impl ProviderLogger for WorkspaceServices {
    fn log(&self, level: meerkat_provider::LogLevel, message: &str) {
        let level = match level {
            meerkat_provider::LogLevel::Debug => log::Level::Debug,
            meerkat_provider::LogLevel::Info => log::Level::Info,
            meerkat_provider::LogLevel::Warn => log::Level::Warn,
            meerkat_provider::LogLevel::Error => log::Level::Error,
        };
        log::log!(level, "provider workspace={}: {message}", self.workspace_id);
    }
}
impl OperationProgress for WorkspaceServices {
    fn report<'a>(&'a self, update: OperationUpdate) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            log::info!(
                "provider operation workspace={} stage={} completed={} total={:?}",
                self.workspace_id,
                update.stage,
                update.completed,
                update.total
            );
            Ok(())
        })
    }
}
impl ResourceSaver for WorkspaceServices {
    fn save<'a>(&'a self, upload: ResourceUpload) -> SdkFuture<'a, meerkat_core::ResourceRef> {
        Box::pin(async move {
            self.resources
                .save(&self.workspace_id, upload)
                .await
                .map(Into::into)
                .map_err(|_| ProviderError::External {
                    operation: "save_resource",
                    retryable: true,
                })
        })
    }
}

fn provider_storage_error(_: StorageError) -> ProviderError {
    ProviderError::External {
        operation: "provider_storage",
        retryable: true,
    }
}
pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(1)
}
