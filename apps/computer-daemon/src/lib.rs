#![forbid(unsafe_code)]

use lankide_computer::{Computer, ComputerConfig, ReconnectPolicy, RegisteredHarness};
pub use lankide_daemon_runtime::InstanceGuard;
use lankide_daemon_runtime::{DaemonRuntimeError, create_private_directory};
use lankide_harness::{HarnessConfig, ModelRef};
use lankide_harness_claude_code::ClaudeCodeHarness;
use lankide_harness_codex::CodexHarness;
use lankide_harness_pi::PiHarness;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputerDaemonPaths {
    pub home: PathBuf,
    pub workspaces: PathBuf,
    pub run: PathBuf,
    pub runtime_socket: PathBuf,
    pub instance_lock: PathBuf,
}

impl ComputerDaemonPaths {
    pub fn for_home(home: impl Into<PathBuf>) -> Result<Self, AppError> {
        let home = home.into();
        if !home.is_absolute() {
            return Err(AppError::InvalidHome);
        }
        Ok(Self {
            workspaces: home.join("workspaces"),
            run: home.join("run"),
            runtime_socket: home.join("run/runtime.sock"),
            instance_lock: home.join("run/computer.lock"),
            home,
        })
    }

    pub fn local_default() -> Result<Self, AppError> {
        let user_home = std::env::var_os("HOME").ok_or(AppError::MissingHome)?;
        Self::for_home(PathBuf::from(user_home).join(".lankide"))
    }

