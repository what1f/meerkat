#[path = "../../../crates/harness/tests/support/plugin_live.rs"]
mod contract;

#[tokio::test]
#[ignore = "requires explicit live opt-in; runs only Codex with gpt-6-luna medium"]
async fn plugin_live() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    contract::run(
        &meerkat_harness_codex::CodexHarness::new(),
        std::env::var_os("MEERKAT_CODEX_PATH")
            .map(Into::into)
            .unwrap_or_else(|| "codex".into()),
        "gpt-6-luna",
        "medium",
    )
    .await
}
