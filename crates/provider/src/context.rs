use crate::{InterfaceDefinition, ProviderError, ProviderEvent, ProviderFuture, SecretValue};
use lankide_core::{AgentId, ChannelId, MessageId, PersonId, ProviderId, ResourceRef, WorkspaceId};
use serde_json::Value;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc, oneshot};

pub trait SecretReader: Send + Sync {
    fn read<'a>(&'a self, name: &'a str) -> ProviderFuture<'a, SecretValue>;
}
pub trait SecretWriter: Send + Sync {
    fn write<'a>(&'a self, name: &'a str, value: SecretValue) -> ProviderFuture<'a, ()>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DeniedSecretWriter;
impl SecretWriter for DeniedSecretWriter {
    fn write<'a>(&'a self, _: &'a str, _: SecretValue) -> ProviderFuture<'a, ()> {
        Box::pin(async { Err(ProviderError::SecretAccessDenied) })
    }
}
pub trait ProviderStorage: Send + Sync {
    /// Invalidate the current workspace's management data after a Provider write.
    fn management_changed(&self) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }

    fn get<'a>(&'a self, key: &'a str) -> ProviderFuture<'a, Option<Vec<u8>>>;
    fn put<'a>(&'a self, key: &'a str, value: Vec<u8>) -> ProviderFuture<'a, ()>;
    fn workspace_state<'a>(&'a self) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            self.get("__workspace_state")
                .await?
                .map(|bytes| {
                    serde_json::from_slice(&bytes).map_err(|_| ProviderError::InvalidInput {
                        field: "workspace_provider_state",
                    })
                })
                .transpose()
                .map(|state| state.unwrap_or_else(|| Value::Object(Default::default())))
        })
    }
    fn put_workspace_state<'a>(&'a self, state: Value) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let bytes = serde_json::to_vec(&state).map_err(|_| ProviderError::InvalidInput {
                field: "workspace_provider_state",
            })?;
            self.put("__workspace_state", bytes).await
        })
    }
    /// Read this Workspace's existing Agent Provider configuration, without secrets.
    fn agent_config<'a>(&'a self, _: &'a AgentId) -> ProviderFuture<'a, Value> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "read_agent_provider_config",
            })
        })
    }
    fn agent_state<'a>(&'a self, agent_id: &'a AgentId) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let key = format!("__agent_state/{}", agent_id.as_str());
            self.get(&key)
                .await?
                .map(|bytes| {
                    serde_json::from_slice(&bytes).map_err(|_| ProviderError::InvalidInput {
                        field: "agent_provider_state",
                    })
                })
                .transpose()
                .map(|state| state.unwrap_or_else(|| Value::Object(Default::default())))
        })
    }
    fn put_agent_state<'a>(
        &'a self,
        agent_id: &'a AgentId,
        state: Value,
    ) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let bytes = serde_json::to_vec(&state).map_err(|_| ProviderError::InvalidInput {
                field: "agent_provider_state",
            })?;
            let key = format!("__agent_state/{}", agent_id.as_str());
            self.put(&key, bytes).await
        })
    }
    /// Resolve one Runtime-owned Message through its internal numeric ID.
    /// Provider implementations use this for native replies and other
    /// transport operations without receiving a copied map in Provider config.
    fn message<'a>(&'a self, _: MessageId) -> ProviderFuture<'a, ProviderMessage> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "read_runtime_message",
            })
        })
    }
    /// Read bytes for a Runtime-owned resource. The Provider receives no local
    /// filesystem path and remains independent from Runtime storage layout.
    fn resource<'a>(&'a self, _: &'a ResourceRef) -> ProviderFuture<'a, ProviderResource> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "read_runtime_resource",
            })
        })
    }
    /// Resolve a Provider-stable human identifier to the compact Runtime ID.
    /// The Runtime creates the row on first observation; Providers never
    /// invent or persist `person_id` values themselves.
    fn get_or_create_person<'a>(
        &'a self,
        _: &'a str,
    ) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "resolve_person_identity",
            })
        })
    }
    fn person<'a>(&'a self, _: &'a PersonId) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "read_person_identity",
            })
        })
    }
    /// List the current Workspace's Team roster. Human people with the
    /// `external` role and deleted Agents are intentionally excluded; Runtime
    /// remains the owner of Team membership.
    fn team_members(&self) -> ProviderFuture<'_, Vec<ProviderTeamMember>> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "list_team_member_identities",
            })
        })
    }
    /// Replace opaque Provider identity state only when the observed revision
    /// still matches. Team membership is Runtime-owned and cannot be changed
    /// through this method.
    fn compare_and_set_person_state<'a>(
        &'a self,
        _: &'a PersonId,
        _: i64,
        _: Value,
    ) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "update_person_identity_state",
            })
        })
    }
    /// Save a file for the current Agent run and return the path that Agent
    /// tools may consume. The Runtime owns allocation and path validation.
    fn save_agent_file<'a>(&'a self, _: ResourceUpload) -> ProviderFuture<'a, ProviderAgentFile> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "save_agent_file",
            })
        })
    }
}

