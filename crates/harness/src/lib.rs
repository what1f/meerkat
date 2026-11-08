#![forbid(unsafe_code)]

//! Adapter-neutral contracts for running Agent harnesses.
//!
//! Concrete native CLI adapters implement [`HarnessFactory`].

mod config;
mod plugin;
mod plugin_mcp;
mod process_group;
mod types;
pub use config::prepare_native_skills;
pub use plugin::JsHarnessFactory;
pub use process_group::kill_process_group;

pub use types::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    FailureCategory, HarnessAccount, HarnessAccountMethod, HarnessAvailability, HarnessCapability,
    HarnessConfig, HarnessCredential, HarnessDefaults, HarnessError, HarnessErrorCode,
    HarnessFactory, HarnessFailure, HarnessFuture, HarnessInfo, HarnessSession, HarnessTestPlan,
    ModelRef, NativeModel, PermissionPolicy, ProbeResult, SessionEntry, SessionHealth,
    SessionRequest, SteerReceipt, ToolDefinition, ToolResultError, ToolResultReceipt,
    ToolResultRequest, ToolResultResource,
};
