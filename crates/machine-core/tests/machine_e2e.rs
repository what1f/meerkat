#[path = "../../harness/tests/support/process.rs"]
mod process_support;
#[path = "../../protocol/tests/support/ws.rs"]
mod ws;
use process_support::{ProcessHarnessFactory, ProcessHarnessSpec};

use lankide_harness::{
    AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest, HarnessAvailability,
    HarnessConfig as NativeHarnessConfig, HarnessError, HarnessErrorCode, HarnessFactory,
    HarnessFuture, HarnessInfo as NativeHarnessInfo, HarnessSession, ModelRef, ProbeResult,
    SessionEntry, SessionHealth, SessionRequest, SteerReceipt, ToolResultReceipt,
    ToolResultRequest,
};
use lankide_machine_core::{
    Machine, MachineConfig, ReconnectPolicy, RegisteredHarness, agent_directory,
    workspace_directory,
};
use lankide_protocol::{
    AgentRef, AgentRuntimeProfile, AssignExecution, AssignmentResult, DeliverSteer, Delivery,
    DeliveryKind, ExecutionCommitted, ExecutionEvent, ExecutionEventAck, ExecutionEventKind,
    ExecutionRef, HarnessConfig, MachineFrame, MachineFrameStream, MachineHello, ResourceChunk,
    ResourceChunkStream, ResourceDescriptor, ResourceRef, ResourceRequest, RuntimeFrame,
    RuntimeFrameStream, RuntimeHello, RuntimePeer, SessionAction, SessionDirective, ToolDefinition,
    TransportConfig, WorkspaceRef, machine_frame, runtime_frame,
};

use lankide_protocol::Status;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

const STEP: Duration = Duration::from_secs(4);

struct RejectingFactory {
    info: NativeHarnessInfo,
    closed: Arc<AtomicBool>,
}

impl HarnessFactory for RejectingFactory {
    fn info(&self) -> &NativeHarnessInfo {
        &self.info
    }

    fn probe<'a>(&'a self, _: &'a NativeHarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            Ok(ProbeResult {
                info: self.info.clone(),
                availability: HarnessAvailability::Available,
                version: Some("machine-e2e".into()),
                capabilities: Default::default(),
                reason: None,
            })
        })
    }

    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move {
            Ok(Box::new(RejectingSession {
                entry: SessionEntry {
                    harness_id: request.config.harness_id,
                    native_session_id: "native-rejected".into(),
                    config_fingerprint: request.config.config_fingerprint,
                    health: SessionHealth::Healthy,
                },
                closed: self.closed.clone(),
            }) as Box<dyn HarnessSession>)
        })
    }

    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        self.start(request)
    }
}

struct RejectingSession {
    entry: SessionEntry,
    closed: Arc<AtomicBool>,
}

impl HarnessSession for RejectingSession {
    fn entry(&self) -> SessionEntry {
        self.entry.clone()
    }

    fn execute<'a>(&'a self, _: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async {
            Err(HarnessError::new(
                HarnessErrorCode::InvalidState,
                "first delivery rejected",
            ))
        })
    }

    fn steer<'a>(&'a self, _: String, _: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async {
            Err(HarnessError::new(
                HarnessErrorCode::InvalidState,
                "not active",
            ))
        })
    }

    fn tool_result<'a>(&'a self, _: ToolResultRequest) -> HarnessFuture<'a, ToolResultReceipt> {
        Box::pin(async {
            Err(HarnessError::new(
                HarnessErrorCode::InvalidState,
                "not active",
            ))
        })
    }

    fn cancel<'a>(&'a self, _: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async { Ok(CancelReceipt { accepted: false }) })
    }

    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            Ok(CloseReceipt {
                already_closed: false,
            })
        })
    }
}

#[derive(Clone)]
struct Peer {
    state: Arc<PeerState>,
}

struct PeerState {
    incoming: mpsc::UnboundedSender<MachineFrame>,
    runtime: Mutex<Option<mpsc::Sender<Result<RuntimeFrame, Status>>>>,
    hellos: Mutex<Vec<MachineHello>>,
    resources: Mutex<HashMap<i64, (String, Vec<u8>)>>,
    uploads: Mutex<Vec<Vec<u8>>>,
    calls: Mutex<HashMap<String, String>>,
}

impl Peer {
    fn new(incoming: mpsc::UnboundedSender<MachineFrame>) -> Self {
        Self {
            state: Arc::new(PeerState {
                incoming,
                runtime: Mutex::new(None),
                hellos: Mutex::new(Vec::new()),
                resources: Mutex::new(HashMap::new()),
                uploads: Mutex::new(Vec::new()),
                calls: Mutex::new(HashMap::new()),
            }),
        }
    }

    async fn send(&self, mut frame: RuntimeFrame) {
        if let Some(runtime_frame::Frame::CapabilityResponse(response)) = frame.frame.as_mut() {
            if let Some(id) = self.state.calls.lock().unwrap().get(&response.request_id) {
                response.request_id = id.clone();
            }
        }
        for _ in 0..100 {
            let sender = self.state.runtime.lock().expect("runtime mutex").clone();
            if let Some(sender) = sender {
                sender.send(Ok(frame)).await.expect("send runtime frame");
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("machine did not connect");
    }

    fn disconnect(&self) {
        self.state.runtime.lock().expect("runtime mutex").take();
    }
}

#[async_trait::async_trait]
impl lankide_protocol::MachinePeer for Peer {
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

    async fn connect_machine(
        &self,
        hello: MachineHello,
        mut frames: MachineFrameStream,
    ) -> Result<RuntimeFrameStream, Status> {
        self.state.hellos.lock().expect("hello mutex").push(hello);
        let incoming = self.state.incoming.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            while let Some(frame) = frames.next().await {
                match frame {
                    Ok(frame) => {
                        if let Some(machine_frame::Frame::CapabilityRequest(request)) = &frame.frame
                        {
                            assert_eq!(request.method, "invoke_capability");
                            assert!(request.eof);
                            let call = lankide_protocol::agent::decode::<
                                lankide_protocol::CapabilityInvokeRequest,
                            >(request.payload.as_slice())
                            .expect("capability request");
                            let context = call.context.expect("run context");
                            let run = context.execution.unwrap();
                            state.calls.lock().unwrap().insert(
                                format!(
                                    "{}:{}:{}",
                                    run.execution_id, run.attempt, call.invocation_id
                                ),
                                request.request_id.clone(),
                            );
                        }
                        let _ = incoming.send(frame);
                    }
                    Err(_) => break,
                }
            }
        });
        let (sender, receiver) = mpsc::channel(64);
        let hello = RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::Hello(RuntimeHello {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                runtime_instance_id: "runtime-machine-e2e".into(),
                heartbeat_interval_ms: 500,
                max_message_bytes: lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES as u64,
            })),
        };
        sender
            .send(Ok(hello))
            .await
            .map_err(|_| Status::unavailable("stream closed"))?;
        *self.state.runtime.lock().expect("runtime mutex") = Some(sender);
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }
}