pub use lankide_core::TeamRole;

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderPersonIdentity {
    pub person_id: PersonId,
    pub external_id: String,
    pub team_role: TeamRole,
    pub provider_state: Value,
    pub state_revision: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderTeamMember {
    Human(ProviderPersonIdentity),
    Agent { agent_id: AgentId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAgentFile {
    pub resource: ResourceRef,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderMessage {
    pub message_id: MessageId,
    pub provider_message_id: Option<String>,
    pub channel_id: ChannelId,
    pub chat_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderResource {
    pub resource: ResourceRef,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}
pub trait ProviderLogger: Send + Sync {
    fn log(&self, level: LogLevel, message: &str);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationUpdate {
    pub stage: String,
    pub completed: u32,
    pub total: Option<u32>,
}
pub trait OperationProgress: Send + Sync {
    fn report<'a>(&'a self, update: OperationUpdate) -> ProviderFuture<'a, ()>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceUpload {
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}
pub trait ResourceSaver: Send + Sync {
    fn save<'a>(&'a self, value: ResourceUpload) -> ProviderFuture<'a, ResourceRef>;
}

#[derive(Clone)]
pub struct HostServices {
    pub secret_reader: Arc<dyn SecretReader>,
    pub secret_writer: Arc<dyn SecretWriter>,
    pub storage: Arc<dyn ProviderStorage>,
    pub logger: Arc<dyn ProviderLogger>,
    pub resource_saver: Arc<dyn ResourceSaver>,
    pub operation_progress: Arc<dyn OperationProgress>,
    pub event_capacity: usize,
    pub invocation_timeout: Duration,
}
impl fmt::Debug for HostServices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostServices")
            .field("secret_reader", &"[REDACTED]")
            .field("secret_writer", &"[REDACTED]")
            .field("event_capacity", &self.event_capacity)
            .field("invocation_timeout", &self.invocation_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct ShutdownSignal {
    pub(crate) stopped: Arc<AtomicBool>,
    pub(crate) notify: Arc<Notify>,
}
impl ShutdownSignal {
    pub(crate) fn new() -> Self {
        Self {
            stopped: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
        }
    }
    pub fn is_shutdown(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        if self.is_shutdown() {
            return;
        }
        let n = self.notify.notified();
        if !self.is_shutdown() {
            n.await;
        }
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }
}
impl fmt::Debug for ShutdownSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShutdownSignal")
            .field("is_shutdown", &self.is_shutdown())
            .finish()
    }
}

#[derive(Clone)]
pub struct EventSink {
    pub(crate) sender: mpsc::Sender<EventDelivery>,
    pub(crate) shutdown: ShutdownSignal,
}
impl EventSink {
    pub async fn emit(&self, event: ProviderEvent) -> Result<(), ProviderError> {
        if self.shutdown.is_shutdown() {
            return Err(ProviderError::EventSinkClosed);
        }
        let permit = tokio::select! {
            _ = self.shutdown.cancelled() => return Err(ProviderError::EventSinkClosed),
            permit = self.sender.reserve() => permit.map_err(|_| ProviderError::EventSinkClosed)?,
        };
        if self.shutdown.is_shutdown() {
            return Err(ProviderError::EventSinkClosed);
        }
        let (accepted, receipt) = oneshot::channel();
        permit.send(EventDelivery {
            event,
            acknowledgement: EventAcknowledgement(accepted),
        });
        tokio::select! {
            _ = self.shutdown.cancelled() => Err(ProviderError::EventSinkClosed),
            result = receipt => result.unwrap_or(Err(ProviderError::EventSinkClosed)),
        }
    }
}

/// One normalized event whose sender is still waiting for durable acceptance.
#[derive(Debug)]
pub struct EventDelivery {
    event: ProviderEvent,
    acknowledgement: EventAcknowledgement,
}

impl EventDelivery {
    pub fn into_parts(self) -> (ProviderEvent, EventAcknowledgement) {
        (self.event, self.acknowledgement)
    }
}

/// Complete only after the receiving module has committed the event.
/// Dropping this receipt rejects delivery, allowing the Provider to retry.
#[must_use]
#[derive(Debug)]
pub struct EventAcknowledgement(oneshot::Sender<Result<(), ProviderError>>);

impl EventAcknowledgement {
    pub fn accept(self) {
        let _ = self.0.send(Ok(()));
    }

    pub fn reject(self) {
        let _ = self.0.send(Err(ProviderError::External {
            operation: "accept_event",
            retryable: true,
        }));
    }
}
impl fmt::Debug for EventSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventSink")
            .field("closed", &self.shutdown.is_shutdown())
            .finish()
    }
}

pub(crate) trait InterfaceRegistrar: Send + Sync {
    fn register<'a>(&'a self, definition: InterfaceDefinition) -> ProviderFuture<'a, ()>;
}

