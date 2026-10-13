use super::*;
use lankide_core::PersonId;
use lankide_protocol::{AgentRef, EnvironmentVariable, SessionAction};
use lankide_provider::{
    OperationProgress, OperationUpdate, ProviderAgentFile, ProviderFuture as SdkFuture,
    ProviderLogger, ProviderMessage, ProviderPersonIdentity, ProviderResource, ProviderStorage,
    ResourceSaver, ResourceUpload, SecretReader, SecretValue as ProviderSecret,
};
use lankide_storage::{AgentRecord, AttentionRecord, PersonRecord};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

const APP_NEW: u8 = 0;
const APP_READY: u8 = 1;
const APP_RUNNING: u8 = 2;
const APP_STOPPED: u8 = 3;

#[derive(Clone)]
pub struct RuntimeApplication {
    inner: Arc<ApplicationInner>,
}

struct ApplicationInner {
    runtime: StdMutex<Option<Arc<Runtime>>>,
    factories: ProviderFactoryRegistry,
    state: AtomicU8,
    sequence: AtomicU64,
    events: broadcast::Sender<lankide_protocol::ManagementEvent>,
    recovery: StdMutex<Option<RecoveryReport>>,
    computer_connected: AtomicBool,
    broker: ComputerBroker,
    resources: StdMutex<Option<Arc<dyn ResourceStore>>>,
    profiles: AgentProfileFiles,
    workspace_tasks: StdMutex<BTreeMap<WorkspaceId, JoinHandle<()>>>,
    execution_tasks: StdMutex<BTreeMap<String, JoinHandle<()>>>,
    shutting_down: AtomicBool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationReadiness {
    pub management_ready: bool,
    pub providers_started: bool,
    pub computer_connected: bool,
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
    WaitForComputer { execution_id: i64 },
    ReconcileProviderSend { request_key: String },
}

impl RuntimeApplication {
    pub(crate) fn stop_agent_execution(
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
            .storage()
            .active_execution_ids_for_agent(workspace_id, agent_id)?
        {
            self.inner
                .broker
                .cancel_execution(execution_id, "Agent was deleted")?;
            runtime
                .storage()
                .cancel_execution(execution_id, now_millis())?;
        }
        Ok(())
    }
    pub async fn open(
        config: RuntimeConfig,
        factories: ProviderFactoryRegistry,
    ) -> Result<Self, RuntimeError> {
        let profiles = AgentProfileFiles::new(&config.database.path);
        let resource_root = config
            .database
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("resources");
        let runtime = Arc::new(Runtime::new(config)?);
        let broker = ComputerBroker::new(runtime.storage_arc());
        let resources: Arc<dyn ResourceStore> = Arc::new(LocalResourceStore::new(
            runtime.storage_arc(),
            resource_root,
        ));
        let application = Self {
            inner: Arc::new(ApplicationInner {
                runtime: StdMutex::new(Some(runtime)),
                factories,
                state: AtomicU8::new(APP_NEW),
                sequence: AtomicU64::new(0),
                events: broadcast::channel(256).0,
                recovery: StdMutex::new(None),
                computer_connected: AtomicBool::new(false),
                broker,
                resources: StdMutex::new(Some(resources)),
                profiles,
                workspace_tasks: StdMutex::new(BTreeMap::new()),
                execution_tasks: StdMutex::new(BTreeMap::new()),
                shutting_down: AtomicBool::new(false),
            }),
        };
        application.load_persisted_workspaces().await?;
        application.inner.state.store(APP_READY, Ordering::Release);
        Ok(application)
    }