#[async_trait::async_trait]
impl RuntimePeer for Peer {
    async fn upload_resource(
        &self,
        mut chunks: ResourceChunkStream,
    ) -> Result<ResourceRef, Status> {
        let mut bytes = Vec::new();
        let mut descriptor = None;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            if descriptor.is_none() {
                descriptor = chunk.descriptor;
            }
            bytes.extend_from_slice(&chunk.bytes);
        }
        let descriptor = descriptor.ok_or_else(|| Status::invalid_argument("descriptor"))?;
        self.state
            .uploads
            .lock()
            .expect("uploads mutex")
            .push(bytes);
        Ok(ResourceRef {
            resource_id: 900 + self.state.uploads.lock().expect("uploads mutex").len() as i64,
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
        let (name, bytes) = self
            .state
            .resources
            .lock()
            .expect("resources mutex")
            .get(&request.resource_id)
            .cloned()
            .ok_or_else(|| Status::not_found("resource"))?;
        let sha = hex(&Sha256::digest(&bytes));
        let split = bytes.len().min(3);
        let descriptor = ResourceDescriptor {
            file_name: name,
            media_type: "text/plain".into(),
            size_bytes: bytes.len() as u64,
            sha256: sha,
            context: request.context,
        };
        let first = ResourceChunk {
            transfer_id: request.transfer_id.clone(),
            resource_id: request.resource_id,
            offset: 0,
            bytes: bytes[..split].to_vec(),
            eof: split == bytes.len(),
            descriptor: Some(descriptor),
        };
        let mut chunks = vec![Ok(first)];
        if split < bytes.len() {
            chunks.push(Ok(ResourceChunk {
                transfer_id: request.transfer_id,
                resource_id: request.resource_id,
                offset: split as u64,
                bytes: bytes[split..].to_vec(),
                eof: true,
                descriptor: None,
            }));
        }
        Ok(Box::pin(tokio_stream::iter(chunks)))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn machine_e2e() {
    let run_id = format!("{}-{}", std::process::id(), now_nanos());
    let started = Instant::now();
    // Keep the UDS path below macOS's sockaddr_un limit.
    let temp = PathBuf::from(format!("/tmp/lk-c-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).expect("temp root");
    let workspace_root = temp.join("workspaces");
    let socket = temp.join("run");
    let artifact_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e-artifacts/machine")
        .join(&run_id);
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let pid_journal = artifact_root.join("pids.tsv");
    let harness_log = PathBuf::from(artifact_harness_log());
    let _ = fs::remove_file(&harness_log);
    let result_path = artifact_root.join("result.json");
    let (incoming_tx, mut incoming_rx) = mpsc::unbounded_channel();
    let peer = Peer::new(incoming_tx);
    peer.state
        .resources
        .lock()
        .expect("resources mutex")
        .insert(7, ("brief.txt".into(), b"resource-body".to_vec()));
    peer.state
        .resources
        .lock()
        .expect("resources mutex")
        .insert(70, ("brief.txt".into(), b"resource-body".to_vec()));
    peer.state
        .resources
        .lock()
        .expect("resources mutex")
        .insert(8, ("tool.txt".into(), b"tool-resource".to_vec()));
    let (server_stop_tx, server_stop_rx) = oneshot::channel();
    let server_peer = peer.clone();
    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        ws::serve_local(
            server_socket,
            server_peer,
            TransportConfig::default(),
            async {
                let _ = server_stop_rx.await;
            },
        )
        .await
    });
    wait_socket(&socket).await;

    let cli_dir = temp.join("capability-bin");
    fs::create_dir_all(&cli_dir).unwrap();
    let capability_cli = cli_dir.join("lankide");
    fs::write(
        &capability_cli,
        "#!/bin/sh\nprintf '%s' \"$LANKIDE_EXECUTION_ID\"\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&capability_cli, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut machine = Machine::new(MachineConfig {
        machine_id: "fixture-machine".into(),
        instance_id: format!("machine-e2e-{run_id}"),
        workspace_root: workspace_root.display().to_string(),
        reconnect: ReconnectPolicy {
            initial: Duration::from_millis(20),
            maximum: Duration::from_millis(100),
            multiplier: 2,
            ..Default::default()
        },
    })
    .with_device_credential(lankide_protocol::link::discover(&socket).unwrap().1)
    .with_capability_cli(&capability_cli)
    .with_execution_liveness(lankide_machine_core::ExecutionLiveness {
        cancel_grace: Duration::from_millis(150),
        ..Default::default()
    });
    for id in ["pi", "codex"] {
        let factory = ProcessHarnessFactory::new(
            NativeHarnessInfo::new(id, id).expect("harness info"),
            ProcessHarnessSpec {
                arguments: Vec::new(),
                bind_timeout: STEP,
                cancel_grace: Duration::from_millis(80),
                close_grace: Duration::from_millis(80),
                pid_journal: Some(pid_journal.clone()),
            },
        );
        machine
            .register_harness(RegisteredHarness::new(
                factory,
                NativeHarnessConfig {
                    launch_arguments: Default::default(),
                    harness_id: id.into(),
                    model: ModelRef::new("fixture", "fixture"),
                    thinking_level: "low".into(),
                    cli_path: env!("CARGO_BIN_EXE_machine-harness-peer").into(),
                    environment: Default::default(),
                    config_fingerprint: "probe".into(),
                },
            ))
            .expect("register harness");
    }
    let rejecting_closed = Arc::new(AtomicBool::new(false));
    machine
        .register_harness(RegisteredHarness::new(
            RejectingFactory {
                info: NativeHarnessInfo::new("claude_code", "claude_code")
                    .expect("rejecting harness info"),
                closed: rejecting_closed.clone(),
            },
            NativeHarnessConfig {
                launch_arguments: Default::default(),
                harness_id: "claude_code".into(),
                model: ModelRef::new("fixture", "fixture"),
                thinking_level: "low".into(),
                cli_path: "/usr/bin/false".into(),
                environment: Default::default(),
                config_fingerprint: "probe".into(),
            },
        ))
        .expect("register rejecting harness");
    let (machine_stop_tx, machine_stop_rx) = oneshot::channel();
    let host_socket = socket.clone();
    let machine_task = tokio::spawn(async move {
        machine
            .run(host_socket, async {
                let _ = machine_stop_rx.await;
            })
            .await
    });

    let outcome = timeout(Duration::from_secs(30), async {
        wait_for(|| peer.state.hellos.lock().expect("hello mutex").len() >= 1).await;
        let hello = peer.state.hellos.lock().expect("hello mutex")[0].clone();
        assert_eq!(hello.machine_id, "fixture-machine");
        assert_eq!(
            hello
                .harnesses
                .iter()
                .map(|h| h.harness_id.as_str())
                .collect::<Vec<_>>(),
            ["pi", "codex", "claude_code"]
        );
        assert!(hello.harnesses.iter().all(|harness| {
            harness.model_catalog.is_none() && !harness.model_catalog_error.is_empty()
        }));

        peer.send(runtime_assign_with_harness(
            21,
            "rejecting",
            "session-rejecting",
            "fp-rejecting",
            "first delivery",
            "claude_code",
        ))
        .await;
        let rejected = wait_assignment(&mut incoming_rx, 21, false).await;
        assert_eq!(rejected.error_code, "harness_error");
        assert!(rejecting_closed.load(Ordering::SeqCst));
        peer.send(runtime_assign(
            22,
            "rejecting",
            "session-after-rejection",
            "fp-after-rejection",
            "accepted after rejection",
            vec![],
            "",
        ))
        .await;
        collect_terminal(&peer, &mut incoming_rx, 22, true).await;

        let content_home =
            agent_directory(&workspace_root, "alpha_team", "reader").expect("agent path");
        fs::create_dir_all(&content_home).expect("content home");
        fs::write(content_home.join("prompt.md"), "content-from-file").expect("content file");
        peer.send(runtime_assign(
            1,
            "reader",
            "session-reader",
            "fp-1",
            "content_file:prompt.md",
            vec![],
            "",
        ))
        .await;
        let reader_events = collect_terminal(&peer, &mut incoming_rx, 1, true).await;
        assert!(payloads(&reader_events).contains("content-from-file"));
        assert_eq!(
            fs::read_to_string(content_home.join("AGENTS.md")).unwrap(),
            "  性格原文\n\nBe direct.\n"
        );
        let diagnostics_path = workspace_root.join(".machine/diagnostics/1-1/diagnostics.json");
        let diagnostics: serde_json::Value =
            serde_json::from_slice(&fs::read(&diagnostics_path).expect("attempt diagnostics"))
                .expect("diagnostics JSON");
        assert_eq!(diagnostics["execution_id"], 1);
        assert_eq!(diagnostics["attempt"], 1);
        assert_eq!(
            diagnostics["output_record"],
            workspace_root
                .join(".machine/diagnostics/1-1/rpc.jsonl")
                .display()
                .to_string()
        );

        fs::write(
            content_home.join("tool-message.md"),
            "content-from-tool-file",
        )
        .expect("tool content file");
        fs::write(content_home.join("AGENTS.md"), "stale personality").unwrap();
        let mut updated_personality = runtime_assign(
            17,
            "reader",
            "session-reader",
            "fp-1",
            "message_send_file tool_wait",
            vec![],
            "",
        );
        if let Some(runtime_frame::Frame::AssignExecution(assign)) = &mut updated_personality.frame
        {
            assign.agent.as_mut().unwrap().soul = "  Updated personality.\n".into();
        }
        peer.send(updated_personality).await;
        let arguments = wait_specific_tool_call(&peer, &mut incoming_rx, 17, "message_send").await;
        assert_eq!(
            fs::read_to_string(content_home.join("AGENTS.md")).unwrap(),
            "  Updated personality.\n"
        );
        assert_eq!(arguments["content"], "content-from-tool-file");
        assert!(arguments.get("content_file").is_none());
        peer.send(runtime_tool_result(execution(17), "call-1", "sent", vec![]))
            .await;
        assert_tool_receipt(&mut incoming_rx, 17, 1, "call-1", true, "").await;
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
                execution: Some(execution(17)),
                delivery: Some(delivery(170, "finish-tool")),
            })),
        })
        .await;
        collect_remaining_terminal(&peer, &mut incoming_rx, 17).await;

        std::fs::File::create(content_home.join("large.md"))
            .expect("large content file")
            .set_len(lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES as u64 + 1)
            .expect("size large content file");
        fs::write(content_home.join("invalid.md"), [0xff, 0xfe])
            .expect("invalid UTF-8 content file");
        for (id, input, code) in [
            (
                18,
                "message_send_missing tool_wait",
                "content_file_unreadable",
            ),
            (19, "message_send_large tool_wait", "content_file_too_large"),
            (
                20,
                "message_send_utf8 tool_wait",
                "content_file_invalid_utf8",
            ),
        ] {
            peer.send(runtime_assign(
                id,
                "reader",
                "session-reader",
                "fp-1",
                input,
                vec![],
                "",
            ))
            .await;
            finish_local_tool_error(&peer, &mut incoming_rx, id, code).await;
        }

        let memory = agent_directory(&workspace_root, "alpha_team", "agent_a")
            .unwrap()
            .join("memory");
        fs::create_dir_all(&memory).unwrap();
        fs::write(
            memory.join("MEMORY.md"),
            format!(
                "Use the green release channel.\n{}DO_NOT_PRELOAD",
                "中".repeat(1600)
            ),
        )
        .unwrap();
        let resource = resource_ref(7, b"resource-body");
        peer.send(runtime_assign(
            2,
            "agent_a",
            "shared",
            "fp-1",
            "read resource://7 resource://70 attachment://7 attachment://70 resource://700 attachment://700",
            vec![resource, resource_ref(70, b"resource-body")],
            "",
        ))
        .await;
        let first_events = collect_terminal(&peer, &mut incoming_rx, 2, false).await;
        assert_ordered(&first_events);
        assert!(
            first_events
                .iter()
                .any(|event| event.kind == ExecutionEventKind::Completed as i32)
        );
        assert!(
            first_events
                .iter()
                .any(|event| String::from_utf8_lossy(&event.payload)
                    .contains("Use the green release channel.")),
            "memory index must reach the Harness input"
        );
        assert!(
            first_events
                .iter()
                .any(|event| String::from_utf8_lossy(&event.payload)
                    .contains("索引超过本轮预载上限"))
        );
        assert!(
            !first_events
                .iter()
                .any(|event| String::from_utf8_lossy(&event.payload).contains("DO_NOT_PRELOAD"))
        );
        let agent_home = agent_directory(&workspace_root, "alpha_team", "agent_a")
            .unwrap().canonicalize().unwrap();
        let expected = format!(
            "read {0} {1} {0} {1} resource://700 attachment://700",
            agent_home.join("resources/7-brief.txt").display(),
            agent_home.join("resources/70-brief.txt").display(),
        );
        assert!(first_events.iter().any(|event| String::from_utf8_lossy(&event.payload).contains(&expected)),
            "both resource ID paths must reach the Harness without rewriting longer IDs: {}", payloads(&first_events));
        fs::remove_file(memory.join("MEMORY.md")).unwrap();
        let downloaded = agent_directory(&workspace_root, "alpha_team", "agent_a")
            .unwrap()
            .join("resources/7-brief.txt");
        assert_eq!(
            fs::read(downloaded).expect("downloaded resource"),
            b"resource-body"
        );

        peer.send(runtime_assign(
            3,
            "agent_a",
            "shared",
            "fp-1",
            "hold-a",
            vec![],
            "",
        ))
        .await;
        wait_assignment(&mut incoming_rx, 3, true).await;
        wait_started(&mut incoming_rx, 3).await;
        peer.send(runtime_assign(
            4,
            "agent_a",
            "busy",
            "fp-1",
            "busy",
            vec![],
            "",
        ))
        .await;
        let busy = wait_assignment(&mut incoming_rx, 4, false).await;
        assert_eq!(busy.error_code, "agent_busy");
        let failure = busy.failure.expect("structured ownership conflict");
        assert_eq!(failure.category, "conflict");
        assert!(failure.retryable);
        peer.send(runtime_assign(
            5,
            "agent_b",
            "other",
            "fp-1",
            "hold-b",
            vec![],
            "ignore_cancel",
        ))
        .await;
        wait_assignment(&mut incoming_rx, 5, true).await;
        wait_started(&mut incoming_rx, 5).await;
        let recovering = fs::read_to_string(workspace_root.join(".machine/state.json"))
            .expect("running recovery state");
        assert!(recovering.contains("interrupted"));
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
                execution: Some(execution(3)),
                delivery: Some(delivery(33, "urgent")),
            })),
        })
        .await;
        wait_delivery(&mut incoming_rx, 3).await;
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
                execution: Some(execution(3)),
                delivery: Some(delivery(33, "urgent")),
            })),
        })
        .await;
        wait_delivery(&mut incoming_rx, 3).await;
        peer.send(cancel(3)).await;
        peer.send(cancel(5)).await;
        // Parallel cancellation may publish either terminal first. Observe and
        // acknowledge both without discarding the other attempt's outcome.
        timeout(STEP, async {
            let mut stopped = HashSet::new();
            while stopped.len() < 2 {
                let frame = incoming_rx.recv().await.expect("machine stream");
                if let Some(machine_frame::Frame::ExecutionEvent(event)) = frame.frame {
                    let id = event.execution.as_ref().expect("execution").execution_id;
                    ack(&peer, &event).await;
                    if [3, 5].contains(&id)
                        && matches!(
                            ExecutionEventKind::try_from(event.kind).unwrap(),
                            ExecutionEventKind::Completed
                                | ExecutionEventKind::Failed
                                | ExecutionEventKind::Cancelled
                        )
                    {
                        stopped.insert(id);
                    }
                }
            }
        })
        .await
        .expect("both cancelled attempts must stop");

        // Treat the original steer receipts as lost. A terminal replay must
        // carry the persisted acceptance first, without a Runtime retry request.
        let connections = peer.state.hellos.lock().unwrap().len();
        peer.disconnect();
        wait_for(|| peer.state.hellos.lock().unwrap().len() > connections).await;
        timeout(STEP, async {
            let mut accepted = false;
            loop {
                let frame = incoming_rx.recv().await.expect("reconnected stream");
                match frame.frame {
                    Some(machine_frame::Frame::DeliveryResult(receipt))
                        if receipt.delivery_id == 33 =>
                    {
                        assert!(receipt.accepted);
                        accepted = true;
                    }
                    Some(machine_frame::Frame::ExecutionEvent(event))
                        if event
                            .execution
                            .as_ref()
                            .is_some_and(|run| run.execution_id == 3)
                            && matches!(
                                ExecutionEventKind::try_from(event.kind).unwrap(),
                                ExecutionEventKind::Completed
                                    | ExecutionEventKind::Cancelled
                                    | ExecutionEventKind::Failed
                            ) =>
                    {
                        assert!(
                            accepted,
                            "terminal replay overtook the accepted steer receipt"
                        );
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("steer acceptance and terminal must replay");

        peer.send(runtime_assign(
            6,
            "agent_a",
            "shared",
            "fp-1",
            "resume",
            vec![],
            "",
        ))
        .await;
        collect_terminal(&peer, &mut incoming_rx, 6, true).await;
        peer.send(runtime_assign(
            7,
            "agent_a",
            "shared",
            "fp-new",
            "new fingerprint",
            vec![],
            "",
        ))
        .await;
        collect_terminal(&peer, &mut incoming_rx, 7, true).await;
        let harness_ops = fs::read_to_string(&harness_log).expect("harness log");
        assert!(harness_ops.lines().any(|line| line.starts_with("resume\t")));
        assert!(
            harness_ops
                .lines()
                .filter(|line| line.starts_with("start\t"))
                .count()
                >= 2
        );
        peer.send(runtime_assign(
            8,
            "replay",
            "replay",
            "fp-1",
            "replay",
            vec![],
            "",
        ))
        .await;
        let replay_events = collect_terminal(&peer, &mut incoming_rx, 8, false).await;
        let terminal = replay_events.last().expect("terminal").clone();
        for event in &replay_events[..replay_events.len() - 1] {
            ack(&peer, event).await;
        }
        let connections = peer.state.hellos.lock().unwrap().len();
        peer.disconnect();
        wait_for(|| peer.state.hellos.lock().expect("hello mutex").len() > connections).await;
        let replayed = wait_replayed_terminal(&mut incoming_rx, &replay_events).await;
        assert_eq!(replayed.sequence, terminal.sequence);
        ack(&peer, &replayed).await;
        let connections = peer.state.hellos.lock().unwrap().len();
        peer.disconnect();
        wait_for(|| peer.state.hellos.lock().expect("hello mutex").len() > connections).await;
        let replayed_after_ack = wait_replayed_terminal(&mut incoming_rx, &replay_events).await;
        assert_eq!(replayed_after_ack.sequence, terminal.sequence);
        peer.send(runtime_assign(
            8,
            "replay",
            "replay",
            "fp-1",
            "conflicting duplicate",
            vec![],
            "",
        ))
        .await;
        assert_eq!(
            wait_assignment(&mut incoming_rx, 8, false).await.error_code,
            "assignment_conflict"
        );
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::ExecutionCommitted(
                ExecutionCommitted {
                    execution: replayed_after_ack.execution.clone(),
                    committed_at: 1,
                },
            )),
        })
        .await;
        wait_for(|| {
            fs::read_to_string(workspace_root.join(".machine/state.json"))
                .ok()
                .and_then(|state| serde_json::from_str::<serde_json::Value>(&state).ok())
                .and_then(|state| state.get("pending_events").cloned())
                .and_then(|events| events.get("8:1").cloned())
                .is_none()
        })
        .await;

        peer.send(runtime_assign(
            9,
            "badseq",
            "badseq",
            "fp-1",
            "bad",
            vec![],
            "bad_sequence",
        ))
        .await;
        let bad = collect_terminal(&peer, &mut incoming_rx, 9, true).await;
        assert_eq!(bad.last().unwrap().kind, ExecutionEventKind::Failed as i32);
        peer.send(runtime_assign(
            10,
            "streamcut",
            "streamcut",
            "fp-1",
            "cut",
            vec![],
            "stream_cut",
        ))
        .await;
        let cut = collect_terminal(&peer, &mut incoming_rx, 10, true).await;
        assert_eq!(cut.last().unwrap().kind, ExecutionEventKind::Failed as i32);

        peer.send(runtime_assign(
            23,
            "completion_retry",
            "completion_retry",
            "fp-1",
            "completion_retry",
            vec![],
            "",
        ))
        .await;
        let corrected = collect_terminal(&peer, &mut incoming_rx, 23, true).await;
        assert_ordered(&corrected);
        assert_eq!(
            corrected.last().unwrap().kind,
            ExecutionEventKind::Completed as i32
        );
        assert_eq!(
            corrected
                .iter()
                .filter(|event| event.kind == ExecutionEventKind::Started as i32)
                .count(),
            2
        );
        assert!(payloads(&corrected).contains(r#""final_output":"OK""#));

        peer.send(runtime_assign(
            24,
            "completion_fail",
            "completion_fail",
            "fp-1",
            "completion_fail",
            vec![],
            "",
        ))
        .await;
        let correction_failed = collect_terminal(&peer, &mut incoming_rx, 24, true).await;
        assert_ordered(&correction_failed);
        assert_eq!(
            correction_failed.last().unwrap().kind,
            ExecutionEventKind::Failed as i32
        );
        assert_eq!(
            correction_failed
                .iter()
                .filter(|event| event.kind == ExecutionEventKind::Started as i32)
                .count(),
            4
        );
        assert!(payloads(&correction_failed).contains("completion_protocol"));

        let mut bad_resource = resource_ref(7, b"resource-body");
        bad_resource.sha256 = "0".repeat(64);
        peer.send(runtime_assign(
            12,
            "badsha",
            "badsha",
            "fp-1",
            "bad resource://7",
            vec![bad_resource],
            "",
        ))
        .await;
        assert_eq!(
            wait_assignment(&mut incoming_rx, 12, false)
                .await
                .error_code,
            "resource_integrity_mismatch"
        );

        peer.send(runtime_assign(
            13,
            "toolagent",
            "toolflow",
            "fp-1",
            "tool_wait",
            vec![],
            "",
        ))
        .await;
        let prefix = wait_tool_prefix(&peer, &mut incoming_rx, 13).await;
        assert_ordered(&prefix);
        let tool_resource = resource_ref_named(8, "tool.txt", b"tool-resource");
        let applied = runtime_tool_result(
            execution(13),
            "call-1",
            "result resource://8",
            vec![tool_resource.clone()],
        );
        peer.send(applied.clone()).await;
        assert_tool_receipt(&mut incoming_rx, 13, 1, "call-1", true, "").await;
        peer.send(applied).await;

        peer.send(runtime_tool_result(
            execution(13),
            "call-1",
            "conflict",
            vec![tool_resource.clone()],
        ))
        .await;
        peer.send(runtime_tool_result(
            execution(13),
            "unknown-call",
            "unknown",
            vec![],
        ))
        .await;
        peer.send(runtime_tool_result(
            ExecutionRef {
                execution_id: 13,
                attempt: 2,
            },
            "call-1",
            "wrong attempt",
            vec![],
        ))
        .await;
        // The Harness writes its log asynchronously after each delivered result.
        let tool_results = || {
            fs::read_to_string(&harness_log)
                .expect("tool harness log")
                .lines()
                .filter(|line| line.starts_with("tool_result\t"))
                .count()
        };
        wait_for(|| tool_results() >= 5).await;
        assert_eq!(tool_results(), 5);
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
                execution: Some(execution(13)),
                delivery: Some(delivery(130, "finish-tool")),
            })),
        })
        .await;
        let tool_terminal = collect_remaining_terminal(&peer, &mut incoming_rx, 13).await;
        assert_eq!(tool_terminal[0].sequence, 5);
        assert_eq!(
            tool_terminal[0].kind,
            ExecutionEventKind::ToolFinished as i32
        );
        assert_eq!(tool_terminal[1].sequence, 6);
        assert_eq!(tool_terminal[1].kind, ExecutionEventKind::Completed as i32);
        peer.send(runtime_tool_result(execution(13), "call-1", "late", vec![]))
            .await;

        peer.send(runtime_assign(
            14,
            "canceltool",
            "canceltool",
            "fp-1",
            "tool_wait",
            vec![],
            "",
        ))
        .await;
        wait_tool_prefix(&peer, &mut incoming_rx, 14).await;
        peer.send(cancel(14)).await;
        collect_remaining_terminal(&peer, &mut incoming_rx, 14).await;
        peer.send(runtime_tool_result(
            execution(14),
            "call-1",
            "late after cancel",
            vec![],
        ))
        .await;

        peer.send(runtime_assign(
            15,
            "badtoolsha",
            "badtoolsha",
            "fp-1",
            "tool_wait",
            vec![],
            "",
        ))
        .await;
        wait_tool_prefix(&peer, &mut incoming_rx, 15).await;
        let mut bad_tool_resource = tool_resource.clone();
        bad_tool_resource.sha256 = "0".repeat(64);
        peer.send(runtime_tool_result(
            execution(15),
            "call-1",
            "bad sha",
            vec![bad_tool_resource],
        ))
        .await;
        assert_tool_receipt(
            &mut incoming_rx,
            15,
            1,
            "call-1",
            false,
            "tool_resource_invalid",
        )
        .await;
        peer.send(cancel(15)).await;
        collect_remaining_terminal(&peer, &mut incoming_rx, 15).await;

        peer.send(runtime_assign(
            16,
            "badtoolpath",
            "badtoolpath",
            "fp-1",
            "tool_wait",
            vec![],
            "",
        ))
        .await;
        wait_tool_prefix(&peer, &mut incoming_rx, 16).await;
        let traversal = resource_ref_named(8, "../escape.txt", b"tool-resource");
        peer.send(runtime_tool_result(
            execution(16),
            "call-1",
            "bad path",
            vec![traversal],
        ))
        .await;
        assert_tool_receipt(
            &mut incoming_rx,
            16,
            1,
            "call-1",
            false,
            "tool_resource_invalid",
        )
        .await;
        peer.send(cancel(16)).await;
        collect_remaining_terminal(&peer, &mut incoming_rx, 16).await;

        assert!(workspace_directory(&workspace_root, "../escape").is_err());
        let state = fs::read_to_string(workspace_root.join(".machine/state.json"))
            .expect("state persistence");
        assert!(state.contains("native-machine-e2e"));
        assert!(state.contains("config_fingerprint"));

        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::DeleteAgentFiles(
                lankide_protocol::DeleteAgentFiles {
                    request_id: "delete-reader".into(),
                    agent: Some(AgentRef {
                        workspace_id: "alpha_team".into(),
                        agent_id: "reader".into(),
                    }),
                },
            )),
        })
        .await;
        let deleted = timeout(STEP, async {
            loop {
                let frame = incoming_rx.recv().await.expect("machine frame");
                if let Some(machine_frame::Frame::DeleteAgentFilesResult(result)) = frame.frame {
                    break result;
                }
            }
        })
        .await
        .expect("Agent file deletion result");
        assert!(deleted.success, "{}", deleted.error_code);
        assert!(!content_home.exists());

        // Fail the real state-file write while a real child process is active.
        peer.send(runtime_assign(
            99,
            "diskfailure",
            "diskfailure",
            "fp-1",
            "hold",
            vec![],
            "",
        ))
        .await;
        wait_assignment(&mut incoming_rx, 99, true).await;
        for sequence in 1..=3 {
            assert_eq!(wait_event(&mut incoming_rx, 99).await.sequence, sequence);
        }
        fs::create_dir(workspace_root.join(".machine/state.json.tmp"))
            .expect("block atomic state write");
        peer.send(RuntimeFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
                execution: Some(execution(99)),
                delivery: Some(delivery(990, "after-disk-failure")),
            })),
        })
        .await;
        timeout(STEP, async {
            while !machine_task.is_finished() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Machine must stop on event persistence failure");
    })
    .await;

    let _ = machine_stop_tx.send(());
    let host_result = timeout(STEP, machine_task)
        .await
        .expect("host shutdown")
        .expect("host join");
    assert!(
        matches!(
            host_result,
            Err(lankide_machine_core::MachineError::PersistenceFailed)
        ),
        "disk failure must report unsuccessful Machine exit: {host_result:?}"
    );
    while let Ok(frame) = incoming_rx.try_recv() {
        if let Some(machine_frame::Frame::ExecutionEvent(event)) = frame.frame {
            assert!(
                !event
                    .execution
                    .is_some_and(|value| value.execution_id == 99)
                    || event.sequence <= 3,
                "unsaved event was sent to Runtime"
            );
        }
    }
    let _ = server_stop_tx.send(());
    let _ = timeout(STEP, server)
        .await
        .expect("server shutdown")
        .expect("server join");
    let cleanup = verify_pids(&pid_journal);
    if harness_log.exists() {
        let _ = fs::copy(&harness_log, artifact_root.join("harness.tsv"));
        let _ = fs::remove_file(&harness_log);
    }
    let temp_cleanup = fs::remove_dir_all(&temp).map_err(|error| error.to_string());
    let status = if outcome.is_ok() && cleanup.is_ok() && temp_cleanup.is_ok() {
        "passed"
    } else {
        "failed"
    };
    let result = json!({
        "run_id":run_id, "module":"machine", "classification":"hermetic",
        "duration_ms":started.elapsed().as_millis(), "hard_timeout_ms":30000,
        "scenarios":["hello_catalog","initial_delivery_rejection_cleanup","serial_parallel","start_resume_fingerprint","steer_cancel_final","ordered_events","reconnect_replay_ack","duplicate_assignment","native_session_persistence","content_file_resource_sha","tool_result_roundtrip_idempotency_conflict_late_resource_validation","agent_directory_deletion","bad_sequence_stream_cut","completion_protocol","forced_cancel_no_orphan","state_write_failure_no_unsaved_event"],
        "cleanup": if cleanup.is_ok() && temp_cleanup.is_ok() { "passed" } else { "failed" }, "status":status,
        "error": outcome.as_ref().err().map(|_| "hard timeout"),
    });
    fs::write(&result_path, serde_json::to_vec_pretty(&result).unwrap()).expect("result artifact");
    println!("machine e2e artifact: {}", result_path.display());
    outcome.expect("machine scenario hard timeout");
    cleanup.expect("PID cleanup");
    temp_cleanup.expect("temp cleanup");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_sessions_reuse_rotate_credentials_expire_and_close() {
    let temp = tempfile::Builder::new()
        .prefix("lk-warm-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = temp.path().join("run");
    let root = temp.path().join("workspaces");
    let agent_home = agent_directory(&root, "alpha_team", "warm").unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    let log = agent_home.canonicalize().unwrap().join("session.jsonl");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let peer = Peer::new(tx);
    let (server_stop, stopped) = oneshot::channel();
    let server = tokio::spawn(ws::serve_local(
        socket.clone(),
        peer.clone(),
        TransportConfig::default(),
        async {
            let _ = stopped.await;
        },
    ));
    wait_socket(&socket).await;
    let source = format!(
        r#"
      const log = {};
      let request;
      function record(kind, extra) {{
        let prior = ""; try {{ prior = ctx.read(log); }} catch {{}}
        ctx.write(log, prior + JSON.stringify({{kind, ...extra}}) + "\n");
      }}
      function open(r) {{ request = r; const id = r.native_session_id || ctx.uuid(); ctx.bindSession(id); record("open", {{id}}); return {{native_session_id:id}}; }}
      defineHarness({{
        apiVersion:1, id:"pi", displayName:"Pi", reuseSession:true,
        probe() {{ return {{info:{{harness_id:"pi",display_name:"Pi"}},availability:"available",version:"1",capabilities:["resume"]}}; }},
        models() {{ return []; }},
        start:open, resume:open,
        rebind(r) {{ request = r; record("rebind"); }},
        execute() {{
          record("execute", JSON.parse(ctx.read(request.config.environment.LANKIDE_RUN_CONTEXT)));
          completed("OK");
        }},
        close() {{ record("close"); }}
      }});
    "#,
        serde_json::to_string(&log).unwrap()
    );
    let factory = lankide_harness::JsHarnessFactory::new("pi", "Pi", source).unwrap();
    let mut machine = Machine::new(MachineConfig {
        machine_id: "warm-machine".into(),
        instance_id: "warm-instance".into(),
        workspace_root: root.display().to_string(),
        reconnect: Default::default(),
    })
    .with_device_credential(lankide_protocol::link::discover(&socket).unwrap().1)
    .with_capability_cli("/usr/bin/true")
    .with_session_idle_timeout(Duration::from_secs(2));
    machine
        .register_harness(RegisteredHarness::new(
            factory,
            NativeHarnessConfig {
                harness_id: "pi".into(),
                model: ModelRef::new("fixture", "fixture"),
                cli_path: "/usr/bin/true".into(),
                thinking_level: "low".into(),
                config_fingerprint: "probe".into(),
                launch_arguments: vec![],
                environment: Default::default(),
            },
        ))
        .unwrap();
    let (machine_stop, stopped) = oneshot::channel();
    let host = tokio::spawn(machine.run(socket, async {
        let _ = stopped.await;
    }));
    let records = || -> Vec<serde_json::Value> {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    for (id, fingerprint) in [(101, "fp"), (102, "fp")] {
        peer.send(runtime_assign(
            id,
            "warm",
            "logical-warm",
            fingerprint,
            "hello",
            vec![],
            "",
        ))
        .await;
        let events = collect_terminal(&peer, &mut rx, id, true).await;
        assert_eq!(
            events.last().unwrap().kind,
            ExecutionEventKind::Completed as i32
        );
        assert!(
            !agent_directory(&root, "alpha_team", "warm")
                .unwrap()
                .join(".lankide/run-context.json")
                .exists()
        );
    }
    let entries = records();
    assert_eq!(entries.iter().filter(|e| e["kind"] == "open").count(), 1);
    assert_eq!(entries.iter().filter(|e| e["kind"] == "rebind").count(), 1);
    let executions: Vec<_> = entries.iter().filter(|e| e["kind"] == "execute").collect();
    assert_eq!(executions[0]["execution_id"], 101);
    assert_eq!(executions[1]["execution_id"], 102);
    assert_ne!(
        executions[0]["execution_token"],
        executions[1]["execution_token"]
    );
    assert_ne!(executions[0]["socket"], executions[1]["socket"]);
    wait_for(|| records().iter().any(|e| e["kind"] == "close")).await;
    for (id, fingerprint) in [(103, "fp"), (104, "changed")] {
        peer.send(runtime_assign(
            id,
            "warm",
            "logical-warm",
            fingerprint,
            "hello",
            vec![],
            "",
        ))
        .await;
        let events = collect_terminal(&peer, &mut rx, id, true).await;
        assert_eq!(
            events.last().unwrap().kind,
            ExecutionEventKind::Completed as i32
        );
    }
    assert_eq!(records().iter().filter(|e| e["kind"] == "open").count(), 3);
    machine_stop.send(()).unwrap();
    timeout(STEP, host).await.unwrap().unwrap().unwrap();
    assert_eq!(records().iter().filter(|e| e["kind"] == "close").count(), 3);
    server_stop.send(()).unwrap();
    timeout(STEP, server).await.unwrap().unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_versions_are_detected_saved_and_refreshed() {
    let temp = tempfile::Builder::new()
        .prefix("lk-ver-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = temp.path().join("run");
    let root = temp.path().join("workspaces");
    let agent_home = agent_directory(&root, "alpha_team", "ver").unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    let log = agent_home.canonicalize().unwrap().join("plugin.jsonl");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let install = |name: &str, version: &str| {
        let path = bin.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$0\" >> {}\necho \"{version}\"\n",
                bin.join("probes.log").display()
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    };
    let default_cli = install("pi", "pi 1.0");
    let custom_cli = install("pi-custom", "pi 7.0");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let peer = Peer::new(tx);
    let (server_stop, stopped) = oneshot::channel();
    let server = tokio::spawn(ws::serve_local(
        socket.clone(),
        peer.clone(),
        TransportConfig::default(),
        async {
            let _ = stopped.await;
        },
    ));
    wait_socket(&socket).await;
    let source = format!(
        r#"
      const log = {};
      function record(kind, extra) {{
        let prior = ""; try {{ prior = ctx.read(log); }} catch {{}}
        ctx.write(log, prior + JSON.stringify({{kind, ...extra}}) + "\n");
      }}
      function open(r) {{
        const id = r.native_session_id || ctx.uuid();
        ctx.bindSession(id);
        record("open", {{cli: r.config.cli_path, version: r.cli_version}});
        return {{native_session_id:id}};
      }}
      defineHarness({{
        apiVersion:1, id:"pi", displayName:"Pi", reuseSession:true,
        probe(config) {{
          return {{info:{{harness_id:"pi",display_name:"Pi"}},availability:"available",
            version:probeVersion(config).version,capabilities:["resume"]}};
        }},
        models() {{ return []; }},
        start:open, resume:open,
        rebind() {{ record("rebind"); }},
        execute() {{ completed("OK"); }},
        close() {{}}
      }});
    "#,
        serde_json::to_string(&log).unwrap()
    );
    let start_machine = || {
        let factory = lankide_harness::JsHarnessFactory::new("pi", "Pi", source.clone()).unwrap();
        let mut machine = Machine::new(MachineConfig {
            machine_id: "ver-machine".into(),
            instance_id: "ver-instance".into(),
            workspace_root: root.display().to_string(),
            reconnect: Default::default(),
        })
        .with_device_credential(lankide_protocol::link::discover(&socket).unwrap().1)
        .with_capability_cli("/usr/bin/true");
        machine
            .register_harness(RegisteredHarness::new(
                factory,
                NativeHarnessConfig {
                    harness_id: "pi".into(),
                    model: ModelRef::new("fixture", "fixture"),
                    cli_path: default_cli.clone(),
                    thinking_level: "low".into(),
                    config_fingerprint: "probe".into(),
                    launch_arguments: vec![],
                    environment: Default::default(),
                },
            ))
            .unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let host = tokio::spawn(machine.run(socket.clone(), async {
            let _ = stopped.await;
        }));
        (stop, host)
    };
    let records = || -> Vec<serde_json::Value> {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let probes = |cli: &Path| {
        fs::read_to_string(bin.join("probes.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| Path::new(line) == cli)
            .count()
    };
    let opened = || -> Vec<String> {
        records()
            .iter()
            .filter(|e| e["kind"] == "open")
            .map(|e| e["version"].as_str().unwrap_or_default().to_owned())
            .collect()
    };
    let assign = |id: i64, cli: Option<&Path>| {
        let mut frame = runtime_assign(id, "ver", "logical-ver", "fp", "hello", vec![], "");
        if let (Some(cli), Some(runtime_frame::Frame::AssignExecution(assignment))) =
            (cli, frame.frame.as_mut())
        {
            assignment.harness.as_mut().unwrap().executable_path = cli.display().to_string();
        }
        frame
    };

    let (stop, host) = start_machine();
    wait_for(|| peer.state.hellos.lock().unwrap().len() == 1).await;
    peer.send(assign(201, None)).await;
    collect_terminal(&peer, &mut rx, 201, true).await;
    assert_eq!(probes(&default_cli), 1, "the startup check is reused");
    assert_eq!(opened(), ["pi 1.0"]);

    install("pi", "pi 1.1");
    peer.send(assign(202, None)).await;
    collect_terminal(&peer, &mut rx, 202, true).await;
    assert_eq!(
        probes(&default_cli),
        2,
        "a replaced executable is checked again"
    );
    assert_eq!(opened(), ["pi 1.0", "pi 1.1"]);

    peer.send(assign(203, Some(&custom_cli))).await;
    collect_terminal(&peer, &mut rx, 203, true).await;
    assert_eq!(probes(&custom_cli), 1);
    assert_eq!(opened().last().unwrap(), "pi 7.0");
    stop.send(()).unwrap();
    timeout(STEP, host).await.unwrap().unwrap().unwrap();
    peer.disconnect();
    let connections = peer.state.hellos.lock().unwrap().len();

    let (stop, host) = start_machine();
    wait_for(|| peer.state.hellos.lock().unwrap().len() > connections).await;
    peer.send(assign(204, Some(&custom_cli))).await;
    collect_terminal(&peer, &mut rx, 204, true).await;
    assert_eq!(
        probes(&custom_cli),
        1,
        "the saved version survives a restart"
    );
    assert_eq!(opened().last().unwrap(), "pi 7.0");
    stop.send(()).unwrap();
    timeout(STEP, host).await.unwrap().unwrap().unwrap();
    server_stop.send(()).unwrap();
    timeout(STEP, server).await.unwrap().unwrap().unwrap();
}

fn runtime_assign(
    id: i64,
    agent: &str,
    logical: &str,
    fingerprint: &str,
    input: &str,
    resources: Vec<ResourceRef>,
    fault: &str,
) -> RuntimeFrame {
    let mut environment_variables = vec![lankide_protocol::EnvironmentVariable {
        name: "MACHINE_HARNESS_LOG".into(),
        value: artifact_harness_log(),
    }];
    environment_variables.extend([
        lankide_protocol::EnvironmentVariable {
            name: "PATH".into(),
            value: "/no-user-executables".into(),
        },
        lankide_protocol::EnvironmentVariable {
            name: "LANKIDE_CLI".into(),
            value: "/not-the-capability-cli".into(),
        },
    ]);
    if !fault.is_empty() {
        environment_variables.push(lankide_protocol::EnvironmentVariable {
            name: "MACHINE_HARNESS_FAULT".into(),
            value: fault.into(),
        });
    }
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::AssignExecution(AssignExecution {
            execution: Some(execution(id)),
            workspace: Some(WorkspaceRef {
                workspace_id: "alpha_team".into(),
            }),
            agent: Some(AgentRuntimeProfile {
                agent: Some(AgentRef {
                    workspace_id: "alpha_team".into(),
                    agent_id: agent.into(),
                }),
                name: agent.into(),
                role: "worker".into(),
                bio: String::new(),
                soul: "  性格原文\n\nBe direct.\n".into(),
            }),
            harness: Some(HarnessConfig {
                launch_arguments: Default::default(),
                harness_id: "pi".into(),
                executable_path: String::new(),
                model: "fixture".into(),
                thinking_level: "low".into(),
                environment_variables,
            }),
            session: Some(SessionDirective {
                logical_session_id: logical.into(),
                action: SessionAction::ResumeOrStart.into(),
                native_session_id: None,
                config_fingerprint: fingerprint.into(),
            }),
            initial_delivery: Some(Delivery {
                delivery_id: id * 10,
                kind: DeliveryKind::Initial.into(),
                rendered_markdown: input.into(),
                snapshot_id: id,
                trigger_reasons: vec!["test".into()],
                resources: resources.clone(),
                created_at: 1,
            }),
            system_prompt: "fixture system".into(),
            tools: vec![
                ToolDefinition {
                    name: "echo".into(),
                    description: "echo".into(),
                    input_schema_json: "{}".into(),
                    output_schema_json: "{}".into(),
                },
                ToolDefinition {
                    name: "message_send".into(),
                    description: "send".into(),
                    input_schema_json: "{}".into(),
                    output_schema_json: "{}".into(),
                },
            ],
            cli_commands: vec![lankide_protocol::CliCommand {
                name: "inspect".into(),
                description: "Inspect the current work".into(),
                argv: vec![],
            }],
            resources,
            execution_token: format!("token-{id}"),
            created_at: 1,
        })),
    }
}

fn runtime_assign_with_harness(
    id: i64,
    agent: &str,
    logical: &str,
    fingerprint: &str,
    input: &str,
    harness_id: &str,
) -> RuntimeFrame {
    let mut frame = runtime_assign(id, agent, logical, fingerprint, input, vec![], "");
    if let Some(runtime_frame::Frame::AssignExecution(assignment)) = frame.frame.as_mut()
        && let Some(harness) = assignment.harness.as_mut()
    {
        harness.harness_id = harness_id.into();
        harness.executable_path = "/usr/bin/false".into();
    }
    frame
}

fn artifact_harness_log() -> String {
    std::env::temp_dir()
        .join(format!(
            "lankide-machine-core-harness-{}.tsv",
            std::process::id()
        ))
        .display()
        .to_string()
}

fn execution(id: i64) -> ExecutionRef {
    ExecutionRef {
        execution_id: id,
        attempt: 1,
    }
}
fn delivery(id: i64, input: &str) -> Delivery {
    Delivery {
        delivery_id: id,
        kind: DeliveryKind::Steer.into(),
        rendered_markdown: input.into(),
        snapshot_id: id,
        trigger_reasons: Vec::new(),
        resources: Vec::new(),
        created_at: 1,
    }
}
fn cancel(id: i64) -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::CancelExecution(
            lankide_protocol::CancelExecution {
                force_close: false,
                execution: Some(execution(id)),
                reason: "test cancel".into(),
            },
        )),
    }
}
fn resource_ref(id: i64, bytes: &[u8]) -> ResourceRef {
    resource_ref_named(id, "brief.txt", bytes)
}
fn resource_ref_named(id: i64, file_name: &str, bytes: &[u8]) -> ResourceRef {
    ResourceRef {
        resource_id: id,
        file_name: file_name.into(),
        media_type: "text/plain".into(),
        size_bytes: bytes.len() as u64,
        sha256: hex(&Sha256::digest(bytes)),
    }
}

fn runtime_tool_result(
    execution: ExecutionRef,
    tool_call_id: &str,
    content_markdown: &str,
    resources: Vec<ResourceRef>,
) -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::CapabilityResponse(
            lankide_protocol::CapabilityResponse {
                request_id: format!(
                    "{}:{}:{}",
                    execution.execution_id, execution.attempt, tool_call_id
                ),
                eof: true,
                payload: lankide_protocol::agent::encode(
                    &lankide_protocol::CapabilityInvokeResult {
                        invocation_id: tool_call_id.into(),
                        success: true,
                        content_markdown: content_markdown.into(),
                        structured_content_json: r#"{"from_runtime":true}"#.into(),
                        resources,
                        error: None,
                    },
                )
                .unwrap(),
                ..Default::default()
            },
        )),
    }
}

