#[path = "../../../crates/harness/tests/support/native_defaults.rs"]
mod support;

#[tokio::test]
async fn s7_2_agent_starts_with_full_access_and_native_home_skills()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    support::agent_native_defaults(
        &lankide_harness_pi::PiHarness::new(),
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pi_native_peer.py"
        )),
    )
    .await
}
