use meerkat_client::ClientError;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
#[cfg(target_os = "macos")]
use objc2_service_management::{SMAppService, SMAppServiceStatus};
#[cfg(target_os = "macos")]
use std::io::Write as _;

#[cfg(target_os = "macos")]
const RUNTIME_AGENT_PLIST: &str = "com.meerkat.daemon.plist";
#[cfg(target_os = "macos")]
const MACHINE_AGENT_PLIST: &str = "com.meerkat.machine.plist";
#[cfg(target_os = "macos")]
const AD_HOC_BACKGROUND_SERVICES_MARKER: &str = "AdHocBackgroundServices";
#[cfg(target_os = "macos")]
const RUNTIME_AGENT_LABEL: &str = "com.meerkat.daemon";
#[cfg(target_os = "macos")]
const MACHINE_AGENT_LABEL: &str = "com.meerkat.machine";

pub(crate) struct BackgroundServices {
    adapter: Adapter,
    stopping: bool,
}

enum Adapter {
    Transient {
        runtime: Option<Child>,
        machine: Option<Child>,
    },
    #[cfg(target_os = "macos")]
    ModernLaunchAgents {
        re_register_runtime: bool,
        re_register_machine: bool,
    },
    #[cfg(target_os = "macos")]
    AdHocLaunchAgents,
}

impl BackgroundServices {
    pub(crate) fn new(persistent: bool) -> Self {
        #[cfg(not(target_os = "macos"))]
        let _ = persistent;
        #[cfg(target_os = "macos")]
        if persistent && bundled_launch_agents_available() && !bundle_is_ephemeral() {
            return Self {
                stopping: false,
                adapter: if ad_hoc_background_services() {
                    Adapter::AdHocLaunchAgents
                } else {
                    Adapter::ModernLaunchAgents {
                        re_register_runtime: false,
                        re_register_machine: false,
                    }
                },
            };
        }

        Self {
            stopping: false,
            adapter: Adapter::Transient {
                runtime: None,
                machine: None,
            },
        }
    }

    pub(crate) fn prepare(&mut self, home: &Path) -> Result<(), ClientError> {
        #[cfg(target_os = "macos")]
        match &mut self.adapter {
            Adapter::ModernLaunchAgents {
                re_register_runtime,
                re_register_machine,
            } => {
                if cleanup_legacy_ad_hoc_launch_agent(
                    home,
                    MACHINE_AGENT_LABEL,
                    MACHINE_AGENT_PLIST,
                    "machine.lock",
                ) {
                    *re_register_machine = true;
                }
                if cleanup_legacy_ad_hoc_launch_agent(
                    home,
                    RUNTIME_AGENT_LABEL,
                    RUNTIME_AGENT_PLIST,
                    "runtime.lock",
                ) {
                    *re_register_runtime = true;
                }
                if prepare_launch_agent(home, "runtime.lock", "meerkatd", RUNTIME_AGENT_PLIST)? {
                    *re_register_runtime = true;
                }
                if prepare_launch_agent(
                    home,
                    "machine.lock",
                    "meerkat-machine",
                    MACHINE_AGENT_PLIST,
                )? {
                    *re_register_machine = true;
                }
                return Ok(());
            }
            Adapter::AdHocLaunchAgents => {
                if !ad_hoc_launch_agent_is_loaded(RUNTIME_AGENT_LABEL)
                    && !ad_hoc_launch_agent_is_loaded(MACHINE_AGENT_LABEL)
                {
                    unregister_modern_launch_agent(
                        RUNTIME_AGENT_PLIST,
                        &home.join("run/runtime.lock"),
                    );
                    unregister_modern_launch_agent(
                        MACHINE_AGENT_PLIST,
                        &home.join("run/machine.lock"),
                    );
                }
                install_ad_hoc_launch_agent(home, RUNTIME_AGENT_LABEL, "meerkatd", "runtime.lock")?;
                install_ad_hoc_launch_agent(
                    home,
                    MACHINE_AGENT_LABEL,
                    "meerkat-machine",
                    "machine.lock",
                )?;
                return Ok(());
            }
            _ => {}
        }

        replace_outdated_local_services(home)
    }

