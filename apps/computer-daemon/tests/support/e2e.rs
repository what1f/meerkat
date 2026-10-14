#[path = "../../../../crates/harness/tests/support/process.rs"]
mod process_support;
#[path = "../../../../crates/protocol/tests/support/ws.rs"]
mod ws;
use process_support::{ProcessHarnessFactory, ProcessHarnessSpec};

use lankide_computer::{Computer, ComputerConfig, ReconnectPolicy, RegisteredHarness};
use lankide_computer_daemon::{
    AppError, ComputerDaemonPaths, DaemonConfig, InstanceGuard, production_harnesses,
};
use lankide_harness::{
    HarnessConfig as NativeHarnessConfig, HarnessInfo as NativeHarnessInfo, ModelRef,
};
use lankide_protocol::{
    AgentRef, AgentRuntimeProfile, AssignExecution, AssignmentResult, ComputerFrame,
    ComputerFrameStream, ComputerHello, DeliverSteer, Delivery, DeliveryKind, ExecutionEvent,
    ExecutionEventAck, ExecutionEventKind, ExecutionRef, HarnessConfig, ResourceChunk,
    ResourceChunkStream, ResourceDescriptor, ResourceRef, ResourceRequest, RuntimeFrame,
    RuntimeFrameStream, RuntimeHello, RuntimePeer, SessionAction, SessionDirective, ToolDefinition,
    ToolResult, TransportConfig, WorkspaceRef, computer_frame, runtime_frame,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lankide_protocol::Status;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

const STEP: Duration = Duration::from_secs(4);
const HARD_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Clone)]
struct Peer {
    state: Arc<PeerState>,
}

struct PeerState {
    incoming: mpsc::UnboundedSender<ComputerFrame>,
    runtime: Mutex<Option<mpsc::Sender<Result<RuntimeFrame, Status>>>>,
    hellos: Mutex<Vec<ComputerHello>>,
    resources: Mutex<HashMap<i64, (String, Vec<u8>)>>,
}

impl Peer {
    fn new(incoming: mpsc::UnboundedSender<ComputerFrame>) -> Self {
        Self {
            state: Arc::new(PeerState {
                incoming,
                runtime: Mutex::new(None),
                hellos: Mutex::new(Vec::new()),
                resources: Mutex::new(HashMap::new()),
            }),
        }
    }

    async fn send(&self, frame: RuntimeFrame) -> Result<(), AppError> {
        for _ in 0..200 {
            let sender = self.state.runtime.lock().expect("runtime lock").clone();
            if let Some(sender) = sender {
                sender
                    .send(Ok(frame))
                    .await
                    .map_err(|_| AppError::Invariant("runtime stream closed"))?;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(AppError::Invariant("computer did not connect"))
    }

    fn disconnect(&self) {
        self.state.runtime.lock().expect("runtime lock").take();
    }
}

#[async_trait::async_trait]
impl lankide_protocol::ComputerPeer for Peer {
    async fn authorize_link(
        &self,
        credential: &str,
        local_secret: &str,
    ) -> Result<(), lankide_protocol::link::CloseReason> {
        if credential == local_secret {
            Ok(())
        } else {
            Err(lankide_protocol::link::CloseReason::Unauthorized)
        }
    }

    async fn connect_computer(
        &self,
        hello: ComputerHello,
        mut frames: ComputerFrameStream,
    ) -> Result<RuntimeFrameStream, Status> {
        self.state.hellos.lock().expect("hello lock").push(hello);
        let incoming = self.state.incoming.clone();
        tokio::spawn(async move {
            while let Some(Ok(frame)) = frames.next().await {
                let _ = incoming.send(frame);
            }
        });
        let (sender, receiver) = mpsc::channel(64);
        sender
            .send(Ok(RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::Hello(RuntimeHello {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    runtime_instance_id: "runtime-daemon-e2e".into(),
                    heartbeat_interval_ms: 500,
                    max_message_bytes: lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES as u64,
                })),
            }))
            .await
            .map_err(|_| Status::unavailable("runtime stream closed"))?;
        *self.state.runtime.lock().expect("runtime lock") = Some(sender);
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }
}

