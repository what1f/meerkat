#![forbid(unsafe_code)]

//! Test-only business scenarios through the real Runtime and Computer daemons.

mod provider;
mod script;
pub use provider::{Incoming, Outbound, OutboundKind};
pub use script::{Script, scripted_failure};

use lankide_client::{AgentDraft, ManagementClient, RuntimeManagementClient, WorkspaceDraft};
use lankide_computer::RegisteredHarness;
use lankide_computer_daemon::DaemonConfig;
use lankide_core::{AgentId, ChannelId};
use lankide_harness::{HarnessConfig, ModelRef};
use lankide_provider::{
    EventSender, EventSenderType, MessageEvent, Provider, ProviderEvent, ProviderEventKind,
    ProviderHost,
};
use lankide_runtime::ProviderFactoryRegistry;
use lankide_runtime_daemon::{RuntimePaths, ShutdownReason, run_daemon};
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
            retry: Default::default(),
        }
    }
}

struct ComputerThread {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<Result<()>>>,
}
impl ComputerThread {
    fn start(config: DaemonConfig, executable: PathBuf, journal: PathBuf) -> Self {
        let (stop, stopped) = oneshot::channel();
        let thread = std::thread::spawn(move || {
            // Dropping this executor is a process-crash boundary: all Computer
            // tasks and their sessions stop together, without an OS-wide signal.
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async move {
                let registration = RegisteredHarness::new(ScriptedHarness::new(journal), HarnessConfig {
                    harness_id: "pi".into(), model: ModelRef::new("scripted", "scripted"), thinking_level: "low".into(),
                    cli_path: executable, environment: BTreeMap::new(), config_fingerprint: "system-rig".into(),
                });
                tokio::select! {
                    result = lankide_computer_daemon::run_with_harnesses(config, vec![registration]) => result.map_err(Into::into),
                    _ = stopped => Ok(()),
                }
            })
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
            thread.join().map_err(|_| "Computer thread panicked")??;
        }
        Ok(())
    }
}
impl Drop for ComputerThread {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct SystemRig {
    // Homes stay alive until both daemons release their tasks.
    computer: Option<ComputerThread>,
    runtime: Option<JoinHandle<std::result::Result<(), lankide_runtime_daemon::AppError>>>,
    shutdown: Option<oneshot::Sender<()>>,
    home: TempDir,
    computer_home: TempDir,
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
        let computer_home = tempfile::Builder::new()
            .prefix("system-")
            .tempdir_in("/tmp")?;
        let paths = RuntimePaths::for_home(home.path())?;
        std::fs::write(
            home.path().join("retry-policy.json"),
            serde_json::to_vec(&config.retry)?,
        )?;
        let im = Arc::new(ImState::default());
        let factories = factories(im.clone())?;
        let (shutdown, stopped) = oneshot::channel();
        let daemon_paths = paths.clone();
        let runtime = tokio::spawn(async move {
            let result = run_daemon(daemon_paths, factories, async move {
                let _ = stopped.await;
                Ok(ShutdownReason::Internal)
            })
            .await;
            if let Err(error) = &result {
                eprintln!("system Runtime: {error:?}");
            }
            result
        });
        let mut rig = Self {
            computer: None,
            runtime: Some(runtime),
            shutdown: Some(shutdown),
            client: RuntimeManagementClient::connect_lazy(&paths.socket),
            home,
            computer_home,
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
        rig.start_computer()?;
        rig.wait_for_computer().await?;
        Ok(rig)
    }
    fn check_daemons(&self) -> Result<()> {
        if self.runtime.as_ref().is_some_and(|task| task.is_finished()) {
            return Err("Runtime daemon stopped unexpectedly".into());
        }
        if self
            .computer
            .as_ref()
            .and_then(|computer| computer.thread.as_ref())
            .is_some_and(|thread| thread.is_finished())
        {
            return Err("Computer daemon stopped unexpectedly".into());
        }
        Ok(())
    }
    fn start_computer(&mut self) -> Result<()> {
        let mut config = DaemonConfig::for_home(self.computer_home.path())?;
        config.paths.prepare()?;
        config.paths.runtime_socket = self.paths.socket.clone();
        self.computer = Some(ComputerThread::start(
            config,
            self.config.harness_executable.clone(),
            self.home.path().join("harness-pids.tsv"),
        ));
        Ok(())
    }
    async fn wait_for_computer(&self) -> Result<()> {
        timeout(self.config.timeout, async {
            loop {
                self.check_daemons()?;
                if let Ok(computers) = self.client.get_computer_catalog(None).await {
                    if computers
                        .iter()
                        .any(|computer| computer.id == "local" && computer.available)
                    {
                        return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "Computer startup timed out")?
    }
    pub fn outbound(&self) -> Vec<Outbound> {
        self.im.outbound.lock().expect("outbound").clone()
    }
    pub fn computer_failure_log(&self) -> Result<String> {
        Ok(std::fs::read_to_string(
            self.computer_home
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
    pub async fn restart_runtime(&mut self) -> Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(runtime) = self.runtime.take() {
            timeout(self.config.timeout, runtime).await???;
        }
        let factories = factories(self.im.clone())?;
        let paths = self.paths.clone();
        let (shutdown, stopped) = oneshot::channel();
        self.shutdown = Some(shutdown);
        self.runtime = Some(tokio::spawn(async move {
            run_daemon(paths, factories, async move {
                let _ = stopped.await;
                Ok(ShutdownReason::Internal)
            })
            .await
        }));
        self.client = RuntimeManagementClient::connect_lazy(&self.paths.socket);
        // The daemon's graceful shutdown frame ends its managed Computer peer.
        // Reopen the same Computer home, as a service supervisor does.
        if let Some(mut computer) = self.computer.take() {
            computer.stop()?;
        }
        self.start_computer()?;
        self.wait_for_computer().await
    }
    pub async fn restart_computer(&mut self) -> Result<()> {
        if let Some(mut computer) = self.computer.take() {
            computer.stop()?;
        }
        self.start_computer()?;
        self.wait_for_computer().await
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
        if let Some(mut computer) = self.computer.take() {
            computer.stop()?;
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
        self.computer.take();
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
    rig: &'a SystemRig,
    workspace: String,
    name: String,
    script: Script,
}
impl<'a> AgentBuilder<'a> {
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
                    id: self.name.clone(),
                    name: self.name.clone(),
                    model: "scripted".into(),
                    cli_path: self.rig.config.harness_executable.display().to_string(),
                    environment_variables: vec![
                        (
                            "LANKIDE_SYSTEM_INPUTS".into(),
                            self.rig
                                .home
                                .path()
                                .join(format!("inputs-{}-{}.jsonl", self.workspace, self.name))
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
                                .home
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
        let mut client = lankide_protocol::connect_uds_lazy(
            &self.rig.paths.socket,
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
                        mode: mode as i32,
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
            .home
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

    pub async fn release_run(&self) -> Result<()> {
        std::fs::write(
            self.rig
                .home
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
