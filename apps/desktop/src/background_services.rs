use lankide_client::ClientError;
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
const RUNTIME_AGENT_PLIST: &str = "com.lankide.runtime.plist";
#[cfg(target_os = "macos")]
const COMPUTER_AGENT_PLIST: &str = "com.lankide.computer.plist";
#[cfg(target_os = "macos")]
const AD_HOC_BACKGROUND_SERVICES_MARKER: &str = "AdHocBackgroundServices";
#[cfg(target_os = "macos")]
const RUNTIME_AGENT_LABEL: &str = "com.lankide.runtime";
#[cfg(target_os = "macos")]
const COMPUTER_AGENT_LABEL: &str = "com.lankide.computer";

pub(crate) struct BackgroundServices {
    adapter: Adapter,
}

enum Adapter {
    Transient {
        runtime: Option<Child>,
        computer: Option<Child>,
    },
    #[cfg(target_os = "macos")]
    ModernLaunchAgents,
    #[cfg(target_os = "macos")]
    AdHocLaunchAgents,
}

impl BackgroundServices {
    pub(crate) fn new(persistent: bool) -> Self {
        #[cfg(target_os = "macos")]
        if persistent && bundled_launch_agents_available() {
            return Self {
                adapter: if ad_hoc_background_services() {
                    Adapter::AdHocLaunchAgents
                } else {
                    Adapter::ModernLaunchAgents
                },
            };
        }

        Self {
            adapter: Adapter::Transient {
                runtime: None,
                computer: None,
            },
        }
    }

    pub(crate) fn prepare(&mut self, home: &Path) -> Result<(), ClientError> {
        #[cfg(target_os = "macos")]
        match self.adapter {
            Adapter::ModernLaunchAgents => {
                prepare_launch_agent(
                    home,
                    "runtime.lock",
                    "lankide-runtime-daemon",
                    RUNTIME_AGENT_PLIST,
                )?;
                prepare_launch_agent(
                    home,
                    "computer.lock",
                    "lankide-computer-daemon",
                    COMPUTER_AGENT_PLIST,
                )?;
                return Ok(());
            }
            Adapter::AdHocLaunchAgents => {
                if !ad_hoc_launch_agent_is_loaded(RUNTIME_AGENT_LABEL)
                    && !ad_hoc_launch_agent_is_loaded(COMPUTER_AGENT_LABEL)
                {
                    unregister_modern_launch_agent(RUNTIME_AGENT_PLIST);
                    unregister_modern_launch_agent(COMPUTER_AGENT_PLIST);
                }
                install_ad_hoc_launch_agent(
                    home,
                    RUNTIME_AGENT_LABEL,
                    "lankide-runtime-daemon",
                    "runtime.lock",
                )?;
                install_ad_hoc_launch_agent(
                    home,
                    COMPUTER_AGENT_LABEL,
                    "lankide-computer-daemon",
                    "computer.lock",
                )?;
                return Ok(());
            }
            _ => {}
        }

        replace_outdated_local_services(home)
    }

