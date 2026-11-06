#[path = "../../../crates/harness/tests/support/native_failures.rs"]
mod native_failures;

#[tokio::test]
async fn native_agent_failures_keep_their_cause_and_classification()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    native_failures::run(&meerkat_harness_codex::CodexHarness::new(), "codex").await
}
