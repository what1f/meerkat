//! Per-user LaunchAgent written to ~/Library/LaunchAgents.

use meerkat_machine::{AppError, MachineDaemonPaths, user_home};
use std::{
    path::Path,
    process::{Command, Stdio},
};

const LABEL: &str = "com.meerkat.machine.remote";

pub fn start(executable: &Path, home: &Path) -> Result<bool, AppError> {
    let plist = plist_path()?;
    std::fs::create_dir_all(plist.parent().expect("LaunchAgents directory"))?;
    std::fs::write(&plist, launch_agent_plist(executable, home))?;
    if load(&plist)? {
        return Ok(true);
    }
    Err(AppError::InvalidArgument(super::NOT_STARTED))
}

pub fn stop(paths: &MachineDaemonPaths) -> bool {
    let (Ok(plist), Ok(uid)) = (plist_path(), user_id()) else {
        return false;
    };
    if !super::serves(&plist, &paths.home) {
        return false;
    }
    for domain in [format!("gui/{uid}"), format!("user/{uid}")] {
        launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
    }
    true
}

pub fn resume(_executable: &Path, _paths: &MachineDaemonPaths) {
    if let Ok(plist) = plist_path()
        && plist.is_file()
    {
        let _ = load(&plist);
    }
}

fn plist_path() -> Result<std::path::PathBuf, AppError> {
    Ok(user_home()?
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist")))
}

fn load(plist: &Path) -> Result<bool, AppError> {
    let uid = user_id()?;
    for domain in [format!("gui/{uid}"), format!("user/{uid}")] {
        launchctl(&["bootout", &format!("{domain}/{LABEL}")]);
        if launchctl(&["bootstrap", &domain, &plist.to_string_lossy()]) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn after_connected() {}

pub fn not_connected(home: &Path) -> &'static str {
    // The link fails with EHOSTUNREACH while macOS withholds Local Network access.
    if super::log_mentions(home, "os error 65") {
        "Allow meerkat-machine in System Settings > Privacy & Security > Local Network. It connects as soon as access is allowed."
    } else {
        super::NOT_CONNECTED
    }
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
