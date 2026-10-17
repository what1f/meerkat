#![forbid(unsafe_code)]

//! Test-only business scenarios through the real Runtime and Machine daemons.

pub mod net_proxy;
mod provider;
mod resources;
mod script;
pub use provider::{Incoming, Outbound, OutboundKind};
pub use script::{Script, scripted_failure};

use lankide_client::{AgentDraft, ManagementClient, RuntimeManagementClient, WorkspaceDraft};
use lankide_core::{AgentId, ChannelId};
use lankide_harness::{HarnessConfig, ModelRef};
use lankide_machine::DaemonConfig;
use lankide_machine_core::RegisteredHarness;
use lankide_provider::{
    EventSender, EventSenderType, MessageEvent, Provider, ProviderEvent, ProviderEventKind,
    ProviderHost,
};
use lankide_runtime::ProviderFactoryRegistry;
use lankided::{RuntimePaths, ShutdownReason, run_daemon};
use provider::{ImState, ScriptedProvider};
use rusqlite::{Connection, OpenFlags, params};
use script::ScriptedHarness;
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{Instant, timeout},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug)]
pub struct RigConfig {
    pub harness_executable: PathBuf,
    pub timeout: Duration,
    pub pairing_validity: Duration,
    pub auto_start_machine: bool,
    pub local_discovery: bool,
    pub machine_ws_only: bool,
    pub execution_token_lifetime: Duration,
    pub attempt_lease: Duration,
    pub network_proxy: bool,
    pub machine_process: Option<PathBuf>,
    pub link: lankide_protocol::link::LinkSettings,
    pub reconnect: lankide_machine_core::ReconnectPolicy,
    pub execution_liveness: lankide_machine_core::ExecutionLiveness,
    pub harness_capabilities: Vec<String>,
    pub retry: lankide_runtime::RetryConfig,
}
impl Default for RigConfig {
    fn default() -> Self {
        let executable = std::env::current_exe().expect("test executable");
        Self {
            harness_executable: executable
                .parent()
                .expect("test directory")
                .parent()
                .expect("target directory")
                .join("system-harness"),
            timeout: Duration::from_secs(15),
            pairing_validity: Duration::from_secs(600),
            auto_start_machine: true,
            local_discovery: false,
            machine_ws_only: false,
            execution_token_lifetime: Duration::from_secs(3600),
            attempt_lease: Duration::from_secs(15),
            network_proxy: false,
            machine_process: None,
            link: Default::default(),
            reconnect: Default::default(),
            execution_liveness: Default::default(),
            harness_capabilities: ["system_prompt", "streaming", "resume", "cancel", "tools"]
                .map(str::to_owned)
                .to_vec(),
            retry: Default::default(),
        }
    }
}

struct MachineThread {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}
impl MachineThread {
    fn start(
        config: DaemonConfig,
        executable: PathBuf,
        journal: PathBuf,
        capabilities: Vec<String>,
    ) -> Self {
        let (stop, stopped) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            // Dropping this executor is a process-crash boundary: all Machine
            // tasks and their sessions stop together, without an OS-wide signal.
            let runtime = tokio::runtime::Runtime::new()?;
            let result = runtime.block_on(async move {
                let registration = RegisteredHarness::new(ScriptedHarness::new(journal), HarnessConfig {
                    harness_id: "pi".into(), model: ModelRef::new("scripted", "scripted"), thinking_level: "low".into(),
                    cli_path: executable, environment: BTreeMap::from([("LANKIDE_SYSTEM_CAPABILITIES".into(), serde_json::to_string(&capabilities)?)]), config_fingerprint: "system-rig".into(),
                });
                tokio::select! {
                    result = lankide_machine::run_with_harnesses(config, vec![registration]) => result.map_err(Into::into),
                    _ = stopped => Ok(()),
                }
            });
            runtime.shutdown_timeout(Duration::from_secs(1));
            result
        });
        Self {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
    fn stop(&mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "Machine thread panicked")??;
        }
        Ok(())
    }
}
impl Drop for MachineThread {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// Drop a dedicated executor to model a Runtime process crash, including all
// spawned daemon tasks. The Machine executor and native Harness stay alive.
fn spawn_runtime(
    paths: RuntimePaths,
    factories: ProviderFactoryRegistry,
    stopped: oneshot::Receiver<()>,
) -> (
    JoinHandle<std::result::Result<(), lankided::AppError>>,
    oneshot::Sender<()>,
) {
    let (kill, killed) = oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        let executor = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Runtime executor");
        executor.block_on(async move {
            tokio::select! {
                result = run_daemon(paths, factories, async move {
                    let _ = stopped.await;
                    Ok(ShutdownReason::Internal)
                }) => result,
                _ = killed => Ok(()),
            }
        })
    });
    (task, kill)
}

/// A second independently paired daemon, with its own durable home.
pub struct PairedMachine {
    // Stop the daemon before removing its home (fields drop in declaration order).
    _daemon: MachineThread,
    pub id: String,
    pub home: TempDir,
}

