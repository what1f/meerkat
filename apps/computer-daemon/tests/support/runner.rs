#![forbid(unsafe_code)]

mod composition;
mod e2e;
mod scripted_harness;

use lankide_computer_daemon::{AppError, DaemonConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), AppError> {
    let raw = std::env::args().skip(1).collect::<Vec<_>>();
    if raw
        .first()
        .is_some_and(|argument| argument == "--scripted-harness")
    {
        scripted_harness::run();
        return Ok(());
    }
    if raw.first().is_some_and(|argument| argument == "--version") {
        println!("lankide-scripted-cli 1.0");
        return Ok(());
    }
    if raw == ["app-server", "--help"] {
        println!("app-server --listen stdio://");
        return Ok(());
    }
    if raw.first().is_some_and(|argument| argument == "--help") {
        println!(
            "--output-format --input-format --session-id --resume --mcp-config --system-prompt"
        );
        return Ok(());
    }
    let mut arguments = std::env::args_os().skip(1);
    let mut home = None;
    let mut e2e = false;
    let mut scripted_composition = false;
    let mut pid_journal = None;
    let mut instance_id = None;
    while let Some(argument) = arguments.next() {
        if argument == "--e2e" {
            e2e = true;
        } else if argument == "--home" {
            home =
                Some(PathBuf::from(arguments.next().ok_or(
                    AppError::InvalidArgument("--home requires a path"),
                )?));
        } else if argument == "--scripted-composition" {
            scripted_composition = true;
        } else if argument == "--pid-journal" {
            pid_journal =
                Some(PathBuf::from(arguments.next().ok_or(
                    AppError::InvalidArgument("--pid-journal requires a path"),
                )?));
        } else if argument == "--instance-id" {
            instance_id = Some(
                arguments
                    .next()
                    .ok_or(AppError::InvalidArgument("--instance-id requires a value"))?
                    .to_string_lossy()
                    .into_owned(),
            );
        } else {
            return Err(AppError::InvalidArgument("unknown option"));
        }
    }
    if e2e {
        let artifact = e2e::run().await?;
        println!("computer-daemon e2e artifact: {}", artifact.display());
        return Ok(());
    }
    let mut config = match home {
        Some(home) => DaemonConfig::for_home(home)?,
        None => DaemonConfig::local_default()?,
    };
    if let Some(instance_id) = instance_id {
        config.instance_id = instance_id;
    }
    if scripted_composition {
        return composition::run_scripted(
            config,
            pid_journal.ok_or(AppError::InvalidArgument(
                "--scripted-composition requires --pid-journal",
            ))?,
        )
        .await;
    }
    lankide_computer_daemon::run(config).await
}
