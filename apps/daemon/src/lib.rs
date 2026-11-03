#![forbid(unsafe_code)]

mod remote_access;

pub use meerkat_daemon_runtime::InstanceGuard;
use meerkat_daemon_runtime::{DaemonRuntimeError, create_private_directory};

use meerkat_provider::{HostServices, Provider, ProviderError, ProviderHost, ProviderInfo};
use meerkat_runtime::{
    ProviderFactoryFuture, ProviderFactoryRegistry, ProviderOnboarding, ProviderTypeFactory,
    RuntimeApplication, RuntimeConfig, RuntimePeerAdapter,
};
use meerkat_storage::DatabaseConfig;
use serde::Serialize;
use std::fs;
use std::future::Future;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub home: PathBuf,
    pub workspaces: PathBuf,
    pub run: PathBuf,
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
            database: home.join("runtime.db"),
            instance_lock: home.join("run/runtime.lock"),
            home,
        })
    }

    pub fn local_default() -> Result<Self, AppError> {
        let user_home = std::env::var_os("HOME").ok_or(AppError::MissingHome)?;
        Self::for_home(PathBuf::from(user_home).join(".meerkat"))
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

#[derive(Clone, Debug, Serialize)]
pub struct Readiness<'a> {
    pub event: &'static str,
    pub status: &'static str,
    pub run_id: &'a str,
    pub runtime_instance_id: &'a str,
    pub management_url: &'a str,
    pub management_ready: bool,
    pub machine_connected: bool,
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
                use meerkat_client::ManagementClient;
                let client =
                    meerkat_client::RuntimeManagementClient::connect_local_lazy(&paths.run);
                client
                    .list_workspaces()
                    .await
                    .map_err(|error| AppError::Io(io::Error::other(error.to_string())))?;
                Ok::<_, AppError>(())
            })
            .await
            .map_err(|_| AppError::AlreadyRunning)??;
            println!("runtime_daemon_already_running");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
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
    let _ = application.recover(now_millis()).await?;
    #[derive(serde::Deserialize)]
    struct LinkSettings {
        #[serde(default = "default_link_listen")]
        machine_link_listen: String,
        #[serde(default = "default_remote_access_port")]
        remote_access_port: u16,
        #[serde(default = "default_pairing_validity_ms")]
        pairing_validity_ms: u64,
        #[serde(default = "default_token_lifetime_ms")]
        execution_token_lifetime_ms: u64,
        #[serde(default = "default_attempt_lease_ms")]
        attempt_lease_ms: u64,
        #[serde(default)]
        machine_link: meerkat_protocol::link::LinkSettings,
    }
    fn default_remote_access_port() -> u16 {
        7443
    }
    fn default_pairing_validity_ms() -> u64 {
        600_000
    }
    fn default_link_listen() -> String {
        "127.0.0.1:7440".into()
    }
    fn default_attempt_lease_ms() -> u64 {
        15_000
    }
    fn default_token_lifetime_ms() -> u64 {
        3_600_000
    }
    let settings = match fs::read(paths.home.join("runtime.json")) {
        Ok(bytes) => serde_json::from_slice::<LinkSettings>(&bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => LinkSettings {
            machine_link_listen: default_link_listen(),
            remote_access_port: default_remote_access_port(),
            pairing_validity_ms: default_pairing_validity_ms(),
            execution_token_lifetime_ms: default_token_lifetime_ms(),
            attempt_lease_ms: default_attempt_lease_ms(),
            machine_link: Default::default(),
        },
        Err(error) => return Err(error.into()),
    };
    application.set_execution_token_lifetime(Duration::from_millis(
        settings.execution_token_lifetime_ms,
    ))?;
    application
        .machine_broker()
        .set_attempt_lease(Duration::from_millis(settings.attempt_lease_ms))?;
    application.set_pairing_validity(Duration::from_millis(settings.pairing_validity_ms))?;
    let address = settings.machine_link_listen;
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .map_err(|error| {
            io::Error::new(error.kind(), format!("cannot listen on {address}: {error}"))
        })?;
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(AppError::InvalidHome("local listener must use loopback"));
    }
    let url = format!("ws://{}", listener.local_addr()?);
    let secret = meerkat_protocol::link::publish(&paths.run, &url)?;
    let mut management_address = listener.local_addr()?;
    if management_address.ip().is_unspecified() {
        management_address.set_ip(match management_address {
            std::net::SocketAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
            std::net::SocketAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
        });
    }
    fs::write(
        paths.run.join(meerkat_protocol::management::URL_FILE),
        format!("http://{management_address}"),
    )?;
    let remote = remote_access::RemoteMachines::open(
        paths.home.clone(),
        RuntimePeerAdapter::new(application.clone()),
        secret.clone(),
        settings.machine_link.clone(),
        settings.remote_access_port,
    )
    .await
    .map_err(AppError::Status)?;
    let (link_stop, link_stopped) = oneshot::channel();
    let link_server = tokio::spawn(meerkat_protocol::resources::serve_configured(
        listener,
        secret,
        RuntimePeerAdapter::new(application.clone()),
        settings.machine_link,
        Some(remote.clone()),
        None,
        Some(std::sync::Arc::new(meerkat_web_assets::serve)),
        async {
            let _ = link_stopped.await;
        },
    ));

    let readiness = application.readiness();
    println!(
        "{}",
        serde_json::to_string(&Readiness {
            event: "ready",
            status: readiness.state,
            run_id: &run_id,
            runtime_instance_id: &run_id,
            management_url: &format!("http://{management_address}"),
            management_ready: readiness.management_ready,
            machine_connected: readiness.machine_connected,
        })?
    );
    io::stdout().flush()?;

    // Active Providers must keep running for background message reception, but
    // their network startup must not delay management readiness.
    let provider_start = {
        let application = application.clone();
        tokio::spawn(async move {
            if let Err(error) = application.start().await {
                log::error!("runtime background Provider startup failed: {error}");
            }
        })
    };
    let _reason = shutdown.await?;
    provider_start.abort();
    let _ = provider_start.await;
    remote.stop().await;
    application.begin_shutdown().await?;
    let _ = link_stop.send(());
    link_server
        .await
        .map_err(|_| AppError::Invariant("machine link server task failed"))?
        .map_err(|error| AppError::Io(io::Error::other(error.to_string())))?;
    for name in [
        "machine-link.url",
        "machine-link.secret",
        meerkat_protocol::management::URL_FILE,
    ] {
        let _ = fs::remove_file(paths.run.join(name));
    }
    application.finish_shutdown().await?;
    Ok(())
}

