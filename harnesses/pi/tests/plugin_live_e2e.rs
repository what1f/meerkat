#[path = "../../../crates/harness/tests/support/plugin_live.rs"]
mod contract;

#[tokio::test]
#[ignore = "requires explicit live opt-in; runs only Pi with its configured default model"]
async fn plugin_live() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let model = std::env::var("MEERKAT_PI_MODEL").unwrap_or_else(|_| "deepseek-flash".into());
    contract::run(
        &meerkat_harness_pi::PiHarness::new(),
        std::env::var_os("MEERKAT_PI_PATH")
            .map(Into::into)
            .unwrap_or_else(|| "pi".into()),
        &model,
        "off",
    )
    .await
}