    /// Stops helpers still running an older build than this app bundles, so
    /// a healthy but outdated Runtime is never reused after an update.
    pub(crate) fn stop_outdated(&mut self, home: &Path) -> Result<bool, ClientError> {
        #[cfg(unix)]
        {
            let mut stopped = false;
            #[cfg(target_os = "macos")]
            if let Adapter::ModernLaunchAgents {
                re_register_runtime,
                re_register_machine,
            } = &mut self.adapter
            {
                if cleanup_legacy_ad_hoc_launch_agent(
                    home,
                    MACHINE_AGENT_LABEL,
                    MACHINE_AGENT_PLIST,
                    "machine.lock",
                ) {
                    *re_register_machine = true;
                    stopped = true;
                }
                if cleanup_legacy_ad_hoc_launch_agent(
                    home,
                    RUNTIME_AGENT_LABEL,
                    RUNTIME_AGENT_PLIST,
                    "runtime.lock",
                ) {
                    *re_register_runtime = true;
                    stopped = true;
                }
            }
            for (lock, executable, is_runtime) in [
                ("run/machine.lock", "meerkat-machine", false),
                ("run/runtime.lock", "meerkatd", true),
            ] {
                if let Some(instance) =
                    local_service(&home.join(lock), &helper_executable(executable)?, true)?
                {
                    stop_local_service(instance)?;
                    #[cfg(target_os = "macos")]
                    if let Adapter::ModernLaunchAgents {
                        re_register_runtime,
                        re_register_machine,
                    } = &mut self.adapter
                    {
                        if is_runtime {
                            *re_register_runtime = true;
                        } else {
                            *re_register_machine = true;
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    let _ = is_runtime;
                    stopped = true;
                }
            }
            Ok(stopped)
        }
        #[cfg(windows)]
        {
            replace_outdated_windows_services(home)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = home;
            Ok(false)
        }
    }

    pub(crate) fn ensure_runtime(&mut self, home: &Path) -> Result<(), ClientError> {
        if self.stopping {
            return Ok(());
        }
        match &mut self.adapter {
            Adapter::Transient { runtime, .. } => {
                if runtime.is_none() {
                    let executable = helper_executable("meerkatd")?;
                    *runtime = Some(spawn_helper(&executable, home)?);
                }
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents {
                re_register_runtime,
                ..
            } => ensure_launch_agent(
                RUNTIME_AGENT_PLIST,
                home,
                "runtime.lock",
                re_register_runtime,
            ),
            #[cfg(target_os = "macos")]
            Adapter::AdHocLaunchAgents => ensure_ad_hoc_launch_agent(RUNTIME_AGENT_LABEL),
        }
    }

    pub(crate) fn ensure_machine(&mut self, home: &Path) -> Result<(), ClientError> {
        if self.stopping {
            return Ok(());
        }
        match &mut self.adapter {
            Adapter::Transient { machine, .. } => {
                if machine.is_none() {
                    let executable = helper_executable("meerkat-machine")?;
                    *machine = Some(spawn_helper(&executable, home)?);
                }
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents {
                re_register_machine,
                ..
            } => ensure_launch_agent(
                MACHINE_AGENT_PLIST,
                home,
                "machine.lock",
                re_register_machine,
            ),
            #[cfg(target_os = "macos")]
            Adapter::AdHocLaunchAgents => ensure_ad_hoc_launch_agent(MACHINE_AGENT_LABEL),
        }
    }

    pub(crate) fn runtime_exited(&mut self) -> Result<Option<String>, ClientError> {
        match &mut self.adapter {
            Adapter::Transient { runtime, .. } => runtime
                .as_mut()
                .map(|child| {
                    child
                        .try_wait()
                        .map(|status| status.map(|status| status.to_string()))
                        .map_err(|error| ClientError::Unavailable(error.to_string()))
                })
                .transpose()
                .map(Option::flatten),
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents { .. } | Adapter::AdHocLaunchAgents => Ok(None),
        }
    }

    pub(crate) fn machine_exited(&mut self) -> Result<Option<String>, ClientError> {
        match &mut self.adapter {
            Adapter::Transient { machine, .. } => machine
                .as_mut()
                .map(|child| {
                    child
                        .try_wait()
                        .map(|status| status.map(|status| status.to_string()))
                        .map_err(|error| ClientError::Unavailable(error.to_string()))
                })
                .transpose()
                .map(Option::flatten),
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents { .. } | Adapter::AdHocLaunchAgents => Ok(None),
        }
    }

    pub(crate) fn stop_owned(&mut self) {
        self.stopping = true;
        match &mut self.adapter {
            Adapter::Transient { runtime, machine } => {
                for child in [machine.take(), runtime.take()].into_iter().flatten() {
                    stop_helper(child);
                }
            }
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents { .. } | Adapter::AdHocLaunchAgents => {}
        }
    }
}

impl Drop for BackgroundServices {
    fn drop(&mut self) {
        self.stop_owned();
    }
}

#[cfg(target_os = "macos")]
fn bundled_launch_agents_available() -> bool {
    let Ok(executable) = std::env::current_exe() else {
        return false;
    };
    let Some(contents) = executable.parent().and_then(Path::parent) else {
        return false;
    };
    [RUNTIME_AGENT_PLIST, MACHINE_AGENT_PLIST]
        .into_iter()
        .all(|name| contents.join("Library/LaunchAgents").join(name).is_file())
}

#[cfg(target_os = "macos")]
fn bundle_contents() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
fn bundle_is_ephemeral() -> bool {
    let Some(bundle) =
        bundle_contents().and_then(|contents| contents.parent().map(Path::to_path_buf))
    else {
        return false;
    };
    let translocated = bundle
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation");
    let mounted_dmg = bundle.starts_with("/Volumes")
        && bundle
            .parent()
            .is_some_and(|volume| volume.join("Applications").is_symlink());
    translocated || mounted_dmg
}

#[cfg(target_os = "macos")]
fn ad_hoc_background_services() -> bool {
    bundle_contents().is_some_and(|contents| {
        contents
            .join("Resources")
            .join(AD_HOC_BACKGROUND_SERVICES_MARKER)
            .is_file()
    })
}

#[cfg(target_os = "macos")]
fn launch_agent(plist_name: &str) -> objc2::rc::Retained<SMAppService> {
    let plist_name = NSString::from_str(plist_name);
    unsafe { SMAppService::agentServiceWithPlistName(&plist_name) }
}

#[cfg(target_os = "macos")]
fn ensure_launch_agent(
    plist_name: &str,
    home: &Path,
    lock_name: &str,
    force_re_register: &mut bool,
) -> Result<(), ClientError> {
    let re_register = std::mem::replace(force_re_register, false);
    let label = plist_name.trim_end_matches(".plist");
    let lock_path = home.join("run").join(lock_name);
    let service = launch_agent(plist_name);
    match unsafe { service.status() } {
        SMAppServiceStatus::RequiresApproval => return Err(background_permission_error()),
        SMAppServiceStatus::Enabled => {
            if !re_register
                && modern_launch_agent_matches_bundle(label)
                && start_launch_agent(label).is_ok()
            {
                return Ok(());
            }
            unregister_modern_launch_agent(plist_name, &lock_path);
        }
        // Before a LaunchAgent is first recorded in BackgroundTaskManagement,
        // macOS 14+ reports SMAppServiceStatus::NotFound rather than NotRegistered.
        SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => {
            if ad_hoc_launch_agent_is_loaded(label) {
                bootout_ad_hoc_launch_agent(label);
                wait_for_lock_release(&lock_path);
            }
        }
        _ => return Err(background_installation_error()),
    }

    if let Err(error) = unsafe { service.registerAndReturnError() } {
        eprintln!("meerkat-desktop: could not register {plist_name}: {error}");
        return match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => start_launch_agent(label),
            SMAppServiceStatus::RequiresApproval => Err(background_permission_error()),
            _ => Err(background_installation_error()),
        };
    }

    match unsafe { service.status() } {
        SMAppServiceStatus::RequiresApproval => Err(background_permission_error()),
        SMAppServiceStatus::Enabled => {
            if ad_hoc_launch_agent_is_loaded(label) {
                let _ = start_launch_agent(label);
            }
            Ok(())
        }
        SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound
            if ad_hoc_launch_agent_is_loaded(label) =>
        {
            let _ = start_launch_agent(label);
            Ok(())
        }
        _ => Err(background_installation_error()),
    }
}

#[cfg(target_os = "macos")]
fn unregister_modern_launch_agent(plist_name: &str, lock_path: &Path) {
    let label = plist_name.trim_end_matches(".plist");
    let service = launch_agent(plist_name);
    if unsafe { service.status() } != SMAppServiceStatus::NotRegistered {
        if let Err(error) = unsafe { service.unregisterAndReturnError() } {
            eprintln!("meerkat-desktop: could not unregister {plist_name}: {error}");
        }
        for _ in 0..100 {
            if !ad_hoc_launch_agent_is_loaded(label) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        wait_for_lock_release(lock_path);
    }
}

#[cfg(target_os = "macos")]
fn modern_launch_agent_matches_bundle(label: &str) -> bool {
    let Some(services) = bundle_contents().map(|contents| contents.join("Library/LaunchServices"))
    else {
        return false;
    };
    let Ok(domain) = launch_agent_domain() else {
        return false;
    };
    let Ok(output) = Command::new("/bin/launchctl")
        .args(["print", &format!("{domain}/{label}")])
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let matches_program = stdout.lines().any(|line| {
        line.trim()
            .strip_prefix("program = ")
            .is_some_and(|program| Path::new(program.trim()).starts_with(&services))
    });
    matches_program && !launchctl_output_uses_legacy_plist(&stdout, label)
}

#[cfg(target_os = "macos")]
fn legacy_ad_hoc_plist_path(label: &str) -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).map(|home| {
        home.join("Library/LaunchAgents")
            .join(format!("{label}.plist"))
    })
}

#[cfg(target_os = "macos")]
fn launchctl_output_uses_legacy_plist(stdout: &str, label: &str) -> bool {
    let Some(legacy_path) = legacy_ad_hoc_plist_path(label) else {
        return false;
    };
    stdout.lines().any(|line| {
        line.trim()
            .strip_prefix("path = ")
            .is_some_and(|path| Path::new(path.trim()) == legacy_path)
    })
}

#[cfg(target_os = "macos")]
fn launchd_service_uses_legacy_plist(label: &str) -> bool {
    let Ok(domain) = launch_agent_domain() else {
        return false;
    };
    let Ok(output) = Command::new("/bin/launchctl")
        .args(["print", &format!("{domain}/{label}")])
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    output.status.success()
        && launchctl_output_uses_legacy_plist(&String::from_utf8_lossy(&output.stdout), label)
}

#[cfg(target_os = "macos")]
fn cleanup_legacy_ad_hoc_launch_agent(
    home: &Path,
    label: &str,
    plist_name: &str,
    lock_name: &str,
) -> bool {
    let legacy_path = legacy_ad_hoc_plist_path(label);
    let plist_exists = legacy_path
        .as_deref()
        .is_some_and(|path| fs::symlink_metadata(path).is_ok());
    let loaded_from_legacy = launchd_service_uses_legacy_plist(label);
    if !plist_exists && !loaded_from_legacy {
        return false;
    }
    let lock_path = home.join("run").join(lock_name);
    bootout_ad_hoc_launch_agent(label);
    if let Some(path) = legacy_path {
        let _ = fs::remove_file(path);
    }
    unregister_modern_launch_agent(plist_name, &lock_path);
    wait_for_lock_release(&lock_path);
    true
}

#[cfg(target_os = "macos")]
fn install_ad_hoc_launch_agent(
    home: &Path,
    label: &str,
    executable_name: &str,
    lock_name: &str,
) -> Result<(), ClientError> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let executable = helper_executable(executable_name)?;
    let helper_outdated =
        local_service(&home.join("run").join(lock_name), &executable, true)?.is_some();
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(background_installation_error)?;
    let directory = user_home.join("Library/LaunchAgents");
    fs::create_dir_all(&directory).map_err(|error| ClientError::Unavailable(error.to_string()))?;

    let path = directory.join(format!("{label}.plist"));
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(background_installation_error());
    }
    let contents = ad_hoc_launch_agent_plist(label, &executable, home);
    if fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
        bootout_ad_hoc_launch_agent(label);
        let temporary = directory.join(format!(".{label}.{}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        let write_result = file
            .write_all(contents.as_bytes())
            .and_then(|_| file.sync_all());
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            return Err(ClientError::Unavailable(error.to_string()));
        }
        fs::rename(&temporary, &path)
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    } else if helper_outdated && ad_hoc_launch_agent_is_loaded(label) {
        restart_launch_agent(label)?;
        return Ok(());
    }

