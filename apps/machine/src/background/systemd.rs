//! systemd user service written to ~/.config/systemd/user.

use meerkat_machine::{AppError, MachineDaemonPaths, user_home};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const UNIT: &str = "meerkat-machine.service";

pub fn start(executable: &Path, home: &Path) -> Result<bool, AppError> {
    // Containers and minimal systems often run without a user service manager.
    if !systemctl(&["show-environment"]) {
        println!("No background service manager here, so it stays in this terminal.");
        return Ok(false);
    }
    let path = unit_path()?;
    std::fs::create_dir_all(path.parent().expect("systemd user directory"))?;
    std::fs::write(&path, unit(executable, home))?;
    for arguments in [
        &["daemon-reload"][..],
        &["enable", UNIT],
        &["restart", UNIT],
    ] {
        if !systemctl(arguments) {
            return Err(AppError::InvalidArgument(super::NOT_STARTED));
        }
    }
    Ok(true)
}

pub fn stop(paths: &MachineDaemonPaths) -> bool {
    let Ok(path) = unit_path() else { return false };
    super::serves(&path, &paths.home) && systemctl(&["stop", UNIT])
}

pub fn resume(_executable: &Path, _paths: &MachineDaemonPaths) {
    systemctl(&["start", UNIT]);
}

/// A user service stops when its last session ends unless lingering is on,
/// which matters for machines reached over SSH.
pub fn after_connected() {
    let Some(user) = user_name() else { return };
    let lingering = Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger", "--value"])
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "yes");
    if lingering || quiet("loginctl", &["enable-linger", &user]) {
        return;
    }
    println!("To keep it running after you sign out, run: sudo loginctl enable-linger {user}");
}

pub fn not_connected(_home: &Path) -> &'static str {
    super::NOT_CONNECTED
}

fn unit_path() -> Result<PathBuf, AppError> {
    let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        _ => user_home()?.join(".config"),
    };
    Ok(config.join("systemd/user").join(UNIT))
}

fn user_name() -> Option<String> {
    let output = Command::new("id").arg("-un").output().ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (output.status.success() && !name.is_empty()).then_some(name)
}

fn systemctl(arguments: &[&str]) -> bool {
    let mut all = vec!["--user"];
    all.extend_from_slice(arguments);
    quiet("systemctl", &all)
}

fn quiet(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn unit(executable: &Path, home: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=Meerkat Machine\n\
         \n\
         [Service]\n\
         ExecStart={} --home {}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        quote(&executable.to_string_lossy(), false),
        quote(&home.to_string_lossy(), true),
    )
}

/// Quotes one ExecStart word. Specifiers (`%`) apply to the whole line, but
/// `$` is expanded only in arguments, not in the executable path.
fn quote(value: &str, argument: bool) -> String {
    let mut escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    if argument {
        escaped = escaped.replace('$', "$$");
    }
    format!("\"{escaped}\"")
}