    pub(crate) fn ensure_runtime(&mut self, home: &Path) -> Result<(), ClientError> {
        match &mut self.adapter {
            Adapter::Transient { runtime, .. } => {
                if runtime.is_none() {
                    let executable = helper_executable("lankide-runtime-daemon")?;
                    *runtime = Some(spawn_helper(&executable, home)?);
                }
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents => ensure_launch_agent(RUNTIME_AGENT_PLIST),
            #[cfg(target_os = "macos")]
            Adapter::AdHocLaunchAgents => ensure_ad_hoc_launch_agent(RUNTIME_AGENT_LABEL),
        }
    }

    pub(crate) fn ensure_computer(&mut self, home: &Path) -> Result<(), ClientError> {
        match &mut self.adapter {
            Adapter::Transient { computer, .. } => {
                if computer.is_none() {
                    let executable = helper_executable("lankide-computer-daemon")?;
                    *computer = Some(spawn_helper(&executable, home)?);
                }
                Ok(())
            }
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents => ensure_launch_agent(COMPUTER_AGENT_PLIST),
            #[cfg(target_os = "macos")]
            Adapter::AdHocLaunchAgents => ensure_ad_hoc_launch_agent(COMPUTER_AGENT_LABEL),
        }
    }

    pub(crate) fn computer_exited(&mut self) -> Result<Option<String>, ClientError> {
        match &mut self.adapter {
            Adapter::Transient { computer, .. } => computer
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
            Adapter::ModernLaunchAgents | Adapter::AdHocLaunchAgents => Ok(None),
        }
    }

    pub(crate) fn stop_owned(&mut self) {
        match &mut self.adapter {
            Adapter::Transient { runtime, computer } => {
                for mut child in [computer.take(), runtime.take()].into_iter().flatten() {
                    stop_helper(&mut child);
                }
            }
            #[cfg(target_os = "macos")]
            Adapter::ModernLaunchAgents | Adapter::AdHocLaunchAgents => {}
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
    [RUNTIME_AGENT_PLIST, COMPUTER_AGENT_PLIST]
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
fn ensure_launch_agent(plist_name: &str) -> Result<(), ClientError> {
    let service = launch_agent(plist_name);
    match unsafe { service.status() } {
        SMAppServiceStatus::Enabled => {
            return start_launch_agent(plist_name.trim_end_matches(".plist"));
        }
        SMAppServiceStatus::RequiresApproval => return Err(background_permission_error()),
        SMAppServiceStatus::NotFound => return Err(background_installation_error()),
        SMAppServiceStatus::NotRegistered => {}
        _ => return Err(background_installation_error()),
    }

    if let Err(error) = unsafe { service.registerAndReturnError() } {
        eprintln!("lankide-desktop: could not register {plist_name}: {error}");
        return match unsafe { service.status() } {
            SMAppServiceStatus::Enabled => Ok(()),
            SMAppServiceStatus::RequiresApproval => Err(background_permission_error()),
            _ => Err(background_installation_error()),
        };
    }

    match unsafe { service.status() } {
        SMAppServiceStatus::Enabled => Ok(()),
        SMAppServiceStatus::RequiresApproval => Err(background_permission_error()),
        _ => Err(background_installation_error()),
    }
}

#[cfg(target_os = "macos")]
fn unregister_modern_launch_agent(plist_name: &str) {
    let service = launch_agent(plist_name);
    if unsafe { service.status() } == SMAppServiceStatus::NotRegistered {
        return;
    }
    if let Err(error) = unsafe { service.unregisterAndReturnError() } {
        eprintln!("lankide-desktop: could not unregister {plist_name}: {error}");
    }
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
    eprintln!("lankide-desktop: could not bootstrap {label}: {last_error}");
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
            "lankide-desktop: could not start {label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Err(background_installation_error())
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
            "lankide-desktop: could not restart {label}: {}",
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
    ClientError::Unavailable("请在系统设置的登录项中允许 Lankide 在后台运行。".into())
}

#[cfg(target_os = "macos")]
fn background_installation_error() -> ClientError {
    ClientError::Unavailable("Lankide 无法在后台运行，请重新安装后重试。".into())
}

#[cfg(target_os = "macos")]
fn prepare_launch_agent(
    home: &Path,
    lock_name: &str,
    executable_name: &str,
    plist_name: &str,
) -> Result<(), ClientError> {
    let status = unsafe { launch_agent(plist_name).status() };
    let only_if_outdated = status == SMAppServiceStatus::Enabled;
    if let Some(instance) = local_service(
        &home.join("run").join(lock_name),
        &helper_executable(executable_name)?,
        only_if_outdated,
    )? {
        stop_local_service(instance)?;
    }
    Ok(())
}

fn stop_helper(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    for _ in 0..50 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn replace_outdated_local_services(home: &Path) -> Result<(), ClientError> {
    let runtime = local_service(
        &home.join("run/runtime.lock"),
        &helper_executable("lankide-runtime-daemon")?,
        true,
    )?;
    let computer = local_service(
        &home.join("run/computer.lock"),
        &helper_executable("lankide-computer-daemon")?,
        true,
    )?;

    if let Some(instance) = computer {
        stop_local_service(instance)?;
    }
    if let Some(instance) = runtime {
        stop_local_service(instance)?;
    }
    Ok(())
}

#[cfg(not(unix))]
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
        let executable_modified = fs::metadata(executable)
            .and_then(|metadata| metadata.modified())
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        let lock_modified = lock_metadata
            .modified()
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        if executable_modified <= lock_modified {
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

#[cfg(unix)]
fn background_update_error() -> ClientError {
    ClientError::Unavailable("Lankide 尚未完成更新，请退出并重新打开。".into())
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

fn helper_command_matches(command: &str, executable: &Path) -> bool {
    let expected = executable.to_string_lossy();
    command == expected
        || command
            .strip_prefix(expected.as_ref())
            .is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
}

fn helper_executable(name: &str) -> Result<PathBuf, ClientError> {
    let current =
        std::env::current_exe().map_err(|error| ClientError::Unavailable(error.to_string()))?;
    let bundled = current
        .parent()
        .and_then(Path::parent)
        .map(|contents| contents.join("Library/LaunchServices").join(name));
    if let Some(path) = bundled.filter(|path| path.is_file()) {
        return Ok(path);
    }
    let sibling = current.with_file_name(name);
    if sibling.is_file() {
        return Ok(sibling);
    }
    Err(ClientError::Unavailable(format!(
        "找不到本地服务程序 {name}"
    )))
}

fn spawn_helper(executable: &Path, home: &Path) -> Result<Child, ClientError> {
    let logs = home.join("logs");
    fs::create_dir_all(&logs).map_err(|error| ClientError::Unavailable(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o700))
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    }
    let name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("service");
    let path = logs.join(format!("{name}.log"));
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ClientError::Unavailable(
            "服务日志路径不能是符号链接。".into(),
        ));
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = options
        .open(path)
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    let stderr = log
        .try_clone()
        .map_err(|error| ClientError::Unavailable(error.to_string()))?;
    Command::new(executable)
        .arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(stderr)
        .spawn()
        .map_err(|error| ClientError::Unavailable(error.to_string()))
}
