#![forbid(unsafe_code)]

//! Local execution host for Runtime-assigned Agent work.

mod capability_cli;
mod host;
mod resources;
mod state;

pub use host::{
    ExecutionLiveness, Machine, MachineConfig, MachineError, MachineHandle, MachineStatus,
    OutputRecordPolicy, ReconnectPolicy, RegisteredHarness,
};
pub use resources::{agent_directory, resolve_content_file, workspace_directory};

/// Stable harness IDs exposed by a local Machine.
pub const STANDARD_HARNESS_IDS: [&str; 4] = ["builtin", "pi", "codex", "claude_code"];
