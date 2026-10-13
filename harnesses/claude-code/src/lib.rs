#![forbid(unsafe_code)]

//! Claude Code Harness adapter.
//!
//! `dcc` is a configurable CLI path for this adapter, never a separate
//! Harness identity.

mod adapter;
mod mcp;

pub use adapter::ClaudeCodeHarness;

pub const HARNESS_ID: &str = "claude_code";
