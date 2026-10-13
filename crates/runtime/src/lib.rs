#![forbid(unsafe_code)]

//! Runtime orchestration. It owns scheduling, persistence, and Provider
//! instance lifecycles; concrete adapters are assembled by application roots.

mod agent_profile;
mod application;
mod capability;
mod computer_broker;
mod coordination;
mod error;
mod execution;
mod lifecycle;
mod management;
mod peer;
mod prompt;
mod provider_ops;
mod provider_registry;
mod resource_store;
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
}
