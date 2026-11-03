//! Run the CLI business contract through both real daemons and the attempt proxy.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let status = std::process::Command::new("cargo")
        .args([
            "test",
            "-p",
            "meerkat-system-e2e",
            "--test",
            "agent_proxy_e2e",
            "--",
            "--nocapture",
        ])
        .status()?;
    if !status.success() {
        return Err("CLI proxy scenario failed".into());
    }
    Ok(())
}
