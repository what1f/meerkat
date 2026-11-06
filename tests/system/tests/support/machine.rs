#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
async fn run() -> meerkat_system_e2e::Result<()> {
    let mut args = std::env::args().skip(1);
    let first = args.next().ok_or("machine home or pair")?;
    if first == "pair" {
        let home: std::path::PathBuf = args.next().ok_or("home")?.into();
        let url = args.next().ok_or("URL")?;
        let code = args.next().ok_or("pairing code")?;
        let pin = args.next().ok_or("certificate pin")?;
        let executable = args.next().ok_or("Harness executable")?.into();
        let capabilities = serde_json::from_str(&args.next().ok_or("capabilities")?)?;
        meerkat_machine::pair_with_pin(
            &meerkat_machine::MachineDaemonPaths::for_home(&home)?,
            &url,
            &code,
            (!pin.is_empty()).then_some(pin.as_str()),
        )
        .await?;
        return meerkat_system_e2e::run_scripted_machine(
            home,
            url,
            "".into(),
            "".into(),
            executable,
            capabilities,
        )
        .await;
    }
    meerkat_system_e2e::run_scripted_machine(
        first.into(),
        args.next().unwrap(),
        args.next().unwrap().into(),
        args.next().unwrap().into(),
        args.next().unwrap().into(),
        serde_json::from_str(&args.next().unwrap())?,
    )
    .await
}
