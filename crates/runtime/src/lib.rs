#![forbid(unsafe_code)]

//! Runtime orchestration. It owns scheduling, persistence, and Provider
//! instance lifecycles; concrete adapters are assembled by application roots.

mod agent_profile;
mod application;
mod capability;
mod computer_broker;
mod management;
mod peer;
mod prompt;
mod resource_store;
pub use application::*;
pub use computer_broker::*;
pub use peer::*;
pub use resource_store::*;

use agent_profile::{AgentProfileFiles, StoredAgentAvatar, avatar_file};

use lankide_core::{
    AgentId, AttentionLevel, ChannelAbility, ChannelId, MessageAuthorType, MessageId, PersonId,
    ProviderId, ResourceId, ScheduleInterval, UnixMillis, WorkspaceId,
};
use lankide_protocol::{
    AgentRuntimeProfile, AssignExecution, CliCommand, Delivery, DeliveryKind, ExecutionRef,
    HarnessConfig, ResourceRef, SessionDirective, ToolDefinition as ProtocolToolDefinition,
    WorkspaceRef,
};
use lankide_provider::{
    AgentRuntimeSpec, AutoReactionSpec, ChannelDefaultAttention, ChannelDefinition,
    CliCommandDefinition, EditMessageSpec, EventDelivery, EventSenderType, HostServices,
    InterfaceCatalogEntry, Provider, ProviderError, ProviderEvent, ProviderEventKind, ProviderHost,
    ProviderHostInstance, ProviderInfo, ReactionAction, RecallMessageSpec, SendMessageResult,
    SendMessageSpec, ToolDefinition,
};
pub use lankide_provider::{ProviderOnboarding, ProviderOnboardingField, ProviderOnboardingStatus};
use lankide_storage::{
    AttemptAuthorization, AttemptCapability, DatabaseConfig, DispatchReason, ExecutionAuthSpec,
    GuardedSendResult, InboxDispatch, IncomingReaction, IngestEvent, IngestResult, MessageInput,
    ReactionOrigin, ReactionResult, ResourceView, SendOutcome, SendRequest, SnapshotView, Storage,
    StorageError, TeamRole, ToolInvocationRequest, WorkspaceRecord,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    io::Read,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc, RwLock as StdRwLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub database: DatabaseConfig,
}

pub trait ProviderTypeFactory: Send + Sync {
    fn info(&self) -> ProviderInfo;
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }
    fn create(
        &self,
        workspace: &WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn ProviderHostInstance>, ProviderError>;
    fn workspace_onboarding(
        &self,
        _action: String,
        _opaque_payload: serde_json::Value,
        _services: HostServices,
    ) -> ProviderFactoryFuture<'_, ProviderOnboarding> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "workspace_onboarding",
            })
        })
    }
}

pub type ProviderFactoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

#[derive(Clone, Default)]
pub struct ProviderFactoryRegistry {
    factories: Arc<StdRwLock<BTreeMap<ProviderId, Arc<dyn ProviderTypeFactory>>>>,
}
impl ProviderFactoryRegistry {
    pub fn register(&self, factory: Arc<dyn ProviderTypeFactory>) -> Result<(), RuntimeError> {
        let id = factory.info().id;
        let mut factories = self
            .factories
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        if factories.insert(id.clone(), factory).is_some() {
            return Err(RuntimeError::DuplicateProviderType(id));
        }
        Ok(())
    }

    pub fn register_host_factory<P, F>(
        &self,
        info: ProviderInfo,
        factory: F,
    ) -> Result<(), RuntimeError>
    where
        P: Provider,
        F: Fn(&WorkspaceRecord, HostServices) -> Result<ProviderHost<P>, ProviderError>
            + Send
            + Sync
            + 'static,
    {
        self.register(Arc::new(HostFactory {
            info,
            factory,
            _provider: PhantomData,
        }))
    }

    pub fn factory(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Arc<dyn ProviderTypeFactory>, RuntimeError> {
        self.factories
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(provider_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownProviderType(provider_id.clone()))
    }

    pub fn catalog(&self) -> Result<Vec<ProviderInfo>, RuntimeError> {
        Ok(self
            .factories
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .values()
            .map(|factory| factory.info())
            .collect())
    }
}

struct HostFactory<P, F> {
    info: ProviderInfo,
    factory: F,
    _provider: PhantomData<fn() -> P>,
}
impl<P, F> ProviderTypeFactory for HostFactory<P, F>
where
    P: Provider,
    F: Fn(&WorkspaceRecord, HostServices) -> Result<ProviderHost<P>, ProviderError> + Send + Sync,
{
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }
    fn capabilities(&self) -> Vec<String> {
        P::capabilities()
            .iter()
            .map(|capability| (*capability).to_owned())
            .collect()
    }
    fn create(
        &self,
        workspace: &WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn ProviderHostInstance>, ProviderError> {
        (self.factory)(workspace, services)
            .map(|host| Arc::new(host) as Arc<dyn ProviderHostInstance>)
    }
}

pub struct Runtime {
    config: RuntimeConfig,
    storage: Arc<Storage>,
    providers: StdRwLock<BTreeMap<WorkspaceId, Arc<ProviderInstance>>>,
}

impl Runtime {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let storage = Arc::new(Storage::open(config.database.clone())?);
        Ok(Self {
            config,
            storage,
            providers: StdRwLock::new(BTreeMap::new()),
        })
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    pub fn storage(&self) -> &Storage {
        self.storage.as_ref()
    }

    pub(crate) fn storage_arc(&self) -> Arc<Storage> {
        self.storage.clone()
    }

    /// Creates a detached Broker for embedding Runtime without RuntimeApplication.
    /// Production management paths must use RuntimeApplication's connected Broker.
    pub fn new_computer_broker(&self) -> ComputerBroker {
        ComputerBroker::new(self.storage.clone())
    }

    pub(crate) fn close(self) -> Result<(), RuntimeError> {
        let providers = self
            .providers
            .into_inner()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        drop(providers);
        Arc::try_unwrap(self.storage)
            .map_err(|_| RuntimeError::StorageStillShared)?
            .close()?;
        Ok(())
    }

    /// Registers a restartable Provider instance for one Workspace.
    ///
    /// The factory is intentionally retained: a stopped [`ProviderHost`] owns a
    /// closed event sink, so resume must construct a fresh host rather than
    /// pretending that the stopped instance can consume events again.
    pub fn register_provider_factory<P, F>(
        &self,
        workspace_id: WorkspaceId,
        factory: F,
    ) -> Result<(), RuntimeError>
    where
        P: Provider,
        F: Fn() -> Result<ProviderHost<P>, ProviderError> + Send + Sync + 'static,
    {
        let factory =
            Arc::new(move || factory().map(|host| Arc::new(host) as Arc<dyn ProviderHostInstance>));
        self.register_provider_instance_factory(workspace_id, factory)
    }

    pub fn register_provider_instance_factory(
        &self,
        workspace_id: WorkspaceId,
        factory: ProviderFactory,
    ) -> Result<(), RuntimeError> {
        let mut providers = self
            .providers
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        if providers.contains_key(&workspace_id) {
            return Err(RuntimeError::DuplicateWorkspace(workspace_id));
        }
        let active = self.storage.workspace(&workspace_id)?.active;
        providers.insert(
            workspace_id,
            Arc::new(ProviderInstance {
                active: AtomicBool::new(active),
                factory,
                host: Mutex::new(None),
                channels: RwLock::new(Vec::new()),
                agents: RwLock::new(BTreeMap::new()),
            }),
        );
        Ok(())
    }

    pub fn provider_count(&self) -> usize {
        self.providers
            .read()
            .map(|values| values.len())
            .unwrap_or(0)
    }

    pub fn has_provider_instance(&self, workspace_id: &WorkspaceId) -> bool {
        self.providers
            .read()
            .map(|values| values.contains_key(workspace_id))
            .unwrap_or(false)
    }

