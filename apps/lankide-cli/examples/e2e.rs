use lankide_cli::{RunConfig, execute};
use lankide_protocol as wire;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;
use tokio_stream::iter;
use tonic::Status;

#[derive(Clone, Default)]
struct Peer {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    contexts: Vec<wire::AgentRunContext>,
    invocations: Vec<wire::CapabilityInvokeRequest>,
    sends: Vec<wire::SendMessageRequest>,
    uploads: Vec<Vec<u8>>,
    profile_updates: Vec<wire::UpdateSelfProfileRequest>,
}

#[tonic::async_trait]
impl wire::RuntimePeer for Peer {
    async fn upload_resource(
        &self,
        mut chunks: wire::ResourceChunkStream,
    ) -> Result<wire::ResourceRef, Status> {
        use tokio_stream::StreamExt;
        let mut bytes = Vec::new();
        let mut descriptor = None;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            descriptor = descriptor.or(chunk.descriptor);
            bytes.extend_from_slice(&chunk.bytes);
            if chunk.eof {
                break;
            }
        }
        let descriptor = descriptor.ok_or_else(|| Status::invalid_argument("descriptor"))?;
        if let Some(context) = descriptor.context.clone() {
            self.state.lock().expect("state").contexts.push(context);
        }
        let id = 40 + self.state.lock().expect("state").uploads.len() as i64 + 1;
        self.state.lock().expect("state").uploads.push(bytes);
        Ok(wire::ResourceRef {
            resource_id: id,
            file_name: descriptor.file_name,
            media_type: descriptor.media_type,
            size_bytes: descriptor.size_bytes,
            sha256: descriptor.sha256,
        })
    }

    async fn download_resource(
        &self,
        request: wire::ResourceRequest,
    ) -> Result<wire::ResourceChunkStream, Status> {
        let context = request.context.clone();
        if let Some(context) = request.context {
            self.state.lock().expect("state").contexts.push(context);
        }
        let bytes = b"downloaded-resource".to_vec();
        let descriptor = wire::ResourceDescriptor {
            file_name: "download.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            size_bytes: bytes.len() as u64,
            sha256: lowercase_hex(&Sha256::digest(&bytes)),
            context,
        };
        Ok(Box::pin(iter(vec![Ok(wire::ResourceChunk {
            transfer_id: request.transfer_id,
            resource_id: request.resource_id,
            offset: 0,
            bytes,
            eof: true,
            descriptor: Some(descriptor),
        })])))
    }

    async fn list_capabilities(
        &self,
        context: wire::AgentRunContext,
    ) -> Result<wire::CapabilityList, Status> {
        self.state.lock().expect("state").contexts.push(context);
        Ok(wire::CapabilityList {
            capabilities: vec![wire::Capability {
                name: "message_recall".to_owned(),
                display_name: "Recall message".to_owned(),
                description:
                    "Recall one previously sent message when the connected service supports it."
                        .to_owned(),
                kind: wire::CapabilityKind::Tool as i32,
                input_schema_json: r#"{"type":"object","required":["message_id"]}"#.to_owned(),
                output_schema_json: r#"{"type":"object"}"#.to_owned(),
            }],
        })
    }

    async fn invoke_capability(
        &self,
        request: wire::CapabilityInvokeRequest,
    ) -> Result<wire::CapabilityInvokeResult, Status> {
        if let Some(context) = request.context.clone() {
            self.state.lock().expect("state").contexts.push(context);
        }
        self.state
            .lock()
            .expect("state")
            .invocations
            .push(request.clone());
        Ok(wire::CapabilityInvokeResult {
            invocation_id: request.invocation_id,
            success: true,
            content_markdown: "Message recalled.".to_owned(),
            structured_content_json: r#"{"recalled":true}"#.to_owned(),
            resources: Vec::new(),
            error: None,
        })
    }

    async fn send_message(
        &self,
        request: wire::SendMessageRequest,
    ) -> Result<wire::SendMessageResult, Status> {
        if let Some(context) = request.context.clone() {
            self.state.lock().expect("state").contexts.push(context);
        }
        self.state.lock().expect("state").sends.push(request);
        Ok(wire::SendMessageResult {
            accepted: true,
            message_id: 51,
            provider_message_id: "provider-51".to_owned(),
            error: None,
        })
    }

    async fn get_content(
        &self,
        request: wire::ContentRequest,
    ) -> Result<wire::ContentResult, Status> {
        if let Some(context) = request.context {
            self.state.lock().expect("state").contexts.push(context);
        }
        Ok(wire::ContentResult {
            content_id: request.content_id,
            content_markdown: "latest message".to_owned(),
            structured_content_json: r#"{"unread":2}"#.to_owned(),
            resources: Vec::new(),
        })
    }

    async fn get_self_profile(
        &self,
        context: wire::AgentRunContext,
    ) -> Result<wire::AgentProfile, Status> {
        self.state.lock().expect("state").contexts.push(context);
        Ok(profile("Bruce", "Builder", None))
    }

    async fn update_self_profile(
        &self,
        request: wire::UpdateSelfProfileRequest,
    ) -> Result<wire::AgentProfile, Status> {
        if let Some(context) = request.context.clone() {
            self.state.lock().expect("state").contexts.push(context);
        }
        let avatar = request.avatar.as_ref().map(|upload| wire::AgentAvatarFile {
            path: "resources/team/agent_1_avatar.jpg".to_owned(),
            media_type: upload.media_type.clone(),
            bytes: upload.bytes.clone(),
        });
        let name = request.name.clone().unwrap_or_else(|| "Bruce".to_owned());
        self.state
            .lock()
            .expect("state")
            .profile_updates
            .push(request);
        Ok(profile(&name, "Builder", avatar))
    }
}

