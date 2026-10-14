use lankide_core::{AgentId, AttentionLevel, ChannelAbility, ChannelId, ProviderId, WorkspaceId};
use lankide_protocol::{
    AgentRef, AgentRunContext, AssignmentResult, ComputerFrame, ComputerHello, ContentKind,
    ContentRequest, DEFAULT_MAX_MESSAGE_BYTES, DeleteAgentRequest, DeliveryResult, DesiredState,
    ExecutionEvent, ExecutionEventKind, ListWorkspacesRequest, PROTOCOL_VERSION, ResourceChunk,
    ResourceDescriptor, ResourceRequest, SetWorkspaceStateRequest, WorkspaceRef, computer_frame,
    connect_uds, runtime_frame,
};
use lankide_provider::{
    AgentContext, ChannelDefaultAttention, ChannelDefinition, InitContext, Provider, ProviderError,
    ProviderFuture, ProviderHost, ProviderInfo, SendMessageContext, SendMessageResult,
};
use lankide_runtime::ProviderFactoryRegistry;
use lankide_storage::{
    AgentSpec, AttentionRecord, DatabaseConfig, DispatchReason, InboxDispatch, IngestEvent,
    MessageInput, Storage, WorkspaceSpec,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use lankide_runtime_daemon::{AppError, InstanceGuard, RuntimePaths, prepare_socket_path};
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Debug, Serialize)]
struct E2eResult {
    run_id: String,
    success: bool,
    status: &'static str,
    checks: Vec<&'static str>,
    cleanup: &'static str,
}

pub async fn run_e2e(selected: Option<&str>) -> Result<PathBuf, AppError> {
    let run_id = unique_run_id();
    let artifact_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e-artifacts/runtime-daemon")
        .join(&run_id);
    fs::create_dir_all(&artifact_dir)?;
    let artifact = artifact_dir.join("result.json");
    let temp = tempfile::Builder::new()
        .prefix("lkrt-")
        .tempdir_in("/tmp")?;
    let paths = RuntimePaths::for_home(temp.path().join(".lankide"))?;
    let scenario = async {
        if selected.is_none() {
            daemon_e2e_scenario(&paths).await?;
        }
        super::shutdown_scenarios::run(selected, temp.path(), &artifact_dir).await
    }
    .await;
    let cleanup = if temp.close().is_ok() {
        "complete"
    } else {
        "failed"
    };
    let result = E2eResult {
        run_id,
        success: scenario.is_ok() && cleanup == "complete",
        status: if scenario.is_ok() && cleanup == "complete" {
            "passed"
        } else {
            "failed"
        },
        checks: match selected {
            Some("S3.1") => vec!["S3.1_shutdown_with_computer_and_provider"],
            Some("S3.2") => vec!["S3.2_second_instance_exits_cleanly"],
            Some("S3.3") => vec!["S3.3_sigkill_restart_recovers"],
            Some(_) => Vec::new(),
            None => vec![
                "temporary_home",
                "private_directories",
                "sqlite_migration",
                "single_instance_lock",
                "second_instance_preserves_socket",
                "stale_instance_lock_removed",
                "unsafe_socket_rejected",
                "stale_socket_removed",
                "production_provider_factories",
                "multi_workspace_restore",
                "management_ready_before_provider_start",
                "pause_resume_provider",
                "computer_assignment",
                "agent_delete_rpc",
                "computer_file_cleanup",
                "authorized_resource_roundtrip",
                "execution_commit",
                "graceful_restart",
                "S3.1_shutdown_with_computer_and_provider",
                "S3.2_second_instance_exits_cleanly",
                "S3.3_sigkill_restart_recovers",
            ],
        },
        cleanup,
    };
    fs::write(&artifact, serde_json::to_vec_pretty(&result)?)?;
    scenario?;
    if cleanup != "complete" {
        return Err(AppError::Invariant("temporary home cleanup failed"));
    }
    Ok(artifact)
}

