use crate::background_services::BackgroundServices;
use meerkat_client::{ClientError, ManagementClient, RuntimeManagementClient};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

const HOME_ENV: &str = "MEERKAT_HOME";
const RUN_ENV: &str = "MEERKAT_RUNTIME_RUN";

pub struct DesktopBackend {
    pub client: Arc<dyn ManagementClient>,
    pub local_home: PathBuf,
    pub startup_error: Option<ClientError>,
    startup_result: Option<tokio::sync::oneshot::Receiver<Result<(), ClientError>>>,
    _tokio: tokio::runtime::Runtime,
    _services: Arc<Mutex<BackgroundServices>>,
}

impl DesktopBackend {
    pub fn launch() -> Self {
        let tokio = tokio::runtime::Runtime::new().expect("failed to create desktop Tokio runtime");
        let home = meerkat_home();
        let configured_run = env::var_os(RUN_ENV);
        let run = configured_run
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("run"));
        let services = Arc::new(Mutex::new(BackgroundServices::new(
            configured_run.is_none() && env::var_os(HOME_ENV).is_none(),
        )));
        let client = {
            let _guard = tokio.enter();
            RuntimeManagementClient::connect_local_lazy(&run)
        };
        let (startup_tx, startup_result) = tokio::sync::oneshot::channel();

        let worker_services = services.clone();
        let worker_home = home.clone();
        let worker_run = run.clone();
        let runtime = tokio.handle().clone();
        let manage_services = configured_run.is_none();
        std::thread::Builder::new()
            .name("meerkat-service-bootstrap".into())
            .spawn(move || {
                let result = start_services(
                    &runtime,
                    &worker_services,
                    &worker_home,
                    &worker_run,
                    manage_services,
                );
                if let Err(error) = &result {
                    eprintln!("meerkat-desktop: {error}");
                }
                let _ = startup_tx.send(result);
            })
            .expect("failed to start service bootstrap thread");

