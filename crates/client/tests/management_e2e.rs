#[path = "support/management.rs"]
mod management_fixture;
use lankide_client::{
    AgentAvatarUpload, AgentDraft, Avatar, ClientError, ManagementClient, ManagementEventKind,
    RuntimeManagementClient, ThinkingLevel, WorkspaceDraft, WorkspaceStatus,
};
use lankide_protocol as wire;
use management_fixture::{MockManagementClient, MockState, local_harnesses};
use serde_json::json;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tempfile::TempDir;
use tokio::sync::{Mutex, oneshot};
use tonic::Status;

const NORMAL: u8 = 0;
const PROVIDER_ERROR: u8 = 1;
const UNAVAILABLE: u8 = 2;
const WRONG_REQUEST_ID: u8 = 3;
const OUT_OF_ORDER_STREAM: u8 = 4;
const ABORTED_STREAM: u8 = 5;

#[derive(Clone, Default)]
struct Peer {
    data: Arc<Mutex<PeerData>>,
    fault: Arc<AtomicU8>,
}

#[derive(Default)]
struct PeerData {
    workspaces: HashMap<String, wire::Workspace>,
    agents: HashMap<(String, String), wire::Agent>,
}

fn wref(id: &str) -> wire::WorkspaceRef {
    wire::WorkspaceRef {
        workspace_id: id.into(),
    }
}
fn workspace(
    id: String,
    name: String,
    rules: String,
    provider_id: String,
    config: String,
) -> wire::Workspace {
    wire::Workspace {
        workspace: Some(wref(&id)),
        name,
        cover: None,
        rules,
        provider_id,
        provider_config_json: config,
        desired_state: wire::DesiredState::Paused as i32,
        observed_state: wire::ObservedState::Paused as i32,
        status_message: String::new(),
        created_at: 1,
        updated_at: 1,
    }
}
fn agent(input: wire::CreateAgentRequest) -> wire::Agent {
    wire::Agent {
        pause_reason: None,
        agent: input.agent,
        name: input.name,
        role: input.role,
        bio: input.bio,
        avatar_path: String::new(),
        harness_id: input.harness_id,
        model: input.model,
        thinking_level: input.thinking_level,
        auto_reaction: input.auto_reaction,
        soul: input.soul,
        cli_path: input.cli_path,
        environment_variables: input.environment_variables,
        computer_id: input.computer_id,
        attention: input.attention,
        provider_agent_config_json: input.provider_agent_config_json,
        created_at: 1,
        updated_at: 1,
    }
}
fn empty_stream<T: Send + 'static>()
-> std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>> {
    Box::pin(tokio_stream::empty())
}

impl Peer {
    fn inject(&self, fault: u8) {
        self.fault.store(fault, Ordering::SeqCst);
    }
    fn unary_fault(&self) -> Result<bool, Status> {
        match self.fault.swap(NORMAL, Ordering::SeqCst) {
            PROVIDER_ERROR => Err(Status::permission_denied("provider_scope_denied")),
            UNAVAILABLE => Err(Status::unavailable("runtime_disconnected")),
            WRONG_REQUEST_ID => Ok(true),
            other => {
                self.fault.store(other, Ordering::SeqCst);
                Ok(false)
            }
        }
    }
}

#[tonic::async_trait]
impl wire::RuntimePeer for Peer {
    async fn connect_computer(
        &self,
        _: wire::ComputerHello,
        _: wire::ComputerFrameStream,
    ) -> Result<wire::RuntimeFrameStream, Status> {
        Ok(empty_stream())
    }
    async fn upload_resource(
        &self,
        _: wire::ResourceChunkStream,
    ) -> Result<wire::ResourceRef, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn download_resource(
        &self,
        _: wire::ResourceRequest,
    ) -> Result<wire::ResourceChunkStream, Status> {
        Err(Status::unimplemented("unused"))
    }