fn profile(name: &str, role: &str, avatar: Option<wire::AgentAvatarFile>) -> wire::AgentProfile {
    wire::AgentProfile {
        agent: Some(wire::AgentRef {
            workspace_id: "team".to_owned(),
            agent_id: "agent_1".to_owned(),
        }),
        name: name.to_owned(),
        role: role.to_owned(),
        bio: "Build something awesome.".to_owned(),
        avatar,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_id = format!("cli-{started}-{}", std::process::id());
    let artifact_dir = workspace_root()
        .join("target/e2e-artifacts/cli")
        .join(&run_id);
    fs::create_dir_all(&artifact_dir)?;
    let temp = tempfile::tempdir()?;
    let socket = temp.path().join("runtime.sock");
    let peer = Peer::default();
    let peer_state = Arc::clone(&peer.state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_socket = socket.clone();
    let server = tokio::spawn(async move {
        wire::serve_uds(
            server_socket,
            peer,
            wire::TransportConfig::default(),
            async move {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });
    wait_for_socket(&socket).await?;

    let config = RunConfig {
        socket,
        agent_home: temp.path().join("agent-home"),
        execution_id: 7,
        attempt: 2,
        execution_token: "execution-token-e2e".to_owned(),
    };
    let capabilities = execute(config.clone(), strings(["capabilities"])).await?;
    assert_eq!(capabilities["capabilities"][0]["name"], "message_recall");
    assert!(
        capabilities["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability["name"] == "profile")
    );

    let current_profile = execute(config.clone(), strings(["profile"])).await?;
    assert_eq!(current_profile["name"], "Bruce");
    assert!(current_profile["avatar_path"].is_null());
    let avatar_source = temp.path().join("portrait.jpg");
    fs::write(&avatar_source, b"jpeg-avatar-e2e")?;
    let updated_profile = execute(
        config.clone(),
        vec![
            "profile".to_owned(),
            "update".to_owned(),
            "--name".to_owned(),
            "Bruce Lee".to_owned(),
            "--avatar".to_owned(),
            avatar_source.display().to_string(),
        ],
    )
    .await?;
    assert_eq!(updated_profile["name"], "Bruce Lee");
    let local_avatar = config.agent_home.join("agent_1_avatar.jpg");
    assert_eq!(
        updated_profile["avatar_path"],
        local_avatar.display().to_string()
    );
    assert_eq!(fs::read(local_avatar)?, b"jpeg-avatar-e2e");

    let arguments_file = temp.path().join("recall.json");
    fs::write(&arguments_file, r#"{"message_id":51}"#)?;
    let invoked = execute(
        config.clone(),
        vec![
            "invoke".to_owned(),
            "message_recall".to_owned(),
            "--arguments-file".to_owned(),
            arguments_file.display().to_string(),
            "--invocation-id".to_owned(),
            "recall-e2e".to_owned(),
        ],
    )
    .await?;
    assert_eq!(invoked["success"], true);

    let content_file = temp.path().join("message.md");
    fs::write(&content_file, "A long message loaded from content-file.")?;
    let attachment = temp.path().join("attachment.txt");
    fs::write(&attachment, "attachment bytes")?;
    let sent = execute(
        config.clone(),
        vec![
            "send".to_owned(),
            "--channel".to_owned(),
            "group_chat".to_owned(),
            "--chat".to_owned(),
            "provider:group_chat:12".to_owned(),
            "--content-file".to_owned(),
            content_file.display().to_string(),
            "--attachment".to_owned(),
            attachment.display().to_string(),
            "--reply-to".to_owned(),
            "50".to_owned(),
            "--idempotency-key".to_owned(),
            "send-e2e".to_owned(),
            "--force".to_owned(),
        ],
    )
    .await?;
    assert_eq!(sent["accepted"], true);

    let content = execute(
        config.clone(),
        strings(["content", "--kind", "message", "--id", "51"]),
    )
    .await?;
    assert_eq!(content["structured_content"]["unread"], 2);

    let output = temp.path().join("downloads/result.txt");
    let downloaded = execute(
        config,
        vec![
            "download".to_owned(),
            "--resource-id".to_owned(),
            "41".to_owned(),
            "--output".to_owned(),
            output.display().to_string(),
        ],
    )
    .await?;
    assert_eq!(downloaded["size_bytes"], 19);
    assert_eq!(fs::read(&output)?, b"downloaded-resource");

    let state = peer_state.lock().expect("state");
    assert!(state.contexts.iter().all(|context| {
        context.execution_token == "execution-token-e2e"
            && context
                .execution
                .as_ref()
                .is_some_and(|execution| execution.execution_id == 7 && execution.attempt == 2)
    }));
    assert_eq!(state.invocations.len(), 1);
    assert_eq!(state.invocations[0].arguments_json, r#"{"message_id":51}"#);
    assert_eq!(state.sends.len(), 1);
    assert!(state.sends[0].force);
    assert_eq!(state.sends[0].reply_to_message_id, Some(50));
    assert_eq!(state.sends[0].resources.len(), 1);
    assert_eq!(state.uploads, vec![b"attachment bytes".to_vec()]);
    assert_eq!(state.profile_updates.len(), 1);
    assert_eq!(state.profile_updates[0].name.as_deref(), Some("Bruce Lee"));
    drop(state);

    let _ = shutdown_tx.send(());
    server.await??;
    let result = json!({
        "run_id": run_id,
        "module": "cli",
        "classification": "hermetic",
        "scenario": "execution_scoped_capability_cli_over_uds",
        "status": "pass",
        "cleanup": "pass",
        "checks": [
            "capabilities",
            "profile_get_and_update",
            "invoke_arguments_file",
            "send_content_file_reply_force_attachment",
            "content",
            "download"
        ]
    });
    let artifact = artifact_dir.join("result.json");
    fs::write(&artifact, serde_json::to_vec_pretty(&result)?)?;
    println!("artifact={}", artifact.display());
    Ok(())
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_owned).collect()
}

async fn wait_for_socket(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..100 {
        if path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err("Runtime UDS did not become ready".into())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("CLI app is nested under the workspace")
        .to_path_buf()
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut value, "{byte:02x}");
    }
    value
}
