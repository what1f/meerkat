#![forbid(unsafe_code)]

pub use lankide_daemon_runtime::InstanceGuard;
use lankide_daemon_runtime::{DaemonRuntimeError, create_private_directory};
use lankide_protocol::{DEFAULT_MAX_MESSAGE_BYTES, PROTOCOL_VERSION, TransportConfig, serve_uds};
use lankide_provider::{HostServices, Provider, ProviderError, ProviderHost, ProviderInfo};
use lankide_runtime::{
    ProviderFactoryFuture, ProviderFactoryRegistry, ProviderOnboarding, ProviderTypeFactory,
    RuntimeApplication, RuntimeConfig, RuntimePeerAdapter,
};
use lankide_storage::DatabaseConfig;
use serde::Serialize;
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub home: PathBuf,
    pub workspaces: PathBuf,
    pub run: PathBuf,
    pub socket: PathBuf,
    pub database: PathBuf,
    pub instance_lock: PathBuf,
}

impl RuntimePaths {
    pub fn for_home(home: impl Into<PathBuf>) -> Result<Self, AppError> {
        let home = home.into();
        if !home.is_absolute() {
            return Err(AppError::InvalidHome("runtime home must be absolute"));
        }
        Ok(Self {
            workspaces: home.join("workspaces"),
            run: home.join("run"),
            socket: home.join("run/runtime.sock"),
            database: home.join("runtime.db"),
            instance_lock: home.join("run/runtime.lock"),
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

    pub fn workspace(&self, workspace_id: &str) -> Result<PathBuf, AppError> {
        if workspace_id.is_empty()
            || workspace_id == "."
            || workspace_id == ".."
            || workspace_id.contains('/')
            || workspace_id.contains('\\')
        {
            return Err(AppError::InvalidWorkspaceId);
        }
        Ok(self.workspaces.join(workspace_id))
    }
}

/// Refuses symlinks and non-socket entries. A stale socket is removed only
/// when it has the same owner as its private parent directory.
pub fn prepare_socket_path(path: &Path) -> Result<(), AppError> {
    let parent = path.parent().ok_or(AppError::InvalidSocketPath)?;
    create_private_directory(parent)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    #[cfg(unix)]
    {
        if !metadata.file_type().is_socket() {
            return Err(AppError::UnsafeSocketEntry);
        }
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(AppError::AlreadyRunning);
        }
        if metadata.uid() != fs::metadata(parent)?.uid() {
            return Err(AppError::UnsafeSocketEntry);
        }
    }
    #[cfg(not(unix))]
    return Err(AppError::UnsupportedPlatform);
    fs::remove_file(path)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct Readiness<'a> {
    pub event: &'static str,
    pub status: &'static str,
    pub run_id: &'a str,
    pub runtime_instance_id: &'a str,
    pub socket_path: &'a Path,
    pub management_ready: bool,
    pub computer_connected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    Internal,
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

pub async fn run_daemon<F>(
    paths: RuntimePaths,
    factories: ProviderFactoryRegistry,
    shutdown: F,
) -> Result<(), AppError>
where
    F: Future<Output = Result<ShutdownReason, AppError>> + Send + 'static,
{
    let run_id = unique_run_id();
    paths.prepare()?;
    let _guard = match InstanceGuard::acquire(&paths.run, &paths.instance_lock, &run_id) {
        Ok(guard) => guard,
        Err(DaemonRuntimeError::AlreadyRunning) => {
            // A live lock owner is not sufficient: prove it serves management
            // before treating this launch as a successful no-op.
            tokio::time::timeout(Duration::from_secs(2), async {
                let mut client =
                    lankide_protocol::connect_uds(&paths.socket, DEFAULT_MAX_MESSAGE_BYTES)
                        .await
                        .map_err(AppError::Protocol)?;
                client
                    .list_workspaces(lankide_protocol::ListWorkspacesRequest {
                        request_id: run_id.clone(),
                    })
                    .await
                    .map_err(AppError::Status)?;
                Ok::<_, AppError>(())
            })
            .await
            .map_err(|_| AppError::AlreadyRunning)??;
            println!("runtime_daemon_already_running");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    prepare_socket_path(&paths.socket)?;
    let application = RuntimeApplication::open(
        RuntimeConfig {
            database: DatabaseConfig::new(&paths.database),
            retry: match fs::read(paths.home.join("retry-policy.json")) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Default::default(),
                Err(error) => return Err(error.into()),
            },
        },
        factories,
    )
    .await?;
    let _ = application.recover(now_millis())?;

    // The management plane only depends on local storage and recovery. Publish
    // it before resuming external Provider connections so the Desktop can
    // render persisted data while the Runtime restores background work.
    let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel();
    let socket = paths.socket.clone();
    let mut server = tokio::spawn(serve_uds(
        socket.clone(),
        RuntimePeerAdapter::new(application.clone()),
        TransportConfig {
            protocol_version: PROTOCOL_VERSION,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        },
        async move {
            let _ = server_shutdown_rx.await;
        },
    ));
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        if server.is_finished() {
            return server
                .await
                .map_err(|_| AppError::Invariant("protocol server task failed"))?
                .map_err(AppError::Protocol);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !socket.exists() {
        return Err(AppError::Invariant("runtime socket was not ready"));
    }
    let readiness = application.readiness();
    println!(
        "{}",
        serde_json::to_string(&Readiness {
            event: "ready",
            status: readiness.state,
            run_id: &run_id,
            runtime_instance_id: &run_id,
            socket_path: &paths.socket,
            management_ready: readiness.management_ready,
            computer_connected: readiness.computer_connected,
        })?
    );
    io::stdout().flush()?;

    // Active Providers must keep running for background message reception, but
    // their network startup must not hold the management socket hostage.
    let provider_start = {
        let application = application.clone();
        tokio::spawn(async move {
            if let Err(error) = application.start().await {
                eprintln!("runtime background Provider startup failed: {error}");
            }
        })
    };
    let _reason = shutdown.await?;
    provider_start.abort();
    let _ = provider_start.await;
    application.begin_shutdown().await?;
    let _ = server_shutdown_tx.send(());
    match tokio::time::timeout(Duration::from_secs(1), &mut server).await {
        Ok(result) => result
            .map_err(|_| AppError::Invariant("protocol server task failed"))?
            .map_err(AppError::Protocol)?,
        Err(_) => {
            server.abort();
            let _ = server.await;
            let _ = std::fs::remove_file(&paths.socket);
        }
    }
    application.finish_shutdown().await?;
    Ok(())
}

pub fn production_factories() -> Result<ProviderFactoryRegistry, AppError> {
    let factories = ProviderFactoryRegistry::default();
    let feishu = lankide_provider_feishu::FeishuProvider::new();
    factories.register(Arc::new(ProductionProviderFactory::new(
        feishu.info(),
        lankide_provider_feishu::FeishuProvider::capabilities(),
        |workspace, services| {
            ProviderHost::new(
                lankide_provider_feishu::FeishuProvider::new(),
                workspace.id.clone(),
                workspace.provider_config.clone(),
                services,
            )
            .map(|host| Arc::new(host) as Arc<dyn lankide_provider::ProviderHostInstance>)
        },
    )))?;
    let telegram = lankide_provider_telegram::TelegramProvider::new();
    factories.register(Arc::new(ProductionProviderFactory::new(
        telegram.info(),
        lankide_provider_telegram::TelegramProvider::capabilities(),
        |workspace, services| {
            ProviderHost::new(
                lankide_provider_telegram::TelegramProvider::new(),
                workspace.id.clone(),
                workspace.provider_config.clone(),
                services,
            )
            .map(|host| Arc::new(host) as Arc<dyn lankide_provider::ProviderHostInstance>)
        },
    )))?;
    Ok(factories)
}

type ProductionCreate = dyn Fn(
        &lankide_storage::WorkspaceRecord,
        HostServices,
    ) -> Result<Arc<dyn lankide_provider::ProviderHostInstance>, ProviderError>
    + Send
    + Sync;
struct ProductionProviderFactory {
    info: ProviderInfo,
    capabilities: Vec<String>,
    create: Arc<ProductionCreate>,
}

impl ProductionProviderFactory {
    fn new<F>(info: ProviderInfo, capabilities: &'static [&'static str], create: F) -> Self
    where
        F: Fn(
                &lankide_storage::WorkspaceRecord,
                HostServices,
            )
                -> Result<Arc<dyn lankide_provider::ProviderHostInstance>, ProviderError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            info,
            capabilities: capabilities
                .iter()
                .map(|capability| (*capability).to_owned())
                .collect(),
            create: Arc::new(create),
        }
    }
}

impl ProviderTypeFactory for ProductionProviderFactory {
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }

    fn create(
        &self,
        workspace: &lankide_storage::WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn lankide_provider::ProviderHostInstance>, ProviderError> {
        (self.create)(workspace, services)
    }

    fn workspace_onboarding(
        &self,
        _action: String,
        opaque_payload: serde_json::Value,
        _services: HostServices,
    ) -> ProviderFactoryFuture<'_, ProviderOnboarding> {
        Box::pin(async move { Ok(production_onboarding(opaque_payload)) })
    }
}

fn production_onboarding(opaque_payload: serde_json::Value) -> ProviderOnboarding {
    ProviderOnboarding::ready(opaque_payload)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn unique_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    format!("runtime-daemon-{}-{nanos}", std::process::id())
}

#[derive(Debug)]
pub enum AppError {
    Io(io::Error),
    Json(serde_json::Error),
    Runtime(lankide_runtime::RuntimeError),
    Storage(lankide_storage::StorageError),
    Protocol(lankide_protocol::TransportError),
    Status(tonic::Status),
    MissingHome,
    InvalidHome(&'static str),
    InvalidWorkspaceId,
    InvalidSocketPath,
    InvalidInstanceLock,
    UnsafeDirectory,
    UnsafeSocketEntry,
    UnsafeInstanceLock,
    AlreadyRunning,
    UnsupportedPlatform,
    Invariant(&'static str),
    CompositionPending,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "runtime_daemon_io: {error}"),
            Self::Json(error) => write!(formatter, "runtime_daemon_json: {error}"),
            Self::Runtime(error) => write!(formatter, "runtime_daemon_runtime: {error}"),
            Self::Storage(error) => write!(formatter, "runtime_daemon_storage: {error}"),
            Self::Protocol(error) => write!(formatter, "runtime_daemon_protocol: {error}"),
            Self::Status(error) => write!(formatter, "runtime_daemon_peer: {error}"),
            Self::MissingHome => formatter.write_str("runtime_daemon_home_unavailable"),
            Self::InvalidHome(message) => {
                write!(formatter, "runtime_daemon_invalid_home: {message}")
            }
            Self::InvalidWorkspaceId => formatter.write_str("runtime_daemon_invalid_workspace_id"),
            Self::InvalidSocketPath => formatter.write_str("runtime_daemon_invalid_socket_path"),
            Self::InvalidInstanceLock => {
                formatter.write_str("runtime_daemon_invalid_instance_lock")
            }
            Self::UnsafeDirectory => formatter.write_str("runtime_daemon_unsafe_directory"),
            Self::UnsafeSocketEntry => formatter.write_str("runtime_daemon_unsafe_socket_entry"),
            Self::UnsafeInstanceLock => formatter.write_str("runtime_daemon_unsafe_instance_lock"),
            Self::AlreadyRunning => formatter.write_str("runtime_daemon_already_running"),
            Self::UnsupportedPlatform => formatter.write_str("runtime_daemon_unsupported_platform"),
            Self::Invariant(message) => write!(formatter, "runtime_daemon_invariant: {message}"),
            Self::CompositionPending => formatter.write_str("runtime_daemon_composition_pending"),
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

impl From<lankide_runtime::RuntimeError> for AppError {
    fn from(value: lankide_runtime::RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

impl From<lankide_storage::StorageError> for AppError {
    fn from(value: lankide_storage::StorageError) -> Self {
        Self::Storage(value)
    }
}
