#![forbid(unsafe_code)]

//! Embedded JavaScript adapter, hosted by Machine through the shared contract.
pub const HARNESS_ID: &str = "claude_code";
lankide_harness::harness_plugin!(
    ClaudeCodeHarness,
    "claude_code",
    "Claude Code",
    include_str!("../plugin.js")
);
