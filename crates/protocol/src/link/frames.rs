//! Canonical JSON Machine link vocabulary.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ActiveAttempt {
    pub execution: Option<ExecutionRef>,
    pub last_event_sequence: u64,
    pub harness_process_state: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AgentRef {
    pub workspace_id: String,
    pub agent_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AgentRuntimeProfile {
    pub agent: Option<AgentRef>,
    pub name: String,
    pub role: String,
    pub bio: String,
    #[serde(default)]
    pub soul: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AssignExecution {
    pub execution: Option<ExecutionRef>,
    pub workspace: Option<WorkspaceRef>,
    pub agent: Option<AgentRuntimeProfile>,
    pub harness: Option<HarnessConfig>,
    pub session: Option<SessionDirective>,
    pub initial_delivery: Option<Delivery>,
    pub system_prompt: String,
    pub tools: Vec<ToolDefinition>,
    pub cli_commands: Vec<CliCommand>,
    pub resources: Vec<ResourceRef>,
    pub execution_token: String,
    pub created_at: i64,
    /// Context rebuilt from Runtime records. Machine places it before the
    /// delivery only when the Harness starts a fresh native session.
    #[serde(default)]
    pub recovery_markdown: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AssignmentResult {
    pub execution: Option<ExecutionRef>,
    pub accepted: bool,
    pub error_code: String,
    pub error_message: String,
    pub failure: Option<AttemptFailure>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AttemptFailure {
    pub code: String,
    pub category: String,
    pub retryable: bool,
    pub fingerprint: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CancelExecution {
    pub execution: Option<ExecutionRef>,
    pub reason: String,
    pub force_close: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CliCommand {
    pub name: String,
    pub description: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub wraps: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct MachineHeartbeat {
    pub machine_id: String,
    pub instance_id: String,
    pub sent_at: i64,
    pub active_attempts: Vec<ActiveAttempt>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct MachineHello {
    pub device_credential: String,
    pub protocol_version: u32,
    pub machine_id: String,
    pub instance_id: String,
    pub machine_version: String,
    pub os: String,
    pub arch: String,
    pub harnesses: Vec<HarnessInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct DeleteAgentFiles {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct DeleteAgentFilesResult {
    pub request_id: String,
    pub success: bool,
    pub error_code: String,
    pub error_message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct DeliverSteer {
    pub execution: Option<ExecutionRef>,
    pub delivery: Option<Delivery>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct Delivery {
    pub delivery_id: i64,
    pub kind: i32,
    pub rendered_markdown: String,
    pub snapshot_id: i64,
    pub trigger_reasons: Vec<String>,
    pub resources: Vec<ResourceRef>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct DeliveryResult {
    pub execution: Option<ExecutionRef>,
    pub delivery_id: i64,
    pub accepted: bool,
    pub error_code: String,
    pub error_message: String,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ExecutionCommitted {
    pub execution: Option<ExecutionRef>,
    pub committed_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ExecutionEvent {
    pub execution: Option<ExecutionRef>,
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: i32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ExecutionEventAck {
    pub execution: Option<ExecutionRef>,
    pub max_saved_sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, Eq, Hash)]
pub struct ExecutionRef {
    pub execution_id: i64,
    pub attempt: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct HarnessConfig {
    pub harness_id: String,
    pub executable_path: String,
    pub launch_arguments: Vec<String>,
    pub model: String,
    pub thinking_level: String,
    pub environment_variables: Vec<EnvironmentVariable>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct HarnessInfo {
    pub harness_id: String,
    pub display_name: String,
    pub availability: i32,
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct InventoryChanged {
    pub harnesses: Vec<HarnessInfo>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ModelCatalog {
    pub models: Vec<ModelCatalogItem>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ModelCatalogItem {
    pub model_id: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
    pub supported_thinking_levels: Vec<String>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ResourceRef {
    pub resource_id: i64,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ResourceTransferResult {
    pub transfer_id: String,
    pub success: bool,
    pub error_code: String,
    pub error_message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct RuntimeHello {
    pub protocol_version: u32,
    pub runtime_instance_id: String,
    pub heartbeat_interval_ms: u64,
    pub max_message_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct SessionDirective {
    pub logical_session_id: String,
    pub action: i32,
    pub native_session_id: Option<String>,
    pub config_fingerprint: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct Shutdown {
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema_json: String,
    pub output_schema_json: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct ToolResult {
    pub execution: Option<ExecutionRef>,
    pub tool_call_id: String,
    pub success: bool,
    pub content_markdown: String,
    pub structured_content_json: String,
    pub resources: Vec<ResourceRef>,
    pub error: Option<ToolError>,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct WorkspaceRef {
    pub workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum MachinePayload {
    CapabilityRequest(CapabilityRequest),
    Hello(MachineHello),
    Heartbeat(MachineHeartbeat),
    AssignmentResult(AssignmentResult),
    ExecutionEvent(ExecutionEvent),
    DeliveryResult(DeliveryResult),
    ResourceTransferResult(ResourceTransferResult),
    InventoryChanged(InventoryChanged),
    DeleteAgentFilesResult(DeleteAgentFilesResult),
    TestHarnessResult(TestHarnessResult),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimePayload {
    CapabilityResponse(CapabilityResponse),
    Heartbeat,
    Hello(RuntimeHello),
    AssignExecution(AssignExecution),
    DeliverSteer(DeliverSteer),
    CancelExecution(CancelExecution),
    ExecutionEventAck(ExecutionEventAck),
    ExecutionCommitted(ExecutionCommitted),
    Shutdown(Shutdown),
    DeleteAgentFiles(DeleteAgentFiles),
    TestHarness(TestHarness),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairMachineRequest {
    pub code: String,
    pub display_name: String,
    pub os: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairMachineResponse {
    pub machine_id: String,
    pub device_credential: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CapabilityRequest {
    pub request_id: String,
    pub method: String,
    pub context: Option<AgentRunContext>,
    pub payload: Vec<u8>,
    pub offset: u64,
    pub eof: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct CapabilityResponse {
    pub request_id: String,
    pub payload: Vec<u8>,
    pub offset: u64,
    pub eof: bool,
    pub error_code: i32,
    pub error_message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Eq)]
pub struct AgentRunContext {
    pub execution: Option<ExecutionRef>,
    pub execution_token: String,
}

pub use crate::wire::{MachineFrame, RuntimeFrame};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq)]
pub struct TestHarness {
    pub request_id: String,
    pub harness: HarnessConfig,
    pub budget_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Eq)]
pub struct TestHarnessResult {
    pub request_id: String,
    pub result: crate::management::TestHarnessResponse,
}