    pub fn prepare(&self) -> Result<(), AppError> {
        for directory in [&self.home, &self.workspaces, &self.run] {
            create_private_directory(directory)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonConfig {
    pub execution_liveness: lankide_computer::ExecutionLiveness,
    pub paths: ComputerDaemonPaths,
    pub instance_id: String,
    pub runtime_url: Option<String>,
    pub link_secret_file: Option<PathBuf>,
    pub pi_cli: PathBuf,
    pub codex_cli: PathBuf,
    pub claude_code_cli: PathBuf,
    environment: BTreeMap<String, String>,
}

impl DaemonConfig {
    pub fn local_default() -> Result<Self, AppError> {
        Ok(Self {
            execution_liveness: read_execution_liveness(
                &ComputerDaemonPaths::local_default()?.home,
            )?,
            paths: ComputerDaemonPaths::local_default()?,
            instance_id: unique_run_id(),
            runtime_url: None,
            link_secret_file: None,
            pi_cli: PathBuf::from("pi"),
            codex_cli: PathBuf::from("codex"),
            claude_code_cli: PathBuf::from("claude"),
            environment: BTreeMap::new(),
        })
    }

    pub fn for_home(home: impl Into<PathBuf>) -> Result<Self, AppError> {
        let paths = ComputerDaemonPaths::for_home(home)?;
        Ok(Self {
            execution_liveness: read_execution_liveness(&paths.home)?,
            paths,
            instance_id: unique_run_id(),
            runtime_url: None,
            link_secret_file: None,
            pi_cli: PathBuf::from("pi"),
            codex_cli: PathBuf::from("codex"),
            claude_code_cli: PathBuf::from("claude"),
            environment: BTreeMap::new(),
        })
    }

    pub fn set_executable_path(&mut self, path: impl Into<String>) {
        let path = path.into();
        self.pi_cli = resolve_executable(&self.pi_cli, &path);
        self.codex_cli = resolve_executable(&self.codex_cli, &path);
        self.claude_code_cli = resolve_executable(&self.claude_code_cli, &path);
        self.environment.insert("PATH".into(), path);
    }
}

// Local Computer settings; absent fields retain production defaults.
fn read_execution_liveness(
    home: &std::path::Path,
) -> Result<lankide_computer::ExecutionLiveness, AppError> {
    #[derive(serde::Deserialize, Default)]
    struct Settings {
        #[serde(default)]
        execution_liveness: lankide_computer::ExecutionLiveness,
    }
    match std::fs::read(home.join("computer.json")) {
        Ok(bytes) => Ok(serde_json::from_slice::<Settings>(&bytes)?.execution_liveness),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

fn read_reconnect(home: &std::path::Path) -> Result<ReconnectPolicy, AppError> {
    #[derive(serde::Deserialize, Default)]
    struct Settings {
        #[serde(default)]
        reconnect: ReconnectPolicy,
    }
    match std::fs::read(home.join("computer.json")) {
        Ok(bytes) => Ok(serde_json::from_slice::<Settings>(&bytes)?.reconnect),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

fn resolve_executable(executable: &std::path::Path, path: &str) -> PathBuf {
    if executable.is_absolute() {
        return executable.to_owned();
    }
    std::env::split_paths(path)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| executable.to_owned())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    Sigterm,
    Sigint,
}

pub async fn wait_for_shutdown() -> Result<ShutdownReason, AppError> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            _ = terminate.recv() => Ok(ShutdownReason::Sigterm),
            result = tokio::signal::ctrl_c() => {
                result?;
                Ok(ShutdownReason::Sigint)
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(ShutdownReason::Sigint)
    }
}

pub async fn run(config: DaemonConfig) -> Result<(), AppError> {
    let registrations = production_harnesses(&config);
    run_with_harnesses(config, registrations).await
}

pub async fn run_with_harnesses(
    config: DaemonConfig,
    registrations: Vec<RegisteredHarness>,
) -> Result<(), AppError> {
    let run_id = unique_run_id();
    let _guard = InstanceGuard::acquire(&config.paths.run, &config.paths.instance_lock, &run_id)?;
    let mut computer = Computer::new(ComputerConfig {
        computer_id: "local".into(),
        instance_id: config.instance_id.clone(),
        workspace_root: config.paths.workspaces.display().to_string(),
        reconnect: read_reconnect(&config.paths.home)?,
    })
    .with_execution_liveness(config.execution_liveness);
    if let Some(url) = &config.runtime_url {
        let secret_file = config
            .link_secret_file
            .clone()
            .unwrap_or_else(|| config.paths.run.join("computer-link.secret"));
        let secret = std::fs::read_to_string(secret_file)?;
        computer = computer.with_link_endpoint(url.clone(), secret.trim().to_owned());
    }
    for registration in registrations {
        computer
            .register_harness(registration)
            .map_err(AppError::Computer)?;
    }
    computer
        .run(&config.paths.runtime_socket, async {
            let _ = wait_for_shutdown().await;
        })
        .await
        .map_err(AppError::Computer)
}

pub fn production_harnesses(config: &DaemonConfig) -> Vec<RegisteredHarness> {
    vec![
        RegisteredHarness::new(
            PiHarness::new(),
            harness_config("pi", config.pi_cli.clone(), &config.environment),
        ),
        RegisteredHarness::new(
            CodexHarness::new(),
            harness_config("codex", config.codex_cli.clone(), &config.environment),
        ),
        RegisteredHarness::new(
            ClaudeCodeHarness::new(),
            harness_config(
                "claude_code",
                config.claude_code_cli.clone(),
                &config.environment,
            ),
        ),
    ]
}

fn harness_config(
    harness_id: &str,
    cli_path: PathBuf,
    environment: &BTreeMap<String, String>,
) -> HarnessConfig {
    HarnessConfig {
        harness_id: harness_id.into(),
        model: ModelRef::new("", ""),
        thinking_level: String::new(),
        cli_path,
        environment: environment.clone(),
        config_fingerprint: format!("local-{harness_id}"),
    }
}

fn unique_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    format!("computer-daemon-{}-{nanos}", std::process::id())
}

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    Json(serde_json::Error),
    Computer(lankide_computer::ComputerError),
    ComputerHarness(lankide_harness::HarnessError),
    MissingHome,
    InvalidHome,
    InvalidInstanceLock,
    UnsafeDirectory,
    UnsafeInstanceLock,
    AlreadyRunning,
    InvalidArgument(&'static str),
    Invariant(&'static str),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "computer_daemon_io: {error}"),
            Self::Json(error) => write!(formatter, "computer_daemon_json: {error}"),
            Self::Computer(error) => write!(formatter, "computer_daemon_computer: {error}"),
            Self::ComputerHarness(error) => write!(formatter, "computer_daemon_harness: {error}"),
            Self::MissingHome => formatter.write_str("computer_daemon_home_unavailable"),
            Self::InvalidHome => formatter.write_str("computer_daemon_home_must_be_absolute"),
            Self::InvalidInstanceLock => {
                formatter.write_str("computer_daemon_invalid_instance_lock")
            }
            Self::UnsafeDirectory => formatter.write_str("computer_daemon_unsafe_directory"),
            Self::UnsafeInstanceLock => formatter.write_str("computer_daemon_unsafe_instance_lock"),
            Self::AlreadyRunning => formatter.write_str("computer_daemon_already_running"),
            Self::InvalidArgument(argument) => {
                write!(formatter, "computer_daemon_invalid_argument: {argument}")
            }
            Self::Invariant(message) => write!(formatter, "computer_daemon_invariant: {message}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<io::Error> for AppError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<DaemonRuntimeError> for AppError {
    fn from(value: DaemonRuntimeError) -> Self {
        match value {
            DaemonRuntimeError::Io(error) => Self::Io(error),
            DaemonRuntimeError::InvalidInstanceLock => Self::InvalidInstanceLock,
            DaemonRuntimeError::UnsafeDirectory => Self::UnsafeDirectory,
            DaemonRuntimeError::UnsafeInstanceLock => Self::UnsafeInstanceLock,
            DaemonRuntimeError::AlreadyRunning => Self::AlreadyRunning,
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
