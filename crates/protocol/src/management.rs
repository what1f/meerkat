//! Management JSON contract. These serde DTOs are independent of prost.
use serde::{Deserialize, Serialize};
pub const PREFIX: &str = "/api/v1/";
pub const EVENTS_PATH: &str = "/api/v1/events";
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AdvanceAgentOnboardingRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub action: String,
    pub field_values_json: String,
}
impl From<AdvanceAgentOnboardingRequest> for crate::AdvanceAgentOnboardingRequest {
    fn from(value: AdvanceAgentOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            action: value.action,
            field_values_json: value.field_values_json,
        }
    }
}
impl From<crate::AdvanceAgentOnboardingRequest> for AdvanceAgentOnboardingRequest {
    fn from(value: crate::AdvanceAgentOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            action: value.action,
            field_values_json: value.field_values_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AdvanceProviderOnboardingRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub action: String,
    pub field_values_json: String,
}
impl From<AdvanceProviderOnboardingRequest> for crate::AdvanceProviderOnboardingRequest {
    fn from(value: AdvanceProviderOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            action: value.action,
            field_values_json: value.field_values_json,
        }
    }
}
impl From<crate::AdvanceProviderOnboardingRequest> for AdvanceProviderOnboardingRequest {
    fn from(value: crate::AdvanceProviderOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            action: value.action,
            field_values_json: value.field_values_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
    pub computer_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub attention: Option<AgentAttention>,
    pub provider_agent_config_json: String,
    pub pause_reason: Option<String>,
}
impl From<Agent> for crate::Agent {
    fn from(value: Agent) -> Self {
        Self {
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
            avatar_path: value.avatar_path,
            harness_id: value.harness_id,
            model: value.model,
            thinking_level: value.thinking_level,
            auto_reaction: value.auto_reaction,
            soul: value.soul,
            cli_path: value.cli_path,
            environment_variables: value
                .environment_variables
                .into_iter()
                .map(Into::into)
                .collect(),
            computer_id: value.computer_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
            attention: value.attention.map(Into::into),
            provider_agent_config_json: value.provider_agent_config_json,
            pause_reason: value.pause_reason,
        }
    }
}
impl From<crate::Agent> for Agent {
    fn from(value: crate::Agent) -> Self {
        Self {
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
            avatar_path: value.avatar_path,
            harness_id: value.harness_id,
            model: value.model,
            thinking_level: value.thinking_level,
            auto_reaction: value.auto_reaction,
            soul: value.soul,
            cli_path: value.cli_path,
            environment_variables: value
                .environment_variables
                .into_iter()
                .map(Into::into)
                .collect(),
            computer_id: value.computer_id,
            created_at: value.created_at,
            updated_at: value.updated_at,
            attention: value.attention.map(Into::into),
            provider_agent_config_json: value.provider_agent_config_json,
            pause_reason: value.pause_reason,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentAttention {
    pub channels: Vec<ChannelAttention>,
}
impl From<AgentAttention> for crate::AgentAttention {
    fn from(value: AgentAttention) -> Self {
        Self {
            channels: value.channels.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::AgentAttention> for AgentAttention {
    fn from(value: crate::AgentAttention) -> Self {
        Self {
            channels: value.channels.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentAvatarFile {
    pub path: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}
impl From<AgentAvatarFile> for crate::AgentAvatarFile {
    fn from(value: AgentAvatarFile) -> Self {
        Self {
            path: value.path,
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
}
impl From<crate::AgentAvatarFile> for AgentAvatarFile {
    fn from(value: crate::AgentAvatarFile) -> Self {
        Self {
            path: value.path,
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentAvatarUpload {
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}
impl From<AgentAvatarUpload> for crate::AgentAvatarUpload {
    fn from(value: AgentAvatarUpload) -> Self {
        Self {
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
}
impl From<crate::AgentAvatarUpload> for AgentAvatarUpload {
    fn from(value: crate::AgentAvatarUpload) -> Self {
        Self {
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
impl From<AgentOnboardingState> for crate::AgentOnboardingState {
    fn from(value: AgentOnboardingState) -> Self {
        Self {
            agent: value.agent.map(Into::into),
            provider_id: value.provider_id,
            step: value.step,
            status: value.status,
            payload_json: value.payload_json,
            verification_uri: value.verification_uri,
            poll_interval_seconds: value.poll_interval_seconds,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
impl From<crate::AgentOnboardingState> for AgentOnboardingState {
    fn from(value: crate::AgentOnboardingState) -> Self {
        Self {
            agent: value.agent.map(Into::into),
            provider_id: value.provider_id,
            step: value.step,
            status: value.status,
            payload_json: value.payload_json,
            verification_uri: value.verification_uri,
            poll_interval_seconds: value.poll_interval_seconds,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentRef {
    pub workspace_id: String,
    pub agent_id: String,
}
impl From<AgentRef> for crate::AgentRef {
    fn from(value: AgentRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
            agent_id: value.agent_id,
        }
    }
}
impl From<crate::AgentRef> for AgentRef {
    fn from(value: crate::AgentRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
            agent_id: value.agent_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AttemptFailure {
    pub code: String,
    pub category: String,
    pub retryable: bool,
    pub fingerprint: String,
    pub detail: String,
}
impl From<AttemptFailure> for crate::AttemptFailure {
    fn from(value: AttemptFailure) -> Self {
        Self {
            code: value.code,
            category: value.category,
            retryable: value.retryable,
            fingerprint: value.fingerprint,
            detail: value.detail,
        }
    }
}
impl From<crate::AttemptFailure> for AttemptFailure {
    fn from(value: crate::AttemptFailure) -> Self {
        Self {
            code: value.code,
            category: value.category,
            retryable: value.retryable,
            fingerprint: value.fingerprint,
            detail: value.detail,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CancelAttemptRequest {
    pub request_id: String,
    pub execution_id: i64,
}
impl From<CancelAttemptRequest> for crate::CancelAttemptRequest {
    fn from(value: CancelAttemptRequest) -> Self {
        Self {
            request_id: value.request_id,
            execution_id: value.execution_id,
        }
    }
}
impl From<crate::CancelAttemptRequest> for CancelAttemptRequest {
    fn from(value: crate::CancelAttemptRequest) -> Self {
        Self {
            request_id: value.request_id,
            execution_id: value.execution_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CancelAttemptResponse {}
impl From<CancelAttemptResponse> for crate::CancelAttemptResponse {
    fn from(_value: CancelAttemptResponse) -> Self {
        Self {}
    }
}
impl From<crate::CancelAttemptResponse> for CancelAttemptResponse {
    fn from(_value: crate::CancelAttemptResponse) -> Self {
        Self {}
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CatalogRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}
impl From<CatalogRequest> for crate::CatalogRequest {
    fn from(value: CatalogRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
impl From<crate::CatalogRequest> for CatalogRequest {
    fn from(value: crate::CatalogRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ChannelAttention {
    pub channel_id: String,
    pub mode: i32,
    pub interval_minutes: u32,
}
impl From<ChannelAttention> for crate::ChannelAttention {
    fn from(value: ChannelAttention) -> Self {
        Self {
            channel_id: value.channel_id,
            mode: value.mode,
            interval_minutes: value.interval_minutes,
        }
    }
}
impl From<crate::ChannelAttention> for ChannelAttention {
    fn from(value: crate::ChannelAttention) -> Self {
        Self {
            channel_id: value.channel_id,
            mode: value.mode,
            interval_minutes: value.interval_minutes,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ComputerCatalog {
    pub computers: Vec<ComputerCatalogItem>,
}
impl From<ComputerCatalog> for crate::ComputerCatalog {
    fn from(value: ComputerCatalog) -> Self {
        Self {
            computers: value.computers.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ComputerCatalog> for ComputerCatalog {
    fn from(value: crate::ComputerCatalog) -> Self {
        Self {
            computers: value.computers.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ComputerCatalogItem {
    pub computer_id: String,
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
impl From<ComputerCatalogItem> for crate::ComputerCatalogItem {
    fn from(value: ComputerCatalogItem) -> Self {
        Self {
            computer_id: value.computer_id,
            display_name: value.display_name,
            observed_state: value.observed_state,
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
            status_message: value.status_message,
            registered: value.registered,
            revoked_at: value.revoked_at,
            last_seen_at: value.last_seen_at,
            paired_at: value.paired_at,
            os: value.os,
            version: value.version,
        }
    }
}
impl From<crate::ComputerCatalogItem> for ComputerCatalogItem {
    fn from(value: crate::ComputerCatalogItem) -> Self {
        Self {
            computer_id: value.computer_id,
            display_name: value.display_name,
            observed_state: value.observed_state,
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
            status_message: value.status_message,
            registered: value.registered,
            revoked_at: value.revoked_at,
            last_seen_at: value.last_seen_at,
            paired_at: value.paired_at,
            os: value.os,
            version: value.version,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
    pub computer_id: String,
    pub attention: Option<AgentAttention>,
    pub provider_agent_config_json: String,
    pub provider_agent_secrets_json: String,
}
impl From<CreateAgentRequest> for crate::CreateAgentRequest {
    fn from(value: CreateAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
            harness_id: value.harness_id,
            model: value.model,
            thinking_level: value.thinking_level,
            auto_reaction: value.auto_reaction,
            soul: value.soul,
            cli_path: value.cli_path,
            environment_variables: value
                .environment_variables
                .into_iter()
                .map(Into::into)
                .collect(),
            computer_id: value.computer_id,
            attention: value.attention.map(Into::into),
            provider_agent_config_json: value.provider_agent_config_json,
            provider_agent_secrets_json: value.provider_agent_secrets_json,
        }
    }
}
impl From<crate::CreateAgentRequest> for CreateAgentRequest {
    fn from(value: crate::CreateAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
            harness_id: value.harness_id,
            model: value.model,
            thinking_level: value.thinking_level,
            auto_reaction: value.auto_reaction,
            soul: value.soul,
            cli_path: value.cli_path,
            environment_variables: value
                .environment_variables
                .into_iter()
                .map(Into::into)
                .collect(),
            computer_id: value.computer_id,
            attention: value.attention.map(Into::into),
            provider_agent_config_json: value.provider_agent_config_json,
            provider_agent_secrets_json: value.provider_agent_secrets_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CreateWorkspaceRequest {
    pub request_id: String,
    pub workspace_id: String,
    pub name: String,
    pub rules: String,
    pub provider_id: String,
    pub provider_config_json: String,
    pub provider_secrets_json: String,
}
impl From<CreateWorkspaceRequest> for crate::CreateWorkspaceRequest {
    fn from(value: CreateWorkspaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace_id: value.workspace_id,
            name: value.name,
            rules: value.rules,
            provider_id: value.provider_id,
            provider_config_json: value.provider_config_json,
            provider_secrets_json: value.provider_secrets_json,
        }
    }
}
impl From<crate::CreateWorkspaceRequest> for CreateWorkspaceRequest {
    fn from(value: crate::CreateWorkspaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace_id: value.workspace_id,
            name: value.name,
            rules: value.rules,
            provider_id: value.provider_id,
            provider_config_json: value.provider_config_json,
            provider_secrets_json: value.provider_secrets_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DeleteAgentRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}
impl From<DeleteAgentRequest> for crate::DeleteAgentRequest {
    fn from(value: DeleteAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<crate::DeleteAgentRequest> for DeleteAgentRequest {
    fn from(value: crate::DeleteAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DeleteAgentResponse {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}
impl From<DeleteAgentResponse> for crate::DeleteAgentResponse {
    fn from(value: DeleteAgentResponse) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<crate::DeleteAgentResponse> for DeleteAgentResponse {
    fn from(value: crate::DeleteAgentResponse) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}
impl From<EnvironmentVariable> for crate::EnvironmentVariable {
    fn from(value: EnvironmentVariable) -> Self {
        Self {
            name: value.name,
            value: value.value,
        }
    }
}
impl From<crate::EnvironmentVariable> for EnvironmentVariable {
    fn from(value: crate::EnvironmentVariable) -> Self {
        Self {
            name: value.name,
            value: value.value,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvironmentVariableSet {
    pub entries: Vec<EnvironmentVariable>,
}
impl From<EnvironmentVariableSet> for crate::EnvironmentVariableSet {
    fn from(value: EnvironmentVariableSet) -> Self {
        Self {
            entries: value.entries.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::EnvironmentVariableSet> for EnvironmentVariableSet {
    fn from(value: crate::EnvironmentVariableSet) -> Self {
        Self {
            entries: value.entries.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ExecutionAttemptDetails {
    pub attempt: u32,
    pub computer_id: String,
    pub state: String,
    pub failure: Option<AttemptFailure>,
}
impl From<ExecutionAttemptDetails> for crate::ExecutionAttemptDetails {
    fn from(value: ExecutionAttemptDetails) -> Self {
        Self {
            attempt: value.attempt,
            computer_id: value.computer_id,
            state: value.state,
            failure: value.failure.map(Into::into),
        }
    }
}
impl From<crate::ExecutionAttemptDetails> for ExecutionAttemptDetails {
    fn from(value: crate::ExecutionAttemptDetails) -> Self {
        Self {
            attempt: value.attempt,
            computer_id: value.computer_id,
            state: value.state,
            failure: value.failure.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ExecutionDetails {
    pub execution_id: i64,
    pub state: String,
    pub attempts: Vec<ExecutionAttemptDetails>,
}
impl From<ExecutionDetails> for crate::ExecutionDetails {
    fn from(value: ExecutionDetails) -> Self {
        Self {
            execution_id: value.execution_id,
            state: value.state,
            attempts: value.attempts.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ExecutionDetails> for ExecutionDetails {
    fn from(value: crate::ExecutionDetails) -> Self {
        Self {
            execution_id: value.execution_id,
            state: value.state,
            attempts: value.attempts.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetAgentAvatarRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}
impl From<GetAgentAvatarRequest> for crate::GetAgentAvatarRequest {
    fn from(value: GetAgentAvatarRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<crate::GetAgentAvatarRequest> for GetAgentAvatarRequest {
    fn from(value: crate::GetAgentAvatarRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetAgentAvatarResponse {
    pub request_id: String,
    pub agent: Option<AgentRef>,
    pub avatar: Option<AgentAvatarFile>,
}
impl From<GetAgentAvatarResponse> for crate::GetAgentAvatarResponse {
    fn from(value: GetAgentAvatarResponse) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            avatar: value.avatar.map(Into::into),
        }
    }
}
impl From<crate::GetAgentAvatarResponse> for GetAgentAvatarResponse {
    fn from(value: crate::GetAgentAvatarResponse) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            avatar: value.avatar.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetAgentOnboardingRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}
impl From<GetAgentOnboardingRequest> for crate::GetAgentOnboardingRequest {
    fn from(value: GetAgentOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<crate::GetAgentOnboardingRequest> for GetAgentOnboardingRequest {
    fn from(value: crate::GetAgentOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetAgentRequest {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}
impl From<GetAgentRequest> for crate::GetAgentRequest {
    fn from(value: GetAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<crate::GetAgentRequest> for GetAgentRequest {
    fn from(value: crate::GetAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetExecutionDetailsRequest {
    pub request_id: String,
    pub execution_id: i64,
}
impl From<GetExecutionDetailsRequest> for crate::GetExecutionDetailsRequest {
    fn from(value: GetExecutionDetailsRequest) -> Self {
        Self {
            request_id: value.request_id,
            execution_id: value.execution_id,
        }
    }
}
impl From<crate::GetExecutionDetailsRequest> for GetExecutionDetailsRequest {
    fn from(value: crate::GetExecutionDetailsRequest) -> Self {
        Self {
            request_id: value.request_id,
            execution_id: value.execution_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetProviderOnboardingRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}
impl From<GetProviderOnboardingRequest> for crate::GetProviderOnboardingRequest {
    fn from(value: GetProviderOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
impl From<crate::GetProviderOnboardingRequest> for GetProviderOnboardingRequest {
    fn from(value: crate::GetProviderOnboardingRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GetWorkspaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}
impl From<GetWorkspaceRequest> for crate::GetWorkspaceRequest {
    fn from(value: GetWorkspaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
impl From<crate::GetWorkspaceRequest> for GetWorkspaceRequest {
    fn from(value: crate::GetWorkspaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HarnessCatalog {
    pub harnesses: Vec<HarnessCatalogItem>,
}
impl From<HarnessCatalog> for crate::HarnessCatalog {
    fn from(value: HarnessCatalog) -> Self {
        Self {
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::HarnessCatalog> for HarnessCatalog {
    fn from(value: crate::HarnessCatalog) -> Self {
        Self {
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HarnessCatalogItem {
    pub harness_id: String,
    pub display_name: String,
    pub description: String,
    pub availability: i32,
    pub supported_thinking_levels: Vec<String>,
}
impl From<HarnessCatalogItem> for crate::HarnessCatalogItem {
    fn from(value: HarnessCatalogItem) -> Self {
        Self {
            harness_id: value.harness_id,
            display_name: value.display_name,
            description: value.description,
            availability: value.availability,
            supported_thinking_levels: value.supported_thinking_levels,
        }
    }
}
impl From<crate::HarnessCatalogItem> for HarnessCatalogItem {
    fn from(value: crate::HarnessCatalogItem) -> Self {
        Self {
            harness_id: value.harness_id,
            display_name: value.display_name,
            description: value.description,
            availability: value.availability,
            supported_thinking_levels: value.supported_thinking_levels,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HarnessInfo {
    pub harness_id: String,
    pub display_name: String,
    pub availability: i32,
    pub version: String,
    pub reason: String,
    pub model_catalog: Option<ModelCatalog>,
    pub model_catalog_error: String,
    pub capabilities: Vec<String>,
}
impl From<HarnessInfo> for crate::HarnessInfo {
    fn from(value: HarnessInfo) -> Self {
        Self {
            harness_id: value.harness_id,
            display_name: value.display_name,
            availability: value.availability,
            version: value.version,
            reason: value.reason,
            model_catalog: value.model_catalog.map(Into::into),
            model_catalog_error: value.model_catalog_error,
            capabilities: value.capabilities,
        }
    }
}
impl From<crate::HarnessInfo> for HarnessInfo {
    fn from(value: crate::HarnessInfo) -> Self {
        Self {
            harness_id: value.harness_id,
            display_name: value.display_name,
            availability: value.availability,
            version: value.version,
            reason: value.reason,
            model_catalog: value.model_catalog.map(Into::into),
            model_catalog_error: value.model_catalog_error,
            capabilities: value.capabilities,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct InvokeProviderInterfaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interface_id: String,
    pub input_json: String,
}
impl From<InvokeProviderInterfaceRequest> for crate::InvokeProviderInterfaceRequest {
    fn from(value: InvokeProviderInterfaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            interface_id: value.interface_id,
            input_json: value.input_json,
        }
    }
}
impl From<crate::InvokeProviderInterfaceRequest> for InvokeProviderInterfaceRequest {
    fn from(value: crate::InvokeProviderInterfaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            interface_id: value.interface_id,
            input_json: value.input_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct InvokeProviderInterfaceResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interface_id: String,
    pub output_json: String,
}
impl From<InvokeProviderInterfaceResponse> for crate::InvokeProviderInterfaceResponse {
    fn from(value: InvokeProviderInterfaceResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            interface_id: value.interface_id,
            output_json: value.output_json,
        }
    }
}
impl From<crate::InvokeProviderInterfaceResponse> for InvokeProviderInterfaceResponse {
    fn from(value: crate::InvokeProviderInterfaceResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            interface_id: value.interface_id,
            output_json: value.output_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ListAgentsRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}
impl From<ListAgentsRequest> for crate::ListAgentsRequest {
    fn from(value: ListAgentsRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
impl From<crate::ListAgentsRequest> for ListAgentsRequest {
    fn from(value: crate::ListAgentsRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ListAgentsResponse {
    pub request_id: String,
    pub agents: Vec<Agent>,
}
impl From<ListAgentsResponse> for crate::ListAgentsResponse {
    fn from(value: ListAgentsResponse) -> Self {
        Self {
            request_id: value.request_id,
            agents: value.agents.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ListAgentsResponse> for ListAgentsResponse {
    fn from(value: crate::ListAgentsResponse) -> Self {
        Self {
            request_id: value.request_id,
            agents: value.agents.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ListProviderInterfacesRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
}
impl From<ListProviderInterfacesRequest> for crate::ListProviderInterfacesRequest {
    fn from(value: ListProviderInterfacesRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
impl From<crate::ListProviderInterfacesRequest> for ListProviderInterfacesRequest {
    fn from(value: crate::ListProviderInterfacesRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ListProviderInterfacesResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub interfaces: Vec<ProviderInterface>,
}
impl From<ListProviderInterfacesResponse> for crate::ListProviderInterfacesResponse {
    fn from(value: ListProviderInterfacesResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            interfaces: value.interfaces.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ListProviderInterfacesResponse> for ListProviderInterfacesResponse {
    fn from(value: crate::ListProviderInterfacesResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            interfaces: value.interfaces.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ListWorkspacesRequest {
    pub request_id: String,
}
impl From<ListWorkspacesRequest> for crate::ListWorkspacesRequest {
    fn from(value: ListWorkspacesRequest) -> Self {
        Self {
            request_id: value.request_id,
        }
    }
}
impl From<crate::ListWorkspacesRequest> for ListWorkspacesRequest {
    fn from(value: crate::ListWorkspacesRequest) -> Self {
        Self {
            request_id: value.request_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ListWorkspacesResponse {
    pub request_id: String,
    pub workspaces: Vec<Workspace>,
}
impl From<ListWorkspacesResponse> for crate::ListWorkspacesResponse {
    fn from(value: ListWorkspacesResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspaces: value.workspaces.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ListWorkspacesResponse> for ListWorkspacesResponse {
    fn from(value: crate::ListWorkspacesResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspaces: value.workspaces.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ManagementEvent {
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: i32,
    pub workspace: Option<WorkspaceRef>,
    pub agent: Option<AgentRef>,
}
impl From<ManagementEvent> for crate::ManagementEvent {
    fn from(value: ManagementEvent) -> Self {
        Self {
            sequence: value.sequence,
            occurred_at: value.occurred_at,
            kind: value.kind,
            workspace: value.workspace.map(Into::into),
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<crate::ManagementEvent> for ManagementEvent {
    fn from(value: crate::ManagementEvent) -> Self {
        Self {
            sequence: value.sequence,
            occurred_at: value.occurred_at,
            kind: value.kind,
            workspace: value.workspace.map(Into::into),
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ManagementSubscription {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub after_sequence: u64,
}
impl From<ManagementSubscription> for crate::ManagementSubscription {
    fn from(value: ManagementSubscription) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            after_sequence: value.after_sequence,
        }
    }
}
impl From<crate::ManagementSubscription> for ManagementSubscription {
    fn from(value: crate::ManagementSubscription) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            after_sequence: value.after_sequence,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelCatalog {
    pub models: Vec<ModelCatalogItem>,
}
impl From<ModelCatalog> for crate::ModelCatalog {
    fn from(value: ModelCatalog) -> Self {
        Self {
            models: value.models.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ModelCatalog> for ModelCatalog {
    fn from(value: crate::ModelCatalog) -> Self {
        Self {
            models: value.models.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelCatalogItem {
    pub model_id: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
    pub supported_thinking_levels: Vec<String>,
}
impl From<ModelCatalogItem> for crate::ModelCatalogItem {
    fn from(value: ModelCatalogItem) -> Self {
        Self {
            model_id: value.model_id,
            display_name: value.display_name,
            description: value.description,
            available: value.available,
            supported_thinking_levels: value.supported_thinking_levels,
        }
    }
}
impl From<crate::ModelCatalogItem> for ModelCatalogItem {
    fn from(value: crate::ModelCatalogItem) -> Self {
        Self {
            model_id: value.model_id,
            display_name: value.display_name,
            description: value.description,
            available: value.available,
            supported_thinking_levels: value.supported_thinking_levels,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelCatalogRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub computer_id: String,
    pub harness_id: String,
}
impl From<ModelCatalogRequest> for crate::ModelCatalogRequest {
    fn from(value: ModelCatalogRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            computer_id: value.computer_id,
            harness_id: value.harness_id,
        }
    }
}
impl From<crate::ModelCatalogRequest> for ModelCatalogRequest {
    fn from(value: crate::ModelCatalogRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            computer_id: value.computer_id,
            harness_id: value.harness_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PairingCode {
    pub code: String,
    pub issued_at: i64,
    pub expires_at: i64,
}
impl From<PairingCode> for crate::PairingCode {
    fn from(value: PairingCode) -> Self {
        Self {
            code: value.code,
            issued_at: value.issued_at,
            expires_at: value.expires_at,
        }
    }
}
impl From<crate::PairingCode> for PairingCode {
    fn from(value: crate::PairingCode) -> Self {
        Self {
            code: value.code,
            issued_at: value.issued_at,
            expires_at: value.expires_at,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderCatalog {
    pub providers: Vec<ProviderCatalogItem>,
}
impl From<ProviderCatalog> for crate::ProviderCatalog {
    fn from(value: ProviderCatalog) -> Self {
        Self {
            providers: value.providers.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ProviderCatalog> for ProviderCatalog {
    fn from(value: crate::ProviderCatalog) -> Self {
        Self {
            providers: value.providers.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderCatalogItem {
    pub provider_id: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
    pub capabilities: Vec<String>,
}
impl From<ProviderCatalogItem> for crate::ProviderCatalogItem {
    fn from(value: ProviderCatalogItem) -> Self {
        Self {
            provider_id: value.provider_id,
            display_name: value.display_name,
            description: value.description,
            available: value.available,
            capabilities: value.capabilities,
        }
    }
}
impl From<crate::ProviderCatalogItem> for ProviderCatalogItem {
    fn from(value: crate::ProviderCatalogItem) -> Self {
        Self {
            provider_id: value.provider_id,
            display_name: value.display_name,
            description: value.description,
            available: value.available,
            capabilities: value.capabilities,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderInterface {
    pub interface_id: String,
    pub description: Option<String>,
    pub input_schema_json: String,
    pub output_schema_json: String,
}
impl From<ProviderInterface> for crate::ProviderInterface {
    fn from(value: ProviderInterface) -> Self {
        Self {
            interface_id: value.interface_id,
            description: value.description,
            input_schema_json: value.input_schema_json,
            output_schema_json: value.output_schema_json,
        }
    }
}
impl From<crate::ProviderInterface> for ProviderInterface {
    fn from(value: crate::ProviderInterface) -> Self {
        Self {
            interface_id: value.interface_id,
            description: value.description,
            input_schema_json: value.input_schema_json,
            output_schema_json: value.output_schema_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
impl From<ProviderOnboardingState> for crate::ProviderOnboardingState {
    fn from(value: ProviderOnboardingState) -> Self {
        Self {
            workspace: value.workspace.map(Into::into),
            provider_id: value.provider_id,
            step: value.step,
            status: value.status,
            payload_json: value.payload_json,
            verification_uri: value.verification_uri,
            poll_interval_seconds: value.poll_interval_seconds,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
impl From<crate::ProviderOnboardingState> for ProviderOnboardingState {
    fn from(value: crate::ProviderOnboardingState) -> Self {
        Self {
            workspace: value.workspace.map(Into::into),
            provider_id: value.provider_id,
            step: value.step,
            status: value.status,
            payload_json: value.payload_json,
            verification_uri: value.verification_uri,
            poll_interval_seconds: value.poll_interval_seconds,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ReactionCatalog {
    pub reactions: Vec<ReactionCatalogItem>,
}
impl From<ReactionCatalog> for crate::ReactionCatalog {
    fn from(value: ReactionCatalog) -> Self {
        Self {
            reactions: value.reactions.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<crate::ReactionCatalog> for ReactionCatalog {
    fn from(value: crate::ReactionCatalog) -> Self {
        Self {
            reactions: value.reactions.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ReactionCatalogItem {
    pub reaction_id: String,
    pub display_name: String,
    pub emoji: String,
    pub available: bool,
}
impl From<ReactionCatalogItem> for crate::ReactionCatalogItem {
    fn from(value: ReactionCatalogItem) -> Self {
        Self {
            reaction_id: value.reaction_id,
            display_name: value.display_name,
            emoji: value.emoji,
            available: value.available,
        }
    }
}
impl From<crate::ReactionCatalogItem> for ReactionCatalogItem {
    fn from(value: crate::ReactionCatalogItem) -> Self {
        Self {
            reaction_id: value.reaction_id,
            display_name: value.display_name,
            emoji: value.emoji,
            available: value.available,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceRef {
    pub resource_id: i64,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}
impl From<ResourceRef> for crate::ResourceRef {
    fn from(value: ResourceRef) -> Self {
        Self {
            resource_id: value.resource_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}
impl From<crate::ResourceRef> for ResourceRef {
    fn from(value: crate::ResourceRef) -> Self {
        Self {
            resource_id: value.resource_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RevokeComputerRequest {
    pub request_id: String,
    pub computer_id: String,
}
impl From<RevokeComputerRequest> for crate::RevokeComputerRequest {
    fn from(value: RevokeComputerRequest) -> Self {
        Self {
            request_id: value.request_id,
            computer_id: value.computer_id,
        }
    }
}
impl From<crate::RevokeComputerRequest> for RevokeComputerRequest {
    fn from(value: crate::RevokeComputerRequest) -> Self {
        Self {
            request_id: value.request_id,
            computer_id: value.computer_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SetWorkspaceStateRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub desired_state: i32,
}
impl From<SetWorkspaceStateRequest> for crate::SetWorkspaceStateRequest {
    fn from(value: SetWorkspaceStateRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            desired_state: value.desired_state,
        }
    }
}
impl From<crate::SetWorkspaceStateRequest> for SetWorkspaceStateRequest {
    fn from(value: crate::SetWorkspaceStateRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            desired_state: value.desired_state,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
    pub computer_id: Option<String>,
    pub avatar: Option<AgentAvatarUpload>,
    pub attention: Option<AgentAttention>,
    pub provider_agent_config_json: Option<String>,
    pub provider_agent_secrets_json: Option<String>,
}
impl From<UpdateAgentRequest> for crate::UpdateAgentRequest {
    fn from(value: UpdateAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
            harness_id: value.harness_id,
            model: value.model,
            thinking_level: value.thinking_level,
            auto_reaction: value.auto_reaction,
            soul: value.soul,
            cli_path: value.cli_path,
            environment_variables: value.environment_variables.map(Into::into),
            computer_id: value.computer_id,
            avatar: value.avatar.map(Into::into),
            attention: value.attention.map(Into::into),
            provider_agent_config_json: value.provider_agent_config_json,
            provider_agent_secrets_json: value.provider_agent_secrets_json,
        }
    }
}
impl From<crate::UpdateAgentRequest> for UpdateAgentRequest {
    fn from(value: crate::UpdateAgentRequest) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
            harness_id: value.harness_id,
            model: value.model,
            thinking_level: value.thinking_level,
            auto_reaction: value.auto_reaction,
            soul: value.soul,
            cli_path: value.cli_path,
            environment_variables: value.environment_variables.map(Into::into),
            computer_id: value.computer_id,
            avatar: value.avatar.map(Into::into),
            attention: value.attention.map(Into::into),
            provider_agent_config_json: value.provider_agent_config_json,
            provider_agent_secrets_json: value.provider_agent_secrets_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UpdateWorkspaceRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub name: Option<String>,
    pub rules: Option<String>,
    pub provider_config_json: Option<String>,
    pub cover_resource_id: Option<i64>,
    pub provider_secrets_json: Option<String>,
}
impl From<UpdateWorkspaceRequest> for crate::UpdateWorkspaceRequest {
    fn from(value: UpdateWorkspaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            name: value.name,
            rules: value.rules,
            provider_config_json: value.provider_config_json,
            cover_resource_id: value.cover_resource_id,
            provider_secrets_json: value.provider_secrets_json,
        }
    }
}
impl From<crate::UpdateWorkspaceRequest> for UpdateWorkspaceRequest {
    fn from(value: crate::UpdateWorkspaceRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            name: value.name,
            rules: value.rules,
            provider_config_json: value.provider_config_json,
            cover_resource_id: value.cover_resource_id,
            provider_secrets_json: value.provider_secrets_json,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UploadWorkspaceImageRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub file_name: String,
    pub media_type: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}
impl From<UploadWorkspaceImageRequest> for crate::UploadWorkspaceImageRequest {
    fn from(value: UploadWorkspaceImageRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            file_name: value.file_name,
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
}
impl From<crate::UploadWorkspaceImageRequest> for UploadWorkspaceImageRequest {
    fn from(value: crate::UploadWorkspaceImageRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            file_name: value.file_name,
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct UploadWorkspaceImageResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource: Option<ResourceRef>,
}
impl From<UploadWorkspaceImageResponse> for crate::UploadWorkspaceImageResponse {
    fn from(value: UploadWorkspaceImageResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            resource: value.resource.map(Into::into),
        }
    }
}
impl From<crate::UploadWorkspaceImageResponse> for UploadWorkspaceImageResponse {
    fn from(value: crate::UploadWorkspaceImageResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            resource: value.resource.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
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
impl From<Workspace> for crate::Workspace {
    fn from(value: Workspace) -> Self {
        Self {
            workspace: value.workspace.map(Into::into),
            name: value.name,
            cover: value.cover.map(Into::into),
            rules: value.rules,
            provider_id: value.provider_id,
            provider_config_json: value.provider_config_json,
            desired_state: value.desired_state,
            observed_state: value.observed_state,
            status_message: value.status_message,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}
impl From<crate::Workspace> for Workspace {
    fn from(value: crate::Workspace) -> Self {
        Self {
            workspace: value.workspace.map(Into::into),
            name: value.name,
            cover: value.cover.map(Into::into),
            rules: value.rules,
            provider_id: value.provider_id,
            provider_config_json: value.provider_config_json,
            desired_state: value.desired_state,
            observed_state: value.observed_state,
            status_message: value.status_message,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceImageRequest {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource_id: i64,
}
impl From<WorkspaceImageRequest> for crate::WorkspaceImageRequest {
    fn from(value: WorkspaceImageRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            resource_id: value.resource_id,
        }
    }
}
impl From<crate::WorkspaceImageRequest> for WorkspaceImageRequest {
    fn from(value: crate::WorkspaceImageRequest) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            resource_id: value.resource_id,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceImageResponse {
    pub request_id: String,
    pub workspace: Option<WorkspaceRef>,
    pub resource: Option<ResourceRef>,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}
impl From<WorkspaceImageResponse> for crate::WorkspaceImageResponse {
    fn from(value: WorkspaceImageResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            resource: value.resource.map(Into::into),
            bytes: value.bytes,
        }
    }
}
impl From<crate::WorkspaceImageResponse> for WorkspaceImageResponse {
    fn from(value: crate::WorkspaceImageResponse) -> Self {
        Self {
            request_id: value.request_id,
            workspace: value.workspace.map(Into::into),
            resource: value.resource.map(Into::into),
            bytes: value.bytes,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRef {
    pub workspace_id: String,
}
impl From<WorkspaceRef> for crate::WorkspaceRef {
    fn from(value: WorkspaceRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
        }
    }
}
impl From<crate::WorkspaceRef> for WorkspaceRef {
    fn from(value: crate::WorkspaceRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
        }
    }
}
/// Stable JSON error body; code follows the existing management status categories.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<String>,
}
impl From<tonic::Status> for ErrorBody {
    fn from(status: tonic::Status) -> Self {
        Self {
            code: status.code().into(),
            message: status.message().to_owned(),
            detail: status
                .metadata()
                .get("lankide-error-code")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
        }
    }
}
impl ErrorBody {
    pub fn into_status(self) -> tonic::Status {
        let code = self.code.into();
        let mut status = tonic::Status::new(code, self.message);
        if let Some(detail) = self.detail.and_then(|v| v.parse().ok()) {
            status.metadata_mut().insert("lankide-error-code", detail);
        }
        status
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
impl From<tonic::Code> for ErrorCode {
    fn from(code: tonic::Code) -> Self {
        match code {
            tonic::Code::Ok => ErrorCode::Ok,
            tonic::Code::Cancelled => ErrorCode::Cancelled,
            tonic::Code::Unknown => ErrorCode::Unknown,
            tonic::Code::InvalidArgument => ErrorCode::InvalidArgument,
            tonic::Code::DeadlineExceeded => ErrorCode::DeadlineExceeded,
            tonic::Code::NotFound => ErrorCode::NotFound,
            tonic::Code::AlreadyExists => ErrorCode::AlreadyExists,
            tonic::Code::PermissionDenied => ErrorCode::PermissionDenied,
            tonic::Code::ResourceExhausted => ErrorCode::ResourceExhausted,
            tonic::Code::FailedPrecondition => ErrorCode::FailedPrecondition,
            tonic::Code::Aborted => ErrorCode::Aborted,
            tonic::Code::OutOfRange => ErrorCode::OutOfRange,
            tonic::Code::Unimplemented => ErrorCode::Unimplemented,
            tonic::Code::Internal => ErrorCode::Internal,
            tonic::Code::Unavailable => ErrorCode::Unavailable,
            tonic::Code::DataLoss => ErrorCode::DataLoss,
            tonic::Code::Unauthenticated => ErrorCode::Unauthenticated,
        }
    }
}
impl From<ErrorCode> for tonic::Code {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::Ok => tonic::Code::Ok,
            ErrorCode::Cancelled => tonic::Code::Cancelled,
            ErrorCode::Unknown => tonic::Code::Unknown,
            ErrorCode::InvalidArgument => tonic::Code::InvalidArgument,
            ErrorCode::DeadlineExceeded => tonic::Code::DeadlineExceeded,
            ErrorCode::NotFound => tonic::Code::NotFound,
            ErrorCode::AlreadyExists => tonic::Code::AlreadyExists,
            ErrorCode::PermissionDenied => tonic::Code::PermissionDenied,
            ErrorCode::ResourceExhausted => tonic::Code::ResourceExhausted,
            ErrorCode::FailedPrecondition => tonic::Code::FailedPrecondition,
            ErrorCode::Aborted => tonic::Code::Aborted,
            ErrorCode::OutOfRange => tonic::Code::OutOfRange,
            ErrorCode::Unimplemented => tonic::Code::Unimplemented,
            ErrorCode::Internal => tonic::Code::Internal,
            ErrorCode::Unavailable => tonic::Code::Unavailable,
            ErrorCode::DataLoss => tonic::Code::DataLoss,
            ErrorCode::Unauthenticated => tonic::Code::Unauthenticated,
        }
    }
}

/// Base64 expansion plus room for metadata, bounded independently of image validation.
pub fn json_limit(binary_limit: usize) -> usize {
    binary_limit
        .saturating_mul(4)
        .saturating_div(3)
        .saturating_add(64 * 1024)
}
mod base64_bytes {
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
