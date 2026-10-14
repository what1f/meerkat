use crate::background_services::BackgroundServices;
use lankide_client::{ClientError, ManagementClient, RuntimeManagementClient};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

const HOME_ENV: &str = "LANKIDE_HOME";
const SOCKET_ENV: &str = "LANKIDE_RUNTIME_SOCKET";

pub struct DesktopBackend {
    pub client: Arc<dyn ManagementClient>,
    pub startup_error: Option<ClientError>,
    startup_result: Option<tokio::sync::oneshot::Receiver<Result<(), ClientError>>>,
    _tokio: tokio::runtime::Runtime,
    _services: Arc<Mutex<BackgroundServices>>,
}

impl DesktopBackend {
    pub fn launch() -> Self {
        let tokio = tokio::runtime::Runtime::new().expect("failed to create desktop Tokio runtime");
        let home = lankide_home();
        let configured_socket = env::var_os(SOCKET_ENV);
        let socket = configured_socket
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("run/runtime.sock"));
        let services = Arc::new(Mutex::new(BackgroundServices::new(
            configured_socket.is_none(),
        )));
        let client = {
            let _guard = tokio.enter();
            RuntimeManagementClient::connect_lazy(&socket)
        };
        let (startup_tx, startup_result) = tokio::sync::oneshot::channel();

        let worker_services = services.clone();
        let worker_home = home.clone();
        let worker_socket = socket.clone();
        let runtime = tokio.handle().clone();
        let manage_services = configured_socket.is_none();
        std::thread::Builder::new()
            .name("lankide-service-bootstrap".into())
            .spawn(move || {
                let result = start_services(
                    &runtime,
                    &worker_services,
                    &worker_home,
                    &worker_socket,
                    manage_services,
                );
                if let Err(error) = &result {
                    eprintln!("lankide-desktop: {error}");
                }
                let _ = startup_tx.send(result);
            })
            .expect("failed to start service bootstrap thread");

        Self {
            client: Arc::new(client),
            startup_error: None,
            startup_result: Some(startup_result),
            _tokio: tokio,
            _services: services,
        }
    }

    pub fn bootstrap() -> Self {
        let tokio = tokio::runtime::Runtime::new().expect("failed to create desktop Tokio runtime");
        let home = lankide_home();
        let configured_socket = env::var_os(SOCKET_ENV);
        let socket = configured_socket
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("run/runtime.sock"));
        // A private home keeps bundled smoke runs on transient helpers.
        // Unbundled runs use transient helpers even with the default home.
        let services = Arc::new(Mutex::new(BackgroundServices::new(
            env::var_os(HOME_ENV).is_none(),
        )));
        let startup = start_services(
            tokio.handle(),
            &services,
            &home,
            &socket,
            configured_socket.is_none(),
        );
        let client =
            startup.and_then(|_| tokio.block_on(RuntimeManagementClient::connect(&socket)));

        match client {
            Ok(client) => Self {
                client: Arc::new(client),
                startup_error: None,
                startup_result: None,
                _tokio: tokio,
                _services: services,
            },
            Err(error) => {
                // Computer can wait for a Runtime that becomes available later.
                let client = {
                    let _guard = tokio.enter();
                    RuntimeManagementClient::connect_lazy(&socket)
                };
                Self {
                    client: Arc::new(client),
                    startup_error: Some(error),
                    startup_result: None,
                    _tokio: tokio,
                    _services: services,
                }
            }
        }
    }

    pub fn stop_local_services(&self) {
        self._services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop_owned();
    }

    pub fn take_startup_result(
        &mut self,
    ) -> Option<tokio::sync::oneshot::Receiver<Result<(), ClientError>>> {
        self.startup_result.take()
    }

    pub fn enter(&self) -> tokio::runtime::EnterGuard<'_> {
        self._tokio.enter()
    }
}

fn start_services(
    runtime: &tokio::runtime::Handle,
    services: &Arc<Mutex<BackgroundServices>>,
    home: &Path,
    socket: &Path,
    manage_services: bool,
) -> Result<(), ClientError> {
    // Reuse a healthy owner before preparing or launching any local service.
    let client = match runtime.block_on(RuntimeManagementClient::connect(socket)) {
        Ok(client) => client,
        Err(_) if manage_services => {
            {
                let mut services = services
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                services.prepare(home)?;
                services.ensure_runtime(home)?;
            }
            runtime.block_on(connect_with_retry(socket))?
        }
        Err(error) => return Err(error),
    };
    if manage_services {
        let mut services = services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.block_on(ensure_computer_ready(&client, &mut services, home))?;
    }
    Ok(())
}

pub fn run_local_bootstrap_smoke() -> i32 {
    let args = env::args().collect::<Vec<_>>();
    let result_path = args
        .windows(2)
        .find(|pair| pair[0] == "--result")
        .map(|pair| PathBuf::from(&pair[1]));
    let Some(result_path) = result_path else {
        eprintln!("lankide-desktop: --result is required for local bootstrap smoke");
        return 2;
    };
    let backend = DesktopBackend::bootstrap();
    let startup_ready = backend.startup_error.is_none();
    let management = backend
        ._tokio
        .block_on(backend.client.list_workspaces())
        .is_ok();
    let runtime_ready = management;
    let mut computer_connected = false;
    if startup_ready && runtime_ready {
        for _ in 0..60 {
            let catalog = backend
                ._tokio
                .block_on(backend.client.get_computer_catalog(None));
            if catalog.is_ok_and(|items| items.iter().any(|item| item.available)) {
                computer_connected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let status = if startup_ready && runtime_ready && computer_connected && management {
        "passed"
    } else {
        "failed"
    };
    let payload = serde_json::json!({
        "status": status,
        "runtime_ready": runtime_ready,
        "computer_connected": computer_connected,
        "management": management,
    });
    let write_result = result_path
        .parent()
        .map(fs::create_dir_all)
        .transpose()
        .and_then(|_| fs::write(&result_path, format!("{}\n", payload)));
    if let Err(error) = write_result {
        eprintln!("lankide-desktop: smoke result could not be written: {error}");
        return 3;
    }
    backend.stop_local_services();
    drop(backend);
    if status == "passed" { 0 } else { 1 }
}

fn lankide_home() -> PathBuf {
    env::var_os(HOME_ENV).map(PathBuf::from).unwrap_or_else(|| {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".lankide")
    })
}

async fn connect_with_retry(socket: &Path) -> Result<RuntimeManagementClient, ClientError> {
    let mut last_error = ClientError::Unavailable("Lankide 尚未就绪".into());
    for _ in 0..50 {
        match RuntimeManagementClient::connect(socket).await {
            Ok(client) => return Ok(client),
            Err(error) => last_error = error,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(last_error)
}

async fn ensure_computer_ready(
    client: &RuntimeManagementClient,
    services: &mut BackgroundServices,
    home: &Path,
) -> Result<(), ClientError> {
    if client
        .get_computer_catalog(None)
        .await?
        .iter()
        .any(|item| item.available)
    {
        return Ok(());
    }
    services.ensure_computer(home)?;
    for _ in 0..60 {
        if client
            .get_computer_catalog(None)
            .await?
            .iter()
            .any(|item| item.available)
        {
            return Ok(());
        }
        if let Some(status) = services.computer_exited()? {
            return Err(ClientError::Unavailable(format!(
                "本地执行服务已退出（{status}），请查看服务日志后重试。"
            )));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ClientError::Unavailable(
        "本地执行服务尚未就绪，请稍后重试。".into(),
    ))
}
