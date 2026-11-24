#![forbid(unsafe_code)]
mod background;

use meerkat_machine::{AppError, DaemonConfig};
use std::{
    env,
    ffi::{OsStr, OsString},
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        if is_revoked(&error) {
            log::error!("This machine was removed from the workspace.");
            // A clean exit keeps launchd from restarting a removed Machine.
            std::process::exit(0);
        }
        let message = match &error {
            AppError::Io(error) => error.to_string(),
            AppError::InvalidArgument(message) => message.to_string(),
            AppError::AlreadyRunning => "Meerkat Machine is already running here.".into(),
            _ => format!("Could not start this machine: {error}"),
        };
        if log::log_enabled!(log::Level::Error) {
            log::error!("{message}");
        } else {
            eprintln!("{message}");
        }
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), AppError> {
    let mut arguments = std::env::args_os().skip(1);
    let mut home = None;
    let mut runtime_url = None;
    let mut link_secret_file = None;
    let mut instance_id = None;
    let mut pairing = None;
    let mut cert_sha256 = None;
    let mut foreground = false;
    let mut shutdown_on_stdin_eof = false;
    while let Some(argument) = arguments.next() {
        if argument == "pair" {
            let url = arguments
                .next()
                .ok_or(AppError::InvalidArgument("pair requires a URL"))?
                .to_string_lossy()
                .into_owned();
            let code = arguments
                .next()
                .ok_or(AppError::InvalidArgument("pair requires a code"))?
                .to_string_lossy()
                .into_owned();
            pairing = Some((url, code));
        } else if argument == "--cert-sha256" {
            cert_sha256 = Some(
                arguments
                    .next()
                    .ok_or(AppError::InvalidArgument("--cert-sha256 requires a value"))?
                    .to_string_lossy()
                    .into_owned(),
            );
        } else if argument == "--foreground" {
            foreground = true;
        } else if argument == "--shutdown-on-stdin-eof" {
            shutdown_on_stdin_eof = true;
        } else if argument == "--version" {
            println!("meerkat-machine {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        } else if argument == "--help" {
            println!(
                "Usage: meerkat-machine [pair URL CODE [--cert-sha256 HEX] [--foreground]] [--home PATH] [--instance-id ID] [--runtime-url URL] [--link-secret-file PATH]"
            );
            return Ok(());
        } else if argument == "--home" {
            home =
                Some(PathBuf::from(arguments.next().ok_or(
                    AppError::InvalidArgument("--home requires a path"),
                )?));
        } else if argument == "--runtime-url" {
            runtime_url = Some(
                arguments
                    .next()
                    .ok_or(AppError::InvalidArgument("--runtime-url requires a URL"))?
                    .to_string_lossy()
                    .into_owned(),
            );
        } else if argument == "--link-secret-file" {
            link_secret_file = Some(PathBuf::from(arguments.next().ok_or(
                AppError::InvalidArgument("--link-secret-file requires a path"),
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
    let paths = match home {
        Some(home) => meerkat_machine::MachineDaemonPaths::for_home(home)?,
        None => meerkat_machine::MachineDaemonPaths::local_default()?,
    };
    meerkat_daemon_runtime::init_logging(&paths.home, "meerkat-machine")?;
    if pairing.is_none() {
        log::info!("started version={}", env!("CARGO_PKG_VERSION"));
    }
    let mut config = DaemonConfig::for_home(paths.home)?;
    if let Some((url, code)) = pairing {
        pair(&config.paths, &url, &code, cert_sha256.as_deref()).await?;
        if !foreground && background::install(&config.paths).await? {
            return Ok(());
        }
    }
    config.runtime_url = runtime_url;
    config.link_secret_file = link_secret_file;
    config.shutdown_on_stdin_eof = shutdown_on_stdin_eof;
    if let Some(path) = user_executable_path() {
        config.set_executable_path(path.to_string_lossy().into_owned());
    }
    if let Some(instance_id) = instance_id {
        config.instance_id = instance_id;
    }
    meerkat_machine::run(config).await
}

/// Pairs with a new code. An earlier pairing, for example of a computer that
/// was removed from the workspace, is replaced: a background Machine still
/// running it is stopped first and started again if the new pairing fails.
async fn pair(
    paths: &meerkat_machine::MachineDaemonPaths,
    url: &str,
    code: &str,
    pin: Option<&str>,
) -> Result<(), AppError> {
    let existing = meerkat_machine::read_credentials(&paths.home)?;
    if existing
        .as_ref()
        .is_some_and(|existing| existing.local_discovery)
    {
        return Err(AppError::InvalidArgument(meerkat_machine::ALREADY_LOCAL));
    }
    match meerkat_machine::pair_with_pin(paths, url, code, pin).await {
        Err(AppError::AlreadyRunning) => {}
        result => return result.map(drop),
    }
    if !background::stop(paths) {
        return Err(AppError::AlreadyRunning);
    }
    // The stopped Machine releases its instance lock shortly after it exits.
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        match meerkat_machine::pair_with_pin(paths, url, code, pin).await {
            Err(AppError::AlreadyRunning) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            result => break result.map(drop),
        }
    };
    if result.is_err() {
        background::resume(paths);
    }
    result
}

fn is_revoked(error: &AppError) -> bool {
    use meerkat_protocol::link::{CloseReason, LinkError};
    matches!(
        error,
        AppError::Machine(meerkat_machine_core::MachineError::Link(LinkError::Closed(
            CloseReason::MachineRevoked | CloseReason::MachineUnknown
        )))
    )
}

fn user_executable_path() -> Option<OsString> {
    merge_paths(
        login_shell_path().as_deref(),
        env::var_os("PATH").as_deref(),
    )
}

fn login_shell_path() -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    const START: &str = "__MEERKAT_PATH_START__";
    const END: &str = "__MEERKAT_PATH_END__";
    const COMMAND_LINE: &str =
        "printf '%s\\n' __MEERKAT_PATH_START__; printenv PATH; printf '%s\\n' __MEERKAT_PATH_END__";

    let shell = env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            OsString::from(if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/sh"
            })
        });
    let mut child = Command::new(shell)
        .args(["-ilc", COMMAND_LINE])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
    };
    if !status.success() {
        let _ = reader.join();
        return None;
    }
    let output = reader.join().ok()?.ok()?;
    let output = String::from_utf8(output).ok()?;
    let mut inside = false;
    for line in output.lines() {
        if line == START {
            inside = true;
        } else if line == END {
            break;
        } else if inside && !line.is_empty() {
            return Some(OsString::from(line));
        }
    }
    None
}

fn merge_paths(preferred: Option<&OsStr>, inherited: Option<&OsStr>) -> Option<OsString> {
    let mut paths = Vec::<PathBuf>::new();
    for value in [preferred, inherited].into_iter().flatten() {
        for path in env::split_paths(value) {
            if !path.as_os_str().is_empty() && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    (!paths.is_empty())
        .then(|| env::join_paths(paths).ok())
        .flatten()
}