    let domain = launch_agent_domain()?;
    let mut last_error = String::new();
    for _ in 0..50 {
        if ad_hoc_launch_agent_is_loaded(label) {
            return Ok(());
        }
        let output = Command::new("/bin/launchctl")
            .args(["bootstrap", &domain])
            .arg(&path)
            .output()
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        if output.status.success() {
            return Ok(());
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("meerkat-desktop: could not bootstrap {label}: {last_error}");
    Err(background_installation_error())
}

#[cfg(target_os = "macos")]
fn start_launch_agent(label: &str) -> Result<(), ClientError> {
    let domain = launch_agent_domain()?;
    // Without -k, launchd leaves an already-running owner in place.
    let output = Command::new("/bin/launchctl")
        .args(["kickstart", &format!("{domain}/{label}")])
        .output()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        eprintln!(
            "meerkat-desktop: could not start {label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Err(background_installation_error())
    }
}

/// Restarts the helpers launchd runs from `bundle` so they pick up the
/// binaries now on disk; helpers owned by another install are left alone.
#[cfg(target_os = "macos")]
pub(crate) fn restart_bundled_helpers(bundle: &Path) {
    let Ok(domain) = launch_agent_domain() else {
        return;
    };
    let services = bundle.join("Contents/Library/LaunchServices");
    for label in [RUNTIME_AGENT_LABEL, MACHINE_AGENT_LABEL] {
        let Ok(output) = Command::new("/bin/launchctl")
            .args(["print", &format!("{domain}/{label}")])
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        let owned = String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.trim()
                .strip_prefix("program = ")
                .is_some_and(|program| Path::new(program).starts_with(&services))
        });
        if output.status.success() && owned {
            let _ = restart_launch_agent(label);
        }
    }
}

