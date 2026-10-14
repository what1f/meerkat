#[path = "../../../../crates/protocol/tests/support/ws.rs"]
mod ws;
use super::daemon_e2e::DaemonChild;
use lankide_core::{ProviderId, WorkspaceId};
use lankide_protocol::{
    ComputerFrame, ComputerHello, DEFAULT_MAX_MESSAGE_BYTES, ListWorkspacesRequest,
    PROTOCOL_VERSION, computer_frame, connect_uds, runtime_frame,
};
use lankide_runtime_daemon::{AppError, RuntimePaths};
use lankide_storage::{DatabaseConfig, Storage, WorkspaceSpec};
use std::{fs, path::Path, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub(super) async fn run(
    selected: Option<&str>,
    root: &Path,
    artifacts: &Path,
) -> Result<(), AppError> {
    if selected.is_some_and(|name| !["S3.1", "S3.2", "S3.3"].contains(&name)) {
        return Err(AppError::Invariant("unknown shutdown scenario"));
    }
    for name in ["S3.1", "S3.2", "S3.3"] {
        if selected.is_some_and(|selected| selected != name) {
            continue;
        }
        let paths = RuntimePaths::for_home(root.join(name))?;
        paths.prepare()?;
        seed_workspace(&paths)?;
        let result = match name {
            "S3.1" => shutdown_with_computer_and_provider(&paths, artifacts).await,
            "S3.2" => second_instance_exits_cleanly(&paths, artifacts).await,
            _ => sigkill_restart_recovers(&paths, artifacts).await,
        };
        fs::write(
            artifacts.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "scenario": name,
                "status": if result.is_ok() { "passed" } else { "failed" },
                "error": result.as_ref().err().map(ToString::to_string),
            }))?,
        )?;
        result?;
        println!("{name} passed");
    }
    Ok(())
}

fn seed_workspace(paths: &RuntimePaths) -> Result<(), AppError> {
    let storage = Storage::open(DatabaseConfig::new(&paths.database))?;
    storage.create_workspace(WorkspaceSpec {
        id: WorkspaceId::new("shutdown_team").map_err(|_| AppError::Invariant("workspace id"))?,
        name: "Shutdown team".into(),
        provider_id: ProviderId::new("fixture").map_err(|_| AppError::Invariant("provider id"))?,
        provider_config: serde_json::json!({"log_path": paths.home.join("provider.log")}),
        provider_secret: serde_json::json!({}),
        active: true,
        now: 1,
    })?;
    storage.close()?;
    Ok(())
}

async fn assert_serving(paths: &RuntimePaths) -> Result<(), AppError> {
    let mut client = connect_uds(&paths.socket, DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(AppError::Protocol)?;
    let response = client
        .list_workspaces(ListWorkspacesRequest {
            request_id: "shutdown-scenario".into(),
        })
        .await
        .map_err(AppError::Status)?
        .into_inner();
    if response.workspaces.len() != 1 {
        return Err(AppError::Invariant("persisted workspace missing"));
    }
    Ok(())
}

async fn shutdown_with_computer_and_provider(
    paths: &RuntimePaths,
    artifacts: &Path,
) -> Result<(), AppError> {
    let mut daemon = DaemonChild::spawn(paths)?;
    daemon.wait_ready()?;
    let (sender, receiver) = mpsc::channel(16);
    sender
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::Hello(ComputerHello {
                protocol_version: PROTOCOL_VERSION,
                computer_id: "shutdown-computer".into(),
                instance_id: "shutdown-instance".into(),
                computer_version: "1".into(),
                os: "test".into(),
                arch: "test".into(),
                harnesses: Vec::new(),
            })),
        })
        .await
        .map_err(|_| AppError::Invariant("Computer input closed"))?;
    let mut frames = ws::connect_stream(&paths.socket, ReceiverStream::new(receiver))
        .await
        .map_err(AppError::Status)?
        .into_inner();
    if !matches!(
        frames
            .message()
            .await
            .map_err(AppError::Status)?
            .and_then(|f| f.frame),
        Some(runtime_frame::Frame::Hello(_))
    ) {
        return Err(AppError::Invariant("Computer was not connected"));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while !paths.home.join("provider.log").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| AppError::Invariant("Provider did not start"))?;
    // Keep both halves of the idle Computer stream alive through SIGTERM.
    let shutdown = daemon.terminate();
    let logs = daemon.logs()?;
    fs::write(artifacts.join("S3.1.log"), &logs)?;
    shutdown?;
    if logs.contains("still_shared") || paths.socket.exists() || paths.instance_lock.exists() {
        return Err(AppError::Invariant(
            "shutdown did not release Runtime ownership",
        ));
    }
    drop(sender);
    drop(frames);
    let mut next = DaemonChild::spawn(paths)?;
    next.wait_ready()?;
    assert_serving(paths).await?;
    next.terminate()?;
    Ok(())
}

async fn second_instance_exits_cleanly(
    paths: &RuntimePaths,
    artifacts: &Path,
) -> Result<(), AppError> {
    let mut first = DaemonChild::spawn(paths)?;
    first.wait_ready()?;
    assert_serving(paths).await?;
    let lock = fs::read(&paths.instance_lock)?;
    // The existing connection can read SQLite; a fresh open cannot. Thus a
    // successful duplicate launch proves it did not open the database.
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        let original = fs::metadata(&paths.database)?.permissions();
        fs::set_permissions(&paths.database, fs::Permissions::from_mode(0o000))?;
        original
    };
    let result = async {
        if Storage::open(DatabaseConfig::new(&paths.database)).is_ok() {
            return Err(AppError::Invariant(
                "database-open denial was not effective",
            ));
        }
        let mut second = DaemonChild::spawn(paths)?;
        let status = second.wait_for_exit(Duration::from_secs(3))?;
        let logs = second.logs()?;
        fs::write(artifacts.join("S3.2.log"), &logs)?;
        if status != 0 || logs.lines().collect::<Vec<_>>() != ["runtime_daemon_already_running"] {
            return Err(AppError::Invariant(
                "duplicate did not exit 0 with one log line",
            ));
        }
        if fs::read(&paths.instance_lock)? != lock {
            return Err(AppError::Invariant("duplicate replaced owner's lock"));
        }
        assert_serving(paths).await
    }
    .await;
    #[cfg(unix)]
    fs::set_permissions(&paths.database, permissions)?;
    result?;
    first.terminate()?;
    Ok(())
}

async fn sigkill_restart_recovers(paths: &RuntimePaths, artifacts: &Path) -> Result<(), AppError> {
    let mut first = DaemonChild::spawn(paths)?;
    first.wait_ready()?;
    assert_serving(paths).await?;
    let lock = fs::read(&paths.instance_lock)?;
    first.kill()?;
    if !paths.socket.exists() || !paths.instance_lock.exists() {
        return Err(AppError::Invariant(
            "SIGKILL did not leave stale ownership files",
        ));
    }
    let mut recovered = DaemonChild::spawn(paths)?;
    recovered.wait_ready()?;
    assert_serving(paths).await?;
    if fs::read(&paths.instance_lock)? == lock {
        return Err(AppError::Invariant(
            "restart did not recover the stale lock",
        ));
    }
    recovered.terminate()?;
    fs::write(artifacts.join("S3.3.log"), recovered.logs()?)?;
    if paths.socket.exists() || paths.instance_lock.exists() {
        return Err(AppError::Invariant(
            "recovered Runtime did not stop cleanly",
        ));
    }
    Ok(())
}
