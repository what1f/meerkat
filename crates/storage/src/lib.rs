#![forbid(unsafe_code)]
//! Real SQLite/WAL persistence and Runtime transaction boundary.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};

mod computer;
pub use computer::ComputerRecord;
mod agent;
mod attempt;
mod error;
mod execution;
mod message;
mod person;
mod schema;
mod send;
mod workspace;

pub use agent::{AgentRecord, AgentSpec, AgentView, AttentionRecord};
pub use attempt::{
    AttemptAuthorization, AttemptCapability, ExecutionAttemptState, ExecutionAttemptView,
    ExecutionEventDisposition, ExecutionEventReceipt, ExecutionEventRecord,
    HeartbeatReconciliation, RecordExecutionEvent,
};
pub use error::{Result, StorageError};
pub use execution::{
    CommitResult, CreatedExecution, ExecutionAuthContext, ExecutionAuthSpec, ExecutionState,
    ExecutionView, RecoverableExecutionView, SteerDelivery,
};
pub use message::{
    ChatRefView, DispatchReason, FrozenSnapshotChatView, InboxDispatch, InboxItemView, InboxState,
    IncomingMessage, IncomingMessageResult, IncomingReaction, IngestEvent, IngestResult,
    MessageInput, MessageView, ProviderMessageView, ReactionAction, ReactionOrigin, ReactionResult,
    RecallTarget, ResourceMetadata, ResourceView, SnapshotChatView, SnapshotView,
};
pub use person::{HumanTeamMemberView, PersonRecord, TeamRole};
pub use schema::{RecoveryState, SqliteSettings};
pub use send::{
    CursorView, FinishSendResult, GuardedSendResult, RecoverableSendView, SendAttemptView,
    SendOutcome, SendRequest, ToolInvocationRequest, ToolInvocationResult,
};
pub use workspace::{
    SecretValue, WorkspaceRecord, WorkspaceSecretRef, WorkspaceSetupStatus, WorkspaceSpec,
    WorkspaceView,
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}
impl DatabaseConfig {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

pub struct Storage {
    config: DatabaseConfig,
    connection: Mutex<Connection>,
}
pub trait Store: Send + Sync {
    fn database_config(&self) -> &DatabaseConfig;
}
impl Store for Storage {
    fn database_config(&self) -> &DatabaseConfig {
        &self.config
    }
}

mod retry;
pub use retry::{FailureHistory, FailureNotice};