#[async_trait::async_trait]
impl RuntimePeer for Peer {
    async fn upload_resource(
        &self,
        mut chunks: ResourceChunkStream,
    ) -> Result<ResourceRef, Status> {
        let mut descriptor = None;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            descriptor = descriptor.or(chunk.descriptor);
        }
        let descriptor = descriptor.ok_or_else(|| Status::invalid_argument("descriptor"))?;
        Ok(ResourceRef {
            resource_id: 901,
            file_name: descriptor.file_name,
            media_type: descriptor.media_type,
            size_bytes: descriptor.size_bytes,
            sha256: descriptor.sha256,
        })
    }

    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status> {
        let (file_name, bytes) = self
            .state
            .resources
            .lock()
            .expect("resource lock")
            .get(&request.resource_id)
            .cloned()
            .ok_or_else(|| Status::not_found("resource"))?;
        let descriptor = ResourceDescriptor {
            file_name,
            media_type: "text/plain".into(),
            size_bytes: bytes.len() as u64,
            sha256: hex(&Sha256::digest(&bytes)),
            context: request.context,
        };
        Ok(Box::pin(tokio_stream::iter([Ok(ResourceChunk {
            transfer_id: request.transfer_id,
            resource_id: request.resource_id,
            offset: 0,
            bytes,
            eof: true,
            descriptor: Some(descriptor),
        })])))
    }
}