#[derive(Clone)]
pub struct InitContext {
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) workspace_config: Value,
    pub(crate) secret_reader: Arc<dyn SecretReader>,
    pub(crate) secret_writer: Arc<dyn SecretWriter>,
    pub(crate) storage: Arc<dyn ProviderStorage>,
    pub(crate) logger: Arc<dyn ProviderLogger>,
    pub(crate) shutdown: ShutdownSignal,
    pub(crate) sink: EventSink,
    pub(crate) registrar: Arc<dyn InterfaceRegistrar>,
    pub(crate) resource_saver: Arc<dyn ResourceSaver>,
}
impl InitContext {
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn workspace_config(&self) -> &Value {
        &self.workspace_config
    }
    pub fn secret_reader(&self) -> &dyn SecretReader {
        self.secret_reader.as_ref()
    }
    pub fn secret_writer(&self) -> &dyn SecretWriter {
        self.secret_writer.as_ref()
    }
    pub fn storage(&self) -> &dyn ProviderStorage {
        self.storage.as_ref()
    }
    pub fn logger(&self) -> &dyn ProviderLogger {
        self.logger.as_ref()
    }
    pub fn shutdown_signal(&self) -> &ShutdownSignal {
        &self.shutdown
    }
    pub fn event_sink(&self) -> &EventSink {
        &self.sink
    }
    pub async fn register_interface(
        &self,
        definition: InterfaceDefinition,
    ) -> Result<(), ProviderError> {
        self.registrar.register(definition).await
    }
    pub async fn save_resource(&self, value: ResourceUpload) -> Result<ResourceRef, ProviderError> {
        self.resource_saver.save(value).await
    }
}
impl fmt::Debug for InitContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitContext")
            .field("workspace_id", &self.workspace_id)
            .field("workspace_config", &"[REDACTED]")
            .field("secret_reader", &"[REDACTED]")
            .field("secret_writer", &"[REDACTED]")
            .field("shutdown", &self.shutdown)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceSpec {
    pub name: String,
    pub cover: Option<String>,
    pub rules: String,
    pub provider_config: Value,
}
#[derive(Clone)]
pub struct WorkspaceContext {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub cover: Option<String>,
    pub rules: String,
    pub old_provider_config: Option<Value>,
    pub new_provider_config: Value,
    pub(crate) secrets: Arc<dyn SecretReader>,
    pub(crate) storage: Arc<dyn ProviderStorage>,
    pub(crate) progress: Arc<dyn OperationProgress>,
}
impl WorkspaceContext {
    pub fn provider_secrets(&self) -> &dyn SecretReader {
        self.secrets.as_ref()
    }
    pub fn operation_progress(&self) -> &dyn OperationProgress {
        self.progress.as_ref()
    }
    pub fn storage(&self) -> &dyn ProviderStorage {
        self.storage.as_ref()
    }
}
impl fmt::Debug for WorkspaceContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceContext")
            .field("workspace_id", &self.workspace_id)
            .field("name", &self.name)
            .field("provider_config", &"[REDACTED]")
            .field("provider_secrets", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}
