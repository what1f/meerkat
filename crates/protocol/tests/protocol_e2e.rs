#[path = "support/ws.rs"]
mod ws;
use lankide_protocol::machine_frame;
use lankide_protocol::runtime_frame;
use lankide_protocol::*;
use lankide_protocol::{Code, Status};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{StreamExt, iter};

const SERVER_LIMIT: usize = 8 * 1024;
const CLIENT_LIMIT: usize = 64 * 1024;

struct ScenarioReport {
    artifact: PathBuf,
    run_id: String,
    started_at: u128,
    timer: Instant,
    completed: bool,
}

impl ScenarioReport {
    fn start() -> Self {
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let run_id = format!("e2e_protocol_{started_at}_{}", std::process::id());
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("protocol crate is nested under the workspace")
            .to_path_buf();
        let artifact_dir = workspace_root
            .join("target/e2e-artifacts/protocol")
            .join(&run_id);
        fs::create_dir_all(&artifact_dir).expect("create protocol artifact directory");
        let mut report = Self {
            artifact: artifact_dir.join("result.json"),
            run_id,
            started_at,
            timer: Instant::now(),
            completed: false,
        };
        report.write("running");
        println!("run_id={}", report.run_id);
        println!("artifact={}", report.artifact.display());
        report
    }

    fn pass(mut self) {
        self.write("pass");
        self.completed = true;
    }

    fn write(&mut self, status: &str) {
        let payload = serde_json::json!({
            "run_id": self.run_id,
            "module": "protocol",
            "classification": "hermetic",
            "started_at_unix_nanos": self.started_at,
            "duration_millis": self.timer.elapsed().as_millis(),
            "prerequisite_check": "pass",
            "scenarios": [{"name": "protocol_e2e", "status": status}],
            "created_resources": [],
            "cleanup": "pass",
            "status": status,
        });
        fs::write(
            &self.artifact,
            serde_json::to_vec_pretty(&payload).expect("serialize protocol result"),
        )
        .expect("write protocol result");
    }
}

impl Drop for ScenarioReport {
    fn drop(&mut self) {
        if !self.completed {
            self.write("fail");
        }
    }
}

#[derive(Clone, Default)]
struct TestPeer {
    state: Arc<Mutex<PeerState>>,
}

#[derive(Default)]
struct PeerState {
    connections: usize,
    hellos: Vec<MachineHello>,
    assignment_results: Vec<AssignmentResult>,
    event_sequences: Vec<u64>,
    resources: HashMap<i64, StoredResource>,
    sent_messages: Vec<SendMessageRequest>,
    workspaces: HashMap<String, Workspace>,
    management_subscribers: Vec<mpsc::Sender<Result<ManagementEvent, Status>>>,
    next_management_sequence: u64,
}

#[derive(Clone)]
struct StoredResource {
    reference: ResourceRef,
    bytes: Vec<u8>,
}