#[cfg(target_os = "macos")]
fn restart_launch_agent(label: &str) -> Result<(), ClientError> {
    let domain = launch_agent_domain()?;
    let output = Command::new("/bin/launchctl")
        .args(["kickstart", "-k", &format!("{domain}/{label}")])
        .output()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        eprintln!(
            "meerkat-desktop: could not restart {label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Err(background_installation_error())
    }
}

#[cfg(target_os = "macos")]
fn ensure_ad_hoc_launch_agent(label: &str) -> Result<(), ClientError> {
    if ad_hoc_launch_agent_is_loaded(label) {
        start_launch_agent(label)
    } else {
        Err(background_installation_error())
    }
}

#[cfg(target_os = "macos")]
fn ad_hoc_launch_agent_is_loaded(label: &str) -> bool {
    let Ok(domain) = launch_agent_domain() else {
        return false;
    };
    Command::new("/bin/launchctl")
        .args(["print", &format!("{domain}/{label}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "macos")]
fn bootout_ad_hoc_launch_agent(label: &str) {
    let Ok(domain) = launch_agent_domain() else {
        return;
    };
    let status = Command::new("/bin/launchctl")
        .args(["bootout", &format!("{domain}/{label}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if status.is_ok_and(|status| status.success()) {
        for _ in 0..100 {
            if !ad_hoc_launch_agent_is_loaded(label) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(target_os = "macos")]
fn launch_agent_domain() -> Result<String, ClientError> {
    let output = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    if !output.status.success() {
        return Err(background_installation_error());
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(background_installation_error());
    }
    Ok(format!("gui/{uid}"))
}

#[cfg(target_os = "macos")]
fn ad_hoc_launch_agent_plist(label: &str, executable: &Path, home: &Path) -> String {
    let executable = xml_escape(&executable.to_string_lossy());
    let home = xml_escape(&home.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
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

#[cfg(target_os = "macos")]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "macos")]
fn background_permission_error() -> ClientError {
    ClientError::Unavailable("请在系统设置的登录项中允许 Meerkat 在后台运行。".into())
}

#[cfg(target_os = "macos")]
fn background_installation_error() -> ClientError {
    ClientError::Unavailable("Meerkat 无法在后台运行，请重新安装后重试。".into())
}

#[cfg(target_os = "macos")]
fn prepare_launch_agent(
    home: &Path,
    lock_name: &str,
    executable_name: &str,
    plist_name: &str,
) -> Result<bool, ClientError> {
    let status = unsafe { launch_agent(plist_name).status() };
    let only_if_outdated = status == SMAppServiceStatus::Enabled;
    if let Some(instance) = local_service(
        &home.join("run").join(lock_name),
        &helper_executable(executable_name)?,
        only_if_outdated,
    )? {
        stop_local_service(instance)?;
        return Ok(true);
    }
    Ok(false)
}

fn stop_helper(mut child: Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    {
        let pid = child.id().to_string();
        let _ = Command::new("kill").args(["-TERM", &pid]).status();
        std::thread::scope(|scope| {
            let (done, exited) = std::sync::mpsc::sync_channel(1);
            let waiter = scope.spawn(move || {
                let _ = child.wait();
                let _ = done.send(());
            });
            if exited.recv_timeout(Duration::from_secs(1)).is_err() {
                let _ = Command::new("kill").args(["-KILL", &pid]).status();
            }
            let _ = waiter.join();
        });
    }
    #[cfg(not(unix))]
    {
        // Closing piped stdin signals `--shutdown-on-stdin-eof` helpers to exit gracefully.
        drop(child.stdin.take());
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
fn replace_outdated_local_services(home: &Path) -> Result<(), ClientError> {
    let runtime = local_service(
        &home.join("run/runtime.lock"),
        &helper_executable("meerkatd")?,
        true,
    )?;
    let machine = local_service(
        &home.join("run/machine.lock"),
        &helper_executable("meerkat-machine")?,
        true,
    )?;

    if let Some(instance) = machine {
        stop_local_service(instance)?;
    }
    if let Some(instance) = runtime {
        stop_local_service(instance)?;
    }
    Ok(())
}

#[cfg(windows)]
fn replace_outdated_local_services(home: &Path) -> Result<(), ClientError> {
    let _ = replace_outdated_windows_services(home)?;
    Ok(())
}

#[cfg(windows)]
fn replace_outdated_windows_services(home: &Path) -> Result<bool, ClientError> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut stopped = false;
    for (lock, executable_name) in [
        ("run/machine.lock", "meerkat-machine"),
        ("run/runtime.lock", "meerkatd"),
    ] {
        let lock_path = home.join(lock);
        let Ok(lock_metadata) = fs::symlink_metadata(&lock_path) else {
            continue;
        };
        if !lock_metadata.file_type().is_file() {
            continue;
        }
        let executable = helper_executable(executable_name)?;
        let Ok(executable_modified) = fs::metadata(&executable).and_then(|meta| meta.modified())
        else {
            continue;
        };
        let Ok(lock_modified) = lock_metadata.modified() else {
            continue;
        };
        if executable_modified <= lock_modified {
            continue;
        }
        let Ok(lock_contents) = fs::read_to_string(&lock_path) else {
            continue;
        };
        let Some(pid) = local_service_lock_pid(&lock_contents) else {
            continue;
        };
        match windows_process_is_alive(pid) {
            Some(true) => {}
            Some(false) => continue,
            None => return Err(background_update_error()),
        }
        let image = format!("{executable_name}{}", std::env::consts::EXE_SUFFIX);
        let status = Command::new("taskkill")
            .args(["/F", "/FI", &format!("PID eq {pid}")])
            .args(["/FI", &format!("IMAGENAME eq {image}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        if !status.success() {
            return Err(background_update_error());
        }
        let mut exited = false;
        for _ in 0..100 {
            match windows_process_is_alive(pid) {
                Some(false) => {
                    exited = true;
                    break;
                }
                Some(true) => std::thread::sleep(Duration::from_millis(20)),
                None => return Err(background_update_error()),
            }
        }
        if !exited {
            return Err(background_update_error());
        }
        // Leave stale lock reclamation to the newly spawned service's InstanceGuard::acquire
        // so a concurrent startup cannot lose its newly created lock file.
        stopped = true;
    }
    Ok(stopped)
}

#[cfg(windows)]
fn windows_process_is_alive(pid: u32) -> Option<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
}

#[cfg(not(any(unix, windows)))]
fn replace_outdated_local_services(_home: &Path) -> Result<(), ClientError> {
    Ok(())
}

#[cfg(unix)]
#[derive(Debug)]
struct LocalService {
    pid: u32,
    lock_path: PathBuf,
    lock_contents: String,
}

#[cfg(unix)]
fn local_service(
    lock_path: &Path,
    executable: &Path,
    only_if_outdated: bool,
) -> Result<Option<LocalService>, ClientError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let lock_metadata = match fs::symlink_metadata(lock_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ClientError::Unavailable(error.to_string())),
    };
    let Some(parent) = lock_path.parent() else {
        return Ok(None);
    };
    let parent_metadata =
        fs::metadata(parent).map_err(|error| ClientError::Unavailable(error.to_string()))?;
    if !lock_metadata.file_type().is_file()
        || lock_metadata.uid() != parent_metadata.uid()
        || lock_metadata.permissions().mode() & 0o777 != 0o600
    {
        return Ok(None);
    }

    if only_if_outdated {
        // Installs copy files with their build mtime, so compare the change
        // time that records when this executable landed on disk.
        let executable_installed = fs::metadata(executable)
            .map(|metadata| (metadata.ctime(), metadata.ctime_nsec()))
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        let lock_written = (lock_metadata.mtime(), lock_metadata.mtime_nsec());
        if executable_installed <= lock_written {
            return Ok(None);
        }
    }

    let lock_contents = fs::read_to_string(lock_path)
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    let Some(pid) = local_service_lock_pid(&lock_contents) else {
        return Ok(None);
    };
    if !local_process_is_alive(pid)? {
        return Ok(None);
    }
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let command = String::from_utf8_lossy(&output.stdout);
    if !helper_command_matches(command.trim(), executable) {
        return Ok(None);
    }
    if fs::read_to_string(lock_path).ok().as_deref() != Some(lock_contents.as_str()) {
        return Ok(None);
    }

    Ok(Some(LocalService {
        pid,
        lock_path: lock_path.to_path_buf(),
        lock_contents,
    }))
}

#[cfg(unix)]
fn stop_local_service(instance: LocalService) -> Result<(), ClientError> {
    if fs::read_to_string(&instance.lock_path).ok().as_deref()
        != Some(instance.lock_contents.as_str())
    {
        return Ok(());
    }
    let status = Command::new("kill")
        .args(["-TERM", &instance.pid.to_string()])
        .status()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    if !status.success() {
        return Err(background_update_error());
    }
    for _ in 0..100 {
        if !local_process_is_alive(instance.pid)? {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(background_update_error())
}

#[cfg(any(unix, windows))]
fn background_update_error() -> ClientError {
    ClientError::Unavailable("Meerkat 尚未完成更新，请退出并重新打开。".into())
}

#[cfg(unix)]
fn local_process_is_alive(pid: u32) -> Result<bool, ClientError> {
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    Ok(status.success())
}

#[cfg(unix)]
fn wait_for_lock_release(lock_path: &Path) {
    for _ in 0..100 {
        let Ok(contents) = fs::read_to_string(lock_path) else {
            return;
        };
        let Some(pid) = local_service_lock_pid(&contents) else {
            return;
        };
        if !local_process_is_alive(pid).unwrap_or(false) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(any(unix, windows))]
fn local_service_lock_pid(contents: &str) -> Option<u32> {
    let fields = contents.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2 {
        return None;
    }
    let mut pid = None;
    let mut run_id = None;
    for field in fields {
        if let Some(value) = field.strip_prefix("pid=") {
            pid = value.parse::<u32>().ok().filter(|pid| *pid > 0);
        } else if let Some(value) = field.strip_prefix("run_id=") {
            run_id = (!value.is_empty()).then_some(value);
        } else {
            return None;
        }
    }
    run_id.and(pid)
}

#[cfg(unix)]
fn helper_command_matches(command: &str, executable: &Path) -> bool {
    let matches = |expected: &str| {
        command == expected
            || command
                .strip_prefix(expected)
                .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
    };
    if matches(&executable.to_string_lossy()) {
        return true;
    }
    #[cfg(target_os = "macos")]
    if let Some(name) = executable.file_name().and_then(|name| name.to_str())
        && executable.ends_with(Path::new("Contents/Library/LaunchServices").join(name))
    {
        return matches(&format!("Contents/Library/LaunchServices/{name}"));
    }
    false
}

fn helper_executable(name: &str) -> Result<PathBuf, ClientError> {
    let binary = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let current =
        std::env::current_exe().map_err(|error| ClientError::Unavailable(error.to_string()))?;
    let bundled = current
        .parent()
        .and_then(Path::parent)
        .map(|contents| contents.join("Library/LaunchServices").join(&binary));
    if let Some(path) = bundled.filter(|path| path.is_file()) {
        return Ok(path);
    }
    let nested = current
        .parent()
        .map(|directory| directory.join("bin").join(&binary));
    if let Some(path) = nested.filter(|path| path.is_file()) {
        return Ok(path);
    }
    let sibling = current.with_file_name(&binary);
    if sibling.is_file() {
        return Ok(sibling);
    }
    Err(ClientError::Unavailable(format!(
        "找不到本地服务程序 {name}"
    )))
}

fn spawn_helper(executable: &Path, home: &Path) -> Result<Child, ClientError> {
    let mut command = Command::new(executable);
    command
        .arg("--home")
        .arg(home)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command
            .arg("--shutdown-on-stdin-eof")
            .stdin(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    command.stdin(Stdio::null());
    command
        .spawn()
        .map_err(|error| ClientError::Unavailable(error.to_string()))
}