pub async fn run() -> Result<PathBuf, AppError> {
    let run_id = unique_run_id();
    let started = Instant::now();
    let temporary = tempfile::Builder::new()
        .prefix("lkcd-")
        .tempdir_in("/tmp")?;
    let paths = ComputerDaemonPaths::for_home(temporary.path().join(".lankide"))?;
    paths.prepare()?;
    let guard = InstanceGuard::acquire(&paths.run, &paths.instance_lock, &run_id)?;
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
        fs::set_permissions(&paths.instance_lock, fs::Permissions::from_mode(0o600))?;
        let recovered = InstanceGuard::acquire(&paths.run, &paths.instance_lock, "stale-recovery")?;
        drop(recovered);
        if paths.instance_lock.exists() {
            return Err(AppError::Invariant("stale instance lock was not released"));
        }
    }

    let artifact_directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e-artifacts/computer-daemon")
        .join(&run_id);
    fs::create_dir_all(&artifact_directory)?;
    let artifact = artifact_directory.join("result.json");
    let pid_journal = artifact_directory.join("pids.tsv");
    let harness_log = artifact_directory.join("harness.tsv");
    let executable = std::env::current_exe()?;
    let dcc_path = temporary.path().join("dcc");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&executable, &dcc_path)?;
    let mut production = DaemonConfig::for_home(paths.home.clone())?;
    let resolved_cli_directory = temporary.path().join("resolved-cli");
    fs::create_dir_all(&resolved_cli_directory)?;
    for name in ["pi", "codex", "claude"] {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&executable, resolved_cli_directory.join(name))?;
    }
    production.set_executable_path(resolved_cli_directory.display().to_string());
    require(
        production.pi_cli.is_absolute()
            && production.codex_cli.is_absolute()
            && production.claude_code_cli.is_absolute(),
        "login PATH did not resolve Harness executables",
    )?;
    production.pi_cli = executable.clone();
    production.codex_cli = executable.clone();
    production.claude_code_cli = executable.clone();
    for registration in production_harnesses(&production) {
        let probe = registration
            .factory
            .probe(&registration.probe_config)
            .await
            .map_err(|_| AppError::Invariant("production adapter probe failed"))?;
        if !matches!(
            probe.availability,
            lankide_harness::HarnessAvailability::Available
        ) {
            return Err(AppError::Invariant("production adapter unavailable"));
        }
        if registration.factory.info().harness_id == "dcc" {
            return Err(AppError::Invariant("dcc appeared as harness id"));
        }
    }

    let (incoming_tx, mut incoming_rx) = mpsc::unbounded_channel();
    let peer = Peer::new(incoming_tx);
    peer.state
        .resources
        .lock()
        .expect("resource lock")
        .insert(7, ("brief.txt".into(), b"resource-body".to_vec()));
    let (server_stop_tx, server_stop_rx) = oneshot::channel();
    let server_peer = peer.clone();
    let socket = paths.runtime_socket.clone();
    let server = tokio::spawn(async move {
        ws::serve_local(socket, server_peer, TransportConfig::default(), async {
            let _ = server_stop_rx.await;
        })
        .await
    });
    wait_socket(&paths.runtime_socket).await?;

    let outcome = timeout(HARD_TIMEOUT, async {
        let first = build_computer(&paths, "daemon-e2e-first", &executable, &pid_journal)?;
        let handle = first
            .spawn(&paths.runtime_socket)
            .await
            .map_err(AppError::Computer)?;
        wait_for("first Computer hello", || peer.state.hellos.lock().expect("hello lock").len() >= 1).await?;
        let hello = peer.state.hellos.lock().expect("hello lock")[0].clone();
        require(hello.computer_id == "local", "computer id was not local")?;
        require(
            hello.instance_id == "daemon-e2e-first",
            "instance id mismatch",
        )?;
        require(
            hello
                .harnesses
                .iter()
                .map(|item| item.harness_id.as_str())
                .collect::<Vec<_>>()
                == ["pi", "codex", "claude_code"],
            "catalog mismatch",
        )?;
        require(
            !hello.harnesses.iter().any(|item| item.harness_id == "dcc"),
            "dcc catalog entry",
        )?;
        let unavailable = hello
            .harnesses
            .iter()
            .find(|item| item.harness_id == "codex")
            .ok_or(AppError::Invariant("codex catalog entry missing"))?;
        require(
            unavailable.availability == lankide_protocol::HarnessAvailability::Unavailable as i32
                && !unavailable.reason.is_empty(),
            "unavailable probe diagnostic missing",
        )?;
        wait_heartbeat(&mut incoming_rx).await?;
        require(
            handle.status().await.connected,
            "computer status was disconnected",
        )?;

        for (id, harness) in [(1, "pi"), (2, "codex"), (3, "claude_code")] {
            let cli = if harness == "claude_code" {
                dcc_path.as_path()
            } else {
                executable.as_path()
            };
            let mut assignment = assign(
                id,
                &format!("agent_{id}"),
                harness,
                "quick",
                vec![],
                cli,
                &harness_log,
                "",
            );
            if id == 1
                && let Some(runtime_frame::Frame::AssignExecution(value)) = &mut assignment.frame
                && let Some(harness) = &mut value.harness
            {
                harness.executable_path.clear();
            }
            peer.send(assignment).await?;
            collect_terminal(&peer, &mut incoming_rx, id).await?;
        }
        peer.send(assign(
            10,
            "agent_1",
            "pi",
            "resume",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        collect_terminal(&peer, &mut incoming_rx, 10).await?;
        require(
            fs::read_to_string(&harness_log)?
                .lines()
                .any(|line| line.starts_with("resume\t")),
            "session was not resumed",
        )?;

        peer.send(assign(
            4,
            "serial",
            "pi",
            "hold-one",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        wait_assignment(&mut incoming_rx, 4, true).await?;
        wait_event_kind(&mut incoming_rx, 4, ExecutionEventKind::Started).await?;
        peer.send(assign(
            5,
            "serial",
            "pi",
            "second",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        let busy = wait_assignment(&mut incoming_rx, 5, false).await?;
        require(
            busy.error_code == "agent_busy",
            "same Agent was not serialized",
        )?;
        peer.send(assign(
            6,
            "parallel",
            "codex",
            "hold-two",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        wait_assignment(&mut incoming_rx, 6, true).await?;
        peer.send(cancel(4)).await?;
        peer.send(cancel(6)).await?;
        collect_parallel_terminals(&mut incoming_rx, [4, 6]).await?;

        peer.send(assign(
            7,
            "tools",
            "claude_code",
            "tool_wait resource://7",
            vec![resource_ref(7, b"resource-body")],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        wait_assignment(&mut incoming_rx, 7, true).await?;
        wait_tool_call(&mut incoming_rx, 7).await?;
        peer.send(tool_result(7)).await?;
        wait_tool_receipt(&mut incoming_rx, 7).await?;
        peer.send(steer(7, "finish-tool")).await?;
        collect_remaining_terminal(&mut incoming_rx, 7).await?;
        let downloaded = paths
            .workspaces
            .join("workspace/agents/tools/resources/7-brief.txt");
        require(
            fs::read(downloaded)? == b"resource-body",
            "resource download mismatch",
        )?;

        peer.send(assign(
            8,
            "replay",
            "pi",
            "replay",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        let events = collect_terminal(&peer, &mut incoming_rx, 8).await?;
        for event in events.iter().take(events.len().saturating_sub(1)) {
            acknowledge(&peer, event).await?;
        }
        peer.disconnect();
        tokio::select! {
            result = wait_for("Computer reconnect hello", || peer.state.hellos.lock().expect("hello lock").len() >= 2) => result?,
            result = handle.join() => {
                result.map_err(AppError::Computer)?;
                return Err(AppError::Invariant("Computer stopped before reconnect"));
            }
        }
        // An acknowledgement sent just before disconnect may still be in flight.
        // Earlier events can replay too; the unacknowledged terminal must arrive.
        let replayed = timeout(STEP, async {
            loop {
                let event = wait_event(&mut incoming_rx, 8).await?;
                require(events.contains(&event), "replay changed an execution event")?;
                if event.sequence == events.last().expect("terminal event").sequence {
                    return Ok::<_, AppError>(event);
                }
                acknowledge(&peer, &event).await?;
            }
        }).await.map_err(|_| AppError::Invariant("terminal replay timeout"))??;
        acknowledge(&peer, &replayed).await?;
        let sessions_before_duplicate = session_open_count(&harness_log)?;
        peer.send(assign(
            8,
            "replay",
            "pi",
            "replay",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        wait_assignment(&mut incoming_rx, 8, true).await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
        require(
            session_open_count(&harness_log)? == sessions_before_duplicate,
            "duplicate assignment started another session",
        )?;

        handle.shutdown("control_stop").await;
        timeout(STEP, handle.join())
            .await
            .map_err(|_| AppError::Invariant("control stop timeout"))?
            .map_err(AppError::Computer)?;
        let second = build_computer(&paths, "daemon-e2e-second", &executable, &pid_journal)?;
        let second_handle = second
            .spawn(&paths.runtime_socket)
            .await
            .map_err(AppError::Computer)?;
        wait_for("restarted Computer hello", || peer.state.hellos.lock().expect("hello lock").len() >= 3).await?;
        let restarted = peer.state.hellos.lock().expect("hello lock")[2].clone();
        require(
            restarted.instance_id == "daemon-e2e-second",
            "restart instance id did not change",
        )?;
        peer.send(assign(
            8,
            "replay",
            "pi",
            "replay",
            vec![],
            &executable,
            &harness_log,
            "",
        ))
        .await?;
        wait_assignment(&mut incoming_rx, 8, true).await?;
        tokio::time::sleep(Duration::from_millis(120)).await;
        require(
            session_open_count(&harness_log)? == sessions_before_duplicate,
            "restart replay started another session",
        )?;
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::Shutdown(lankide_protocol::Shutdown {
                reason: "runtime_shutdown".into(),
            })),
        })
        .await?;
        timeout(STEP, second_handle.join())
            .await
            .map_err(|_| AppError::Invariant("shutdown timeout"))?
            .map_err(AppError::Computer)?;
        let (url, credential) = lankide_protocol::link::discover(&paths.runtime_socket)?;
        fs::write(paths.home.join("computer-credentials.json"), serde_json::to_vec(&lankide_computer_daemon::DeviceCredentials {
            computer_id:"local".into(),device_credential:credential,runtime_url:url,local_discovery:true,
        })?)?;
        let mut child = Command::new(&executable)
            .arg("--home")
            .arg(&paths.home)
            .arg("--scripted-composition")
            .arg("--pid-journal")
            .arg(&pid_journal)
            .arg("--instance-id")
            .arg("daemon-e2e-signal")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        wait_for("signal daemon hello", || peer.state.hellos.lock().expect("hello lock").len() >= 4).await?;
        peer.send(assign(
            9,
            "signal",
            "pi",
            "hold-signal",
            vec![],
            &executable,
            &harness_log,
            "ignore_cancel",
        ))
        .await?;
        wait_assignment(&mut incoming_rx, 9, true).await?;
        wait_event_kind(&mut incoming_rx, 9, ExecutionEventKind::Started).await?;
        let signal_status = Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()?;
        require(signal_status.success(), "could not send SIGTERM")?;
        let exit = child.wait()?;
        require(exit.success(), "daemon did not exit cleanly after SIGTERM")?;
        wait_for("Computer ownership release", || !paths.instance_lock.exists()).await?;
        Ok::<(), AppError>(())
    })
    .await;

    let scenario = match outcome {
        Ok(result) => result,
        Err(_) => Err(AppError::Invariant("hard timeout")),
    };
    let _ = server_stop_tx.send(());
    let _ = timeout(STEP, server).await;
    let processes_clean = verify_pids(&pid_journal);
    let socket_clean = !paths.runtime_socket.exists();
    let status = scenario.is_ok() && processes_clean && socket_clean;
    let report = json!({
        "run_id": run_id,
        "module": "computer-daemon",
        "classification": "hermetic",
        "hard_timeout_ms": HARD_TIMEOUT.as_millis(),
        "duration_ms": started.elapsed().as_millis(),
        "scenarios": [
            "ready_catalog_probe_no_dcc", "heartbeat", "three_scripted_harnesses",
            "stale_instance_lock_recovery",
            "parallel_agents_serial_same_agent", "start_resume_steer_tool_cancel_resource",
            "runtime_disconnect_reconnect_replay", "duplicate_accept_noop",
            "daemon_restart_session_state", "runtime_shutdown", "control_start_stop",
            "sigterm_forced_reap_no_orphan"
        ],
        "cleanup": {"processes": processes_clean, "socket": socket_clean, "temporary_home": true},
        "status": if status { "passed" } else { "failed" },
        "error": scenario.as_ref().err().map(ToString::to_string),
    });
    fs::write(&artifact, serde_json::to_vec_pretty(&report)?)?;
    temporary.close()?;
    if let Err(error) = scenario {
        return Err(error);
    }
    require(processes_clean, "scripted process orphan")?;
    require(socket_clean, "runtime socket remained")?;
    Ok(artifact)
}

fn build_computer(
    paths: &ComputerDaemonPaths,
    instance_id: &str,
    executable: &Path,
    pid_journal: &Path,
) -> Result<Computer, AppError> {
    let harness_bin = paths.home.join("harness-bin");
    fs::create_dir_all(&harness_bin)?;
    let pi_executable = harness_bin.join("pi");
    if !pi_executable.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(executable, &pi_executable)?;
    }
    let mut computer = Computer::new(ComputerConfig {
        computer_id: "local".into(),
        instance_id: instance_id.into(),
        workspace_root: paths.workspaces.display().to_string(),
        reconnect: ReconnectPolicy {
            initial: Duration::from_millis(15),
            maximum: Duration::from_millis(80),
            multiplier: 2,
            ..Default::default()
        },
    })
    .with_device_credential(lankide_protocol::link::discover(&paths.runtime_socket)?.1)
    .with_execution_liveness(lankide_computer::ExecutionLiveness {
        cancel_grace: Duration::from_millis(150),
        ..Default::default()
    });
    for id in ["pi", "codex", "claude_code"] {
        let factory = ProcessHarnessFactory::new(
            NativeHarnessInfo::new(id, id)
                .map_err(|_| AppError::Invariant("invalid scripted harness info"))?,
            ProcessHarnessSpec {
                arguments: vec!["--scripted-harness".into()],
                bind_timeout: STEP,
                cancel_grace: Duration::from_millis(100),
                close_grace: Duration::from_millis(100),
                pid_journal: Some(pid_journal.to_owned()),
            },
        );
        computer
            .register_harness(RegisteredHarness::new(
                factory,
                NativeHarnessConfig {
                    harness_id: id.into(),
                    model: ModelRef::new("fixture", "fixture"),
                    thinking_level: "low".into(),
                    cli_path: if id == "codex" {
                        PathBuf::from("/missing/lankide-codex-e2e")
                    } else if id == "pi" {
                        PathBuf::from("pi")
                    } else {
                        executable.to_owned()
                    },
                    environment: if id == "pi" {
                        BTreeMap::from([("PATH".to_owned(), harness_bin.display().to_string())])
                    } else {
                        BTreeMap::new()
                    },
                    config_fingerprint: format!("probe-{id}"),
                },
            ))
            .map_err(AppError::Computer)?;
    }
    Ok(computer)
}

fn assign(
    id: i64,
    agent: &str,
    harness_id: &str,
    input: &str,
    resources: Vec<ResourceRef>,
    executable: &Path,
    log: &Path,
    fault: &str,
) -> RuntimeFrame {
    let mut environment_variables = vec![lankide_protocol::EnvironmentVariable {
        name: "COMPUTER_HARNESS_LOG".into(),
        value: log.display().to_string(),
    }];
    if !fault.is_empty() {
        environment_variables.push(lankide_protocol::EnvironmentVariable {
            name: "COMPUTER_HARNESS_FAULT".into(),
            value: fault.into(),
        });
    }
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::AssignExecution(AssignExecution {
            execution: Some(execution(id)),
            workspace: Some(WorkspaceRef {
                workspace_id: "workspace".into(),
            }),
            agent: Some(AgentRuntimeProfile {
                agent: Some(AgentRef {
                    workspace_id: "workspace".into(),
                    agent_id: agent.into(),
                }),
                name: agent.into(),
                role: "worker".into(),
                bio: String::new(),
            }),
            harness: Some(HarnessConfig {
                harness_id: harness_id.into(),
                executable_path: executable.display().to_string(),
                model: "fixture".into(),
                thinking_level: "low".into(),
                environment_variables,
            }),
            session: Some(SessionDirective {
                logical_session_id: format!("session-{agent}"),
                action: SessionAction::ResumeOrStart.into(),
                native_session_id: None,
                config_fingerprint: "fp-1".into(),
            }),
            initial_delivery: Some(Delivery {
                delivery_id: id * 10,
                kind: DeliveryKind::Initial.into(),
                rendered_markdown: input.into(),
                snapshot_id: id,
                trigger_reasons: vec!["e2e".into()],
                resources: resources.clone(),
                created_at: 1,
            }),
            system_prompt: "fixture system".into(),
            tools: vec![ToolDefinition {
                name: "echo".into(),
                description: "echo".into(),
                input_schema_json: "{}".into(),
                output_schema_json: "{}".into(),
            }],
            cli_commands: Vec::new(),
            resources,
            execution_token: format!("token-{id}"),
            created_at: 1,
        })),
    }
}

fn execution(id: i64) -> ExecutionRef {
    ExecutionRef {
        execution_id: id,
        attempt: 1,
    }
}
fn cancel(id: i64) -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::CancelExecution(
            lankide_protocol::CancelExecution {
                force_close: false,
                execution: Some(execution(id)),
                reason: "cancel".into(),
            },
        )),
    }
}
fn steer(id: i64, input: &str) -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
            execution: Some(execution(id)),
            delivery: Some(Delivery {
                delivery_id: id * 100,
                kind: DeliveryKind::Steer.into(),
                rendered_markdown: input.into(),
                snapshot_id: id,
                trigger_reasons: Vec::new(),
                resources: Vec::new(),
                created_at: 1,
            }),
        })),
    }
}
fn tool_result(id: i64) -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::ToolResult(ToolResult {
            execution: Some(execution(id)),
            tool_call_id: "call-1".into(),
            success: true,
            content_markdown: "done".into(),
            structured_content_json: "{}".into(),
            resources: Vec::new(),
            error: None,
        })),
    }
}
fn resource_ref(id: i64, bytes: &[u8]) -> ResourceRef {
    ResourceRef {
        resource_id: id,
        file_name: "brief.txt".into(),
        media_type: "text/plain".into(),
        size_bytes: bytes.len() as u64,
        sha256: hex(&Sha256::digest(bytes)),
    }
}