pub fn production_factories() -> Result<ProviderFactoryRegistry, AppError> {
    let factories = ProviderFactoryRegistry::default();
    let feishu = meerkat_provider_feishu::FeishuProvider::new();
    factories.register(Arc::new(ProductionProviderFactory::new(
        feishu.info(),
        meerkat_provider_feishu::FeishuProvider::capabilities(),
        |workspace, services| {
            ProviderHost::new(
                meerkat_provider_feishu::FeishuProvider::new(),
                workspace.id.clone(),
                workspace.provider_config.clone(),
                services,
            )
            .map(|host| Arc::new(host) as Arc<dyn meerkat_provider::ProviderHostInstance>)
        },
    )))?;
    let telegram = meerkat_provider_telegram::TelegramProvider::new();
    factories.register(Arc::new(ProductionProviderFactory::new(
        telegram.info(),
        meerkat_provider_telegram::TelegramProvider::capabilities(),
        |workspace, services| {
            ProviderHost::new(
                meerkat_provider_telegram::TelegramProvider::new(),
                workspace.id.clone(),
                workspace.provider_config.clone(),
                services,
            )
            .map(|host| Arc::new(host) as Arc<dyn meerkat_provider::ProviderHostInstance>)
        },
    )))?;
    Ok(factories)
}

type ProductionCreate = dyn Fn(
        &meerkat_storage::WorkspaceRecord,
        HostServices,
    ) -> Result<Arc<dyn meerkat_provider::ProviderHostInstance>, ProviderError>
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
                &meerkat_storage::WorkspaceRecord,
                HostServices,
            )
                -> Result<Arc<dyn meerkat_provider::ProviderHostInstance>, ProviderError>
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
        workspace: &meerkat_storage::WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn meerkat_provider::ProviderHostInstance>, ProviderError> {
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
    Runtime(meerkat_runtime::RuntimeError),
    Storage(meerkat_storage::StorageError),
    Protocol(meerkat_protocol::TransportError),
    Status(meerkat_protocol::Status),
    MissingHome,
    InvalidHome(&'static str),
    InvalidWorkspaceId,
    InvalidInstanceLock,
    UnsafeDirectory,
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
            Self::InvalidInstanceLock => {
                formatter.write_str("runtime_daemon_invalid_instance_lock")
            }
            Self::UnsafeDirectory => formatter.write_str("runtime_daemon_unsafe_directory"),
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

impl From<meerkat_runtime::RuntimeError> for AppError {
    fn from(value: meerkat_runtime::RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

impl From<meerkat_storage::StorageError> for AppError {
    fn from(value: meerkat_storage::StorageError) -> Self {
        Self::Storage(value)
    }
}
