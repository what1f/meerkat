#![forbid(unsafe_code)]

//! Shared filesystem lifecycle for local Lankide daemons.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;

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
    Ok(())
}

#[derive(Debug)]
pub struct InstanceGuard {
    path: PathBuf,
    token: String,
    _file: File,
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
            _file: file,
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
    #[cfg(unix)]
    if !metadata.file_type().is_file()
        || metadata.uid() != fs::metadata(run_directory)?.uid()
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
        Err(error) => Err(error.into()),
    }
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

#[cfg(not(unix))]
fn instance_process_is_alive(_pid: u32) -> Result<Option<bool>, DaemonRuntimeError> {
    Ok(None)
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|contents| contents == self.token) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
