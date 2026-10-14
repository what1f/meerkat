//! JSON Computer link vocabulary. These DTOs are independent of prost.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveAttempt {
    pub execution: Option<ExecutionRef>,
    pub last_event_sequence: u64,
    pub harness_process_state: String,
}
impl From<crate::wire::ActiveAttempt> for ActiveAttempt {
    fn from(value: crate::wire::ActiveAttempt) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            last_event_sequence: value.last_event_sequence,
            harness_process_state: value.harness_process_state,
        }
    }
}
impl From<ActiveAttempt> for crate::wire::ActiveAttempt {
    fn from(value: ActiveAttempt) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            last_event_sequence: value.last_event_sequence,
            harness_process_state: value.harness_process_state,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentRef {
    pub workspace_id: String,
    pub agent_id: String,
}
impl From<crate::wire::AgentRef> for AgentRef {
    fn from(value: crate::wire::AgentRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
            agent_id: value.agent_id,
        }
    }
}
impl From<AgentRef> for crate::wire::AgentRef {
    fn from(value: AgentRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
            agent_id: value.agent_id,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentRuntimeProfile {
    pub agent: Option<AgentRef>,
    pub name: String,
    pub role: String,
    pub bio: String,
}
impl From<crate::wire::AgentRuntimeProfile> for AgentRuntimeProfile {
    fn from(value: crate::wire::AgentRuntimeProfile) -> Self {
        Self {
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
        }
    }
}
impl From<AgentRuntimeProfile> for crate::wire::AgentRuntimeProfile {
    fn from(value: AgentRuntimeProfile) -> Self {
        Self {
            agent: value.agent.map(Into::into),
            name: value.name,
            role: value.role,
            bio: value.bio,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
}
impl From<crate::wire::AssignExecution> for AssignExecution {
    fn from(value: crate::wire::AssignExecution) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            workspace: value.workspace.map(Into::into),
            agent: value.agent.map(Into::into),
            harness: value.harness.map(Into::into),
            session: value.session.map(Into::into),
            initial_delivery: value.initial_delivery.map(Into::into),
            system_prompt: value.system_prompt,
            tools: value.tools.into_iter().map(Into::into).collect(),
            cli_commands: value.cli_commands.into_iter().map(Into::into).collect(),
            resources: value.resources.into_iter().map(Into::into).collect(),
            execution_token: value.execution_token,
            created_at: value.created_at,
        }
    }
}
impl From<AssignExecution> for crate::wire::AssignExecution {
    fn from(value: AssignExecution) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            workspace: value.workspace.map(Into::into),
            agent: value.agent.map(Into::into),
            harness: value.harness.map(Into::into),
            session: value.session.map(Into::into),
            initial_delivery: value.initial_delivery.map(Into::into),
            system_prompt: value.system_prompt,
            tools: value.tools.into_iter().map(Into::into).collect(),
            cli_commands: value.cli_commands.into_iter().map(Into::into).collect(),
            resources: value.resources.into_iter().map(Into::into).collect(),
            execution_token: value.execution_token,
            created_at: value.created_at,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AssignmentResult {
    pub execution: Option<ExecutionRef>,
    pub accepted: bool,
    pub error_code: String,
    pub error_message: String,
    pub failure: Option<AttemptFailure>,
}
impl From<crate::wire::AssignmentResult> for AssignmentResult {
    fn from(value: crate::wire::AssignmentResult) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            accepted: value.accepted,
            error_code: value.error_code,
            error_message: value.error_message,
            failure: value.failure.map(Into::into),
        }
    }
}
impl From<AssignmentResult> for crate::wire::AssignmentResult {
    fn from(value: AssignmentResult) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            accepted: value.accepted,
            error_code: value.error_code,
            error_message: value.error_message,
            failure: value.failure.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AttemptFailure {
    pub code: String,
    pub category: String,
    pub retryable: bool,
    pub fingerprint: String,
    pub detail: String,
}
impl From<crate::wire::AttemptFailure> for AttemptFailure {
    fn from(value: crate::wire::AttemptFailure) -> Self {
        Self {
            code: value.code,
            category: value.category,
            retryable: value.retryable,
            fingerprint: value.fingerprint,
            detail: value.detail,
        }
    }
}
impl From<AttemptFailure> for crate::wire::AttemptFailure {
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CancelExecution {
    pub execution: Option<ExecutionRef>,
    pub reason: String,
    pub force_close: bool,
}
impl From<crate::wire::CancelExecution> for CancelExecution {
    fn from(value: crate::wire::CancelExecution) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            reason: value.reason,
            force_close: value.force_close,
        }
    }
}
impl From<CancelExecution> for crate::wire::CancelExecution {
    fn from(value: CancelExecution) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            reason: value.reason,
            force_close: value.force_close,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CliCommand {
    pub name: String,
    pub description: String,
    pub argv: Vec<String>,
}
impl From<crate::wire::CliCommand> for CliCommand {
    fn from(value: crate::wire::CliCommand) -> Self {
        Self {
            name: value.name,
            description: value.description,
            argv: value.argv,
        }
    }
}
impl From<CliCommand> for crate::wire::CliCommand {
    fn from(value: CliCommand) -> Self {
        Self {
            name: value.name,
            description: value.description,
            argv: value.argv,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputerHeartbeat {
    pub computer_id: String,
    pub instance_id: String,
    pub sent_at: i64,
    pub active_attempts: Vec<ActiveAttempt>,
}
impl From<crate::wire::ComputerHeartbeat> for ComputerHeartbeat {
    fn from(value: crate::wire::ComputerHeartbeat) -> Self {
        Self {
            computer_id: value.computer_id,
            instance_id: value.instance_id,
            sent_at: value.sent_at,
            active_attempts: value.active_attempts.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<ComputerHeartbeat> for crate::wire::ComputerHeartbeat {
    fn from(value: ComputerHeartbeat) -> Self {
        Self {
            computer_id: value.computer_id,
            instance_id: value.instance_id,
            sent_at: value.sent_at,
            active_attempts: value.active_attempts.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputerHello {
    #[serde(default)]
    pub device_credential: String,
    pub protocol_version: u32,
    pub computer_id: String,
    pub instance_id: String,
    pub computer_version: String,
    pub os: String,
    pub arch: String,
    pub harnesses: Vec<HarnessInfo>,
}
impl From<crate::wire::ComputerHello> for ComputerHello {
    fn from(value: crate::wire::ComputerHello) -> Self {
        Self {
            protocol_version: value.protocol_version,
            computer_id: value.computer_id,
            device_credential: value.device_credential,
            instance_id: value.instance_id,
            computer_version: value.computer_version,
            os: value.os,
            arch: value.arch,
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<ComputerHello> for crate::wire::ComputerHello {
    fn from(value: ComputerHello) -> Self {
        Self {
            protocol_version: value.protocol_version,
            computer_id: value.computer_id,
            device_credential: value.device_credential,
            instance_id: value.instance_id,
            computer_version: value.computer_version,
            os: value.os,
            arch: value.arch,
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteAgentFiles {
    pub request_id: String,
    pub agent: Option<AgentRef>,
}
impl From<crate::wire::DeleteAgentFiles> for DeleteAgentFiles {
    fn from(value: crate::wire::DeleteAgentFiles) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
impl From<DeleteAgentFiles> for crate::wire::DeleteAgentFiles {
    fn from(value: DeleteAgentFiles) -> Self {
        Self {
            request_id: value.request_id,
            agent: value.agent.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeleteAgentFilesResult {
    pub request_id: String,
    pub success: bool,
    pub error_code: String,
    pub error_message: String,
}
impl From<crate::wire::DeleteAgentFilesResult> for DeleteAgentFilesResult {
    fn from(value: crate::wire::DeleteAgentFilesResult) -> Self {
        Self {
            request_id: value.request_id,
            success: value.success,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
impl From<DeleteAgentFilesResult> for crate::wire::DeleteAgentFilesResult {
    fn from(value: DeleteAgentFilesResult) -> Self {
        Self {
            request_id: value.request_id,
            success: value.success,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliverSteer {
    pub execution: Option<ExecutionRef>,
    pub delivery: Option<Delivery>,
}
impl From<crate::wire::DeliverSteer> for DeliverSteer {
    fn from(value: crate::wire::DeliverSteer) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            delivery: value.delivery.map(Into::into),
        }
    }
}
impl From<DeliverSteer> for crate::wire::DeliverSteer {
    fn from(value: DeliverSteer) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            delivery: value.delivery.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub delivery_id: i64,
    pub kind: i32,
    pub rendered_markdown: String,
    pub snapshot_id: i64,
    pub trigger_reasons: Vec<String>,
    pub resources: Vec<ResourceRef>,
    pub created_at: i64,
}
impl From<crate::wire::Delivery> for Delivery {
    fn from(value: crate::wire::Delivery) -> Self {
        Self {
            delivery_id: value.delivery_id,
            kind: value.kind,
            rendered_markdown: value.rendered_markdown,
            snapshot_id: value.snapshot_id,
            trigger_reasons: value.trigger_reasons,
            resources: value.resources.into_iter().map(Into::into).collect(),
            created_at: value.created_at,
        }
    }
}
impl From<Delivery> for crate::wire::Delivery {
    fn from(value: Delivery) -> Self {
        Self {
            delivery_id: value.delivery_id,
            kind: value.kind,
            rendered_markdown: value.rendered_markdown,
            snapshot_id: value.snapshot_id,
            trigger_reasons: value.trigger_reasons,
            resources: value.resources.into_iter().map(Into::into).collect(),
            created_at: value.created_at,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeliveryResult {
    pub execution: Option<ExecutionRef>,
    pub delivery_id: i64,
    pub accepted: bool,
    pub error_code: String,
    pub error_message: String,
}
impl From<crate::wire::DeliveryResult> for DeliveryResult {
    fn from(value: crate::wire::DeliveryResult) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            delivery_id: value.delivery_id,
            accepted: value.accepted,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
impl From<DeliveryResult> for crate::wire::DeliveryResult {
    fn from(value: DeliveryResult) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            delivery_id: value.delivery_id,
            accepted: value.accepted,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub name: String,
    pub value: String,
}
impl From<crate::wire::EnvironmentVariable> for EnvironmentVariable {
    fn from(value: crate::wire::EnvironmentVariable) -> Self {
        Self {
            name: value.name,
            value: value.value,
        }
    }
}
impl From<EnvironmentVariable> for crate::wire::EnvironmentVariable {
    fn from(value: EnvironmentVariable) -> Self {
        Self {
            name: value.name,
            value: value.value,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionCommitted {
    pub execution: Option<ExecutionRef>,
    pub committed_at: i64,
}
impl From<crate::wire::ExecutionCommitted> for ExecutionCommitted {
    fn from(value: crate::wire::ExecutionCommitted) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            committed_at: value.committed_at,
        }
    }
}
impl From<ExecutionCommitted> for crate::wire::ExecutionCommitted {
    fn from(value: ExecutionCommitted) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            committed_at: value.committed_at,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub execution: Option<ExecutionRef>,
    pub sequence: u64,
    pub occurred_at: i64,
    pub kind: i32,
    pub payload: Vec<u8>,
}
impl From<crate::wire::ExecutionEvent> for ExecutionEvent {
    fn from(value: crate::wire::ExecutionEvent) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            sequence: value.sequence,
            occurred_at: value.occurred_at,
            kind: value.kind,
            payload: value.payload,
        }
    }
}
impl From<ExecutionEvent> for crate::wire::ExecutionEvent {
    fn from(value: ExecutionEvent) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            sequence: value.sequence,
            occurred_at: value.occurred_at,
            kind: value.kind,
            payload: value.payload,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEventAck {
    pub execution: Option<ExecutionRef>,
    pub max_saved_sequence: u64,
}
impl From<crate::wire::ExecutionEventAck> for ExecutionEventAck {
    fn from(value: crate::wire::ExecutionEventAck) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            max_saved_sequence: value.max_saved_sequence,
        }
    }
}
impl From<ExecutionEventAck> for crate::wire::ExecutionEventAck {
    fn from(value: ExecutionEventAck) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            max_saved_sequence: value.max_saved_sequence,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRef {
    pub execution_id: i64,
    pub attempt: u32,
}
impl From<crate::wire::ExecutionRef> for ExecutionRef {
    fn from(value: crate::wire::ExecutionRef) -> Self {
        Self {
            execution_id: value.execution_id,
            attempt: value.attempt,
        }
    }
}
impl From<ExecutionRef> for crate::wire::ExecutionRef {
    fn from(value: ExecutionRef) -> Self {
        Self {
            execution_id: value.execution_id,
            attempt: value.attempt,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HarnessConfig {
    pub harness_id: String,
    pub executable_path: String,
    pub model: String,
    pub thinking_level: String,
    pub environment_variables: Vec<EnvironmentVariable>,
}
impl From<crate::wire::HarnessConfig> for HarnessConfig {
    fn from(value: crate::wire::HarnessConfig) -> Self {
        Self {
            harness_id: value.harness_id,
            executable_path: value.executable_path,
            model: value.model,
            thinking_level: value.thinking_level,
            environment_variables: value
                .environment_variables
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}
impl From<HarnessConfig> for crate::wire::HarnessConfig {
    fn from(value: HarnessConfig) -> Self {
        Self {
            harness_id: value.harness_id,
            executable_path: value.executable_path,
            model: value.model,
            thinking_level: value.thinking_level,
            environment_variables: value
                .environment_variables
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
impl From<crate::wire::HarnessInfo> for HarnessInfo {
    fn from(value: crate::wire::HarnessInfo) -> Self {
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
impl From<HarnessInfo> for crate::wire::HarnessInfo {
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InventoryChanged {
    pub harnesses: Vec<HarnessInfo>,
}
impl From<crate::wire::InventoryChanged> for InventoryChanged {
    fn from(value: crate::wire::InventoryChanged) -> Self {
        Self {
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<InventoryChanged> for crate::wire::InventoryChanged {
    fn from(value: InventoryChanged) -> Self {
        Self {
            harnesses: value.harnesses.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub models: Vec<ModelCatalogItem>,
}
impl From<crate::wire::ModelCatalog> for ModelCatalog {
    fn from(value: crate::wire::ModelCatalog) -> Self {
        Self {
            models: value.models.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<ModelCatalog> for crate::wire::ModelCatalog {
    fn from(value: ModelCatalog) -> Self {
        Self {
            models: value.models.into_iter().map(Into::into).collect(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCatalogItem {
    pub model_id: String,
    pub display_name: String,
    pub description: String,
    pub available: bool,
    pub supported_thinking_levels: Vec<String>,
}
impl From<crate::wire::ModelCatalogItem> for ModelCatalogItem {
    fn from(value: crate::wire::ModelCatalogItem) -> Self {
        Self {
            model_id: value.model_id,
            display_name: value.display_name,
            description: value.description,
            available: value.available,
            supported_thinking_levels: value.supported_thinking_levels,
        }
    }
}
impl From<ModelCatalogItem> for crate::wire::ModelCatalogItem {
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub resource_id: i64,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}
impl From<crate::wire::ResourceRef> for ResourceRef {
    fn from(value: crate::wire::ResourceRef) -> Self {
        Self {
            resource_id: value.resource_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}
impl From<ResourceRef> for crate::wire::ResourceRef {
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
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceTransferResult {
    pub transfer_id: String,
    pub success: bool,
    pub error_code: String,
    pub error_message: String,
}
impl From<crate::wire::ResourceTransferResult> for ResourceTransferResult {
    fn from(value: crate::wire::ResourceTransferResult) -> Self {
        Self {
            transfer_id: value.transfer_id,
            success: value.success,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
impl From<ResourceTransferResult> for crate::wire::ResourceTransferResult {
    fn from(value: ResourceTransferResult) -> Self {
        Self {
            transfer_id: value.transfer_id,
            success: value.success,
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeHello {
    pub protocol_version: u32,
    pub runtime_instance_id: String,
    pub heartbeat_interval_ms: u64,
    pub max_message_bytes: u64,
}
impl From<crate::wire::RuntimeHello> for RuntimeHello {
    fn from(value: crate::wire::RuntimeHello) -> Self {
        Self {
            protocol_version: value.protocol_version,
            runtime_instance_id: value.runtime_instance_id,
            heartbeat_interval_ms: value.heartbeat_interval_ms,
            max_message_bytes: value.max_message_bytes,
        }
    }
}
impl From<RuntimeHello> for crate::wire::RuntimeHello {
    fn from(value: RuntimeHello) -> Self {
        Self {
            protocol_version: value.protocol_version,
            runtime_instance_id: value.runtime_instance_id,
            heartbeat_interval_ms: value.heartbeat_interval_ms,
            max_message_bytes: value.max_message_bytes,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionDirective {
    pub logical_session_id: String,
    pub action: i32,
    pub native_session_id: Option<String>,
    pub config_fingerprint: String,
}
impl From<crate::wire::SessionDirective> for SessionDirective {
    fn from(value: crate::wire::SessionDirective) -> Self {
        Self {
            logical_session_id: value.logical_session_id,
            action: value.action,
            native_session_id: value.native_session_id,
            config_fingerprint: value.config_fingerprint,
        }
    }
}
impl From<SessionDirective> for crate::wire::SessionDirective {
    fn from(value: SessionDirective) -> Self {
        Self {
            logical_session_id: value.logical_session_id,
            action: value.action,
            native_session_id: value.native_session_id,
            config_fingerprint: value.config_fingerprint,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shutdown {
    pub reason: String,
}
impl From<crate::wire::Shutdown> for Shutdown {
    fn from(value: crate::wire::Shutdown) -> Self {
        Self {
            reason: value.reason,
        }
    }
}
impl From<Shutdown> for crate::wire::Shutdown {
    fn from(value: Shutdown) -> Self {
        Self {
            reason: value.reason,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub execution: Option<ExecutionRef>,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments_json: String,
    pub execution_token: String,
}
impl From<crate::wire::ToolCall> for ToolCall {
    fn from(value: crate::wire::ToolCall) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            tool_call_id: value.tool_call_id,
            tool_name: value.tool_name,
            arguments_json: value.arguments_json,
            execution_token: value.execution_token,
        }
    }
}
impl From<ToolCall> for crate::wire::ToolCall {
    fn from(value: ToolCall) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            tool_call_id: value.tool_call_id,
            tool_name: value.tool_name,
            arguments_json: value.arguments_json,
            execution_token: value.execution_token,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema_json: String,
    pub output_schema_json: String,
}
impl From<crate::wire::ToolDefinition> for ToolDefinition {
    fn from(value: crate::wire::ToolDefinition) -> Self {
        Self {
            name: value.name,
            description: value.description,
            input_schema_json: value.input_schema_json,
            output_schema_json: value.output_schema_json,
        }
    }
}
impl From<ToolDefinition> for crate::wire::ToolDefinition {
    fn from(value: ToolDefinition) -> Self {
        Self {
            name: value.name,
            description: value.description,
            input_schema_json: value.input_schema_json,
            output_schema_json: value.output_schema_json,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}
impl From<crate::wire::ToolError> for ToolError {
    fn from(value: crate::wire::ToolError) -> Self {
        Self {
            code: value.code,
            message: value.message,
            retryable: value.retryable,
        }
    }
}
impl From<ToolError> for crate::wire::ToolError {
    fn from(value: ToolError) -> Self {
        Self {
            code: value.code,
            message: value.message,
            retryable: value.retryable,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub execution: Option<ExecutionRef>,
    pub tool_call_id: String,
    pub success: bool,
    pub content_markdown: String,
    pub structured_content_json: String,
    pub resources: Vec<ResourceRef>,
    pub error: Option<ToolError>,
}
impl From<crate::wire::ToolResult> for ToolResult {
    fn from(value: crate::wire::ToolResult) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            tool_call_id: value.tool_call_id,
            success: value.success,
            content_markdown: value.content_markdown,
            structured_content_json: value.structured_content_json,
            resources: value.resources.into_iter().map(Into::into).collect(),
            error: value.error.map(Into::into),
        }
    }
}
impl From<ToolResult> for crate::wire::ToolResult {
    fn from(value: ToolResult) -> Self {
        Self {
            execution: value.execution.map(Into::into),
            tool_call_id: value.tool_call_id,
            success: value.success,
            content_markdown: value.content_markdown,
            structured_content_json: value.structured_content_json,
            resources: value.resources.into_iter().map(Into::into).collect(),
            error: value.error.map(Into::into),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceRef {
    pub workspace_id: String,
}
impl From<crate::wire::WorkspaceRef> for WorkspaceRef {
    fn from(value: crate::wire::WorkspaceRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
        }
    }
}
impl From<WorkspaceRef> for crate::wire::WorkspaceRef {
    fn from(value: WorkspaceRef) -> Self {
        Self {
            workspace_id: value.workspace_id,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputerFrame {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub frame: ComputerPayload,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ComputerPayload {
    Hello(ComputerHello),
    Heartbeat(ComputerHeartbeat),
    AssignmentResult(AssignmentResult),
    ExecutionEvent(ExecutionEvent),
    DeliveryResult(DeliveryResult),
    ToolCall(ToolCall),
    ResourceTransferResult(ResourceTransferResult),
    InventoryChanged(InventoryChanged),
    DeleteAgentFilesResult(DeleteAgentFilesResult),
}
impl TryFrom<crate::wire::ComputerFrame> for ComputerFrame {
    type Error = super::CloseReason;
    fn try_from(value: crate::wire::ComputerFrame) -> Result<Self, Self::Error> {
        let frame = match value.frame {
            Some(crate::wire::computer_frame::Frame::Hello(value)) => {
                ComputerPayload::Hello(value.into())
            }
            Some(crate::wire::computer_frame::Frame::Heartbeat(value)) => {
                ComputerPayload::Heartbeat(value.into())
            }
            Some(crate::wire::computer_frame::Frame::AssignmentResult(value)) => {
                ComputerPayload::AssignmentResult(value.into())
            }
            Some(crate::wire::computer_frame::Frame::ExecutionEvent(value)) => {
                ComputerPayload::ExecutionEvent(value.into())
            }
            Some(crate::wire::computer_frame::Frame::DeliveryResult(value)) => {
                ComputerPayload::DeliveryResult(value.into())
            }
            Some(crate::wire::computer_frame::Frame::ToolCall(value)) => {
                ComputerPayload::ToolCall(value.into())
            }
            Some(crate::wire::computer_frame::Frame::ResourceTransferResult(value)) => {
                ComputerPayload::ResourceTransferResult(value.into())
            }
            Some(crate::wire::computer_frame::Frame::InventoryChanged(value)) => {
                ComputerPayload::InventoryChanged(value.into())
            }
            Some(crate::wire::computer_frame::Frame::DeleteAgentFilesResult(value)) => {
                ComputerPayload::DeleteAgentFilesResult(value.into())
            }
            None => return Err(super::CloseReason::UnknownFrameType),
        };
        let protocol_version = match &frame {
            ComputerPayload::Hello(hello) => hello.protocol_version,
            _ => crate::PROTOCOL_VERSION,
        };
        Ok(Self {
            protocol_version,
            frame,
        })
    }
}
impl From<ComputerFrame> for crate::wire::ComputerFrame {
    fn from(value: ComputerFrame) -> Self {
        Self {
            frame: Some(match value.frame {
                ComputerPayload::Hello(value) => {
                    crate::wire::computer_frame::Frame::Hello(value.into())
                }
                ComputerPayload::Heartbeat(value) => {
                    crate::wire::computer_frame::Frame::Heartbeat(value.into())
                }
                ComputerPayload::AssignmentResult(value) => {
                    crate::wire::computer_frame::Frame::AssignmentResult(value.into())
                }
                ComputerPayload::ExecutionEvent(value) => {
                    crate::wire::computer_frame::Frame::ExecutionEvent(value.into())
                }
                ComputerPayload::DeliveryResult(value) => {
                    crate::wire::computer_frame::Frame::DeliveryResult(value.into())
                }
                ComputerPayload::ToolCall(value) => {
                    crate::wire::computer_frame::Frame::ToolCall(value.into())
                }
                ComputerPayload::ResourceTransferResult(value) => {
                    crate::wire::computer_frame::Frame::ResourceTransferResult(value.into())
                }
                ComputerPayload::InventoryChanged(value) => {
                    crate::wire::computer_frame::Frame::InventoryChanged(value.into())
                }
                ComputerPayload::DeleteAgentFilesResult(value) => {
                    crate::wire::computer_frame::Frame::DeleteAgentFilesResult(value.into())
                }
            }),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeFrame {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub frame: RuntimePayload,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RuntimePayload {
    Heartbeat,
    Hello(RuntimeHello),
    AssignExecution(AssignExecution),
    DeliverSteer(DeliverSteer),
    CancelExecution(CancelExecution),
    ToolResult(ToolResult),
    ExecutionEventAck(ExecutionEventAck),
    ExecutionCommitted(ExecutionCommitted),
    Shutdown(Shutdown),
    DeleteAgentFiles(DeleteAgentFiles),
}
impl TryFrom<crate::wire::RuntimeFrame> for RuntimeFrame {
    type Error = super::CloseReason;
    fn try_from(value: crate::wire::RuntimeFrame) -> Result<Self, Self::Error> {
        let frame = match value.frame {
            Some(crate::wire::runtime_frame::Frame::Hello(value)) => {
                RuntimePayload::Hello(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::AssignExecution(value)) => {
                RuntimePayload::AssignExecution(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::DeliverSteer(value)) => {
                RuntimePayload::DeliverSteer(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::CancelExecution(value)) => {
                RuntimePayload::CancelExecution(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::ToolResult(value)) => {
                RuntimePayload::ToolResult(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::ExecutionEventAck(value)) => {
                RuntimePayload::ExecutionEventAck(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::ExecutionCommitted(value)) => {
                RuntimePayload::ExecutionCommitted(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::Shutdown(value)) => {
                RuntimePayload::Shutdown(value.into())
            }
            Some(crate::wire::runtime_frame::Frame::DeleteAgentFiles(value)) => {
                RuntimePayload::DeleteAgentFiles(value.into())
            }
            None => return Err(super::CloseReason::UnknownFrameType),
        };
        let protocol_version = match &frame {
            RuntimePayload::Hello(hello) => hello.protocol_version,
            _ => crate::PROTOCOL_VERSION,
        };
        Ok(Self {
            protocol_version,
            frame,
        })
    }
}
impl From<RuntimeFrame> for crate::wire::RuntimeFrame {
    fn from(value: RuntimeFrame) -> Self {
        Self {
            frame: Some(match value.frame {
                RuntimePayload::Heartbeat => return Self { frame: None },
                RuntimePayload::Hello(value) => {
                    crate::wire::runtime_frame::Frame::Hello(value.into())
                }
                RuntimePayload::AssignExecution(value) => {
                    crate::wire::runtime_frame::Frame::AssignExecution(value.into())
                }
                RuntimePayload::DeliverSteer(value) => {
                    crate::wire::runtime_frame::Frame::DeliverSteer(value.into())
                }
                RuntimePayload::CancelExecution(value) => {
                    crate::wire::runtime_frame::Frame::CancelExecution(value.into())
                }
                RuntimePayload::ToolResult(value) => {
                    crate::wire::runtime_frame::Frame::ToolResult(value.into())
                }
                RuntimePayload::ExecutionEventAck(value) => {
                    crate::wire::runtime_frame::Frame::ExecutionEventAck(value.into())
                }
                RuntimePayload::ExecutionCommitted(value) => {
                    crate::wire::runtime_frame::Frame::ExecutionCommitted(value.into())
                }
                RuntimePayload::Shutdown(value) => {
                    crate::wire::runtime_frame::Frame::Shutdown(value.into())
                }
                RuntimePayload::DeleteAgentFiles(value) => {
                    crate::wire::runtime_frame::Frame::DeleteAgentFiles(value.into())
                }
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairComputerRequest {
    pub code: String,
    pub display_name: String,
    pub os: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairComputerResponse {
    pub computer_id: String,
    pub device_credential: String,
}