pub fn fixture_factories() -> Result<ProviderFactoryRegistry, AppError> {
    let factories = lankide_runtime_daemon::production_factories()?;
    factories.register_host_factory::<FixtureProvider, _>(
        FixtureProvider::info_value(),
        |workspace, services| {
            ProviderHost::new(
                FixtureProvider {
                    log_path: workspace
                        .provider_config
                        .get("log_path")
                        .and_then(serde_json::Value::as_str)
                        .map(PathBuf::from),
                    startup_delay_ms: workspace
                        .provider_config
                        .get("startup_delay_ms")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                },
                workspace.id.clone(),
                workspace.provider_config.clone(),
                services,
            )
        },
    )?;
    Ok(factories)
}

async fn daemon_e2e_scenario(paths: &RuntimePaths) -> Result<(), AppError> {
    paths.prepare()?;
    verify_private_directory(&paths.home)?;
    verify_private_directory(&paths.workspaces)?;
    verify_private_directory(&paths.run)?;
    let guard = InstanceGuard::acquire(&paths.run, &paths.instance_lock, "preflight")?;
    if !matches!(
        InstanceGuard::acquire(&paths.run, &paths.instance_lock, "second-instance"),
        Err(lankide_daemon_runtime::DaemonRuntimeError::AlreadyRunning)
    ) {
        return Err(AppError::Invariant("second instance was not rejected"));
    }
    drop(guard);
    #[cfg(unix)]
    {
        let mut exited = Command::new("/bin/sh").args(["-c", "exit 0"]).spawn()?;
        let stale_pid = exited.id();
        exited.wait()?;
        fs::write(
            &paths.instance_lock,
            format!("pid={stale_pid} run_id=stale-instance\n"),
        )?;
        fs::set_permissions(
            &paths.instance_lock,
            fs::Permissions::from_mode(PRIVATE_FILE_MODE),
        )?;
        let recovered = InstanceGuard::acquire(&paths.run, &paths.instance_lock, "stale-recovery")?;
        drop(recovered);
        if paths.instance_lock.exists() {
            return Err(AppError::Invariant("stale instance lock was not released"));
        }
    }
    let unsafe_entry = paths.run.join("unsafe.sock");
    File::create(&unsafe_entry)?;
    if !matches!(
        prepare_socket_path(&unsafe_entry),
        Err(AppError::UnsafeSocketEntry)
    ) {
        return Err(AppError::Invariant("unsafe socket entry was accepted"));
    }
    fs::remove_file(unsafe_entry)?;
    #[cfg(unix)]
    {
        let stale = std::os::unix::net::UnixListener::bind(&paths.socket)?;
        drop(stale);
        prepare_socket_path(&paths.socket)?;
        if paths.socket.exists() {
            return Err(AppError::Invariant("stale socket was not removed"));
        }
    }

    let fixture = ProviderId::new("fixture").map_err(|_| AppError::Invariant("fixture id"))?;
    let active_workspace = WorkspaceId::new("active_fixture")
        .map_err(|_| AppError::Invariant("active workspace id"))?;
    let paused_workspace = WorkspaceId::new("paused_fixture")
        .map_err(|_| AppError::Invariant("paused workspace id"))?;
    let agent_id = AgentId::new("agent_a").map_err(|_| AppError::Invariant("fixture agent id"))?;
    let deletable_agent_id = AgentId::new("deletable_agent")
        .map_err(|_| AppError::Invariant("deletable fixture agent id"))?;
    let channel_id =
        ChannelId::new("chat").map_err(|_| AppError::Invariant("fixture channel id"))?;
    let provider_log = paths.home.join("fixture-provider.log");
    let storage = Storage::open(DatabaseConfig::new(&paths.database))?;
    let settings = storage.sqlite_settings()?;
    if settings.journal_mode != "wal" || !settings.foreign_keys {
        return Err(AppError::Invariant("SQLite migration settings are invalid"));
    }
    for (workspace_id, active) in [(&active_workspace, true), (&paused_workspace, false)] {
        storage.create_workspace(WorkspaceSpec {
            id: workspace_id.clone(),
            name: workspace_id.to_string(),
            provider_id: fixture.clone(),
            provider_config: if active {
                serde_json::json!({"log_path": provider_log, "startup_delay_ms": 2500})
            } else {
                serde_json::json!({"log_path": provider_log})
            },
            provider_secret: serde_json::json!({}),
            active,
            now: 1,
        })?;
    }
    for (id, provider_id) in [("paused_feishu", "feishu"), ("paused_telegram", "telegram")] {
        storage.create_workspace(WorkspaceSpec {
            id: WorkspaceId::new(id).map_err(|_| AppError::Invariant("provider workspace id"))?,
            name: id.into(),
            provider_id: ProviderId::new(provider_id)
                .map_err(|_| AppError::Invariant("provider id"))?,
            provider_config: serde_json::json!({}),
            provider_secret: serde_json::json!({}),
            active: false,
            now: 1,
        })?;
    }
    storage.create_agent(AgentSpec {
        workspace_id: active_workspace.clone(),
        id: agent_id.clone(),
        name: "Agent A".into(),
        now: 2,
    })?;
    storage.create_agent(AgentSpec {
        workspace_id: active_workspace.clone(),
        id: deletable_agent_id.clone(),
        name: "Deletable Agent".into(),
        now: 2,
    })?;
    let mut agent = storage.agent_record(&active_workspace, &agent_id)?;
    agent.computer_id = "computer-a".into();
    agent.harness_id = "fixture".into();
    agent.model = "fixture".into();
    agent.thinking = "low".into();
    agent
        .attention
        .insert(channel_id.clone(), AttentionRecord::Always);
    storage.upsert_agent_record(agent)?;
    let mut deletable_agent = storage.agent_record(&active_workspace, &deletable_agent_id)?;
    deletable_agent.computer_id = "computer-a".into();
    deletable_agent.harness_id = "fixture".into();
    deletable_agent.model = "fixture".into();
    deletable_agent.thinking = "low".into();
    storage.upsert_agent_record(deletable_agent)?;
    let ingested = storage.ingest_event(IngestEvent {
        workspace_id: active_workspace.clone(),
        provider_id: fixture,
        channel_id: channel_id.clone(),
        channel_name: "Chat".into(),
        provider_chat_id: "chat-a".into(),
        chat_name: "Chat A".into(),
        provider_event_id: Some("daemon-event-1".into()),
        provider_message_id: "daemon-message-1".into(),
        message: MessageInput {
            sender_kind: lankide_core::MessageAuthorType::Human,
            sender_id: "human".into(),
            sender_name: "Human".into(),
            content: "daemon trigger".into(),
            mentions: Vec::new(),
            reply_to_provider_message_id: None,
            resources: Vec::new(),
            occurred_at: 3,
        },
        dispatch: vec![InboxDispatch {
            agent_id: agent_id.clone(),
            available_at: 3,
            trigger_reason: DispatchReason::Follow,
        }],
        now: 3,
    })?;
    storage.close()?;
    let deletable_directory = paths
        .workspaces
        .join(active_workspace.as_str())
        .join("agents")
        .join(deletable_agent_id.as_str());
    fs::create_dir_all(&deletable_directory)?;
    fs::write(
        deletable_directory.join("state.txt"),
        b"delete through Computer",
    )?;

    let mut child = DaemonChild::spawn(paths)?;
    let ready_elapsed = child.wait_ready()?;
    if ready_elapsed >= Duration::from_millis(1500) {
        return Err(AppError::Invariant(
            "management readiness waited for Provider startup",
        ));
    }
    let mut management = connect_uds(&paths.socket, DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(AppError::Protocol)?;
    let listed = management
        .list_workspaces(ListWorkspacesRequest {
            request_id: "daemon-list-1".into(),
        })
        .await
        .map_err(AppError::Status)?
        .into_inner();
    if listed.workspaces.len() != 4 {
        return Err(AppError::Invariant("not all workspaces were restored"));
    }
    let mut second = DaemonChild::spawn(paths)?;
    if second.wait_for_exit(Duration::from_secs(3))? != 0 {
        return Err(AppError::Invariant(
            "second daemon instance did not exit cleanly",
        ));
    }
    let mut second_connection = connect_uds(&paths.socket, DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(AppError::Protocol)?;
    second_connection
        .list_workspaces(ListWorkspacesRequest {
            request_id: "daemon-list-after-second-instance".into(),
        })
        .await
        .map_err(AppError::Status)?;
    management
        .set_workspace_state(SetWorkspaceStateRequest {
            request_id: "resume-paused".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: paused_workspace.to_string(),
            }),
            desired_state: DesiredState::Running as i32,
        })
        .await
        .map_err(AppError::Status)?;
    management
        .set_workspace_state(SetWorkspaceStateRequest {
            request_id: "pause-active".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: active_workspace.to_string(),
            }),
            desired_state: DesiredState::Paused as i32,
        })
        .await
        .map_err(AppError::Status)?;
    management
        .set_workspace_state(SetWorkspaceStateRequest {
            request_id: "resume-active".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: active_workspace.to_string(),
            }),
            desired_state: DesiredState::Running as i32,
        })
        .await
        .map_err(AppError::Status)?;

    let mut computer_client = connect_uds(&paths.socket, DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(AppError::Protocol)?;
    let (computer_tx, computer_rx) = mpsc::channel(16);
    computer_tx
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::Hello(ComputerHello {
                protocol_version: PROTOCOL_VERSION,
                computer_id: "computer-a".into(),
                instance_id: "instance-a".into(),
                computer_version: "e2e".into(),
                os: "test".into(),
                arch: "test".into(),
                harnesses: Vec::new(),
            })),
        })
        .await
        .map_err(|_| AppError::Invariant("Computer hello channel closed"))?;
    let mut runtime_frames = computer_client
        .connect_computer(ReceiverStream::new(computer_rx))
        .await
        .map_err(AppError::Status)?
        .into_inner();
    let first = runtime_frames
        .message()
        .await
        .map_err(AppError::Status)?
        .ok_or(AppError::Invariant("Runtime hello missing"))?;
    if !matches!(first.frame, Some(runtime_frame::Frame::Hello(_))) {
        return Err(AppError::Invariant("Runtime hello missing"));
    }
    let assignment = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let frame = runtime_frames
                .message()
                .await
                .map_err(AppError::Status)?
                .ok_or(AppError::Invariant("assignment stream closed"))?;
            if let Some(runtime_frame::Frame::AssignExecution(assignment)) = frame.frame {
                return Ok::<_, AppError>(assignment);
            }
        }
    })
    .await
    .map_err(|_| AppError::Invariant("Computer assignment timed out"))??;
    let execution = assignment
        .execution
        .clone()
        .ok_or(AppError::Invariant("assignment execution missing"))?;
    let context = AgentRunContext {
        execution: Some(execution.clone()),
        execution_token: assignment.execution_token.clone(),
    };
    computer_tx
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::AssignmentResult(AssignmentResult {
                failure: None,
                execution: Some(execution.clone()),
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        })
        .await
        .map_err(|_| AppError::Invariant("assignment result channel closed"))?;
    computer_tx
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::DeliveryResult(DeliveryResult {
                execution: Some(execution.clone()),
                delivery_id: assignment
                    .initial_delivery
                    .as_ref()
                    .map(|delivery| delivery.delivery_id)
                    .unwrap_or_default(),
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        })
        .await
        .map_err(|_| AppError::Invariant("delivery result channel closed"))?;

    let resource_bytes = b"runtime-daemon-resource".to_vec();
    let resource_sha = Sha256::digest(&resource_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let uploaded = management
        .upload_resource(tokio_stream::iter(vec![ResourceChunk {
            transfer_id: "daemon-upload".into(),
            resource_id: 0,
            offset: 0,
            bytes: resource_bytes.clone(),
            eof: true,
            descriptor: Some(ResourceDescriptor {
                file_name: "daemon.txt".into(),
                media_type: "text/plain".into(),
                size_bytes: resource_bytes.len() as u64,
                sha256: resource_sha,
                context: Some(context.clone()),
            }),
        }]))
        .await
        .map_err(AppError::Status)?
        .into_inner();
    let mut download = management
        .download_resource(ResourceRequest {
            resource_id: uploaded.resource_id,
            transfer_id: "daemon-download".into(),
            context: Some(context.clone()),
        })
        .await
        .map_err(AppError::Status)?
        .into_inner();
    let mut downloaded = Vec::new();
    while let Some(chunk) = download.message().await.map_err(AppError::Status)? {
        downloaded.extend(chunk.bytes);
    }
    if downloaded != resource_bytes {
        return Err(AppError::Invariant("resource roundtrip mismatch"));
    }
    let content = management
        .get_content(ContentRequest {
            context: Some(context),
            kind: ContentKind::Message as i32,
            content_id: ingested.message_id.to_string(),
        })
        .await
        .map_err(AppError::Status)?
        .into_inner();
    if content.content_markdown != "daemon trigger" {
        return Err(AppError::Invariant("content gateway mismatch"));
    }
    for (sequence, kind) in [
        (1, ExecutionEventKind::Started),
        (2, ExecutionEventKind::Completed),
    ] {
        computer_tx
            .send(ComputerFrame {
                frame: Some(computer_frame::Frame::ExecutionEvent(ExecutionEvent {
                    execution: Some(execution.clone()),
                    sequence,
                    occurred_at: now_millis(),
                    kind: kind as i32,
                    payload: Vec::new(),
                })),
            })
            .await
            .map_err(|_| AppError::Invariant("execution event channel closed"))?;
    }
    for _ in 0..2 {
        let frame = tokio::time::timeout(Duration::from_secs(2), runtime_frames.message())
            .await
            .map_err(|_| AppError::Invariant("execution ack timed out"))?
            .map_err(AppError::Status)?
            .ok_or(AppError::Invariant("execution ack stream closed"))?;
        if !matches!(
            frame.frame,
            Some(runtime_frame::Frame::ExecutionEventAck(_))
        ) {
            return Err(AppError::Invariant("execution ack missing"));
        }
    }
    let deleted = management
        .delete_agent(DeleteAgentRequest {
            request_id: "delete-agent-rpc".into(),
            agent: Some(AgentRef {
                workspace_id: active_workspace.to_string(),
                agent_id: deletable_agent_id.to_string(),
            }),
        })
        .await
        .map_err(AppError::Status)?
        .into_inner();
    if deleted.request_id != "delete-agent-rpc"
        || deleted.agent.as_ref().is_none_or(|agent| {
            agent.workspace_id != active_workspace.as_str()
                || agent.agent_id != deletable_agent_id.as_str()
        })
    {
        return Err(AppError::Invariant("delete Agent RPC response mismatch"));
    }
    let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frame = runtime_frames
                .message()
                .await
                .map_err(AppError::Status)?
                .ok_or(AppError::Invariant("Agent cleanup stream closed"))?;
            if let Some(runtime_frame::Frame::DeleteAgentFiles(request)) = frame.frame {
                return Ok::<_, AppError>(request);
            }
        }
    })
    .await
    .map_err(|_| AppError::Invariant("Agent cleanup request timed out"))??;
    if cleanup.agent.as_ref().is_none_or(|agent| {
        agent.workspace_id != active_workspace.as_str()
            || agent.agent_id != deletable_agent_id.as_str()
    }) {
        return Err(AppError::Invariant("Agent cleanup target mismatch"));
    }
    fs::remove_dir_all(&deletable_directory)?;
    computer_tx
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::DeleteAgentFilesResult(
                lankide_protocol::DeleteAgentFilesResult {
                    request_id: cleanup.request_id,
                    success: true,
                    error_code: String::new(),
                    error_message: String::new(),
                },
            )),
        })
        .await
        .map_err(|_| AppError::Invariant("Agent cleanup result channel closed"))?;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let storage = Storage::open(DatabaseConfig::new(&paths.database))?;
            let pending = storage.pending_agent_file_deletions("computer-a")?;
            let deleted = storage
                .agent_record(&active_workspace, &deletable_agent_id)
                .is_err();
            storage.close()?;
            if pending.is_empty() && deleted && !deletable_directory.exists() {
                return Ok::<_, AppError>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| AppError::Invariant("Agent deletion did not finish"))??;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(management);
    child.terminate()?;
    drop(computer_tx);
    drop(runtime_frames);
    if paths.socket.exists() || paths.instance_lock.exists() {
        return Err(AppError::Invariant("graceful shutdown left runtime files"));
    }
    let log = fs::read_to_string(&provider_log)?;
    if !log.lines().any(|line| line.contains("active_fixture"))
        || !log.lines().any(|line| line.contains("paused_fixture"))
        || !log
            .lines()
            .any(|line| line.contains("delete_agent active_fixture/deletable_agent"))
    {
        return Err(AppError::Invariant(
            "Workspace Provider lifecycle was not observed",
        ));
    }
    let storage = Storage::open(DatabaseConfig::new(&paths.database))?;
    if storage.cursors(&active_workspace, &agent_id)?.len() != 1 {
        return Err(AppError::Invariant("execution cursor was not committed"));
    }
    if storage
        .resource_metadata(
            &active_workspace,
            lankide_core::ResourceId::new(uploaded.resource_id)
                .map_err(|_| AppError::Invariant("resource id"))?,
        )?
        .is_none()
    {
        return Err(AppError::Invariant("resource metadata was not recovered"));
    }
    storage.close()?;
    let mut restarted = DaemonChild::spawn(paths)?;
    restarted.wait_ready()?;
    let mut restart_client = connect_uds(&paths.socket, DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(AppError::Protocol)?;
    if restart_client
        .list_workspaces(ListWorkspacesRequest {
            request_id: "daemon-list-restart".into(),
        })
        .await
        .map_err(AppError::Status)?
        .into_inner()
        .workspaces
        .len()
        != 4
    {
        return Err(AppError::Invariant("restart did not restore workspaces"));
    }
    drop(restart_client);
    restarted.terminate()?;
    Ok(())
}