impl fmt::Debug for EnvironmentVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvironmentVariable")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct AgentSpec {
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Option<String>,
    pub soul: String,
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    pub auto_reaction: Option<String>,
    pub cli_path: Option<String>,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub provider_agent_config: Value,
}
#[derive(Clone)]
pub struct AgentContext {
    pub workspace_id: WorkspaceId,
    pub workspace_config: Value,
    pub agent_id: AgentId,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Option<String>,
    pub soul: String,
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    pub auto_reaction: Option<String>,
    pub cli_path: Option<String>,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub old_provider_agent_config: Option<Value>,
    pub new_provider_agent_config: Value,
    pub(crate) secrets: Arc<dyn SecretReader>,
    pub(crate) storage: Arc<dyn ProviderStorage>,
    pub(crate) progress: Arc<dyn OperationProgress>,
}
impl AgentContext {
    pub fn provider_secrets(&self) -> &dyn SecretReader {
        self.secrets.as_ref()
    }
    pub fn operation_progress(&self) -> &dyn OperationProgress {
        self.progress.as_ref()
    }
    pub fn storage(&self) -> &dyn ProviderStorage {
        self.storage.as_ref()
    }
}
impl fmt::Debug for AgentContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentContext")
            .field("workspace_id", &self.workspace_id)
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("provider_config", &"[REDACTED]")
            .field("provider_secrets", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct AgentOnboardingContext {
    pub workspace_id: WorkspaceId,
    pub workspace_config: Value,
    pub agent_id: AgentId,
    pub name: String,
    pub provider_agent_config: Value,
    secret_reader: Arc<dyn SecretReader>,
    secret_writer: Arc<dyn SecretWriter>,
    storage: Arc<dyn ProviderStorage>,
    resource_saver: Arc<dyn ResourceSaver>,
}
impl AgentOnboardingContext {
    pub fn new(
        workspace_id: WorkspaceId,
        workspace_config: Value,
        agent_id: AgentId,
        name: String,
        provider_agent_config: Value,
        services: HostServices,
    ) -> Self {
        Self {
            workspace_id,
            workspace_config,
            agent_id,
            name,
            provider_agent_config,
            secret_reader: services.secret_reader,
            secret_writer: services.secret_writer,
            storage: services.storage,
            resource_saver: services.resource_saver,
        }
    }
    pub async fn save_resource(
        &self,
        upload: ResourceUpload,
    ) -> Result<ResourceRef, ProviderError> {
        self.resource_saver.save(upload).await
    }
    pub fn workspace_secrets(&self) -> &dyn SecretReader {
        self.secret_reader.as_ref()
    }
    pub fn workspace_secret_writer(&self) -> &dyn SecretWriter {
        self.secret_writer.as_ref()
    }
    pub async fn write_agent_secret(
        &self,
        name: &str,
        value: SecretValue,
    ) -> Result<(), ProviderError> {
        self.secret_writer
            .write(&format!("agent:{}:{name}", self.agent_id.as_str()), value)
            .await
    }
    pub fn storage(&self) -> &dyn ProviderStorage {
        self.storage.as_ref()
    }
}
impl fmt::Debug for AgentOnboardingContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentOnboardingContext")
            .field("workspace_id", &self.workspace_id)
            .field("agent_id", &self.agent_id)
            .field("name", &self.name)
            .field("provider_data", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRuntimeChat {
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub chat_id: String,
    pub chat_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRuntimeSpec {
    pub agent_config: Value,
    pub provider_agent_config: Value,
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    pub trigger_reason: String,
    pub chats: Vec<AgentRuntimeChat>,
    pub unread_snapshot: Value,
}
#[derive(Clone)]
pub struct AgentRuntimeContext {
    pub workspace_id: WorkspaceId,
    pub workspace_config: Value,
    pub agent_id: AgentId,
    pub agent_config: Value,
    pub provider_agent_config: Value,
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    pub trigger_reason: String,
    pub chats: Vec<AgentRuntimeChat>,
    pub unread_snapshot: Value,
    pub(crate) secrets: Arc<dyn SecretReader>,
    pub(crate) storage: Arc<dyn ProviderStorage>,
}
impl AgentRuntimeContext {
    pub fn provider_secrets(&self) -> &dyn SecretReader {
        self.secrets.as_ref()
    }
    pub fn storage(&self) -> &dyn ProviderStorage {
        self.storage.as_ref()
    }
}
impl fmt::Debug for AgentRuntimeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentRuntimeContext")
            .field("workspace_id", &self.workspace_id)
            .field("agent_id", &self.agent_id)
            .field("chats", &self.chats)
            .field("configs", &"[REDACTED]")
            .field("provider_secrets", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

macro_rules! provider_context {
    ($name:ident { $($field:ident : $ty:ty),+ $(,)? }) => {
        #[derive(Clone)] pub struct $name { $(pub $field: $ty,)+ pub(crate) secrets: Arc<dyn SecretReader>, pub(crate) storage: Arc<dyn ProviderStorage> }
        impl $name {
            pub fn provider_secrets(&self) -> &dyn SecretReader { self.secrets.as_ref() }
            pub fn storage(&self) -> &dyn ProviderStorage { self.storage.as_ref() }
        }
        impl fmt::Debug for $name { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct(stringify!($name)).field("sensitive_fields", &"[REDACTED]").finish_non_exhaustive()
        } }
    };
}
provider_context!(SendMessageContext {
    workspace_id: WorkspaceId, agent_id: AgentId, channel_id: ChannelId, chat_id: String,
    message_id: MessageId, idempotency_key: String, content: String, reply_to: Option<MessageId>, mentions: Vec<AgentId>,
    attachments: Vec<crate::MessageAttachment>, force: bool, provider_config: Value,
    provider_agent_config: Value
});
provider_context!(RecallMessageContext {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    channel_id: ChannelId,
    chat_id: String,
    message_id: MessageId,
    provider_message_id: String,
    provider_config: Value,
    provider_agent_config: Value
});
provider_context!(EditMessageContext {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    channel_id: ChannelId,
    chat_id: String,
    message_id: MessageId,
    provider_message_id: String,
    content: String,
    provider_config: Value,
    provider_agent_config: Value
});
provider_context!(AutoReactionContext {
    workspace_id: WorkspaceId, agent_id: AgentId, auto_reaction: String, channel_id: ChannelId,
    chat_id: String, trigger_messages: Vec<MessageId>, provider_config: Value,
    provider_agent_config: Value
});
