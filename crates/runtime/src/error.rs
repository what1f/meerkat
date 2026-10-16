use super::*;

#[derive(Debug)]
pub enum RuntimeError {
    Storage(StorageError),
    Provider {
        workspace_id: WorkspaceId,
        source: ProviderError,
    },
    ProviderCall(ProviderError),
    DuplicateWorkspace(WorkspaceId),
    DuplicateAgent {
        workspace_id: WorkspaceId,
        agent_id: AgentId,
    },
    UnknownWorkspace(WorkspaceId),
    WorkspacePaused(WorkspaceId),
    ProviderNotStarted(WorkspaceId),
    UnknownAgent {
        workspace_id: WorkspaceId,
        agent_id: AgentId,
    },
    UnknownChannel {
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
    },
    InvalidCore(String),
    UnknownSend(String),
    RuntimeLockPoisoned,
    DuplicateProviderType(ProviderId),
    UnknownProviderType(ProviderId),
    StorageStillShared,
    ApplicationStopped,
    RuntimeStillShared,
    Application(String),
    ActionRequired(String),
    InvalidExecutionToken,
    MachineNotRegistered,
    MachineRevoked,
    MachineHasAssignedAgents,
    LocalMachineCannotBeRemoved,
    MachineNameInvalid,
    HarnessUnavailableOnMachine,
    ProviderMismatch {
        workspace_id: WorkspaceId,
        expected: ProviderId,
        actual: ProviderId,
    },
    InvalidProviderSchema(serde_json::Error),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(formatter, "runtime_storage: {error}"),
            Self::Provider {
                workspace_id,
                source,
            } => {
                write!(formatter, "runtime_provider[{workspace_id}]: {source}")
            }
            Self::ProviderCall(error) => write!(formatter, "runtime_provider_call: {error}"),
            Self::DuplicateWorkspace(id) => write!(formatter, "runtime_duplicate_workspace: {id}"),
            Self::DuplicateAgent {
                workspace_id,
                agent_id,
            } => write!(
                formatter,
                "runtime_duplicate_agent[{workspace_id}]: {agent_id}"
            ),
            Self::UnknownWorkspace(id) => write!(formatter, "runtime_unknown_workspace: {id}"),
            Self::WorkspacePaused(id) => write!(formatter, "runtime_workspace_paused: {id}"),
            Self::ProviderNotStarted(id) => {
                write!(formatter, "runtime_provider_not_started: {id}")
            }
            Self::UnknownAgent {
                workspace_id,
                agent_id,
            } => write!(
                formatter,
                "runtime_unknown_agent[{workspace_id}]: {agent_id}"
            ),
            Self::UnknownChannel {
                workspace_id,
                channel_id,
            } => write!(
                formatter,
                "runtime_unknown_channel[{workspace_id}]: {channel_id}"
            ),
            Self::InvalidCore(error) => write!(formatter, "runtime_invalid_core: {error}"),
            Self::UnknownSend(key) => write!(formatter, "runtime_unknown_send: {key}"),
            Self::RuntimeLockPoisoned => formatter.write_str("runtime_lock_poisoned"),
            Self::DuplicateProviderType(id) => {
                write!(formatter, "runtime_duplicate_provider: {id}")
            }
            Self::UnknownProviderType(id) => write!(formatter, "runtime_unknown_provider: {id}"),
            Self::StorageStillShared => formatter.write_str("runtime_storage_still_shared"),
            Self::ApplicationStopped => formatter.write_str("runtime_application_stopped"),
            Self::RuntimeStillShared => formatter.write_str("runtime_still_shared"),
            Self::Application(error) => write!(formatter, "runtime_application: {error}"),
            Self::ActionRequired(message) => formatter.write_str(message),
            Self::MachineNotRegistered => formatter.write_str("machine_not_registered"),
            Self::MachineHasAssignedAgents => formatter.write_str("machine_has_assigned_agents"),
            Self::LocalMachineCannotBeRemoved => {
                formatter.write_str("local_machine_cannot_be_removed")
            }
            Self::MachineNameInvalid => formatter.write_str("machine_name_invalid"),
            Self::MachineRevoked => formatter.write_str("machine_revoked"),
            Self::HarnessUnavailableOnMachine => {
                formatter.write_str("harness_unavailable_on_machine")
            }
            Self::InvalidExecutionToken => formatter.write_str("runtime_invalid_execution_token"),
            Self::ProviderMismatch {
                workspace_id,
                expected,
                actual,
            } => write!(
                formatter,
                "runtime_provider_mismatch[{workspace_id}]: expected {expected}, got {actual}"
            ),
            Self::InvalidProviderSchema(error) => {
                write!(formatter, "runtime_invalid_provider_schema: {error}")
            }
        }
    }
}
impl std::error::Error for RuntimeError {}
impl From<StorageError> for RuntimeError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}
impl From<ProviderError> for RuntimeError {
    fn from(value: ProviderError) -> Self {
        Self::ProviderCall(value)
    }
}