struct FixtureProvider {
    log_path: Option<PathBuf>,
    startup_delay_ms: u64,
}

impl FixtureProvider {
    fn info_value() -> ProviderInfo {
        ProviderInfo {
            id: ProviderId::new("fixture").expect("static fixture Provider ID"),
            name: "Fixture".into(),
            description: "Hermetic messaging fixture".into(),
            version: "1".into(),
            icon: String::new(),
        }
    }
}

impl Provider for FixtureProvider {
    fn info(&self) -> ProviderInfo {
        Self::info_value()
    }

    fn channels<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async {
            Ok(vec![ChannelDefinition::new(
                ChannelId::new("chat").map_err(|_| ProviderError::InvalidInput { field: "id" })?,
                "Chat",
                ChannelDefaultAttention::Mention,
                [AttentionLevel::Follow, AttentionLevel::Mention],
                [ChannelAbility::Reply, ChannelAbility::Attachment],
            )?])
        })
    }

    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            // Hold real HostServices while the outer receiver stops, as a
            // native blocking receiver does, including during cancelled init.
            let receiver_context = context.clone();
            tokio::spawn(async move {
                receiver_context.shutdown_signal().cancelled().await;
                tokio::time::sleep(Duration::from_millis(150)).await;
                drop(receiver_context);
            });
            if self.startup_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.startup_delay_ms)).await;
            }
            if let Some(path) = &self.log_path {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|_| ProviderError::External {
                        operation: "fixture_init",
                        retryable: true,
                    })?;
                writeln!(file, "init {}", context.workspace_id()).map_err(|_| {
                    ProviderError::External {
                        operation: "fixture_init",
                        retryable: true,
                    }
                })?;
            }
            Ok(())
        })
    }

    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult> {
        Box::pin(
            async move { SendMessageResult::sent(format!("fixture-{}", context.message_id.get())) },
        )
    }

    fn delete_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if let Some(path) = &self.log_path {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(|_| ProviderError::External {
                        operation: "fixture_delete_agent",
                        retryable: true,
                    })?;
                writeln!(
                    file,
                    "delete_agent {}/{}",
                    context.workspace_id, context.agent_id
                )
                .map_err(|_| ProviderError::External {
                    operation: "fixture_delete_agent",
                    retryable: true,
                })?;
            }
            Ok(())
        })
    }
}

