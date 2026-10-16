//! Management JSON contract. Canonical serde DTOs for management.
use serde::{Deserialize, Serialize};
pub const PREFIX: &str = "/api/v1/";
pub const EVENTS_PATH: &str = "/api/v1/events";
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvanceAgentOnboardingRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub action: String,
    pub field_values_json: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvanceProviderOnboardingRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub action: String,
    pub field_values_json: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Agent {
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAttention {
    pub channels: Vec<ChannelAttention>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAvatarFile {
    pub path: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentAvatarUpload {
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentOnboardingState {
    pub agent: Option<AgentRef>,
    pub provider_id: String,
    pub step: String,
    pub status: i32,
    pub payload_json: String,
    pub verification_uri: String,
    pub poll_interval_seconds: u32,
    pub error_code: String,
    pub error_message: String,
}

pub use crate::link::AgentRef;

pub use crate::link::AttemptFailure;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelAttemptRequest {
    pub request_id: String,
    pub execution_id: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelAttemptResponse {}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelAttention {
    pub channel_id: String,
    pub mode: i32,
    pub interval_minutes: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineCatalog {
    pub machines: Vec<MachineCatalogItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MachineCatalogItem {
    pub machine_id: String,
    pub display_name: String,
    pub observed_state: i32,
    pub harnesses: Vec<HarnessInfo>,
    pub status_message: String,
    pub registered: bool,
    pub revoked_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub paired_at: i64,
    pub os: String,
    pub version: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateAgentRequest {
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAgentRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteAgentResponse {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

pub use crate::link::EnvironmentVariable;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentVariableSet {
    pub entries: Vec<EnvironmentVariable>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionAttemptDetails {
    pub attempt: u32,
    pub machine_id: String,
    pub state: String,
    pub failure: Option<AttemptFailure>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionDetails {
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub execution_id: i64,
    pub state: String,
    pub attempts: Vec<ExecutionAttemptDetails>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentAvatarRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentAvatarResponse {
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub avatar: Option<AgentAvatarFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentOnboardingRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetAgentRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetExecutionDetailsRequest {
    pub request_id: String,
    pub execution_id: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetProviderOnboardingRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetWorkspaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessCatalog {
    pub harnesses: Vec<HarnessCatalogItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessCatalogItem {
    pub harness_id: String,
    pub display_name: String,
    pub description: String,
    pub availability: i32,
    pub supported_thinking_levels: Vec<String>,
}

pub use crate::link::HarnessInfo;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvokeProviderInterfaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interface_id: String,
    pub input_json: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvokeProviderInterfaceResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interface_id: String,
    pub output_json: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListAgentsRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListAgentsResponse {
    pub request_id: String,
    pub agents: Vec<Agent>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListProviderInterfacesRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListProviderInterfacesResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interfaces: Vec<ProviderInterface>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListWorkspacesRequest {
    pub request_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListWorkspacesResponse {
    pub request_id: String,
    pub workspaces: Vec<Workspace>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagementEvent {
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: i32,
    pub workspace: Option<WorkspaceRef>,
    pub agent: Option<AgentRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagementSubscription {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub after_sequence: u64,
}

pub use crate::link::ModelCatalog;

pub use crate::link::ModelCatalogItem;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCatalogRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub machine_id: String,
    pub harness_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCatalog {
    pub providers: Vec<ProviderCatalogItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCatalogItem {
    pub provider_id: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderInterface {
    pub interface_id: String,
    pub description: Option<String>,
    pub input_schema_json: String,
    pub output_schema_json: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderOnboardingState {
    pub workspace: Option<WorkspaceRef>,
    pub provider_id: String,
    pub step: String,
    pub status: i32,
    pub payload_json: String,
    pub verification_uri: String,
    pub poll_interval_seconds: u32,
    pub error_code: String,
    pub error_message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionCatalog {
    pub reactions: Vec<ReactionCatalogItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionCatalogItem {
    pub reaction_id: String,
    pub display_name: String,
    pub emoji: String,
    pub available: bool,
}

pub use crate::link::ResourceRef;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameMachineRequest {
    pub request_id: String,
    pub machine_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeMachineRequest {
    pub request_id: String,
    pub machine_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetWorkspaceStateRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub desired_state: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateAgentRequest {
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadWorkspaceImageRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub file_name: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadWorkspaceImageResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource: Option<ResourceRef>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub workspace: Option<WorkspaceRef>,
    pub name: String,
    pub cover: Option<ResourceRef>,
    pub rules: String,
    pub provider_id: String,
    pub provider_config_json: String,
    pub desired_state: i32,
    pub observed_state: i32,
    pub status_message: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceImageRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource_id: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceImageResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource: Option<ResourceRef>,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

pub use crate::link::WorkspaceRef;

/// Stable JSON error body; code follows the existing management status categories.
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

/// Owner secret and listener discovery are local files, never API parameters.
pub const OWNER_FILE: &str = "management-owner.secret";
pub const URL_FILE: &str = "management.url";

#[derive(Clone)]
pub struct OwnerCredential(String);
impl OwnerCredential {
    pub fn load_or_create(run: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::create_dir_all(run)?;
        let path = run.join(OWNER_FILE);
        let secret = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        if !path.exists() {
            // Publish only a complete, synced secret. A crash cannot leave an empty owner file.
            let temporary = run.join(format!(".management-owner-{}.tmp", uuid::Uuid::new_v4()));
            let publication = (|| {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)?;
                file.write_all(secret.as_bytes())?;
                file.sync_all()?;
                match std::fs::hard_link(&temporary, &path) {
                    Ok(()) => std::fs::File::open(run)?.sync_all(),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
                    Err(error) => Err(error),
                }
            })();
            let _ = std::fs::remove_file(temporary);
            publication?;
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "unsafe owner credential file",
            ));
        }
        let secret = std::fs::read_to_string(path)?;
        if secret.len() != 64 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid owner credential",
            ));
        }
        Ok(Self(secret))
    }
    pub fn authorizes(&self, candidate: &str) -> bool {
        use sha2::{Digest, Sha256};
        use subtle::ConstantTimeEq;
        // Fixed-size digests ensure the comparison does not leak a matching prefix.
        bool::from(Sha256::digest(self.0.as_bytes()).ct_eq(&Sha256::digest(candidate.as_bytes())))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventFrame {
    Event(ManagementEvent),
    Error(ErrorBody),
}

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
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAccess {
    pub address: Option<String>,
    pub cert_sha256: Option<String>,
}
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