pub struct SystemRig {
    // Homes stay alive until both daemons release their tasks.
    machine: Option<MachineThread>,
    child_machine: Option<std::process::Child>,
    pub proxy: Option<net_proxy::NetProxy>,
    runtime: Option<JoinHandle<std::result::Result<(), lankided::AppError>>>,
    shutdown: Option<oneshot::Sender<()>>,
    runtime_kill: Option<oneshot::Sender<()>>,
    home: TempDir,
    machine_home: TempDir,
    paths: RuntimePaths,
    config: RigConfig,
    client: RuntimeManagementClient,
    im: Arc<ImState>,
    sequence: AtomicU64,
}
impl SystemRig {
    pub async fn start(config: RigConfig) -> Result<Self> {
        let home = tempfile::Builder::new()
            .prefix("system-")
            .tempdir_in("/tmp")?;
        let machine_home = tempfile::Builder::new()
            .prefix("system-")
            .tempdir_in("/tmp")?;
        let paths = RuntimePaths::for_home(home.path())?;
        std::fs::write(
            home.path().join("retry-policy.json"),
            serde_json::to_vec(&config.retry)?,
        )?;
        std::fs::write(
            home.path().join("runtime.json"),
            serde_json::to_vec(
                &json!({"remote_access_port": 0, "pairing_validity_ms":config.pairing_validity.as_millis(), "execution_token_lifetime_ms": config.execution_token_lifetime.as_millis(), "attempt_lease_ms": config.attempt_lease.as_millis(), "machine_link": config.link}),
            )?,
        )?;
        let im = Arc::new(ImState::default());
        let factories = factories(im.clone())?;
        let (shutdown, stopped) = oneshot::channel();
        let daemon_paths = paths.clone();
        let (runtime, runtime_kill) = spawn_runtime(daemon_paths, factories, stopped);
        let mut rig = Self {
            machine: None,
            child_machine: None,
            proxy: None,
            runtime: Some(runtime),
            runtime_kill: Some(runtime_kill),
            shutdown: Some(shutdown),
            client: RuntimeManagementClient::connect_local_lazy(&paths.run),
            home,
            machine_home,
            paths,
            config,
            im,
            sequence: AtomicU64::new(0),
        };
        timeout(rig.config.timeout, async {
            loop {
                if rig.client.list_workspaces().await.is_ok() {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                rig.check_daemons()?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Runtime management startup timed out")??;
        if rig.config.network_proxy {
            let (url, _) = rig.local_pairing_endpoint()?;
            let code = rig.client.create_pairing_code().await?;
            lankide_machine::pair(
                &lankide_machine::MachineDaemonPaths::for_home(rig.machine_home.path())?,
                &url,
                &code.code,
            )
            .await?;
            rig.proxy =
                Some(net_proxy::NetProxy::start(url.trim_start_matches("ws://").parse()?).await?);
        }
        if rig.config.auto_start_machine {
            rig.start_machine()?;
            rig.wait_for_machine().await?;
        }
        Ok(rig)
    }
    fn check_daemons(&self) -> Result<()> {
        if self.runtime.as_ref().is_some_and(|task| task.is_finished()) {
            return Err("Runtime daemon stopped unexpectedly".into());
        }
        if self
            .machine
            .as_ref()
            .and_then(|machine| machine.thread.as_ref())
            .is_some_and(|thread| thread.is_finished())
        {
            return Err("Machine daemon stopped unexpectedly".into());
        }
        Ok(())
    }
    fn start_machine(&mut self) -> Result<()> {
        std::fs::write(
            self.machine_home.path().join("machine.json"),
            serde_json::to_vec(
                &json!({"execution_liveness": self.config.execution_liveness, "reconnect": self.config.reconnect}),
            )?,
        )?;
        let mut config = DaemonConfig::for_home(self.machine_home.path())?;
        config.paths.prepare()?;
        let (url, secret) = lankide_protocol::link::discover(&self.paths.run)?;
        let secret_path = self.machine_home.path().join("run/machine-link.secret");
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&secret_path)?;
            file.write_all(secret.as_bytes())?;
        }
        if !self.config.machine_ws_only {
            config.paths.runtime_run = self.paths.run.clone();
        }
        config.runtime_url = if self.config.local_discovery {
            None
        } else {
            Some(self.proxy.as_ref().map_or(url, net_proxy::NetProxy::url))
        };
        config.link_secret_file = Some(secret_path);
        if let Some(binary) = &self.config.machine_process {
            self.child_machine = Some(
                std::process::Command::new(binary)
                    .arg(self.machine_home.path())
                    .arg(config.runtime_url.as_ref().expect("URL"))
                    .arg(config.link_secret_file.as_ref().expect("secret"))
                    .arg(&self.paths.run)
                    .arg(&self.config.harness_executable)
                    .arg(serde_json::to_string(&self.config.harness_capabilities)?)
                    .spawn()?,
            );
            return Ok(());
        }
        self.machine = Some(MachineThread::start(
            config,
            self.config.harness_executable.clone(),
            self.machine_home.path().join("harness-pids.tsv"),
            self.config.harness_capabilities.clone(),
        ));
        Ok(())
    }
    async fn wait_for_machine(&self) -> Result<()> {
        timeout(self.config.timeout, async {
            loop {
                self.check_daemons()?;
                if let Ok(machines) = self.client.get_machine_catalog(None).await {
                    if machines
                        .iter()
                        .any(|machine| machine.registered && machine.available)
                    {
                        return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Machine startup timed out")?
    }
    pub fn kill_machine_process(&mut self) -> Result<()> {
        if let Some(mut child) = self.child_machine.take() {
            child.kill()?;
            child.wait()?;
        }
        Ok(())
    }
    pub fn restart_machine_process(&mut self) -> Result<()> {
        self.start_machine()
    }
    fn machine_state_path(&self) -> PathBuf {
        self.machine_home
            .path()
            .join("workspaces/.machine/state.json")
    }
    pub fn machine_state(&self) -> Result<serde_json::Value> {
        Ok(serde_json::from_slice(&std::fs::read(
            self.machine_state_path(),
        )?)?)
    }
    /// Inspect durable queue lengths without allocating every payload byte as a JSON value.
    pub fn pending_event_counts(&self) -> Result<Vec<usize>> {
        #[derive(serde::Deserialize)]
        struct Backlog {
            pending_events: std::collections::BTreeMap<
                String,
                std::collections::BTreeMap<String, serde::de::IgnoredAny>,
            >,
        }
        let state: Backlog = serde_json::from_slice(&std::fs::read(self.machine_state_path())?)?;
        Ok(state
            .pending_events
            .values()
            .map(|events| events.len())
            .collect())
    }
    pub async fn machine_online(&self) -> Result<bool> {
        Ok(self
            .client
            .get_machine_catalog(None)
            .await?
            .iter()
            .any(|c| c.registered && c.available))
    }
    pub fn machine_endpoint(&self) -> Result<(String, String)> {
        let (url, _) = lankide_protocol::link::discover(&self.paths.run)?;
        let credentials = lankide_machine::read_credentials(self.machine_home.path())?
            .ok_or("Machine is unpaired")?;
        Ok((url, credentials.device_credential))
    }
    pub fn local_pairing_endpoint(&self) -> Result<(String, String)> {
        Ok(lankide_protocol::link::discover(&self.paths.run)?)
    }
    pub fn management(&self) -> &RuntimeManagementClient {
        &self.client
    }
    pub fn machine_home(&self) -> &std::path::Path {
        self.machine_home.path()
    }
    pub fn runtime_home(&self) -> &std::path::Path {
        self.home.path()
    }
    pub async fn add_machine(&self) -> Result<PairedMachine> {
        let home = tempfile::tempdir()?;
        let code = self.client.create_pairing_code().await?;
        let (url, _) = self.local_pairing_endpoint()?;
        let credentials = lankide_machine::pair(
            &lankide_machine::MachineDaemonPaths::for_home(home.path())?,
            &url,
            &code.code,
        )
        .await?;
        let mut config = DaemonConfig::for_home(home.path())?;
        config.runtime_url = Some(url);
        config.paths.runtime_run = self.paths.run.clone();
        let daemon = MachineThread::start(
            config,
            self.config.harness_executable.clone(),
            home.path().join("harness-pids.tsv"),
            self.config.harness_capabilities.clone(),
        );
        timeout(self.config.timeout, async {
            loop {
                if self
                    .client
                    .get_machine_catalog(None)
                    .await?
                    .iter()
                    .any(|c| c.id == credentials.machine_id && c.available)
                {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        Ok(PairedMachine {
            id: credentials.machine_id,
            home,
            _daemon: daemon,
        })
    }

    pub fn start_paired_machine(&mut self) -> Result<()> {
        self.start_machine()
    }
    pub async fn wait_machine_online(&self) -> Result<()> {
        self.wait_for_machine().await
    }

    pub fn runtime_run(&self) -> &std::path::Path {
        &self.paths.run
    }
    pub fn homes_are_separate(&self) -> bool {
        self.home.path() != self.machine_home.path()
    }
    pub fn machine_has_runtime_socket(&self) -> bool {
        self.machine_home.path().join("run/runtime.sock").exists()
    }

    pub fn outbound(&self) -> Vec<Outbound> {
        self.im.outbound.lock().expect("outbound").clone()
    }
    pub fn machine_failure_log(&self) -> Result<String> {
        Ok(std::fs::read_to_string(
            self.machine_home
                .path()
                .join("workspaces/attempt-failures.jsonl"),
        )?)
    }
    pub async fn execution_details(
        &self,
        execution_id: i64,
    ) -> Result<lankide_client::ExecutionDetails> {
        Ok(self.client.get_execution_details(execution_id).await?)
    }
    pub async fn cancel_execution(&self, execution_id: i64) -> Result<()> {
        self.client.cancel_execution(execution_id).await?;
        Ok(())
    }

    pub fn harness_pids(&self) -> Result<Vec<u32>> {
        Ok(
            std::fs::read_to_string(self.machine_home.path().join("harness-pids.tsv"))?
                .lines()
                .filter_map(|line| line.strip_prefix("spawned\t")?.parse().ok())
                .collect(),
        )
    }

    pub async fn restart_runtime(&mut self) -> Result<()> {
        self.stop_runtime().await?;
        self.start_runtime().await
    }

    /// Stop the Runtime gracefully and leave it down until `start_runtime`.
    pub async fn stop_runtime(&mut self) -> Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(runtime) = self.runtime.take() {
            timeout(self.config.timeout, runtime).await???;
        }
        Ok(())
    }

    pub async fn start_runtime(&mut self) -> Result<()> {
        let factories = factories(self.im.clone())?;
        let paths = self.paths.clone();
        let (shutdown, stopped) = oneshot::channel();
        self.shutdown = Some(shutdown);
        let (runtime, kill) = spawn_runtime(paths, factories, stopped);
        self.runtime = Some(runtime);
        self.runtime_kill = Some(kill);
        self.client = RuntimeManagementClient::connect_local_lazy(&self.paths.run);
        // The daemon's graceful shutdown frame ends its managed Machine peer.
        // Reopen the same Machine home, as a service supervisor does.
        if let Some(mut machine) = self.machine.take() {
            machine.stop()?;
        }
        timeout(self.config.timeout, async {
            loop {
                if self.client.list_workspaces().await.is_ok() {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                self.check_daemons()?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Runtime restart timed out")??;
        self.start_machine()?;
        self.wait_for_machine().await
    }
    /// Hold the scripted IM platform's prompt response during preparation.
    pub fn hold_preparation(&self) -> PathBuf {
        let gate = self.home.path().join("release-preparation");
        *self.im.preparation_gate.lock().expect("preparation gate") = Some(gate.clone());
        gate
    }

    pub async fn crash_and_restart_runtime(&mut self) -> Result<()> {
        if let Some(kill) = self.runtime_kill.take() {
            let _ = kill.send(());
        }
        if let Some(runtime) = self.runtime.take() {
            timeout(self.config.timeout, runtime).await???;
        }
        self.shutdown.take();
        let (shutdown, stopped) = oneshot::channel();
        let (runtime, kill) =
            spawn_runtime(self.paths.clone(), factories(self.im.clone())?, stopped);
        self.runtime = Some(runtime);
        self.runtime_kill = Some(kill);
        self.shutdown = Some(shutdown);
        self.client = RuntimeManagementClient::connect_local_lazy(&self.paths.run);
        timeout(self.config.timeout, async {
            loop {
                self.check_daemons()?;
                if self.client.list_workspaces().await.is_ok() {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Runtime crash restart timed out")??;
        self.wait_for_machine().await
    }

    pub fn stop_machine(&mut self) -> Result<()> {
        if let Some(mut machine) = self.machine.take() {
            machine.stop()?;
        }
        self.kill_machine_process()
    }
    pub async fn await_terminal_machine(&mut self) -> Result<String> {
        timeout(self.config.timeout, async {
            loop {
                if self
                    .machine
                    .as_ref()
                    .and_then(|c| c.thread.as_ref())
                    .is_some_and(|t| t.is_finished())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Machine terminal stop timed out")?;
        let mut machine = self.machine.take().ok_or("missing Machine")?;
        let thread = machine.thread.take().ok_or("missing Machine thread")?;
        let result = thread.join().map_err(|_| "Machine thread panicked")?;
        Ok(result
            .expect_err("revoked Machine must exit with an error")
            .to_string())
    }
    pub async fn restart_machine(&mut self) -> Result<()> {
        if let Some(mut machine) = self.machine.take() {
            machine.stop()?;
        }
        self.start_machine()?;
        self.wait_for_machine().await
    }
    pub fn existing_workspace(&self, name: &str) -> Workspace<'_> {
        Workspace {
            rig: self,
            id: name.into(),
        }
    }
    pub async fn workspace(&self, name: &str) -> Result<Workspace<'_>> {
        self.client
            .create_workspace(WorkspaceDraft {
                id: name.into(),
                name: name.into(),
                rules: String::new(),
                provider_id: "system_im".into(),
                provider_config: json!({}),
                provider_secrets: json!({}),
            })
            .await?;
        timeout(self.config.timeout, async {
            loop {
                if self.im.sinks.lock().expect("sinks").contains_key(name) {
                    break;
                }
                self.check_daemons()?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await
        .map_err(|_| "IM startup timed out")??;
        Ok(Workspace {
            rig: self,
            id: name.into(),
        })
    }
    fn database(&self) -> Result<Connection> {
        // Observation only; Runtime remains the sole writer and schema owner.
        let connection =
            Connection::open_with_flags(&self.paths.database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(1))?;
        Ok(connection)
    }
    /// Empty means no pending/claimed inbox items and no nonterminal execution.
    /// Inbox acceptance is acknowledged before scenario message steps return.
    pub async fn settle(&self) -> Result<()> {
        let deadline = Instant::now() + self.config.timeout;
        loop {
            self.check_daemons()?;
            let mut database = self.database()?;
            let transaction = database.transaction()?;
            let inbox: i64 = transaction.query_row(
                "SELECT count(*) FROM agent_inbox_items WHERE state IN ('pending','claimed','delivered')",
                [],
                |row| row.get(0),
            )?;
            let running: i64 = transaction.query_row("SELECT count(*) FROM executions WHERE state IN ('queued','assigning','running','recovering')", [], |row| row.get(0))?;
            drop(transaction);
            drop(database);
            let pending_notices: i64 = self.database()?.query_row("SELECT (SELECT count(*) FROM failure_notices WHERE sent_at IS NULL)+(SELECT count(*) FROM executions WHERE state IN ('failed','cancelled') AND retry_policy_applied=0 AND (state='cancelled' OR EXISTS(SELECT 1 FROM execution_attempts a WHERE a.execution_id=executions.execution_id AND a.failure_json IS NOT NULL)))",[],|r|r.get(0))?;
            if inbox == 0 && running == 0 && pending_notices == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "settle timed out: {inbox} inbox items, {running} active executions, {pending_notices} pending failure actions"
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    pub async fn close(mut self) -> Result<()> {
        self.kill_machine_process()?;
        if let Some(mut machine) = self.machine.take() {
            machine.stop()?;
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut runtime) = self.runtime.take() {
            match timeout(Duration::from_secs(10), &mut runtime).await {
                Ok(result) => result??,
                Err(_) => {
                    runtime.abort();
                    let _ = runtime.await;
                    return Err("Runtime shutdown timed out".into());
                }
            }
        }
        Ok(())
    }
}
impl Drop for SystemRig {
    fn drop(&mut self) {
        let _ = self.kill_machine_process();
        self.machine.take();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

pub struct Workspace<'a> {
    rig: &'a SystemRig,
    pub id: String,
}
impl<'a> Workspace<'a> {
    pub fn existing_agent(&self, name: &str) -> Agent<'a> {
        Agent {
            rig: self.rig,
            workspace: self.id.clone(),
            id: name.into(),
            account: format!("bot-{name}"),
        }
    }
    pub fn agent(&self, name: &str) -> AgentBuilder<'a> {
        AgentBuilder {
            machine_id: String::new(),
            rig: self.rig,
            workspace: self.id.clone(),
            name: name.into(),
            script: Script::reply("hi"),
        }
    }
    pub async fn group_message(&self, chat: &str, person: &str, text: &str) -> Result<()> {
        self.message("group_chat", chat, person, text, None, &[])
            .await
    }
    pub async fn mention(
        &self,
        agent: &Agent<'_>,
        chat: &str,
        person: &str,
        text: &str,
    ) -> Result<()> {
        self.message(
            "group_chat",
            chat,
            person,
            text,
            None,
            &[AgentId::new(&agent.id)?],
        )
        .await
    }
    pub async fn agent_message(
        &self,
        sender: &Agent<'_>,
        recipient: &Agent<'_>,
        chat: &str,
        text: &str,
    ) -> Result<()> {
        let sink = self
            .rig
            .im
            .sinks
            .lock()
            .expect("sinks")
            .get(&self.id)
            .expect("workspace IM")
            .clone();
        let id = self.rig.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        sink.emit(ProviderEvent {
            receiving_agent_id: None,
            channel_id: ChannelId::new("group_chat")?,
            provider_chat_id: chat.into(),
            sender: EventSender {
                id: sender.id.clone(),
                kind: EventSenderType::Agent,
                name: "Agent".into(),
            },
            kind: ProviderEventKind::Message(MessageEvent {
                provider_message_id: format!("incoming-{id}"),
                content: text.into(),
                reply_to_provider_message_id: None,
                mentioned_agent_ids: vec![AgentId::new(&recipient.id)?],
                attachments: Vec::new(),
            }),
            provider_data: json!({"event_id":format!("event-{id}")}),
        })
        .await?;
        Ok(())
    }
    pub async fn direct_message(&self, agent: &Agent<'_>, person: &str, text: &str) -> Result<()> {
        // Telegram uses the person ID as the private chat ID for every bot.
        self.message(
            "private_chat",
            person,
            person,
            text,
            Some(&agent.account),
            &[],
        )
        .await
    }
    pub async fn unaddressed_direct_message(&self, person: &str, text: &str) -> Result<()> {
        self.message("private_chat", person, person, text, None, &[])
            .await
    }
    pub async fn direct_reaction(
        &self,
        agent: &Agent<'_>,
        person: &str,
        target: &str,
        event_id: &str,
    ) -> Result<()> {
        let sink = self
            .rig
            .im
            .sinks
            .lock()
            .expect("sinks")
            .get(&self.id)
            .expect("workspace IM")
            .clone();
        let receiver = self
            .rig
            .im
            .accounts
            .lock()
            .expect("accounts")
            .iter()
            .find(|((workspace, _), account)| {
                workspace == &self.id && account.as_str() == agent.account.as_str()
            })
            .and_then(|((_, id), _)| AgentId::new(id.clone()).ok());
        let sender = EventSender {
            id: person.into(),
            kind: EventSenderType::Human,
            name: person.into(),
        };
        sink.emit(ProviderEvent {
            receiving_agent_id: receiver,
            channel_id: ChannelId::new("private_chat")?,
            provider_chat_id: person.into(),
            sender: sender.clone(),
            kind: ProviderEventKind::Reaction(lankide_provider::ReactionEvent {
                target_provider_message_id: target.into(),
                sender,
                reaction: "thumbs_up".into(),
                action: lankide_provider::ReactionAction::Add,
            }),
            provider_data: json!({"event_id":event_id}),
        })
        .await?;
        Ok(())
    }
    pub async fn group_reaction(
        &self,
        chat: &str,
        sender_id: &str,
        bot: bool,
        target: &str,
        event_id: &str,
    ) -> Result<()> {
        let sink = self
            .rig
            .im
            .sinks
            .lock()
            .expect("sinks")
            .get(&self.id)
            .expect("workspace IM")
            .clone();
        let sender = EventSender {
            id: sender_id.into(),
            name: if bot {
                "Agent".into()
            } else {
                sender_id.into()
            },
            kind: if bot {
                EventSenderType::Agent
            } else {
                EventSenderType::Human
            },
        };
        sink.emit(ProviderEvent {
            receiving_agent_id: None,
            channel_id: ChannelId::new("group_chat")?,
            provider_chat_id: chat.into(),
            sender: sender.clone(),
            kind: ProviderEventKind::Reaction(lankide_provider::ReactionEvent {
                target_provider_message_id: target.into(),
                sender,
                reaction: "thumbs_up".into(),
                action: lankide_provider::ReactionAction::Add,
            }),
            provider_data: json!({"event_id":event_id}),
        })
        .await?;
        Ok(())
    }
    async fn message(
        &self,
        channel: &str,
        chat: &str,
        person: &str,
        text: &str,
        account: Option<&str>,
        mentions: &[AgentId],
    ) -> Result<()> {
        let id = self.rig.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let sink = self
            .rig
            .im
            .sinks
            .lock()
            .expect("sinks")
            .get(&self.id)
            .expect("workspace IM")
            .clone();
        let order = self.rig.im.order.fetch_add(1, Ordering::SeqCst);
        let receiving_agent_id = account.and_then(|account| {
            self.rig
                .im
                .accounts
                .lock()
                .expect("accounts")
                .iter()
                .find(|((workspace, _), registered)| {
                    workspace == &self.id && registered.as_str() == account
                })
                .and_then(|((_, agent), _)| AgentId::new(agent.clone()).ok())
        });
        let provider_message_id = if let Some(account) = account {
            let received = self
                .rig
                .im
                .incoming
                .lock()
                .expect("incoming")
                .iter()
                .filter(|message| {
                    message.workspace == self.id
                        && message.chat == chat
                        && message.account.as_deref() == Some(account)
                })
                .count();
            format!("incoming-{}", received + 1)
        } else {
            format!("incoming-{id}")
        };
        sink.emit(ProviderEvent { receiving_agent_id, channel_id: ChannelId::new(channel)?, provider_chat_id: chat.into(),
            sender: EventSender { id: person.into(), kind: EventSenderType::Human, name: person.into() },
            kind: ProviderEventKind::Message(MessageEvent { provider_message_id, content: text.into(), reply_to_provider_message_id: None, mentioned_agent_ids: mentions.to_vec(), attachments: Vec::new() }),
            provider_data: json!({"event_id":format!("event-{id}"), "chat_name":chat, "receiving_account":account}) }).await?;
        self.rig
            .im
            .incoming
            .lock()
            .expect("incoming")
            .push(Incoming {
                order,
                workspace: self.id.clone(),
                account: account.map(str::to_owned),
                chat: chat.into(),
                person: person.into(),
                content: text.into(),
            });
        Ok(())
    }
    pub fn outbound(&self, agent: &Agent<'_>, chat: &str) -> Vec<Outbound> {
        self.rig
            .im
            .outbound
            .lock()
            .expect("outbound")
            .iter()
            .filter(|send| {
                send.workspace == self.id && send.account == agent.account && send.chat == chat
            })
            .cloned()
            .collect()
    }
    pub fn transcript(&self, agent: &Agent<'_>, chat: &str) -> Vec<String> {
        let mut transcript: Vec<_> = self
            .rig
            .im
            .incoming
            .lock()
            .expect("incoming")
            .iter()
            .filter(|message| {
                message.workspace == self.id
                    && message.chat == chat
                    && message
                        .account
                        .as_ref()
                        .is_none_or(|account| account == &agent.account)
            })
            .map(|message| (message.order, message.content.clone()))
            .collect();
        transcript.extend(
            self.outbound(agent, chat)
                .into_iter()
                .filter(|send| matches!(send.kind, OutboundKind::Send | OutboundKind::Reply))
                .map(|send| (send.order, send.content)),
        );
        transcript.sort_by_key(|(order, _)| *order);
        transcript.into_iter().map(|(_, text)| text).collect()
    }
    pub fn dm_transcript(&self, agent: &Agent<'_>, person: &str) -> Vec<String> {
        self.transcript(agent, person)
    }
}

pub struct AgentBuilder<'a> {
    machine_id: String,
    rig: &'a SystemRig,
    workspace: String,
    name: String,
    script: Script,
}
impl<'a> AgentBuilder<'a> {
    pub fn machine(mut self, machine_id: &str) -> Self {
        self.machine_id = machine_id.into();
        self
    }

    pub fn harness(mut self, script: Script) -> Self {
        self.script = script;
        self
    }
    pub async fn create(self) -> Result<Agent<'a>> {
        let account = format!("bot-{}", self.name);
        self.rig
            .client
            .create_agent(
                &self.workspace,
                AgentDraft {
                    machine_id: self.machine_id,
                    id: self.name.clone(),
                    name: self.name.clone(),
                    model: "scripted".into(),
                    cli_path: self.rig.config.harness_executable.display().to_string(),
                    environment_variables: vec![
                        (
                            "LANKIDE_SYSTEM_INPUTS".into(),
                            self.rig
                                .machine_home
                                .path()
                                .join(format!("inputs-{}-{}.jsonl", self.workspace, self.name))
                                .display()
                                .to_string(),
                        ),
                        (
                            "LANKIDE_SYSTEM_CONTROLS".into(),
                            self.rig
                                .machine_home
                                .path()
                                .join(format!("controls-{}-{}.jsonl", self.workspace, self.name))
                                .display()
                                .to_string(),
                        ),
                        (
                            "LANKIDE_SYSTEM_STEERS".into(),
                            self.rig
                                .machine_home
                                .path()
                                .join(format!("steers-{}-{}.jsonl", self.workspace, self.name))
                                .display()
                                .to_string(),
                        ),
                        (
                            "LANKIDE_SYSTEM_SCRIPT".into(),
                            serde_json::to_string(&self.script)?,
                        ),
                        (
                            "LANKIDE_SYSTEM_RELEASE".into(),
                            self.rig
                                .machine_home
                                .path()
                                .join(format!("release-{}-{}", self.workspace, self.name))
                                .display()
                                .to_string(),
                        ),
                    ],
                    provider_handle: account.clone(),
                    provider_agent_config: json!({"account":account}),
                    ..Default::default()
                },
            )
            .await?;
        Ok(Agent {
            rig: self.rig,
            workspace: self.workspace,
            id: self.name,
            account,
        })
    }
}
pub struct Agent<'a> {
    rig: &'a SystemRig,
    workspace: String,
    pub id: String,
    pub account: String,
}
#[derive(Clone, Debug)]
pub struct Execution {
    pub id: i64,
    pub state: String,
    pub output: Option<String>,
    pub error_code: Option<String>,
    pub attempt: i64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub trigger_messages: Vec<String>,
}
impl Agent<'_> {
    pub async fn child_pid(&self) -> Result<u32> {
        let path = lankide_machine_core::agent_directory(
            &self.rig.machine_home.path().join("workspaces"),
            &self.workspace,
            &self.id,
        )?
        .join("system-child.pid");
        timeout(self.rig.config.timeout, async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&path)
                    && let Ok(pid) = pid.parse()
                {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(pid);
                }
                self.rig.check_daemons()?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "child did not start")?
    }

    pub async fn progress_sequence(&self) -> Result<i64> {
        Ok(self.rig.database()?.query_row("SELECT coalesce(max(v.last_event_sequence),0) FROM execution_attempts v JOIN executions e USING(execution_id) WHERE e.workspace_id=?1 AND e.agent_id=?2", params![self.workspace, self.id], |row| row.get(0))?)
    }

    pub async fn view(&self) -> Result<lankide_client::Agent> {
        Ok(self.rig.client.get_agent(&self.workspace, &self.id).await?)
    }
    pub async fn resume(&self) -> Result<()> {
        self.rig
            .client
            .resume_agent(&self.workspace, &self.id)
            .await?;
        Ok(())
    }
    pub async fn change_script(&self, script: Script, model: &str) -> Result<()> {
        let mut agent = self.view().await?;
        agent.model = model.into();
        for (key, value) in &mut agent.environment_variables {
            if key == "LANKIDE_SYSTEM_SCRIPT" {
                *value = serde_json::to_string(&script)?;
            }
        }
        self.rig.client.update_agent(agent, None).await?;
        Ok(())
    }
    pub async fn wait_for(
        &self,
        mut condition: impl FnMut(&[Execution], &lankide_client::Agent) -> bool,
    ) -> Result<()> {
        timeout(self.rig.config.timeout, async {
            loop {
                self.rig.check_daemons()?;
                if condition(&self.executions().await?, &self.view().await?) {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Agent state wait timed out")?
    }
    pub async fn group_attention(&self, mode: lankide_protocol::AttentionMode) -> Result<()> {
        let client = lankide_client::http::HttpManagementTransport::local(
            &self.rig.paths.run,
            lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES,
        );
        client
            .update_agent(lankide_protocol::UpdateAgentRequest {
                request_id: format!("attention-{}-{}", self.workspace, self.id),
                agent: Some(lankide_protocol::AgentRef {
                    workspace_id: self.workspace.clone(),
                    agent_id: self.id.clone(),
                }),
                attention: Some(lankide_protocol::AgentAttention {
                    channels: vec![lankide_protocol::ChannelAttention {
                        channel_id: "group_chat".into(),
                        mode,
                        interval_minutes: 0,
                    }],
                }),
                ..Default::default()
            })
            .await?;
        Ok(())
    }
    pub fn inbox_states(&self) -> Result<Vec<String>> {
        let c = self.rig.database()?;
        let mut q=c.prepare("SELECT state FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 ORDER BY inbox_item_id")?;
        Ok(q.query_map(params![self.workspace, self.id], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn inbox_count(&self) -> Result<i64> {
        Ok(self.rig.database()?.query_row(
            "SELECT count(*) FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2",
            params![self.workspace, self.id],
            |row| row.get(0),
        )?)
    }
    pub fn harness_inputs(&self) -> Result<Vec<String>> {
        let path = self
            .rig
            .machine_home
            .path()
            .join(format!("inputs-{}-{}.jsonl", self.workspace, self.id));
        if !path.exists() {
            return Ok(Vec::new());
        }
        std::fs::read_to_string(path)?
            .lines()
            .map(|line| serde_json::from_str(line).map_err(Into::into))
            .collect()
    }

    pub fn native_controls(&self) -> Result<Vec<serde_json::Value>> {
        let path = self
            .rig
            .machine_home
            .path()
            .join(format!("controls-{}-{}.jsonl", self.workspace, self.id));
        let text = std::fs::read_to_string(path)?;
        Ok(text
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?)
    }

    pub async fn delete(&self) -> Result<()> {
        Ok(self
            .rig
            .client
            .delete_agent(&self.workspace, &self.id)
            .await?)
    }

    pub fn received_steers(&self) -> Result<Vec<serde_json::Value>> {
        let path = self
            .rig
            .machine_home
            .path()
            .join(format!("steers-{}-{}.jsonl", self.workspace, self.id));
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        Ok(text
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?)
    }

    pub async fn release_run(&self) -> Result<()> {
        std::fs::write(
            self.rig
                .machine_home
                .path()
                .join(format!("release-{}-{}", self.workspace, self.id)),
            b"release",
        )?;
        Ok(())
    }

    pub async fn executions(&self) -> Result<Vec<Execution>> {
        let database = self.rig.database()?;
        let mut query = database.prepare("SELECT execution_id,state,final_output,attempt,error_code,created_at,completed_at FROM executions WHERE workspace_id=?1 AND agent_id=?2 ORDER BY execution_id")?;
        let mut executions = query
            .query_map(params![self.workspace, self.id], |row| {
                Ok(Execution {
                    id: row.get(0)?,
                    state: row.get(1)?,
                    output: row.get(2)?,
                    attempt: row.get(3)?,
                    error_code: row.get(4)?,
                    created_at: row.get(5)?,
                    completed_at: row.get(6)?,
                    trigger_messages: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for execution in &mut executions {
            let mut triggers = database.prepare("SELECT m.content_markdown FROM unread_snapshot_triggers t JOIN messages m USING(message_id) JOIN executions e USING(snapshot_id) WHERE e.execution_id=?1 ORDER BY t.position")?;
            execution.trigger_messages = triggers
                .query_map([execution.id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
        }
        Ok(executions)
    }
    pub async fn wait_until_running(&self) -> Result<()> {
        timeout(self.rig.config.timeout, async {
            loop {
                self.rig.check_daemons()?;
                // A saved started event proves the Harness is running, beyond
                // the Runtime's pre-assignment running state.
                let started: bool = self.rig.database()?.query_row("SELECT EXISTS(SELECT 1 FROM execution_events v JOIN executions e USING(execution_id) WHERE e.workspace_id=?1 AND e.agent_id=?2 AND e.state='running' AND v.kind='started')", params![self.workspace,self.id], |row| row.get(0))?;
                if started { return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(()); }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.map_err(|_| "Agent did not start before timeout")?
    }
}

fn factories(im: Arc<ImState>) -> Result<ProviderFactoryRegistry> {
    let factories = ProviderFactoryRegistry::default();
    let state = im.clone();
    factories.register_host_factory(
        ScriptedProvider {
            state: state.clone(),
        }
        .info(),
        move |workspace, services| {
            ProviderHost::new(
                ScriptedProvider {
                    state: state.clone(),
                },
                workspace.id.clone(),
                workspace.provider_config.clone(),
                services,
            )
        },
    )?;
    Ok(factories)
}

/// Test-only process composition with the same scripted native Harness.
pub async fn run_scripted_machine(
    home: PathBuf,
    url: String,
    secret: PathBuf,
    socket: PathBuf,
    executable: PathBuf,
    capabilities: Vec<String>,
) -> Result<()> {
    let mut config = DaemonConfig::for_home(&home)?;
    config.runtime_url = Some(url);
    config.link_secret_file = Some(secret);
    config.paths.runtime_run = socket;
    let registration = RegisteredHarness::new(
        ScriptedHarness::new(home.join("harness-pids.tsv")),
        HarnessConfig {
            harness_id: "pi".into(),
            model: ModelRef::new("scripted", "scripted"),
            thinking_level: "low".into(),
            cli_path: executable,
            environment: BTreeMap::from([(
                "LANKIDE_SYSTEM_CAPABILITIES".into(),
                serde_json::to_string(&capabilities)?,
            )]),
            config_fingerprint: "system-rig".into(),
        },
    );
    lankide_machine::run_with_harnesses(config, vec![registration]).await?;
    Ok(())
}
