#![forbid(unsafe_code)]

//! Embedded JavaScript adapter, hosted by Machine through the shared contract.
pub const HARNESS_ID: &str = "codex";
meerkat_harness::harness_plugin!(CodexHarness, "codex", "Codex", include_str!("../plugin.js"));
