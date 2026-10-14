#[path = "../../../crates/harness/tests/support/native_defaults.rs"]
mod support;

#[tokio::test]
async fn s7_2_agent_starts_with_full_access_and_native_home_skills()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    support::agent_native_defaults(
        &lankide_harness_claude_code::ClaudeCodeHarness::new(),
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/harness/tests/support/native_defaults_peer.py"
        )),
    )
    .await
}