#[async_trait::async_trait]
impl lankide_protocol::MachinePeer for TestPeer {
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
        let connection = {
            let mut state = self.state.lock().expect("peer state");
            state.connections += 1;
            state.hellos.push(hello);
            state.connections
        };
        let (outgoing, receiver) = mpsc::channel(16);
        outgoing
            .send(Ok(runtime_hello_frame()))
            .await
            .expect("runtime hello receiver");
        if connection <= 2 {
            outgoing
                .send(Ok(RuntimeFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    frame: Some(runtime_frame::Frame::AssignExecution(assignment())),
                }))
                .await
                .expect("assignment receiver");
        }

        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            while let Some(next) = frames.next().await {
                let frame = match next {
                    Ok(frame) => frame,
                    Err(status) => {
                        let _ = outgoing.send(Err(status)).await;
                        break;
                    }
                };
                match frame.frame {
                    Some(machine_frame::Frame::AssignmentResult(result)) => {
                        state
                            .lock()
                            .expect("peer state")
                            .assignment_results
                            .push(result);
                    }
                    Some(machine_frame::Frame::ExecutionEvent(event)) => {
                        state
                            .lock()
                            .expect("peer state")
                            .event_sequences
                            .push(event.sequence);
                        match (connection, event.sequence) {
                            (1, 1) | (2, 2) => {
                                outgoing
                                    .send(Ok(RuntimeFrame {
                                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                                        frame: Some(runtime_frame::Frame::ExecutionEventAck(
                                            ExecutionEventAck {
                                                execution: event.execution,
                                                max_saved_sequence: event.sequence,
                                            },
                                        )),
                                    }))
                                    .await
                                    .expect("ack receiver");
                            }
                            (1, 2) => break,
                            (2, 3) => {
                                let execution = event.execution;
                                outgoing
                                    .send(Ok(RuntimeFrame {
                                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                                        frame: Some(runtime_frame::Frame::ExecutionEventAck(
                                            ExecutionEventAck {
                                                execution: execution.clone(),
                                                max_saved_sequence: 3,
                                            },
                                        )),
                                    }))
                                    .await
                                    .expect("final ack receiver");
                                outgoing
                                    .send(Ok(RuntimeFrame {
                                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                                        frame: Some(runtime_frame::Frame::ExecutionCommitted(
                                            ExecutionCommitted {
                                                execution,
                                                committed_at: 1_725_000_000_500,
                                            },
                                        )),
                                    }))
                                    .await
                                    .expect("commit receiver");
                                break;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }
}

#[async_trait::async_trait]
impl RuntimePeer for TestPeer {
    async fn upload_resource(
        &self,
        mut chunks: ResourceChunkStream,
    ) -> Result<ResourceRef, Status> {
        let mut bytes = Vec::new();
        let mut descriptor = None;
        while let Some(next) = chunks.next().await {
            let chunk = next?;
            descriptor = descriptor.or(chunk.descriptor);
            bytes.extend_from_slice(&chunk.bytes);
            if chunk.eof {
                break;
            }
        }
        let descriptor = descriptor.ok_or_else(|| Status::invalid_argument("missing metadata"))?;
        let reference = ResourceRef {
            resource_id: 41,
            file_name: descriptor.file_name,
            media_type: descriptor.media_type,
            size_bytes: descriptor.size_bytes,
            sha256: descriptor.sha256,
        };
        self.state.lock().expect("peer state").resources.insert(
            reference.resource_id,
            StoredResource {
                reference: reference.clone(),
                bytes,
            },
        );
        Ok(reference)
    }

    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status> {
        let stored = self
            .state
            .lock()
            .expect("peer state")
            .resources
            .get(&request.resource_id)
            .cloned()
            .ok_or_else(|| Status::not_found("resource_not_found"))?;
        let descriptor = ResourceDescriptor {
            file_name: stored.reference.file_name,
            media_type: stored.reference.media_type,
            size_bytes: stored.reference.size_bytes,
            sha256: stored.reference.sha256,
            context: request.context.clone(),
        };
        let split = [0, 5, 11, stored.bytes.len()];
        let chunks = split
            .windows(2)
            .enumerate()
            .map(|(index, range)| {
                Ok(ResourceChunk {
                    transfer_id: request.transfer_id.clone(),
                    resource_id: request.resource_id,
                    offset: range[0] as u64,
                    bytes: stored.bytes[range[0]..range[1]].to_vec(),
                    eof: index == 2,
                    descriptor: (index == 0).then(|| descriptor.clone()),
                })
            })
            .collect::<Vec<_>>();
        Ok(Box::pin(iter(chunks)))
    }

    async fn advance_agent_onboarding(
        &self,
        request: AdvanceAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, Status> {
        Ok(AgentOnboardingState {
            agent: request.agent,
            provider_id: "feishu".into(),
            step: "create_app".into(),
            status: OnboardingStatus::ActionRequired,
            payload_json: if request.action == "invalid" {
                "invalid".into()
            } else {
                request.field_values_json
            },
            verification_uri: "https://example.invalid/verify".into(),
            poll_interval_seconds: 5,
            error_code: String::new(),
            error_message: String::new(),
        })
    }

    async fn list_workspaces(
        &self,
        request: ListWorkspacesRequest,
    ) -> Result<ListWorkspacesResponse, Status> {
        let mut workspaces = self
            .state
            .lock()
            .expect("peer state")
            .workspaces
            .values()
            .cloned()
            .collect::<Vec<_>>();
        workspaces.sort_by(|left, right| {
            left.workspace
                .as_ref()
                .unwrap()
                .workspace_id
                .cmp(&right.workspace.as_ref().unwrap().workspace_id)
        });
        Ok(ListWorkspacesResponse {
            request_id: request.request_id,
            workspaces,
        })
    }

    async fn create_workspace(&self, request: CreateWorkspaceRequest) -> Result<Workspace, Status> {
        let workspace = Workspace {
            workspace: Some(WorkspaceRef {
                workspace_id: request.workspace_id.clone(),
            }),
            name: request.name,
            cover: None,
            rules: request.rules,
            provider_id: request.provider_id,
            provider_config_json: request.provider_config_json,
            desired_state: DesiredState::Running.into(),
            observed_state: ObservedState::Running.into(),
            status_message: String::new(),
            created_at: 1_725_000_001_000,
            updated_at: 1_725_000_001_000,
        };
        self.state
            .lock()
            .expect("peer state")
            .workspaces
            .insert(request.workspace_id, workspace.clone());
        self.publish_management(
            ManagementEventKind::WorkspaceCreated,
            workspace.workspace.clone().unwrap(),
        )
        .await;
        Ok(workspace)
    }

    async fn update_workspace(&self, request: UpdateWorkspaceRequest) -> Result<Workspace, Status> {
        let id = request.workspace.unwrap().workspace_id;
        let workspace = {
            let mut state = self.state.lock().expect("peer state");
            let workspace = state
                .workspaces
                .get_mut(&id)
                .ok_or_else(|| Status::not_found("workspace_not_found"))?;
            if let Some(name) = request.name {
                workspace.name = name;
            }
            if let Some(rules) = request.rules {
                workspace.rules = rules;
            }
            if let Some(config) = request.provider_config_json {
                workspace.provider_config_json = config;
            }
            workspace.updated_at += 1;
            workspace.clone()
        };
        self.publish_management(
            ManagementEventKind::WorkspaceUpdated,
            workspace.workspace.clone().unwrap(),
        )
        .await;
        Ok(workspace)
    }

    async fn set_workspace_state(
        &self,
        request: SetWorkspaceStateRequest,
    ) -> Result<Workspace, Status> {
        let id = request.workspace.unwrap().workspace_id;
        let workspace = {
            let mut state = self.state.lock().expect("peer state");
            let workspace = state
                .workspaces
                .get_mut(&id)
                .ok_or_else(|| Status::not_found("workspace_not_found"))?;
            workspace.desired_state = request.desired_state;
            workspace.observed_state = match DesiredState::try_from(request.desired_state) {
                Ok(DesiredState::Running) => ObservedState::Running.into(),
                Ok(DesiredState::Paused) => ObservedState::Paused.into(),
                _ => return Err(Status::invalid_argument("invalid_state")),
            };
            workspace.updated_at += 1;
            workspace.clone()
        };
        self.publish_management(
            ManagementEventKind::WorkspaceStateChanged,
            workspace.workspace.clone().unwrap(),
        )
        .await;
        Ok(workspace)
    }

    async fn subscribe_management_events(
        &self,
        _request: ManagementSubscription,
    ) -> Result<ManagementEventStream, Status> {
        let (sender, receiver) = mpsc::channel(8);
        self.state
            .lock()
            .expect("peer state")
            .management_subscribers
            .push(sender);
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvokeRequest,
    ) -> Result<CapabilityInvokeResult, Status> {
        let context = request.context.expect("validated context");
        if context.execution != Some(execution_ref())
            || context.execution_token != "execution_token_17"
        {
            return Err(Status::permission_denied("execution_scope_denied"));
        }
        Ok(CapabilityInvokeResult {
            invocation_id: request.invocation_id,
            success: true,
            content_markdown: "capability invoked".into(),
            structured_content_json: r#"{"sent":true}"#.into(),
            resources: Vec::new(),
            error: None,
        })
    }

    async fn send_message(&self, request: SendMessageRequest) -> Result<SendMessageResult, Status> {
        self.state
            .lock()
            .expect("peer state")
            .sent_messages
            .push(request);
        Ok(SendMessageResult {
            accepted: true,
            message_id: 52,
            provider_message_id: "protocol-provider-message-52".into(),
            error: None,
        })
    }
}

impl TestPeer {
    async fn publish_management(&self, kind: ManagementEventKind, workspace: WorkspaceRef) {
        let (event, subscribers) = {
            let mut state = self.state.lock().expect("peer state");
            state.next_management_sequence += 1;
            (
                ManagementEvent {
                    sequence: state.next_management_sequence,
                    occurred_at: 1_725_000_002_000 + state.next_management_sequence as i64,
                    kind: kind.into(),
                    workspace: Some(workspace),
                    agent: None,
                },
                state.management_subscribers.clone(),
            )
        };
        for subscriber in subscribers {
            let _ = subscriber.send(Ok(event.clone())).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protocol_e2e() {
    let report = ScenarioReport::start();
    tokio::time::timeout(Duration::from_secs(30), complete_protocol_scenario())
        .await
        .expect("protocol E2E exceeded 30 seconds");
    report.pass();
}

async fn complete_protocol_scenario() {
    let mut invalid_tool = assignment();
    invalid_tool.tools[0].name = "feishu.reaction.catalog".into();
    let status = validate_runtime_frame(&RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::AssignExecution(invalid_tool)),
    })
    .expect_err("dotted Tool name must be rejected by the shared protocol");
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(status.message(), "invalid_tool_name");

    let temp = TempDir::new().expect("temporary protocol home");
    let path = temp.path().join("run");
    let peer = TestPeer::default();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_peer = peer.clone();
    let server_path = path.clone();
    let server = tokio::spawn(async move {
        ws::serve_local(
            server_path,
            server_peer,
            TransportConfig {
                protocol_version: PROTOCOL_VERSION,
                max_message_bytes: SERVER_LIMIT,
            },
            async move {
                let _ = shutdown_rx.await;
            },
        )
        .await
        .expect("serve protocol UDS")
    });
    wait_for_socket(&path).await;

    exchange_assignment_events_and_reconnect(&path, &peer).await;
    let uploaded = exchange_resource_chunks(&path).await;
    exchange_attachment_only_message(&path, &peer, uploaded).await;
    exchange_management_and_cli(&path).await;
    exchange_agent_json_socket(&path).await;
    reject_version_mismatch(&path).await;
    reject_oversized_frame(&path).await;
    reject_out_of_order_resource_offset(&path).await;
    reject_unknown_frame(&path).await;
    reject_event_sequence_regression(&path).await;
    reject_empty_message(&path).await;

    shutdown_tx.send(()).expect("shutdown server");
    server.await.expect("join protocol server");
    assert!(
        !path.join("agent.sock").exists(),
        "server removes its owned socket on shutdown"
    );
}

async fn exchange_assignment_events_and_reconnect(path: &std::path::Path, peer: &TestPeer) {
    let (first_tx, first_rx) = mpsc::channel(8);
    first_tx
        .send(hello_frame())
        .await
        .expect("send first hello");
    let mut first_runtime = ws::connect_stream(path, ReceiverStream::new(first_rx))
        .await
        .expect("first connect stream")
        .into_inner();
    assert_runtime_hello(
        first_runtime
            .message()
            .await
            .expect("runtime hello status")
            .expect("runtime hello"),
    );
    let assign = expect_assignment(
        first_runtime
            .message()
            .await
            .expect("assignment status")
            .expect("assignment"),
    );
    assert_complete_assignment(&assign);
    first_tx
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::AssignmentResult(AssignmentResult {
                failure: None,
                execution: assign.execution.clone(),
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        })
        .await
        .expect("send assignment result");
    first_tx
        .send(event_frame(1, ExecutionEventKind::Started, b"started"))
        .await
        .expect("send event one");
    first_tx
        .send(event_frame(
            2,
            ExecutionEventKind::AssistantDelta,
            b"partial",
        ))
        .await
        .expect("send event two");
    assert_ack(
        first_runtime
            .message()
            .await
            .expect("first ack status")
            .expect("first ack"),
        1,
    );
    assert!(
        first_runtime
            .message()
            .await
            .expect("first disconnect status")
            .is_none(),
        "runtime deliberately disconnects before acknowledging event 2"
    );
    drop(first_tx);

    let (second_tx, second_rx) = mpsc::channel(8);
    second_tx
        .send(hello_frame())
        .await
        .expect("send reconnect hello");
    let mut second_runtime = ws::connect_stream(path, ReceiverStream::new(second_rx))
        .await
        .expect("reconnect stream")
        .into_inner();
    assert_runtime_hello(
        second_runtime
            .message()
            .await
            .expect("reconnect hello status")
            .expect("reconnect hello"),
    );
    expect_assignment(
        second_runtime
            .message()
            .await
            .expect("reconnect assignment status")
            .expect("reconnect assignment"),
    );
    second_tx
        .send(event_frame(
            2,
            ExecutionEventKind::AssistantDelta,
            b"partial",
        ))
        .await
        .expect("retransmit unacknowledged event");
    assert_ack(
        second_runtime
            .message()
            .await
            .expect("replay ack status")
            .expect("replay ack"),
        2,
    );
    second_tx
        .send(event_frame(
            3,
            ExecutionEventKind::Completed,
            b"final answer",
        ))
        .await
        .expect("send completed event");
    assert_ack(
        second_runtime
            .message()
            .await
            .expect("final ack status")
            .expect("final ack"),
        3,
    );
    let committed = second_runtime
        .message()
        .await
        .expect("commit status")
        .expect("commit frame");
    match committed.frame {
        Some(runtime_frame::Frame::ExecutionCommitted(committed)) => {
            assert_eq!(committed.execution, Some(execution_ref()));
            assert_eq!(committed.committed_at, 1_725_000_000_500);
        }
        other => panic!("expected ExecutionCommitted, got {other:?}"),
    }
    drop(second_tx);

    let state = peer.state.lock().expect("peer state");
    assert_eq!(state.connections, 2);
    assert_eq!(state.hellos[0], state.hellos[1]);
    assert_eq!(state.assignment_results.len(), 1);
    assert_eq!(state.event_sequences, [1, 2, 2, 3]);
}

async fn exchange_resource_chunks(path: &std::path::Path) -> ResourceRef {
    let bytes = b"real multi chunk resource bytes".to_vec();
    let sha256 = lowercase_hex(&Sha256::digest(&bytes));
    let descriptor = ResourceDescriptor {
        file_name: "evidence.bin".into(),
        media_type: "application/octet-stream".into(),
        size_bytes: bytes.len() as u64,
        sha256: sha256.clone(),
        context: Some(run_context("execution_token_17")),
    };
    let chunks = vec![
        ResourceChunk {
            transfer_id: "upload_1".into(),
            resource_id: 0,
            offset: 0,
            bytes: bytes[0..5].to_vec(),
            eof: false,
            descriptor: Some(descriptor),
        },
        ResourceChunk {
            transfer_id: "upload_1".into(),
            resource_id: 0,
            offset: 5,
            bytes: bytes[5..11].to_vec(),
            eof: false,
            descriptor: None,
        },
        ResourceChunk {
            transfer_id: "upload_1".into(),
            resource_id: 0,
            offset: 11,
            bytes: bytes[11..].to_vec(),
            eof: true,
            descriptor: None,
        },
    ];
    let mut client = connect_uds(path.join("agent.sock"), CLIENT_LIMIT)
        .await
        .expect("resource client");
    let uploaded = client
        .upload_resource(iter(chunks))
        .await
        .expect("multi chunk upload")
        .into_inner();
    assert_eq!(uploaded.resource_id, 41);
    assert_eq!(uploaded.size_bytes, bytes.len() as u64);
    assert_eq!(uploaded.sha256, sha256);

    let mut download = client
        .download_resource(ResourceRequest {
            resource_id: uploaded.resource_id,
            transfer_id: "download_1".into(),
            context: Some(run_context("execution_token_17")),
        })
        .await
        .expect("resource download")
        .into_inner();
    let mut downloaded = Vec::new();
    let mut expected_offset = 0;
    while let Some(chunk) = download.message().await.expect("download chunk status") {
        assert_eq!(chunk.offset, expected_offset);
        expected_offset += chunk.bytes.len() as u64;
        downloaded.extend_from_slice(&chunk.bytes);
        if chunk.eof {
            break;
        }
    }
    assert_eq!(downloaded, bytes);
    assert_eq!(lowercase_hex(&Sha256::digest(&downloaded)), sha256);
    uploaded
}

async fn exchange_attachment_only_message(
    path: &std::path::Path,
    peer: &TestPeer,
    attachment: ResourceRef,
) {
    let mut client = connect_uds(path.join("agent.sock"), CLIENT_LIMIT)
        .await
        .expect("attachment-only message client");
    let sent = client
        .send_message(SendMessageRequest {
            context: Some(run_context("execution_token_17")),
            channel_id: "feishu:chat:41".into(),
            chat_id: "application-chat".into(),
            content_markdown: String::new(),
            reply_to_message_id: Some(23),
            resources: vec![attachment.clone()],
            idempotency_key: "protocol-attachment-only".into(),
            force: false,
        })
        .await
        .expect("attachment-only message should pass protocol validation")
        .into_inner();
    assert!(sent.accepted);
    assert_eq!(sent.message_id, 52);
    assert_eq!(sent.provider_message_id, "protocol-provider-message-52");

    let state = peer.state.lock().expect("peer state");
    let request = state
        .sent_messages
        .last()
        .expect("peer received attachment-only message");
    assert!(request.content_markdown.is_empty());
    assert_eq!(request.reply_to_message_id, Some(23));
    assert_eq!(request.resources, vec![attachment]);
}

async fn reject_empty_message(path: &std::path::Path) {
    let mut client = connect_uds(path.join("agent.sock"), CLIENT_LIMIT)
        .await
        .expect("empty message client");
    let error = client
        .send_message(SendMessageRequest {
            context: Some(run_context("execution_token_17")),
            channel_id: "feishu:chat:41".into(),
            chat_id: "application-chat".into(),
            content_markdown: String::new(),
            reply_to_message_id: None,
            resources: Vec::new(),
            idempotency_key: "protocol-empty-message".into(),
            force: false,
        })
        .await
        .expect_err("message without text or resources must be rejected");
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(error.message(), "empty_content_markdown");
}

async fn exchange_management_and_cli(path: &std::path::Path) {
    let client = lankide_client::http::HttpManagementTransport::local(path, CLIENT_LIMIT);
    let onboarding_request = AdvanceAgentOnboardingRequest {
        request_id: "onboarding_1".into(),
        agent: Some(AgentRef {
            workspace_id: "managed_team".into(),
            agent_id: "builder".into(),
        }),
        action: "start".into(),
        field_values_json: r#"{"opaque":"provider-owned"}"#.into(),
    };
    let onboarding = client
        .advance_agent_onboarding(onboarding_request.clone())
        .await
        .expect("advance onboarding")
        .into_inner();
    assert_eq!(
        onboarding.payload_json,
        onboarding_request.field_values_json
    );
    assert_eq!(
        onboarding.verification_uri,
        "https://example.invalid/verify"
    );
    assert_eq!(onboarding.poll_interval_seconds, 5);
    let invalid = client
        .advance_agent_onboarding(AdvanceAgentOnboardingRequest {
            action: "invalid".into(),
            ..onboarding_request
        })
        .await
        .expect_err("reject invalid onboarding payload");
    assert_eq!(invalid.code(), Code::InvalidArgument);

    let mut changes = client
        .subscribe_management_events(ManagementSubscription {
            request_id: "subscribe_1".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: "managed_team".into(),
            }),
            after_sequence: 0,
        })
        .await
        .expect("management subscription")
        .into_inner();
    let created = client
        .create_workspace(CreateWorkspaceRequest {
            request_id: "create_1".into(),
            workspace_id: "managed_team".into(),
            name: "Managed Team".into(),
            rules: "Use primary sources.".into(),
            provider_id: "feishu".into(),
            provider_config_json: r#"{"tasks_enabled":true}"#.into(),
            provider_secrets_json: r#"{"app_secret":"opaque"}"#.into(),
        })
        .await
        .expect("create workspace")
        .into_inner();
    assert_eq!(created.name, "Managed Team");
    let create_event = changes
        .message()
        .await
        .expect("create event status")
        .expect("create event");
    assert_eq!(create_event.sequence, 1);
    assert_eq!(
        ManagementEventKind::try_from(create_event.kind).unwrap(),
        ManagementEventKind::WorkspaceCreated
    );

    let updated = client
        .update_workspace(UpdateWorkspaceRequest {
            request_id: "update_1".into(),
            workspace: created.workspace.clone(),
            name: Some("Evidence Team".into()),
            rules: Some("Cite evidence.".into()),
            provider_config_json: Some(r#"{"tasks_enabled":false}"#.into()),
            cover_resource_id: None,
            provider_secrets_json: None,
        })
        .await
        .expect("update workspace")
        .into_inner();
    assert_eq!(updated.name, "Evidence Team");
    assert_eq!(updated.rules, "Cite evidence.");
    assert_eq!(
        ManagementEventKind::try_from(
            changes
                .message()
                .await
                .expect("update event status")
                .expect("update event")
                .kind
        )
        .unwrap(),
        ManagementEventKind::WorkspaceUpdated
    );

    let paused = client
        .set_workspace_state(SetWorkspaceStateRequest {
            request_id: "pause_1".into(),
            workspace: created.workspace.clone(),
            desired_state: DesiredState::Paused.into(),
        })
        .await
        .expect("pause workspace")
        .into_inner();
    assert_eq!(
        DesiredState::try_from(paused.desired_state).unwrap(),
        DesiredState::Paused
    );
    assert_eq!(
        ManagementEventKind::try_from(
            changes
                .message()
                .await
                .expect("state event status")
                .expect("state event")
                .kind
        )
        .unwrap(),
        ManagementEventKind::WorkspaceStateChanged
    );
    let resumed = client
        .set_workspace_state(SetWorkspaceStateRequest {
            request_id: "resume_1".into(),
            workspace: created.workspace,
            desired_state: DesiredState::Running.into(),
        })
        .await
        .expect("resume workspace")
        .into_inner();
    assert_eq!(
        DesiredState::try_from(resumed.desired_state).unwrap(),
        DesiredState::Running
    );
    changes
        .message()
        .await
        .expect("resume event status")
        .expect("resume event");

    let listed = client
        .list_workspaces(ListWorkspacesRequest {
            request_id: "list_1".into(),
        })
        .await
        .expect("list workspaces")
        .into_inner();
    assert_eq!(listed.request_id, "list_1");
    assert_eq!(listed.workspaces, vec![resumed]);

    let mut cli = connect_uds(path.join("agent.sock"), CLIENT_LIMIT)
        .await
        .expect("Agent client");
    let allowed = cli
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(run_context("execution_token_17")),
            capability_name: "reply".into(),
            arguments_json: r#"{"content":"done"}"#.into(),
            invocation_id: "invoke_1".into(),
        })
        .await
        .expect("authorized capability invoke")
        .into_inner();
    assert!(allowed.success);
    assert_eq!(allowed.content_markdown, "capability invoked");

    let denied = cli
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(run_context("wrong_token")),
            capability_name: "reply".into(),
            arguments_json: r#"{"content":"stolen"}"#.into(),
            invocation_id: "invoke_2".into(),
        })
        .await
        .expect_err("wrong execution token must be denied");
    assert_eq!(denied.code(), Code::PermissionDenied);
    assert_eq!(denied.message(), "execution_scope_denied");
}