    pub async fn remove_provider_instance(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<bool, RuntimeError> {
        let provider = self
            .providers
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(workspace_id);
        let Some(provider) = provider else {
            return Ok(false);
        };
        provider.active.store(false, Ordering::Release);
        if let Some(host) = provider.host.lock().await.take() {
            host.shutdown().await?;
        }
        Ok(true)
    }

    pub fn is_workspace_active(&self, workspace_id: &WorkspaceId) -> Result<bool, RuntimeError> {
        Ok(self.provider(workspace_id)?.active.load(Ordering::Acquire))
    }

    /// Starts every registered Workspace whose configured state is active.
    pub async fn start(&self) -> Result<(), RuntimeError> {
        let providers = self.provider_entries()?;
        for (workspace_id, provider) in providers {
            if provider.active.load(Ordering::Acquire) {
                self.start_instance(&workspace_id, &provider).await?;
            }
        }
        Ok(())
    }

    /// Stops consumption before asking the Provider to shut down.
    pub async fn pause_workspace(
        &self,
        workspace_id: &WorkspaceId,
        now: i64,
    ) -> Result<(), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        provider.active.store(false, Ordering::Release);
        if let Err(error) = self.storage.set_workspace_active(workspace_id, false, now) {
            provider.active.store(true, Ordering::Release);
            return Err(error.into());
        }
        provider.channels.write().await.clear();
        if !self.storage.has_active_execution(workspace_id)? {
            if let Some(host) = provider.host.lock().await.take() {
                host.shutdown().await?;
            }
        }
        Ok(())
    }

    /// Creates a new ProviderHost and resumes event consumption for a paused Workspace.
    pub async fn resume_workspace(
        &self,
        workspace_id: &WorkspaceId,
        now: i64,
    ) -> Result<(), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        if provider.active.load(Ordering::Acquire) && provider.host.lock().await.is_some() {
            return Ok(());
        }
        let has_existing_host = provider.host.lock().await.is_some();
        if has_existing_host && self.storage.has_active_execution(workspace_id)? {
            return Err(RuntimeError::Application(
                "workspace is still completing an active execution".into(),
            ));
        }
        if let Some(host) = provider.host.lock().await.take() {
            host.shutdown().await?;
        }
        if let Err(error) = self.start_instance(workspace_id, &provider).await {
            provider.active.store(false, Ordering::Release);
            return Err(error);
        }
        self.storage.set_workspace_active(workspace_id, true, now)?;
        provider.active.store(true, Ordering::Release);
        Ok(())
    }