    pub async fn start(&self) -> Result<(), RuntimeError> {
        let runtime = self.runtime()?;
        self.inner.state.store(APP_RUNNING, Ordering::Release);
        for workspace in runtime.storage().list_workspace_records()? {
            if workspace.active {
                match runtime.resume_workspace(&workspace.id, now_millis()).await {
                    Ok(()) => {
                        runtime.storage().set_workspace_setup(
                            &workspace.id,
                            lankide_storage::WorkspaceSetupStatus::Ready,
                            None,
                            now_millis(),
                        )?;
                        self.start_workspace_loop(workspace.id)?;
                    }
                    Err(error) => {
                        eprintln!(
                            "[runtime] workspace={} Provider startup failed: {error}",
                            workspace.id
                        );
                        runtime.storage().set_workspace_active(
                            &workspace.id,
                            false,
                            now_millis(),
                        )?;
                        runtime.storage().set_workspace_setup(
                            &workspace.id,
                            lankide_storage::WorkspaceSetupStatus::Error,
                            Some("provider_setup_failed"),
                            now_millis(),
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn recover(&self, now: i64) -> Result<RecoveryReport, RuntimeError> {
        let runtime = self.runtime()?;
        let state = runtime.storage().recover_startup(now)?;
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
                .push(RecoveryAction::WaitForComputer { execution_id });
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
            computer_connected: self.inner.computer_connected.load(Ordering::Acquire),
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
        self.finish_shutdown()
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
        let runtime = self.runtime()?;
        runtime.shutdown().await?;
        self.inner
            .computer_connected
            .store(false, Ordering::Release);
        drop(runtime);
        Ok(())
    }

    pub fn finish_shutdown(&self) -> Result<(), RuntimeError> {
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
            .close()?;
        self.inner.state.store(APP_STOPPED, Ordering::Release);
        Ok(())
    }

    pub fn subscribe_management(&self) -> broadcast::Receiver<lankide_protocol::ManagementEvent> {
        self.inner.events.subscribe()
    }

    pub fn computer_broker(&self) -> ComputerBroker {
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

    pub(crate) fn set_computer_connected(&self, connected: bool) {
        self.inner
            .computer_connected
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
        for workspace in runtime.storage().list_workspace_records()? {
            if self
                .attach_workspace(&runtime, workspace.clone())
                .await
                .is_err()
            {
                runtime
                    .storage()
                    .set_workspace_active(&workspace.id, false, now_millis())?;
                runtime.storage().set_workspace_setup(
                    &workspace.id,
                    lankide_storage::WorkspaceSetupStatus::Error,
                    Some("provider_setup_failed"),
                    now_millis(),
                )?;
            }
        }
        Ok(())
    }

    async fn consume_workspace(&self, workspace_id: WorkspaceId) {
        let Ok(runtime) = self.runtime() else {
            return;
        };
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if self.inner.shutting_down.load(Ordering::Acquire)
                || !runtime.is_workspace_active(&workspace_id).unwrap_or(false)
            {
                break;
            }
            tokio::select! {
                event = runtime.next_provider_event(&workspace_id) => {
                    match event {
                        Ok(Some(event)) => {
                            match runtime.accept_provider_event(&workspace_id, event, now_millis()).await {
                                Ok(CoordinatedEvent::Message { steer_deliveries, .. }) => {
                                    for delivery in steer_deliveries {
                                        // Failed sends remain pending for this same Computer instance.
                                        if let Err(error) = self.inner.broker.deliver_steer(delivery).await {
                                            eprintln!("[runtime] workspace={workspace_id} steer delivery failed: {error}");
                                        }
                                    }
                                }
                                Ok(_) => {},
                                Err(error) => eprintln!("[runtime] workspace={workspace_id} event acceptance failed: {error}"),
                            }
                        }
                        Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                        Err(RuntimeError::WorkspacePaused(_)) => break,
                        Err(error) => {
                            eprintln!("[runtime] workspace={workspace_id} event receive failed: {error}");
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        },
                    }
                }
                _ = interval.tick() => {}
            }
            if let Err(error) = self.inner.broker.expire_leases(now_millis()) {
                eprintln!("[runtime] workspace={workspace_id} lease expiry failed: {error}");
            }
            let agents = match runtime.storage().list_agent_records(&workspace_id) {
                Ok(agents) => agents,
                Err(error) => {
                    eprintln!("[runtime] workspace={workspace_id} agent listing failed: {error}");
                    continue;
                }
            };
            for agent in agents {
                if !self.inner.broker.is_connected(&agent.computer_id) {
                    continue;
                }
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
                let executor = self.inner.broker.executor(agent.computer_id);
                let execution_runtime = runtime.clone();
                let execution_workspace = workspace_id.clone();
                let execution_agent = agent.agent_id.clone();
                let task = tokio::spawn(async move {
                    if let Err(error) = execution_runtime
                        .execute_ready(
                            &execution_workspace,
                            &execution_agent,
                            &executor,
                            now_millis(),
                        )
                        .await
                    {
                        eprintln!(
                            "[runtime] workspace={execution_workspace} agent={execution_agent} execution failed: {error}"
                        );
                    }
                    if let Err(error) = execution_runtime
                        .finish_paused_workspace(&execution_workspace)
                        .await
                    {
                        eprintln!(
                            "[runtime] workspace={execution_workspace} pause completion failed: {error}"
                        );
                    }
                });
                if let Ok(mut tasks) = self.inner.execution_tasks.lock() {
                    tasks.insert(task_key, task);
                } else {
                    task.abort();
                }
            }
        }
    }

    pub(crate) async fn attach_workspace(
        &self,
        runtime: &Arc<Runtime>,
        workspace: WorkspaceRecord,
    ) -> Result<(), RuntimeError> {
        let factory = self.inner.factories.factory(&workspace.provider_id)?;
        let services =
            workspace_services(runtime.storage_arc(), self.resource_store()?, &workspace);
        let factory_workspace = workspace.clone();
        runtime.register_provider_instance_factory(
            workspace.id.clone(),
            Arc::new(move || factory.create(&factory_workspace, services.clone())),
        )?;
        for agent in runtime.storage().list_agent_records(&workspace.id)? {
            let config = agent_runtime_config(&workspace, agent)?;
            runtime.configure_agent(&workspace.id, config).await?;
        }
        Ok(())
    }

    pub(crate) fn emit_management(
        &self,
        kind: lankide_protocol::ManagementEventKind,
        workspace: &WorkspaceId,
        agent: Option<&AgentId>,
        occurred_at: i64,
    ) {
        let sequence = self.inner.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.inner.events.send(lankide_protocol::ManagementEvent {
            sequence,
            occurred_at,
            kind: kind as i32,
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
            .map(str::to_owned),
        config_fingerprint: lowercase_sha256(
            serde_json::json!({
                "harness": agent.harness_id,
                "model": agent.model,
                "thinking": agent.thinking,
                "cli": agent.cli_path,
                "environment": agent.env,
            })
            .to_string()
            .as_bytes(),
        ),
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
        },
        computer_id: agent.computer_id,
        harness: HarnessConfig {
            harness_id: agent.harness_id,
            executable_path: agent.cli_path.unwrap_or_default(),
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
    storage: Arc<Storage>,
    resources: Arc<dyn ResourceStore>,
    workspace: &WorkspaceRecord,
) -> HostServices {
    workspace_services_with_secrets(storage, resources, workspace, BTreeMap::new())
}

pub(crate) fn workspace_services_with_secrets(
    storage: Arc<Storage>,
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
    storage: Arc<Storage>,
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
                .map_err(provider_storage_error)?
                .map(|value| ProviderSecret::new(value.expose_secret()))
                .ok_or(ProviderError::SecretAccessDenied)
        })
    }
}
impl lankide_provider::SecretWriter for WorkspaceServices {
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
                        lankide_storage::SecretValue::new(value.expose_secret()),
                        now_millis(),
                    )
                    .map_err(provider_storage_error);
            }
            self.storage
                .put_secret(
                    &self.workspace_id,
                    name,
                    lankide_storage::SecretValue::new(value.expose_secret()),
                    now_millis(),
                )
                .map(|_| ())
                .map_err(provider_storage_error)
        })
    }
}
impl ProviderStorage for WorkspaceServices {
    fn get<'a>(&'a self, key: &'a str) -> SdkFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            self.storage
                .provider_kv_get(&self.workspace_id, &self.namespace, key)
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
                .map_err(provider_storage_error)
        })
    }
    fn workspace_state<'a>(&'a self) -> SdkFuture<'a, serde_json::Value> {
        Box::pin(async move {
            self.storage
                .workspace_provider_state(&self.workspace_id)
                .map_err(provider_storage_error)
        })
    }
    fn put_workspace_state<'a>(&'a self, state: serde_json::Value) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            self.storage
                .put_workspace_provider_state(&self.workspace_id, &state)
                .map_err(provider_storage_error)
        })
    }
    fn agent_state<'a>(&'a self, agent_id: &'a AgentId) -> SdkFuture<'a, serde_json::Value> {
        Box::pin(async move {
            self.storage
                .agent_provider_state(&self.workspace_id, agent_id)
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
                .map_err(provider_storage_error)
        })
    }
    fn message<'a>(&'a self, message_id: MessageId) -> SdkFuture<'a, ProviderMessage> {
        Box::pin(async move {
            let message = self
                .storage
                .provider_message(&self.workspace_id, message_id.get())
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
        resource: &'a lankide_core::ResourceRef,
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
            let stored_resource = lankide_core::ResourceRef::from(stored.resource);
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
                .map(person_identity)
                .map_err(provider_storage_error)
        })
    }
    fn person<'a>(&'a self, person_id: &'a PersonId) -> SdkFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.storage
                .person(&self.workspace_id, person_id)
                .map_err(provider_storage_error)?
                .map(person_identity)
                .ok_or(ProviderError::NotFound { entity: "person" })
        })
    }
    fn team_members(&self) -> SdkFuture<'_, Vec<lankide_provider::ProviderTeamMember>> {
        Box::pin(async move {
            let humans = self
                .storage
                .list_human_team_members(&self.workspace_id)
                .map_err(provider_storage_error)?;
            let agents = self
                .storage
                .list_agents(&self.workspace_id)
                .map_err(provider_storage_error)?;
            Ok(humans
                .into_iter()
                .map(person_identity)
                .map(lankide_provider::ProviderTeamMember::Human)
                .chain(agents.into_iter().map(|agent| {
                    lankide_provider::ProviderTeamMember::Agent { agent_id: agent.id }
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
    fn log(&self, level: lankide_provider::LogLevel, message: &str) {
        eprintln!(
            "provider workspace={} level={level:?}: {message}",
            self.workspace_id
        );
    }
}
impl OperationProgress for WorkspaceServices {
    fn report<'a>(&'a self, update: OperationUpdate) -> SdkFuture<'a, ()> {
        Box::pin(async move {
            eprintln!(
                "provider operation workspace={} stage={} completed={} total={:?}",
                self.workspace_id, update.stage, update.completed, update.total
            );
            Ok(())
        })
    }
}
impl ResourceSaver for WorkspaceServices {
    fn save<'a>(&'a self, upload: ResourceUpload) -> SdkFuture<'a, lankide_core::ResourceRef> {
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
