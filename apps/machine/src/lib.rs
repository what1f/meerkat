#![forbid(unsafe_code)]

pub use meerkat_daemon_runtime::InstanceGuard;
use meerkat_daemon_runtime::{DaemonRuntimeError, create_private_directory};
use meerkat_harness::{HarnessConfig, ModelRef};
use meerkat_harness_builtin::BuiltinHarness;
use meerkat_harness_claude_code::ClaudeCodeHarness;
use meerkat_harness_codex::CodexHarness;
use meerkat_harness_pi::PiHarness;
use meerkat_machine_core::{Machine, MachineConfig, ReconnectPolicy, RegisteredHarness};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineDaemonPaths {
    pub home: PathBuf,
    pub workspaces: PathBuf,
    pub run: PathBuf,
    pub runtime_run: PathBuf,
    pub instance_lock: PathBuf,
}

impl MachineDaemonPaths {
    pub fn for_home(home: impl Into<PathBuf>) -> Result<Self, AppError> {
        let home = home.into();
        if !home.is_absolute() {
            return Err(AppError::InvalidHome);
        }
        Ok(Self {
            workspaces: home.join("workspaces"),
            run: home.join("run"),
            runtime_run: home.join("run"),
            instance_lock: home.join("run/machine.lock"),
            home,
        })
    }

    pub fn local_default() -> Result<Self, AppError> {
        Self::for_home(user_home()?.join(".meerkat"))
    }

    pub fn prepare(&self) -> Result<(), AppError> {
        for directory in [&self.home, &self.workspaces, &self.run] {
            create_private_directory(directory)?;
        }
        Ok(())
    }
}

/// The signed-in user's home directory.
pub fn user_home() -> Result<PathBuf, AppError> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(AppError::MissingHome)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonConfig {
    pub execution_liveness: meerkat_machine_core::ExecutionLiveness,
    pub paths: MachineDaemonPaths,
    pub instance_id: String,
    pub runtime_url: Option<String>,
    pub link_secret_file: Option<PathBuf>,
    pub builtin_agent_cli: PathBuf,
    pub pi_cli: PathBuf,
    pub codex_cli: PathBuf,
    pub claude_code_cli: PathBuf,
    environment: BTreeMap<String, String>,
}

impl DaemonConfig {
    pub fn local_default() -> Result<Self, AppError> {
        Ok(Self {
            execution_liveness: read_execution_liveness(
                &MachineDaemonPaths::local_default()?.home,
            )?,
            paths: MachineDaemonPaths::local_default()?,
            instance_id: unique_run_id(),
            runtime_url: None,
            link_secret_file: None,
            builtin_agent_cli: builtin_agent_cli(),
            pi_cli: PathBuf::from("pi"),
            codex_cli: PathBuf::from("codex"),
            claude_code_cli: PathBuf::from("claude"),
            environment: BTreeMap::new(),
        })
    }

    pub fn for_home(home: impl Into<PathBuf>) -> Result<Self, AppError> {
        let paths = MachineDaemonPaths::for_home(home)?;
        Ok(Self {
            execution_liveness: read_execution_liveness(&paths.home)?,
            paths,
            instance_id: unique_run_id(),
            runtime_url: None,
            link_secret_file: None,
            builtin_agent_cli: builtin_agent_cli(),
            pi_cli: PathBuf::from("pi"),
            codex_cli: PathBuf::from("codex"),
            claude_code_cli: PathBuf::from("claude"),
            environment: BTreeMap::new(),
        })
    }

    pub fn set_executable_path(&mut self, path: impl Into<String>) {
        let path = path.into();
        self.builtin_agent_cli = resolve_executable(&self.builtin_agent_cli, &path);
        self.pi_cli = resolve_executable(&self.pi_cli, &path);
        self.codex_cli = resolve_executable(&self.codex_cli, &path);
        self.claude_code_cli = resolve_executable(&self.claude_code_cli, &path);
        self.environment.insert("PATH".into(), path);
    }
}