    async fn get_workspace_image(
        &self,
        r: wire::WorkspaceImageRequest,
    ) -> Result<wire::WorkspaceImageResponse, Status> {
        use sha2::{Digest, Sha256};
        let wrong = self.unary_fault()?;
        let bytes = include_bytes!("../../runtime/tests/assets/profile.png").to_vec();
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(wire::WorkspaceImageResponse {
            request_id: if wrong {
                "wrong-image-request".into()
            } else {
                r.request_id
            },
            workspace: r.workspace,
            resource: Some(wire::ResourceRef {
                resource_id: r.resource_id,
                file_name: "profile.png".into(),
                media_type: "image/png".into(),
                size_bytes: bytes.len() as u64,
                sha256: if r.resource_id == 10 {
                    "bad-checksum".into()
                } else {
                    sha256
                },
            }),
            bytes,
        })
    }
    async fn upload_workspace_image(
        &self,
        r: wire::UploadWorkspaceImageRequest,
    ) -> Result<wire::UploadWorkspaceImageResponse, Status> {
        use sha2::{Digest, Sha256};
        let wrong = self.unary_fault()?;
        let sha256 = Sha256::digest(&r.bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(wire::UploadWorkspaceImageResponse {
            request_id: if wrong {
                "wrong-upload-request".into()
            } else {
                r.request_id
            },
            workspace: r.workspace,
            resource: Some(wire::ResourceRef {
                resource_id: 12,
                file_name: r.file_name,
                media_type: r.media_type,
                size_bytes: r.bytes.len() as u64,
                sha256,
            }),
        })
    }
    async fn list_workspaces(
        &self,
        r: wire::ListWorkspacesRequest,
    ) -> Result<wire::ListWorkspacesResponse, Status> {
        let wrong = self.unary_fault()?;
        Ok(wire::ListWorkspacesResponse {
            request_id: if wrong {
                "late-other-request".into()
            } else {
                r.request_id
            },
            workspaces: self
                .data
                .lock()
                .await
                .workspaces
                .values()
                .cloned()
                .collect(),
        })
    }
    async fn get_workspace(&self, r: wire::GetWorkspaceRequest) -> Result<wire::Workspace, Status> {
        let id = r.workspace.unwrap().workspace_id;
        self.data
            .lock()
            .await
            .workspaces
            .get(&id)
            .cloned()
            .ok_or_else(|| Status::not_found("workspace_not_found"))
    }
    async fn create_workspace(
        &self,
        r: wire::CreateWorkspaceRequest,
    ) -> Result<wire::Workspace, Status> {
        let v = workspace(
            r.workspace_id.clone(),
            r.name,
            r.rules,
            r.provider_id,
            r.provider_config_json,
        );
        let mut data = self.data.lock().await;
        if data.workspaces.contains_key(&r.workspace_id) {
            return Err(Status::already_exists("duplicate_workspace_id"));
        }
        data.workspaces.insert(r.workspace_id, v.clone());
        Ok(v)
    }
    async fn update_workspace(
        &self,
        r: wire::UpdateWorkspaceRequest,
    ) -> Result<wire::Workspace, Status> {
        let id = r.workspace.unwrap().workspace_id;
        let mut d = self.data.lock().await;
        let v = d
            .workspaces
            .get_mut(&id)
            .ok_or_else(|| Status::not_found("workspace_not_found"))?;
        if let Some(x) = r.name {
            v.name = x
        }
        if let Some(x) = r.rules {
            v.rules = x
        }
        if let Some(x) = r.provider_config_json {
            v.provider_config_json = x
        }
        Ok(v.clone())
    }
    async fn set_workspace_state(
        &self,
        r: wire::SetWorkspaceStateRequest,
    ) -> Result<wire::Workspace, Status> {
        let id = r.workspace.unwrap().workspace_id;
        let mut d = self.data.lock().await;
        let v = d
            .workspaces
            .get_mut(&id)
            .ok_or_else(|| Status::not_found("workspace_not_found"))?;
        v.desired_state = r.desired_state;
        v.observed_state = if r.desired_state == wire::DesiredState::Paused as i32 {
            wire::ObservedState::Paused as i32
        } else {
            wire::ObservedState::Running as i32
        };
        Ok(v.clone())
    }
    async fn list_agents(
        &self,
        r: wire::ListAgentsRequest,
    ) -> Result<wire::ListAgentsResponse, Status> {
        let id = r.workspace.unwrap().workspace_id;
        if id == "telegram_team" {
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
        let values = self
            .data
            .lock()
            .await
            .agents
            .iter()
            .filter(|((w, _), _)| w == &id)
            .map(|(_, a)| a.clone())
            .collect();
        Ok(wire::ListAgentsResponse {
            request_id: r.request_id,
            agents: values,
        })
    }
    async fn get_agent(&self, r: wire::GetAgentRequest) -> Result<wire::Agent, Status> {
        let a = r.agent.unwrap();
        self.data
            .lock()
            .await
            .agents
            .get(&(a.workspace_id, a.agent_id))
            .cloned()
            .ok_or_else(|| Status::not_found("agent_not_found"))
    }
    async fn get_agent_avatar(
        &self,
        r: wire::GetAgentAvatarRequest,
    ) -> Result<wire::GetAgentAvatarResponse, Status> {
        let bytes = include_bytes!("../../runtime/tests/assets/profile.png").to_vec();
        let agent = r.agent.unwrap();
        Ok(wire::GetAgentAvatarResponse {
            request_id: r.request_id,
            avatar: Some(wire::AgentAvatarFile {
                path: format!(
                    "resources/{}/{}_avatar.png",
                    agent.workspace_id, agent.agent_id
                ),
                media_type: "image/png".into(),
                bytes,
            }),
            agent: Some(agent),
        })
    }
    async fn create_agent(&self, r: wire::CreateAgentRequest) -> Result<wire::Agent, Status> {
        let a = agent(r);
        let key = {
            let x = a.agent.as_ref().unwrap();
            (x.workspace_id.clone(), x.agent_id.clone())
        };
        let mut d = self.data.lock().await;
        if d.agents.contains_key(&key) {
            return Err(Status::already_exists("duplicate_agent_id"));
        }
        d.agents.insert(key, a.clone());
        Ok(a)
    }
    async fn resume_agent(&self, r: wire::GetAgentRequest) -> Result<wire::Agent, Status> {
        let agent = r
            .agent
            .ok_or_else(|| Status::invalid_argument("agent required"))?;
        let mut d = self.data.lock().await;
        let record = d
            .agents
            .get_mut(&(agent.workspace_id, agent.agent_id))
            .ok_or_else(|| Status::not_found("agent_not_found"))?;
        record.pause_reason = None;
        Ok(record.clone())
    }
    async fn update_agent(&self, r: wire::UpdateAgentRequest) -> Result<wire::Agent, Status> {
        let key = {
            let x = r.agent.as_ref().unwrap();
            (x.workspace_id.clone(), x.agent_id.clone())
        };
        let mut d = self.data.lock().await;
        let a = d
            .agents
            .get_mut(&key)
            .ok_or_else(|| Status::not_found("agent_not_found"))?;
        if let Some(x) = r.name {
            a.name = x
        }
        if let Some(x) = r.role {
            a.role = x
        }
        if let Some(x) = r.bio {
            a.bio = x
        }
        if let Some(x) = r.harness_id {
            a.harness_id = x
        }
        if let Some(x) = r.model {
            a.model = x
        }
        if let Some(x) = r.thinking_level {
            a.thinking_level = x
        }
        if let Some(x) = r.auto_reaction {
            a.auto_reaction = x
        }
        if let Some(x) = r.soul {
            a.soul = x
        }
        if let Some(x) = r.cli_path {
            a.cli_path = x
        }
        if let Some(x) = r.computer_id {
            a.computer_id = x
        }
        if let Some(environment) = r.environment_variables {
            a.environment_variables = environment.entries;
        }
        if r.avatar.is_some() {
            a.avatar_path = format!("resources/{}/{}_avatar.png", key.0, key.1);
        }
        Ok(a.clone())
    }
    async fn delete_agent(
        &self,
        r: wire::DeleteAgentRequest,
    ) -> Result<wire::DeleteAgentResponse, Status> {
        let agent = r
            .agent
            .ok_or_else(|| Status::invalid_argument("agent_ref_required"))?;
        let deleted = wire::AgentRef {
            workspace_id: agent.workspace_id.clone(),
            agent_id: agent.agent_id.clone(),
        };
        self.data
            .lock()
            .await
            .agents
            .remove(&(agent.workspace_id, agent.agent_id))
            .ok_or_else(|| Status::not_found("agent_not_found"))?;
        Ok(wire::DeleteAgentResponse {
            request_id: r.request_id,
            agent: Some(deleted),
        })
    }
    async fn get_provider_onboarding(
        &self,
        r: wire::GetProviderOnboardingRequest,
    ) -> Result<wire::ProviderOnboardingState, Status> {
        let w = r.workspace.unwrap();
        let provider_id = self
            .data
            .lock()
            .await
            .workspaces
            .get(&w.workspace_id)
            .unwrap()
            .provider_id
            .clone();
        Ok(wire::ProviderOnboardingState {
            workspace: Some(w),
            provider_id,
            step: "credentials".into(),
            status: wire::OnboardingStatus::ActionRequired as i32,
            verification_uri: "https://example.invalid/verify".into(),
            poll_interval_seconds: 5,
            payload_json: "{}".into(),
            error_code: String::new(),
            error_message: String::new(),
        })
    }
    async fn advance_provider_onboarding(
        &self,
        r: wire::AdvanceProviderOnboardingRequest,
    ) -> Result<wire::ProviderOnboardingState, Status> {
        let w = r.workspace.unwrap();
        let provider_id = self
            .data
            .lock()
            .await
            .workspaces
            .get(&w.workspace_id)
            .unwrap()
            .provider_id
            .clone();
        Ok(wire::ProviderOnboardingState {
            workspace: Some(w),
            provider_id,
            step: r.action,
            status: wire::OnboardingStatus::Ready as i32,
            verification_uri: String::new(),
            poll_interval_seconds: 0,
            payload_json: r.field_values_json,
            error_code: String::new(),
            error_message: String::new(),
        })
    }
    async fn get_agent_onboarding(
        &self,
        request: wire::GetAgentOnboardingRequest,
    ) -> Result<wire::AgentOnboardingState, Status> {
        let agent = request.agent.unwrap();
        let provider_id = self
            .data
            .lock()
            .await
            .workspaces
            .get(&agent.workspace_id)
            .unwrap()
            .provider_id
            .clone();
        Ok(wire::AgentOnboardingState {
            agent: Some(agent),
            provider_id,
            step: "create_bot".into(),
            status: wire::OnboardingStatus::ActionRequired as i32,
            verification_uri: "https://example.invalid/verify".into(),
            poll_interval_seconds: 5,
            payload_json: "{}".into(),
            error_code: String::new(),
            error_message: String::new(),
        })
    }
    async fn advance_agent_onboarding(
        &self,
        request: wire::AdvanceAgentOnboardingRequest,
    ) -> Result<wire::AgentOnboardingState, Status> {
        let agent = request.agent.unwrap();
        let provider_id = self
            .data
            .lock()
            .await
            .workspaces
            .get(&agent.workspace_id)
            .unwrap()
            .provider_id
            .clone();
        Ok(wire::AgentOnboardingState {
            agent: Some(agent),
            provider_id,
            step: request.action,
            status: wire::OnboardingStatus::Ready as i32,
            verification_uri: String::new(),
            poll_interval_seconds: 0,
            payload_json: request.field_values_json,
            error_code: String::new(),
            error_message: String::new(),
        })
    }
    async fn get_provider_catalog(
        &self,
        _: wire::CatalogRequest,
    ) -> Result<wire::ProviderCatalog, Status> {
        self.unary_fault()?;
        Ok(wire::ProviderCatalog {
            providers: vec![
                wire::ProviderCatalogItem {
                    provider_id: "feishu".into(),
                    display_name: "飞书".into(),
                    description: String::new(),
                    available: true,
                    capabilities: vec![],
                },
                wire::ProviderCatalogItem {
                    provider_id: "telegram".into(),
                    display_name: "Telegram".into(),
                    description: String::new(),
                    available: true,
                    capabilities: vec![],
                },
            ],
        })
    }
    async fn get_computer_catalog(
        &self,
        _: wire::CatalogRequest,
    ) -> Result<wire::ComputerCatalog, Status> {
        Ok(wire::ComputerCatalog {
            computers: vec![wire::ComputerCatalogItem {
                computer_id: "local".into(),
                display_name: "Local Computer".into(),
                observed_state: wire::ObservedState::Running as i32,
                harnesses: vec![],
                status_message: String::new(),
            }],
        })
    }
    async fn get_harness_catalog(
        &self,
        _: wire::CatalogRequest,
    ) -> Result<wire::HarnessCatalog, Status> {
        Ok(wire::HarnessCatalog {
            harnesses: vec!["pi", "codex", "claude_code"]
                .into_iter()
                .map(|id| wire::HarnessCatalogItem {
                    harness_id: id.into(),
                    display_name: id.into(),
                    description: String::new(),
                    availability: wire::HarnessAvailability::Available as i32,
                    supported_thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                })
                .collect(),
        })
    }
    async fn get_model_catalog(
        &self,
        r: wire::ModelCatalogRequest,
    ) -> Result<wire::ModelCatalog, Status> {
        Ok(wire::ModelCatalog {
            models: vec![wire::ModelCatalogItem {
                supported_thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
                model_id: if r.harness_id == "codex" {
                    "gpt-5.6-luna"
                } else {
                    "deepseek-v4-flash"
                }
                .into(),
                display_name: "Default".into(),
                description: String::new(),
                available: true,
            }],
        })
    }
    async fn get_reaction_catalog(
        &self,
        _: wire::CatalogRequest,
    ) -> Result<wire::ReactionCatalog, Status> {
        Ok(wire::ReactionCatalog {
            reactions: vec![wire::ReactionCatalogItem {
                reaction_id: "thumbsup".into(),
                display_name: "Like".into(),
                emoji: "👍".into(),
                available: true,
            }],
        })
    }
    async fn invoke_provider_interface(
        &self,
        request: wire::InvokeProviderInterfaceRequest,
    ) -> Result<wire::InvokeProviderInterfaceResponse, Status> {
        let wrong = self.unary_fault()?;
        Ok(wire::InvokeProviderInterfaceResponse {
            request_id: if wrong {
                "wrong".into()
            } else {
                request.request_id
            },
            workspace: request.workspace,
            interface_id: request.interface_id,
            output_json: request.input_json,
        })
    }
    async fn list_provider_interfaces(
        &self,
        request: wire::ListProviderInterfacesRequest,
    ) -> Result<wire::ListProviderInterfacesResponse, Status> {
        let wrong = self.unary_fault()?;
        Ok(wire::ListProviderInterfacesResponse {
            request_id: if wrong {
                "wrong".into()
            } else {
                request.request_id
            },
            workspace: request.workspace,
            interfaces: vec![wire::ProviderInterface {
                interface_id: "fixture.echo".into(),
                description: Some("Echo".into()),
                input_schema_json: json!({"type":"object"}).to_string(),
                output_schema_json: json!({"type":"object"}).to_string(),
            }],
        })
    }
    async fn subscribe_management_events(
        &self,
        r: wire::ManagementSubscription,
    ) -> Result<wire::ManagementEventStream, Status> {
        let id = r.workspace.unwrap_or_else(|| wref("feishu_team"));
        let mode = self.fault.swap(NORMAL, Ordering::SeqCst);
        let event = |sequence| wire::ManagementEvent {
            sequence,
            occurred_at: 1,
            kind: wire::ManagementEventKind::CatalogChanged as i32,
            workspace: Some(id.clone()),
            agent: None,
        };
        match mode {
            OUT_OF_ORDER_STREAM => Ok(Box::pin(tokio_stream::iter(vec![
                Ok(event(r.after_sequence + 2)),
                Ok(event(r.after_sequence + 1)),
            ]))),
            ABORTED_STREAM => Ok(Box::pin(tokio_stream::iter(vec![Err(
                Status::unavailable("stream_aborted"),
            )]))),
            _ => Ok(Box::pin(tokio_stream::iter(vec![Ok(event(
                r.after_sequence + 1,
            ))]))),
        }
    }
}

struct Fixture {
    _root: TempDir,
    socket: std::path::PathBuf,
    peer: Peer,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), wire::TransportError>>,
}
impl Fixture {
    async fn start(root: TempDir) -> Result<Self, String> {
        let socket = root.path().join("runtime.sock");
        let peer = Peer::default();
        let (tx, rx) = oneshot::channel();
        let server_socket = socket.clone();
        let server_peer = peer.clone();
        let task = tokio::spawn(async move {
            wire::serve_uds(
                server_socket,
                server_peer,
                wire::TransportConfig::default(),
                async move {
                    let _ = rx.await;
                },
            )
            .await
        });
        for _ in 0..100 {
            if socket.exists() {
                return Ok(Self {
                    _root: root,
                    socket,
                    peer,
                    stop: Some(tx),
                    task,
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await
        }
        Err("UDS did not become ready".into())
    }
    async fn stop(mut self) -> Result<(), String> {
        drop(self.stop.take());
        self.task
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if self.socket.exists() {
            return Err("socket was not cleaned up".into());
        }
        Ok(())
    }
}

fn need(value: bool, message: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(message.into()) }
}

async fn common_contract<C: ManagementClient>(client: &C, prefix: &str) -> Result<(), String> {
    need(
        client
            .list_workspaces()
            .await
            .map_err(|e| e.to_string())?
            .is_empty(),
        "initial workspace list was not empty",
    )?;
    for (suffix, provider) in [("feishu", "feishu"), ("telegram", "telegram")] {
        let w = client
            .create_workspace(WorkspaceDraft {
                id: format!("{prefix}_{suffix}"),
                name: suffix.into(),
                rules: "team rules".into(),
                provider_id: provider.into(),
                provider_config: json!({"opaque":suffix}),
                provider_secrets: json!({"secret_ref": suffix}),
            })
            .await
            .map_err(|e| e.to_string())?;
        need(w.provider.id == provider, "provider identity changed")?;
        let state = client
            .get_provider_onboarding(&w.id)
            .await
            .map_err(|e| e.to_string())?;
        need(
            state.status == lankide_client::OnboardingStatus::ActionRequired,
            "unexpected onboarding state",
        )?;
        let ready = client
            .advance_provider_onboarding(&w.id, "verify", json!({"credential_ref":"opaque"}))
            .await
            .map_err(|e| e.to_string())?;
        need(
            ready.status == lankide_client::OnboardingStatus::Ready,
            "onboarding did not advance",
        )?;
        need(
            ready.payload == json!({"credential_ref":"opaque"})
                && ready.verification_uri.is_none()
                && ready.poll_interval_seconds.is_none(),
            "opaque onboarding payload changed",
        )?;
    }
    let workspace_id = format!("{prefix}_feishu");
    let duplicate_workspace = client
        .create_workspace(WorkspaceDraft {
            id: workspace_id.clone(),
            name: "must not replace existing workspace".into(),
            rules: String::new(),
            provider_id: "telegram".into(),
            provider_config: json!({}),
            provider_secrets: json!({}),
        })
        .await;
    need(
        matches!(
            duplicate_workspace,
            Err(ClientError::DuplicateWorkspaceId(_))
        ),
        "duplicate Workspace identity was not preserved as a business conflict",
    )?;
    let draft = AgentDraft {
        id: "builder".into(),
        name: "Builder".into(),
        role: "RD".into(),
        bio: "Ships".into(),
        avatar: lankide_client::Avatar::Custom("B".into()),
        soul: "Care about quality".into(),
        computer_id: "local".into(),
        harness_id: "claude_code".into(),
        model: "deepseek-v4-flash".into(),
        thinking_level: ThinkingLevel::High,
        auto_reaction: Some("👍".into()),
        cli_path: "/usr/local/bin/dcc".into(),
        environment_variables: vec![("MODE".into(), "test".into())],
        provider_handle: "opaque-provider-bot".into(),
        provider_agent_config: json!({"provider_handle":"opaque-provider-bot"}),
        provider_agent_secrets: json!({"credential":"opaque"}),
    };
    let mut a = client
        .create_agent(&workspace_id, draft.clone())
        .await
        .map_err(|e| e.to_string())?;
    need(
        a.workspace_id == workspace_id
            && a.id == "builder"
            && a.harness_id == "claude_code"
            && a.cli_path.ends_with("dcc"),
        "standard Agent DTO was not preserved",
    )?;
    let duplicate_agent = client.create_agent(&workspace_id, draft).await;
    need(
        matches!(duplicate_agent, Err(ClientError::DuplicateAgentId(_))),
        "duplicate Agent identity was not preserved as a business conflict",
    )?;
    let agent_onboarding = client
        .get_agent_onboarding(&workspace_id, &a.id)
        .await
        .map_err(|error| error.to_string())?;
    need(
        agent_onboarding.status == lankide_client::OnboardingStatus::ActionRequired,
        "unexpected Agent onboarding state",
    )?;
    let agent_ready = client
        .advance_agent_onboarding(&workspace_id, &a.id, "verify", json!({"bot_ref":"opaque"}))
        .await
        .map_err(|error| error.to_string())?;
    need(
        agent_ready.status == lankide_client::OnboardingStatus::Ready
            && agent_ready.payload == json!({"bot_ref":"opaque"})
            && agent_ready.verification_uri.is_none()
            && agent_ready.poll_interval_seconds.is_none(),
        "Agent onboarding did not advance",
    )?;
    a.bio = "Edited".into();
    let updated = client
        .update_agent(
            a,
            Some(AgentAvatarUpload {
                media_type: "image/png".into(),
                bytes: include_bytes!("../../runtime/tests/assets/profile.png").to_vec(),
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    if updated.bio != "Edited"
        || !matches!(&updated.avatar, Avatar::Path(path) if path.ends_with("builder_avatar.png"))
    {
        return Err(format!("agent edit failed: {updated:?}"));
    }
    let resumed = client
        .resume_agent(&workspace_id, "builder")
        .await
        .map_err(|e| e.to_string())?;
    need(
        resumed.pause_reason.is_none() && resumed.bio == "Edited",
        "resume must preserve Agent configuration",
    )?;
    let avatar = client
        .get_agent_avatar(&workspace_id, "builder")
        .await
        .map_err(|e| e.to_string())?;
    need(
        avatar.path.ends_with("builder_avatar.png")
            && avatar.bytes == include_bytes!("../../runtime/tests/assets/profile.png"),
        "agent avatar read failed",
    )?;
    let invalid_dcc = AgentDraft {
        id: "wrong_harness".into(),
        harness_id: "codex".into(),
        cli_path: "/usr/local/bin/dcc".into(),
        ..AgentDraft::default()
    };
    need(
        matches!(
            client.create_agent(&workspace_id, invalid_dcc).await,
            Err(ClientError::Validation(_))
        ),
        "dcc was accepted outside claude_code",
    )?;
    need(
        client
            .get_provider_catalog()
            .await
            .map_err(|e| e.to_string())?
            .len()
            == 2,
        "provider catalog",
    )?;
    need(
        client
            .get_computer_catalog(Some(&workspace_id))
            .await
            .map_err(|e| e.to_string())?[0]
            .id
            == "local",
        "computer catalog",
    )?;
    need(
        client
            .get_harness_catalog(Some(&workspace_id))
            .await
            .map_err(|e| e.to_string())?
            .iter()
            .map(|h| h.id.as_str())
            .collect::<Vec<_>>()
            == ["pi", "codex", "claude_code"],
        "harness catalog",
    )?;
    let models = client
        .get_model_catalog(Some(&workspace_id), "local", "claude_code")
        .await
        .map_err(|e| e.to_string())?;
    need(
        models.len() == 1 && models[0].thinking_levels == ["low", "medium", "high"],
        "native model reasoning levels survive client conversion",
    )?;
    need(
        !client
            .get_reaction_catalog(Some(&workspace_id))
            .await
            .map_err(|e| e.to_string())?
            .is_empty(),
        "reaction catalog",
    )?;
    need(
        client
            .invoke_provider_interface(&workspace_id, "fixture.echo", json!({"value":"ok"}))
            .await
            .map_err(|e| e.to_string())?
            == json!({"value":"ok"}),
        "provider interface",
    )?;
    let interfaces = client
        .list_provider_interfaces(&workspace_id)
        .await
        .map_err(|e| e.to_string())?;
    need(
        interfaces.len() == 1
            && interfaces[0].id == "fixture.echo"
            && interfaces[0].input_schema == json!({"type":"object"}),
        "provider interface catalog",
    )?;
    client
        .delete_agent(&workspace_id, "builder")
        .await
        .map_err(|e| e.to_string())?;
    need(
        client
            .list_agents(&workspace_id)
            .await
            .map_err(|e| e.to_string())?
            .is_empty(),
        "delete did not remove the Agent",
    )?;
    need(
        client
            .set_workspace_state(&workspace_id, WorkspaceStatus::Paused)
            .await
            .map_err(|e| e.to_string())?
            .status
            == WorkspaceStatus::Paused,
        "pause failed",
    )?;
    need(
        client
            .set_workspace_state(&workspace_id, WorkspaceStatus::Active)
            .await
            .map_err(|e| e.to_string())?
            .status
            == WorkspaceStatus::Active,
        "resume failed",
    )?;
    let mut events = client
        .subscribe_management_events(Some(&workspace_id), 0)
        .await
        .map_err(|e| e.to_string())?;
    let event = events
        .next()
        .await
        .ok_or("event stream ended")?
        .map_err(|e| e.to_string())?;
    need(
        event.kind == ManagementEventKind::CatalogChanged && event.workspace_id == workspace_id,
        "event identity changed",
    )?;
    drop(events);
    Ok(())
}

async fn runtime_scenario() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    let client = RuntimeManagementClient::connect_lazy(root.path().join("runtime.sock"));
    need(
        client.list_workspaces().await.is_err(),
        "initial unavailable Runtime should be reported",
    )?;
    let fixture = Fixture::start(root).await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if client.list_workspaces().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| "same client did not reconnect after Runtime startup".to_owned())?;
    common_contract(&client, "runtime").await?;
    let onboarding = client
        .get_provider_onboarding("runtime_feishu")
        .await
        .map_err(|error| error.to_string())?;
    need(
        onboarding.verification_uri.as_deref() == Some("https://example.invalid/verify")
            && onboarding.poll_interval_seconds == Some(5),
        "explicit onboarding fields changed",
    )?;
    {
        let mut data = fixture.peer.data.lock().await;
        let workspace = data.workspaces.get_mut("runtime_feishu").unwrap();
        workspace.desired_state = wire::DesiredState::Running as i32;
        workspace.observed_state = wire::ObservedState::Error as i32;
        workspace.status_message = "Connection needs attention".into();
        workspace.cover = Some(wire::ResourceRef {
            resource_id: 9,
            ..Default::default()
        });
    }
    let observed = client
        .get_workspace("runtime_feishu")
        .await
        .map_err(|e| e.to_string())?;
    need(
        observed.status == WorkspaceStatus::Active
            && observed.observed_state == lankide_client::WorkspaceObservedState::Error
            && observed.status_message == "Connection needs attention"
            && observed.cover_resource_id == Some(9),
        "actual state, error and cover were lost in client conversion",
    )?;

    let image = client
        .get_workspace_image("runtime_feishu", 9)
        .await
        .map_err(|e| e.to_string())?;
    need(
        image.bytes == include_bytes!("../../runtime/tests/assets/profile.png"),
        "client changed image bytes",
    )?;
    let uploaded_avatar = client
        .upload_workspace_image(
            "runtime_feishu",
            "avatar.png",
            "image/png",
            include_bytes!("../../runtime/tests/assets/profile.png").to_vec(),
        )
        .await
        .map_err(|e| e.to_string())?;
    need(
        uploaded_avatar == 12,
        "client changed uploaded image identity",
    )?;
    fixture.peer.inject(WRONG_REQUEST_ID);
    need(
        client
            .get_workspace_image("runtime_feishu", 9)
            .await
            .is_err(),
        "client accepted stale image response",
    )?;
    need(
        client
            .get_workspace_image("runtime_feishu", 10)
            .await
            .is_err(),
        "client accepted corrupt image checksum",
    )?;
    let (feishu, telegram) = tokio::join!(
        client.list_agents("runtime_feishu"),
        client.list_agents("runtime_telegram")
    );
    need(
        feishu
            .map_err(|e| e.to_string())?
            .iter()
            .all(|a| a.workspace_id == "runtime_feishu"),
        "late response polluted Feishu workspace",
    )?;
    need(
        telegram
            .map_err(|e| e.to_string())?
            .iter()
            .all(|a| a.workspace_id == "runtime_telegram"),
        "late response polluted Telegram workspace",
    )?;
    fixture.peer.inject(WRONG_REQUEST_ID);
    need(
        matches!(
            client.list_workspaces().await,
            Err(ClientError::Protocol(_))
        ),
        "response correlation mismatch accepted",
    )?;
    need(
        client.list_workspaces().await.is_ok(),
        "retry after correlation failure failed",
    )?;
    fixture.peer.inject(PROVIDER_ERROR);
    need(
        matches!(
            client.get_provider_catalog().await,
            Err(ClientError::Provider(_))
        ),
        "provider error mapping",
    )?;
    need(
        client.get_provider_catalog().await.is_ok(),
        "provider retry failed",
    )?;
    fixture.peer.inject(UNAVAILABLE);
    need(
        matches!(
            client.get_provider_catalog().await,
            Err(ClientError::Unavailable(_))
        ),
        "disconnect/unavailable mapping",
    )?;
    need(
        client.get_provider_catalog().await.is_ok(),
        "unavailable retry failed",
    )?;
    let invalid = client
        .create_workspace(WorkspaceDraft {
            id: "INVALID".into(),
            name: "bad".into(),
            rules: String::new(),
            provider_id: "feishu".into(),
            provider_config: json!({}),
            provider_secrets: json!({}),
        })
        .await;
    need(
        matches!(invalid, Err(ClientError::Validation(_))),
        "wire validation error mapping",
    )?;
    fixture.peer.inject(OUT_OF_ORDER_STREAM);
    let mut stream = client
        .subscribe_management_events(Some("runtime_feishu"), 0)
        .await
        .map_err(|e| e.to_string())?;
    need(
        stream.next().await.unwrap().is_ok(),
        "first stream event missing",
    )?;
    need(
        matches!(stream.next().await.unwrap(), Err(ClientError::Protocol(_))),
        "out-of-order stream accepted",
    )?;
    drop(stream);
    let mut retry = client
        .subscribe_management_events(Some("runtime_feishu"), 2)
        .await
        .map_err(|e| e.to_string())?;
    need(
        retry.next().await.unwrap().is_ok(),
        "retry after out-of-order stream failed",
    )?;
    drop(retry);
    fixture.peer.inject(ABORTED_STREAM);
    let mut stream = client
        .subscribe_management_events(Some("runtime_feishu"), 0)
        .await
        .map_err(|e| e.to_string())?;
    need(
        matches!(
            stream.next().await.unwrap(),
            Err(ClientError::Unavailable(_))
        ),
        "stream abort mapping",
    )?;
    drop(stream);
    let mut retry = client
        .subscribe_management_events(Some("runtime_feishu"), 0)
        .await
        .map_err(|e| e.to_string())?;
    need(
        retry.next().await.unwrap().is_ok(),
        "retry after stream abort failed",
    )?;
    drop(retry);
    drop(client);
    fixture.stop().await
}

async fn mock_scenario() -> Result<(), String> {
    let client = MockManagementClient::empty();
    need(
        client
            .list_workspaces()
            .await
            .map_err(|e| e.to_string())?
            .is_empty(),
        "mock empty state",
    )?;
    client.set_state(MockState::Loading).await;
    need(
        matches!(
            client.list_workspaces().await,
            Err(ClientError::Unavailable(_))
        ),
        "mock loading",
    )?;
    client.set_state(MockState::Success).await;
    common_contract(&client, "mock").await?;
    client.set_state(MockState::Error).await;
    need(
        matches!(
            client.list_workspaces().await,
            Err(ClientError::Provider(_))
        ),
        "mock error",
    )?;
    client.set_state(MockState::Retrying).await;
    need(
        matches!(
            client.list_workspaces().await,
            Err(ClientError::Unavailable(_))
        ),
        "mock retrying",
    )?;
    client.set_state(MockState::Success).await;
    need(
        client.list_workspaces().await.is_ok(),
        "mock retry recovery",
    )?;
    client
        .fail_next(ClientError::Validation("bad form".into()))
        .await;
    need(
        matches!(
            client.list_workspaces().await,
            Err(ClientError::Validation(_))
        ),
        "mock validation injection",
    )?;
    need(
        client.list_workspaces().await.is_ok(),
        "mock retry after injected failure",
    )
}

fn verify_local_harness_catalog() -> Result<(), String> {
    let harnesses = local_harnesses();
    need(
        harnesses
            .iter()
            .map(|harness| harness.id.as_str())
            .collect::<Vec<_>>()
            == ["pi", "codex", "claude_code"],
        "local harness catalog contains an unexpected harness",
    )?;
    need(
        harnesses
            .get(2)
            .and_then(|harness| harness.executable_path.as_deref())
            == Some("/usr/local/bin/dcc"),
        "claude_code local CLI path is not configured as dcc",
    )
}

fn artifact_path(run_id: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e-artifacts/client")
        .join(run_id)
        .join("result.json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn management_e2e() {
    let run_id = format!(
        "management_{}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        std::process::id()
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        verify_local_harness_catalog()?;
        runtime_scenario().await?;
        mock_scenario().await
    })
    .await
    .map_err(|_| "30s hard timeout".to_string())
    .and_then(|x| x);
    let path = artifact_path(&run_id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let artifact = json!({"run_id":run_id,"status":if result.is_ok(){"passed"}else{"failed"},"scenario":"runtime_uds_and_mock_management_contract","error":result.as_ref().err()});
    std::fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    assert!(
        result.is_ok(),
        "{}; artifact={}",
        result.unwrap_err(),
        path.display()
    );
}
