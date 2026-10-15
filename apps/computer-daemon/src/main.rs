#![forbid(unsafe_code)]
use lankide_computer_daemon::{AppError, DaemonConfig};
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
        match &error {
            AppError::Io(error) => eprintln!("{error}"),
            AppError::InvalidArgument(message) => eprintln!("{message}"),
            _ => eprintln!("Could not start this computer: {error}"),
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
        } else if argument == "--version" {
            println!("lankide-computer-daemon {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        } else if argument == "--help" {
            println!(
                "Usage: lankide-computer-daemon [pair URL CODE [--cert-sha256 HEX]] [--home PATH] [--instance-id ID] [--runtime-url URL] [--link-secret-file PATH]"
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
    let mut config = match home {
        Some(home) => DaemonConfig::for_home(home)?,
        None => DaemonConfig::local_default()?,
    };
    if let Some((url, code)) = pairing {
        lankide_computer_daemon::pair_with_pin(&config.paths, &url, &code, cert_sha256.as_deref())
            .await?;
    }
    config.runtime_url = runtime_url;
    config.link_secret_file = link_secret_file;
    if let Some(path) = user_executable_path() {
        config.set_executable_path(path.to_string_lossy().into_owned());
    }
    if let Some(instance_id) = instance_id {
        config.instance_id = instance_id;
    }
    lankide_computer_daemon::run(config).await
}

fn user_executable_path() -> Option<OsString> {
    merge_paths(
        login_shell_path().as_deref(),
        env::var_os("PATH").as_deref(),
    )
}

fn login_shell_path() -> Option<OsString> {
    const START: &str = "__LANKIDE_PATH_START__";
    const END: &str = "__LANKIDE_PATH_END__";
    const COMMAND_LINE: &str =
        "printf '%s\\n' __LANKIDE_PATH_START__; printenv PATH; printf '%s\\n' __LANKIDE_PATH_END__";

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
