#![forbid(unsafe_code)]

//! Shared filesystem lifecycle for local Meerkat daemons.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug)]
pub enum DaemonRuntimeError {
    Io(io::Error),
    InvalidInstanceLock,
    UnsafeDirectory,
    UnsafeInstanceLock,
    AlreadyRunning,
}

impl std::fmt::Display for DaemonRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "daemon_runtime_io: {error}"),
            Self::InvalidInstanceLock => formatter.write_str("daemon_runtime_invalid_lock"),
            Self::UnsafeDirectory => formatter.write_str("daemon_runtime_unsafe_directory"),
            Self::UnsafeInstanceLock => formatter.write_str("daemon_runtime_unsafe_lock"),
            Self::AlreadyRunning => formatter.write_str("daemon_runtime_already_running"),
        }
    }
}

impl std::error::Error for DaemonRuntimeError {}

impl From<io::Error> for DaemonRuntimeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn create_private_directory(path: &Path) -> Result<(), DaemonRuntimeError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(DaemonRuntimeError::UnsafeDirectory);
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE))?;
    #[cfg(windows)]
    restrict_to_current_user(path)?;
    Ok(())
}

/// Replaces the whole access list with the current user and SYSTEM, so a home
/// placed in a shared folder does not expose credentials to other users.
#[cfg(windows)]
fn restrict_to_current_user(path: &Path) -> Result<(), DaemonRuntimeError> {
    // Only the access section is written, so ownership is left untouched.
    const SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
        $user = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value; \
        $acl = New-Object Security.AccessControl.DirectorySecurity; \
        $acl.SetSecurityDescriptorSddlForm('D:P(A;OICI;FA;;;' + $user + ')(A;OICI;FA;;;SY)', 'Access'); \
        (Get-Item -Force -LiteralPath $env:MEERKAT_PRIVATE_DIRECTORY).SetAccessControl($acl)";
    let restricted = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("MEERKAT_PRIVATE_DIRECTORY", path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if !restricted.success() {
        return Err(DaemonRuntimeError::UnsafeDirectory);
    }
    Ok(())
}

#[derive(Debug)]
pub struct InstanceGuard {
    path: PathBuf,
    token: String,
    _file: Option<File>,
}

impl InstanceGuard {
    pub fn acquire(
        run_directory: &Path,
        lock_path: &Path,
        run_id: &str,
    ) -> Result<Self, DaemonRuntimeError> {
        create_private_directory(run_directory)?;
        if run_id.is_empty() || lock_path.parent() != Some(run_directory) {
            return Err(DaemonRuntimeError::InvalidInstanceLock);
        }
        let token = format!("pid={} run_id={run_id}\n", std::process::id());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(PRIVATE_FILE_MODE);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Allow other processes to read the lock token while preventing concurrent
            // deletion or overwrite until the owning process drops the handle or exits.
            const FILE_SHARE_READ: u32 = 0x0000_0001;
            options.share_mode(FILE_SHARE_READ);
        }
        let mut file = loop {
            match options.open(lock_path) {
                Ok(file) => break file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    if !reclaim_stale_instance_lock(run_directory, lock_path)? {
                        return Err(DaemonRuntimeError::AlreadyRunning);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        };
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        Ok(Self {
            path: lock_path.to_path_buf(),
            token,
            _file: Some(file),
        })
    }
}

fn reclaim_stale_instance_lock(
    run_directory: &Path,
    lock_path: &Path,
) -> Result<bool, DaemonRuntimeError> {
    if lock_path.parent() != Some(run_directory) {
        return Err(DaemonRuntimeError::InvalidInstanceLock);
    }
    let metadata = match fs::symlink_metadata(lock_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        return Err(DaemonRuntimeError::UnsafeInstanceLock);
    }
    #[cfg(unix)]
    if metadata.uid() != fs::metadata(run_directory)?.uid()
        || metadata.permissions().mode() & 0o777 != PRIVATE_FILE_MODE
    {
        return Err(DaemonRuntimeError::UnsafeInstanceLock);
    }

    let contents = match fs::read_to_string(lock_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    let pid = instance_lock_pid(&contents)?;
    if !matches!(instance_process_is_alive(pid)?, Some(false)) {
        return Ok(false);
    }
    if fs::read_to_string(lock_path).ok().as_deref() != Some(contents.as_str()) {
        return Ok(false);
    }
    match fs::remove_file(lock_path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(32) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// The process recorded in an instance lock, if the lock is present and valid.
pub fn instance_owner(lock_path: &Path) -> Option<u32> {
    instance_lock_pid(&fs::read_to_string(lock_path).ok()?).ok()
}

fn instance_lock_pid(contents: &str) -> Result<u32, DaemonRuntimeError> {
    let mut pid = None;
    let mut run_id = None;
    let fields: Vec<_> = contents.split_whitespace().collect();
    if fields.len() != 2 {
        return Err(DaemonRuntimeError::InvalidInstanceLock);
    }
    for field in fields {
        if let Some(value) = field.strip_prefix("pid=") {
            pid = Some(
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or(DaemonRuntimeError::InvalidInstanceLock)?,
            );
        } else if let Some(value) = field.strip_prefix("run_id=") {
            if value.is_empty() {
                return Err(DaemonRuntimeError::InvalidInstanceLock);
            }
            run_id = Some(value);
        } else {
            return Err(DaemonRuntimeError::InvalidInstanceLock);
        }
    }
    if run_id.is_none() {
        return Err(DaemonRuntimeError::InvalidInstanceLock);
    }
    pid.ok_or(DaemonRuntimeError::InvalidInstanceLock)
}

#[cfg(unix)]
fn instance_process_is_alive(pid: u32) -> Result<Option<bool>, DaemonRuntimeError> {
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()?;
    Ok(match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    })
}

#[cfg(windows)]
fn instance_process_is_alive(pid: u32) -> Result<Option<bool>, DaemonRuntimeError> {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let listed = String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""));
    Ok(Some(listed))
}

#[cfg(not(any(unix, windows)))]
fn instance_process_is_alive(_pid: u32) -> Result<Option<bool>, DaemonRuntimeError> {
    Ok(None)
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // Close the lock file handle before deleting so Windows does not refuse removal.
        self._file.take();
        if fs::read_to_string(&self.path).is_ok_and(|contents| contents == self.token) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

mod logging;
pub use logging::init_logging;
