//! Management JSON contract. Canonical serde DTOs for management.
pub use crate::wire::{
    AttentionMode, DesiredState, HarnessAvailability, ManagementEventKind, ObservedState,
    OnboardingStatus,
};
use serde::{Deserialize, Serialize};
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentFirstReplyState {
    #[default]
    Waiting,
    Replied,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFirstReply {
    pub state: AgentFirstReplyState,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanTeamMember {
    pub person_id: String,
    pub team_role: String,
    pub name: Option<String>,
    pub avatar: Option<ResourceRef>,
    pub position: String,
    pub signature: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateHumanPositionRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub person_id: String,
    pub position: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyncHumanProfileRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub person_id: String,
}

/// Input for the Feishu workspace task-list management interface.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceTasksInput {
    List,
    Rename { name: String },
}

pub const PREFIX: &str = "/api/v1/";
pub const EVENTS_PATH: &str = "/api/v1/events";
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvanceAgentOnboardingRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub action: String,
    pub field_values_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvanceProviderOnboardingRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub action: String,
    pub field_values_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Agent {
    pub figure: String,
    pub launch_arguments: Vec<String>,
    pub activity: AgentActivity,
    pub agent: Option<AgentRef>,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar_path: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    pub auto_reaction: String,
    pub soul: String,
    pub cli_path: String,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub machine_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub attention: Option<AgentAttention>,
    pub provider_agent_config_json: String,
    pub pause_reason: Option<String>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAttention {
    pub channels: Vec<ChannelAttention>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAvatarFile {
    pub path: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub bytes: Vec<u8>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAvatarUpload {
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub bytes: Vec<u8>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentOnboardingState {
    pub agent: Option<AgentRef>,
    pub provider_id: String,
    pub step: String,
    pub status: OnboardingStatus,
    pub payload_json: String,
    pub verification_uri: String,
    pub poll_interval_seconds: u32,
    pub error_code: String,
    pub error_message: String,
}

pub use crate::link::AgentRef;

pub use crate::link::AttemptFailure;

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelAttemptRequest {
    pub request_id: String,
    pub execution_id: i64,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelAttemptResponse {}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelAttention {
    pub channel_id: String,
    pub mode: AttentionMode,
    pub interval_minutes: u32,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineCatalog {
    pub machines: Vec<MachineCatalogItem>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineCatalogItem {
    pub is_local: bool,
    pub machine_id: String,
    pub display_name: String,
    pub observed_state: ObservedState,
    pub harnesses: Vec<HarnessInfo>,
    pub status_message: String,
    pub registered: bool,
    pub revoked_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub paired_at: i64,
    pub os: String,
    pub version: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateAgentRequest {
    #[serde(default)]
    pub figure: String,
    #[serde(default)]
    pub launch_arguments: Vec<String>,
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    pub auto_reaction: String,
    pub soul: String,
    pub cli_path: String,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub machine_id: String,
    pub attention: Option<AgentAttention>,
    pub provider_agent_config_json: String,
    pub provider_agent_secrets_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateWorkspaceRequest {
    pub request_id: String,
    pub workspace_id: String,
    pub name: String,
    pub rules: String,
    pub provider_id: String,
    pub provider_config_json: String,
    pub provider_secrets_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAgentRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAgentResponse {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

pub use crate::link::EnvironmentVariable;

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentVariableSet {
    pub entries: Vec<EnvironmentVariable>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAttemptDetails {
    pub attempt: u32,
    pub machine_id: String,
    pub state: String,
    pub failure: Option<AttemptFailure>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionDetails {
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub execution_id: i64,
    pub state: String,
    pub attempts: Vec<ExecutionAttemptDetails>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentAvatarRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentAvatarResponse {
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub avatar: Option<AgentAvatarFile>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentOnboardingRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetExecutionDetailsRequest {
    pub request_id: String,
    pub execution_id: i64,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetProviderOnboardingRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetWorkspaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessCatalog {
    pub harnesses: Vec<HarnessCatalogItem>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessCatalogItem {
    pub harness_id: String,
    pub display_name: String,
    pub description: String,
    pub availability: HarnessAvailability,
    pub supported_thinking_levels: Vec<String>,
    #[serde(default)]
    pub default_environment_variables: Vec<EnvironmentVariable>,
    #[serde(default)]
    pub default_launch_arguments: Vec<String>,
}

/// Management inventory uses string enums; Machine link inventory keeps numeric fields.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessInfo {
    pub harness_id: String,
    pub display_name: String,
    pub availability: HarnessAvailability,
    pub version: String,
    pub reason: String,
    pub model_catalog: Option<ModelCatalog>,
    pub model_catalog_error: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub default_environment_variables: Vec<EnvironmentVariable>,
    #[serde(default)]
    pub default_launch_arguments: Vec<String>,
}
impl TryFrom<crate::link::HarnessInfo> for HarnessInfo {
    type Error = crate::Status;
    fn try_from(value: crate::link::HarnessInfo) -> Result<Self, Self::Error> {
        Ok(Self {
            harness_id: value.harness_id,
            display_name: value.display_name,
            availability: HarnessAvailability::try_from(value.availability)?,
            version: value.version,
            reason: value.reason,
            model_catalog: value.model_catalog,
            model_catalog_error: value.model_catalog_error,
            capabilities: value.capabilities,
            default_environment_variables: value.default_environment_variables,
            default_launch_arguments: value.default_launch_arguments,
        })
    }
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvokeProviderInterfaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interface_id: String,
    pub input_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvokeProviderInterfaceResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interface_id: String,
    pub output_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListAgentsRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListAgentsResponse {
    pub request_id: String,
    pub agents: Vec<Agent>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListProviderInterfacesRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListProviderInterfacesResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interfaces: Vec<ProviderInterface>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListWorkspacesRequest {
    pub request_id: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListWorkspacesResponse {
    pub request_id: String,
    pub workspaces: Vec<Workspace>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagementEvent {
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: ManagementEventKind,
    pub workspace: Option<WorkspaceRef>,
    pub agent: Option<AgentRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagementSubscription {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub after_sequence: u64,
}

pub use crate::link::ModelCatalog;

pub use crate::link::ModelCatalogItem;

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCatalogRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub machine_id: String,
    pub harness_id: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCatalog {
    pub providers: Vec<ProviderCatalogItem>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCatalogItem {
    pub provider_id: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
    pub capabilities: Vec<String>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderInterface {
    pub interface_id: String,
    pub description: Option<String>,
    pub input_schema_json: String,
    pub output_schema_json: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderOnboardingState {
    pub workspace: Option<WorkspaceRef>,
    pub provider_id: String,
    pub step: String,
    pub status: OnboardingStatus,
    pub payload_json: String,
    pub verification_uri: String,
    pub poll_interval_seconds: u32,
    pub error_code: String,
    pub error_message: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionCatalog {
    pub reactions: Vec<ReactionCatalogItem>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionCatalogItem {
    pub reaction_id: String,
    pub display_name: String,
    pub emoji: String,
    pub available: bool,
}

pub use crate::link::ResourceRef;

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameMachineRequest {
    pub request_id: String,
    pub machine_id: String,
    pub name: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeMachineRequest {
    pub request_id: String,
    pub machine_id: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetWorkspaceStateRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub desired_state: DesiredState,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateAgentRequest {
    pub figure: Option<String>,
    pub launch_arguments: Option<Vec<String>>,
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub bio: Option<String>,
    pub harness_id: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub auto_reaction: Option<String>,
    pub soul: Option<String>,
    pub cli_path: Option<String>,
    pub environment_variables: Option<EnvironmentVariableSet>,
    pub machine_id: Option<String>,
    pub avatar: Option<AgentAvatarUpload>,
    pub attention: Option<AgentAttention>,
    pub provider_agent_config_json: Option<String>,
    pub provider_agent_secrets_json: Option<String>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateWorkspaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub name: Option<String>,
    pub rules: Option<String>,
    pub provider_config_json: Option<String>,
    pub cover_resource_id: Option<i64>,
    pub provider_secrets_json: Option<String>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadWorkspaceImageRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub file_name: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub bytes: Vec<u8>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadWorkspaceImageResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource: Option<ResourceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub workspace: Option<WorkspaceRef>,
    pub name: String,
    pub cover: Option<ResourceRef>,
    pub rules: String,
    pub provider_id: String,
    pub provider_config_json: String,
    pub desired_state: DesiredState,
    pub observed_state: ObservedState,
    pub status_message: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceImageRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource_id: i64,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceImageResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource: Option<ResourceRef>,
    #[serde(with = "base64_bytes")]
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub bytes: Vec<u8>,
}

pub use crate::link::WorkspaceRef;

/// Stable JSON error body; code follows the existing management status categories.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<String>,
}
impl From<crate::Status> for ErrorBody {
    fn from(status: crate::Status) -> Self {
        Self {
            code: status.code(),
            message: status.message().to_owned(),
            detail: status.detail().map(str::to_owned),
        }
    }
}
impl ErrorBody {
    pub fn into_status(self) -> crate::Status {
        crate::Status::new(self.code, self.message).with_detail(self.detail)
    }
}

/// Local listener discovery.
pub const URL_FILE: &str = "management.url";

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventFrame {
    Event(ManagementEvent),
    Error(ErrorBody),
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum ErrorCode {
    Ok,
    Cancelled,
    Unknown,
    InvalidArgument,
    DeadlineExceeded,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    ResourceExhausted,
    FailedPrecondition,
    Aborted,
    OutOfRange,
    Unimplemented,
    Internal,
    Unavailable,
    DataLoss,
    Unauthenticated,
}
/// Base64 expansion plus room for metadata, bounded independently of image validation.
pub fn json_limit(binary_limit: usize) -> usize {
    binary_limit
        .saturating_mul(4)
        .saturating_div(3)
        .saturating_add(64 * 1024)
}
pub(crate) mod base64_bytes {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let value = String::deserialize(deserializer)?;
        STANDARD.decode(value).map_err(serde::de::Error::custom)
    }
}

/// Remote Machine entry point. Management always remains on loopback.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAccess {
    pub address: Option<String>,
    pub cert_sha256: Option<String>,
}
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetRemoteAccessRequest {
    pub address: Option<String>,
}
#[async_trait::async_trait]
pub trait RemoteAccessControl: Send + Sync {
    async fn get(&self) -> Result<RemoteAccess, crate::Status>;
    async fn enable(&self) -> Result<RemoteAccess, crate::Status>;
    async fn set(&self, address: Option<String>) -> Result<RemoteAccess, crate::Status>;
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivity {
    #[default]
    Idle,
    Busy,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FigureFrames {
    pub work: ResourceRef,
    pub sleep: ResourceRef,
    pub carry: ResourceRef,
    pub stand: ResourceRef,
}
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FigurePack {
    pub id: String,
    pub name: String,
    pub author: String,
    pub scale: f64,
    pub frames: FigureFrames,
}
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportFigurePackRequest {
    pub workspace: Option<WorkspaceRef>,
    #[serde(with = "base64_bytes")]
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub bytes: Vec<u8>,
}
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListFigurePacksRequest {
    pub workspace: Option<WorkspaceRef>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestHarnessRequest {
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: String,
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub cli_path: Option<String>,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub launch_arguments: Vec<String>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessTestState {
    Passed,
    Failed,
    TimedOut,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestHarnessResponse {
    pub state: HarnessTestState,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub error_code: Option<String>,
}