async fn reject_version_mismatch(path: &std::path::Path) {
    let mut hello = machine_hello();
    hello.protocol_version += 1;
    let error = ws::connect_stream(
        path,
        iter([MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::Hello(hello)),
        }]),
    )
    .await
    .expect_err("mismatched version must fail");
    assert_eq!(error.code(), Code::FailedPrecondition);
    assert!(error.message().contains("protocol_version_unsupported"));
}

async fn reject_oversized_frame(path: &std::path::Path) {
    let mut hello = machine_hello();
    hello.machine_version = "x".repeat(SERVER_LIMIT * 2);
    let error = ws::connect_stream(
        path,
        iter([MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::Hello(hello)),
        }]),
    )
    .await
    .expect_err("oversized frame must fail");
    assert_eq!(error.code(), Code::OutOfRange);
    assert!(error.message().contains("message_too_large"));
}

async fn reject_out_of_order_resource_offset(path: &std::path::Path) {
    let bytes = b"bad offset";
    let sha256 = lowercase_hex(&Sha256::digest(bytes));
    let chunks = vec![
        ResourceChunk {
            transfer_id: "bad_offset".into(),
            resource_id: 0,
            offset: 0,
            bytes: bytes[..3].to_vec(),
            eof: false,
            descriptor: Some(ResourceDescriptor {
                file_name: "bad.bin".into(),
                media_type: "application/octet-stream".into(),
                size_bytes: bytes.len() as u64,
                sha256,
                context: Some(run_context("execution_token_17")),
            }),
        },
        ResourceChunk {
            transfer_id: "bad_offset".into(),
            resource_id: 0,
            offset: 4,
            bytes: bytes[3..].to_vec(),
            eof: true,
            descriptor: None,
        },
    ];
    let mut client = connect_uds(path.join("agent.sock"), CLIENT_LIMIT)
        .await
        .expect("offset test client");
    let error = client
        .upload_resource(iter(chunks))
        .await
        .expect_err("out of order offset must fail");
    assert_eq!(error.code(), Code::InvalidArgument);
    assert!(error.message().contains("resource_offset_mismatch"));
}