        Self {
            client: Arc::new(client),
            local_home: home.clone(),
            startup_error: None,
            startup_result: Some(startup_result),
            _tokio: tokio,
            _services: services,
        }
    }

    pub fn bootstrap() -> Self {
        let tokio = tokio::runtime::Runtime::new().expect("failed to create desktop Tokio runtime");
        let home = meerkat_home();
        let configured_run = env::var_os(RUN_ENV);
        let run = configured_run
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("run"));
        // A private home keeps bundled smoke runs on transient helpers.
        // Unbundled runs use transient helpers even with the default home.
        let services = Arc::new(Mutex::new(BackgroundServices::new(
            env::var_os(HOME_ENV).is_none(),
        )));
        let startup = start_services(
            tokio.handle(),
            &services,
            &home,
            &run,
            configured_run.is_none(),
        );
        let client =
            startup.and_then(|_| tokio.block_on(RuntimeManagementClient::connect_local(&run)));

        match client {
            Ok(client) => Self {
                client: Arc::new(client),
                local_home: home.clone(),
                startup_error: None,
                startup_result: None,
                _tokio: tokio,
                _services: services,
            },
            Err(error) => {
                // Machine can wait for a Runtime that becomes available later.
                let client = {
                    let _guard = tokio.enter();
                    RuntimeManagementClient::connect_local_lazy(&run)
                };
                Self {
                    client: Arc::new(client),
                    local_home: home.clone(),
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

    pub fn navigation_url(&self) -> Result<tauri::Url, ClientError> {
        let run = env::var_os(RUN_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| self.local_home.join("run"));
        let value = fs::read_to_string(run.join("management.url"))
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        let url = tauri::Url::parse(value.trim())
            .map_err(|error| ClientError::Unavailable(error.to_string()))?;
        tauri::Url::parse(&format!("{}/", url.origin().ascii_serialization()))
            .map_err(|error| ClientError::Unavailable(error.to_string()))
    }
}

fn start_services(
    runtime: &tokio::runtime::Handle,
    services: &Arc<Mutex<BackgroundServices>>,
    home: &Path,
    run: &Path,
    manage_services: bool,
) -> Result<(), ClientError> {
    // Reuse a healthy owner before preparing or launching any local service,
    // unless it runs an older build than this app.
    let mut reused = runtime.block_on(RuntimeManagementClient::connect_local(&run));
    if reused.is_ok()
        && manage_services
        && services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop_outdated(home)?
    {
        reused = Err(ClientError::Unavailable("outdated Runtime stopped".into()));
    }
    let client = match reused {
        Ok(client) => client,
        Err(_) if manage_services => {
            {
                let mut services = services
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                services.prepare(home)?;
                services.ensure_runtime(home)?;
            }
            runtime.block_on(connect_with_retry(run))?
        }
        Err(error) => return Err(error),
    };
    if manage_services {
        let mut services = services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime.block_on(ensure_machine_ready(&client, &mut services, home))?;
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
        eprintln!("meerkat-desktop: --result is required for local bootstrap smoke");
        return 2;
    };
    let backend = DesktopBackend::bootstrap();
    let startup_ready = backend.startup_error.is_none();
    let management = backend
        ._tokio
        .block_on(backend.client.list_workspaces())
        .is_ok();
    let runtime_ready = management;
    let mut machine_connected = false;
    if startup_ready && runtime_ready {
        for _ in 0..60 {
            let catalog = backend
                ._tokio
                .block_on(backend.client.get_machine_catalog(None));
            if catalog.is_ok_and(|items| items.iter().any(|item| local_machine_online(item))) {
                machine_connected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let navigation_url = startup_ready
        .then(|| backend.navigation_url().ok())
        .flatten();
    let status = if startup_ready
        && runtime_ready
        && machine_connected
        && management
        && navigation_url.is_some()
    {
        "passed"
    } else {
        "failed"
    };
    let payload = serde_json::json!({
        "status": status,
        "navigation_url": navigation_url.map(|url| url.to_string()),
        "navigation_mode": "headless-intended",
        "error_code": backend.startup_error.as_ref().map(startup_error_code),
        "runtime_ready": runtime_ready,
        "machine_connected": machine_connected,
        "local_machine_id": backend._tokio.block_on(backend.client.get_machine_catalog(None)).ok().and_then(|items| items.into_iter().find(|item| item.is_local).map(|item| item.id)),
        "management": management,
    });
    let write_result = result_path
        .parent()
        .map(fs::create_dir_all)
        .transpose()
        .and_then(|_| fs::write(&result_path, format!("{}\n", payload)));
    if let Err(error) = write_result {
        eprintln!("meerkat-desktop: smoke result could not be written: {error}");
        return 3;
    }
    backend.stop_local_services();
    drop(backend);
    if status == "passed" { 0 } else { 1 }
}

fn meerkat_home() -> PathBuf {
    env::var_os(HOME_ENV).map(PathBuf::from).unwrap_or_else(|| {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".meerkat")
    })
}

async fn connect_with_retry(run: &Path) -> Result<RuntimeManagementClient, ClientError> {
    let client = RuntimeManagementClient::connect_local_lazy(run);
    let mut last_error = ClientError::Unavailable("Meerkat 尚未就绪".into());
    for _ in 0..50 {
        match client.list_workspaces().await {
            Ok(_) => return Ok(client),
            Err(error) => last_error = error,
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(last_error)
}

async fn ensure_machine_ready(
    client: &RuntimeManagementClient,
    services: &mut BackgroundServices,
    home: &Path,
) -> Result<(), ClientError> {
    if client
        .get_machine_catalog(None)
        .await?
        .iter()
        .any(|item| local_machine_online(item))
    {
        return Ok(());
    }
    services.ensure_machine(home)?;
    // A replaced Machine discovers its Harnesses before connecting. Model
    // discovery alone allows five seconds per Harness, plus shell/version
    // probes, so the old six-second window rejected healthy cold starts.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        if client
            .get_machine_catalog(None)
            .await?
            .iter()
            .any(|item| local_machine_online(item))
        {
            return Ok(());
        }
        if let Some(status) = services.machine_exited()? {
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

fn local_machine_online(machine: &meerkat_client::MachineOption) -> bool {
    machine.registered && machine.revoked_at.is_none() && machine.available && machine.is_local
}

/// Preserve the existing bootstrap failure categories without displaying raw errors.
pub fn startup_error_code(error: &ClientError) -> &'static str {
    let message = error.to_string();
    if message.contains("登录项") {
        "background-permission"
    } else if message.contains("重新安装") {
        "background-installation"
    } else if message.contains("完成更新") {
        "update-pending"
    } else if message.contains("找不到本地服务程序") {
        "helper-missing"
    } else {
        "startup-failed"
    }
}
