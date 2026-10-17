#![forbid(unsafe_code)]

//! Typed management boundary and production Runtime HTTP client.

use lankide_protocol as wire;
pub mod http;
pub use lankide_protocol::{
    AttemptFailure, ExecutionAttemptDetails, ExecutionDetails, PROVIDER_CAPABILITY_TASK_SYSTEM,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio_stream::{Stream, StreamExt};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub icon: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProviderCapabilityCatalogEntry {
    pub provider_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProviderInterface {
    pub id: String,
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HarnessOption {
    pub id: String,
    pub name: String,
    pub executable_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum WorkspaceStatus {
    Active,
    Paused,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum WorkspaceObservedState {
    Unknown,
    Starting,
    Running,
    Paused,
    Error,
    Stopped,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub rules: String,
    pub provider: ProviderInfo,
    pub status: WorkspaceStatus,
    pub observed_state: WorkspaceObservedState,
    pub status_message: String,
    pub cover_resource_id: Option<i64>,
    pub task_system_enabled: bool,
    pub provider_config: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorkspaceDraft {
    pub id: String,
    pub name: String,
    pub rules: String,
    pub provider_id: String,
    pub provider_config: Value,
    pub provider_secrets: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Avatar {
    Path(String),
    Custom(String),
}
impl Avatar {
    pub fn initials(&self, fallback: &str) -> String {
        let name = match self {
            Self::Custom(name) => name.as_str(),
            Self::Path(_) => fallback,
        };
        name.chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }
    pub fn color(&self) -> u32 {
        0xD9E7FA
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ThinkingLevel {
    None,
    Off,
    Minimal,
    XHigh,
    Max,
    Low,
    Medium,
    High,
}
impl ThinkingLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ClientError> {
        match value {
            "none" => Ok(Self::None),
            "off" => Ok(Self::Off),
            "minimal" => Ok(Self::Minimal),
            "xhigh" => Ok(Self::XHigh),
            "max" => Ok(Self::Max),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(ClientError::Protocol(format!(
                "unknown thinking level: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Agent {
    pub pause_reason: Option<String>,
    pub workspace_id: String,
    pub id: String,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Avatar,
    pub soul: String,
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: ThinkingLevel,
    pub auto_reaction: Option<String>,
    pub cli_path: String,
    pub environment_variables: Vec<(String, String)>,
    pub provider_handle: String,
    pub provider_agent_config: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AgentDraft {
    pub id: String,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Avatar,
    pub soul: String,
    pub machine_id: String,
    pub harness_id: String,
    pub model: String,
    pub thinking_level: ThinkingLevel,
    pub auto_reaction: Option<String>,
    pub cli_path: String,
    pub environment_variables: Vec<(String, String)>,
    pub provider_handle: String,
    pub provider_agent_config: Value,
    pub provider_agent_secrets: Value,
}
impl Default for AgentDraft {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            role: String::new(),
            bio: String::new(),
            avatar: Avatar::Custom(String::new()),
            soul: String::new(),
            machine_id: String::new(),
            harness_id: "pi".into(),
            model: String::new(),
            thinking_level: ThinkingLevel::High,
            auto_reaction: None,
            cli_path: String::new(),
            environment_variables: vec![],
            provider_handle: String::new(),
            provider_agent_config: json!({}),
            provider_agent_secrets: json!({}),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum OnboardingStatus {
    ActionRequired,
    Verifying,
    Ready,
    Error,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProviderOnboardingState {
    pub workspace_id: String,
    pub provider_id: String,
    pub step: String,
    pub status: OnboardingStatus,
    pub verification_uri: Option<String>,
    pub poll_interval_seconds: Option<u32>,
    pub payload: Value,
    pub error: Option<ClientError>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AgentOnboardingState {
    pub workspace_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub step: String,
    pub status: OnboardingStatus,
    pub verification_uri: Option<String>,
    pub poll_interval_seconds: Option<u32>,
    pub payload: Value,
    pub error: Option<ClientError>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct MachineOption {
    pub is_local: bool,
    pub registered: bool,
    pub revoked_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub paired_at: i64,
    pub os: String,
    pub version: String,
    pub inventory: Vec<lankide_protocol::management::HarnessInfo>,
    pub id: String,
    pub name: String,
    pub available: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelOption {
    pub thinking_levels: Vec<String>,
    pub id: String,
    pub name: String,
    pub available: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReactionOption {
    pub id: String,
    pub name: String,
    pub emoji: String,
    pub available: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ManagementEventKind {
    WorkspaceCreated,
    WorkspaceUpdated,
    WorkspaceStateChanged,
    AgentCreated,
    AgentUpdated,
    AgentDeleted,
    ProviderOnboardingChanged,
    CatalogChanged,
    MachineCatalogChanged,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManagementEvent {
    pub sequence: u64,
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub kind: ManagementEventKind,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum ClientError {
    WorkspaceNotFound(String),
    DuplicateWorkspaceId(String),
    AgentNotFound(String),
    DuplicateAgentId(String),
    InvalidAgentId,
    MachineNotRegistered,
    MachineRevoked,
    MachineHasAssignedAgents,
    LocalMachineCannotBeRemoved,
    MachineNameInvalid,
    HarnessUnavailableOnMachine,
    Validation(String),
    Provider(String),
    Unavailable(String),
    Transport(String),
    Protocol(String),
    StreamEnded,
}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkspaceNotFound(v) => write!(f, "workspace not found: {v}"),
            Self::DuplicateWorkspaceId(v) => write!(f, "workspace id already exists: {v}"),
            Self::AgentNotFound(v) => write!(f, "agent not found: {v}"),
            Self::DuplicateAgentId(v) => write!(f, "agent id already exists: {v}"),
            Self::MachineNotRegistered => f.write_str("machine_not_registered"),
            Self::MachineHasAssignedAgents => f.write_str("machine_has_assigned_agents"),
            Self::LocalMachineCannotBeRemoved => f.write_str("local_machine_cannot_be_removed"),
            Self::MachineNameInvalid => f.write_str("machine_name_invalid"),
            Self::MachineRevoked => f.write_str("machine_revoked"),
            Self::HarnessUnavailableOnMachine => f.write_str("harness_unavailable_on_machine"),
            Self::InvalidAgentId => write!(
                f,
                "agent id must use lowercase letters, numbers, and underscores"
            ),
            Self::Validation(v) => write!(f, "validation: {v}"),
            Self::Provider(v) => write!(f, "provider: {v}"),
            Self::Unavailable(v) => write!(f, "unavailable: {v}"),
            Self::Transport(v) => write!(f, "transport: {v}"),
            Self::Protocol(v) => write!(f, "protocol: {v}"),
            Self::StreamEnded => write!(f, "management event stream ended"),
        }
    }
}
impl std::error::Error for ClientError {}

pub struct ManagementSubscription {
    inner: Pin<Box<dyn Stream<Item = Result<ManagementEvent, ClientError>> + Send>>,
}
impl ManagementSubscription {
    pub fn from_stream(
        stream: impl Stream<Item = Result<ManagementEvent, ClientError>> + Send + 'static,
    ) -> Self {
        Self {
            inner: Box::pin(stream),
        }
    }

    pub async fn next(&mut self) -> Option<Result<ManagementEvent, ClientError>> {
        self.inner.next().await
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceImage {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct AgentAvatarUpload {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct AgentAvatarImage {
    pub path: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[async_trait::async_trait]
pub trait ManagementClient: Send + Sync {
    async fn upload_workspace_image(
        &self,
        _workspace_id: &str,
        _file_name: &str,
        _media_type: &str,
        _bytes: Vec<u8>,
    ) -> Result<i64, ClientError> {
        Err(ClientError::Protocol("Image upload is unavailable".into()))
    }
    async fn get_workspace_image(
        &self,
        _workspace_id: &str,
        _resource_id: i64,
    ) -> Result<WorkspaceImage, ClientError> {
        Err(ClientError::Protocol("Image loading is unavailable".into()))
    }
    async fn get_agent_avatar(
        &self,
        _workspace_id: &str,
        _agent_id: &str,
    ) -> Result<AgentAvatarImage, ClientError> {
        Err(ClientError::Protocol(
            "Agent avatar loading is unavailable".into(),
        ))
    }
    async fn list_workspaces(&self) -> Result<Vec<Workspace>, ClientError>;
    async fn get_workspace(&self, workspace_id: &str) -> Result<Workspace, ClientError>;
    async fn create_workspace(&self, draft: WorkspaceDraft) -> Result<Workspace, ClientError>;
    async fn update_workspace(&self, workspace: Workspace) -> Result<Workspace, ClientError>;
    async fn set_workspace_state(
        &self,
        workspace_id: &str,
        state: WorkspaceStatus,
    ) -> Result<Workspace, ClientError>;
    async fn list_agents(&self, workspace_id: &str) -> Result<Vec<Agent>, ClientError>;
    async fn get_execution_details(
        &self,
        _execution_id: i64,
    ) -> Result<ExecutionDetails, ClientError> {
        Err(ClientError::Unavailable(
            "execution details are unavailable".into(),
        ))
    }
    async fn get_agent(&self, workspace_id: &str, agent_id: &str) -> Result<Agent, ClientError>;
    async fn create_agent(
        &self,
        workspace_id: &str,
        draft: AgentDraft,
    ) -> Result<Agent, ClientError>;
    async fn update_agent(
        &self,
        agent: Agent,
        avatar: Option<AgentAvatarUpload>,
    ) -> Result<Agent, ClientError>;
    async fn resume_agent(
        &self,
        _workspace_id: &str,
        _agent_id: &str,
    ) -> Result<Agent, ClientError> {
        Err(ClientError::Protocol("Agent resume is unavailable".into()))
    }
    async fn delete_agent(&self, workspace_id: &str, agent_id: &str) -> Result<(), ClientError> {
        let _ = (workspace_id, agent_id);
        Err(ClientError::Protocol(
            "Agent deletion is unavailable".into(),
        ))
    }
    async fn get_provider_onboarding(
        &self,
        workspace_id: &str,
    ) -> Result<ProviderOnboardingState, ClientError>;
    async fn advance_provider_onboarding(
        &self,
        workspace_id: &str,
        action: &str,
        payload: Value,
    ) -> Result<ProviderOnboardingState, ClientError>;
    async fn get_agent_onboarding(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> Result<AgentOnboardingState, ClientError> {
        let _ = (workspace_id, agent_id);
        Err(ClientError::Provider(
            "agent onboarding is unavailable".into(),
        ))
    }
    async fn advance_agent_onboarding(
        &self,
        workspace_id: &str,
        agent_id: &str,
        action: &str,
        payload: Value,
    ) -> Result<AgentOnboardingState, ClientError> {
        let _ = (workspace_id, agent_id, action, payload);
        Err(ClientError::Provider(
            "agent onboarding is unavailable".into(),
        ))
    }
    async fn get_provider_catalog(&self) -> Result<Vec<ProviderInfo>, ClientError>;
    async fn get_provider_capability_catalog(
        &self,
    ) -> Result<Vec<ProviderCapabilityCatalogEntry>, ClientError> {
        Ok(Vec::new())
    }
    async fn enable_remote_access(&self) -> Result<wire::RemoteAccess, ClientError> {
        Err(ClientError::Protocol("remote_access_unavailable".into()))
    }
    async fn get_remote_access(&self) -> Result<wire::RemoteAccess, ClientError> {
        Err(ClientError::Protocol("remote_access_unavailable".into()))
    }
    async fn set_remote_access(
        &self,
        _address: Option<String>,
    ) -> Result<wire::RemoteAccess, ClientError> {
        Err(ClientError::Protocol("remote_access_unavailable".into()))
    }
    async fn create_pairing_code(&self) -> Result<PairingCode, ClientError> {
        Err(ClientError::Protocol("pairing_unavailable".into()))
    }
    async fn rename_machine(&self, _machine_id: &str, _name: &str) -> Result<(), ClientError> {
        Err(ClientError::Protocol("rename_unavailable".into()))
    }
    async fn revoke_machine(&self, _machine_id: &str) -> Result<(), ClientError> {
        Err(ClientError::Protocol("revocation_unavailable".into()))
    }
    async fn get_machine_catalog(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<MachineOption>, ClientError>;
    async fn get_harness_catalog(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<HarnessOption>, ClientError>;
    async fn get_model_catalog(
        &self,
        workspace_id: Option<&str>,
        machine_id: &str,
        harness_id: &str,
    ) -> Result<Vec<ModelOption>, ClientError>;
    async fn get_reaction_catalog(
        &self,
        workspace_id: Option<&str>,
    ) -> Result<Vec<ReactionOption>, ClientError>;
    async fn invoke_provider_interface(
        &self,
        workspace_id: &str,
        interface_id: &str,
        input: Value,
    ) -> Result<Value, ClientError> {
        let _ = (workspace_id, interface_id, input);
        Err(ClientError::Provider(
            "provider interface unavailable".into(),
        ))
    }
    async fn list_provider_interfaces(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<ProviderInterface>, ClientError> {
        let _ = workspace_id;
        Err(ClientError::Provider(
            "provider interface catalog unavailable".into(),
        ))
    }
    async fn subscribe_management_events(
        &self,
        workspace_id: Option<&str>,
        after_sequence: u64,
    ) -> Result<ManagementSubscription, ClientError>;
}

#[derive(Clone)]
pub struct RuntimeManagementClient {
    client: http::HttpManagementTransport,
    requests: Arc<AtomicU64>,
}
impl RuntimeManagementClient {
    /// Discover the HTTP listener and owner credential in a shared local run directory.
    pub fn connect_local_lazy(run: impl AsRef<Path>) -> Self {
        Self {
            client: http::HttpManagementTransport::local(run, wire::DEFAULT_MAX_MESSAGE_BYTES),
            requests: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn connect_local(run: impl AsRef<Path>) -> Result<Self, ClientError> {
        let client = Self::connect_local_lazy(run);
        client.list_workspaces().await?;
        Ok(client)
    }

    /// Request cancellation; execution details become terminal after cleanup.
    pub async fn cancel_execution(&self, execution_id: i64) -> Result<(), ClientError> {
        self.client
            .clone()
            .cancel_attempt(wire::CancelAttemptRequest {
                request_id: self.request_id(),
                execution_id,
            })
            .await
            .map_err(map_status)?;
        Ok(())
    }

    fn request_id(&self) -> String {
        format!(
            "management-{}",
            self.requests.fetch_add(1, Ordering::Relaxed) + 1
        )
    }
}

fn wref(id: &str) -> wire::WorkspaceRef {
    wire::WorkspaceRef {
        workspace_id: id.into(),
    }
}
fn aref(w: &str, a: &str) -> wire::AgentRef {
    wire::AgentRef {
        workspace_id: w.into(),
        agent_id: a.into(),
    }
}
fn map_status(s: lankide_protocol::Status) -> ClientError {
    match s.detail() {
        Some("machine_not_registered") => return ClientError::MachineNotRegistered,
        Some("machine_has_assigned_agents") => return ClientError::MachineHasAssignedAgents,
        Some("local_machine_cannot_be_removed") => {
            return ClientError::LocalMachineCannotBeRemoved;
        }
        Some("machine_name_invalid") => return ClientError::MachineNameInvalid,
        Some("machine_revoked") => return ClientError::MachineRevoked,
        Some("harness_unavailable_on_machine") => {
            return ClientError::HarnessUnavailableOnMachine;
        }
        _ => {}
    }

    match s.code() {
        lankide_protocol::Code::InvalidArgument => ClientError::Validation(s.message().into()),
        lankide_protocol::Code::AlreadyExists => ClientError::Validation(s.message().into()),
        lankide_protocol::Code::NotFound => ClientError::WorkspaceNotFound(s.message().into()),
        lankide_protocol::Code::PermissionDenied
        | lankide_protocol::Code::Unauthenticated
        | lankide_protocol::Code::FailedPrecondition => ClientError::Provider(s.message().into()),
        lankide_protocol::Code::Unavailable
        | lankide_protocol::Code::DeadlineExceeded
        | lankide_protocol::Code::Cancelled => ClientError::Unavailable(s.message().into()),
        _ => ClientError::Protocol(format!("{}: {}", s.code(), s.message())),
    }
}

fn map_workspace_status(s: lankide_protocol::Status) -> ClientError {
    if s.code() == lankide_protocol::Code::AlreadyExists {
        ClientError::DuplicateWorkspaceId(s.message().into())
    } else {
        map_status(s)
    }
}

fn map_agent_status(s: lankide_protocol::Status) -> ClientError {
    if s.code() == lankide_protocol::Code::AlreadyExists {
        ClientError::DuplicateAgentId(s.message().into())
    } else {
        map_status(s)
    }
}
fn provider_name(id: &str) -> String {
    match id {
        "feishu" => "飞书".into(),
        "telegram" => "Telegram".into(),
        other => other.into(),
    }
}
fn from_workspace(v: wire::Workspace) -> Result<Workspace, ClientError> {
    let id = v
        .workspace
        .ok_or_else(|| ClientError::Protocol("workspace ref missing".into()))?
        .workspace_id;
    let config: Value = serde_json::from_str(&v.provider_config_json)
        .map_err(|e| ClientError::Protocol(e.to_string()))?;
    Ok(Workspace {
        id,
        name: v.name,
        rules: v.rules,
        provider: ProviderInfo {
            id: v.provider_id.clone(),
            name: provider_name(&v.provider_id),
            icon: v.provider_id,
        },
        status: if v.desired_state == wire::DesiredState::Paused {
            WorkspaceStatus::Paused
        } else {
            WorkspaceStatus::Active
        },
        observed_state: match v.observed_state {
            wire::ObservedState::Starting => WorkspaceObservedState::Starting,
            wire::ObservedState::Running => WorkspaceObservedState::Running,
            wire::ObservedState::Paused => WorkspaceObservedState::Paused,
            wire::ObservedState::Error => WorkspaceObservedState::Error,
            wire::ObservedState::Stopped => WorkspaceObservedState::Stopped,
            _ => WorkspaceObservedState::Unknown,
        },
        status_message: v.status_message,
        cover_resource_id: v.cover.map(|resource| resource.resource_id),
        task_system_enabled: config
            .get("task_system_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        provider_config: config,
    })
}
fn from_agent(v: wire::Agent) -> Result<Agent, ClientError> {
    let r = v
        .agent
        .ok_or_else(|| ClientError::Protocol("agent ref missing".into()))?;
    let provider_agent_config: Value = serde_json::from_str(&v.provider_agent_config_json)
        .map_err(|e| ClientError::Protocol(format!("invalid provider agent config: {e}")))?;
    let provider_handle = ["provider_handle", "bot_username", "app_id"]
        .into_iter()
        .find_map(|key| provider_agent_config.get(key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned();
    Ok(Agent {
        pause_reason: v.pause_reason,
        workspace_id: r.workspace_id,
        id: r.agent_id,
        name: v.name.clone(),
        role: v.role,
        bio: v.bio,
        avatar: if v.avatar_path.is_empty() {
            Avatar::Custom(v.name)
        } else {
            Avatar::Path(v.avatar_path)
        },
        soul: v.soul,
        machine_id: v.machine_id,
        harness_id: v.harness_id,
        model: v.model,
        thinking_level: ThinkingLevel::parse(&v.thinking_level)?,
        auto_reaction: (!v.auto_reaction.is_empty()).then_some(v.auto_reaction),
        cli_path: v.cli_path,
        environment_variables: v
            .environment_variables
            .into_iter()
            .map(|x| (x.name, x.value))
            .collect(),
        provider_handle,
        provider_agent_config,
    })
}
fn expect_workspace(expected: &str, value: wire::Workspace) -> Result<Workspace, ClientError> {
    let workspace = from_workspace(value)?;
    if workspace.id != expected {
        return Err(ClientError::Protocol(format!(
            "workspace response mismatch: expected {expected}, got {}",
            workspace.id
        )));
    }
    Ok(workspace)
}
fn expect_agent(
    workspace_id: &str,
    agent_id: Option<&str>,
    value: wire::Agent,
) -> Result<Agent, ClientError> {
    let agent = from_agent(value)?;
    if agent.workspace_id != workspace_id || agent_id.is_some_and(|id| id != agent.id) {
        return Err(ClientError::Protocol(
            "agent response scope mismatch".into(),
        ));
    }
    Ok(agent)
}
fn envs(v: &[(String, String)]) -> Vec<wire::EnvironmentVariable> {
    v.iter()
        .map(|(name, value)| wire::EnvironmentVariable {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}
fn validate_agent_choice(harness_id: &str, cli_path: &str) -> Result<(), ClientError> {
    if cli_path.ends_with("/dcc") && harness_id != "claude_code" {
        return Err(ClientError::Validation(
            "dcc is only a Claude Code executable choice".into(),
        ));
    }
    Ok(())
}
fn catalog(id: String, w: Option<&str>) -> wire::CatalogRequest {
    wire::CatalogRequest {
        request_id: id,
        workspace: w.map(wref),
    }
}
fn from_onboarding(
    v: wire::ProviderOnboardingState,
) -> Result<ProviderOnboardingState, ClientError> {
    let workspace_id = v
        .workspace
        .ok_or_else(|| ClientError::Protocol("onboarding workspace missing".into()))?
        .workspace_id;
    let status = match v.status {
        wire::OnboardingStatus::ActionRequired => OnboardingStatus::ActionRequired,
        wire::OnboardingStatus::Verifying => OnboardingStatus::Verifying,
        wire::OnboardingStatus::Ready => OnboardingStatus::Ready,
        wire::OnboardingStatus::Error => OnboardingStatus::Error,
        _ => {
            return Err(ClientError::Protocol(
                "unspecified onboarding status".into(),
            ));
        }
    };
    let payload = serde_json::from_str(&v.payload_json)
        .map_err(|error| ClientError::Protocol(format!("invalid onboarding payload: {error}")))?;
    let error = (!v.error_message.is_empty())
        .then(|| ClientError::Provider(format!("{}: {}", v.error_code, v.error_message)));
    Ok(ProviderOnboardingState {
        workspace_id,
        provider_id: v.provider_id,
        step: v.step,
        status,
        verification_uri: (!v.verification_uri.is_empty()).then_some(v.verification_uri),
        poll_interval_seconds: (v.poll_interval_seconds != 0).then_some(v.poll_interval_seconds),
        payload,
        error,
    })
}

fn from_agent_onboarding(
    v: wire::AgentOnboardingState,
) -> Result<AgentOnboardingState, ClientError> {
    let agent = v
        .agent
        .ok_or_else(|| ClientError::Protocol("agent onboarding scope missing".into()))?;
    let status = match v.status {
        wire::OnboardingStatus::ActionRequired => OnboardingStatus::ActionRequired,
        wire::OnboardingStatus::Verifying => OnboardingStatus::Verifying,
        wire::OnboardingStatus::Ready => OnboardingStatus::Ready,
        wire::OnboardingStatus::Error => OnboardingStatus::Error,
        _ => {
            return Err(ClientError::Protocol(
                "unspecified onboarding status".into(),
            ));
        }
    };
    let payload = serde_json::from_str(&v.payload_json)
        .map_err(|error| ClientError::Protocol(error.to_string()))?;
    let error = (!v.error_message.is_empty())
        .then(|| ClientError::Provider(format!("{}: {}", v.error_code, v.error_message)));
    Ok(AgentOnboardingState {
        workspace_id: agent.workspace_id,
        agent_id: agent.agent_id,
        provider_id: v.provider_id,
        step: v.step,
        status,
        verification_uri: (!v.verification_uri.is_empty()).then_some(v.verification_uri),
        poll_interval_seconds: (v.poll_interval_seconds != 0).then_some(v.poll_interval_seconds),
        payload,
        error,
    })
}

#[async_trait::async_trait]
impl ManagementClient for RuntimeManagementClient {
    async fn upload_workspace_image(
        &self,
        workspace_id: &str,
        file_name: &str,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Result<i64, ClientError> {
        let id = self.request_id();
        let response = self
            .client
            .clone()
            .upload_workspace_image(wire::UploadWorkspaceImageRequest {
                request_id: id.clone(),
                workspace: Some(wref(workspace_id)),
                file_name: file_name.to_owned(),
                media_type: media_type.to_owned(),
                bytes,
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if response.request_id != id
            || response.workspace.as_ref().map(|w| w.workspace_id.as_str()) != Some(workspace_id)
        {
            return Err(ClientError::Protocol(
                "Image upload belongs to another request".into(),
            ));
        }
        let resource = response
            .resource
            .ok_or_else(|| ClientError::Protocol("Image metadata missing".into()))?;
        if resource.resource_id <= 0 {
            return Err(ClientError::Protocol("Image resource is invalid".into()));
        }
        Ok(resource.resource_id)
    }

    async fn get_workspace_image(
        &self,
        workspace_id: &str,
        resource_id: i64,
    ) -> Result<WorkspaceImage, ClientError> {
        use sha2::{Digest, Sha256};
        let id = self.request_id();
        let response = self
            .client
            .clone()
            .get_workspace_image(wire::WorkspaceImageRequest {
                request_id: id.clone(),
                workspace: Some(wref(workspace_id)),
                resource_id,
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if response.request_id != id
            || response.workspace.as_ref().map(|w| w.workspace_id.as_str()) != Some(workspace_id)
        {
            return Err(ClientError::Protocol(
                "Image response belongs to another request".into(),
            ));
        }
        let resource = response
            .resource
            .ok_or_else(|| ClientError::Protocol("Image metadata missing".into()))?;
        let hash = Sha256::digest(&response.bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if resource.resource_id != resource_id
            || response.bytes.len() > wire::MAX_WORKSPACE_IMAGE_BYTES
            || resource.size_bytes != response.bytes.len() as u64
            || resource.sha256 != hash
        {
            return Err(ClientError::Protocol("Image integrity check failed".into()));
        }
        Ok(WorkspaceImage {
            media_type: resource.media_type,
            bytes: response.bytes,
        })
    }

    async fn get_agent_avatar(
        &self,
        workspace_id: &str,
        agent_id: &str,
    ) -> Result<AgentAvatarImage, ClientError> {
        let id = self.request_id();
        let response = self
            .client
            .clone()
            .get_agent_avatar(wire::GetAgentAvatarRequest {
                request_id: id.clone(),
                agent: Some(aref(workspace_id, agent_id)),
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if response.request_id != id
            || response
                .agent
                .as_ref()
                .map(|value| (value.workspace_id.as_str(), value.agent_id.as_str()))
                != Some((workspace_id, agent_id))
        {
            return Err(ClientError::Protocol(
                "Agent avatar belongs to another request".into(),
            ));
        }
        let avatar = response
            .avatar
            .ok_or_else(|| ClientError::Protocol("Agent avatar is missing".into()))?;
        if avatar.bytes.len() > wire::MAX_WORKSPACE_IMAGE_BYTES {
            return Err(ClientError::Protocol(
                "Agent avatar exceeds the size limit".into(),
            ));
        }
        Ok(AgentAvatarImage {
            path: avatar.path,
            media_type: avatar.media_type,
            bytes: avatar.bytes,
        })
    }

    async fn list_workspaces(&self) -> Result<Vec<Workspace>, ClientError> {
        let id = self.request_id();
        let r = self
            .client
            .clone()
            .list_workspaces(wire::ListWorkspacesRequest {
                request_id: id.clone(),
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if r.request_id != id {
            return Err(ClientError::Protocol(
                "list_workspaces response request_id mismatch".into(),
            ));
        }
        r.workspaces.into_iter().map(from_workspace).collect()
    }
    async fn get_workspace(&self, w: &str) -> Result<Workspace, ClientError> {
        expect_workspace(
            w,
            self.client
                .clone()
                .get_workspace(wire::GetWorkspaceRequest {
                    request_id: self.request_id(),
                    workspace: Some(wref(w)),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn create_workspace(&self, d: WorkspaceDraft) -> Result<Workspace, ClientError> {
        let expected = d.id.clone();
        expect_workspace(
            &expected,
            self.client
                .clone()
                .create_workspace(wire::CreateWorkspaceRequest {
                    request_id: self.request_id(),
                    workspace_id: d.id,
                    name: d.name,
                    rules: d.rules,
                    provider_id: d.provider_id,
                    provider_config_json: d.provider_config.to_string(),
                    provider_secrets_json: d.provider_secrets.to_string(),
                })
                .await
                .map_err(map_workspace_status)?
                .into_inner(),
        )
    }
    async fn update_workspace(&self, w: Workspace) -> Result<Workspace, ClientError> {
        let expected = w.id.clone();
        expect_workspace(
            &expected,
            self.client
                .clone()
                .update_workspace(wire::UpdateWorkspaceRequest {
                    request_id: self.request_id(),
                    workspace: Some(wref(&w.id)),
                    name: Some(w.name),
                    rules: Some(w.rules),
                    provider_config_json: Some(w.provider_config.to_string()),
                    cover_resource_id: None,
                    provider_secrets_json: None,
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn set_workspace_state(
        &self,
        w: &str,
        s: WorkspaceStatus,
    ) -> Result<Workspace, ClientError> {
        expect_workspace(
            w,
            self.client
                .clone()
                .set_workspace_state(wire::SetWorkspaceStateRequest {
                    request_id: self.request_id(),
                    workspace: Some(wref(w)),
                    desired_state: match s {
                        WorkspaceStatus::Active => wire::DesiredState::Running,
                        WorkspaceStatus::Paused => wire::DesiredState::Paused,
                    },
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn list_agents(&self, w: &str) -> Result<Vec<Agent>, ClientError> {
        let id = self.request_id();
        let r = self
            .client
            .clone()
            .list_agents(wire::ListAgentsRequest {
                request_id: id.clone(),
                workspace: Some(wref(w)),
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if r.request_id != id {
            return Err(ClientError::Protocol(
                "list_agents response request_id mismatch".into(),
            ));
        }
        r.agents
            .into_iter()
            .map(|agent| expect_agent(w, None, agent))
            .collect()
    }
    async fn get_execution_details(
        &self,
        execution_id: i64,
    ) -> Result<ExecutionDetails, ClientError> {
        let details = self
            .client
            .clone()
            .get_execution_details(wire::GetExecutionDetailsRequest {
                request_id: self.request_id(),
                execution_id,
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if details.execution_id != execution_id {
            return Err(ClientError::Protocol(
                "execution details response execution_id mismatch".into(),
            ));
        }
        Ok(details)
    }
    async fn get_agent(&self, w: &str, a: &str) -> Result<Agent, ClientError> {
        expect_agent(
            w,
            Some(a),
            self.client
                .clone()
                .get_agent(wire::GetAgentRequest {
                    request_id: self.request_id(),
                    agent: Some(aref(w, a)),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn create_agent(&self, w: &str, mut d: AgentDraft) -> Result<Agent, ClientError> {
        validate_agent_choice(&d.harness_id, &d.cli_path)?;
        if d.machine_id.is_empty() {
            let machines = self
                .get_machine_catalog(None)
                .await?
                .into_iter()
                .filter(|c| c.registered && c.revoked_at.is_none())
                .collect::<Vec<_>>();
            if let [machine] = machines.as_slice() {
                d.machine_id = machine.id.clone();
            }
        }
        let expected = d.id.clone();
        expect_agent(
            w,
            Some(&expected),
            self.client
                .clone()
                .create_agent(wire::CreateAgentRequest {
                    figure: Default::default(),
                    launch_arguments: Default::default(),
                    request_id: self.request_id(),
                    agent: Some(aref(w, &d.id)),
                    name: d.name,
                    role: d.role,
                    bio: d.bio,
                    harness_id: d.harness_id,
                    model: d.model,
                    thinking_level: d.thinking_level.label().into(),
                    auto_reaction: d.auto_reaction.unwrap_or_default(),
                    soul: d.soul,
                    cli_path: d.cli_path,
                    environment_variables: envs(&d.environment_variables),
                    machine_id: d.machine_id,
                    attention: None,
                    provider_agent_config_json: d.provider_agent_config.to_string(),
                    provider_agent_secrets_json: d.provider_agent_secrets.to_string(),
                })
                .await
                .map_err(map_agent_status)?
                .into_inner(),
        )
    }
    async fn resume_agent(&self, workspace_id: &str, agent_id: &str) -> Result<Agent, ClientError> {
        let client = self.client.clone();
        expect_agent(
            workspace_id,
            Some(agent_id),
            client
                .resume_agent(wire::GetAgentRequest {
                    request_id: self.request_id(),
                    agent: Some(wire::AgentRef {
                        workspace_id: workspace_id.into(),
                        agent_id: agent_id.into(),
                    }),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn update_agent(
        &self,
        a: Agent,
        avatar: Option<AgentAvatarUpload>,
    ) -> Result<Agent, ClientError> {
        validate_agent_choice(&a.harness_id, &a.cli_path)?;
        let expected_workspace = a.workspace_id.clone();
        let expected_agent = a.id.clone();
        expect_agent(
            &expected_workspace,
            Some(&expected_agent),
            self.client
                .clone()
                .update_agent(wire::UpdateAgentRequest {
                    figure: Default::default(),
                    launch_arguments: Default::default(),
                    request_id: self.request_id(),
                    agent: Some(aref(&a.workspace_id, &a.id)),
                    name: Some(a.name),
                    role: Some(a.role),
                    bio: Some(a.bio),
                    harness_id: Some(a.harness_id),
                    model: Some(a.model),
                    thinking_level: Some(a.thinking_level.label().into()),
                    auto_reaction: Some(a.auto_reaction.unwrap_or_default()),
                    soul: Some(a.soul),
                    cli_path: Some(a.cli_path),
                    environment_variables: Some(wire::EnvironmentVariableSet {
                        entries: envs(&a.environment_variables),
                    }),
                    machine_id: Some(a.machine_id),
                    avatar: avatar.map(|avatar| wire::AgentAvatarUpload {
                        media_type: avatar.media_type,
                        bytes: avatar.bytes,
                    }),
                    attention: None,
                    provider_agent_config_json: Some(a.provider_agent_config.to_string()),
                    provider_agent_secrets_json: None,
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn delete_agent(&self, workspace_id: &str, agent_id: &str) -> Result<(), ClientError> {
        let response = self
            .client
            .clone()
            .delete_agent(wire::DeleteAgentRequest {
                request_id: self.request_id(),
                agent: Some(aref(workspace_id, agent_id)),
            })
            .await
            .map_err(map_status)?
            .into_inner();
        let deleted = response
            .agent
            .ok_or_else(|| ClientError::Protocol("missing deleted Agent reference".into()))?;
        if deleted.workspace_id != workspace_id || deleted.agent_id != agent_id {
            return Err(ClientError::Protocol(
                "deleted Agent reference mismatch".into(),
            ));
        }
        Ok(())
    }
    async fn get_provider_onboarding(
        &self,
        w: &str,
    ) -> Result<ProviderOnboardingState, ClientError> {
        from_onboarding(
            self.client
                .clone()
                .get_provider_onboarding(wire::GetProviderOnboardingRequest {
                    request_id: self.request_id(),
                    workspace: Some(wref(w)),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn advance_provider_onboarding(
        &self,
        w: &str,
        a: &str,
        p: Value,
    ) -> Result<ProviderOnboardingState, ClientError> {
        from_onboarding(
            self.client
                .clone()
                .advance_provider_onboarding(wire::AdvanceProviderOnboardingRequest {
                    request_id: self.request_id(),
                    workspace: Some(wref(w)),
                    action: a.into(),
                    field_values_json: p.to_string(),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn get_agent_onboarding(
        &self,
        w: &str,
        a: &str,
    ) -> Result<AgentOnboardingState, ClientError> {
        from_agent_onboarding(
            self.client
                .clone()
                .get_agent_onboarding(wire::GetAgentOnboardingRequest {
                    request_id: self.request_id(),
                    agent: Some(aref(w, a)),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn advance_agent_onboarding(
        &self,
        w: &str,
        a: &str,
        action: &str,
        payload: Value,
    ) -> Result<AgentOnboardingState, ClientError> {
        from_agent_onboarding(
            self.client
                .clone()
                .advance_agent_onboarding(wire::AdvanceAgentOnboardingRequest {
                    request_id: self.request_id(),
                    agent: Some(aref(w, a)),
                    action: action.into(),
                    field_values_json: payload.to_string(),
                })
                .await
                .map_err(map_status)?
                .into_inner(),
        )
    }
    async fn get_provider_catalog(&self) -> Result<Vec<ProviderInfo>, ClientError> {
        Ok(self
            .client
            .clone()
            .get_provider_catalog(catalog(self.request_id(), None))
            .await
            .map_err(map_status)?
            .into_inner()
            .providers
            .into_iter()
            .map(|p| ProviderInfo {
                id: p.provider_id,
                name: p.display_name,
                icon: String::new(),
            })
            .collect())
    }
    async fn get_provider_capability_catalog(
        &self,
    ) -> Result<Vec<ProviderCapabilityCatalogEntry>, ClientError> {
        Ok(self
            .client
            .clone()
            .get_provider_catalog(catalog(self.request_id(), None))
            .await
            .map_err(map_status)?
            .into_inner()
            .providers
            .into_iter()
            .map(|provider| ProviderCapabilityCatalogEntry {
                provider_id: provider.provider_id,
                capabilities: provider.capabilities,
            })
            .collect())
    }
    async fn enable_remote_access(&self) -> Result<wire::RemoteAccess, ClientError> {
        self.client.enable_remote_access().await.map_err(map_status)
    }
    async fn get_remote_access(&self) -> Result<wire::RemoteAccess, ClientError> {
        self.client.get_remote_access().await.map_err(map_status)
    }
    async fn set_remote_access(
        &self,
        address: Option<String>,
    ) -> Result<wire::RemoteAccess, ClientError> {
        self.client
            .set_remote_access(address)
            .await
            .map_err(map_status)
    }
    async fn create_pairing_code(&self) -> Result<PairingCode, ClientError> {
        let value = self
            .client
            .clone()
            .create_pairing_code(catalog(self.request_id(), None))
            .await
            .map_err(map_status)?
            .into_inner();
        Ok(PairingCode {
            code: value.code,
            issued_at: value.issued_at,
            expires_at: value.expires_at,
        })
    }
    async fn rename_machine(&self, machine_id: &str, name: &str) -> Result<(), ClientError> {
        self.client
            .clone()
            .rename_machine(wire::RenameMachineRequest {
                request_id: self.request_id(),
                machine_id: machine_id.into(),
                name: name.into(),
            })
            .await
            .map_err(map_status)?;
        Ok(())
    }
    async fn revoke_machine(&self, machine_id: &str) -> Result<(), ClientError> {
        self.client
            .clone()
            .revoke_machine(wire::RevokeMachineRequest {
                request_id: self.request_id(),
                machine_id: machine_id.into(),
            })
            .await
            .map_err(map_status)?;
        Ok(())
    }
    async fn get_machine_catalog(
        &self,
        w: Option<&str>,
    ) -> Result<Vec<MachineOption>, ClientError> {
        Ok(self
            .client
            .clone()
            .get_machine_catalog(catalog(self.request_id(), w))
            .await
            .map_err(map_status)?
            .into_inner()
            .machines
            .into_iter()
            .map(|c| MachineOption {
                is_local: c.is_local,
                registered: c.registered,
                revoked_at: c.revoked_at,
                last_seen_at: c.last_seen_at,
                paired_at: c.paired_at,
                os: c.os,
                version: c.version,
                inventory: c.harnesses,
                id: c.machine_id,
                name: c.display_name,
                available: c.observed_state == wire::ObservedState::Running,
            })
            .collect())
    }
    async fn get_harness_catalog(
        &self,
        w: Option<&str>,
    ) -> Result<Vec<HarnessOption>, ClientError> {
        Ok(self
            .client
            .clone()
            .get_harness_catalog(catalog(self.request_id(), w))
            .await
            .map_err(map_status)?
            .into_inner()
            .harnesses
            .into_iter()
            .map(|h| HarnessOption {
                id: h.harness_id,
                name: h.display_name,
                executable_path: None,
            })
            .collect())
    }
    async fn get_model_catalog(
        &self,
        w: Option<&str>,
        c: &str,
        h: &str,
    ) -> Result<Vec<ModelOption>, ClientError> {
        Ok(self
            .client
            .clone()
            .get_model_catalog(wire::ModelCatalogRequest {
                request_id: self.request_id(),
                workspace: w.map(wref),
                machine_id: c.into(),
                harness_id: h.into(),
            })
            .await
            .map_err(map_status)?
            .into_inner()
            .models
            .into_iter()
            .map(|m| ModelOption {
                thinking_levels: m.supported_thinking_levels,
                id: m.model_id,
                name: m.display_name,
                available: m.available,
            })
            .collect())
    }
    async fn get_reaction_catalog(
        &self,
        w: Option<&str>,
    ) -> Result<Vec<ReactionOption>, ClientError> {
        Ok(self
            .client
            .clone()
            .get_reaction_catalog(catalog(self.request_id(), w))
            .await
            .map_err(map_status)?
            .into_inner()
            .reactions
            .into_iter()
            .map(|r| ReactionOption {
                id: r.reaction_id,
                name: r.display_name,
                emoji: r.emoji,
                available: r.available,
            })
            .collect())
    }
    async fn invoke_provider_interface(
        &self,
        workspace_id: &str,
        interface_id: &str,
        input: Value,
    ) -> Result<Value, ClientError> {
        let request_id = self.request_id();
        let response = self
            .client
            .clone()
            .invoke_provider_interface(wire::InvokeProviderInterfaceRequest {
                request_id: request_id.clone(),
                workspace: Some(wref(workspace_id)),
                interface_id: interface_id.into(),
                input_json: input.to_string(),
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if response.request_id != request_id
            || response
                .workspace
                .as_ref()
                .map(|workspace| workspace.workspace_id.as_str())
                != Some(workspace_id)
            || response.interface_id != interface_id
        {
            return Err(ClientError::Protocol(
                "provider interface response mismatch".into(),
            ));
        }
        serde_json::from_str(&response.output_json).map_err(|error| {
            ClientError::Protocol(format!("invalid provider interface output: {error}"))
        })
    }
    async fn list_provider_interfaces(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<ProviderInterface>, ClientError> {
        let request_id = self.request_id();
        let response = self
            .client
            .clone()
            .list_provider_interfaces(wire::ListProviderInterfacesRequest {
                request_id: request_id.clone(),
                workspace: Some(wref(workspace_id)),
            })
            .await
            .map_err(map_status)?
            .into_inner();
        if response.request_id != request_id
            || response
                .workspace
                .as_ref()
                .map(|workspace| workspace.workspace_id.as_str())
                != Some(workspace_id)
        {
            return Err(ClientError::Protocol(
                "provider interface catalog response mismatch".into(),
            ));
        }
        response
            .interfaces
            .into_iter()
            .map(|interface| {
                Ok(ProviderInterface {
                    id: interface.interface_id,
                    description: interface.description,
                    input_schema: serde_json::from_str(&interface.input_schema_json).map_err(
                        |error| {
                            ClientError::Protocol(format!(
                                "invalid interface input schema: {error}"
                            ))
                        },
                    )?,
                    output_schema: serde_json::from_str(&interface.output_schema_json).map_err(
                        |error| {
                            ClientError::Protocol(format!(
                                "invalid interface output schema: {error}"
                            ))
                        },
                    )?,
                })
            })
            .collect()
    }
    async fn subscribe_management_events(
        &self,
        w: Option<&str>,
        after: u64,
    ) -> Result<ManagementSubscription, ClientError> {
        let stream = self
            .client
            .clone()
            .subscribe_management_events(wire::ManagementSubscription {
                request_id: self.request_id(),
                workspace: w.map(wref),
                after_sequence: after,
            })
            .await
            .map_err(map_status)?
            .into_inner()
            .map(|r| {
                r.map_err(map_status).and_then(|e| {
                    let kind = match e.kind {
                        wire::ManagementEventKind::WorkspaceCreated => {
                            ManagementEventKind::WorkspaceCreated
                        }
                        wire::ManagementEventKind::WorkspaceUpdated => {
                            ManagementEventKind::WorkspaceUpdated
                        }
                        wire::ManagementEventKind::WorkspaceStateChanged => {
                            ManagementEventKind::WorkspaceStateChanged
                        }
                        wire::ManagementEventKind::AgentCreated => {
                            ManagementEventKind::AgentCreated
                        }
                        wire::ManagementEventKind::AgentUpdated => {
                            ManagementEventKind::AgentUpdated
                        }
                        wire::ManagementEventKind::AgentDeleted => {
                            ManagementEventKind::AgentDeleted
                        }
                        wire::ManagementEventKind::ProviderOnboardingChanged => {
                            ManagementEventKind::ProviderOnboardingChanged
                        }
                        wire::ManagementEventKind::MachineCatalogChanged => {
                            ManagementEventKind::MachineCatalogChanged
                        }
                        wire::ManagementEventKind::CatalogChanged => {
                            ManagementEventKind::CatalogChanged
                        }
                        _ => return Err(ClientError::Protocol("unspecified event kind".into())),
                    };
                    let workspace_id = match e.workspace {
                        Some(workspace) => workspace.workspace_id,
                        None if kind == ManagementEventKind::MachineCatalogChanged => String::new(),
                        None => {
                            return Err(ClientError::Protocol("event workspace missing".into()));
                        }
                    };
                    Ok(ManagementEvent {
                        sequence: e.sequence,
                        workspace_id,
                        agent_id: e.agent.map(|a| a.agent_id),
                        kind,
                    })
                })
            });
        Ok(ManagementSubscription {
            inner: Box::pin(stream),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub issued_at: i64,
    pub expires_at: i64,
}