pub(super) struct DaemonChild {
    child: Child,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl DaemonChild {
    pub(super) fn spawn(paths: &RuntimePaths) -> Result<Self, AppError> {
        let id = unique_run_id();
        let stdout_log = paths.home.join(format!("{id}.stdout.log"));
        let stderr_log = paths.home.join(format!("{id}.stderr.log"));
        let child = Command::new(std::env::current_exe()?)
            .arg("--home")
            .arg(&paths.home)
            .arg("--e2e-fixture-provider")
            .stdout(File::create(&stdout_log)?)
            .stderr(File::create(&stderr_log)?)
            .spawn()?;
        Ok(Self {
            child,
            stdout_log,
            stderr_log,
        })
    }

    pub(super) fn wait_ready(&mut self) -> Result<Duration, AppError> {
        let started = Instant::now();
        loop {
            let output = fs::read_to_string(&self.stdout_log)?;
            if let Some(line) = output
                .lines()
                .find(|line| line.contains("\"event\":\"ready\""))
            {
                let readiness: serde_json::Value = serde_json::from_str(line)?;
                if readiness.get("management_ready").and_then(|v| v.as_bool()) != Some(true) {
                    return Err(AppError::Invariant("daemon readiness was invalid"));
                }
                return Ok(started.elapsed());
            }
            if self.child.try_wait()?.is_some() {
                eprintln!("{}", fs::read_to_string(&self.stderr_log)?);
                return Err(AppError::Invariant("daemon exited before readiness"));
            }
            if started.elapsed() >= Duration::from_secs(5) {
                return Err(AppError::Invariant("daemon readiness timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn logs(&self) -> Result<String, AppError> {
        Ok(format!(
            "{}{}",
            fs::read_to_string(&self.stdout_log)?,
            fs::read_to_string(&self.stderr_log)?
        ))
    }

    pub(super) fn kill(&mut self) -> Result<(), AppError> {
        self.child.kill()?;
        let status = self.child.wait()?;
        if status.success() {
            return Err(AppError::Invariant("SIGKILL unexpectedly succeeded"));
        }
        Ok(())
    }

    pub(super) fn wait_for_exit(&mut self, timeout: Duration) -> Result<i32, AppError> {
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status.code().unwrap_or(1));
            }
            if started.elapsed() >= timeout {
                return Err(AppError::Invariant("daemon process did not exit"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn terminate(&mut self) -> Result<(), AppError> {
        #[cfg(unix)]
        {
            let status = Command::new("kill")
                .arg("-TERM")
                .arg(self.child.id().to_string())
                .status()?;
            if !status.success() {
                return Err(AppError::Invariant("could not stop daemon"));
            }
        }
        #[cfg(not(unix))]
        self.child.kill()?;
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break status;
            }
            if started.elapsed() >= Duration::from_secs(5) {
                self.child.kill()?;
                let _ = self.child.wait();
                return Err(AppError::Invariant("daemon shutdown timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if !status.success() {
            eprintln!("daemon shutdown status={status}; logs={}", self.logs()?);
            return Err(AppError::Invariant("daemon did not stop cleanly"));
        }
        Ok(())
    }
}

impl Drop for DaemonChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
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

fn verify_private_directory(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    if fs::metadata(path)?.permissions().mode() & 0o777 != PRIVATE_DIRECTORY_MODE {
        return Err(AppError::Invariant("directory permissions are not 0700"));
    }
    Ok(())
}
