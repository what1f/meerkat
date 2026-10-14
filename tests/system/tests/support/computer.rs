#[tokio::main]
async fn main() -> lankide_system_e2e::Result<()> {
    let mut args = std::env::args().skip(1);
    lankide_system_e2e::run_scripted_computer(
        args.next().unwrap().into(),
        args.next().unwrap(),
        args.next().unwrap().into(),
        args.next().unwrap().into(),
        args.next().unwrap().into(),
        serde_json::from_str(&args.next().unwrap())?,
    )
    .await
}