async fn reject_unknown_frame(path: &std::path::Path) {
    let (sender, receiver) = mpsc::channel(4);
    sender.send(hello_frame()).await.expect("send hello");
    let mut runtime = ws::connect_stream(path, ReceiverStream::new(receiver))
        .await
        .expect("open unknown frame stream")
        .into_inner();
    assert_runtime_hello(
        runtime
            .message()
            .await
            .expect("hello status")
            .expect("hello frame"),
    );
    sender
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: None,
        })
        .await
        .expect("send unknown frame");
    let error = runtime
        .message()
        .await
        .expect_err("unknown frame must fail explicitly");
    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(error.message(), "unknown_frame_type");
}

async fn reject_event_sequence_regression(path: &std::path::Path) {
    let (sender, receiver) = mpsc::channel(4);
    sender.send(hello_frame()).await.expect("send hello");
    let mut runtime = ws::connect_stream(path, ReceiverStream::new(receiver))
        .await
        .expect("open sequence stream")
        .into_inner();
    assert_runtime_hello(
        runtime
            .message()
            .await
            .expect("hello status")
            .expect("hello frame"),
    );
    sender
        .send(event_frame(2, ExecutionEventKind::Started, b"two"))
        .await
        .expect("send sequence two");
    sender
        .send(event_frame(1, ExecutionEventKind::Started, b"one"))
        .await
        .expect("send regressive sequence");
    let error = runtime
        .message()
        .await
        .expect_err("sequence regression must fail explicitly");
    assert_eq!(error.code(), Code::OutOfRange);
    assert_eq!(error.message(), "event_sequence_regression");
}

