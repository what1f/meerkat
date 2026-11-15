//! Sign-in entry under HKCU\...\Run; needs no administrator rights.

use meerkat_machine::{AppError, MachineDaemonPaths};
use std::{
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
};

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const ENTRY: &str = "Meerkat Machine";
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub fn start(executable: &Path, home: &Path) -> Result<bool, AppError> {
    let command = format!(
        "{} --home {}",
        quote(&executable.to_string_lossy()),
        quote(&home.to_string_lossy())
    );
    let registered = Command::new("reg")
        .args([
            "add", RUN_KEY, "/v", ENTRY, "/t", "REG_SZ", "/d", &command, "/f",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !registered {
        return Err(AppError::InvalidArgument(super::NOT_STARTED));
    }
    spawn(executable, home).map_err(|_| AppError::InvalidArgument(super::NOT_STARTED))?;
    Ok(true)
}

/// Ends the Machine holding this home's instance lock. The image name filter
/// keeps a reused process id from matching another program.
pub fn stop(paths: &MachineDaemonPaths) -> bool {
    let Some(pid) = meerkat_daemon_runtime::instance_owner(&paths.instance_lock) else {
        return false;
    };
    Command::new("taskkill")
        .args(["/F", "/FI", &format!("PID eq {pid}")])
        .args(["/FI", "IMAGENAME eq meerkat-machine.exe"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub fn resume(executable: &Path, paths: &MachineDaemonPaths) {
    let _ = spawn(executable, &paths.home);
}

/// Detached so closing this terminal leaves the Machine running.
fn spawn(executable: &Path, home: &Path) -> std::io::Result<()> {
    Command::new(executable)
        .arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(drop)
}

pub fn after_connected() {}

pub fn not_connected(_home: &Path) -> &'static str {
    super::NOT_CONNECTED
}

/// Quotes one argument for a command line split with the Windows argv rules:
/// backslashes before a quote, including the closing one, are doubled.
fn quote(value: &str) -> String {
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        let doubled = if character == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        quoted.extend(std::iter::repeat_n('\\', doubled));
        quoted.push(character);
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}
