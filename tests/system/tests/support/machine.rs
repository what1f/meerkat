#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
async fn run() -> lankide_system_e2e::Result<()> {
    let mut args = std::env::args().skip(1);
    lankide_system_e2e::run_scripted_machine(
        args.next().unwrap().into(),
        args.next().unwrap(),
        args.next().unwrap().into(),
        args.next().unwrap().into(),
        args.next().unwrap().into(),
        serde_json::from_str(&args.next().unwrap())?,
    )
    .await
}