async fn wait_tool_prefix(
    peer: &Peer,
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
) -> Vec<ExecutionEvent> {
    wait_assignment(receiver, id, true).await;
    let mut events = Vec::new();
    let mut saw_call = false;
    let mut saw_started = false;
    loop {
        let frame = timeout(STEP, receiver.recv())
            .await
            .expect("tool prefix timeout")
            .expect("machine stream");
        match frame.frame {
            Some(machine_frame::Frame::CapabilityRequest(call))
                if call
                    .context
                    .as_ref()
                    .and_then(|context| context.execution.as_ref())
                    .is_some_and(|execution| execution.execution_id == id) =>
            {
                let call: lankide_protocol::CapabilityInvokeRequest =
                    lankide_protocol::agent::decode(&call.payload).unwrap();
                assert_eq!(call.invocation_id, "call-1");
                saw_call = true;
            }
            Some(machine_frame::Frame::ExecutionEvent(event))
                if event
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.execution_id == id) =>
            {
                saw_started |= event.kind == ExecutionEventKind::ToolStarted as i32;
                ack(peer, &event).await;
                events.push(event);
            }
            _ => {}
        }
        if saw_call && saw_started {
            return events;
        }
    }
}

async fn wait_specific_tool_call(
    peer: &Peer,
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
    tool_name: &str,
) -> serde_json::Value {
    wait_assignment(receiver, id, true).await;
    loop {
        let frame = timeout(STEP, receiver.recv())
            .await
            .expect("tool call timeout")
            .expect("machine stream");
        match frame.frame {
            Some(machine_frame::Frame::CapabilityRequest(call))
                if call
                    .context
                    .as_ref()
                    .and_then(|context| context.execution.as_ref())
                    .is_some_and(|execution| execution.execution_id == id) =>
            {
                let call: lankide_protocol::CapabilityInvokeRequest =
                    lankide_protocol::agent::decode(&call.payload).unwrap();
                assert_eq!(call.capability_name, tool_name);
                return serde_json::from_str(&call.arguments_json).expect("tool arguments JSON");
            }
            Some(machine_frame::Frame::ExecutionEvent(event))
                if event
                    .execution
                    .as_ref()
                    .is_some_and(|execution| execution.execution_id == id) =>
            {
                ack(peer, &event).await;
            }
            _ => {}
        }
    }
}