async fn wait_socket(path: &Path) -> Result<(), AppError> {
    wait_for("Runtime socket readiness", || path.exists()).await
}
async fn wait_for(
    description: &'static str,
    mut predicate: impl FnMut() -> bool,
) -> Result<(), AppError> {
    for _ in 0..400 {
        if predicate() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(AppError::Invariant(description))
}
async fn next_frame(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
) -> Result<ComputerFrame, AppError> {
    timeout(STEP, receiver.recv())
        .await
        .map_err(|_| AppError::Invariant("frame timeout"))?
        .ok_or(AppError::Invariant("frame channel closed"))
}
async fn wait_heartbeat(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
) -> Result<(), AppError> {
    loop {
        if matches!(
            next_frame(receiver).await?.frame,
            Some(computer_frame::Frame::Heartbeat(_))
        ) {
            return Ok(());
        }
    }
}
async fn wait_assignment(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
    accepted: bool,
) -> Result<AssignmentResult, AppError> {
    loop {
        if let Some(computer_frame::Frame::AssignmentResult(result)) =
            next_frame(receiver).await?.frame
        {
            if result
                .execution
                .as_ref()
                .is_some_and(|value| value.execution_id == id)
            {
                require(
                    result.accepted == accepted,
                    "assignment acceptance mismatch",
                )?;
                return Ok(result);
            }
        }
    }
}
async fn wait_event(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
) -> Result<ExecutionEvent, AppError> {
    loop {
        if let Some(computer_frame::Frame::ExecutionEvent(event)) =
            next_frame(receiver).await?.frame
        {
            if event
                .execution
                .as_ref()
                .is_some_and(|value| value.execution_id == id)
            {
                return Ok(event);
            }
        }
    }
}
async fn wait_event_kind(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
    kind: ExecutionEventKind,
) -> Result<ExecutionEvent, AppError> {
    loop {
        let event = wait_event(receiver, id).await?;
        if event.kind == kind as i32 {
            return Ok(event);
        }
    }
}
async fn collect_terminal(
    _peer: &Peer,
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
) -> Result<Vec<ExecutionEvent>, AppError> {
    wait_assignment(receiver, id, true).await?;
    collect_remaining_terminal(receiver, id).await
}
async fn collect_remaining_terminal(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
) -> Result<Vec<ExecutionEvent>, AppError> {
    let mut events = Vec::new();
    loop {
        let event = wait_event(receiver, id).await?;
        let terminal = matches!(
            ExecutionEventKind::try_from(event.kind).ok(),
            Some(
                ExecutionEventKind::Completed
                    | ExecutionEventKind::Failed
                    | ExecutionEventKind::Cancelled
            )
        );
        events.push(event);
        if terminal {
            return Ok(events);
        }
    }
}
async fn collect_parallel_terminals(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    ids: [i64; 2],
) -> Result<(), AppError> {
    let mut terminal = HashMap::from([(ids[0], false), (ids[1], false)]);
    while terminal.values().any(|done| !done) {
        let frame = next_frame(receiver).await?;
        let Some(computer_frame::Frame::ExecutionEvent(event)) = frame.frame else {
            continue;
        };
        let Some(id) = event
            .execution
            .as_ref()
            .map(|execution| execution.execution_id)
        else {
            continue;
        };
        let Some(done) = terminal.get_mut(&id) else {
            continue;
        };
        if matches!(
            ExecutionEventKind::try_from(event.kind).ok(),
            Some(
                ExecutionEventKind::Completed
                    | ExecutionEventKind::Failed
                    | ExecutionEventKind::Cancelled
            )
        ) {
            *done = true;
        }
    }
    Ok(())
}
async fn wait_tool_call(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
) -> Result<(), AppError> {
    loop {
        if let Some(computer_frame::Frame::CapabilityRequest(call)) =
            next_frame(receiver).await?.frame
        {
            if call.method == "invoke_capability"
                && call
                    .context
                    .as_ref()
                    .and_then(|context| context.execution.as_ref())
                    .is_some_and(|value| value.execution_id == id)
            {
                return Ok(());
            }
        }
    }
}
async fn wait_tool_receipt(
    receiver: &mut mpsc::UnboundedReceiver<ComputerFrame>,
    id: i64,
) -> Result<(), AppError> {
    let expected = format!("tool_result:{id}:1:call-1");
    loop {
        if let Some(computer_frame::Frame::ResourceTransferResult(receipt)) =
            next_frame(receiver).await?.frame
        {
            if receipt.transfer_id == expected {
                return require(receipt.success, "tool result rejected");
            }
        }
    }
}
async fn acknowledge(peer: &Peer, event: &ExecutionEvent) -> Result<(), AppError> {
    peer.send(RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::ExecutionEventAck(ExecutionEventAck {
            execution: event.execution.clone(),
            max_saved_sequence: event.sequence,
        })),
    })
    .await
}
fn require(value: bool, message: &'static str) -> Result<(), AppError> {
    if value {
        Ok(())
    } else {
        Err(AppError::Invariant(message))
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn unique_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("computer-daemon-{}-{nanos}", std::process::id())
}
fn verify_pids(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
        return true;
    };
    let mut live = HashMap::<u32, bool>::new();
    for line in contents.lines() {
        let mut fields = line.split('\t');
        let action = fields.next().unwrap_or("");
        let pid = fields
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        if pid > 0 {
            live.insert(pid, action == "spawned");
        }
    }
    !live.values().any(|value| *value)
}

fn session_open_count(path: &Path) -> Result<usize, AppError> {
    Ok(fs::read_to_string(path)?
        .lines()
        .filter(|line| line.starts_with("start\t") || line.starts_with("resume\t"))
        .count())
}
