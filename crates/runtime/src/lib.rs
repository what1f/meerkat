#![forbid(unsafe_code)]

//! Runtime orchestration. It owns scheduling, persistence, and Provider
//! instance lifecycles; concrete adapters are assembled by application roots.

mod agent_profile;
mod application;
mod capability;
mod capability_link;
mod computer_broker;
mod coordination;
mod error;
mod execution;
mod lifecycle;
mod management;
mod pairing;
mod peer;
mod prompt;
mod provider_ops;
mod provider_registry;
mod resource_store;
mod retry;
pub use application::*;
pub use computer_broker::*;
pub use coordination::{AttentionPolicy, CoordinatedEvent};
pub use error::RuntimeError;
pub(crate) use execution::token_hash;
pub use execution::{
    ComputerCompletion, ComputerError, ComputerExecutor, ComputerFuture, ComputerOutcome,
    DynamicExecutionInput, ExecutionProgress, ExecutionReply, ToolProgress,
};
use execution::{
    ToolInvocationGuard, lowercase_sha256, message_json, provider_send_idempotency_key,
};
use lifecycle::ProviderInstance;
pub use lifecycle::{AgentRuntimeConfig, ProviderFactory};
pub use peer::*;
pub use provider_ops::ValidatedRunContext;
pub use provider_registry::{ProviderFactoryFuture, ProviderFactoryRegistry, ProviderTypeFactory};
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
pub use lankide_provider::{ProviderOnboarding, ProviderOnboardingStatus};
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
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};
use tokio::sync::{Mutex, RwLock};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub database: DatabaseConfig,
    #[serde(default)]
    pub retry: RetryConfig,
}

pub struct Runtime {
    config: RuntimeConfig,
    execution_token_lifetime_ms: AtomicI64,
    storage: Arc<Storage>,
    background_tasks: std::sync::Mutex<tokio::task::JoinSet<()>>,
    providers: StdRwLock<BTreeMap<WorkspaceId, Arc<ProviderInstance>>>,
}

impl Runtime {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        if config.retry.base_ms <= 0
            || config.retry.max_ms < config.retry.base_ms
            || config.retry.notice_dedupe_ms <= 0
            || config.retry.poll_ms == 0
        {
            return Err(RuntimeError::Application(
                "invalid retry policy durations".into(),
            ));
        }
        let storage = Arc::new(Storage::open(config.database.clone())?);
        Ok(Self {
            config,
            execution_token_lifetime_ms: AtomicI64::new(3_600_000),
            storage,
            background_tasks: std::sync::Mutex::new(tokio::task::JoinSet::new()),
            providers: StdRwLock::new(BTreeMap::new()),
        })
    }

    /// Configure the authorization lifetime for newly issued attempts.
    pub fn set_execution_token_lifetime(
        &self,
        duration: std::time::Duration,
    ) -> Result<(), RuntimeError> {
        let millis = i64::try_from(duration.as_millis())
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| RuntimeError::Application("invalid execution token lifetime".into()))?;
        self.execution_token_lifetime_ms
            .store(millis, Ordering::Relaxed);
        Ok(())
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
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct RetryConfig {
    pub base_ms: i64,
    pub max_ms: i64,
    pub notice_dedupe_ms: i64,
    pub poll_ms: u64,
}
impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            base_ms: 30_000,
            max_ms: 600_000,
            notice_dedupe_ms: 900_000,
            poll_ms: 1000,
        }
    }
}
