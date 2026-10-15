//! Shared public protocol vocabulary. Each DTO has one definition.
pub use crate::link::{
    ActiveAttempt, AgentRef, AgentRunContext, AgentRuntimeProfile, AssignExecution,
    AssignmentResult, AttemptFailure, CancelExecution, CapabilityRequest, CapabilityResponse,
    CliCommand, ComputerHeartbeat, ComputerHello, DeleteAgentFiles, DeleteAgentFilesResult,
    DeliverSteer, Delivery, DeliveryResult, EnvironmentVariable, ExecutionCommitted,
    ExecutionEvent, ExecutionEventAck, ExecutionRef, HarnessConfig, HarnessInfo, InventoryChanged,
    ModelCatalog, ModelCatalogItem, PairComputerRequest, PairComputerResponse, ResourceRef,
    ResourceTransferResult, RuntimeHello, SessionDirective, Shutdown, ToolDefinition, ToolError,
    ToolResult, WorkspaceRef,
};
pub use crate::management::{
    AdvanceAgentOnboardingRequest, AdvanceProviderOnboardingRequest, Agent, AgentAttention,
    AgentAvatarFile, AgentAvatarUpload, AgentOnboardingState, CancelAttemptRequest,
    CancelAttemptResponse, CatalogRequest, ChannelAttention, ComputerCatalog, ComputerCatalogItem,
    CreateAgentRequest, CreateWorkspaceRequest, DeleteAgentRequest, DeleteAgentResponse,
    EnvironmentVariableSet, ExecutionAttemptDetails, ExecutionDetails, GetAgentAvatarRequest,
    GetAgentAvatarResponse, GetAgentOnboardingRequest, GetAgentRequest, GetExecutionDetailsRequest,
    GetProviderOnboardingRequest, GetWorkspaceRequest, HarnessCatalog, HarnessCatalogItem,
    InvokeProviderInterfaceRequest, InvokeProviderInterfaceResponse, ListAgentsRequest,
    ListAgentsResponse, ListProviderInterfacesRequest, ListProviderInterfacesResponse,
    ListWorkspacesRequest, ListWorkspacesResponse, ManagementEvent, ManagementSubscription,
    ModelCatalogRequest, PairingCode, ProviderCatalog, ProviderCatalogItem, ProviderInterface,
    ProviderOnboardingState, ReactionCatalog, ReactionCatalogItem, RemoteAccess,
    RenameComputerRequest, RevokeComputerRequest, SetRemoteAccessRequest, SetWorkspaceStateRequest,
    UpdateAgentRequest, UpdateWorkspaceRequest, UploadWorkspaceImageRequest,
    UploadWorkspaceImageResponse, Workspace, WorkspaceImageRequest, WorkspaceImageResponse,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ResourceDescriptor {
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub context: Option<AgentRunContext>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ResourceChunk {
    pub transfer_id: String,
    pub resource_id: i64,
    pub offset: u64,
    #[serde(with = "crate::management::base64_bytes")]
    pub bytes: Vec<u8>,
    pub eof: bool,
    pub descriptor: Option<ResourceDescriptor>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ResourceRequest {
    pub resource_id: i64,
    pub transfer_id: String,
    pub context: Option<AgentRunContext>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct Capability {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub kind: i32,
    pub input_schema_json: String,
    pub output_schema_json: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CapabilityList {
    pub capabilities: Vec<Capability>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AgentProfile {
    pub agent: Option<AgentRef>,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Option<AgentAvatarFile>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct UpdateSelfProfileRequest {
    pub context: Option<AgentRunContext>,
    pub name: Option<String>,
    pub role: Option<String>,
    pub bio: Option<String>,
    pub avatar: Option<AgentAvatarUpload>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CapabilityInvokeRequest {
    pub context: Option<AgentRunContext>,
    pub capability_name: String,
    pub arguments_json: String,
    pub invocation_id: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CapabilityInvokeResult {
    pub invocation_id: String,
    pub success: bool,
    pub content_markdown: String,
    pub structured_content_json: String,
    pub resources: Vec<ResourceRef>,
    pub error: Option<ToolError>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct SendMessageRequest {
    pub context: Option<AgentRunContext>,
    pub channel_id: String,
    pub chat_id: String,
    pub content_markdown: String,
    pub reply_to_message_id: Option<i64>,
    pub resources: Vec<ResourceRef>,
    pub idempotency_key: String,
    pub force: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct SendMessageResult {
    pub accepted: bool,
    pub message_id: i64,
    pub provider_message_id: String,
    pub error: Option<ToolError>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ContentRequest {
    pub context: Option<AgentRunContext>,
    pub kind: i32,
    pub content_id: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ContentResult {
    pub content_id: String,
    pub content_markdown: String,
    pub structured_content_json: String,
    pub resources: Vec<ResourceRef>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum HarnessAvailability {
    Unspecified = 0,
    Available = 1,
    Unavailable = 2,
    Unknown = 3,
}
impl Default for HarnessAvailability {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for HarnessAvailability {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Available),
            2 => Ok(Self::Unavailable),
            3 => Ok(Self::Unknown),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum SessionAction {
    Unspecified = 0,
    ResumeOrStart = 1,
    StartNew = 2,
}
impl Default for SessionAction {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for SessionAction {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::ResumeOrStart),
            2 => Ok(Self::StartNew),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum DeliveryKind {
    Unspecified = 0,
    Initial = 1,
    Steer = 2,
}
impl Default for DeliveryKind {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for DeliveryKind {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Initial),
            2 => Ok(Self::Steer),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ExecutionEventKind {
    Unspecified = 0,
    Started = 1,
    NativeSessionBound = 2,
    AssistantDelta = 3,
    ReasoningDelta = 4,
    ToolStarted = 5,
    ToolFinished = 6,
    Usage = 7,
    Warning = 8,
    Completed = 9,
    Failed = 10,
    Cancelled = 11,
}
impl Default for ExecutionEventKind {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for ExecutionEventKind {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Started),
            2 => Ok(Self::NativeSessionBound),
            3 => Ok(Self::AssistantDelta),
            4 => Ok(Self::ReasoningDelta),
            5 => Ok(Self::ToolStarted),
            6 => Ok(Self::ToolFinished),
            7 => Ok(Self::Usage),
            8 => Ok(Self::Warning),
            9 => Ok(Self::Completed),
            10 => Ok(Self::Failed),
            11 => Ok(Self::Cancelled),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum DesiredState {
    Unspecified = 0,
    Running = 1,
    Paused = 2,
}
impl Default for DesiredState {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for DesiredState {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Running),
            2 => Ok(Self::Paused),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ObservedState {
    Unspecified = 0,
    Starting = 1,
    Running = 2,
    Paused = 3,
    Error = 4,
    Stopped = 5,
}
impl Default for ObservedState {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for ObservedState {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Starting),
            2 => Ok(Self::Running),
            3 => Ok(Self::Paused),
            4 => Ok(Self::Error),
            5 => Ok(Self::Stopped),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum AttentionMode {
    Unspecified = 0,
    Mention = 1,
    Always = 2,
    Mute = 3,
    Schedule = 4,
}
impl Default for AttentionMode {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for AttentionMode {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Mention),
            2 => Ok(Self::Always),
            3 => Ok(Self::Mute),
            4 => Ok(Self::Schedule),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum OnboardingStatus {
    Unspecified = 0,
    ActionRequired = 1,
    Verifying = 2,
    Ready = 3,
    Error = 4,
}
impl Default for OnboardingStatus {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for OnboardingStatus {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::ActionRequired),
            2 => Ok(Self::Verifying),
            3 => Ok(Self::Ready),
            4 => Ok(Self::Error),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ManagementEventKind {
    Unspecified = 0,
    WorkspaceCreated = 1,
    WorkspaceUpdated = 2,
    WorkspaceStateChanged = 3,
    AgentCreated = 4,
    AgentUpdated = 5,
    ProviderOnboardingChanged = 6,
    CatalogChanged = 7,
    AgentDeleted = 8,
    ComputerCatalogChanged = 9,
}
impl Default for ManagementEventKind {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for ManagementEventKind {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::WorkspaceCreated),
            2 => Ok(Self::WorkspaceUpdated),
            3 => Ok(Self::WorkspaceStateChanged),
            4 => Ok(Self::AgentCreated),
            5 => Ok(Self::AgentUpdated),
            6 => Ok(Self::ProviderOnboardingChanged),
            7 => Ok(Self::CatalogChanged),
            8 => Ok(Self::AgentDeleted),
            9 => Ok(Self::ComputerCatalogChanged),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum CapabilityKind {
    Unspecified = 0,
    Tool = 1,
    CliCommand = 2,
    ProviderInterface = 3,
}
impl Default for CapabilityKind {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for CapabilityKind {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Tool),
            2 => Ok(Self::CliCommand),
            3 => Ok(Self::ProviderInterface),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ContentKind {
    Unspecified = 0,
    Message = 1,
    Task = 2,
    ProviderObject = 3,
}
impl Default for ContentKind {
    fn default() -> Self {
        Self::Unspecified
    }
}
impl TryFrom<i32> for ContentKind {
    type Error = crate::Status;
    fn try_from(value: i32) -> Result<Self, crate::Status> {
        match value {
            0 => Ok(Self::Unspecified),
            1 => Ok(Self::Message),
            2 => Ok(Self::Task),
            3 => Ok(Self::ProviderObject),
            _ => Err(crate::Status::invalid_argument("unknown_enum_value")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Eq)]
pub struct ComputerFrame {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub frame: Option<crate::link::ComputerPayload>,
}
pub mod computer_frame {
    pub use crate::link::ComputerPayload as Frame;
}
#[derive(Clone, Debug, PartialEq, Serialize, Eq)]
pub struct RuntimeFrame {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub frame: Option<crate::link::RuntimePayload>,
}
pub mod runtime_frame {
    pub use crate::link::RuntimePayload as Frame;
}

impl From<HarnessAvailability> for i32 {
    fn from(value: HarnessAvailability) -> Self {
        value as i32
    }
}

impl From<SessionAction> for i32 {
    fn from(value: SessionAction) -> Self {
        value as i32
    }
}

impl From<DeliveryKind> for i32 {
    fn from(value: DeliveryKind) -> Self {
        value as i32
    }
}

impl From<ExecutionEventKind> for i32 {
    fn from(value: ExecutionEventKind) -> Self {
        value as i32
    }
}

impl From<DesiredState> for i32 {
    fn from(value: DesiredState) -> Self {
        value as i32
    }
}

impl From<ObservedState> for i32 {
    fn from(value: ObservedState) -> Self {
        value as i32
    }
}

impl From<AttentionMode> for i32 {
    fn from(value: AttentionMode) -> Self {
        value as i32
    }
}

impl From<OnboardingStatus> for i32 {
    fn from(value: OnboardingStatus) -> Self {
        value as i32
    }
}

impl From<ManagementEventKind> for i32 {
    fn from(value: ManagementEventKind) -> Self {
        value as i32
    }
}

impl From<CapabilityKind> for i32 {
    fn from(value: CapabilityKind) -> Self {
        value as i32
    }
}

impl From<ContentKind> for i32 {
    fn from(value: ContentKind) -> Self {
        value as i32
    }
}

impl<'de> Deserialize<'de> for ComputerFrame {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Envelope {
            protocol_version: u32,
            #[serde(flatten)]
            frame: crate::link::ComputerPayload,
        }
        let value = Envelope::deserialize(deserializer)?;
        Ok(Self {
            protocol_version: value.protocol_version,
            frame: Some(value.frame),
        })
    }
}

impl<'de> Deserialize<'de> for RuntimeFrame {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Envelope {
            protocol_version: u32,
            #[serde(flatten)]
            frame: crate::link::RuntimePayload,
        }
        let value = Envelope::deserialize(deserializer)?;
        Ok(Self {
            protocol_version: value.protocol_version,
            frame: Some(value.frame),
        })
    }
}