async fn finish_local_tool_error(
    peer: &Peer,
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
    code: &str,
) {
    wait_assignment(receiver, id, true).await;
    let expected = format!("tool_result_error:{code}");
    for _ in 0..100 {
        if fs::read_to_string(artifact_harness_log())
            .unwrap_or_default()
            .contains(&expected)
        {
            break;
        }
        if let Ok(Some(frame)) = timeout(Duration::from_millis(10), receiver.recv()).await {
            match frame.frame {
                Some(machine_frame::Frame::CapabilityRequest(call))
                    if call
                        .context
                        .as_ref()
                        .and_then(|context| context.execution.as_ref())
                        .is_some_and(|execution| execution.execution_id == id) =>
                {
                    panic!("invalid local content_file reached Runtime")
                }
                Some(machine_frame::Frame::ExecutionEvent(event))
                    if event
                        .execution
                        .as_ref()
                        .is_some_and(|execution| execution.execution_id == id) =>
                {
                    ack(peer, &event).await;
                }
                _ => {}
            }
        }
    }
    assert!(
        fs::read_to_string(artifact_harness_log())
            .unwrap_or_default()
            .contains(&expected),
        "missing local Tool error {code}"
    );
    peer.send(RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::DeliverSteer(DeliverSteer {
            execution: Some(execution(id)),
            delivery: Some(delivery(id * 10, "finish-tool")),
        })),
    })
    .await;
    collect_remaining_terminal(peer, receiver, id).await;
}