async fn wait_for_socket(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            // A socket pathname can exist before the listener is ready.
            if connect_uds(path.join("agent.sock"), CLIENT_LIMIT)
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("UDS did not accept connections: {}", path.display()));
}

fn hello_frame() -> MachineFrame {
    MachineFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(machine_frame::Frame::Hello(machine_hello())),
    }
}

fn machine_hello() -> MachineHello {
    MachineHello {
        device_credential: String::new(),
        protocol_version: PROTOCOL_VERSION,
        machine_id: "fixture-machine".into(),
        instance_id: "machine_instance_1".into(),
        machine_version: "0.1.0".into(),
        os: "macos".into(),
        arch: "aarch64".into(),
        harnesses: vec![HarnessInfo {
            capabilities: vec!["resume".into(), "steer".into(), "cancel".into()],
            model_catalog: None,
            model_catalog_error: String::new(),
            harness_id: "pi".into(),
            display_name: "Pi".into(),
            availability: HarnessAvailability::Available.into(),
            version: "1.0".into(),
            reason: String::new(),
        }],
    }
}

fn runtime_hello_frame() -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::Hello(RuntimeHello {
            protocol_version: PROTOCOL_VERSION,
            runtime_instance_id: "runtime_instance_1".into(),
            heartbeat_interval_ms: 5_000,
            max_message_bytes: SERVER_LIMIT as u64,
        })),
    }
}

