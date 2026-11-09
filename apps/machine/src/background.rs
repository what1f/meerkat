//! Keeps a paired remote Machine running as a per-user LaunchAgent on macOS.

use meerkat_machine::{AppError, MachineDaemonPaths, connected_marker, machine_name};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const LABEL: &str = "com.meerkat.machine.remote";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);

pub async fn install(paths: &MachineDaemonPaths) -> Result<(), AppError> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let user_home = std::env::var_os("HOME").ok_or(AppError::MissingHome)?;
    let agents = PathBuf::from(user_home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&agents)?;
    let plist = agents.join(format!("{LABEL}.plist"));
    std::fs::write(&plist, launch_agent_plist(&executable, &paths.home))?;

    let marker = connected_marker(paths);
    let _ = std::fs::remove_file(&marker);
    let uid = user_id()?;
    let mut loaded = false;
    for domain in [format!("gui/{uid}"), format!("user/{uid}")] {
        launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
        if launchctl(&["bootstrap", &domain, &plist.to_string_lossy()]) {
            loaded = true;
            break;
        }
    }
    if !loaded {
        return Err(AppError::InvalidArgument(
            "Paired, but could not start in the background. Run meerkat-machine to stay connected.",
        ));
    }

    let deadline = Instant::now() + CONNECT_TIMEOUT;
    while Instant::now() < deadline {
        if marker.exists() {
            println!("Connected as {}", machine_name());
            println!("Runs in the background and starts at login.");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(AppError::InvalidArgument(
        if local_network_blocked(&paths.home) {
            "Allow meerkat-machine in System Settings > Privacy & Security > Local Network. It connects as soon as access is allowed."
        } else {
            "Paired, but not connected yet. It keeps retrying in the background; see ~/.meerkat/logs/meerkat-machine.log."
        },
    ))
}

fn launchctl(arguments: &[&str]) -> bool {
    Command::new("/bin/launchctl")
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn user_id() -> Result<String, AppError> {
    let output = Command::new("/usr/bin/id").arg("-u").output()?;
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || uid.is_empty() {
        return Err(AppError::Invariant("user id unavailable"));
    }
    Ok(uid)
}

/// The link fails with EHOSTUNREACH while macOS withholds Local Network access.
fn local_network_blocked(home: &Path) -> bool {
    std::fs::read_to_string(home.join("logs/meerkat-machine.log")).is_ok_and(|log| {
        log.lines()
            .rev()
            .take(5)
            .any(|line| line.contains("os error 65"))
    })
}

fn launch_agent_plist(executable: &Path, home: &Path) -> String {
    let executable = xml_escape(&executable.to_string_lossy());
    let home = xml_escape(&home.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{executable}</string>
        <string>--home</string>
        <string>{home}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Background</string>
    <key>ThrottleInterval</key>
    <integer>5</integer>
</dict>
</plist>
"#
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
