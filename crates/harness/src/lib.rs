#![forbid(unsafe_code)]

//! Adapter-neutral contracts for running Agent harnesses.
//!
//! Concrete native CLI adapters implement [`HarnessFactory`].

mod catalog;
mod config;
pub use catalog::NativeCatalogProcess;
mod types;
pub use config::{prepare_config_overlay, user_config_directory};

pub use types::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    HarnessAvailability, HarnessCapability, HarnessConfig, HarnessError, HarnessErrorCode,
    HarnessFactory, HarnessFuture, HarnessInfo, HarnessSession, ModelRef, NativeModel, ProbeResult,
    SessionEntry, SessionHealth, SessionRequest, SteerReceipt, ToolDefinition, ToolResultError,
    ToolResultReceipt, ToolResultRequest, ToolResultResource,
};