    pub async fn finish_paused_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<(), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        if provider.active.load(Ordering::Acquire)
            || self.storage.has_active_execution(workspace_id)?
        {
            return Ok(());
        }
        if let Some(host) = provider.host.lock().await.take() {
            host.shutdown().await?;
        }
        Ok(())
    }

    pub async fn configure_agent(
        &self,
        workspace_id: &WorkspaceId,
        config: AgentRuntimeConfig,
    ) -> Result<(), RuntimeError> {
        if !self
            .storage
            .list_agents(workspace_id)?
            .iter()
            .any(|agent| agent.id == config.agent_id)
        {
            return Err(RuntimeError::UnknownAgent {
                workspace_id: workspace_id.clone(),
                agent_id: config.agent_id,
            });
        }
        self.provider(workspace_id)?
            .agents
            .write()
            .await
            .insert(config.agent_id.clone(), config);
        Ok(())
    }

    pub async fn remove_agent(&self, workspace_id: &WorkspaceId, agent_id: &AgentId) {
        if let Ok(provider) = self.provider(workspace_id) {
            provider.agents.write().await.remove(agent_id);
        }
    }

    pub async fn channels(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ChannelDefinition>, RuntimeError> {
        self.require_active(workspace_id)?;
        Ok(self.provider(workspace_id)?.channels.read().await.clone())
    }

    /// Waits for one normalized event from a running Workspace instance.
    /// Paused Workspaces never touch the Provider receiver.
    pub async fn next_provider_event(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<EventDelivery>, RuntimeError> {
        self.require_active(workspace_id)?;
        let host = self.host(workspace_id).await?;
        let event = host.next_event().await;
        if !self.is_workspace_active(workspace_id)? {
            return Err(RuntimeError::WorkspacePaused(workspace_id.clone()));
        }
        Ok(event)
    }

    /// Accept a Provider delivery only after its canonical event has been committed.
    pub async fn accept_provider_event(
        &self,
        workspace_id: &WorkspaceId,
        delivery: EventDelivery,
        now: i64,
    ) -> Result<CoordinatedEvent, RuntimeError> {
        let (event, acknowledgement) = delivery.into_parts();
        let result = self.coordinate_event(workspace_id, event, now).await;
        if result.is_ok() {
            acknowledgement.accept();
        } else {
            acknowledgement.reject();
        }
        result
    }

    /// Persists one normalized Provider event and computes its Agent dispatches.
    /// The returned snapshot boundary is still created later by `create_execution`.
    pub async fn coordinate_event(
        &self,
        workspace_id: &WorkspaceId,
        event: ProviderEvent,
        now: i64,
    ) -> Result<CoordinatedEvent, RuntimeError> {
        self.require_active(workspace_id)?;
        let provider = self.provider(workspace_id)?;
        let channels = provider.channels.read().await;
        let channel = channels
            .iter()
            .find(|channel| channel.id == event.channel_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownChannel {
                workspace_id: workspace_id.clone(),
                channel_id: event.channel_id.clone(),
            })?;
        drop(channels);
        let agents = provider.agents.read().await.clone();
        let provider_id = self.storage.workspace(workspace_id)?.provider_id;
        let occurred_at = event.occurred_at_millis().unwrap_or(now);
        let chat_name = event
            .provider_data
            .get("chat_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&event.provider_chat_id)
            .to_owned();
        let provider_event_id = event.event_id().map(str::to_owned);

        match event.kind {
            ProviderEventKind::Message(message) => {
                let mut chat_attention = BTreeMap::new();
                for agent_id in agents.keys() {
                    if let Some(attention) = self.storage.chat_attention(
                        workspace_id,
                        agent_id,
                        &event.channel_id,
                        &event.provider_chat_id,
                    )? {
                        chat_attention.insert(agent_id.clone(), attention_policy(&attention)?);
                    }
                }
                let team_command =
                    self.apply_team_command(workspace_id, &event.sender, &message.content, now)?;
                let dispatch = if team_command {
                    Vec::new()
                } else {
                    message_dispatches(
                        &channel,
                        &agents,
                        &chat_attention,
                        &event.sender,
                        &message,
                        now,
                    )?
                };
                let resources = message
                    .attachments
                    .iter()
                    .map(resource_view)
                    .collect::<Result<Vec<_>, _>>()?;
                let result = self.storage.ingest_event(IngestEvent {
                    workspace_id: workspace_id.clone(),
                    provider_id,
                    channel_id: event.channel_id.clone(),
                    channel_name: channel.name.clone(),
                    provider_chat_id: event.provider_chat_id.clone(),
                    chat_name,
                    provider_event_id,
                    provider_message_id: message.provider_message_id,
                    message: MessageInput {
                        sender_kind: sender_kind(event.sender.kind),
                        sender_id: event.sender.id,
                        sender_name: event.sender.name,
                        content: message.content,
                        mentions: message.mentioned_agent_ids,
                        reply_to_provider_message_id: message.reply_to_provider_message_id,
                        resources,
                        occurred_at,
                    },
                    dispatch: dispatch.clone(),
                    now,
                })?;
                // Keep new messages pending for the next execution. Injecting only
                // their text into a running execution would not expand that run's
                // readable messages, reply scope, or cursor boundary.
                let steer_deliveries = Vec::new();
                let auto_reaction_errors = Vec::new();
                if !result.new_inbox_items.is_empty()
                    && channel.abilities.contains(&ChannelAbility::Reaction)
                {
                    let message_id = MessageId::new(result.message_id)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                    let host = self.host(workspace_id).await?;
                    for dispatch in dispatch.into_iter().filter(|dispatch| {
                        result
                            .new_inbox_items
                            .iter()
                            .any(|(agent_id, _)| agent_id == &dispatch.agent_id)
                    }) {
                        let Some(agent) = agents.get(&dispatch.agent_id) else {
                            continue;
                        };
                        let Some(auto_reaction) = agent.auto_reaction.clone() else {
                            continue;
                        };
                        let host = host.clone();
                        let workspace_id = workspace_id.clone();
                        let agent_id = agent.agent_id.clone();
                        let spec = AutoReactionSpec {
                            auto_reaction,
                            channel_id: event.channel_id.clone(),
                            chat_id: event.provider_chat_id.clone(),
                            trigger_messages: vec![message_id],
                            provider_agent_config: agent.provider_agent_config.clone(),
                        };
                        tokio::spawn(async move {
                            if let Err(error) = host.auto_reaction(agent_id.clone(), spec).await {
                                eprintln!(
                                    "[runtime] workspace={workspace_id} agent={agent_id} auto reaction failed: {error}"
                                );
                            }
                        });
                    }
                }
                Ok(CoordinatedEvent::Message {
                    result,
                    auto_reaction_errors,
                    steer_deliveries,
                })
            }
            ProviderEventKind::Reaction(reaction) => {
                let own_auto_reaction = agents.values().any(|agent| {
                    agent.provider_sender_id == reaction.sender.id
                        && agent.auto_reaction.as_deref() == Some(reaction.reaction.as_str())
                });
                let dispatch = if own_auto_reaction
                    || reaction.action == ReactionAction::Remove
                    || !channel.abilities.contains(&ChannelAbility::Reaction)
                {
                    Vec::new()
                } else {
                    reaction_dispatches(&agents, &reaction.sender, &reaction.reaction, now)
                };
                let event_id = provider_event_id.unwrap_or_else(|| {
                    format!(
                        "reaction:{}:{}:{}:{:?}",
                        reaction.target_provider_message_id,
                        reaction.sender.id,
                        reaction.reaction,
                        reaction.action
                    )
                });
                let result = self.storage.record_reaction(IncomingReaction {
                    workspace_id: workspace_id.clone(),
                    provider_id,
                    channel_id: event.channel_id,
                    provider_chat_id: event.provider_chat_id,
                    target_provider_message_id: reaction.target_provider_message_id,
                    sender_kind: sender_kind(reaction.sender.kind),
                    sender_id: reaction.sender.id,
                    sender_name: reaction.sender.name,
                    reaction: reaction.reaction,
                    action: match reaction.action {
                        ReactionAction::Add => lankide_storage::ReactionAction::Add,
                        ReactionAction::Remove => lankide_storage::ReactionAction::Remove,
                    },
                    origin: if own_auto_reaction {
                        ReactionOrigin::AutoReaction
                    } else {
                        ReactionOrigin::Provider
                    },
                    provider_event_id: event_id,
                    dispatch,
                    occurred_at,
                    now,
                })?;
                Ok(CoordinatedEvent::Reaction(result))
            }
            ProviderEventKind::Custom(custom) => {
                let dispatch = custom
                    .target_agent_ids
                    .iter()
                    .filter(|id| agents.get(*id).is_some_and(|agent| !agent.muted()))
                    .map(|id| InboxDispatch {
                        agent_id: id.clone(),
                        available_at: now,
                        trigger_reason: DispatchReason::Custom {
                            name: custom.reason.clone(),
                            object_id: custom.subject_id.clone(),
                        },
                    })
                    .collect();
                let synthetic_message_id = provider_event_id
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| format!("custom-event:{value}"))
                    .unwrap_or_else(|| format!("custom-object:{}", custom.subject_id));
                let result = self.storage.ingest_event(IngestEvent {
                    workspace_id: workspace_id.clone(),
                    provider_id,
                    channel_id: event.channel_id,
                    channel_name: channel.name.clone(),
                    provider_chat_id: event.provider_chat_id,
                    chat_name,
                    provider_event_id,
                    provider_message_id: synthetic_message_id,
                    message: MessageInput {
                        sender_kind: sender_kind(event.sender.kind),
                        sender_id: event.sender.id,
                        sender_name: event.sender.name,
                        content: custom.subject_name,
                        mentions: custom.target_agent_ids,
                        reply_to_provider_message_id: None,
                        resources: Vec::new(),
                        occurred_at,
                    },
                    dispatch,
                    now,
                })?;
                Ok(CoordinatedEvent::Custom(result))
            }
        }
    }

    fn apply_team_command(
        &self,
        workspace_id: &WorkspaceId,
        sender: &lankide_provider::EventSender,
        content: &str,
        now: i64,
    ) -> Result<bool, RuntimeError> {
        if sender.kind != EventSenderType::Human {
            return Ok(false);
        }
        let Some(command) = content.strip_prefix("/team") else {
            return Ok(false);
        };
        let mut parts = command.trim_start().splitn(2, char::is_whitespace);
        let role = match parts.next() {
            Some("member") => TeamRole::Member,
            Some("lead") => TeamRole::Lead,
            _ => return Ok(false),
        };
        let Some(target) = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(false);
        };
        let Some(target) = lankide_core::markdown_mentions(target)
            .into_iter()
            .find(|mention| mention.range == (0..target.len()))
            .and_then(|mention| PersonId::new(mention.id.to_owned()).ok())
        else {
            return Ok(false);
        };
        let Ok(actor) = PersonId::new(sender.id.clone()) else {
            return Ok(false);
        };
        match self
            .storage
            .update_team_member(workspace_id, &actor, &target, role, now)
        {
            Ok(_) => Ok(true),
            Err(StorageError::Conflict(_)) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Builds Provider-owned prompt and capabilities from the same runtime
    /// context that will be sent to Computer.
    pub async fn dynamic_execution_input(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> Result<DynamicExecutionInput, RuntimeError> {
        self.require_active(workspace_id)?;
        let host = self.host(workspace_id).await?;
        let provider_prompt = host.system_prompt(agent_id.clone(), spec.clone()).await?;
        let workspace = self.storage.workspace_record(workspace_id)?;
        let humans = self.storage.list_human_team_member_views(workspace_id)?;
        let agents = self.storage.list_agent_records(workspace_id)?;
        let system_prompt = prompt::system_prompt(
            &workspace,
            &agent_id,
            &spec.agent_config,
            &humans,
            &agents,
            &provider_prompt,
        );
        let tools = host.tools(agent_id.clone(), spec.clone()).await?;
        let cli_commands = host.cli_commands(agent_id, spec).await?;
        Ok(DynamicExecutionInput {
            system_prompt,
            tools: tools
                .into_iter()
                .map(protocol_tool)
                .collect::<Result<Vec<_>, _>>()?,
            cli_commands: cli_commands.into_iter().map(protocol_cli).collect(),
        })
    }

    /// Claims all currently available unread Chats for one Agent and executes
    /// exactly that frozen multi-Chat snapshot.
    pub async fn execute_ready<C: ComputerExecutor>(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        computer: &C,
        now: i64,
    ) -> Result<ExecutionProgress, RuntimeError> {
        self.require_active(workspace_id)?;
        let agent = self
            .provider(workspace_id)?
            .agents
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownAgent {
                workspace_id: workspace_id.clone(),
                agent_id: agent_id.clone(),
            })?;
        let execution_token = if agent.execution_token.is_empty() {
            generate_execution_token()?
        } else {
            agent.execution_token.clone()
        };
        let provider_send_namespace = token_hash(&execution_token);
        let Some(created) = self.storage.create_execution_with_auth(
            workspace_id,
            agent_id,
            "unread",
            now,
            ExecutionAuthSpec {
                attempt: 1,
                token_hash: token_hash(&execution_token),
                expires_at: now.saturating_add(3_600_000),
            },
        )?
        else {
            return Ok(ExecutionProgress::Idle);
        };
        let snapshot_json = snapshot_json(&created.snapshot);
        let runtime_spec = AgentRuntimeSpec {
            agent_config: agent.agent_config.clone(),
            provider_agent_config: agent.provider_agent_config.clone(),
            computer_id: agent.computer_id.clone(),
            harness_id: agent.harness.harness_id.clone(),
            model: agent.harness.model.clone(),
            thinking_level: agent.harness.thinking_level.clone(),
            trigger_reason: trigger_summary(&created.snapshot),
            chats: runtime_chats(&created.snapshot),
            unread_snapshot: snapshot_json.clone(),
        };
        let mut dynamic = match self
            .dynamic_execution_input(workspace_id, agent_id.clone(), runtime_spec.clone())
            .await
        {
            Ok(input) => input,
            Err(error) => {
                self.storage.fail_execution_retry_after(
                    created.execution_id,
                    "input_preparation_failed",
                    now.saturating_add(60_000),
                    now,
                )?;
                return Err(error);
            }
        };
        let mut tools = capability::standard_tool_definitions();
        tools.append(&mut dynamic.tools);
        let context_json = frozen_runtime_spec_json(&runtime_spec).to_string();
        let context_fingerprint = lowercase_sha256(context_json.as_bytes());
        let mut capabilities = tools
            .iter()
            .map(|tool| AttemptCapability {
                name: tool.name.clone(),
                display_name: tool.name.clone(),
                description: tool.description.clone(),
                kind: lankide_protocol::CapabilityKind::Tool as i32,
                input_schema_json: tool.input_schema_json.clone(),
                output_schema_json: tool.output_schema_json.clone(),
            })
            .collect::<Vec<_>>();
        capabilities.extend(
            dynamic
                .cli_commands
                .iter()
                .map(|command| AttemptCapability {
                    name: format!("cli: {}", command.name),
                    display_name: command.name.clone(),
                    description: command.description.clone(),
                    kind: lankide_protocol::CapabilityKind::CliCommand as i32,
                    input_schema_json: r#"{"type":"array","items":{"type":"string"}}"#.into(),
                    output_schema_json: "{}".into(),
                }),
        );
        if let Err(error) = self
            .storage
            .freeze_attempt_authorization(AttemptAuthorization {
                execution_id: created.execution_id,
                attempt: 1,
                context_json,
                context_fingerprint,
                capabilities,
            })
        {
            self.storage.fail_execution(
                created.execution_id,
                "authorization_preparation_failed",
                now,
            )?;
            return Err(error.into());
        }
        let execution_ref = ExecutionRef {
            execution_id: created.execution_id,
            attempt: 1,
        };
        let assignment = AssignExecution {
            execution: Some(execution_ref),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            agent: Some(agent.agent_profile.clone()),
            harness: Some(agent.harness.clone()),
            session: Some(agent.session.clone()),
            initial_delivery: Some(Delivery {
                delivery_id: created.delivery_id,
                kind: DeliveryKind::Initial as i32,
                rendered_markdown: render_snapshot(&created.snapshot),
                snapshot_id: created.snapshot_id,
                trigger_reasons: trigger_reasons(&created.snapshot),
                resources: snapshot_resources(&created.snapshot),
                created_at: now,
            }),
            system_prompt: dynamic.system_prompt,
            tools,
            cli_commands: dynamic.cli_commands,
            resources: snapshot_resources(&created.snapshot),
            execution_token,
            created_at: now,
        };
        self.storage
            .mark_execution_running(created.execution_id, now)?;
        let completion = match computer.execute(assignment).await {
            Ok(completion) => completion,
            Err(error) => {
                if error.code == "computer_disconnected" {
                    self.storage
                        .mark_execution_recovering(created.execution_id, now)?;
                } else {
                    self.storage.fail_execution(
                        created.execution_id,
                        "computer_unavailable",
                        now,
                    )?;
                }
                return Ok(ExecutionProgress::Failed {
                    execution_id: created.execution_id,
                    code: error.code,
                });
            }
        };
        if !completion.delivery_accepted {
            self.storage
                .fail_execution(created.execution_id, "delivery_rejected", now)?;
            return Ok(ExecutionProgress::Failed {
                execution_id: created.execution_id,
                code: "delivery_rejected".into(),
            });
        }
        self.storage
            .mark_delivery_delivered(created.delivery_id, now)?;
        match completion.outcome {
            ComputerOutcome::Failed { code } => {
                self.storage.fail_execution_retry_after(
                    created.execution_id,
                    &code,
                    now.saturating_add(60_000),
                    now,
                )?;
                Ok(ExecutionProgress::Failed {
                    execution_id: created.execution_id,
                    code,
                })
            }
            ComputerOutcome::Cancelled => {
                self.storage.cancel_execution(created.execution_id, now)?;
                Ok(ExecutionProgress::Cancelled {
                    execution_id: created.execution_id,
                })
            }
            ComputerOutcome::Succeeded { output, reply } => {
                let Some(reply) = reply else {
                    let committed =
                        self.storage
                            .commit_execution(created.execution_id, &output, None, now)?;
                    return Ok(ExecutionProgress::Committed {
                        execution_id: created.execution_id,
                        committed: matches!(
                            committed,
                            lankide_storage::CommitResult::Committed
                                | lankide_storage::CommitResult::AlreadyCommitted
                        ),
                    });
                };
                let request_key = format!("execution:{}:reply", created.execution_id);
                let send = match self.storage.begin_send_guarded(
                    created.snapshot_id,
                    &reply.channel_id,
                    &reply.provider_chat_id,
                    false,
                    SendRequest {
                        workspace_id: workspace_id.clone(),
                        channel_id: reply.channel_id.clone(),
                        provider_chat_id: reply.provider_chat_id.clone(),
                        request_key: request_key.clone(),
                        sender_agent_id: agent_id.clone(),
                        content: reply.content.clone(),
                        now,
                    },
                )? {
                    GuardedSendResult::Ready(send) => send,
                    GuardedSendResult::NewMessages { .. } => {
                        let committed = self.storage.commit_execution(
                            created.execution_id,
                            &output,
                            None,
                            now,
                        )?;
                        return Ok(ExecutionProgress::Committed {
                            execution_id: created.execution_id,
                            committed: matches!(
                                committed,
                                lankide_storage::CommitResult::Committed
                                    | lankide_storage::CommitResult::AlreadyCommitted
                            ),
                        });
                    }
                };
                let message_id = MessageId::new(send.message_id)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                let provider_result = self
                    .send_message(
                        workspace_id,
                        agent_id.clone(),
                        SendMessageSpec {
                            channel_id: reply.channel_id,
                            chat_id: reply.provider_chat_id,
                            message_id,
                            idempotency_key: provider_send_idempotency_key(
                                &provider_send_namespace,
                                &request_key,
                            ),
                            content: reply.content,
                            reply_to: None,
                            mentions: Vec::new(),
                            attachments: Vec::new(),
                            force: false,
                            provider_agent_config: agent.provider_agent_config,
                        },
                    )
                    .await;
                match provider_result {
                    Ok(result) if result.status == lankide_provider::SendMessageStatus::Sent => {
                        let provider_message_id = result.provider_message_id.ok_or_else(|| {
                            RuntimeError::InvalidCore(
                                "sent Provider result missing message id".into(),
                            )
                        })?;
                        self.storage.finish_send(
                            &request_key,
                            SendOutcome::Sent {
                                provider_message_id,
                            },
                            now,
                        )?;
                        let committed = self.storage.commit_execution(
                            created.execution_id,
                            &output,
                            Some(&request_key),
                            now,
                        )?;
                        Ok(ExecutionProgress::Committed {
                            execution_id: created.execution_id,
                            committed: matches!(
                                committed,
                                lankide_storage::CommitResult::Committed
                                    | lankide_storage::CommitResult::AlreadyCommitted
                            ),
                        })
                    }
                    Ok(_) => {
                        self.storage
                            .finish_send(&request_key, SendOutcome::Unknown, now)?;
                        Ok(ExecutionProgress::PendingSend {
                            execution_id: created.execution_id,
                            request_key,
                        })
                    }
                    Err(_) => {
                        self.storage.finish_send(
                            &request_key,
                            SendOutcome::Failed {
                                code: "provider_send_failed".into(),
                            },
                            now,
                        )?;
                        self.storage.fail_execution(
                            created.execution_id,
                            "provider_send_failed",
                            now,
                        )?;
                        Ok(ExecutionProgress::Failed {
                            execution_id: created.execution_id,
                            code: "provider_send_failed".into(),
                        })
                    }
                }
            }
        }
    }

    pub fn recoverable_executions(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<lankide_storage::RecoverableExecutionView>, RuntimeError> {
        Ok(self.storage.recoverable_executions(workspace_id)?)
    }

    pub fn recoverable_sends(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<lankide_storage::RecoverableSendView>, RuntimeError> {
        Ok(self.storage.recoverable_sends(workspace_id)?)
    }

    /// Resolves a Provider send whose outcome was unknown across a restart.
    /// The execution is committed only after Storage has durably linked the
    /// returned Provider message id to the pre-existing outbound Message.
    pub fn resolve_unknown_send(
        &self,
        workspace_id: &WorkspaceId,
        request_key: &str,
        provider_message_id: String,
        now: i64,
    ) -> Result<ExecutionProgress, RuntimeError> {
        let recoverable = self
            .storage
            .recoverable_sends(Some(workspace_id))?
            .into_iter()
            .find(|send| send.attempt.request_key == request_key)
            .ok_or_else(|| RuntimeError::UnknownSend(request_key.to_owned()))?;
        self.storage.finish_send(
            request_key,
            SendOutcome::Sent {
                provider_message_id,
            },
            now,
        )?;
        let execution_id = execution_id_from_request_key(request_key)?;
        let committed = self.storage.commit_execution(
            execution_id,
            &recoverable.content,
            Some(request_key),
            now,
        )?;
        Ok(ExecutionProgress::Committed {
            execution_id,
            committed: matches!(
                committed,
                lankide_storage::CommitResult::Committed
                    | lankide_storage::CommitResult::AlreadyCommitted
            ),
        })
    }

    pub async fn send_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: SendMessageSpec,
    ) -> Result<SendMessageResult, RuntimeError> {
        self.host(workspace_id)
            .await?
            .send_message(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn invoke_provider_tool(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        runtime_spec: AgentRuntimeSpec,
        request: ToolInvocationRequest,
    ) -> Result<ToolProgress, RuntimeError> {
        let begun = self.storage.begin_tool_invocation(request.clone())?;
        if !begun.created {
            return Ok(match (begun.state.as_str(), begun.result) {
                ("completed", Some(result)) => ToolProgress::Completed { result },
                ("failed", result) => ToolProgress::Failed {
                    code: result
                        .as_ref()
                        .and_then(|value| value.get("error"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("provider_tool_failed")
                        .to_owned(),
                },
                _ => ToolProgress::InProgress,
            });
        }
        let guard = ToolInvocationGuard::new(self.storage.as_ref(), request.clone());
        match self
            .host(workspace_id)
            .await?
            .invoke_tool(
                agent_id,
                runtime_spec,
                request.tool_name.clone(),
                request.arguments.clone(),
            )
            .await
        {
            Ok(result) => {
                guard.complete(result.clone(), request.now)?;
                Ok(ToolProgress::Completed { result })
            }
            Err(error) => {
                guard.fail("provider_tool_failed", now_millis())?;
                Err(error.into())
            }
        }
    }

    pub async fn invoke_provider_cli(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        runtime_spec: AgentRuntimeSpec,
        command: String,
        arguments: Vec<String>,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.host(workspace_id)
            .await?
            .invoke_cli(agent_id, runtime_spec, command, arguments)
            .await
            .map_err(Into::into)
    }

    pub async fn invoke_provider_interface(
        &self,
        workspace_id: &WorkspaceId,
        interface_id: String,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.require_active(workspace_id)?;
        self.host(workspace_id)
            .await?
            .invoke_interface(interface_id, input)
            .await
            .map_err(Into::into)
    }

    pub async fn recall_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: RecallMessageSpec,
    ) -> Result<(), RuntimeError> {
        self.host(workspace_id)
            .await?
            .recall_message(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn edit_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: EditMessageSpec,
    ) -> Result<(), RuntimeError> {
        self.host(workspace_id)
            .await?
            .edit_message(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn list_provider_interfaces(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<InterfaceCatalogEntry>, RuntimeError> {
        self.require_active(workspace_id)?;
        self.host(workspace_id)
            .await?
            .list_interfaces()
            .await
            .map_err(Into::into)
    }

    pub async fn validate_run_context(
        &self,
        context: &lankide_protocol::AgentRunContext,
        now: i64,
    ) -> Result<ValidatedRunContext, RuntimeError> {
        let execution = context
            .execution
            .as_ref()
            .ok_or_else(|| RuntimeError::Application("execution ref required".into()))?;
        let auth = self
            .storage
            .execution_auth_context(execution.execution_id, i64::from(execution.attempt))?
            .ok_or(RuntimeError::InvalidExecutionToken)?;
        if auth.expires_at < now
            || !constant_time_eq(&auth.token_hash, &token_hash(&context.execution_token))
            || matches!(
                auth.state,
                lankide_storage::ExecutionState::Succeeded
                    | lankide_storage::ExecutionState::Failed
                    | lankide_storage::ExecutionState::Cancelled
            )
        {
            return Err(RuntimeError::InvalidExecutionToken);
        }
        let snapshot_id = self.storage.execution(execution.execution_id)?.snapshot_id;
        let snapshot = self.storage.snapshot(snapshot_id)?;
        let authorization = self
            .storage
            .attempt_authorization(execution.execution_id, i64::from(execution.attempt))?
            .ok_or(RuntimeError::InvalidExecutionToken)?;
        if !constant_time_eq(
            &authorization.context_fingerprint,
            &lowercase_sha256(authorization.context_json.as_bytes()),
        ) {
            return Err(RuntimeError::InvalidExecutionToken);
        }
        let runtime_spec = frozen_runtime_spec(&authorization.context_json)?;
        let agent = self
            .provider(&auth.workspace_id)?
            .agents
            .read()
            .await
            .get(&auth.agent_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownAgent {
                workspace_id: auth.workspace_id.clone(),
                agent_id: auth.agent_id.clone(),
            })?;
        Ok(ValidatedRunContext {
            workspace_id: auth.workspace_id,
            agent_id: auth.agent_id,
            execution_id: execution.execution_id,
            attempt: execution.attempt,
            snapshot,
            agent,
            runtime_spec,
            capabilities: authorization.capabilities,
        })
    }

    pub async fn auto_reaction(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: AutoReactionSpec,
    ) -> Result<(), RuntimeError> {
        self.require_active(workspace_id)?;
        self.host(workspace_id)
            .await?
            .auto_reaction(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let providers = self.provider_entries()?;
        for (workspace_id, provider) in providers {
            provider.active.store(false, Ordering::Release);
            provider.channels.write().await.clear();
            if let Some(host) = provider.host.lock().await.take() {
                host.shutdown()
                    .await
                    .map_err(|source| RuntimeError::Provider {
                        workspace_id: workspace_id.clone(),
                        source,
                    })?;
            }
        }
        Ok(())
    }

    async fn start_instance(
        &self,
        workspace_id: &WorkspaceId,
        provider: &ProviderInstance,
    ) -> Result<(), RuntimeError> {
        let mut current = provider.host.lock().await;
        if current.is_some() {
            return Ok(());
        }
        let host = (provider.factory)().map_err(|source| RuntimeError::Provider {
            workspace_id: workspace_id.clone(),
            source,
        })?;
        let expected_provider_id = self.storage.workspace(workspace_id)?.provider_id;
        let actual_provider_id = host.provider_id();
        if actual_provider_id != expected_provider_id {
            return Err(RuntimeError::ProviderMismatch {
                workspace_id: workspace_id.clone(),
                expected: expected_provider_id,
                actual: actual_provider_id,
            });
        }
        host.init().await.map_err(|source| RuntimeError::Provider {
            workspace_id: workspace_id.clone(),
            source,
        })?;
        for agent in self.storage.list_agent_records(workspace_id)? {
            host.create_agent(
                agent.agent_id.clone(),
                management::provider_agent_spec(&agent),
            )
            .await
            .map_err(|source| RuntimeError::Provider {
                workspace_id: workspace_id.clone(),
                source,
            })?;
        }
        let channels = host
            .channels()
            .await
            .map_err(|source| RuntimeError::Provider {
                workspace_id: workspace_id.clone(),
                source,
            })?;
        *provider.channels.write().await = channels;
        *current = Some(host);
        Ok(())
    }

    fn provider(&self, workspace_id: &WorkspaceId) -> Result<Arc<ProviderInstance>, RuntimeError> {
        self.providers
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownWorkspace(workspace_id.clone()))
    }

    fn provider_entries(&self) -> Result<Vec<(WorkspaceId, Arc<ProviderInstance>)>, RuntimeError> {
        Ok(self
            .providers
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .iter()
            .map(|(id, provider)| (id.clone(), provider.clone()))
            .collect())
    }

    fn require_active(&self, workspace_id: &WorkspaceId) -> Result<(), RuntimeError> {
        if self.provider(workspace_id)?.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(RuntimeError::WorkspacePaused(workspace_id.clone()))
        }
    }

    pub async fn set_chat_attention(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        attention: lankide_storage::AttentionRecord,
        now: i64,
    ) -> Result<(), RuntimeError> {
        self.require_active(workspace_id)?;
        let provider = self.provider(workspace_id)?;
        let channels = provider.channels.read().await;
        let channel = channels
            .iter()
            .find(|channel| &channel.id == channel_id)
            .ok_or_else(|| RuntimeError::UnknownChannel {
                workspace_id: workspace_id.clone(),
                channel_id: channel_id.clone(),
            })?;
        let supported = match &attention {
            lankide_storage::AttentionRecord::Mention => {
                channel.attention_levels.contains(&AttentionLevel::Mention)
            }
            lankide_storage::AttentionRecord::Always => {
                channel.attention_levels.contains(&AttentionLevel::Follow)
            }
            lankide_storage::AttentionRecord::Schedule { .. } => {
                channel.attention_levels.contains(&AttentionLevel::Schedule)
            }
            lankide_storage::AttentionRecord::Mute => !channel.attention_levels.is_empty(),
        };
        if !supported {
            return Err(RuntimeError::Application(
                "This conversation does not support that attention level.".into(),
            ));
        }
        self.storage.set_chat_attention(
            workspace_id,
            agent_id,
            channel_id,
            provider_chat_id,
            &attention,
            now,
        )?;
        Ok(())
    }

    async fn host(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Arc<dyn ProviderHostInstance>, RuntimeError> {
        self.provider(workspace_id)?
            .host
            .lock()
            .await
            .clone()
            .ok_or_else(|| RuntimeError::ProviderNotStarted(workspace_id.clone()))
    }

    pub(crate) async fn lifecycle_host(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<(Arc<dyn ProviderHostInstance>, bool), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        if let Some(host) = provider.host.lock().await.clone() {
            return Ok((host, false));
        }
        let host = (provider.factory)().map_err(|source| RuntimeError::Provider {
            workspace_id: workspace_id.clone(),
            source,
        })?;
        Ok((host, true))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynamicExecutionInput {
    pub system_prompt: String,
    pub tools: Vec<ProtocolToolDefinition>,
    pub cli_commands: Vec<CliCommand>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttentionPolicy {
    Mention,
    Schedule(ScheduleInterval),
    Always,
    Mute,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRuntimeConfig {
    pub agent_id: AgentId,
    /// Provider-native sender identity used to suppress self-triggering echoes.
    pub provider_sender_id: String,
    pub attention: BTreeMap<ChannelId, AttentionPolicy>,
    pub auto_reaction: Option<String>,
    pub agent_config: serde_json::Value,
    pub provider_agent_config: serde_json::Value,
    pub agent_profile: AgentRuntimeProfile,
    pub computer_id: String,
    pub harness: HarnessConfig,
    pub session: SessionDirective,
    pub execution_token: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedRunContext {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub execution_id: i64,
    pub attempt: u32,
    pub snapshot: SnapshotView,
    pub agent: AgentRuntimeConfig,
    pub runtime_spec: AgentRuntimeSpec,
    pub capabilities: Vec<AttemptCapability>,
}
impl AgentRuntimeConfig {
    fn muted(&self) -> bool {
        !self
            .attention
            .values()
            .any(|policy| policy != &AttentionPolicy::Mute)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinatedEvent {
    Message {
        result: IngestResult,
        auto_reaction_errors: Vec<String>,
        steer_deliveries: Vec<lankide_storage::SteerDelivery>,
    },
    Reaction(ReactionResult),
    Custom(IngestResult),
}

pub type ComputerFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ComputerError>> + Send + 'a>>;

pub trait ComputerExecutor: Send + Sync {
    fn execute(&self, assignment: AssignExecution) -> ComputerFuture<'_, ComputerCompletion>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputerError {
    pub code: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComputerCompletion {
    pub delivery_accepted: bool,
    pub outcome: ComputerOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComputerOutcome {
    Succeeded {
        output: String,
        reply: Option<ExecutionReply>,
    },
    Failed {
        code: String,
    },
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReply {
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionProgress {
    Idle,
    Committed {
        execution_id: i64,
        committed: bool,
    },
    PendingSend {
        execution_id: i64,
        request_key: String,
    },
    Failed {
        execution_id: i64,
        code: String,
    },
    Cancelled {
        execution_id: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolProgress {
    InProgress,
    Completed { result: serde_json::Value },
    Failed { code: String },
}

pub(crate) struct ToolInvocationGuard<'a> {
    storage: &'a Storage,
    request: Option<ToolInvocationRequest>,
}

impl<'a> ToolInvocationGuard<'a> {
    pub(crate) fn new(storage: &'a Storage, request: ToolInvocationRequest) -> Self {
        Self {
            storage,
            request: Some(request),
        }
    }

    pub(crate) fn complete(
        mut self,
        result: serde_json::Value,
        now: i64,
    ) -> Result<(), RuntimeError> {
        let request = self.request.as_ref().expect("active tool invocation");
        self.storage.finish_tool_invocation(request, result, now)?;
        self.request = None;
        Ok(())
    }

    pub(crate) fn fail(mut self, code: &str, now: i64) -> Result<(), RuntimeError> {
        let request = self.request.as_ref().expect("active tool invocation");
        self.storage.fail_tool_invocation(request, code, now)?;
        self.request = None;
        Ok(())
    }
}

impl Drop for ToolInvocationGuard<'_> {
    fn drop(&mut self) {
        let Some(request) = self.request.take() else {
            return;
        };
        if let Err(error) =
            self.storage
                .fail_tool_invocation(&request, "invocation_cancelled", now_millis())
        {
            eprintln!(
                "[runtime] execution={} attempt={} tool_call={} finalization failed: {error}",
                request.execution_id, request.attempt, request.tool_call_id
            );
        }
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    Storage(StorageError),
    Provider {
        workspace_id: WorkspaceId,
        source: ProviderError,
    },
    ProviderCall(ProviderError),
    DuplicateWorkspace(WorkspaceId),
    DuplicateAgent {
        workspace_id: WorkspaceId,
        agent_id: AgentId,
    },
    UnknownWorkspace(WorkspaceId),
    WorkspacePaused(WorkspaceId),
    ProviderNotStarted(WorkspaceId),
    UnknownAgent {
        workspace_id: WorkspaceId,
        agent_id: AgentId,
    },
    UnknownChannel {
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
    },
    InvalidCore(String),
    UnknownSend(String),
    RuntimeLockPoisoned,
    DuplicateProviderType(ProviderId),
    UnknownProviderType(ProviderId),
    StorageStillShared,
    ApplicationStopped,
    RuntimeStillShared,
    Application(String),
    ActionRequired(String),
    InvalidExecutionToken,
    ProviderMismatch {
        workspace_id: WorkspaceId,
        expected: ProviderId,
        actual: ProviderId,
    },
    InvalidProviderSchema(serde_json::Error),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "runtime_storage: {error}"),
            Self::Provider {
                workspace_id,
                source,
            } => {
                write!(formatter, "runtime_provider[{workspace_id}]: {source}")
            }
            Self::ProviderCall(error) => write!(formatter, "runtime_provider_call: {error}"),
            Self::DuplicateWorkspace(id) => write!(formatter, "runtime_duplicate_workspace: {id}"),
            Self::DuplicateAgent {
                workspace_id,
                agent_id,
            } => write!(
                formatter,
                "runtime_duplicate_agent[{workspace_id}]: {agent_id}"
            ),
            Self::UnknownWorkspace(id) => write!(formatter, "runtime_unknown_workspace: {id}"),
            Self::WorkspacePaused(id) => write!(formatter, "runtime_workspace_paused: {id}"),
            Self::ProviderNotStarted(id) => {
                write!(formatter, "runtime_provider_not_started: {id}")
            }
            Self::UnknownAgent {
                workspace_id,
                agent_id,
            } => write!(
                formatter,
                "runtime_unknown_agent[{workspace_id}]: {agent_id}"
            ),
            Self::UnknownChannel {
                workspace_id,
                channel_id,
            } => write!(
                formatter,
                "runtime_unknown_channel[{workspace_id}]: {channel_id}"
            ),
            Self::InvalidCore(error) => write!(formatter, "runtime_invalid_core: {error}"),
            Self::UnknownSend(key) => write!(formatter, "runtime_unknown_send: {key}"),
            Self::RuntimeLockPoisoned => formatter.write_str("runtime_lock_poisoned"),
            Self::DuplicateProviderType(id) => {
                write!(formatter, "runtime_duplicate_provider: {id}")
            }
            Self::UnknownProviderType(id) => write!(formatter, "runtime_unknown_provider: {id}"),
            Self::StorageStillShared => formatter.write_str("runtime_storage_still_shared"),
            Self::ApplicationStopped => formatter.write_str("runtime_application_stopped"),
            Self::RuntimeStillShared => formatter.write_str("runtime_still_shared"),
            Self::Application(error) => write!(formatter, "runtime_application: {error}"),
            Self::ActionRequired(message) => formatter.write_str(message),
            Self::InvalidExecutionToken => formatter.write_str("runtime_invalid_execution_token"),
            Self::ProviderMismatch {
                workspace_id,
                expected,
                actual,
            } => write!(
                formatter,
                "runtime_provider_mismatch[{workspace_id}]: expected {expected}, got {actual}"
            ),
            Self::InvalidProviderSchema(error) => {
                write!(formatter, "runtime_invalid_provider_schema: {error}")
            }
        }
    }
}
impl std::error::Error for RuntimeError {}
impl From<StorageError> for RuntimeError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}
impl From<ProviderError> for RuntimeError {
    fn from(value: ProviderError) -> Self {
        Self::ProviderCall(value)
    }
}

struct ProviderInstance {
    active: AtomicBool,
    factory: ProviderFactory,
    host: Mutex<Option<Arc<dyn ProviderHostInstance>>>,
    channels: RwLock<Vec<ChannelDefinition>>,
    agents: RwLock<BTreeMap<AgentId, AgentRuntimeConfig>>,
}

pub type ProviderFactory =
    Arc<dyn Fn() -> Result<Arc<dyn ProviderHostInstance>, ProviderError> + Send + Sync>;

fn protocol_tool(value: ToolDefinition) -> Result<ProtocolToolDefinition, RuntimeError> {
    Ok(ProtocolToolDefinition {
        name: value.id,
        description: value.description,
        input_schema_json: serde_json::to_string(value.input_schema.value())
            .map_err(RuntimeError::InvalidProviderSchema)?,
        output_schema_json: serde_json::to_string(value.output_schema.value())
            .map_err(RuntimeError::InvalidProviderSchema)?,
    })
}

fn protocol_cli(value: CliCommandDefinition) -> CliCommand {
    CliCommand {
        name: value.id,
        description: value.description,
        argv: Vec::new(),
    }
}

fn sender_kind(value: EventSenderType) -> MessageAuthorType {
    match value {
        EventSenderType::Human => MessageAuthorType::Human,
        EventSenderType::Agent => MessageAuthorType::Agent,
        EventSenderType::Bot => MessageAuthorType::Bot,
        EventSenderType::System => MessageAuthorType::System,
    }
}

fn resource_view(value: &lankide_protocol::ResourceRef) -> Result<ResourceView, RuntimeError> {
    Ok(ResourceView {
        resource_id: ResourceId::new(value.resource_id)
            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
        file_name: value.file_name.clone(),
        media_type: value.media_type.clone(),
        size_bytes: value.size_bytes,
        sha256: value.sha256.clone(),
    })
}

fn message_dispatches(
    channel: &ChannelDefinition,
    agents: &BTreeMap<AgentId, AgentRuntimeConfig>,
    chat_attention: &BTreeMap<AgentId, AttentionPolicy>,
    sender: &lankide_provider::EventSender,
    message: &lankide_provider::MessageEvent,
    now: i64,
) -> Result<Vec<InboxDispatch>, RuntimeError> {
    let mut dispatches = Vec::new();
    for agent in agents.values() {
        if sender.kind == EventSenderType::Agent && sender.id == agent.provider_sender_id {
            continue;
        }
        let default_policy = match channel.default_attention {
            ChannelDefaultAttention::DirectMessage | ChannelDefaultAttention::Follow => {
                AttentionPolicy::Always
            }
            ChannelDefaultAttention::Mention => AttentionPolicy::Mention,
            ChannelDefaultAttention::Mute => AttentionPolicy::Mute,
        };
        let policy = chat_attention
            .get(&agent.agent_id)
            .or_else(|| agent.attention.get(&channel.id))
            .unwrap_or(&default_policy);
        if policy == &AttentionPolicy::Mute {
            continue;
        }
        let mentioned = message.mentioned_agent_ids.contains(&agent.agent_id)
            && (channel.attention_levels.contains(&AttentionLevel::Mention)
                || channel.default_attention == ChannelDefaultAttention::Mention);
        let (available_at, reason) =
            if channel.default_attention == ChannelDefaultAttention::DirectMessage {
                if policy != &AttentionPolicy::Always {
                    continue;
                }
                (now, DispatchReason::DirectMessage)
            } else if mentioned {
                (now, DispatchReason::Mention)
            } else if matches!(sender.kind, EventSenderType::Agent | EventSenderType::Bot) {
                continue;
            } else {
                match policy {
                    AttentionPolicy::Mute => unreachable!("mute policy handled above"),
                    AttentionPolicy::Mention => continue,
                    AttentionPolicy::Always
                        if channel.attention_levels.contains(&AttentionLevel::Follow)
                            || channel.default_attention == ChannelDefaultAttention::Follow =>
                    {
                        (now, DispatchReason::Follow)
                    }
                    AttentionPolicy::Always => continue,
                    AttentionPolicy::Schedule(interval)
                        if channel.attention_levels.contains(&AttentionLevel::Schedule) =>
                    {
                        let boundary =
                            interval
                                .next_boundary(UnixMillis::new(now).map_err(|error| {
                                    RuntimeError::InvalidCore(error.to_string())
                                })?)
                                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                        (boundary.get(), DispatchReason::Schedule)
                    }
                    AttentionPolicy::Schedule(_) => continue,
                }
            };
        dispatches.push(InboxDispatch {
            agent_id: agent.agent_id.clone(),
            available_at,
            trigger_reason: reason,
        });
    }
    Ok(dispatches)
}

fn attention_policy(
    record: &lankide_storage::AttentionRecord,
) -> Result<AttentionPolicy, RuntimeError> {
    Ok(match record {
        lankide_storage::AttentionRecord::Mention => AttentionPolicy::Mention,
        lankide_storage::AttentionRecord::Always => AttentionPolicy::Always,
        lankide_storage::AttentionRecord::Mute => AttentionPolicy::Mute,
        lankide_storage::AttentionRecord::Schedule { interval_minutes } => {
            AttentionPolicy::Schedule(
                ScheduleInterval::try_from(*interval_minutes)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
            )
        }
    })
}

fn reaction_dispatches(
    agents: &BTreeMap<AgentId, AgentRuntimeConfig>,
    sender: &lankide_provider::EventSender,
    reaction: &str,
    now: i64,
) -> Vec<InboxDispatch> {
    agents
        .values()
        .filter(|agent| {
            !(sender.kind == EventSenderType::Agent && sender.id == agent.provider_sender_id)
                && !agent.muted()
        })
        .map(|agent| InboxDispatch {
            agent_id: agent.agent_id.clone(),
            available_at: now,
            trigger_reason: DispatchReason::Reaction {
                reaction: reaction.to_owned(),
            },
        })
        .collect()
}

fn trigger_summary(snapshot: &SnapshotView) -> String {
    let mut values = trigger_reasons(snapshot);
    values.dedup();
    values.join(",")
}

fn trigger_reasons(snapshot: &SnapshotView) -> Vec<String> {
    snapshot
        .chats
        .iter()
        .flat_map(|chat| chat.trigger_reasons.iter())
        .map(trigger_reason_name)
        .collect()
}

fn trigger_reason_name(reason: &DispatchReason) -> String {
    match reason {
        DispatchReason::DirectMessage => "direct_message".to_owned(),
        DispatchReason::Mention => "mention".to_owned(),
        DispatchReason::Follow => "follow".to_owned(),
        DispatchReason::Schedule => "schedule".to_owned(),
        DispatchReason::Reaction { reaction } => format!("reaction:{reaction}"),
        DispatchReason::Custom { name, .. } => format!("custom:{name}"),
    }
}

fn snapshot_json(snapshot: &SnapshotView) -> serde_json::Value {
    serde_json::json!({
        "snapshot_id": snapshot.snapshot_id,
        "workspace_id": snapshot.workspace_id.as_str(),
        "agent_id": snapshot.agent_id.as_str(),
        "created_at": snapshot.created_at,
        "chats": snapshot.chats.iter().map(|chat| serde_json::json!({
            "provider_id": chat.chat.provider_id.as_str(),
            "channel_id": chat.chat.channel_id.as_str(),
            "channel_name": chat.chat.channel_name,
            "chat_id": chat.chat.provider_chat_id,
            "chat_name": chat.chat.chat_name,
            "unread_count": chat.unread_count,
            "read_to_message_id": chat.read_to_message_id,
            "recent_messages": chat.recent_messages.iter().map(message_preview_json).collect::<Vec<_>>(),
            "mentioned_message": chat.mentioned_message.as_ref().map(message_preview_json),
            "trigger_messages": chat.trigger_messages.iter().map(message_preview_json).collect::<Vec<_>>(),
            "trigger_reasons": chat.trigger_reasons.iter().map(|reason| serde_json::to_value(reason).unwrap_or(serde_json::Value::Null)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn message_preview_json(message: &lankide_storage::MessageView) -> serde_json::Value {
    let mut value = message_json(message);
    value["content"] = serde_json::Value::String(truncate_message(&message.content));
    value
}

fn message_json(message: &lankide_storage::MessageView) -> serde_json::Value {
    let sender_kind = match message.sender_kind {
        MessageAuthorType::Human => "human",
        MessageAuthorType::Agent => "agent",
        MessageAuthorType::Bot => "bot",
        MessageAuthorType::System => "system",
    };
    serde_json::json!({
        "message_id": message.message_id,
        "sender": {
            "kind": sender_kind,
            "id": message.sender_id,
            "name": message.sender_name,
        },
        "content": message.content,
        "resources": message.resources.iter().map(|resource| serde_json::json!({
            "resource_id": resource.resource_id.get(),
            "file_name": resource.file_name,
            "media_type": resource.media_type,
            "size_bytes": resource.size_bytes,
            "sha256": resource.sha256,
        })).collect::<Vec<_>>(),
        "mentions": message.mentions.iter().map(AgentId::as_str).collect::<Vec<_>>(),
        "occurred_at": message.occurred_at,
    })
}

fn render_snapshot(snapshot: &SnapshotView) -> String {
    let mut markdown = String::from("未读事项：");
    for chat in &snapshot.chats {
        let kind = match chat.chat.channel_id.as_str() {
            "group_chat" => "群聊",
            "private_chat" => "私聊",
            "task" => "任务",
            _ => chat.chat.channel_name.as_str(),
        };
        let name = if chat.chat.chat_name.trim().is_empty() {
            chat.chat.provider_chat_id.as_str()
        } else {
            chat.chat.chat_name.as_str()
        };
        markdown.push_str("\n\n### ");
        markdown.push_str(kind);
        markdown.push('：');
        markdown.push_str(name);
        markdown.push_str("（channel_id=");
        markdown.push_str(chat.chat.channel_id.as_str());
        markdown.push_str("，chat_id=");
        markdown.push_str(&chat.chat.provider_chat_id);
        markdown.push('）');
        for message in &chat.recent_messages {
            markdown.push_str("\n- [message_id=");
            markdown.push_str(&message.message_id.to_string());
            markdown.push_str("] ");
            if message.sender_name.trim().is_empty() {
                markdown.push_str(&message.sender_id);
            } else {
                markdown.push_str(&message.sender_name);
            }
            markdown.push_str(": ");
            markdown.push_str(&truncate_message(&message.content));
        }
        if chat.unread_count > chat.recent_messages.len() {
            markdown.push_str("\n- 还有更早的未读内容，需要时使用 chat_messages 继续查看。");
        }
        if let Some(mentioned) = chat.mentioned_message.as_ref()
            && !chat
                .recent_messages
                .iter()
                .any(|message| message.message_id == mentioned.message_id)
        {
            markdown.push_str("\n\n有人 @ 你：[message_id=");
            markdown.push_str(&mentioned.message_id.to_string());
            markdown.push_str("] ");
            markdown.push_str(&truncate_message(&mentioned.content));
        }
    }
    markdown.push_str("\n\n<system-reminder>以上是本轮冻结的未读会话。每个会话只展示最近几条内容；需要完整上下文时，用 chat_messages 主动读取。不要假装已经看过未展示的内容。</system-reminder>");
    markdown
}

fn runtime_chats(snapshot: &SnapshotView) -> Vec<lankide_provider::AgentRuntimeChat> {
    snapshot
        .chats
        .iter()
        .map(|chat| lankide_provider::AgentRuntimeChat {
            provider_id: chat.chat.provider_id.clone(),
            channel_id: chat.chat.channel_id.clone(),
            channel_name: chat.chat.channel_name.clone(),
            chat_id: chat.chat.provider_chat_id.clone(),
            chat_name: chat.chat.chat_name.clone(),
        })
        .collect()
}

fn frozen_runtime_spec_json(spec: &AgentRuntimeSpec) -> serde_json::Value {
    serde_json::json!({
        "agent_config": spec.agent_config,
        "provider_agent_config": spec.provider_agent_config,
        "computer_id": spec.computer_id,
        "harness_id": spec.harness_id,
        "model": spec.model,
        "thinking_level": spec.thinking_level,
        "trigger_reason": spec.trigger_reason,
        "chats": spec.chats.iter().map(|chat| serde_json::json!({
            "provider_id": chat.provider_id.as_str(),
            "channel_id": chat.channel_id.as_str(),
            "channel_name": chat.channel_name,
            "chat_id": chat.chat_id,
            "chat_name": chat.chat_name,
        })).collect::<Vec<_>>(),
        "unread_snapshot": spec.unread_snapshot,
    })
}

fn frozen_runtime_spec(value: &str) -> Result<AgentRuntimeSpec, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| RuntimeError::Application(format!("invalid frozen context: {error}")))?;
    let text = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| RuntimeError::Application(format!("frozen context missing {name}")))
    };
    let chats = value
        .get("chats")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| RuntimeError::Application("frozen context missing chats".into()))?
        .iter()
        .map(|chat| {
            let field = |name: &str| {
                chat.get(name)
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| RuntimeError::Application(format!("frozen chat missing {name}")))
            };
            Ok::<lankide_provider::AgentRuntimeChat, RuntimeError>(
                lankide_provider::AgentRuntimeChat {
                    provider_id: ProviderId::new(field("provider_id")?)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                    channel_id: ChannelId::new(field("channel_id")?)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                    channel_name: field("channel_name")?.to_owned(),
                    chat_id: field("chat_id")?.to_owned(),
                    chat_name: field("chat_name")?.to_owned(),
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AgentRuntimeSpec {
        agent_config: value.get("agent_config").cloned().unwrap_or_default(),
        provider_agent_config: value
            .get("provider_agent_config")
            .cloned()
            .unwrap_or_default(),
        computer_id: text("computer_id")?,
        harness_id: text("harness_id")?,
        model: text("model")?,
        thinking_level: text("thinking_level")?,
        trigger_reason: text("trigger_reason")?,
        chats,
        unread_snapshot: value.get("unread_snapshot").cloned().unwrap_or_default(),
    })
}

fn lowercase_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn provider_send_idempotency_key(namespace: &str, request_key: &str) -> String {
    lowercase_sha256(format!("{namespace}\0{request_key}").as_bytes())
}

fn truncate_message(content: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut chars = content.chars();
    let truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        let mut truncated = truncated;
        truncated.pop();
        truncated.push('…');
        truncated
    } else {
        truncated
    }
}

fn snapshot_resources(snapshot: &SnapshotView) -> Vec<ResourceRef> {
    let mut resources = BTreeMap::new();
    for message in snapshot
        .chats
        .iter()
        .flat_map(|chat| chat.recent_messages.iter().chain(&chat.trigger_messages))
    {
        for resource in &message.resources {
            resources
                .entry(resource.resource_id.get())
                .or_insert_with(|| ResourceRef {
                    resource_id: resource.resource_id.get(),
                    file_name: resource.file_name.clone(),
                    media_type: resource.media_type.clone(),
                    size_bytes: resource.size_bytes,
                    sha256: resource.sha256.clone(),
                });
        }
    }
    resources.into_values().collect()
}

fn execution_id_from_request_key(request_key: &str) -> Result<i64, RuntimeError> {
    let value = request_key
        .strip_prefix("execution:")
        .and_then(|value| value.strip_suffix(":reply"))
        .ok_or_else(|| RuntimeError::UnknownSend(request_key.to_owned()))?;
    value
        .parse()
        .map_err(|_| RuntimeError::UnknownSend(request_key.to_owned()))
}

fn generate_execution_token() -> Result<String, RuntimeError> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| RuntimeError::Application(format!("execution token: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
