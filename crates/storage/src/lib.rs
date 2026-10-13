#![forbid(unsafe_code)]
//! Real SQLite/WAL persistence and Runtime transaction boundary.

use lankide_core::{
    AgentId, ChannelId, MessageAuthorType, PersonId, ProviderId, ResourceId, WorkspaceId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

type RecoverableExecutionRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    i64,
    String,
);
type StoredMessageRow = (
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
);
type StoredAgentRow = (
    String,
    String,
    i64,
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
);
type RecoverableSendRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    String,
);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DatabaseConfig {
    pub path: PathBuf,
}
impl DatabaseConfig {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Invalid(String),
    Conflict(String),
    LockPoisoned,
    Busy(String),
}
impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "storage_sqlite: {e}"),
            Self::Json(e) => write!(f, "storage_json: {e}"),
            Self::Invalid(e) => write!(f, "storage_invalid: {e}"),
            Self::Conflict(e) => write!(f, "storage_conflict: {e}"),
            Self::LockPoisoned => f.write_str("storage_lock_poisoned"),
            Self::Busy(e) => write!(f, "storage_busy: {e}"),
        }
    }
}
impl std::error::Error for StorageError {}
impl From<rusqlite::Error> for StorageError {
    fn from(v: rusqlite::Error) -> Self {
        Self::Sqlite(v)
    }
}
impl From<serde_json::Error> for StorageError {
    fn from(v: serde_json::Error) -> Self {
        Self::Json(v)
    }
}
pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    External,
    Member,
    Lead,
}
impl TeamRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Member => "member",
            Self::Lead => "lead",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "external" => Ok(Self::External),
            "member" => Ok(Self::Member),
            "lead" => Ok(Self::Lead),
            _ => Err(StorageError::Invalid(format!("team role {value}"))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PersonRecord {
    pub workspace_id: WorkspaceId,
    pub person_id: PersonId,
    pub team_role: TeamRole,
    pub provider_instance_id: ProviderId,
    pub external_id: String,
    pub provider_identity_state: Value,
    pub provider_state_revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HumanTeamMemberView {
    pub person_id: PersonId,
    pub name: Option<String>,
    pub team_role: TeamRole,
}

#[derive(Clone, Debug)]
pub struct WorkspaceSpec {
    pub id: WorkspaceId,
    pub name: String,
    pub provider_id: ProviderId,
    pub provider_config: Value,
    pub provider_secret: Value,
    pub active: bool,
    pub now: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WorkspaceView {
    pub id: WorkspaceId,
    pub name: String,
    pub provider_id: ProviderId,
    pub provider_config: Value,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WorkspaceSecretRef {
    pub workspace_id: WorkspaceId,
    pub revision: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub name: String,
    pub provider_id: ProviderId,
    pub provider_config: Value,
    /// Opaque, Provider-owned durable state. Management updates never replace it.
    pub provider_state: Value,
    pub rules: String,
    pub cover_resource_id: Option<ResourceId>,
    pub setup_status: WorkspaceSetupStatus,
    pub setup_error_code: Option<String>,
    pub secret_ref: Option<WorkspaceSecretRef>,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSetupStatus {
    Pending,
    Ready,
    Error,
}
impl WorkspaceSetupStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "error" => Ok(Self::Error),
            _ => Err(StorageError::Invalid(format!(
                "workspace setup status {value}"
            ))),
        }
    }
}
#[derive(Clone, Debug)]
pub struct AgentSpec {
    pub workspace_id: WorkspaceId,
    pub id: AgentId,
    pub name: String,
    pub now: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct AgentView {
    pub workspace_id: WorkspaceId,
    pub id: AgentId,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct AgentRecord {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Option<String>,
    pub soul: String,
    pub computer_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking: String,
    pub auto_reaction: Option<String>,
    pub cli_path: Option<String>,
    pub env: BTreeMap<String, String>,
    pub provider_agent_config: Value,
    /// Opaque, Provider-owned durable state. Management updates never replace it.
    pub provider_state: Value,
    pub session_config: Value,
    pub attention: BTreeMap<ChannelId, AttentionRecord>,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionRecord {
    Mention,
    Always,
    Mute,
    Schedule { interval_minutes: u32 },
}
impl AttentionRecord {
    pub fn validate(&self) -> Result<()> {
        if let Self::Schedule { interval_minutes } = self
            && !matches!(*interval_minutes, 5 | 15 | 30 | 60 | 120 | 240 | 480 | 1440)
        {
            return Err(StorageError::Invalid(format!(
                "unsupported schedule interval {interval_minutes}"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SecretValue(String);
impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResourceMetadata {
    pub workspace_id: WorkspaceId,
    pub resource_id: ResourceId,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub storage_ref: String,
    pub created_at: i64,
}
#[derive(Clone, Debug)]
pub struct IncomingMessage {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub provider_chat_id: String,
    pub chat_name: String,
    pub provider_message_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub occurred_at: i64,
    pub now: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomingMessageResult {
    pub chat_id: i64,
    pub message_id: i64,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResourceView {
    pub resource_id: ResourceId,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DispatchReason {
    DirectMessage,
    Mention,
    Follow,
    Schedule,
    Reaction { reaction: String },
    Custom { name: String, object_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InboxDispatch {
    pub agent_id: AgentId,
    pub available_at: i64,
    pub trigger_reason: DispatchReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageInput {
    pub sender_kind: MessageAuthorType,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub mentions: Vec<AgentId>,
    pub reply_to_provider_message_id: Option<String>,
    pub resources: Vec<ResourceView>,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestEvent {
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub provider_chat_id: String,
    pub chat_name: String,
    pub provider_event_id: Option<String>,
    pub provider_message_id: String,
    pub message: MessageInput,
    pub dispatch: Vec<InboxDispatch>,
    pub now: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestResult {
    pub chat_id: i64,
    pub message_id: i64,
    pub duplicate: bool,
    pub inbox_item_ids: Vec<i64>,
    pub new_inbox_items: Vec<(AgentId, i64)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SteerDelivery {
    pub delivery_id: i64,
    pub execution_id: i64,
    pub attempt: i64,
    pub computer_id: String,
    pub computer_instance_id: String,
    pub rendered_markdown: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReactionAction {
    Add,
    Remove,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReactionOrigin {
    Provider,
    AutoReaction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomingReaction {
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub target_provider_message_id: String,
    pub sender_kind: MessageAuthorType,
    pub sender_id: String,
    pub sender_name: String,
    pub reaction: String,
    pub action: ReactionAction,
    pub origin: ReactionOrigin,
    pub provider_event_id: String,
    pub dispatch: Vec<InboxDispatch>,
    pub occurred_at: i64,
    pub now: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionResult {
    pub reaction_id: Option<i64>,
    pub target_message_id: Option<i64>,
    pub duplicate: bool,
    pub ignored_unknown_target: bool,
    pub inbox_item_ids: Vec<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatRefView {
    pub chat_id: i64,
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub provider_chat_id: String,
    pub chat_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageView {
    pub message_id: i64,
    pub provider_message_id: Option<String>,
    pub sender_kind: MessageAuthorType,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub mentions: Vec<AgentId>,
    pub reply_to_provider_message_id: Option<String>,
    pub resources: Vec<ResourceView>,
    pub occurred_at: i64,
}

/// The transport-facing identity of one Runtime-owned Message.
///
/// Provider implementations need this small projection to translate an
/// internal reply target into the external platform's native message ID. It
/// intentionally excludes message content and storage implementation details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderMessageView {
    pub message_id: i64,
    pub provider_message_id: Option<String>,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecallTarget {
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub message_id: i64,
    pub provider_message_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenSnapshotChatView {
    pub chat: ChatRefView,
    pub read_from_message_id: Option<i64>,
    pub read_to_message_id: i64,
    pub unread_count: usize,
    pub recent_messages: Vec<MessageView>,
    pub mentioned_message: Option<MessageView>,
    pub trigger_messages: Vec<MessageView>,
    pub trigger_reasons: Vec<DispatchReason>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotView {
    pub snapshot_id: i64,
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub created_at: i64,
    pub chats: Vec<FrozenSnapshotChatView>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InboxState {
    Pending,
    Claimed,
    Delivered,
    Consumed,
}
impl InboxState {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "delivered" => Ok(Self::Delivered),
            "consumed" => Ok(Self::Consumed),
            _ => Err(StorageError::Invalid(format!("inbox state {s}"))),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxItemView {
    pub inbox_item_id: i64,
    pub chat_id: i64,
    pub source_id: i64,
    pub state: InboxState,
    pub claimed_execution_id: Option<i64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotChatView {
    pub chat_id: i64,
    pub read_to_message_id: i64,
    pub unread_count: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedExecution {
    pub execution_id: i64,
    pub snapshot_id: i64,
    pub delivery_id: i64,
    pub inbox_item_ids: Vec<i64>,
    pub chats: Vec<SnapshotChatView>,
    pub snapshot: SnapshotView,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Queued,
    Assigning,
    Running,
    Recovering,
    Succeeded,
    Failed,
    Cancelled,
}
impl ExecutionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Assigning => "assigning",
            Self::Running => "running",
            Self::Recovering => "recovering",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        match s {
            "queued" => Ok(Self::Queued),
            "assigning" => Ok(Self::Assigning),
            "running" => Ok(Self::Running),
            "recovering" => Ok(Self::Recovering),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(StorageError::Invalid(format!("execution state {s}"))),
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionView {
    pub execution_id: i64,
    pub snapshot_id: i64,
    pub state: ExecutionState,
    pub final_output: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAttemptState {
    Offered,
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    Lost,
    Rejected,
}
impl ExecutionAttemptState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offered => "offered",
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Lost => "lost",
            Self::Rejected => "rejected",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "offered" => Ok(Self::Offered),
            "accepted" => Ok(Self::Accepted),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "lost" => Ok(Self::Lost),
            "rejected" => Ok(Self::Rejected),
            _ => Err(StorageError::Invalid(format!(
                "execution attempt state {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAttemptView {
    pub execution_id: i64,
    pub attempt: i64,
    pub computer_id: String,
    pub computer_instance_id: String,
    pub state: ExecutionAttemptState,
    pub last_event_sequence: u64,
    pub native_session_id: Option<String>,
    pub lease_expires_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptCapability {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub kind: i32,
    pub input_schema_json: String,
    pub output_schema_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptAuthorization {
    pub execution_id: i64,
    pub attempt: i64,
    pub context_json: String,
    pub context_fingerprint: String,
    pub capabilities: Vec<AttemptCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEventRecord {
    pub execution_id: i64,
    pub attempt: i64,
    pub sequence: u64,
    pub kind: String,
    pub payload: Vec<u8>,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordExecutionEvent {
    pub execution_id: i64,
    pub attempt: i64,
    pub computer_id: String,
    pub computer_instance_id: String,
    pub sequence: u64,
    pub kind: String,
    pub payload: Vec<u8>,
    pub occurred_at: i64,
    pub now: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionEventDisposition {
    Saved,
    Duplicate,
    Committed,
    AlreadyCommitted,
    Abandoned,
    AlreadyAbandoned,
    StaleAttempt,
    SequenceGap,
}
impl ExecutionEventDisposition {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Committed | Self::AlreadyCommitted | Self::Abandoned | Self::AlreadyAbandoned
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEventReceipt {
    pub max_saved_sequence: u64,
    pub disposition: ExecutionEventDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct HeartbeatReconciliation {
    pub continued: Vec<(i64, i64)>,
    pub lost: Vec<(i64, i64)>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAuthSpec {
    pub attempt: i64,
    pub token_hash: String,
    pub expires_at: i64,
}
#[derive(Clone, Eq, PartialEq)]
pub struct ExecutionAuthContext {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub execution_id: i64,
    pub attempt: i64,
    pub token_hash: String,
    pub expires_at: i64,
    pub state: ExecutionState,
}
impl fmt::Debug for ExecutionAuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionAuthContext")
            .field("workspace_id", &self.workspace_id)
            .field("agent_id", &self.agent_id)
            .field("execution_id", &self.execution_id)
            .field("attempt", &self.attempt)
            .field("token_hash", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("state", &self.state)
            .finish()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorView {
    pub chat_id: i64,
    pub read_to_message_id: i64,
}
#[derive(Clone, Debug)]
pub struct ToolInvocationRequest {
    pub execution_id: i64,
    pub attempt: i64,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub now: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInvocationResult {
    pub created: bool,
    pub state: String,
    pub result: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct SendRequest {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub request_key: String,
    pub sender_agent_id: AgentId,
    pub content: String,
    pub now: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendAttemptView {
    pub created: bool,
    pub message_id: i64,
    pub request_key: String,
    pub state: String,
    pub provider_message_id: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardedSendResult {
    Ready(SendAttemptView),
    NewMessages {
        snapshot_message_id: i64,
        latest_message_id: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SendOutcome {
    Sent { provider_message_id: String },
    Unknown,
    Failed { code: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishSendResult {
    pub changed: bool,
    pub attempt: SendAttemptView,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitResult {
    Committed,
    AlreadyCommitted,
    NotReady,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableExecutionView {
    pub execution: ExecutionView,
    pub snapshot: SnapshotView,
    pub delivery_id: i64,
    pub delivery_state: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableSendView {
    pub attempt: SendAttemptView,
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub sender_agent_id: AgentId,
    pub content: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryState {
    pub executions: Vec<RecoverableExecutionView>,
    pub sends: Vec<RecoverableSendView>,
    pub transitioned_executions: usize,
    pub transitioned_sends: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteSettings {
    pub journal_mode: String,
    pub foreign_keys: bool,
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
impl Storage {
    pub fn open(config: DatabaseConfig) -> Result<Self> {
        if let Some(p) = config.path.parent() {
            std::fs::create_dir_all(p).map_err(|e| StorageError::Invalid(e.to_string()))?;
        }
        let mut c = Connection::open(&config.path)?;
        configure(&c)?;
        migrate(&mut c)?;
        Ok(Self {
            config,
            connection: Mutex::new(c),
        })
    }
    fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
    pub fn database_path(&self) -> &Path {
        &self.config.path
    }
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn()?
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
    pub fn sqlite_settings(&self) -> Result<SqliteSettings> {
        let c = self.conn()?;
        let j: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        let f: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        Ok(SqliteSettings {
            journal_mode: j.to_ascii_lowercase(),
            foreign_keys: f == 1,
        })
    }
    pub fn recover_startup(&self, now: i64) -> Result<RecoveryState> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        t.execute(
            "UPDATE execution_deliveries SET state='abandoned' WHERE state='pending' AND execution_id IN (SELECT execution_id FROM executions WHERE state IN('succeeded','failed','cancelled'))",
            [],
        )?;
        let transitioned_sends=t.execute("UPDATE message_send_attempts SET state='unknown',finished_at=coalesce(finished_at,?1) WHERE state='sending'",[now])?;
        t.execute(
            "UPDATE messages SET send_status='unknown',updated_at=?1 WHERE send_status='sending'",
            [now],
        )?;
        t.execute("UPDATE tool_invocations SET state='unknown',finished_at=coalesce(finished_at,?1) WHERE state='running'",[now])?;
        let transitioned_executions=t.execute("UPDATE executions SET state='recovering',updated_at=?1 WHERE state IN('queued','assigning','running')",[now])?;
        t.commit()?;
        let executions = load_recoverable_executions(&c, None)?;
        let sends = load_recoverable_sends(&c, None)?;
        Ok(RecoveryState {
            executions,
            sends,
            transitioned_executions,
            transitioned_sends,
        })
    }
    pub fn close(self) -> Result<()> {
        let c = self
            .connection
            .into_inner()
            .map_err(|_| StorageError::LockPoisoned)?;
        let (busy, _, _): (i64, i64, i64) =
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        if busy != 0 {
            return Err(StorageError::Busy(
                "wal checkpoint has active readers".into(),
            ));
        }
        c.close()
            .map_err(|(_, error)| StorageError::Sqlite(error))?;
        Ok(())
    }
    pub fn create_workspace(&self, s: WorkspaceSpec) -> Result<()> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        t.execute(
            "INSERT INTO workspaces(workspace_id,name,provider_id,provider_config_json,desired_state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",
            params![
                s.id.as_str(),
                s.name,
                s.provider_id.as_str(),
                serde_json::to_string(&s.provider_config)?,
                if s.active { "running" } else { "paused" },
                s.now
            ],
        )?;
        t.execute(
            "INSERT INTO workspace_provider_secrets VALUES(?1,?2,?3)",
            params![
                s.id.as_str(),
                serde_json::to_string(&s.provider_secret)?,
                s.now
            ],
        )?;
        t.commit()?;
        Ok(())
    }
    pub fn workspace(&self, id: &WorkspaceId) -> Result<WorkspaceView> {
        Ok(self.conn()?.query_row(
            "SELECT * FROM workspaces WHERE workspace_id=?1",
            [id.as_str()],
            map_workspace,
        )?)
    }
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceView>> {
        let c = self.conn()?;
        let mut s = c.prepare("SELECT * FROM workspaces ORDER BY workspace_id")?;
        Ok(s.query_map([], map_workspace)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Resolve one stable Provider identity inside a Workspace. New people are
    /// assigned a compact UUID-derived ID which is also checked against Agent
    /// IDs so the value remains unambiguous in Agent-facing text.
    pub fn get_or_create_person(
        &self,
        workspace_id: &WorkspaceId,
        provider_instance_id: &ProviderId,
        external_id: &str,
        now: i64,
    ) -> Result<PersonRecord> {
        let external_id = external_id.trim();
        if external_id.is_empty() {
            return Err(StorageError::Invalid("empty external person id".into()));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        if let Some(person) = load_person_by_external(
            &transaction,
            workspace_id,
            provider_instance_id,
            external_id,
        )? {
            transaction.commit()?;
            return Ok(person);
        }
        let person_id = loop {
            let candidate = uuid::Uuid::new_v4().simple().to_string()[..6].to_owned();
            let occupied: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM people WHERE workspace_id=?1 AND person_id=?2 UNION ALL SELECT 1 FROM agents WHERE workspace_id=?1 AND agent_id=?2)",
                params![workspace_id.as_str(), candidate],
                |row| row.get(0),
            )?;
            if !occupied {
                break PersonId::new(candidate)
                    .map_err(|error| StorageError::Invalid(error.to_string()))?;
            }
        };
        transaction.execute(
            "INSERT INTO people(workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at) VALUES(?1,?2,'external',?3,?4,'{}',0,?5,?5)",
            params![workspace_id.as_str(), person_id.as_str(), provider_instance_id.as_str(), external_id, now],
        )?;
        let person = load_person_by_id(&transaction, workspace_id, &person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))?;
        transaction.commit()?;
        Ok(person)
    }

    pub fn person(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
    ) -> Result<Option<PersonRecord>> {
        load_person_by_id(&*self.conn()?, workspace_id, person_id)
    }

    pub fn list_human_team_members(&self, workspace_id: &WorkspaceId) -> Result<Vec<PersonRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at FROM people WHERE workspace_id=?1 AND team_role IN('member','lead') ORDER BY person_id",
        )?;
        Ok(statement
            .query_map([workspace_id.as_str()], map_person)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Return the Agent-facing human roster. Display names come from accepted
    /// messages, so the people identity record remains limited to stable IDs
    /// and Provider-owned identity state.
    pub fn list_human_team_member_views(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<HumanTeamMemberView>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT p.person_id,p.team_role,(SELECT NULLIF(TRIM(m.sender_name),'') FROM messages m JOIN chats c ON c.chat_id=m.chat_id WHERE c.workspace_id=p.workspace_id AND m.sender_kind='human' AND m.sender_id=p.person_id AND NULLIF(TRIM(m.sender_name),'') IS NOT NULL ORDER BY m.occurred_at DESC,m.message_id DESC LIMIT 1) FROM people p WHERE p.workspace_id=?1 AND p.team_role IN('member','lead') ORDER BY p.person_id",
        )?;
        Ok(statement
            .query_map([workspace_id.as_str()], |row| {
                let role = row.get::<_, String>(1)?;
                Ok(HumanTeamMemberView {
                    person_id: person_id(row.get(0)?)?,
                    name: row.get(2)?,
                    team_role: TeamRole::parse(&role).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn compare_and_set_person_state(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        expected_revision: i64,
        provider_identity_state: &Value,
        now: i64,
    ) -> Result<PersonRecord> {
        if !provider_identity_state.is_object() {
            return Err(StorageError::Invalid(
                "person provider identity state must be an object".into(),
            ));
        }
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE people SET provider_identity_state_json=?1,provider_state_revision=provider_state_revision+1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4 AND provider_state_revision=?5",
            params![serde_json::to_string(provider_identity_state)?, now, workspace_id.as_str(), person_id.as_str(), expected_revision],
        )?;
        if changed != 1 {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM people WHERE workspace_id=?1 AND person_id=?2)",
                params![workspace_id.as_str(), person_id.as_str()],
                |row| row.get(0),
            )?;
            return if exists {
                Err(StorageError::Conflict("stale person identity state".into()))
            } else {
                Err(StorageError::Invalid("unknown person".into()))
            };
        }
        load_person_by_id(&*connection, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))
    }

    pub fn set_person_team_role(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        role: TeamRole,
        now: i64,
    ) -> Result<PersonRecord> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE people SET team_role=?1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4",
            params![
                role.as_str(),
                now,
                workspace_id.as_str(),
                person_id.as_str()
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid("unknown person".into()));
        }
        load_person_by_id(&*connection, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))
    }

    pub fn initialize_team_lead(
        &self,
        workspace_id: &WorkspaceId,
        person_id: &PersonId,
        now: i64,
    ) -> Result<PersonRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let current_lead: Option<String> = transaction
            .query_row(
                "SELECT person_id FROM people WHERE workspace_id=?1 AND team_role='lead'",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match current_lead {
            Some(current) if current != person_id.as_str() => {
                return Err(StorageError::Conflict(
                    "workspace already has a team lead".into(),
                ));
            }
            Some(_) => {}
            None => {
                let changed = transaction.execute(
                    "UPDATE people SET team_role='lead',updated_at=?1 WHERE workspace_id=?2 AND person_id=?3",
                    params![now, workspace_id.as_str(), person_id.as_str()],
                )?;
                if changed != 1 {
                    return Err(StorageError::Invalid("unknown person".into()));
                }
            }
        }
        let person = load_person_by_id(&transaction, workspace_id, person_id)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))?;
        transaction.commit()?;
        Ok(person)
    }

    pub fn update_team_member(
        &self,
        workspace_id: &WorkspaceId,
        actor: &PersonId,
        target: &PersonId,
        role: TeamRole,
        now: i64,
    ) -> Result<PersonRecord> {
        if role == TeamRole::External {
            return Err(StorageError::Invalid("unsupported team role update".into()));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let actor_role: Option<String> = transaction
            .query_row(
                "SELECT team_role FROM people WHERE workspace_id=?1 AND person_id=?2",
                params![workspace_id.as_str(), actor.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if actor_role.as_deref() != Some("lead") {
            return Err(StorageError::Conflict("team lead required".into()));
        }
        if role == TeamRole::Lead {
            transaction.execute(
                "UPDATE people SET team_role='member',updated_at=?1 WHERE workspace_id=?2 AND team_role='lead'",
                params![now, workspace_id.as_str()],
            )?;
        }
        let changed = transaction.execute(
            "UPDATE people SET team_role=?1,updated_at=?2 WHERE workspace_id=?3 AND person_id=?4",
            params![role.as_str(), now, workspace_id.as_str(), target.as_str()],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid("unknown person".into()));
        }
        let person = load_person_by_id(&transaction, workspace_id, target)?
            .ok_or_else(|| StorageError::Invalid("person disappeared".into()))?;
        transaction.commit()?;
        Ok(person)
    }
    pub fn workspace_record(&self, id: &WorkspaceId) -> Result<WorkspaceRecord> {
        let c = self.conn()?;
        load_workspace_record(&c, id)
    }
    pub fn list_workspace_records(&self) -> Result<Vec<WorkspaceRecord>> {
        let c = self.conn()?;
        let mut statement =
            c.prepare("SELECT workspace_id FROM workspaces ORDER BY workspace_id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let mut records = Vec::new();
        for id in ids {
            records.push(load_workspace_record(&c, &wid(id)?)?);
        }
        Ok(records)
    }
    pub fn upsert_workspace_record(&self, record: WorkspaceRecord) -> Result<WorkspaceRecord> {
        self.commit_workspace_record(record, &Value::Object(Default::default()))
    }
    pub fn insert_new_workspace_record(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> Result<WorkspaceRecord> {
        self.write_workspace_record(record, provider_secrets, true)
    }
    pub fn update_workspace_record_with_secrets(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> Result<WorkspaceRecord> {
        self.write_workspace_record(record, provider_secrets, false)
    }
    /// Atomically commits the public Workspace record and an opaque set of
    /// named Provider secrets. Secret values must be strings and are never
    /// returned from this operation.
    pub fn commit_workspace_record(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> Result<WorkspaceRecord> {
        match self.workspace_record(&record.id) {
            Ok(_) => self.update_workspace_record_with_secrets(record, provider_secrets),
            Err(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                self.insert_new_workspace_record(record, provider_secrets)
            }
            Err(error) => Err(error),
        }
    }
    fn write_workspace_record(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
        insert: bool,
    ) -> Result<WorkspaceRecord> {
        if record.name.trim().is_empty() {
            return Err(StorageError::Invalid("empty workspace name".into()));
        }
        if record.setup_status != WorkspaceSetupStatus::Error && record.setup_error_code.is_some() {
            return Err(StorageError::Invalid(
                "workspace setup error requires error status".into(),
            ));
        }
        if !record.provider_state.is_object() {
            return Err(StorageError::Invalid(
                "workspace provider state must be a JSON object".into(),
            ));
        }
        let secrets = provider_secrets.as_object().ok_or_else(|| {
            StorageError::Invalid("provider secrets must be a JSON object".into())
        })?;
        for (name, value) in secrets {
            if name.trim().is_empty() || !value.is_string() {
                return Err(StorageError::Invalid(
                    "provider secret names and values must be strings".into(),
                ));
            }
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = if insert {
            t.execute("INSERT INTO workspaces(workspace_id,name,provider_id,provider_config_json,desired_state,created_at,updated_at,rules,cover_resource_id,setup_status,setup_error_code,provider_state_json)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![record.id.as_str(),record.name,record.provider_id.as_str(),serde_json::to_string(&record.provider_config)?,if record.active{"running"}else{"paused"},record.created_at,record.updated_at,record.rules,record.cover_resource_id.map(ResourceId::get),record.setup_status.as_str(),record.setup_error_code,serde_json::to_string(&record.provider_state)?])?
        } else {
            t.execute("UPDATE workspaces SET name=?2,provider_id=?3,provider_config_json=?4,desired_state=?5,updated_at=?6,rules=?7,cover_resource_id=?8,setup_status=?9,setup_error_code=?10,provider_state_json=?11 WHERE workspace_id=?1",params![record.id.as_str(),record.name,record.provider_id.as_str(),serde_json::to_string(&record.provider_config)?,if record.active{"running"}else{"paused"},record.updated_at,record.rules,record.cover_resource_id.map(ResourceId::get),record.setup_status.as_str(),record.setup_error_code,serde_json::to_string(&record.provider_state)?])?
        };
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "workspace {} not found",
                record.id
            )));
        }
        let mut revision: i64 = t.query_row(
            "SELECT coalesce(max(revision),0) FROM workspace_named_secrets WHERE workspace_id=?1",
            [record.id.as_str()],
            |row| row.get(0),
        )?;
        for (name, value) in secrets {
            revision += 1;
            t.execute(
                "INSERT INTO workspace_named_secrets(workspace_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",
                params![record.id.as_str(), name, value.as_str().unwrap_or_default(), revision, record.updated_at],
            )?;
        }
        t.commit()?;
        load_workspace_record(&c, &record.id)
    }
    pub fn update_workspace_record(&self, record: WorkspaceRecord) -> Result<WorkspaceRecord> {
        self.update_workspace_record_with_secrets(record, &Value::Object(Default::default()))
    }
    pub fn delete_workspace(&self, id: &WorkspaceId) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM workspaces WHERE workspace_id=?1",
            [id.as_str()],
        )? == 1)
    }
    /// Roll back a Workspace that failed during its initial Provider setup.
    /// This is intentionally not exposed as a product deletion capability.
    pub fn rollback_new_workspace(&self, id: &WorkspaceId) -> Result<()> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let agents: i64 = transaction.query_row(
            "SELECT count(*) FROM agents WHERE workspace_id=?1",
            [id.as_str()],
            |row| row.get(0),
        )?;
        if agents != 0 {
            return Err(StorageError::Conflict(
                "workspace rollback after Agent creation".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM resource_metadata WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspace_resource_counters WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM provider_kv WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspace_named_secrets WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspace_provider_secrets WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspaces WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn set_workspace_active(
        &self,
        workspace_id: &WorkspaceId,
        active: bool,
        now: i64,
    ) -> Result<bool> {
        let desired = if active { "running" } else { "paused" };
        Ok(self.conn()?.execute(
            "UPDATE workspaces SET desired_state=?2,updated_at=?3 WHERE workspace_id=?1 AND desired_state<>?2",
            params![workspace_id.as_str(), desired, now],
        )? == 1)
    }
    pub fn set_workspace_setup(
        &self,
        workspace_id: &WorkspaceId,
        status: WorkspaceSetupStatus,
        error_code: Option<&str>,
        now: i64,
    ) -> Result<WorkspaceRecord> {
        if status != WorkspaceSetupStatus::Error && error_code.is_some() {
            return Err(StorageError::Invalid(
                "workspace setup error requires error status".into(),
            ));
        }
        let changed = self.conn()?.execute(
            "UPDATE workspaces SET setup_status=?2,setup_error_code=?3,updated_at=?4 WHERE workspace_id=?1 AND (setup_status<>?2 OR setup_error_code IS NOT ?3)",
            params![workspace_id.as_str(), status.as_str(), error_code, now],
        )?;
        if changed > 1 {
            return Err(StorageError::Conflict("workspace setup update".into()));
        }
        self.workspace_record(workspace_id)
    }
    pub fn provider_secret(&self, id: &WorkspaceId) -> Result<Value> {
        let s: String = self.conn()?.query_row(
            "SELECT secret_json FROM workspace_provider_secrets WHERE workspace_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&s)?)
    }
    pub fn put_secret(
        &self,
        workspace_id: &WorkspaceId,
        name: &str,
        value: SecretValue,
        now: i64,
    ) -> Result<WorkspaceSecretRef> {
        if name.trim().is_empty() {
            return Err(StorageError::Invalid("empty secret name".into()));
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let next: i64 = t.query_row(
            "SELECT coalesce(max(revision),0)+1 FROM workspace_named_secrets WHERE workspace_id=?1",
            [workspace_id.as_str()],
            |r| r.get(0),
        )?;
        t.execute("INSERT INTO workspace_named_secrets(workspace_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",params![workspace_id.as_str(),name,value.expose_secret(),next,now])?;
        t.commit()?;
        Ok(WorkspaceSecretRef {
            workspace_id: workspace_id.clone(),
            revision: next,
        })
    }
    pub fn put_agent_secret(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        name: &str,
        value: SecretValue,
        now: i64,
    ) -> Result<()> {
        if name.trim().is_empty() || name != name.trim() {
            return Err(StorageError::Invalid("empty secret name".into()));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let next: i64 = transaction.query_row(
            "SELECT coalesce(max(revision),0)+1 FROM agent_named_secrets WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO agent_named_secrets(workspace_id,agent_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,agent_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",
            params![workspace_id.as_str(), agent_id.as_str(), name, value.expose_secret(), next, now],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn read_secret(
        &self,
        workspace_id: &WorkspaceId,
        name: &str,
    ) -> Result<Option<SecretValue>> {
        if let Some(scoped) = name.strip_prefix("agent:")
            && let Some((agent_id, secret_name)) = scoped.split_once(':')
        {
            return Ok(self
                .conn()?
                .query_row(
                    "SELECT value FROM agent_named_secrets WHERE workspace_id=?1 AND agent_id=?2 AND name=?3",
                    params![workspace_id.as_str(), agent_id, secret_name],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(SecretValue::new));
        }
        Ok(self
            .conn()?
            .query_row(
                "SELECT value FROM workspace_named_secrets WHERE workspace_id=?1 AND name=?2",
                params![workspace_id.as_str(), name],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(SecretValue::new))
    }
    pub fn delete_secret(&self, workspace_id: &WorkspaceId, name: &str) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM workspace_named_secrets WHERE workspace_id=?1 AND name=?2",
            params![workspace_id.as_str(), name],
        )? == 1)
    }
    pub fn workspace_provider_state(&self, workspace_id: &WorkspaceId) -> Result<Value> {
        let value: String = self.conn()?.query_row(
            "SELECT provider_state_json FROM workspaces WHERE workspace_id=?1",
            [workspace_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&value)?)
    }
    pub fn put_workspace_provider_state(
        &self,
        workspace_id: &WorkspaceId,
        state: &Value,
    ) -> Result<()> {
        if !state.is_object() {
            return Err(StorageError::Invalid(
                "workspace provider state must be a JSON object".into(),
            ));
        }
        let changed = self.conn()?.execute(
            "UPDATE workspaces SET provider_state_json=?2 WHERE workspace_id=?1",
            params![workspace_id.as_str(), serde_json::to_string(state)?],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "workspace {workspace_id} not found"
            )));
        }
        Ok(())
    }
    pub fn agent_provider_state(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<Value> {
        let value: String = self.conn()?.query_row(
            "SELECT provider_state_json FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&value)?)
    }
    pub fn put_agent_provider_state(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        state: &Value,
    ) -> Result<()> {
        if !state.is_object() {
            return Err(StorageError::Invalid(
                "agent provider state must be a JSON object".into(),
            ));
        }
        let changed = self.conn()?.execute(
            "UPDATE agents SET provider_state_json=?3 WHERE workspace_id=?1 AND agent_id=?2",
            params![
                workspace_id.as_str(),
                agent_id.as_str(),
                serde_json::to_string(state)?
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "agent {workspace_id}/{agent_id} not found"
            )));
        }
        Ok(())
    }
    pub fn provider_kv_get(
        &self,
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()?
            .query_row(
                "SELECT value FROM provider_kv WHERE workspace_id=?1 AND namespace=?2 AND key=?3",
                params![workspace_id.as_str(), namespace, key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn provider_kv_put(
        &self,
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
        value: &[u8],
        now: i64,
    ) -> Result<()> {
        if namespace.trim().is_empty() || key.trim().is_empty() {
            return Err(StorageError::Invalid(
                "empty provider namespace or key".into(),
            ));
        }
        self.conn()?.execute("INSERT INTO provider_kv(workspace_id,namespace,key,value,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,namespace,key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![workspace_id.as_str(),namespace,key,value,now])?;
        Ok(())
    }
    pub fn provider_kv_delete(
        &self,
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM provider_kv WHERE workspace_id=?1 AND namespace=?2 AND key=?3",
            params![workspace_id.as_str(), namespace, key],
        )? == 1)
    }
    pub fn create_agent(&self, s: AgentSpec) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO agents(workspace_id,agent_id,name,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
            params![s.workspace_id.as_str(), s.id.as_str(), s.name, s.now],
        )?;
        Ok(())
    }
    pub fn list_agents(&self, w: &WorkspaceId) -> Result<Vec<AgentView>> {
        let c = self.conn()?;
        let mut s = c.prepare(
            "SELECT workspace_id,agent_id,name FROM agents WHERE workspace_id=?1 ORDER BY agent_id",
        )?;
        Ok(s.query_map([w.as_str()], |r| {
            Ok(AgentView {
                workspace_id: wid(r.get(0)?)?,
                id: aid(r.get(1)?)?,
                name: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn agent_record(&self, w: &WorkspaceId, a: &AgentId) -> Result<AgentRecord> {
        let c = self.conn()?;
        load_agent_record(&c, w, a)
    }
    pub fn list_agent_records(&self, w: &WorkspaceId) -> Result<Vec<AgentRecord>> {
        let c = self.conn()?;
        let mut s =
            c.prepare("SELECT agent_id FROM agents WHERE workspace_id=?1 ORDER BY agent_id")?;
        let ids = s
            .query_map([w.as_str()], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(s);
        let mut out = Vec::new();
        for id in ids {
            out.push(load_agent_record(&c, w, &aid(id)?)?);
        }
        Ok(out)
    }
    pub fn upsert_agent_record(&self, record: AgentRecord) -> Result<AgentRecord> {
        self.commit_agent_record(record, &Value::Object(Default::default()))
    }
    pub fn insert_new_agent_record(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> Result<AgentRecord> {
        self.write_agent_record(record, provider_secrets, true)
    }
    pub fn update_agent_record_with_secrets(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> Result<AgentRecord> {
        self.write_agent_record(record, provider_secrets, false)
    }
    /// Atomically saves the Agent record and any newly supplied opaque Provider
    /// secrets. Existing secrets not present in `provider_secrets` are kept.
    pub fn commit_agent_record(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> Result<AgentRecord> {
        match self.agent_record(&record.workspace_id, &record.agent_id) {
            Ok(_) => self.update_agent_record_with_secrets(record, provider_secrets),
            Err(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                self.insert_new_agent_record(record, provider_secrets)
            }
            Err(error) => Err(error),
        }
    }
    fn write_agent_record(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
        insert: bool,
    ) -> Result<AgentRecord> {
        if record.name.trim().is_empty() {
            return Err(StorageError::Invalid("empty agent name".into()));
        }
        if !record.provider_state.is_object() {
            return Err(StorageError::Invalid(
                "agent provider state must be a JSON object".into(),
            ));
        }
        let secrets = provider_secrets.as_object().ok_or_else(|| {
            StorageError::Invalid("provider secrets must be a JSON object".into())
        })?;
        for (name, value) in secrets {
            if name.trim().is_empty() || !value.is_string() {
                return Err(StorageError::Invalid(
                    "provider secret names and values must be strings".into(),
                ));
            }
        }
        for attention in record.attention.values() {
            attention.validate()?;
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = if insert {
            t.execute("INSERT INTO agents(workspace_id,agent_id,name,created_at,updated_at,role,bio,avatar,soul,computer_id,harness_id,model,thinking,auto_reaction,cli_path,env_json,provider_agent_config_json,session_config_json,attention_json,provider_state_json)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",params![record.workspace_id.as_str(),record.agent_id.as_str(),record.name,record.created_at,record.updated_at,record.role,record.bio,record.avatar,record.soul,record.computer_id,record.harness_id,record.model,record.thinking,record.auto_reaction,record.cli_path,serde_json::to_string(&record.env)?,serde_json::to_string(&record.provider_agent_config)?,serde_json::to_string(&record.session_config)?,serde_json::to_string(&record.attention)?,serde_json::to_string(&record.provider_state)?])?
        } else {
            t.execute("UPDATE agents SET name=?3,updated_at=?4,role=?5,bio=?6,avatar=?7,soul=?8,computer_id=?9,harness_id=?10,model=?11,thinking=?12,auto_reaction=?13,cli_path=?14,env_json=?15,provider_agent_config_json=?16,session_config_json=?17,attention_json=?18,provider_state_json=?19 WHERE workspace_id=?1 AND agent_id=?2",params![record.workspace_id.as_str(),record.agent_id.as_str(),record.name,record.updated_at,record.role,record.bio,record.avatar,record.soul,record.computer_id,record.harness_id,record.model,record.thinking,record.auto_reaction,record.cli_path,serde_json::to_string(&record.env)?,serde_json::to_string(&record.provider_agent_config)?,serde_json::to_string(&record.session_config)?,serde_json::to_string(&record.attention)?,serde_json::to_string(&record.provider_state)?])?
        };
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "agent {} not found",
                record.agent_id
            )));
        }
        let mut revision: i64 = t.query_row(
            "SELECT coalesce(max(revision),0) FROM agent_named_secrets WHERE workspace_id=?1 AND agent_id=?2",
            params![record.workspace_id.as_str(), record.agent_id.as_str()],
            |row| row.get(0),
        )?;
        for (name, value) in secrets {
            revision += 1;
            t.execute(
                "INSERT INTO agent_named_secrets(workspace_id,agent_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,agent_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",
                params![record.workspace_id.as_str(), record.agent_id.as_str(), name, value.as_str().unwrap_or_default(), revision, record.updated_at],
            )?;
        }
        t.commit()?;
        load_agent_record(&c, &record.workspace_id, &record.agent_id)
    }

    /// Roll back an Agent that failed during its initial Provider validation.
    /// This is intentionally not exposed as a product deletion capability.
    pub fn rollback_new_agent(&self, workspace_id: &WorkspaceId, agent_id: &AgentId) -> Result<()> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        delete_agent_state(&transaction, workspace_id, agent_id)?;
        transaction.execute(
            "DELETE FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn update_agent_record(&self, record: AgentRecord) -> Result<AgentRecord> {
        self.update_agent_record_with_secrets(record, &Value::Object(Default::default()))
    }
    pub fn delete_agent(&self, w: &WorkspaceId, a: &AgentId) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| StorageError::Invalid(error.to_string()))?
            .as_millis() as i64;
        let computer_id = transaction
            .query_row(
                "SELECT computer_id FROM agents WHERE workspace_id=?1 AND agent_id=?2",
                params![w.as_str(), a.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(computer_id) = computer_id else {
            transaction.commit()?;
            return Ok(false);
        };
        if !computer_id.is_empty() {
            transaction.execute(
                "INSERT INTO agent_file_deletions(workspace_id,agent_id,computer_id,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,agent_id) DO UPDATE SET computer_id=excluded.computer_id,created_at=excluded.created_at",
                params![w.as_str(), a.as_str(), computer_id, now],
            )?;
        }
        delete_agent_state(&transaction, w, a)?;
        let changed = transaction.execute(
            "DELETE FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
        )? == 1;
        transaction.commit()?;
        Ok(changed)
    }
    pub fn pending_agent_file_deletions(
        &self,
        computer_id: &str,
    ) -> Result<Vec<(WorkspaceId, AgentId)>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT workspace_id,agent_id FROM agent_file_deletions WHERE computer_id=?1 ORDER BY created_at,workspace_id,agent_id",
        )?;
        Ok(statement
            .query_map([computer_id], |row| {
                Ok((wid(row.get(0)?)?, aid(row.get(1)?)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn complete_agent_file_deletion(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM agent_file_deletions WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
        )? == 1)
    }
    pub fn allocate_resource_id(&self, workspace_id: &WorkspaceId, now: i64) -> Result<ResourceId> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO workspace_resource_counters(workspace_id,next_id,updated_at) VALUES(?1,1,?2)",
            params![workspace_id.as_str(), now],
        )?;
        let next_id: i64 = transaction.query_row(
            "SELECT next_id FROM workspace_resource_counters WHERE workspace_id=?1",
            [workspace_id.as_str()],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE workspace_resource_counters SET next_id=?2,updated_at=?3 WHERE workspace_id=?1 AND next_id=?4",
            params![workspace_id.as_str(), next_id + 1, now, next_id],
        )?;
        transaction.commit()?;
        ResourceId::new(next_id).map_err(|error| StorageError::Invalid(error.to_string()))
    }
    pub fn upsert_resource_metadata(&self, metadata: ResourceMetadata) -> Result<ResourceMetadata> {
        self.conn()?.execute("INSERT INTO resource_metadata(workspace_id,resource_id,file_name,media_type,size_bytes,sha256,storage_ref,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(workspace_id,resource_id) DO UPDATE SET file_name=excluded.file_name,media_type=excluded.media_type,size_bytes=excluded.size_bytes,sha256=excluded.sha256,storage_ref=excluded.storage_ref",params![metadata.workspace_id.as_str(),metadata.resource_id.get(),metadata.file_name,metadata.media_type,metadata.size_bytes as i64,metadata.sha256,metadata.storage_ref,metadata.created_at])?;
        let c = self.conn()?;
        load_resource_metadata(&c, &metadata.workspace_id, metadata.resource_id)?
            .ok_or_else(|| StorageError::Invalid("resource metadata disappeared".into()))
    }
    pub fn resource_metadata(
        &self,
        w: &WorkspaceId,
        id: ResourceId,
    ) -> Result<Option<ResourceMetadata>> {
        let c = self.conn()?;
        load_resource_metadata(&c, w, id)
    }
    pub fn receive_message(&self, m: IncomingMessage) -> Result<IncomingMessageResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let o = receive(&t, &m)?;
        t.commit()?;
        Ok(o)
    }
    pub fn ingest_event(&self, event: IngestEvent) -> Result<IngestResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        ensure_workspace_provider(&t, &event.workspace_id, &event.provider_id)?;
        t.execute("INSERT INTO channels VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,channel_id) DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![event.workspace_id.as_str(),event.channel_id.as_str(),event.channel_name,event.now])?;
        t.execute("INSERT INTO chats(workspace_id,channel_id,provider_chat_id,name,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT(workspace_id,channel_id,provider_chat_id) DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![event.workspace_id.as_str(),event.channel_id.as_str(),event.provider_chat_id,event.chat_name,event.now])?;
        let chat_id: i64 = t.query_row("SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3",params![event.workspace_id.as_str(),event.channel_id.as_str(),event.provider_chat_id],|r|r.get(0))?;
        if let Some((message_id, direction, sender_kind, sender_id)) = t
            .query_row(
                "SELECT message_id,direction,sender_kind,sender_id FROM messages WHERE chat_id=?1 AND provider_message_id=?2",
                params![chat_id, event.provider_message_id],
                |row| Ok((row.get(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
            )
            .optional()?
        {
            validate_dispatch(&event.dispatch)?;
            let existing_mentions: String = t.query_row(
                "SELECT mentions_json FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get(0),
            )?;
            let mut merged_mentions =
                serde_json::from_str::<Vec<AgentId>>(&existing_mentions).unwrap_or_default();
            for mention in &event.message.mentions {
                if !merged_mentions.contains(mention) {
                    merged_mentions.push(mention.clone());
                }
            }
            merged_mentions.sort_by(|left, right| left.as_str().cmp(right.as_str()));
            t.execute(
                "UPDATE messages SET mentions_json=?2,provider_event_id=coalesce(provider_event_id,?3),updated_at=?4 WHERE message_id=?1",
                params![
                    message_id,
                    serde_json::to_string(&merged_mentions)?,
                    event.provider_event_id,
                    event.now
                ],
            )?;
            let mut inbox_item_ids = Vec::new();
            let mut new_inbox_items = Vec::new();
            for dispatch in &event.dispatch {
                if direction == "outbound"
                    && sender_kind == "agent"
                    && sender_id.as_deref() == Some(dispatch.agent_id.as_str())
                {
                    continue;
                }
                let changed = t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'message',?5,?6,'pending',?7,?7)",params![event.workspace_id.as_str(),dispatch.agent_id.as_str(),chat_id,message_id,serde_json::to_string(&dispatch.trigger_reason)?,dispatch.available_at,event.now])?;
                if changed == 1 {
                    let inbox_item_id = t.last_insert_rowid();
                    inbox_item_ids.push(inbox_item_id);
                    new_inbox_items.push((dispatch.agent_id.clone(), inbox_item_id));
                }
            }
            t.commit()?;
            return Ok(IngestResult {
                chat_id,
                message_id,
                duplicate: true,
                inbox_item_ids,
                new_inbox_items,
            });
        }
        validate_dispatch(&event.dispatch)?;
        t.execute("INSERT INTO messages(chat_id,provider_message_id,provider_event_id,direction,sender_kind,sender_id,sender_name,content_markdown,mentions_json,reply_to_provider_message_id,resources_json,send_status,occurred_at,created_at,updated_at) VALUES(?1,?2,?3,'inbound',?4,?5,?6,?7,?8,?9,?10,'received',?11,?12,?12)",params![chat_id,event.provider_message_id,event.provider_event_id,sender_kind(event.message.sender_kind),event.message.sender_id,event.message.sender_name,event.message.content,serde_json::to_string(&event.message.mentions)?,event.message.reply_to_provider_message_id,serde_json::to_string(&event.message.resources)?,event.message.occurred_at,event.now])?;
        let message_id = t.last_insert_rowid();
        t.execute("UPDATE chats SET first_message_id=coalesce(first_message_id,?2),last_message_id=?2,updated_at=?3 WHERE chat_id=?1",params![chat_id,message_id,event.now])?;
        let mut inbox_item_ids = Vec::new();
        let mut new_inbox_items = Vec::new();
        for dispatch in &event.dispatch {
            let changed = t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'message',?5,?6,'pending',?7,?7)",params![event.workspace_id.as_str(),dispatch.agent_id.as_str(),chat_id,message_id,serde_json::to_string(&dispatch.trigger_reason)?,dispatch.available_at,event.now])?;
            if changed == 1 {
                let inbox_item_id = t.last_insert_rowid();
                inbox_item_ids.push(inbox_item_id);
                new_inbox_items.push((dispatch.agent_id.clone(), inbox_item_id));
            }
        }
        t.commit()?;
        Ok(IngestResult {
            chat_id,
            message_id,
            duplicate: false,
            inbox_item_ids,
            new_inbox_items,
        })
    }

    pub fn record_reaction(&self, reaction: IncomingReaction) -> Result<ReactionResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        ensure_workspace_provider(&t, &reaction.workspace_id, &reaction.provider_id)?;
        if let Some((reaction_id, target_message_id)) = t.query_row("SELECT reaction_id,target_message_id FROM reactions WHERE workspace_id=?1 AND provider_event_id=?2",params![reaction.workspace_id.as_str(),reaction.provider_event_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()? {
            return Ok(ReactionResult { reaction_id: Some(reaction_id), target_message_id: Some(target_message_id), duplicate: true, ignored_unknown_target: false, inbox_item_ids: Vec::new() });
        }
        let target: Option<(i64, i64)>=t.query_row("SELECT c.chat_id,m.message_id FROM chats c JOIN messages m ON m.chat_id=c.chat_id WHERE c.workspace_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND m.provider_message_id=?4",params![reaction.workspace_id.as_str(),reaction.channel_id.as_str(),reaction.provider_chat_id,reaction.target_provider_message_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((chat_id, target_message_id)) = target else {
            return Ok(ReactionResult {
                reaction_id: None,
                target_message_id: None,
                duplicate: false,
                ignored_unknown_target: true,
                inbox_item_ids: Vec::new(),
            });
        };
        let dispatches: &[InboxDispatch] = if reaction.origin == ReactionOrigin::AutoReaction {
            &[]
        } else {
            validate_dispatch(&reaction.dispatch)?;
            &reaction.dispatch
        };
        t.execute("INSERT INTO reactions(workspace_id,chat_id,target_message_id,provider_event_id,sender_kind,sender_id,sender_name,reaction,action,origin,occurred_at,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![reaction.workspace_id.as_str(),chat_id,target_message_id,reaction.provider_event_id,sender_kind(reaction.sender_kind),reaction.sender_id,reaction.sender_name,reaction.reaction,reaction_action(reaction.action),reaction_origin(reaction.origin),reaction.occurred_at,reaction.now])?;
        let reaction_id = t.last_insert_rowid();
        let source_id = -reaction_id;
        let mut inbox_item_ids = Vec::new();
        for dispatch in dispatches {
            let changed=t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at)VALUES(?1,?2,?3,?4,'reaction',?5,?6,'pending',?7,?7)",params![reaction.workspace_id.as_str(),dispatch.agent_id.as_str(),chat_id,source_id,serde_json::to_string(&dispatch.trigger_reason)?,dispatch.available_at,reaction.now])?;
            if changed == 1 {
                inbox_item_ids.push(t.last_insert_rowid());
            }
        }
        t.commit()?;
        Ok(ReactionResult {
            reaction_id: Some(reaction_id),
            target_message_id: Some(target_message_id),
            duplicate: false,
            ignored_unknown_target: false,
            inbox_item_ids,
        })
    }
    pub fn find_message(
        &self,
        w: &WorkspaceId,
        ch: &ChannelId,
        pc: &str,
        pm: &str,
    ) -> Result<Option<i64>> {
        Ok(self.conn()?.query_row("SELECT m.message_id FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND m.provider_message_id=?4",params![w.as_str(),ch.as_str(),pc,pm],|r|r.get(0)).optional()?)
    }
    pub fn message_count(&self, w: &WorkspaceId) -> Result<usize> {
        let n: i64 = self.conn()?.query_row(
            "SELECT count(*) FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=?1",
            [w.as_str()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
    pub fn provider_message(
        &self,
        workspace_id: &WorkspaceId,
        message_id: i64,
    ) -> Result<Option<ProviderMessageView>> {
        let row: Option<(Option<String>, String, String)> = self.conn()?.query_row(
            "SELECT m.provider_message_id,ch.channel_id,ch.provider_chat_id FROM messages m JOIN chats ch USING(chat_id) WHERE m.message_id=?1 AND ch.workspace_id=?2",
            params![message_id, workspace_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        row.map(|(provider_message_id, channel_id, provider_chat_id)| {
            Ok(ProviderMessageView {
                message_id,
                provider_message_id,
                channel_id: cid(channel_id)?,
                provider_chat_id,
            })
        })
        .transpose()
    }
    pub fn chat_attention(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
    ) -> Result<Option<AttentionRecord>> {
        let value: Option<String> = self
            .conn()?
            .query_row(
                "SELECT a.attention_json FROM agent_chat_attention a JOIN chats c USING(chat_id) WHERE a.workspace_id=?1 AND a.agent_id=?2 AND c.channel_id=?3 AND c.provider_chat_id=?4",
                params![workspace_id.as_str(), agent_id.as_str(), channel_id.as_str(), provider_chat_id],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(StorageError::from))
            .transpose()
    }
    pub fn set_chat_attention(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        attention: &AttentionRecord,
        now: i64,
    ) -> Result<()> {
        attention.validate()?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let chat_id: Option<i64> = transaction
            .query_row(
                "SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3",
                params![workspace_id.as_str(), channel_id.as_str(), provider_chat_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(chat_id) = chat_id else {
            return Err(StorageError::Invalid("chat not found".into()));
        };
        transaction.execute(
            "INSERT INTO agent_chat_attention(workspace_id,agent_id,chat_id,attention_json,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,agent_id,chat_id) DO UPDATE SET attention_json=excluded.attention_json,updated_at=excluded.updated_at",
            params![workspace_id.as_str(), agent_id.as_str(), chat_id, serde_json::to_string(attention)?, now],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn recall_target(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        message_id: i64,
    ) -> Result<Option<RecallTarget>> {
        let row: Option<(String, String, Option<String>)> = self.conn()?.query_row(
            "SELECT ch.channel_id,ch.provider_chat_id,m.provider_message_id FROM messages m JOIN chats ch USING(chat_id) WHERE m.message_id=?1 AND ch.workspace_id=?2 AND m.direction='outbound' AND m.sender_kind='agent' AND m.sender_id=?3",
            params![message_id, workspace_id.as_str(), agent_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        row.map(|(channel_id, provider_chat_id, provider_message_id)| {
            Ok(RecallTarget {
                channel_id: cid(channel_id)?,
                provider_chat_id,
                message_id,
                provider_message_id,
            })
        })
        .transpose()
    }
    pub fn update_outbound_message_content(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        message_id: i64,
        content: &str,
        now: i64,
    ) -> Result<bool> {
        let changed = self.conn()?.execute(
            "UPDATE messages SET content_markdown=?4,updated_at=?5 WHERE message_id=?1 AND chat_id IN (SELECT chat_id FROM chats WHERE workspace_id=?2) AND direction='outbound' AND sender_kind='agent' AND sender_id=?3 AND send_status='sent'",
            params![message_id, workspace_id.as_str(), agent_id.as_str(), content, now],
        )?;
        Ok(changed == 1)
    }
    pub fn snapshot_chat_messages(
        &self,
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        before_message_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<MessageView>> {
        let connection = self.conn()?;
        let boundary: Option<(i64, i64)> = connection
            .query_row(
                "SELECT sc.chat_id,sc.read_to_message_id FROM unread_snapshot_chats sc JOIN chats ch USING(chat_id) WHERE sc.snapshot_id=?1 AND ch.channel_id=?2 AND ch.provider_chat_id=?3",
                params![snapshot_id, channel_id.as_str(), provider_chat_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((chat_id, read_to_message_id)) = boundary else {
            return Err(StorageError::Invalid("snapshot chat not found".into()));
        };
        let upper = before_message_id
            .map(|message_id| message_id.saturating_sub(1))
            .unwrap_or(read_to_message_id)
            .min(read_to_message_id);
        let limit = limit.clamp(1, 100) as i64;
        let mut statement = connection.prepare(
            "SELECT message_id FROM messages WHERE chat_id=?1 AND message_id<=?2 AND send_status IN ('received','sent') ORDER BY message_id DESC LIMIT ?3",
        )?;
        let mut ids = statement
            .query_map(params![chat_id, upper, limit], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.reverse();
        ids.into_iter()
            .map(|message_id| load_message(&connection, message_id))
            .collect()
    }
    pub fn snapshot_message(
        &self,
        snapshot_id: i64,
        message_id: i64,
    ) -> Result<Option<MessageView>> {
        let connection = self.conn()?;
        let included: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM unread_snapshot_chats sc JOIN messages m ON m.chat_id=sc.chat_id WHERE sc.snapshot_id=?1 AND m.message_id=?2 AND m.message_id<=sc.read_to_message_id AND m.send_status IN ('received','sent'))",
            params![snapshot_id, message_id], |row| row.get(0),
        )?;
        included
            .then(|| load_message(&connection, message_id))
            .transpose()
    }
    pub fn snapshot_contains_message(
        &self,
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        message_id: i64,
    ) -> Result<bool> {
        let count: i64 = self.conn()?.query_row(
            "SELECT count(*) FROM unread_snapshot_chats sc JOIN chats ch USING(chat_id) JOIN messages m ON m.chat_id=sc.chat_id WHERE sc.snapshot_id=?1 AND ch.channel_id=?2 AND ch.provider_chat_id=?3 AND m.message_id=?4 AND m.message_id<=sc.read_to_message_id AND m.send_status IN ('received','sent')",
            params![snapshot_id, channel_id.as_str(), provider_chat_id, message_id],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }
    pub fn inbox_items(&self, w: &WorkspaceId, a: &AgentId) -> Result<Vec<InboxItemView>> {
        let c = self.conn()?;
        let mut s=c.prepare("SELECT inbox_item_id,chat_id,source_id,state,claimed_execution_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 ORDER BY inbox_item_id")?;
        let rows = s.query_map(params![w.as_str(), a.as_str()], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get::<_, String>(3)?,
                r.get(4)?,
            ))
        })?;
        let mut out = vec![];
        for x in rows {
            let (i, ch, src, st, ce) = x?;
            out.push(InboxItemView {
                inbox_item_id: i,
                chat_id: ch,
                source_id: src,
                state: InboxState::parse(&st)?,
                claimed_execution_id: ce,
            })
        }
        Ok(out)
    }
    pub fn create_execution(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
    ) -> Result<Option<CreatedExecution>> {
        self.create_execution_inner(w, a, summary, now, None)
    }
    pub fn create_execution_with_auth(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
        auth: ExecutionAuthSpec,
    ) -> Result<Option<CreatedExecution>> {
        if auth.attempt <= 0 {
            return Err(StorageError::Invalid(
                "execution attempt must be positive".into(),
            ));
        }
        if auth.token_hash.trim().is_empty() {
            return Err(StorageError::Invalid(
                "execution token hash is empty".into(),
            ));
        }
        if auth.expires_at <= now {
            return Err(StorageError::Invalid(
                "execution token already expired".into(),
            ));
        }
        self.create_execution_inner(w, a, summary, now, Some(&auth))
    }
    fn create_execution_inner(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
        auth: Option<&ExecutionAuthSpec>,
    ) -> Result<Option<CreatedExecution>> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let n:i64=t.query_row("SELECT count(*) FROM executions WHERE workspace_id=?1 AND agent_id=?2 AND state IN('queued','assigning','running','recovering')",params![w.as_str(),a.as_str()],|r|r.get(0))?;
        if n > 0 {
            return Err(StorageError::Invalid("active execution".into()));
        }
        let mut s=t.prepare("SELECT inbox_item_id,chat_id,source_id,source_kind,trigger_reason_json FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 AND state='pending' AND available_at<=?3 ORDER BY inbox_item_id")?;
        let p = s
            .query_map(params![w.as_str(), a.as_str(), now], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(s);
        if p.is_empty() {
            return Ok(None);
        }
        t.execute("INSERT INTO unread_snapshots(workspace_id,agent_id,state,created_at)VALUES(?1,?2,'open',?3)",params![w.as_str(),a.as_str(),now])?;
        let sid = t.last_insert_rowid();
        let mut map = std::collections::BTreeMap::<i64, Vec<i64>>::new();
        for (_, ch, source, source_kind, _) in &p {
            let message_id = if source_kind == "reaction" {
                t.query_row(
                    "SELECT target_message_id FROM reactions WHERE reaction_id=?1",
                    [-*source],
                    |r| r.get(0),
                )?
            } else {
                *source
            };
            map.entry(*ch).or_default().push(message_id)
        }
        let mut chats = vec![];
        for (ch, _trigger_message_ids) in map {
            let to: i64 = t.query_row(
                "SELECT last_message_id FROM chats WHERE chat_id=?1",
                [ch],
                |r| r.get(0),
            )?;
            let from:Option<i64>=t.query_row("SELECT read_to_message_id FROM agent_chat_cursors WHERE workspace_id=?1 AND agent_id=?2 AND chat_id=?3",params![w.as_str(),a.as_str(),ch],|r|r.get(0)).optional()?.flatten();
            let unread_count:i64=t.query_row("SELECT count(*) FROM messages WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent')",params![ch,from.unwrap_or(0),to],|r|r.get(0))?;
            t.execute(
                "INSERT INTO unread_snapshot_chats VALUES(?1,?2,?3,?4,?5,?6)",
                params![sid, ch, from, to, unread_count, summary],
            )?;
            let mentioned_message_id = p.iter().rev().find_map(
                |(_, item_chat_id, source_id, source_kind, reason_json)| {
                    if *item_chat_id != ch
                        || serde_json::from_str::<DispatchReason>(reason_json).ok()
                            != Some(DispatchReason::Mention)
                    {
                        return None;
                    }
                    if source_kind == "reaction" {
                        t.query_row(
                            "SELECT target_message_id FROM reactions WHERE reaction_id=?1",
                            [-*source_id],
                            |row| row.get(0),
                        )
                        .ok()
                    } else {
                        Some(*source_id)
                    }
                },
            );
            let mut recent=t.prepare("SELECT message_id FROM messages WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent') ORDER BY message_id DESC LIMIT ?4")?;
            let mut recent_ids = recent
                .query_map(params![ch, from.unwrap_or(0), to, 3], |r| {
                    r.get::<_, i64>(0)
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            recent_ids.reverse();
            drop(recent);
            if let Some(mentioned_message_id) = mentioned_message_id {
                if !recent_ids.contains(&mentioned_message_id) {
                    recent_ids = recent_ids.into_iter().rev().take(2).collect::<Vec<_>>();
                    recent_ids.reverse();
                    recent_ids.insert(0, mentioned_message_id);
                }
            }
            for (pos, m) in recent_ids.iter().enumerate() {
                t.execute(
                    "INSERT INTO unread_snapshot_messages VALUES(?1,?2,?3,?4)",
                    params![sid, ch, m, pos as i64],
                )?;
            }
            chats.push(SnapshotChatView {
                chat_id: ch,
                read_to_message_id: to,
                unread_count: unread_count as usize,
            })
        }
        for (position, (_, chat_id, source_id, source_kind, reason)) in p.iter().enumerate() {
            let message_id = if source_kind == "reaction" {
                t.query_row(
                    "SELECT target_message_id FROM reactions WHERE reaction_id=?1",
                    [-*source_id],
                    |r| r.get(0),
                )?
            } else {
                *source_id
            };
            t.execute("INSERT INTO unread_snapshot_triggers(snapshot_id,chat_id,message_id,reason_json,position)VALUES(?1,?2,?3,?4,?5)",params![sid,chat_id,message_id,reason,position as i64])?;
        }
        let (attempt, token_hash, token_expires_at) = auth
            .map(|value| {
                (
                    value.attempt,
                    Some(value.token_hash.as_str()),
                    Some(value.expires_at),
                )
            })
            .unwrap_or((1, None, None));
        t.execute("INSERT INTO executions(workspace_id,agent_id,snapshot_id,state,trigger_summary,created_at,updated_at,attempt,token_hash,token_expires_at)VALUES(?1,?2,?3,'queued',?4,?5,?5,?6,?7,?8)",params![w.as_str(),a.as_str(),sid,summary,now,attempt,token_hash,token_expires_at])?;
        let eid = t.last_insert_rowid();
        t.execute("INSERT INTO execution_deliveries(execution_id,state,created_at)VALUES(?1,'pending',?2)",params![eid,now])?;
        let did = t.last_insert_rowid();
        let ids = p.iter().map(|x| x.0).collect::<Vec<_>>();
        for id in &ids {
            t.execute(
                "INSERT INTO execution_delivery_items VALUES(?1,?2)",
                params![did, id],
            )?;
            t.execute("UPDATE agent_inbox_items SET state='claimed',claimed_execution_id=?1,updated_at=?2 WHERE inbox_item_id=?3",params![eid,now,id])?;
        }
        for ch in &chats {
            t.execute(
                "INSERT INTO execution_delivery_chats VALUES(?1,?2,?3)",
                params![did, ch.chat_id, ch.read_to_message_id],
            )?;
        }
        let snapshot = load_snapshot(&t, sid)?;
        t.commit()?;
        Ok(Some(CreatedExecution {
            execution_id: eid,
            snapshot_id: sid,
            delivery_id: did,
            inbox_item_ids: ids,
            chats,
            snapshot,
        }))
    }
    pub fn execution_auth_context(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Option<ExecutionAuthContext>> {
        let row: Option<(String, String, i64, String, i64, String)> = self
            .conn()?
            .query_row(
                "SELECT workspace_id,agent_id,attempt,token_hash,token_expires_at,state FROM executions WHERE execution_id=?1 AND attempt=?2 AND token_hash IS NOT NULL AND token_expires_at IS NOT NULL",
                params![execution_id, attempt],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .optional()?;
        row.map(
            |(workspace, agent, stored_attempt, token_hash, expires_at, state)| {
                Ok(ExecutionAuthContext {
                    workspace_id: wid(workspace)?,
                    agent_id: aid(agent)?,
                    execution_id,
                    attempt: stored_attempt,
                    token_hash,
                    expires_at,
                    state: ExecutionState::parse(&state)?,
                })
            },
        )
        .transpose()
    }
    pub fn snapshot(&self, snapshot_id: i64) -> Result<SnapshotView> {
        let connection = self.conn()?;
        load_snapshot(&connection, snapshot_id)
    }
    pub fn mark_execution_running(&self, id: i64, now: i64) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE executions SET state='running',started_at=coalesce(started_at,?2),updated_at=?2 WHERE execution_id=?1 AND state IN('queued','assigning','recovering')",params![id,now])?==1)
    }
    pub fn mark_execution_recovering(&self, id: i64, now: i64) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE executions SET state='recovering',updated_at=?2 WHERE execution_id=?1 AND state IN('queued','assigning','running','recovering')",params![id,now])?==1)
    }
    pub fn freeze_attempt_authorization(&self, authorization: AttemptAuthorization) -> Result<()> {
        if authorization.attempt <= 0
            || authorization.context_json.trim().is_empty()
            || authorization.context_fingerprint.trim().is_empty()
        {
            return Err(StorageError::Invalid(
                "invalid attempt authorization".into(),
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        if authorization
            .capabilities
            .iter()
            .any(|capability| capability.name.trim().is_empty() || !names.insert(&capability.name))
        {
            return Err(StorageError::Invalid(
                "attempt capability names must be unique".into(),
            ));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let current_attempt: i64 = transaction.query_row(
            "SELECT attempt FROM executions WHERE execution_id=?1",
            [authorization.execution_id],
            |row| row.get(0),
        )?;
        if current_attempt != authorization.attempt {
            return Err(StorageError::Conflict("stale execution attempt".into()));
        }
        let existing: Option<(String, String)> = transaction
            .query_row(
                "SELECT context_json,context_fingerprint FROM execution_attempt_authorizations WHERE execution_id=?1 AND attempt=?2",
                params![authorization.execution_id, authorization.attempt],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((context_json, fingerprint)) = existing {
            let existing_capabilities = load_attempt_capabilities(
                &transaction,
                authorization.execution_id,
                authorization.attempt,
            )?;
            if context_json == authorization.context_json
                && fingerprint == authorization.context_fingerprint
                && existing_capabilities == authorization.capabilities
            {
                return Ok(());
            }
            return Err(StorageError::Conflict(
                "attempt authorization is immutable".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO execution_attempt_authorizations(execution_id,attempt,context_json,context_fingerprint) VALUES(?1,?2,?3,?4)",
            params![authorization.execution_id, authorization.attempt, authorization.context_json, authorization.context_fingerprint],
        )?;
        for (position, capability) in authorization.capabilities.into_iter().enumerate() {
            transaction.execute(
                "INSERT INTO execution_attempt_capabilities(execution_id,attempt,name,display_name,description,kind,input_schema_json,output_schema_json,position) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![authorization.execution_id, authorization.attempt, capability.name, capability.display_name, capability.description, capability.kind, capability.input_schema_json, capability.output_schema_json, position as i64],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn attempt_authorization(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Option<AttemptAuthorization>> {
        let connection = self.conn()?;
        let header: Option<(String, String)> = connection
            .query_row(
                "SELECT context_json,context_fingerprint FROM execution_attempt_authorizations WHERE execution_id=?1 AND attempt=?2",
                params![execution_id, attempt],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        header
            .map(|(context_json, context_fingerprint)| {
                Ok(AttemptAuthorization {
                    execution_id,
                    attempt,
                    context_json,
                    context_fingerprint,
                    capabilities: load_attempt_capabilities(&connection, execution_id, attempt)?,
                })
            })
            .transpose()
    }
    pub fn offer_execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        computer_id: &str,
        computer_instance_id: &str,
        lease_expires_at: i64,
        now: i64,
    ) -> Result<ExecutionAttemptView> {
        if attempt <= 0 || computer_id.is_empty() || computer_instance_id.is_empty() {
            return Err(StorageError::Invalid(
                "invalid execution attempt identity".into(),
            ));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let current: Option<(i64, String)> = transaction
            .query_row(
                "SELECT attempt,state FROM executions WHERE execution_id=?1",
                [execution_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_attempt, execution_state)) = current else {
            return Err(StorageError::Invalid("unknown execution".into()));
        };
        if current_attempt != attempt
            || !matches!(
                execution_state.as_str(),
                "queued" | "assigning" | "running" | "recovering"
            )
        {
            return Err(StorageError::Conflict("stale execution attempt".into()));
        }
        transaction.execute(
            "INSERT INTO execution_attempts(execution_id,attempt,computer_id,computer_instance_id,state,last_event_sequence,lease_expires_at,assigned_at) VALUES(?1,?2,?3,?4,'offered',0,?5,?6) ON CONFLICT(execution_id,attempt) DO NOTHING",
            params![execution_id, attempt, computer_id, computer_instance_id, lease_expires_at, now],
        )?;
        let view = load_execution_attempt(&transaction, execution_id, attempt)?;
        if view.computer_id != computer_id || view.computer_instance_id != computer_instance_id {
            return Err(StorageError::Conflict(
                "execution attempt belongs to another computer instance".into(),
            ));
        }
        transaction.execute(
            "UPDATE executions SET state='assigning',updated_at=?2 WHERE execution_id=?1 AND state IN('queued','recovering')",
            params![execution_id, now],
        )?;
        transaction.commit()?;
        Ok(view)
    }

    pub fn accept_execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        accepted: bool,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let state = if accepted { "accepted" } else { "rejected" };
        let changed = transaction.execute(
            "UPDATE execution_attempts SET state=?3,accepted_at=CASE WHEN ?4 THEN coalesce(accepted_at,?5) ELSE accepted_at END,finished_at=CASE WHEN ?4 THEN finished_at ELSE coalesce(finished_at,?5) END WHERE execution_id=?1 AND attempt=?2 AND state='offered'",
            params![execution_id, attempt, state, accepted, now],
        )? == 1;
        if changed {
            if accepted {
                transaction.execute("UPDATE executions SET state='running',started_at=coalesce(started_at,?2),updated_at=?2 WHERE execution_id=?1 AND attempt=?3", params![execution_id, now, attempt])?;
            } else {
                abandon_execution(
                    &transaction,
                    execution_id,
                    ExecutionState::Failed,
                    Some("assignment_rejected"),
                    now,
                )?;
            }
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn reject_execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        error_code: &str,
        error_message: &str,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let code = if error_code.trim().is_empty() {
            "assignment_rejected"
        } else {
            error_code.trim()
        };
        let changed = transaction.execute(
            "UPDATE execution_attempts SET state='rejected',error_code=?3,error_message=?4,finished_at=coalesce(finished_at,?5) WHERE execution_id=?1 AND attempt=?2 AND state='offered'",
            params![execution_id, attempt, code, error_message, now],
        )? == 1;
        if changed {
            // A rejected assignment is generally caused by a stable local
            // configuration problem. Keep the Inbox pending, but avoid a hot
            // loop that immediately creates and rejects another execution.
            transaction.execute(
                "UPDATE agent_inbox_items SET available_at=max(available_at,?2) WHERE claimed_execution_id=?1 AND state!='consumed'",
                params![execution_id, now + 60_000],
            )?;
            abandon_execution(
                &transaction,
                execution_id,
                ExecutionState::Failed,
                Some(code),
                now,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn mark_attempt_delivery(
        &self,
        execution_id: i64,
        attempt: i64,
        delivery_id: i64,
        accepted: bool,
        now: i64,
    ) -> Result<bool> {
        let connection = self.conn()?;
        let current: i64 = connection.query_row(
            "SELECT count(*) FROM executions e JOIN execution_attempts a USING(execution_id) WHERE e.execution_id=?1 AND e.attempt=?2 AND a.attempt=?2 AND a.state IN('accepted','running')",
            params![execution_id, attempt],
            |row| row.get(0),
        )?;
        if current == 0 {
            return Ok(false);
        }
        drop(connection);
        if accepted {
            self.mark_delivery_delivered(delivery_id, now)
        } else {
            self.fail_attempt(execution_id, attempt, "delivery_rejected", now)
        }
    }

    pub fn execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Option<ExecutionAttemptView>> {
        let connection = self.conn()?;
        match load_execution_attempt(&connection, execution_id, attempt) {
            Ok(value) => Ok(Some(value)),
            Err(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn execution_events(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Vec<ExecutionEventRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare("SELECT sequence,kind,payload,occurred_at FROM execution_events WHERE execution_id=?1 AND attempt=?2 ORDER BY sequence")?;
        let rows = statement
            .query_map(params![execution_id, attempt], |row| {
                Ok(ExecutionEventRecord {
                    execution_id,
                    attempt,
                    sequence: row.get::<_, i64>(0)? as u64,
                    kind: row.get(1)?,
                    payload: row.get(2)?,
                    occurred_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_execution_event(
        &self,
        event: RecordExecutionEvent,
    ) -> Result<ExecutionEventReceipt> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let execution: Option<(i64, String)> = transaction
            .query_row(
                "SELECT attempt,state FROM executions WHERE execution_id=?1",
                [event.execution_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_attempt, execution_state)) = execution else {
            return Err(StorageError::Invalid("unknown execution".into()));
        };
        let attempt = transaction.query_row(
            "SELECT computer_id,computer_instance_id,state,last_event_sequence FROM execution_attempts WHERE execution_id=?1 AND attempt=?2",
            params![event.execution_id, event.attempt],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
        ).optional()?;
        let Some((computer_id, instance_id, attempt_state, last_sequence)) = attempt else {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: 0,
                disposition: ExecutionEventDisposition::StaleAttempt,
            });
        };
        if current_attempt != event.attempt
            || computer_id != event.computer_id
            || instance_id != event.computer_instance_id
            || matches!(attempt_state.as_str(), "lost" | "rejected")
        {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition: ExecutionEventDisposition::StaleAttempt,
            });
        }
        if event.sequence <= last_sequence as u64 {
            let disposition = match (execution_state.as_str(), event.kind.as_str()) {
                ("succeeded", "completed") => ExecutionEventDisposition::AlreadyCommitted,
                ("failed", "failed") | ("cancelled", "cancelled") => {
                    ExecutionEventDisposition::AlreadyAbandoned
                }
                _ => ExecutionEventDisposition::Duplicate,
            };
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition,
            });
        }
        if event.sequence != last_sequence as u64 + 1 {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition: ExecutionEventDisposition::SequenceGap,
            });
        }
        if !matches!(
            execution_state.as_str(),
            "assigning" | "running" | "recovering"
        ) {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition: ExecutionEventDisposition::StaleAttempt,
            });
        }
        if durable_event_kind(&event.kind) {
            transaction.execute(
                "INSERT INTO execution_events(execution_id,attempt,sequence,kind,payload,occurred_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![event.execution_id, event.attempt, event.sequence as i64, event.kind, event.payload, event.occurred_at],
            )?;
        }
        transaction.execute(
            "UPDATE execution_attempts SET last_event_sequence=?3,state=CASE WHEN ?4='started' THEN 'running' ELSE state END,native_session_id=CASE WHEN ?4='native_session_bound' THEN coalesce(json_extract(CAST(?5 AS TEXT),'$.native_session_id'),native_session_id) ELSE native_session_id END WHERE execution_id=?1 AND attempt=?2",
            params![event.execution_id, event.attempt, event.sequence as i64, event.kind, event.payload],
        )?;
        let disposition = match event.kind.as_str() {
            "completed" => {
                let output = event_final_output(&event.payload);
                commit_execution_transaction(
                    &transaction,
                    event.execution_id,
                    event.attempt,
                    &output,
                    event.now,
                )?;
                ExecutionEventDisposition::Committed
            }
            "failed" => {
                transaction.execute("UPDATE execution_attempts SET state='failed',error_code='computer_execution_failed',finished_at=?3 WHERE execution_id=?1 AND attempt=?2", params![event.execution_id, event.attempt, event.now])?;
                abandon_execution(
                    &transaction,
                    event.execution_id,
                    ExecutionState::Failed,
                    Some("computer_execution_failed"),
                    event.now,
                )?;
                ExecutionEventDisposition::Abandoned
            }
            "cancelled" => {
                transaction.execute("UPDATE execution_attempts SET state='cancelled',finished_at=?3 WHERE execution_id=?1 AND attempt=?2", params![event.execution_id, event.attempt, event.now])?;
                abandon_execution(
                    &transaction,
                    event.execution_id,
                    ExecutionState::Cancelled,
                    None,
                    event.now,
                )?;
                ExecutionEventDisposition::Abandoned
            }
            _ => ExecutionEventDisposition::Saved,
        };
        transaction.commit()?;
        Ok(ExecutionEventReceipt {
            max_saved_sequence: event.sequence,
            disposition,
        })
    }

    pub fn reconcile_computer_heartbeat(
        &self,
        computer_id: &str,
        instance_id: &str,
        active_attempts: &[(i64, i64)],
        now: i64,
        lease_expires_at: i64,
    ) -> Result<HeartbeatReconciliation> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare(
            "SELECT a.execution_id,a.attempt,a.computer_instance_id,a.state FROM execution_attempts a JOIN executions e USING(execution_id) WHERE a.computer_id=?1 AND e.attempt=a.attempt AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') ORDER BY a.execution_id",
        )?;
        let attempts = statement
            .query_map([computer_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let mut result = HeartbeatReconciliation::default();
        for (execution_id, attempt, stored_instance, state) in attempts {
            let reported = active_attempts.contains(&(execution_id, attempt));
            if stored_instance == instance_id && reported {
                transaction.execute("UPDATE execution_attempts SET lease_expires_at=?3 WHERE execution_id=?1 AND attempt=?2", params![execution_id, attempt, lease_expires_at])?;
                transaction.execute("UPDATE executions SET state='running',updated_at=?2 WHERE execution_id=?1 AND state='recovering'", params![execution_id, now])?;
                result.continued.push((execution_id, attempt));
            } else if stored_instance == instance_id && state == "offered" {
                // Assignment may still be in flight. Its original lease remains
                // authoritative until Computer accepts and reports it active.
            } else {
                lose_attempt_transaction(&transaction, execution_id, attempt, now)?;
                result.lost.push((execution_id, attempt));
            }
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn expire_execution_attempt_leases(&self, now: i64) -> Result<Vec<(i64, i64)>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare("SELECT a.execution_id,a.attempt FROM execution_attempts a JOIN executions e USING(execution_id) WHERE e.attempt=a.attempt AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') AND a.lease_expires_at<=?1 ORDER BY a.execution_id")?;
        let expired = statement
            .query_map([now], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for (execution_id, attempt) in &expired {
            lose_attempt_transaction(&transaction, *execution_id, *attempt, now)?;
        }
        transaction.commit()?;
        Ok(expired)
    }

    pub fn fail_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        code: &str,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute("UPDATE execution_attempts SET state='failed',error_code=?3,finished_at=?4 WHERE execution_id=?1 AND attempt=?2 AND state IN('offered','accepted','running')", params![execution_id, attempt, code, now])? == 1;
        if changed {
            abandon_execution(
                &transaction,
                execution_id,
                ExecutionState::Failed,
                Some(code),
                now,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
    pub fn mark_delivery_delivered(&self, id: i64, now: i64) -> Result<bool> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let ok=t.execute("UPDATE execution_deliveries SET state='delivered',delivered_at=?2 WHERE delivery_id=?1 AND state='pending'",params![id,now])?==1;
        if ok {
            t.execute("UPDATE agent_inbox_items SET state='delivered',updated_at=?2 WHERE inbox_item_id IN(SELECT inbox_item_id FROM execution_delivery_items WHERE delivery_id=?1) AND state='claimed'",params![id,now])?;
        }
        t.commit()?;
        Ok(ok)
    }
    pub fn create_steer_delivery(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        inbox_item_id: i64,
        rendered_markdown: &str,
        now: i64,
    ) -> Result<Option<SteerDelivery>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let active: Option<(i64, i64, String, String)> = transaction
            .query_row(
                "SELECT e.execution_id,e.attempt,a.computer_id,a.computer_instance_id FROM executions e JOIN execution_attempts a ON a.execution_id=e.execution_id AND a.attempt=e.attempt WHERE e.workspace_id=?1 AND e.agent_id=?2 AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') ORDER BY e.execution_id DESC LIMIT 1",
                params![workspace_id.as_str(), agent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((execution_id, attempt, computer_id, computer_instance_id)) = active else {
            return Ok(None);
        };
        transaction.execute(
            "INSERT INTO steer_deliveries(execution_id,attempt,inbox_item_id,computer_id,computer_instance_id,rendered_markdown,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7) ON CONFLICT(execution_id,attempt,inbox_item_id) DO NOTHING",
            params![execution_id,attempt,inbox_item_id,computer_id,computer_instance_id,rendered_markdown,now],
        )?;
        let delivery = transaction.query_row(
            "SELECT steer_delivery_id,execution_id,attempt,computer_id,computer_instance_id,rendered_markdown,created_at FROM steer_deliveries WHERE execution_id=?1 AND attempt=?2 AND inbox_item_id=?3",
            params![execution_id, attempt, inbox_item_id],
            steer_delivery_row,
        )?;
        transaction.commit()?;
        Ok(Some(delivery))
    }
    pub fn pending_steer_deliveries(
        &self,
        computer_id: &str,
        computer_instance_id: &str,
    ) -> Result<Vec<SteerDelivery>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT s.steer_delivery_id,s.execution_id,s.attempt,s.computer_id,s.computer_instance_id,s.rendered_markdown,s.created_at FROM steer_deliveries s JOIN executions e USING(execution_id) JOIN execution_attempts a ON a.execution_id=s.execution_id AND a.attempt=s.attempt WHERE s.computer_id=?1 AND s.computer_instance_id=?2 AND s.state='pending' AND e.attempt=s.attempt AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') ORDER BY s.steer_delivery_id",
        )?;
        Ok(statement
            .query_map(
                params![computer_id, computer_instance_id],
                steer_delivery_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn mark_steer_delivery_result(
        &self,
        delivery_id: i64,
        execution_id: i64,
        attempt: i64,
        computer_id: &str,
        computer_instance_id: &str,
        accepted: bool,
        now: i64,
    ) -> Result<bool> {
        let state = if accepted { "accepted" } else { "pending" };
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE steer_deliveries SET state=?6,accepted_at=CASE WHEN ?7 THEN ?8 ELSE accepted_at END,last_error_at=CASE WHEN ?7 THEN last_error_at ELSE ?8 END WHERE steer_delivery_id=?1 AND execution_id=?2 AND attempt=?3 AND computer_id=?4 AND computer_instance_id=?5 AND state='pending' AND EXISTS(SELECT 1 FROM executions e JOIN execution_attempts a ON a.execution_id=e.execution_id AND a.attempt=e.attempt WHERE e.execution_id=?2 AND e.attempt=?3 AND e.state IN('assigning','running','recovering') AND a.computer_id=?4 AND a.computer_instance_id=?5 AND a.state IN('offered','accepted','running'))",
            params![delivery_id,execution_id,attempt,computer_id,computer_instance_id,state, accepted, now],
        )? == 1;
        if changed && accepted {
            transaction.execute(
                "UPDATE agent_inbox_items SET state='delivered',claimed_execution_id=?2,updated_at=?3 WHERE inbox_item_id=(SELECT inbox_item_id FROM steer_deliveries WHERE steer_delivery_id=?1) AND state='pending'",
                params![delivery_id, execution_id, now],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
    pub fn complete_execution(&self, id: i64, out: &str, now: i64) -> Result<bool> {
        Ok(matches!(
            self.commit_execution(id, out, None, now)?,
            CommitResult::Committed
        ))
    }
    pub fn commit_execution(
        &self,
        id: i64,
        out: &str,
        response_send_request_key: Option<&str>,
        now: i64,
    ) -> Result<CommitResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let current = state(&t, id)?;
        if current == ExecutionState::Succeeded {
            return Ok(CommitResult::AlreadyCommitted);
        }
        if current.terminal() {
            return Ok(CommitResult::NotReady);
        }
        let delivery_ready: i64 = t.query_row(
            "SELECT count(*) FROM execution_deliveries WHERE execution_id=?1 AND state='delivered'",
            [id],
            |r| r.get(0),
        )?;
        if delivery_ready == 0 {
            return Ok(CommitResult::NotReady);
        }
        if let Some(key) = response_send_request_key {
            let sent:i64=t.query_row("SELECT count(*) FROM message_send_attempts a JOIN messages m USING(message_id) WHERE a.request_key=?1 AND a.execution_id=?2 AND a.state='sent' AND m.send_status='sent'",params![key,id],|r|r.get(0))?;
            if sent == 0 {
                return Ok(CommitResult::NotReady);
            }
        }
        commit_execution_transaction(&t, id, 0, out, now)?;
        t.commit()?;
        Ok(CommitResult::Committed)
    }
    pub fn fail_execution(&self, id: i64, e: &str, now: i64) -> Result<bool> {
        self.unsuccessful(id, ExecutionState::Failed, Some(e), now)
    }
    pub fn fail_execution_retry_after(
        &self,
        id: i64,
        error: &str,
        retry_at: i64,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        if state(&transaction, id)?.terminal() {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE agent_inbox_items SET available_at=max(available_at,?2) WHERE claimed_execution_id=?1 AND state!='consumed'",
            params![id, retry_at],
        )?;
        abandon_execution(&transaction, id, ExecutionState::Failed, Some(error), now)?;
        transaction.commit()?;
        Ok(true)
    }
    pub fn cancel_execution(&self, id: i64, now: i64) -> Result<bool> {
        self.unsuccessful(id, ExecutionState::Cancelled, None, now)
    }
    fn unsuccessful(
        &self,
        id: i64,
        end: ExecutionState,
        err: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if state(&t, id)?.terminal() {
            return Ok(false);
        }
        abandon_execution(&t, id, end, err, now)?;
        t.commit()?;
        Ok(true)
    }
    pub fn execution(&self, id: i64) -> Result<ExecutionView> {
        let(s,st,o,e):(i64,String,Option<String>,Option<String>)=self.conn()?.query_row("SELECT snapshot_id,state,final_output,error_code FROM executions WHERE execution_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        Ok(ExecutionView {
            execution_id: id,
            snapshot_id: s,
            state: ExecutionState::parse(&st)?,
            final_output: o,
            error_code: e,
        })
    }
    pub fn has_active_execution(&self, workspace_id: &WorkspaceId) -> Result<bool> {
        Ok(self.conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions WHERE workspace_id=?1 AND state IN('queued','assigning','running','recovering'))",
            [workspace_id.as_str()],
            |row| row.get(0),
        )?)
    }
    pub fn active_execution_ids_for_agent(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<Vec<i64>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2 AND state IN('queued','assigning','running','recovering') ORDER BY execution_id",
        )?;
        Ok(statement
            .query_map(params![workspace_id.as_str(), agent_id.as_str()], |row| {
                row.get(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn recoverable_executions(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<RecoverableExecutionView>> {
        let c = self.conn()?;
        let mut sql="SELECT e.execution_id,e.snapshot_id,e.state,e.final_output,e.error_code,d.delivery_id,d.state FROM executions e JOIN execution_deliveries d USING(execution_id) WHERE e.state IN('queued','assigning','running','recovering')".to_owned();
        if workspace_id.is_some() {
            sql.push_str(" AND e.workspace_id=?1");
        }
        sql.push_str(" ORDER BY e.execution_id");
        let mut statement = c.prepare(&sql)?;
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RecoverableExecutionRow> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        };
        let rows = if let Some(workspace_id) = workspace_id {
            statement
                .query_map([workspace_id.as_str()], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut views = Vec::new();
        for (
            execution_id,
            snapshot_id,
            state_name,
            final_output,
            error_code,
            delivery_id,
            delivery_state,
        ) in rows
        {
            views.push(RecoverableExecutionView {
                execution: ExecutionView {
                    execution_id,
                    snapshot_id,
                    state: ExecutionState::parse(&state_name)?,
                    final_output,
                    error_code,
                },
                snapshot: load_snapshot(&c, snapshot_id)?,
                delivery_id,
                delivery_state,
            });
        }
        Ok(views)
    }
    pub fn cursors(&self, w: &WorkspaceId, a: &AgentId) -> Result<Vec<CursorView>> {
        let c = self.conn()?;
        let mut s=c.prepare("SELECT chat_id,read_to_message_id FROM agent_chat_cursors WHERE workspace_id=?1 AND agent_id=?2 ORDER BY chat_id")?;
        Ok(s.query_map(params![w.as_str(), a.as_str()], |r| {
            Ok(CursorView {
                chat_id: r.get(0)?,
                read_to_message_id: r.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn cursor(&self, w: &WorkspaceId, a: &AgentId, ch: i64) -> Result<Option<i64>> {
        Ok(self.conn()?.query_row("SELECT read_to_message_id FROM agent_chat_cursors WHERE workspace_id=?1 AND agent_id=?2 AND chat_id=?3",params![w.as_str(),a.as_str(),ch],|r|r.get(0)).optional()?)
    }
    pub fn begin_tool_invocation(&self, r: ToolInvocationRequest) -> Result<ToolInvocationResult> {
        let c = self.conn()?;
        if let Some((tool_name, arguments, state, result)) = c
            .query_row(
                "SELECT tool_name,arguments_json,state,result_json FROM tool_invocations WHERE execution_id=?1 AND attempt=?2 AND tool_call_id=?3",
                params![r.execution_id, r.attempt, r.tool_call_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        {
            if tool_name != r.tool_name
                || serde_json::from_str::<Value>(&arguments)? != r.arguments
            {
                return Err(StorageError::Conflict(
                    "tool invocation identity reused with different request".into(),
                ));
            }
            return Ok(ToolInvocationResult {
                created: false,
                state,
                result: result
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            });
        }
        c.execute(
            "INSERT INTO tool_invocations VALUES(?1,?2,?3,?4,?5,'running',NULL,?6,NULL)",
            params![
                r.execution_id,
                r.attempt,
                r.tool_call_id,
                r.tool_name,
                serde_json::to_string(&r.arguments)?,
                r.now
            ],
        )?;
        Ok(ToolInvocationResult {
            created: true,
            state: "running".into(),
            result: None,
        })
    }
    pub fn finish_tool_invocation(
        &self,
        r: &ToolInvocationRequest,
        v: Value,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE tool_invocations SET state='completed',result_json=?4,finished_at=?5 WHERE execution_id=?1 AND attempt=?2 AND tool_call_id=?3 AND state='running'",params![r.execution_id,r.attempt,r.tool_call_id,serde_json::to_string(&v)?,now])?==1)
    }
    pub fn fail_tool_invocation(
        &self,
        request: &ToolInvocationRequest,
        code: &str,
        now: i64,
    ) -> Result<bool> {
        if code.trim().is_empty() {
            return Err(StorageError::Invalid("empty tool failure code".into()));
        }
        let result = serde_json::json!({"error": code});
        Ok(self.conn()?.execute(
            "UPDATE tool_invocations SET state='failed',result_json=?4,finished_at=?5 WHERE execution_id=?1 AND attempt=?2 AND tool_call_id=?3 AND state='running'",
            params![request.execution_id, request.attempt, request.tool_call_id, serde_json::to_string(&result)?, now],
        )? == 1)
    }
    pub fn begin_send(&self, r: SendRequest) -> Result<SendAttemptView> {
        self.begin_send_for_execution(None, r)
    }
    pub fn begin_send_guarded(
        &self,
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        force: bool,
        r: SendRequest,
    ) -> Result<GuardedSendResult> {
        if &r.channel_id != channel_id || r.provider_chat_id != provider_chat_id {
            return Err(StorageError::Invalid("guarded send target mismatch".into()));
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let execution_id: i64 = t.query_row(
            "SELECT execution_id FROM executions WHERE snapshot_id=?1 AND workspace_id=?2 AND agent_id=?3 ORDER BY execution_id DESC LIMIT 1",
            params![snapshot_id, r.workspace_id.as_str(), r.sender_agent_id.as_str()],
            |row| row.get(0),
        )?;
        if let Some(existing) = load_send(&t, &r.request_key)? {
            let owner: (String, String, String, Option<String>, Option<i64>, String) = t.query_row(
                "SELECT ch.workspace_id,ch.channel_id,ch.provider_chat_id,m.sender_id,a.execution_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats ch USING(chat_id) WHERE a.request_key=?1",
                [r.request_key.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )?;
            if owner.0 != r.workspace_id.as_str()
                || owner.1 != channel_id.as_str()
                || owner.2 != provider_chat_id
                || owner.3.as_deref() != Some(r.sender_agent_id.as_str())
                || owner.4 != Some(execution_id)
            {
                return Err(StorageError::Conflict(
                    "guarded send idempotency owner mismatch".into(),
                ));
            }
            if owner.5 != r.content {
                return Err(StorageError::Conflict(
                    "guarded send idempotency key reused with different content".into(),
                ));
            }
            return Ok(GuardedSendResult::Ready(existing));
        }
        let guarded: Option<(i64, i64, String, String)> = t
            .query_row(
                "SELECT sc.chat_id,sc.read_to_message_id,s.workspace_id,s.agent_id FROM unread_snapshot_chats sc JOIN unread_snapshots s USING(snapshot_id) JOIN chats ch ON ch.chat_id=sc.chat_id WHERE sc.snapshot_id=?1 AND ch.channel_id=?2 AND ch.provider_chat_id=?3",
                params![snapshot_id, channel_id.as_str(), provider_chat_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((chat_id, snapshot_message_id, workspace_id, agent_id)) = guarded else {
            return Err(StorageError::Invalid(
                "guarded send snapshot target not found".into(),
            ));
        };
        if workspace_id != r.workspace_id.as_str() || agent_id != r.sender_agent_id.as_str() {
            return Err(StorageError::Invalid(
                "guarded send snapshot owner mismatch".into(),
            ));
        }
        let latest_message_id: i64 = t.query_row(
            "SELECT coalesce(max(message_id),0) FROM messages WHERE chat_id=?1 AND send_status IN ('received','sent') AND NOT(sender_kind='agent' AND coalesce(sender_id,'')=?2)",
            params![chat_id, r.sender_agent_id.as_str()],
            |row| row.get(0),
        )?;
        if !force && latest_message_id > snapshot_message_id {
            let latest_boundary_id: i64 = t.query_row(
                "SELECT coalesce(max(message_id),0) FROM messages WHERE chat_id=?1 AND send_status IN ('received','sent')",
                [chat_id],
                |row| row.get(0),
            )?;
            expand_execution_chat_boundary(
                &t,
                execution_id,
                snapshot_id,
                chat_id,
                latest_boundary_id,
                r.now,
            )?;
            t.commit()?;
            return Ok(GuardedSendResult::NewMessages {
                snapshot_message_id,
                latest_message_id,
            });
        }
        t.execute("INSERT INTO messages(chat_id,direction,sender_kind,sender_id,content_markdown,send_status,occurred_at,created_at,updated_at)VALUES(?1,'outbound','agent',?2,?3,'sending',?4,?4,?4)",params![chat_id,r.sender_agent_id.as_str(),r.content,r.now])?;
        let message_id = t.last_insert_rowid();
        t.execute("INSERT INTO message_send_attempts(message_id,execution_id,request_key,state,started_at)VALUES(?1,?2,?3,'sending',?4)",params![message_id,execution_id,r.request_key,r.now])?;
        t.commit()?;
        Ok(GuardedSendResult::Ready(SendAttemptView {
            created: true,
            message_id,
            request_key: r.request_key,
            state: "sending".into(),
            provider_message_id: None,
            error_code: None,
        }))
    }
    pub fn begin_execution_send(
        &self,
        execution_id: i64,
        r: SendRequest,
    ) -> Result<SendAttemptView> {
        self.begin_send_for_execution(Some(execution_id), r)
    }
    fn begin_send_for_execution(
        &self,
        execution_id: Option<i64>,
        r: SendRequest,
    ) -> Result<SendAttemptView> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if let Some(x) = load_send(&t, &r.request_key)? {
            let owner: (Option<i64>, String, String, String, Option<String>, String) = t.query_row(
                "SELECT a.execution_id,ch.workspace_id,ch.channel_id,ch.provider_chat_id,m.sender_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats ch USING(chat_id) WHERE a.request_key=?1",
                [r.request_key.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )?;
            if owner.0 != execution_id
                || owner.1 != r.workspace_id.as_str()
                || owner.2 != r.channel_id.as_str()
                || owner.3 != r.provider_chat_id
                || owner.4.as_deref() != Some(r.sender_agent_id.as_str())
                || owner.5 != r.content
            {
                return Err(StorageError::Conflict(
                    "send idempotency key reused with different request".into(),
                ));
            }
            return Ok(x);
        }
        let ch:i64=t.query_row("SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3",params![r.workspace_id.as_str(),r.channel_id.as_str(),r.provider_chat_id],|x|x.get(0))?;
        if let Some(execution_id) = execution_id {
            let matches_workspace: i64 = t.query_row(
                "SELECT count(*) FROM executions WHERE execution_id=?1 AND workspace_id=?2",
                params![execution_id, r.workspace_id.as_str()],
                |row| row.get(0),
            )?;
            if matches_workspace == 0 {
                return Err(StorageError::Invalid(
                    "response send execution mismatch".into(),
                ));
            }
        }
        t.execute("INSERT INTO messages(chat_id,direction,sender_kind,sender_id,content_markdown,send_status,occurred_at,created_at,updated_at)VALUES(?1,'outbound','agent',?2,?3,'sending',?4,?4,?4)",params![ch,r.sender_agent_id.as_str(),r.content,r.now])?;
        let mid = t.last_insert_rowid();
        t.execute("INSERT INTO message_send_attempts(message_id,execution_id,request_key,state,started_at)VALUES(?1,?2,?3,'sending',?4)",params![mid,execution_id,r.request_key,r.now])?;
        t.commit()?;
        Ok(SendAttemptView {
            created: true,
            message_id: mid,
            request_key: r.request_key,
            state: "sending".into(),
            provider_message_id: None,
            error_code: None,
        })
    }
    pub fn finish_send(
        &self,
        request_key: &str,
        outcome: SendOutcome,
        now: i64,
    ) -> Result<FinishSendResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let current = load_send(&t, request_key)?
            .ok_or_else(|| StorageError::Invalid("send not found".into()))?;
        let (next, provider_message_id, error_code) = match &outcome {
            SendOutcome::Sent {
                provider_message_id,
            } if provider_message_id.trim().is_empty() => {
                return Err(StorageError::Invalid("empty provider message id".into()));
            }
            SendOutcome::Sent {
                provider_message_id,
            } => ("sent", Some(provider_message_id.as_str()), None),
            SendOutcome::Unknown => ("unknown", None, None),
            SendOutcome::Failed { code } if code.trim().is_empty() => {
                return Err(StorageError::Invalid("empty send failure code".into()));
            }
            SendOutcome::Failed { code } => ("failed", None, Some(code.as_str())),
        };
        let can_reconcile_unknown = current.state == "unknown" && next != "unknown";
        if current.state != "sending" && !can_reconcile_unknown {
            let same = current.state == next
                && current.provider_message_id.as_deref() == provider_message_id
                && current.error_code.as_deref() == error_code;
            if !same {
                return Err(StorageError::Conflict(format!(
                    "send outcome for {request_key}"
                )));
            }
            return Ok(FinishSendResult {
                changed: false,
                attempt: current,
            });
        }
        if let Some(provider_message_id) = provider_message_id {
            let conflict:Option<i64>=t.query_row("SELECT message_id FROM messages WHERE chat_id=(SELECT chat_id FROM messages WHERE message_id=?1) AND provider_message_id=?2 AND message_id<>?1",params![current.message_id,provider_message_id],|r|r.get(0)).optional()?;
            if conflict.is_some() {
                return Err(StorageError::Conflict(format!(
                    "provider message id {provider_message_id}"
                )));
            }
        }
        t.execute("UPDATE message_send_attempts SET state=?2,provider_message_id=?3,error_code=?4,finished_at=?5 WHERE request_key=?1 AND state IN ('sending','unknown')",params![request_key,next,provider_message_id,error_code,now])?;
        t.execute("UPDATE messages SET send_status=?2,provider_message_id=?3,updated_at=?4 WHERE message_id=?1 AND send_status IN ('sending','unknown')",params![current.message_id,next,provider_message_id,now])?;
        if next == "sent" {
            t.execute(
                "UPDATE chats SET first_message_id=coalesce(first_message_id,?2),last_message_id=CASE WHEN last_message_id IS NULL OR last_message_id<?2 THEN ?2 ELSE last_message_id END,updated_at=?3 WHERE chat_id=(SELECT chat_id FROM messages WHERE message_id=?2)",
                params![current.message_id, current.message_id, now],
            )?;
        }
        let attempt = load_send(&t, request_key)?
            .ok_or_else(|| StorageError::Invalid("send disappeared".into()))?;
        t.commit()?;
        Ok(FinishSendResult {
            changed: true,
            attempt,
        })
    }
    pub fn send_attempt(&self, k: &str) -> Result<SendAttemptView> {
        let connection = self.conn()?;
        load_send(&connection, k)?.ok_or_else(|| StorageError::Invalid("send not found".into()))
    }
    pub fn recoverable_sends(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<RecoverableSendView>> {
        let c = self.conn()?;
        let mut sql="SELECT a.message_id,a.request_key,a.state,a.provider_message_id,a.error_code,c.workspace_id,w.provider_id,c.channel_id,c.provider_chat_id,m.sender_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats c USING(chat_id) JOIN workspaces w USING(workspace_id) WHERE a.state IN('sending','unknown')".to_owned();
        if workspace_id.is_some() {
            sql.push_str(" AND c.workspace_id=?1");
        }
        sql.push_str(" ORDER BY a.send_attempt_id");
        let mut s = c.prepare(&sql)?;
        let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<_> {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, Option<String>>(10)?.unwrap_or_default(),
            ))
        };
        let rows = if let Some(workspace_id) = workspace_id {
            s.query_map([workspace_id.as_str()], mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            s.query_map([], mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut out = Vec::new();
        for (
            message_id,
            request_key,
            state,
            provider_message_id,
            error_code,
            workspace,
            provider,
            channel,
            provider_chat_id,
            sender,
            content,
        ) in rows
        {
            out.push(RecoverableSendView {
                attempt: SendAttemptView {
                    created: false,
                    message_id,
                    request_key,
                    state,
                    provider_message_id,
                    error_code,
                },
                workspace_id: wid(workspace)?,
                provider_id: pid(provider)?,
                channel_id: cid(channel)?,
                provider_chat_id,
                sender_agent_id: aid(sender)?,
                content,
            });
        }
        Ok(out)
    }
}

fn configure(c: &Connection) -> Result<()> {
    c.busy_timeout(std::time::Duration::from_secs(2))?;
    c.execute_batch("PRAGMA foreign_keys=ON;PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;")?;
    Ok(())
}

fn delete_agent_state(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    agent_id: &AgentId,
) -> Result<()> {
    let agent = params![workspace_id.as_str(), agent_id.as_str()];
    transaction.execute(
        "DELETE FROM execution_delivery_items
         WHERE delivery_id IN (
             SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
                 SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
             )
         ) OR inbox_item_id IN (
             SELECT inbox_item_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2
         )",
        agent,
    )?;
    transaction.execute(
        "DELETE FROM execution_delivery_chats WHERE delivery_id IN (
             SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
                 SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
             )
         )",
        params![workspace_id.as_str(), agent_id.as_str()],
    )?;
    transaction.execute(
        "DELETE FROM steer_deliveries
         WHERE execution_id IN (
             SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
         ) OR inbox_item_id IN (
             SELECT inbox_item_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2
         )",
        params![workspace_id.as_str(), agent_id.as_str()],
    )?;
    for table in [
        "execution_attempt_capabilities",
        "execution_attempt_authorizations",
        "execution_events",
        "execution_attempts",
        "message_send_attempts",
        "tool_invocations",
        "execution_deliveries",
    ] {
        transaction.execute(
            &format!(
                "DELETE FROM {table} WHERE execution_id IN (
                    SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
                )"
            ),
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
    }
    transaction.execute(
        "DELETE FROM executions WHERE workspace_id=?1 AND agent_id=?2",
        params![workspace_id.as_str(), agent_id.as_str()],
    )?;
    for table in [
        "unread_snapshot_chats",
        "unread_snapshot_messages",
        "unread_snapshot_triggers",
    ] {
        transaction.execute(
            &format!(
                "DELETE FROM {table} WHERE snapshot_id IN (
                    SELECT snapshot_id FROM unread_snapshots WHERE workspace_id=?1 AND agent_id=?2
                )"
            ),
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
    }
    for table in [
        "unread_snapshots",
        "agent_inbox_items",
        "agent_chat_cursors",
        "agent_chat_attention",
        "agent_named_secrets",
    ] {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE workspace_id=?1 AND agent_id=?2"),
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
    }
    Ok(())
}

fn steer_delivery_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SteerDelivery> {
    Ok(SteerDelivery {
        delivery_id: -row.get::<_, i64>(0)?,
        execution_id: row.get(1)?,
        attempt: row.get(2)?,
        computer_id: row.get(3)?,
        computer_instance_id: row.get(4)?,
        rendered_markdown: row.get(5)?,
        created_at: row.get(6)?,
    })
}
fn migrate(c: &mut Connection) -> Result<()> {
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v > 15 {
        return Err(StorageError::Invalid(format!("unsupported schema {v}")));
    }
    if v == 0 {
        let t = c.transaction()?;
        t.execute_batch(SCHEMA)?;
        t.pragma_update(None, "user_version", 15)?;
        t.commit()?;
    } else if v == 1 {
        let t = c.transaction()?;
        t.execute_batch(MIGRATION_V2)?;
        t.pragma_update(None, "user_version", 2)?;
        t.commit()?;
    }
    if v == 1 || v == 2 {
        let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current == 2 {
            let t = c.transaction()?;
            t.execute_batch(MIGRATION_V3)?;
            t.pragma_update(None, "user_version", 3)?;
            t.commit()?;
        }
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 3 {
        let t = c.transaction()?;
        t.execute_batch(MIGRATION_V4)?;
        t.pragma_update(None, "user_version", 4)?;
        t.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 4 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V5)?;
        transaction.pragma_update(None, "user_version", 5)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 5 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V6)?;
        transaction.pragma_update(None, "user_version", 6)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 6 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V7)?;
        transaction.pragma_update(None, "user_version", 7)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 7 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V8)?;
        transaction.pragma_update(None, "user_version", 8)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 8 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V9)?;
        transaction.pragma_update(None, "user_version", 9)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 9 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V10)?;
        transaction.pragma_update(None, "user_version", 10)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 10 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V11)?;
        transaction.pragma_update(None, "user_version", 11)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 11 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V12)?;
        transaction.pragma_update(None, "user_version", 12)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 12 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V13)?;
        transaction.pragma_update(None, "user_version", 13)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 13 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V14)?;
        transaction.pragma_update(None, "user_version", 14)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 14 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V15)?;
        transaction.pragma_update(None, "user_version", 15)?;
        transaction.commit()?;
    }
    Ok(())
}
fn receive(t: &Transaction<'_>, m: &IncomingMessage) -> Result<IncomingMessageResult> {
    t.execute("INSERT INTO channels VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,channel_id)DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![m.workspace_id.as_str(),m.channel_id.as_str(),m.channel_name,m.now])?;
    t.execute("INSERT INTO chats(workspace_id,channel_id,provider_chat_id,name,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?5)ON CONFLICT(workspace_id,channel_id,provider_chat_id)DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![m.workspace_id.as_str(),m.channel_id.as_str(),m.provider_chat_id,m.chat_name,m.now])?;
    let ch: i64 = t.query_row(
        "SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3",
        params![
            m.workspace_id.as_str(),
            m.channel_id.as_str(),
            m.provider_chat_id
        ],
        |r| r.get(0),
    )?;
    if let Some(id) = t
        .query_row(
            "SELECT message_id FROM messages WHERE chat_id=?1 AND provider_message_id=?2",
            params![ch, m.provider_message_id],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(IncomingMessageResult {
            chat_id: ch,
            message_id: id,
            duplicate: true,
        });
    }
    t.execute("INSERT INTO messages(chat_id,provider_message_id,direction,sender_kind,sender_id,sender_name,content_markdown,send_status,occurred_at,created_at,updated_at)VALUES(?1,?2,'inbound','human',?3,?4,?5,'received',?6,?7,?7)",params![ch,m.provider_message_id,m.sender_id,m.sender_name,m.content,m.occurred_at,m.now])?;
    let id = t.last_insert_rowid();
    t.execute("UPDATE chats SET first_message_id=coalesce(first_message_id,?2),last_message_id=?2,updated_at=?3 WHERE chat_id=?1",params![ch,id,m.now])?;
    t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,available_at,state,created_at,updated_at)SELECT workspace_id,agent_id,?2,?3,?4,'pending',?4,?4 FROM agents WHERE workspace_id=?1",params![m.workspace_id.as_str(),ch,id,m.now])?;
    Ok(IncomingMessageResult {
        chat_id: ch,
        message_id: id,
        duplicate: false,
    })
}
fn state(c: &Connection, id: i64) -> Result<ExecutionState> {
    let s: String = c.query_row(
        "SELECT state FROM executions WHERE execution_id=?1",
        [id],
        |r| r.get(0),
    )?;
    ExecutionState::parse(&s)
}

fn load_execution_attempt(
    connection: &Connection,
    execution_id: i64,
    attempt: i64,
) -> Result<ExecutionAttemptView> {
    let row = connection.query_row(
        "SELECT computer_id,computer_instance_id,state,last_event_sequence,native_session_id,lease_expires_at FROM execution_attempts WHERE execution_id=?1 AND attempt=?2",
        params![execution_id, attempt],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, i64>(5)?)),
    )?;
    Ok(ExecutionAttemptView {
        execution_id,
        attempt,
        computer_id: row.0,
        computer_instance_id: row.1,
        state: ExecutionAttemptState::parse(&row.2)?,
        last_event_sequence: row.3 as u64,
        native_session_id: row.4,
        lease_expires_at: row.5,
    })
}

fn load_attempt_capabilities(
    connection: &Connection,
    execution_id: i64,
    attempt: i64,
) -> Result<Vec<AttemptCapability>> {
    let mut statement = connection.prepare(
        "SELECT name,display_name,description,kind,input_schema_json,output_schema_json FROM execution_attempt_capabilities WHERE execution_id=?1 AND attempt=?2 ORDER BY position",
    )?;
    Ok(statement
        .query_map(params![execution_id, attempt], |row| {
            Ok(AttemptCapability {
                name: row.get(0)?,
                display_name: row.get(1)?,
                description: row.get(2)?,
                kind: row.get(3)?,
                input_schema_json: row.get(4)?,
                output_schema_json: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn durable_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "started"
            | "native_session_bound"
            | "tool_started"
            | "tool_finished"
            | "usage"
            | "warning"
            | "completed"
            | "failed"
            | "cancelled"
    )
}

fn event_final_output(payload: &[u8]) -> String {
    serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|value| {
            value
                .get("final_output")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(payload).into_owned())
}

fn expand_execution_chat_boundary(
    transaction: &Transaction<'_>,
    execution_id: i64,
    snapshot_id: i64,
    chat_id: i64,
    read_to_message_id: i64,
    now: i64,
) -> Result<()> {
    let read_from_message_id: Option<i64> = transaction.query_row(
        "SELECT read_from_message_id FROM unread_snapshot_chats WHERE snapshot_id=?1 AND chat_id=?2",
        params![snapshot_id, chat_id],
        |row| row.get(0),
    )?;
    let unread_count: i64 = transaction.query_row(
        "SELECT count(*) FROM messages WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent')",
        params![chat_id, read_from_message_id.unwrap_or(0), read_to_message_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE unread_snapshot_chats SET read_to_message_id=max(read_to_message_id,?3),unread_count=?4 WHERE snapshot_id=?1 AND chat_id=?2",
        params![snapshot_id, chat_id, read_to_message_id, unread_count],
    )?;

    transaction.execute(
        "DELETE FROM unread_snapshot_messages WHERE snapshot_id=?1 AND chat_id=?2",
        params![snapshot_id, chat_id],
    )?;
    let mut recent = transaction.prepare(
        "SELECT message_id FROM messages WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent') ORDER BY message_id DESC LIMIT 3",
    )?;
    let mut recent_ids = recent
        .query_map(
            params![
                chat_id,
                read_from_message_id.unwrap_or(0),
                read_to_message_id
            ],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    recent_ids.reverse();
    drop(recent);
    for (position, message_id) in recent_ids.into_iter().enumerate() {
        transaction.execute(
            "INSERT INTO unread_snapshot_messages(snapshot_id,chat_id,message_id,position) VALUES(?1,?2,?3,?4)",
            params![snapshot_id, chat_id, message_id, position as i64],
        )?;
    }

    let (delivery_id, delivery_state): (i64, String) = transaction.query_row(
        "SELECT delivery_id,state FROM execution_deliveries WHERE execution_id=?1 ORDER BY delivery_id DESC LIMIT 1",
        [execution_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    transaction.execute(
        "INSERT INTO execution_delivery_chats(delivery_id,chat_id,read_to_message_id) VALUES(?1,?2,?3) ON CONFLICT(delivery_id,chat_id) DO UPDATE SET read_to_message_id=max(read_to_message_id,excluded.read_to_message_id)",
        params![delivery_id, chat_id, read_to_message_id],
    )?;
    let inbox_state = if delivery_state == "delivered" {
        "delivered"
    } else {
        "claimed"
    };

    let mut pending = transaction.prepare(
        "SELECT inbox_item_id FROM agent_inbox_items WHERE workspace_id=(SELECT workspace_id FROM executions WHERE execution_id=?1) AND agent_id=(SELECT agent_id FROM executions WHERE execution_id=?1) AND chat_id=?2 AND state='pending' AND available_at<=?3 ORDER BY inbox_item_id",
    )?;
    let pending_ids = pending
        .query_map(params![execution_id, chat_id, now], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(pending);
    for inbox_item_id in pending_ids {
        transaction.execute(
            "INSERT OR IGNORE INTO execution_delivery_items(delivery_id,inbox_item_id) VALUES(?1,?2)",
            params![delivery_id, inbox_item_id],
        )?;
        transaction.execute(
            "UPDATE agent_inbox_items SET state=?4,claimed_execution_id=?2,updated_at=?3 WHERE inbox_item_id=?1 AND state='pending'",
            params![inbox_item_id, execution_id, now, inbox_state],
        )?;
    }
    Ok(())
}

fn commit_execution_transaction(
    transaction: &Transaction<'_>,
    execution_id: i64,
    attempt: i64,
    output: &str,
    now: i64,
) -> Result<()> {
    let (workspace_id, agent_id, snapshot_id, current_attempt, state): (String, String, i64, i64, String) = transaction.query_row(
        "SELECT workspace_id,agent_id,snapshot_id,attempt,state FROM executions WHERE execution_id=?1",
        [execution_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    if attempt > 0 && current_attempt != attempt {
        return Err(StorageError::Conflict("stale execution attempt".into()));
    }
    if state == "succeeded" {
        return Ok(());
    }
    if !matches!(state.as_str(), "assigning" | "running" | "recovering") {
        return Err(StorageError::Conflict(
            "execution is not committable".into(),
        ));
    }
    let delivery_ready: i64 = transaction.query_row(
        "SELECT count(*) FROM execution_deliveries WHERE execution_id=?1 AND state='delivered'",
        [execution_id],
        |row| row.get(0),
    )?;
    if delivery_ready == 0 {
        return Err(StorageError::Conflict(
            "execution delivery is not durable".into(),
        ));
    }
    let mut statement = transaction.prepare("SELECT dc.chat_id,max(dc.read_to_message_id) FROM execution_delivery_chats dc JOIN execution_deliveries d USING(delivery_id) WHERE d.execution_id=?1 AND d.state='delivered' GROUP BY dc.chat_id")?;
    let cursors = statement
        .query_map([execution_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for (chat_id, read_to_message_id) in cursors {
        transaction.execute("INSERT INTO agent_chat_cursors VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,agent_id,chat_id) DO UPDATE SET read_to_message_id=max(read_to_message_id,excluded.read_to_message_id),updated_at=excluded.updated_at", params![workspace_id,agent_id,chat_id,read_to_message_id,now])?;
    }
    transaction.execute("UPDATE agent_inbox_items SET state='consumed',updated_at=?2 WHERE claimed_execution_id=?1 AND state='delivered'", params![execution_id,now])?;
    transaction.execute("UPDATE execution_deliveries SET state='consumed' WHERE execution_id=?1 AND state='delivered'", [execution_id])?;
    transaction.execute(
        "UPDATE unread_snapshots SET state='consumed',completed_at=?2 WHERE snapshot_id=?1",
        params![snapshot_id, now],
    )?;
    if attempt > 0 {
        transaction.execute("UPDATE execution_attempts SET state='completed',finished_at=?3 WHERE execution_id=?1 AND attempt=?2", params![execution_id,attempt,now])?;
    }
    transaction.execute("UPDATE executions SET state='succeeded',final_output=?2,completed_at=?3,updated_at=?3 WHERE execution_id=?1", params![execution_id,output,now])?;
    Ok(())
}

fn abandon_execution(
    transaction: &Transaction<'_>,
    execution_id: i64,
    end: ExecutionState,
    error: Option<&str>,
    now: i64,
) -> Result<()> {
    let snapshot_id: i64 = transaction.query_row(
        "SELECT snapshot_id FROM executions WHERE execution_id=?1",
        [execution_id],
        |row| row.get(0),
    )?;
    transaction.execute("UPDATE agent_inbox_items SET state='pending',claimed_execution_id=NULL,updated_at=?2 WHERE claimed_execution_id=?1 AND state!='consumed'", params![execution_id,now])?;
    transaction.execute(
        "UPDATE unread_snapshots SET state='abandoned',completed_at=?2 WHERE snapshot_id=?1",
        params![snapshot_id, now],
    )?;
    transaction.execute(
        "UPDATE execution_deliveries SET state='abandoned' WHERE execution_id=?1 AND state='pending'",
        [execution_id],
    )?;
    transaction.execute("UPDATE executions SET state=?2,error_code=?3,completed_at=?4,updated_at=?4 WHERE execution_id=?1", params![execution_id,end.as_str(),error,now])?;
    Ok(())
}

fn lose_attempt_transaction(
    transaction: &Transaction<'_>,
    execution_id: i64,
    attempt: i64,
    now: i64,
) -> Result<()> {
    let changed = transaction.execute("UPDATE execution_attempts SET state='lost',error_code='computer_lost',finished_at=?3 WHERE execution_id=?1 AND attempt=?2 AND state IN('offered','accepted','running')", params![execution_id,attempt,now])? == 1;
    if changed {
        abandon_execution(
            transaction,
            execution_id,
            ExecutionState::Failed,
            Some("computer_lost"),
            now,
        )?;
    }
    Ok(())
}
fn load_send(c: &Connection, k: &str) -> Result<Option<SendAttemptView>> {
    Ok(c.query_row("SELECT message_id,request_key,state,provider_message_id,error_code FROM message_send_attempts WHERE request_key=?1",[k],|r|Ok(SendAttemptView{created:false,message_id:r.get(0)?,request_key:r.get(1)?,state:r.get(2)?,provider_message_id:r.get(3)?,error_code:r.get(4)?})).optional()?)
}
fn load_workspace_record(c: &Connection, id: &WorkspaceId) -> Result<WorkspaceRecord> {
    let view = c.query_row(
        "SELECT * FROM workspaces WHERE workspace_id=?1",
        [id.as_str()],
        map_workspace,
    )?;
    let named: Option<i64> = c.query_row(
        "SELECT max(revision) FROM workspace_named_secrets WHERE workspace_id=?1",
        [id.as_str()],
        |r| r.get(0),
    )?;
    let legacy: Option<i64> = c
        .query_row(
            "SELECT updated_at FROM workspace_provider_secrets WHERE workspace_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    let revision = named.or(legacy);
    let (rules, cover_resource_id, setup_status, setup_error_code, provider_state_json): (
        String,
        Option<i64>,
        String,
        Option<String>,
        String,
    ) = c.query_row(
        "SELECT rules,cover_resource_id,setup_status,setup_error_code,provider_state_json FROM workspaces WHERE workspace_id=?1",
        [id.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    Ok(WorkspaceRecord {
        id: view.id.clone(),
        name: view.name,
        provider_id: view.provider_id,
        provider_config: view.provider_config,
        provider_state: serde_json::from_str(&provider_state_json)?,
        rules,
        cover_resource_id: cover_resource_id
            .map(ResourceId::new)
            .transpose()
            .map_err(|error| StorageError::Invalid(error.to_string()))?,
        setup_status: WorkspaceSetupStatus::parse(&setup_status)?,
        setup_error_code,
        secret_ref: revision.map(|revision| WorkspaceSecretRef {
            workspace_id: view.id,
            revision,
        }),
        active: view.active,
        created_at: view.created_at,
        updated_at: view.updated_at,
    })
}
fn load_agent_record(c: &Connection, w: &WorkspaceId, a: &AgentId) -> Result<AgentRecord> {
    let row:StoredAgentRow=c.query_row("SELECT name,agent_id,created_at,updated_at,role,bio,avatar,soul,computer_id,harness_id,model,thinking,auto_reaction,cli_path,env_json,provider_agent_config_json,session_config_json,attention_json,provider_state_json FROM agents WHERE workspace_id=?1 AND agent_id=?2",params![w.as_str(),a.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?,r.get(15)?,r.get(16)?,r.get(17)?,r.get(18)?)))?;
    Ok(AgentRecord {
        workspace_id: w.clone(),
        agent_id: aid(row.1)?,
        name: row.0,
        created_at: row.2,
        updated_at: row.3,
        role: row.4,
        bio: row.5,
        avatar: row.6,
        soul: row.7,
        computer_id: row.8,
        harness_id: row.9,
        model: row.10,
        thinking: row.11,
        auto_reaction: row.12,
        cli_path: row.13,
        env: serde_json::from_str(&row.14)?,
        provider_agent_config: serde_json::from_str(&row.15)?,
        provider_state: serde_json::from_str(&row.18)?,
        session_config: serde_json::from_str(&row.16)?,
        attention: serde_json::from_str(&row.17)?,
    })
}
fn load_resource_metadata(
    c: &Connection,
    w: &WorkspaceId,
    id: ResourceId,
) -> Result<Option<ResourceMetadata>> {
    let row:Option<(String,String,i64,String,String,i64)>=c.query_row("SELECT file_name,media_type,size_bytes,sha256,storage_ref,created_at FROM resource_metadata WHERE workspace_id=?1 AND resource_id=?2",params![w.as_str(),id.get()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    Ok(row.map(
        |(file_name, media_type, size_bytes, sha256, storage_ref, created_at)| ResourceMetadata {
            workspace_id: w.clone(),
            resource_id: id,
            file_name,
            media_type,
            size_bytes: size_bytes as u64,
            sha256,
            storage_ref,
            created_at,
        },
    ))
}
fn load_recoverable_executions(
    c: &Connection,
    workspace_id: Option<&WorkspaceId>,
) -> Result<Vec<RecoverableExecutionView>> {
    let mut sql="SELECT e.execution_id,e.snapshot_id,e.state,e.final_output,e.error_code,d.delivery_id,d.state FROM executions e JOIN execution_deliveries d USING(execution_id) WHERE e.state IN('queued','assigning','running','recovering')".to_owned();
    if workspace_id.is_some() {
        sql.push_str(" AND e.workspace_id=?1");
    }
    sql.push_str(" ORDER BY e.execution_id");
    let mut s = c.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RecoverableExecutionRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        ))
    };
    let rows = if let Some(w) = workspace_id {
        s.query_map([w.as_str()], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        s.query_map([], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    drop(s);
    let mut out = Vec::new();
    for (execution_id, snapshot_id, state, final_output, error_code, delivery_id, delivery_state) in
        rows
    {
        out.push(RecoverableExecutionView {
            execution: ExecutionView {
                execution_id,
                snapshot_id,
                state: ExecutionState::parse(&state)?,
                final_output,
                error_code,
            },
            snapshot: load_snapshot(c, snapshot_id)?,
            delivery_id,
            delivery_state,
        });
    }
    Ok(out)
}
fn load_recoverable_sends(
    c: &Connection,
    workspace_id: Option<&WorkspaceId>,
) -> Result<Vec<RecoverableSendView>> {
    let mut sql="SELECT a.message_id,a.request_key,a.state,a.provider_message_id,a.error_code,c.workspace_id,w.provider_id,c.channel_id,c.provider_chat_id,m.sender_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats c USING(chat_id) JOIN workspaces w USING(workspace_id) WHERE a.state IN('sending','unknown')".to_owned();
    if workspace_id.is_some() {
        sql.push_str(" AND c.workspace_id=?1");
    }
    sql.push_str(" ORDER BY a.send_attempt_id");
    let mut s = c.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RecoverableSendRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
            r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        ))
    };
    let rows = if let Some(w) = workspace_id {
        s.query_map([w.as_str()], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        s.query_map([], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut out = Vec::new();
    for (
        message_id,
        request_key,
        state,
        provider_message_id,
        error_code,
        workspace,
        provider,
        channel,
        provider_chat_id,
        sender,
        content,
    ) in rows
    {
        out.push(RecoverableSendView {
            attempt: SendAttemptView {
                created: false,
                message_id,
                request_key,
                state,
                provider_message_id,
                error_code,
            },
            workspace_id: wid(workspace)?,
            provider_id: pid(provider)?,
            channel_id: cid(channel)?,
            provider_chat_id,
            sender_agent_id: aid(sender)?,
            content,
        });
    }
    Ok(out)
}
fn ensure_workspace_provider(c: &Connection, w: &WorkspaceId, p: &ProviderId) -> Result<()> {
    let actual: String = c.query_row(
        "SELECT provider_id FROM workspaces WHERE workspace_id=?1",
        [w.as_str()],
        |r| r.get(0),
    )?;
    if actual != p.as_str() {
        return Err(StorageError::Invalid(format!(
            "provider mismatch for workspace {w}"
        )));
    }
    Ok(())
}
fn validate_dispatch(dispatch: &[InboxDispatch]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for item in dispatch {
        if !seen.insert(item.agent_id.as_str()) {
            return Err(StorageError::Invalid(format!(
                "duplicate dispatch agent {}",
                item.agent_id
            )));
        }
    }
    Ok(())
}
fn sender_kind(kind: MessageAuthorType) -> &'static str {
    match kind {
        MessageAuthorType::Human => "human",
        MessageAuthorType::Agent => "agent",
        MessageAuthorType::Bot => "bot",
        MessageAuthorType::System => "system",
    }
}
fn parse_sender_kind(value: &str) -> Result<MessageAuthorType> {
    match value {
        "human" => Ok(MessageAuthorType::Human),
        "agent" => Ok(MessageAuthorType::Agent),
        "bot" => Ok(MessageAuthorType::Bot),
        "system" => Ok(MessageAuthorType::System),
        _ => Err(StorageError::Invalid(format!("sender kind {value}"))),
    }
}
fn reaction_action(value: ReactionAction) -> &'static str {
    match value {
        ReactionAction::Add => "add",
        ReactionAction::Remove => "remove",
    }
}
fn reaction_origin(value: ReactionOrigin) -> &'static str {
    match value {
        ReactionOrigin::Provider => "provider",
        ReactionOrigin::AutoReaction => "auto_reaction",
    }
}
fn load_message(c: &Connection, message_id: i64) -> Result<MessageView> {
    let tuple:StoredMessageRow=c.query_row("SELECT provider_message_id,sender_kind,sender_id,sender_name,content_markdown,mentions_json,reply_to_provider_message_id,resources_json,occurred_at FROM messages WHERE message_id=?1",[message_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))?;
    Ok(MessageView {
        message_id,
        provider_message_id: tuple.0,
        sender_kind: parse_sender_kind(&tuple.1)?,
        sender_id: tuple.2.unwrap_or_default(),
        sender_name: tuple.3.unwrap_or_default(),
        content: tuple.4.unwrap_or_default(),
        mentions: serde_json::from_str(&tuple.5)?,
        reply_to_provider_message_id: tuple.6,
        resources: serde_json::from_str(&tuple.7)?,
        occurred_at: tuple.8,
    })
}
fn load_snapshot(c: &Connection, snapshot_id: i64) -> Result<SnapshotView> {
    let (workspace, agent, created_at): (String, String, i64) = c.query_row(
        "SELECT workspace_id,agent_id,created_at FROM unread_snapshots WHERE snapshot_id=?1",
        [snapshot_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let workspace_id = wid(workspace)?;
    let provider_id: ProviderId = pid(c.query_row(
        "SELECT provider_id FROM workspaces WHERE workspace_id=?1",
        [workspace_id.as_str()],
        |r| r.get(0),
    )?)?;
    let mut statement=c.prepare("SELECT sc.chat_id,ch.channel_id,cn.name,ch.provider_chat_id,ch.name,sc.read_from_message_id,sc.read_to_message_id,sc.unread_count FROM unread_snapshot_chats sc JOIN chats ch USING(chat_id) JOIN channels cn ON cn.workspace_id=ch.workspace_id AND cn.channel_id=ch.channel_id WHERE sc.snapshot_id=?1 ORDER BY sc.chat_id")?;
    let rows = statement
        .query_map([snapshot_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    let mut chats = Vec::new();
    for (
        chat_id,
        channel,
        channel_name,
        provider_chat_id,
        chat_name,
        read_from_message_id,
        read_to_message_id,
        unread_count,
    ) in rows
    {
        let mut recent_stmt=c.prepare("SELECT message_id FROM unread_snapshot_messages WHERE snapshot_id=?1 AND chat_id=?2 ORDER BY position")?;
        let recent_ids = recent_stmt
            .query_map(params![snapshot_id, chat_id], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(recent_stmt);
        let mut recent_messages = Vec::new();
        for id in recent_ids {
            recent_messages.push(load_message(c, id)?);
        }
        let mut trigger_stmt=c.prepare("SELECT message_id,reason_json FROM unread_snapshot_triggers WHERE snapshot_id=?1 AND chat_id=?2 ORDER BY position")?;
        let trigger_rows = trigger_stmt
            .query_map(params![snapshot_id, chat_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(trigger_stmt);
        let mut trigger_messages = Vec::new();
        let mut trigger_reasons = Vec::new();
        let mut mentioned_message = None;
        for (id, reason) in trigger_rows {
            let message = load_message(c, id)?;
            let reason = serde_json::from_str(&reason)?;
            if reason == DispatchReason::Mention {
                mentioned_message = Some(message.clone());
            }
            trigger_messages.push(message);
            trigger_reasons.push(reason);
        }
        chats.push(FrozenSnapshotChatView {
            chat: ChatRefView {
                chat_id,
                workspace_id: workspace_id.clone(),
                provider_id: provider_id.clone(),
                channel_id: cid(channel)?,
                channel_name,
                provider_chat_id,
                chat_name,
            },
            read_from_message_id,
            read_to_message_id,
            unread_count: unread_count as usize,
            recent_messages,
            mentioned_message,
            trigger_messages,
            trigger_reasons,
        });
    }
    Ok(SnapshotView {
        snapshot_id,
        workspace_id,
        agent_id: aid(agent)?,
        created_at,
        chats,
    })
}
fn map_workspace(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceView> {
    let c: String = r.get(3)?;
    Ok(WorkspaceView {
        id: wid(r.get(0)?)?,
        name: r.get(1)?,
        provider_id: pid(r.get(2)?)?,
        provider_config: serde_json::from_str(&c).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?,
        active: r.get::<_, String>(4)? == "running",
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}
fn conv(e: lankide_core::DomainError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}
fn wid(s: String) -> rusqlite::Result<WorkspaceId> {
    WorkspaceId::new(s).map_err(conv)
}
fn aid(s: String) -> rusqlite::Result<AgentId> {
    AgentId::new(s).map_err(conv)
}
fn pid(s: String) -> rusqlite::Result<ProviderId> {
    ProviderId::new(s).map_err(conv)
}
fn cid(s: String) -> rusqlite::Result<ChannelId> {
    ChannelId::new(s).map_err(conv)
}
fn person_id(s: String) -> rusqlite::Result<PersonId> {
    PersonId::new(s).map_err(conv)
}

fn map_person(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersonRecord> {
    let role = row.get::<_, String>(2)?;
    let provider_state = row.get::<_, String>(5)?;
    Ok(PersonRecord {
        workspace_id: wid(row.get(0)?)?,
        person_id: person_id(row.get(1)?)?,
        team_role: TeamRole::parse(&role).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        provider_instance_id: pid(row.get(3)?)?,
        external_id: row.get(4)?,
        provider_identity_state: serde_json::from_str(&provider_state).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        provider_state_revision: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn load_person_by_external(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    provider_instance_id: &ProviderId,
    external_id: &str,
) -> Result<Option<PersonRecord>> {
    Ok(connection.query_row(
        "SELECT workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at FROM people WHERE workspace_id=?1 AND provider_instance_id=?2 AND external_id=?3",
        params![workspace_id.as_str(), provider_instance_id.as_str(), external_id],
        map_person,
    ).optional()?)
}

fn load_person_by_id(
    connection: &Connection,
    workspace_id: &WorkspaceId,
    person_id: &PersonId,
) -> Result<Option<PersonRecord>> {
    Ok(connection.query_row(
        "SELECT workspace_id,person_id,team_role,provider_instance_id,external_id,provider_identity_state_json,provider_state_revision,created_at,updated_at FROM people WHERE workspace_id=?1 AND person_id=?2",
        params![workspace_id.as_str(), person_id.as_str()],
        map_person,
    ).optional()?)
}

const SCHEMA: &str = r#"
CREATE TABLE workspaces(workspace_id TEXT PRIMARY KEY,name TEXT NOT NULL,provider_id TEXT NOT NULL,provider_config_json TEXT NOT NULL,desired_state TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,rules TEXT NOT NULL DEFAULT '',cover_resource_id INTEGER,setup_status TEXT NOT NULL DEFAULT 'pending',setup_error_code TEXT,provider_state_json TEXT NOT NULL DEFAULT '{}');
CREATE TABLE workspace_provider_secrets(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,secret_json TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE workspace_named_secrets(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,name));
CREATE TABLE agent_named_secrets(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,name),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
CREATE TABLE provider_kv(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,namespace TEXT NOT NULL,key TEXT NOT NULL,value BLOB NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,namespace,key));
CREATE TABLE resource_metadata(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,resource_id INTEGER NOT NULL,file_name TEXT NOT NULL,media_type TEXT NOT NULL,size_bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,storage_ref TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,resource_id));
CREATE TABLE workspace_resource_counters(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,next_id INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE agents(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),agent_id TEXT NOT NULL,name TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,role TEXT NOT NULL DEFAULT '',bio TEXT NOT NULL DEFAULT '',avatar TEXT,soul TEXT NOT NULL DEFAULT '',computer_id TEXT NOT NULL DEFAULT '',harness_id TEXT NOT NULL DEFAULT '',model TEXT NOT NULL DEFAULT '',thinking TEXT NOT NULL DEFAULT '',auto_reaction TEXT,cli_path TEXT,env_json TEXT NOT NULL DEFAULT '{}',provider_agent_config_json TEXT NOT NULL DEFAULT '{}',session_config_json TEXT NOT NULL DEFAULT '{}',attention_json TEXT NOT NULL DEFAULT '{}',provider_state_json TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(workspace_id,agent_id));
CREATE TABLE agent_file_deletions(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,computer_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id));
CREATE TABLE people(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,person_id TEXT NOT NULL,team_role TEXT NOT NULL CHECK(team_role IN('external','member','lead')),provider_instance_id TEXT NOT NULL,external_id TEXT NOT NULL,provider_identity_state_json TEXT NOT NULL DEFAULT '{}',provider_state_revision INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,person_id),UNIQUE(workspace_id,provider_instance_id,external_id));
CREATE UNIQUE INDEX people_one_lead_per_workspace ON people(workspace_id) WHERE team_role='lead';
CREATE TABLE channels(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),channel_id TEXT NOT NULL,name TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,channel_id));
CREATE TABLE chats(chat_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,channel_id TEXT NOT NULL,provider_chat_id TEXT NOT NULL,name TEXT,first_message_id INTEGER,last_message_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,channel_id)REFERENCES channels(workspace_id,channel_id),UNIQUE(workspace_id,channel_id,provider_chat_id));
CREATE TABLE messages(message_id INTEGER PRIMARY KEY AUTOINCREMENT,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),provider_message_id TEXT,provider_event_id TEXT,direction TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT,sender_name TEXT,content_markdown TEXT,mentions_json TEXT NOT NULL DEFAULT '[]',reply_to_provider_message_id TEXT,resources_json TEXT NOT NULL DEFAULT '[]',send_status TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);CREATE UNIQUE INDEX message_provider_key ON messages(chat_id,provider_message_id)WHERE provider_message_id IS NOT NULL;
CREATE TABLE reactions(reaction_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),target_message_id INTEGER NOT NULL REFERENCES messages(message_id),provider_event_id TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT NOT NULL,reaction TEXT NOT NULL,action TEXT NOT NULL,origin TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,UNIQUE(workspace_id,provider_event_id));
CREATE TABLE agent_inbox_items(inbox_item_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),source_id INTEGER NOT NULL,source_kind TEXT NOT NULL DEFAULT 'message',trigger_reason_json TEXT NOT NULL DEFAULT '{"kind":"follow"}',available_at INTEGER NOT NULL,state TEXT NOT NULL,claimed_execution_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id),UNIQUE(workspace_id,agent_id,source_id));
CREATE TABLE agent_chat_cursors(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE agent_chat_attention(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),attention_json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE unread_snapshots(snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,completed_at INTEGER,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE unread_snapshot_chats(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_from_message_id INTEGER,read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),unread_count INTEGER NOT NULL,trigger_summary TEXT NOT NULL,PRIMARY KEY(snapshot_id,chat_id));
CREATE TABLE unread_snapshot_messages(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,chat_id,message_id));
CREATE TABLE unread_snapshot_triggers(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),reason_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,position));
CREATE TABLE executions(execution_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),state TEXT NOT NULL,trigger_summary TEXT NOT NULL,final_output TEXT,error_code TEXT,created_at INTEGER NOT NULL,started_at INTEGER,completed_at INTEGER,updated_at INTEGER NOT NULL,attempt INTEGER NOT NULL DEFAULT 1,token_hash TEXT,token_expires_at INTEGER,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));CREATE UNIQUE INDEX one_active_execution ON executions(workspace_id,agent_id)WHERE state IN('queued','assigning','running','recovering');
CREATE TABLE execution_attempts(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,state TEXT NOT NULL,last_event_sequence INTEGER NOT NULL DEFAULT 0,native_session_id TEXT,error_code TEXT,error_message TEXT,lease_expires_at INTEGER NOT NULL,assigned_at INTEGER NOT NULL,accepted_at INTEGER,finished_at INTEGER,PRIMARY KEY(execution_id,attempt));
CREATE INDEX active_attempt_lease ON execution_attempts(computer_id,lease_expires_at) WHERE state IN('offered','accepted','running');
CREATE TABLE execution_events(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,sequence INTEGER NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL,occurred_at INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,sequence),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt));
CREATE TABLE steer_deliveries(steer_delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,rendered_markdown TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,accepted_at INTEGER,last_error_at INTEGER,FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt),UNIQUE(execution_id,attempt,inbox_item_id));
CREATE TABLE execution_attempt_authorizations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,context_json TEXT NOT NULL,context_fingerprint TEXT NOT NULL,PRIMARY KEY(execution_id,attempt));
CREATE TABLE execution_attempt_capabilities(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,name TEXT NOT NULL,display_name TEXT NOT NULL,description TEXT NOT NULL,kind INTEGER NOT NULL,input_schema_json TEXT NOT NULL,output_schema_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,name),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempt_authorizations(execution_id,attempt));
CREATE TABLE execution_deliveries(delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL REFERENCES executions(execution_id),state TEXT NOT NULL,created_at INTEGER NOT NULL,delivered_at INTEGER);CREATE TABLE execution_delivery_items(delivery_id INTEGER NOT NULL REFERENCES execution_deliveries(delivery_id),inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),PRIMARY KEY(delivery_id,inbox_item_id));CREATE TABLE execution_delivery_chats(delivery_id INTEGER NOT NULL REFERENCES execution_deliveries(delivery_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),PRIMARY KEY(delivery_id,chat_id));
CREATE TABLE message_send_attempts(send_attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,message_id INTEGER NOT NULL REFERENCES messages(message_id),execution_id INTEGER REFERENCES executions(execution_id),request_key TEXT NOT NULL UNIQUE,state TEXT NOT NULL,provider_message_id TEXT,error_code TEXT,started_at INTEGER NOT NULL,finished_at INTEGER);CREATE TABLE tool_invocations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,tool_call_id TEXT NOT NULL,tool_name TEXT NOT NULL,arguments_json TEXT NOT NULL,state TEXT NOT NULL,result_json TEXT,started_at INTEGER NOT NULL,finished_at INTEGER,PRIMARY KEY(execution_id,attempt,tool_call_id));
"#;

const MIGRATION_V2: &str = r#"
ALTER TABLE messages ADD COLUMN provider_event_id TEXT;
ALTER TABLE messages ADD COLUMN mentions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN reply_to_provider_message_id TEXT;
ALTER TABLE messages ADD COLUMN resources_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agent_inbox_items ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'message';
ALTER TABLE agent_inbox_items ADD COLUMN trigger_reason_json TEXT NOT NULL DEFAULT '{"kind":"follow"}';
ALTER TABLE message_send_attempts ADD COLUMN error_code TEXT;
ALTER TABLE message_send_attempts ADD COLUMN finished_at INTEGER;
CREATE TABLE reactions(reaction_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),target_message_id INTEGER NOT NULL REFERENCES messages(message_id),provider_event_id TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT NOT NULL,reaction TEXT NOT NULL,action TEXT NOT NULL,origin TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,UNIQUE(workspace_id,provider_event_id));
CREATE TABLE unread_snapshot_triggers(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),reason_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,position));
"#;
const MIGRATION_V3: &str = r#"
ALTER TABLE message_send_attempts ADD COLUMN execution_id INTEGER REFERENCES executions(execution_id);
"#;
const MIGRATION_V4: &str = r#"
ALTER TABLE agents ADD COLUMN role TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN bio TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN avatar TEXT;
ALTER TABLE agents ADD COLUMN soul TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN computer_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN harness_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN model TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN thinking TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN auto_reaction TEXT;
ALTER TABLE agents ADD COLUMN cli_path TEXT;
ALTER TABLE agents ADD COLUMN env_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN provider_agent_config_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN session_config_json TEXT NOT NULL DEFAULT '{}';
CREATE TABLE workspace_named_secrets(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,name));
CREATE TABLE provider_kv(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,namespace TEXT NOT NULL,key TEXT NOT NULL,value BLOB NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,namespace,key));
CREATE TABLE resource_metadata(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,resource_id INTEGER NOT NULL,file_name TEXT NOT NULL,media_type TEXT NOT NULL,size_bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,storage_ref TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,resource_id));
"#;
const MIGRATION_V5: &str = r#"
ALTER TABLE workspaces ADD COLUMN rules TEXT NOT NULL DEFAULT '';
ALTER TABLE workspaces ADD COLUMN cover_resource_id INTEGER;
ALTER TABLE workspaces ADD COLUMN setup_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE workspaces ADD COLUMN setup_error_code TEXT;
ALTER TABLE agents ADD COLUMN attention_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE executions ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;
ALTER TABLE executions ADD COLUMN token_hash TEXT;
ALTER TABLE executions ADD COLUMN token_expires_at INTEGER;
CREATE TABLE workspace_resource_counters(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,next_id INTEGER NOT NULL,updated_at INTEGER NOT NULL);
"#;
const MIGRATION_V6: &str = r#"
CREATE TABLE execution_attempts(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,state TEXT NOT NULL,last_event_sequence INTEGER NOT NULL DEFAULT 0,native_session_id TEXT,error_code TEXT,error_message TEXT,lease_expires_at INTEGER NOT NULL,assigned_at INTEGER NOT NULL,accepted_at INTEGER,finished_at INTEGER,PRIMARY KEY(execution_id,attempt));
CREATE INDEX active_attempt_lease ON execution_attempts(computer_id,lease_expires_at) WHERE state IN('offered','accepted','running');
CREATE TABLE execution_events(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,sequence INTEGER NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL,occurred_at INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,sequence),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt));
"#;
const MIGRATION_V7: &str = r#"
CREATE TABLE execution_attempt_authorizations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,context_json TEXT NOT NULL,context_fingerprint TEXT NOT NULL,PRIMARY KEY(execution_id,attempt));
CREATE TABLE execution_attempt_capabilities(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,name TEXT NOT NULL,display_name TEXT NOT NULL,description TEXT NOT NULL,kind INTEGER NOT NULL,input_schema_json TEXT NOT NULL,output_schema_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,name),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempt_authorizations(execution_id,attempt));
"#;
const MIGRATION_V8: &str = r#"
CREATE TABLE steer_deliveries(steer_delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,rendered_markdown TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,accepted_at INTEGER,last_error_at INTEGER,FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt),UNIQUE(execution_id,attempt,inbox_item_id));
"#;
const MIGRATION_V9: &str = r#"
CREATE TABLE agent_named_secrets(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,name),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
"#;
const MIGRATION_V10: &str = r#"
ALTER TABLE workspaces ADD COLUMN provider_state_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN provider_state_json TEXT NOT NULL DEFAULT '{}';
"#;
const MIGRATION_V11: &str = r#"
CREATE TABLE agent_chat_attention(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),attention_json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
"#;
const MIGRATION_V12: &str = r#"
ALTER TABLE agents ADD COLUMN deleted_at INTEGER;
"#;
const MIGRATION_V13: &str = r#"
CREATE TABLE people(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,person_id TEXT NOT NULL,team_role TEXT NOT NULL CHECK(team_role IN('external','member','lead')),provider_instance_id TEXT NOT NULL,external_id TEXT NOT NULL,provider_identity_state_json TEXT NOT NULL DEFAULT '{}',provider_state_revision INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,person_id),UNIQUE(workspace_id,provider_instance_id,external_id));
CREATE UNIQUE INDEX people_one_lead_per_workspace ON people(workspace_id) WHERE team_role='lead';
"#;
const MIGRATION_V14: &str = r#"
CREATE TABLE agent_file_deletions(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,computer_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
"#;
const MIGRATION_V15: &str = r#"
CREATE TEMP TABLE deleted_agents_v15 AS
SELECT workspace_id,agent_id FROM agents WHERE deleted_at IS NOT NULL;

CREATE TABLE agent_file_deletions_v15(
    workspace_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    computer_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id,agent_id)
);
INSERT INTO agent_file_deletions_v15(workspace_id,agent_id,computer_id,created_at)
SELECT workspace_id,agent_id,computer_id,created_at FROM agent_file_deletions;
DROP TABLE agent_file_deletions;
ALTER TABLE agent_file_deletions_v15 RENAME TO agent_file_deletions;

DELETE FROM execution_delivery_items
WHERE delivery_id IN (
    SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
        SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
    )
) OR inbox_item_id IN (
    SELECT inbox_item_id FROM agent_inbox_items JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_delivery_chats WHERE delivery_id IN (
    SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
        SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
    )
);
DELETE FROM steer_deliveries
WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
) OR inbox_item_id IN (
    SELECT inbox_item_id FROM agent_inbox_items JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_attempt_capabilities WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_attempt_authorizations WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_events WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_attempts WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM message_send_attempts WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM tool_invocations WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_deliveries WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM executions WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=executions.workspace_id
      AND deleted_agents_v15.agent_id=executions.agent_id
);
DELETE FROM unread_snapshot_chats WHERE snapshot_id IN (
    SELECT snapshot_id FROM unread_snapshots JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM unread_snapshot_messages WHERE snapshot_id IN (
    SELECT snapshot_id FROM unread_snapshots JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM unread_snapshot_triggers WHERE snapshot_id IN (
    SELECT snapshot_id FROM unread_snapshots JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM unread_snapshots WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=unread_snapshots.workspace_id
      AND deleted_agents_v15.agent_id=unread_snapshots.agent_id
);
DELETE FROM agent_inbox_items WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_inbox_items.workspace_id
      AND deleted_agents_v15.agent_id=agent_inbox_items.agent_id
);
DELETE FROM agent_chat_cursors WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_chat_cursors.workspace_id
      AND deleted_agents_v15.agent_id=agent_chat_cursors.agent_id
);
DELETE FROM agent_chat_attention WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_chat_attention.workspace_id
      AND deleted_agents_v15.agent_id=agent_chat_attention.agent_id
);
DELETE FROM agent_named_secrets WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_named_secrets.workspace_id
      AND deleted_agents_v15.agent_id=agent_named_secrets.agent_id
);
DELETE FROM agents WHERE deleted_at IS NOT NULL;
DROP TABLE deleted_agents_v15;
ALTER TABLE agents DROP COLUMN deleted_at;
"#;