fn execution_ref() -> ExecutionRef {
    ExecutionRef {
        execution_id: 17,
        attempt: 1,
    }
}

fn run_context(execution_token: &str) -> AgentRunContext {
    AgentRunContext {
        execution: Some(execution_ref()),
        execution_token: execution_token.into(),
    }
}

fn assignment() -> AssignExecution {
    let attachment = ResourceRef {
        resource_id: 7,
        file_name: "brief.md".into(),
        media_type: "text/markdown".into(),
        size_bytes: 5,
        sha256: "a".repeat(64),
    };
    AssignExecution {
        execution: Some(execution_ref()),
        workspace: Some(WorkspaceRef {
            workspace_id: "alpha_team".into(),
        }),
        agent: Some(AgentRuntimeProfile {
            agent: Some(AgentRef {
                workspace_id: "alpha_team".into(),
                agent_id: "researcher".into(),
            }),
            name: "Researcher".into(),
            role: "Analyst".into(),
            bio: "Finds primary evidence".into(),
        }),
        harness: Some(HarnessConfig {
            launch_arguments: Default::default(),
            harness_id: "pi".into(),
            executable_path: "/usr/local/bin/pi".into(),
            model: "deepseek-v4-flash".into(),
            thinking_level: "medium".into(),
            environment_variables: vec![EnvironmentVariable {
                name: "LANG".into(),
                value: "en_US.UTF-8".into(),
            }],
        }),
        session: Some(SessionDirective {
            logical_session_id: "session_17".into(),
            action: SessionAction::ResumeOrStart.into(),
            native_session_id: Some("native_17".into()),
            config_fingerprint: "fingerprint_17".into(),
        }),
        initial_delivery: Some(Delivery {
            delivery_id: 23,
            kind: DeliveryKind::Initial.into(),
            rendered_markdown: "Please inspect the attached brief.".into(),
            snapshot_id: 31,
            trigger_reasons: vec!["mention".into()],
            resources: vec![attachment.clone()],
            created_at: 1_725_000_000_000,
        }),
        system_prompt: "You are the team researcher.".into(),
        tools: vec![ToolDefinition {
            name: "reply".into(),
            description: "Reply to the current chat".into(),
            input_schema_json: r#"{"type":"object"}"#.into(),
            output_schema_json: r#"{"type":"object"}"#.into(),
        }],
        cli_commands: vec![CliCommand {
            name: "lankide".into(),
            description: "Use assigned capabilities".into(),
            argv: vec!["lankide".into(), "tool".into()],
        }],
        resources: vec![attachment],
        execution_token: "execution_token_17".into(),
        created_at: 1_725_000_000_000,
    }
}

