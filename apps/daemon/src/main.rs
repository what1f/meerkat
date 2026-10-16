#![forbid(unsafe_code)]
use lankided::{AppError, RuntimePaths, production_factories, run_daemon, wait_for_shutdown};

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        if log::log_enabled!(log::Level::Error) {
            log::error!("runtime-daemon could not start: {error}");
        } else {
            eprintln!("runtime-daemon could not start: {error}");
        }
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), AppError> {
    let mut arguments = std::env::args_os().skip(1);
    let mut home = None;
    while let Some(argument) = arguments.next() {
        if argument == "--version" {
            println!("lankided {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        } else if argument == "--help" {
            println!("Usage: lankided [--home PATH]");
            return Ok(());
        } else if argument == "--home" {
            home = Some(std::path::PathBuf::from(
                arguments
                    .next()
                    .ok_or(AppError::Invariant("--home requires a path"))?,
            ));
        } else {
            return Err(AppError::Invariant("unknown option"));
        }
    }
    let paths = match home {
        Some(home) => RuntimePaths::for_home(home)?,
        None => RuntimePaths::local_default()?,
    };
    lankide_daemon_runtime::init_logging(&paths.home, "lankided")?;
    log::info!("started version={}", env!("CARGO_PKG_VERSION"));
    run_daemon(paths, production_factories()?, wait_for_shutdown()).await
}