// Local Machine settings; absent fields retain production defaults.
fn read_execution_liveness(
    home: &std::path::Path,
) -> Result<meerkat_machine_core::ExecutionLiveness, AppError> {
    #[derive(serde::Deserialize, Default)]
    struct Settings {
        #[serde(default)]
        execution_liveness: meerkat_machine_core::ExecutionLiveness,
    }
    match std::fs::read(home.join("machine.json")) {
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
    match std::fs::read(home.join("machine.json")) {
        Ok(bytes) => Ok(serde_json::from_slice::<Settings>(&bytes)?.reconnect),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.into()),
    }
}

fn builtin_agent_cli() -> PathBuf {
    meerkat_harness_builtin::discover()
        .unwrap_or_else(|| PathBuf::from(meerkat_harness_builtin::EXECUTABLE))
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
    let mut registrations = production_harnesses(&config);
    load_harness_plugins(&config, &mut registrations)?;
    run_with_harnesses(config, registrations).await
}

pub async fn run_with_harnesses(
    config: DaemonConfig,
    registrations: Vec<RegisteredHarness>,
) -> Result<(), AppError> {
    let run_id = unique_run_id();
    let _guard = InstanceGuard::acquire(&config.paths.run, &config.paths.instance_lock, &run_id)?;
    let credentials = match read_credentials(&config.paths.home)? {
        Some(credentials) => credentials,
        None => {
            let (url, secret) = if let Some(url) = &config.runtime_url {
                let path = config
                    .link_secret_file
                    .clone()
                    .unwrap_or_else(|| config.paths.run.join("machine-link.secret"));
                (
                    url.clone(),
                    std::fs::read_to_string(path)?.trim().to_owned(),
                )
            } else {
                tokio::time::timeout(
                    read_reconnect(&config.paths.home)?.handshake_timeout,
                    async {
                        loop {
                            if let Ok(endpoint) =
                                meerkat_protocol::link::discover(&config.paths.runtime_run)
                            {
                                break endpoint;
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    },
                )
                .await
                .map_err(|_| AppError::InvalidArgument("runtime_unavailable"))?
            };
            pair_inner(
                &config.paths,
                &url,
                &secret,
                config.runtime_url.is_none(),
                None,
            )
            .await?
        }
    };
    let mut machine = Machine::new(MachineConfig {
        machine_id: credentials.machine_id,
        instance_id: config.instance_id.clone(),
        workspace_root: config.paths.workspaces.display().to_string(),
        reconnect: read_reconnect(&config.paths.home)?,
    })
    .with_certificate_pin(credentials.cert_sha256.clone())
    .with_device_credential(credentials.device_credential.clone())
    .with_execution_liveness(config.execution_liveness);
    if !credentials.local_discovery || config.runtime_url.is_some() {
        machine = machine.with_link_endpoint(
            config
                .runtime_url
                .clone()
                .unwrap_or(credentials.runtime_url),
            credentials.device_credential,
        );
    }
    for registration in registrations {
        machine
            .register_harness(registration)
            .map_err(AppError::Machine)?;
    }
    let handle = machine
        .spawn(&config.paths.runtime_run)
        .await
        .map_err(AppError::Machine)?;
    let connected_marker = connected_marker(&config.paths);
    let _ = std::fs::remove_file(&connected_marker);
    let shutdown = wait_for_shutdown();
    tokio::pin!(shutdown);
    let mut announced = false;
    loop {
        tokio::select! {
            result = handle.join() => return result.map_err(AppError::Machine),
            _ = &mut shutdown => {
                handle.shutdown("machine_shutdown").await;
                return handle.join().await.map_err(AppError::Machine);
            }
            _ = handle.wait_until_connected(), if !announced => {
                println!("Connected as {}", machine_name());
                std::fs::write(&connected_marker, &run_id)?;
                announced = true;
            }
        }
    }
}

/// Written once the Machine link is authenticated, so a waiting `pair` can see it.
pub fn connected_marker(paths: &MachineDaemonPaths) -> PathBuf {
    paths.run.join("machine-connected")
}

pub fn production_harnesses(config: &DaemonConfig) -> Vec<RegisteredHarness> {
    vec![
        RegisteredHarness::new(
            BuiltinHarness::new(builtin_agent_home(config))
                .with_context_window(builtin_agent_context_window()),
            harness_config(
                meerkat_harness_builtin::HARNESS_ID,
                config.builtin_agent_cli.clone(),
                &config.environment,
            ),
        ),
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

/// `MEERKAT_AGENT_CONTEXT_WINDOW` overrides the Built-in Agent's context
/// window; anything but a positive token count keeps the default.
fn builtin_agent_context_window() -> u32 {
    std::env::var("MEERKAT_AGENT_CONTEXT_WINDOW")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|tokens| *tokens > 0)
        .unwrap_or(meerkat_harness_builtin::DEFAULT_CONTEXT_WINDOW)
}

/// One Built-in Agent home per Machine; members keep their own sessions.
pub fn builtin_agent_home(config: &DaemonConfig) -> PathBuf {
    config.paths.workspaces.join(".machine/builtin-agent")
}

/// Local JS packages may replace a bundled adapter or add a CLI harness.
/// Loading validates declarations only; native discovery is Machine-owned.
pub fn load_harness_plugins(
    config: &DaemonConfig,
    registrations: &mut Vec<RegisteredHarness>,
) -> Result<(), AppError> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Manifest {
        api_version: u32,
        id: String,
        name: String,
        entry: PathBuf,
        cli: PathBuf,
    }
    let directory = config.paths.home.join("harnesses");
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut paths = entries
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|e| e.path())
        .collect::<Vec<_>>();
    paths.sort();
    let mut loaded = std::collections::BTreeSet::new();
    for path in paths {
        if !path.is_dir() {
            continue;
        }
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(path.join("plugin.json"))?)?;
        if manifest.api_version != 1
            || manifest.entry.is_absolute()
            || manifest
                .entry
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
            || !loaded.insert(manifest.id.clone())
        {
            return Err(meerkat_harness::HarnessError::invalid_config(
                "Invalid or duplicate Harness plugin manifest",
            )
            .into());
        }
        let root = path.canonicalize()?;
        let entry = path.join(manifest.entry).canonicalize()?;
        if !entry.starts_with(root) {
            return Err(meerkat_harness::HarnessError::invalid_config(
                "Harness plugin entry escapes its package",
            )
            .into());
        }
        let plugin = meerkat_harness::JsHarnessFactory::new(
            &manifest.id,
            &manifest.name,
            std::fs::read_to_string(entry)?,
        )?;
        let cli = config
            .environment
            .get("PATH")
            .map(|p| resolve_executable(&manifest.cli, p))
            .unwrap_or(manifest.cli);
        registrations.retain(|item| item.factory.info().harness_id != manifest.id);
        registrations.push(RegisteredHarness::new(
            plugin,
            harness_config(&manifest.id, cli, &config.environment),
        ));
    }
    Ok(())
}

