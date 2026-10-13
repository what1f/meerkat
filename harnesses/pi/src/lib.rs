#![forbid(unsafe_code)]

//! Pi Harness adapter over Pi's native RPC/JSONL protocol.

mod adapter;

pub use adapter::{HARNESS_ID, PiHarness};