async fn assert_tool_receipt(
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    execution_id: i64,
    attempt: u32,
    tool_call_id: &str,
    success: bool,
    error_code: &str,
) -> lankide_protocol::ResourceTransferResult {
    let expected = format!("tool_result:{execution_id}:{attempt}:{tool_call_id}");
    loop {
        let frame = timeout(STEP, receiver.recv())
            .await
            .expect("tool receipt timeout")
            .expect("machine stream");
        if let Some(machine_frame::Frame::ResourceTransferResult(receipt)) = frame.frame
            && receipt.transfer_id == expected
        {
            assert_eq!(receipt.success, success);
            assert_eq!(receipt.error_code, error_code);
            return receipt;
        }
    }
}

async fn collect_terminal(
    peer: &Peer,
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
    acknowledge: bool,
) -> Vec<ExecutionEvent> {
    wait_assignment(receiver, id, true).await;
    let mut events = Vec::new();
    loop {
        let event = wait_event(receiver, id).await;
        let terminal = matches!(
            ExecutionEventKind::try_from(event.kind).unwrap(),
            ExecutionEventKind::Completed
                | ExecutionEventKind::Failed
                | ExecutionEventKind::Cancelled
        );
        if acknowledge {
            ack(peer, &event).await;
        }
        events.push(event);
        if terminal {
            return events;
        }
    }
}

