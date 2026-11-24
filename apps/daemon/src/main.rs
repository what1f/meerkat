#![forbid(unsafe_code)]
use meerkatd::{
    AppError, RuntimePaths, ShutdownReason, production_factories, run_daemon, wait_for_shutdown,
};
use std::io::Read as _;

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
    let mut shutdown_on_stdin_eof = false;
    while let Some(argument) = arguments.next() {
        if argument == "--version" {
            println!("meerkatd {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        } else if argument == "--help" {
            println!("Usage: meerkatd [--home PATH] [--shutdown-on-stdin-eof]");
            return Ok(());
        } else if argument == "--home" {
            home = Some(std::path::PathBuf::from(
                arguments
                    .next()
                    .ok_or(AppError::Invariant("--home requires a path"))?,
            ));
        } else if argument == "--shutdown-on-stdin-eof" {
            shutdown_on_stdin_eof = true;
        } else {
            return Err(AppError::Invariant("unknown option"));
        }
    }
    let paths = match home {
        Some(home) => RuntimePaths::for_home(home)?,
        None => RuntimePaths::local_default()?,
    };
    meerkat_daemon_runtime::init_logging(&paths.home, "meerkatd")?;
    log::info!("started version={}", env!("CARGO_PKG_VERSION"));
    run_daemon(
        paths,
        production_factories()?,
        daemon_shutdown(shutdown_on_stdin_eof),
    )
    .await
}

async fn daemon_shutdown(shutdown_on_stdin_eof: bool) -> Result<ShutdownReason, AppError> {
    if !shutdown_on_stdin_eof {
        return wait_for_shutdown().await;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0u8; 64];
        while matches!(stdin.read(&mut buffer), Ok(n) if n > 0) {}
        let _ = tx.send(());
    });
    tokio::select! {
        result = wait_for_shutdown() => result,
        _ = rx => Ok(ShutdownReason::Sigterm),
    }
}