fn event_frame(sequence: u64, kind: ExecutionEventKind, payload: &[u8]) -> MachineFrame {
    MachineFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(machine_frame::Frame::ExecutionEvent(ExecutionEvent {
            execution: Some(execution_ref()),
            sequence,
            occurred_at: 1_725_000_000_000 + sequence as i64,
            kind: kind.into(),
            payload: payload.to_vec(),
        })),
    }
}

fn assert_runtime_hello(frame: RuntimeFrame) {
    validate_runtime_frame(&frame).expect("valid runtime hello DTO");
    match frame.frame {
        Some(runtime_frame::Frame::Hello(hello)) => {
            assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
            assert_eq!(hello.runtime_instance_id, "runtime_instance_1");
            assert_eq!(hello.max_message_bytes, SERVER_LIMIT as u64);
        }
        other => panic!("expected RuntimeHello, got {other:?}"),
    }
}

fn expect_assignment(frame: RuntimeFrame) -> AssignExecution {
    validate_runtime_frame(&frame).expect("valid assignment DTO");
    match frame.frame {
        Some(runtime_frame::Frame::AssignExecution(assign)) => assign,
        other => panic!("expected AssignExecution, got {other:?}"),
    }
}

fn assert_complete_assignment(assign: &AssignExecution) {
    assert_eq!(assign.execution, Some(execution_ref()));
    assert_eq!(
        assign.workspace.as_ref().unwrap().workspace_id,
        "alpha_team"
    );
    assert_eq!(
        assign
            .agent
            .as_ref()
            .unwrap()
            .agent
            .as_ref()
            .unwrap()
            .agent_id,
        "researcher"
    );
    assert_eq!(assign.harness.as_ref().unwrap().harness_id, "pi");
    assert_eq!(
        assign
            .session
            .as_ref()
            .unwrap()
            .native_session_id
            .as_deref(),
        Some("native_17")
    );
    assert_eq!(
        assign
            .initial_delivery
            .as_ref()
            .unwrap()
            .resources
            .first()
            .unwrap()
            .resource_id,
        7
    );
    assert_eq!(assign.tools.first().unwrap().name, "reply");
    assert_eq!(assign.cli_commands.first().unwrap().name, "lankide");
    assert_eq!(assign.execution_token, "execution_token_17");
}