async fn collect_remaining_terminal(
    peer: &Peer,
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
) -> Vec<ExecutionEvent> {
    let mut events = Vec::new();
    loop {
        let event = wait_event(receiver, id).await;
        let terminal = matches!(
            ExecutionEventKind::try_from(event.kind).unwrap(),
            ExecutionEventKind::Completed
                | ExecutionEventKind::Failed
                | ExecutionEventKind::Cancelled
        );
        ack(peer, &event).await;
        events.push(event);
        if terminal {
            return events;
        }
    }
}

async fn wait_assignment(
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
    accepted: bool,
) -> AssignmentResult {
    loop {
        let frame = timeout(STEP, receiver.recv())
            .await
            .unwrap_or_else(|_| panic!("assignment {id} timed out"))
            .expect("machine stream");
        if let Some(machine_frame::Frame::AssignmentResult(result)) = frame.frame
            && result
                .execution
                .as_ref()
                .is_some_and(|value| value.execution_id == id)
        {
            assert_eq!(result.accepted, accepted, "assignment result: {result:?}");
            return result;
        }
    }
}

// A sent acknowledgement may still be in flight when the peer disconnects.
// Every replay must match the saved execution; the terminal must arrive in time.
async fn wait_replayed_terminal(
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    original: &[ExecutionEvent],
) -> ExecutionEvent {
    let terminal = original.last().expect("terminal event");
    let id = terminal.execution.as_ref().expect("execution").execution_id;
    timeout(STEP, async {
        loop {
            let event = wait_event(receiver, id).await;
            assert!(
                original.contains(&event),
                "replay changed an execution event"
            );
            if event.sequence == terminal.sequence {
                return event;
            }
        }
    })
    .await
    .expect("terminal replay timeout")
}

