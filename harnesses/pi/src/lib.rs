#![forbid(unsafe_code)]

//! Embedded JavaScript adapter, hosted by Machine through the shared contract.
pub const HARNESS_ID: &str = "pi";
meerkat_harness::harness_plugin!(PiHarness, "pi", "Pi", include_str!("../plugin.js"));