fn assert_ack(frame: RuntimeFrame, expected: u64) {
    validate_runtime_frame(&frame).expect("valid ack DTO");
    match frame.frame {
        Some(runtime_frame::Frame::ExecutionEventAck(ack)) => {
            assert_eq!(ack.execution, Some(execution_ref()));
            assert_eq!(ack.max_saved_sequence, expected);
        }
        other => panic!("expected ExecutionEventAck, got {other:?}"),
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

// A native Agent can use JSON directly, but cannot use management, another
// attempt's credential, an older protocol, or unbounded request lengths.
async fn exchange_agent_json_socket(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    assert_eq!(
        fs::metadata(path.join("agent.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let context = run_context("execution_token_17");
    let mut call = serde_json::json!({
        "protocol_version": PROTOCOL_VERSION,
        "method": "invoke_capability",
        "context": context,
        "payload": CapabilityInvokeRequest {
            context: Some(context.clone()), capability_name: "reply".into(),
            arguments_json: "{}".into(), invocation_id: "json-native".into(),
        },
    });
    for expected in [
        "success",
        "permission_denied",
        "unimplemented",
        "failed_precondition",
        "resource_exhausted",
    ] {
        match expected {
            "permission_denied" => {
                call["context"]["execution_token"] = "wrong_token".into();
                call["payload"]["context"]["execution_token"] = "wrong_token".into();
            }
            "unimplemented" => call["method"] = "list_workspaces".into(),
            "failed_precondition" => call["protocol_version"] = (PROTOCOL_VERSION - 1).into(),
            _ => {}
        }
        let mut socket = tokio::net::UnixStream::connect(path.join("agent.sock"))
            .await
            .unwrap();
        if expected == "resource_exhausted" {
            socket
                .write_u32((agent::MAX_CALL_BYTES + 1) as u32)
                .await
                .unwrap();
        } else {
            let bytes = serde_json::to_vec(&call).unwrap();
            socket.write_u32(bytes.len() as u32).await.unwrap();
            socket.write_all(&bytes).await.unwrap();
        }
        let value = tokio::time::timeout(Duration::from_secs(2), async {
            let length = socket.read_u32().await.unwrap() as usize;
            assert!(length <= agent::MAX_CALL_BYTES);
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        })
        .await
        .expect("bounded Agent JSON reply");
        if expected == "success" {
            assert_eq!(value["Ok"]["success"], true);
        } else {
            assert_eq!(value["Err"]["code"], expected, "{value}");
        }
    }
}
