#[path = "../../../crates/harness/tests/support/plugin_live.rs"]
mod contract;

#[tokio::test]
#[ignore = "requires an isolated HOME/environment and a DeepSeek-only Claude CLI wrapper"]
async fn dcc_isolated() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    assert_eq!(
        std::env::var("LANKIDE_CLAUDE_DCC_ISOLATED").as_deref(),
        Ok("1")
    );
    contract::run(
        &lankide_harness_claude_code::ClaudeCodeHarness::new(),
        std::env::var_os("LANKIDE_CLAUDE_CODE_PATH")
            .expect("an isolated DeepSeek-only wrapper is required")
            .into(),
        "deepseek-flash",
        "off",
    )
    .await
}
