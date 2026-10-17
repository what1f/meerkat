#![forbid(unsafe_code)]

//! Adapter-neutral contracts for running Agent harnesses.
//!
//! Concrete native CLI adapters implement [`HarnessFactory`].

mod catalog;
mod config;
pub use catalog::NativeCatalogProcess;
mod process_group;
mod types;
pub use config::{prepare_config_overlay, prepare_native_skills, user_config_directory};
pub use process_group::kill_process_group;

pub use types::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    FailureCategory, HarnessAvailability, HarnessCapability, HarnessConfig, HarnessError,
    HarnessErrorCode, HarnessFactory, HarnessFailure, HarnessFuture, HarnessInfo, HarnessSession,
    ModelRef, NativeModel, PermissionPolicy, ProbeResult, SessionEntry, SessionHealth,
    SessionRequest, SteerReceipt, ToolDefinition, ToolResultError, ToolResultReceipt,
    ToolResultRequest, ToolResultResource,
};