async fn wait_event(
    receiver: &mut mpsc::UnboundedReceiver<MachineFrame>,
    id: i64,
) -> ExecutionEvent {
    loop {
        let frame = timeout(STEP, receiver.recv())
            .await
            .expect("event timeout")
            .expect("machine stream");
        if let Some(machine_frame::Frame::ExecutionEvent(event)) = frame.frame
            && event
                .execution
                .as_ref()
                .is_some_and(|value| value.execution_id == id)
        {
            return event;
        }
    }
}

async fn wait_started(receiver: &mut mpsc::UnboundedReceiver<MachineFrame>, id: i64) {
    loop {
        if wait_event(receiver, id).await.kind == ExecutionEventKind::Started as i32 {
            return;
        }
    }
}
async fn wait_delivery(receiver: &mut mpsc::UnboundedReceiver<MachineFrame>, id: i64) {
    loop {
        let frame = timeout(STEP, receiver.recv())
            .await
            .expect("delivery timeout")
            .expect("stream");
        if let Some(machine_frame::Frame::DeliveryResult(result)) = frame.frame
            && result
                .execution
                .as_ref()
                .is_some_and(|value| value.execution_id == id)
        {
            assert!(result.accepted);
            return;
        }
    }
}
async fn ack(peer: &Peer, event: &ExecutionEvent) {
    peer.send(RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::ExecutionEventAck(ExecutionEventAck {
            execution: event.execution.clone(),
            max_saved_sequence: event.sequence,
        })),
    })
    .await;
}
fn assert_ordered(events: &[ExecutionEvent]) {
    assert!(
        events
            .iter()
            .enumerate()
            .all(|(index, event)| event.sequence == index as u64 + 1)
    );
}
fn payloads(events: &[ExecutionEvent]) -> String {
    events
        .iter()
        .map(|event| String::from_utf8_lossy(&event.payload))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn wait_socket(path: &Path) {
    wait_for(|| path.join("agent.sock").exists()).await;
}
async fn wait_for(mut condition: impl FnMut() -> bool) {
    timeout(STEP, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition timeout");
}

fn verify_pids(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut spawned = HashSet::new();
    let mut reaped = HashSet::new();
    for line in content.lines() {
        let (action, pid) = line
            .split_once('\t')
            .ok_or_else(|| format!("bad pid line: {line}"))?;
        match action {
            "spawned" => {
                spawned.insert(pid.to_owned());
            }
            "reaped" => {
                reaped.insert(pid.to_owned());
            }
            "forced" => {}
            _ => return Err(format!("bad action {action}")),
        }
    }
    if spawned != reaped {
        return Err(format!(
            "PID imbalance spawned={spawned:?} reaped={reaped:?}"
        ));
    }
    for pid in spawned {
        if std::process::Command::new("kill")
            .args(["-0", &pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            return Err(format!("orphan PID {pid}"));
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos()
}