fn harness_config(
    harness_id: &str,
    cli_path: PathBuf,
    environment: &BTreeMap<String, String>,
) -> HarnessConfig {
    HarnessConfig {
        launch_arguments: Default::default(),
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
    format!("machine-daemon-{}-{nanos}", std::process::id())
}

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    Json(serde_json::Error),
    Machine(meerkat_machine_core::MachineError),
    MachineHarness(meerkat_harness::HarnessError),
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
            Self::Io(error) => write!(formatter, "machine_daemon_io: {error}"),
            Self::Json(error) => write!(formatter, "machine_daemon_json: {error}"),
            Self::Machine(error) => write!(formatter, "machine_daemon_machine: {error}"),
            Self::MachineHarness(error) => write!(formatter, "machine_daemon_harness: {error}"),
            Self::MissingHome => formatter.write_str("machine_daemon_home_unavailable"),
            Self::InvalidHome => formatter.write_str("machine_daemon_home_must_be_absolute"),
            Self::InvalidInstanceLock => {
                formatter.write_str("machine_daemon_invalid_instance_lock")
            }
            Self::UnsafeDirectory => formatter.write_str("machine_daemon_unsafe_directory"),
            Self::UnsafeInstanceLock => formatter.write_str("machine_daemon_unsafe_instance_lock"),
            Self::AlreadyRunning => formatter.write_str("machine_daemon_already_running"),
            Self::InvalidArgument(argument) => {
                write!(formatter, "machine_daemon_invalid_argument: {argument}")
            }
            Self::Invariant(message) => write!(formatter, "machine_daemon_invariant: {message}"),
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

#[derive(Clone, Debug, serde::Deserialize, Serialize)]
pub struct DeviceCredentials {
    pub machine_id: String,
    pub device_credential: String,
    pub runtime_url: String,
    pub local_discovery: bool,
    pub cert_sha256: Option<String>,
}

pub fn read_credentials(home: &std::path::Path) -> Result<Option<DeviceCredentials>, AppError> {
    match std::fs::read(home.join("machine-credentials.json")) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub const ALREADY_LOCAL: &str =
    "This computer already runs Meerkat, so it is in the workspace already.";

pub async fn pair(
    paths: &MachineDaemonPaths,
    runtime_url: &str,
    code: &str,
) -> Result<DeviceCredentials, AppError> {
    pair_with_pin(paths, runtime_url, code, None).await
}

pub async fn pair_with_pin(
    paths: &MachineDaemonPaths,
    runtime_url: &str,
    code: &str,
    pin: Option<&str>,
) -> Result<DeviceCredentials, AppError> {
    paths.prepare()?;
    let _guard = InstanceGuard::acquire(&paths.run, &paths.instance_lock, &unique_run_id())?;
    // A new code replaces an earlier pairing, but never the Desktop's own Machine.
    if read_credentials(&paths.home)?.is_some_and(|existing| existing.local_discovery) {
        return Err(AppError::InvalidArgument(ALREADY_LOCAL));
    }
    pair_inner(paths, runtime_url, code, false, pin).await
}

async fn pair_inner(
    paths: &MachineDaemonPaths,
    runtime_url: &str,
    code: &str,
    local_discovery: bool,
    pin: Option<&str>,
) -> Result<DeviceCredentials, AppError> {
    use std::io::Write;
    let runtime_url = runtime_url
        .trim_end_matches('/')
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    meerkat_protocol::tls::validate_url(&runtime_url, pin)?;
    let url = runtime_url
        .trim_end_matches('/')
        .replacen("ws://", "http://", 1)
        .replacen("wss://", "https://", 1);
    let mut client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10));
    if let Some(pin) = pin {
        client = client.use_preconfigured_tls(meerkat_protocol::tls::client_config(pin)?);
    }
    let response = client
        .build()
        .map_err(|e| io::Error::other(e.to_string()))?
        .post(format!("{url}/machines/pair"))
        .json(&meerkat_protocol::link::PairMachineRequest {
            code: code.into(),
            display_name: machine_name(),
            os: std::env::consts::OS.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        })
        .send()
        .await
        .map_err(|e| {
            let detail = format!("{e:?}");
            io::Error::other(if detail.contains("pin mismatch") {
                "Certificate pin mismatch. Check the copied command."
            } else {
                "Could not reach this address. Check the address and try again."
            })
        })?;
    if !response.status().is_success() {
        return Err(AppError::InvalidArgument(
            "Pairing code was rejected or expired. Copy a new command.",
        ));
    }
    let paired: meerkat_protocol::link::PairMachineResponse = response
        .json()
        .await
        .map_err(|e| io::Error::other(e.to_string()))?;
    let credentials = DeviceCredentials {
        machine_id: paired.machine_id,
        device_credential: paired.device_credential,
        runtime_url: runtime_url
            .trim_end_matches('/')
            .replacen("http://", "ws://", 1)
            .replacen("https://", "wss://", 1),
        local_discovery,
        cert_sha256: pin.map(str::to_ascii_lowercase),
    };
    let temporary = paths
        .home
        .join(format!(".machine-credentials-{}", unique_run_id()));
    let result = (|| -> Result<(), AppError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(&credentials)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, paths.home.join("machine-credentials.json"))?;
        #[cfg(unix)]
        std::fs::File::open(&paths.home)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result?;
    Ok(credentials)
}

/// Use the same user-visible machine name in local and remote registration.
pub fn machine_name() -> String {
    #[cfg(target_os = "macos")]
    for key in ["MachineName", "LocalHostName"] {
        if let Some(name) = command_name("/usr/sbin/scutil", &["--get", key]) {
            return name;
        }
    }
    command_name("hostname", &[]).unwrap_or_else(|| "Machine".into())
}
fn command_name(command: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(command)
        .args(args)
        .output()
        .ok()?;
    let name = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !name.is_empty()).then_some(name)
}

impl From<meerkat_harness::HarnessError> for AppError {
    fn from(error: meerkat_harness::HarnessError) -> Self {
        Self::MachineHarness(error)
    }
}
