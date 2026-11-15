//! Keeps a paired remote Machine running for the signed-in user: a LaunchAgent
//! on macOS, a systemd user service on Linux, and a sign-in entry on Windows.

use meerkat_machine::{AppError, MachineDaemonPaths, connected_marker, machine_name};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
mod launchd;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use launchd as service;
#[cfg(target_os = "linux")]
use systemd as service;
#[cfg(windows)]
use windows as service;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);

/// Registers the Machine to start at sign-in, starts it, and waits for it to
/// connect. Returns `false` when this system has no per-user service manager,
/// so the caller keeps the Machine running in the foreground instead.
pub async fn install(paths: &MachineDaemonPaths) -> Result<bool, AppError> {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    {
        let executable = executable()?;
        let marker = connected_marker(paths);
        let _ = std::fs::remove_file(&marker);
        if !service::start(&executable, &paths.home)? {
            return Ok(false);
        }
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        while Instant::now() < deadline {
            if marker.exists() {
                println!("Connected as {}", machine_name());
                println!("Runs in the background and starts when you sign in.");
                service::after_connected();
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(AppError::InvalidArgument(service::not_connected(
            &paths.home,
        )))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = paths;
        Ok(false)
    }
}

/// Stops the background Machine serving this home. Returns `false` when no
/// background service runs it, for example a Machine in another terminal.
pub fn stop(paths: &MachineDaemonPaths) -> bool {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    return service::stop(paths);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = paths;
        false
    }
}

/// Whether a registered service definition starts the Machine for this home.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn serves(definition: &Path, home: &Path) -> bool {
    std::fs::read_to_string(definition).is_ok_and(|text| text.contains(&*home.to_string_lossy()))
}

/// Starts the earlier background Machine again after a failed re-pairing.
pub fn resume(paths: &MachineDaemonPaths) {
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    if let Ok(executable) = executable() {
        service::resume(&executable, paths);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let _ = paths;
}

#[allow(dead_code)]
const NOT_CONNECTED: &str = "Paired, but not connected yet. It keeps retrying in the background; see the meerkat-machine log under .meerkat/logs.";
#[allow(dead_code)]
const NOT_STARTED: &str =
    "Paired, but could not start in the background. Run meerkat-machine to stay connected.";

fn executable() -> Result<PathBuf, AppError> {
    let executable = std::env::current_exe()?;
    // Windows canonical paths carry a verbatim prefix that sign-in entries do not need.
    #[cfg(unix)]
    let executable = executable.canonicalize()?;
    Ok(executable)
}

#[allow(dead_code)]
fn log_mentions(home: &Path, needle: &str) -> bool {
    std::fs::read_to_string(home.join("logs/meerkat-machine.log"))
        .is_ok_and(|log| log.lines().rev().take(5).any(|line| line.contains(needle)))
}
