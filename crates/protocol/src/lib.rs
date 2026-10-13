#![forbid(unsafe_code)]

//! The single versioned gRPC contract shared by Runtime, Computer, and clients.
//!
//! This crate owns wire data and Unix-domain-socket transport only. Runtime and
//! Computer business behavior is supplied through [`RuntimePeer`].

use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tokio_stream::{Stream, StreamExt};
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Request, Response, Status, Streaming};
use tower::service_fn;

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WORKSPACE_IMAGE_BYTES: usize = 3 * 1024 * 1024;
pub const PROVIDER_CAPABILITY_TASK_SYSTEM: &str = "task_system";

pub mod wire {
    tonic::include_proto!("lankide.protocol.v1");
}

pub use wire::runtime_protocol_client::RuntimeProtocolClient;
pub use wire::runtime_protocol_server::{RuntimeProtocol, RuntimeProtocolServer};
pub use wire::*;

pub type ComputerFrameStream =
    Pin<Box<dyn Stream<Item = Result<ComputerFrame, Status>> + Send + 'static>>;
pub type ResourceChunkStream =
    Pin<Box<dyn Stream<Item = Result<ResourceChunk, Status>> + Send + 'static>>;
pub type RuntimeFrameStream =
    Pin<Box<dyn Stream<Item = Result<RuntimeFrame, Status>> + Send + 'static>>;
pub type ManagementEventStream =
    Pin<Box<dyn Stream<Item = Result<ManagementEvent, Status>> + Send + 'static>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportConfig {
    pub protocol_version: u32,
    pub max_message_bytes: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

#[derive(Debug)]
pub enum TransportError {
    Io(std::io::Error),
    Tonic(tonic::transport::Error),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "uds_io: {error}"),
            Self::Tonic(error) => write!(formatter, "grpc_transport: {error}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<tonic::transport::Error> for TransportError {
    fn from(value: tonic::transport::Error) -> Self {
        Self::Tonic(value)
    }
}

/// A Runtime-side peer behind the protocol validation and UDS/gRPC boundary.
#[tonic::async_trait]
pub trait RuntimePeer: Send + Sync + 'static {
    async fn connect_computer(
        &self,
        hello: ComputerHello,
        frames: ComputerFrameStream,
    ) -> Result<RuntimeFrameStream, Status>;

    async fn upload_resource(&self, chunks: ResourceChunkStream) -> Result<ResourceRef, Status>;

    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status>;

    async fn list_workspaces(
        &self,
        _request: ListWorkspacesRequest,
    ) -> Result<ListWorkspacesResponse, Status> {
        Err(Status::unimplemented("list_workspaces"))
    }

    async fn get_workspace_image(
        &self,
        _request: WorkspaceImageRequest,
    ) -> Result<WorkspaceImageResponse, Status> {
        Err(Status::unimplemented("get_workspace_image"))
    }

    async fn upload_workspace_image(
        &self,
        _request: UploadWorkspaceImageRequest,
    ) -> Result<UploadWorkspaceImageResponse, Status> {
        Err(Status::unimplemented("upload_workspace_image"))
    }

    async fn get_workspace(&self, _request: GetWorkspaceRequest) -> Result<Workspace, Status> {
        Err(Status::unimplemented("get_workspace"))
    }

    async fn create_workspace(
        &self,
        _request: CreateWorkspaceRequest,
    ) -> Result<Workspace, Status> {
        Err(Status::unimplemented("create_workspace"))
    }

    async fn update_workspace(
        &self,
        _request: UpdateWorkspaceRequest,
    ) -> Result<Workspace, Status> {
        Err(Status::unimplemented("update_workspace"))
    }

    async fn set_workspace_state(
        &self,
        _request: SetWorkspaceStateRequest,
    ) -> Result<Workspace, Status> {
        Err(Status::unimplemented("set_workspace_state"))
    }

    async fn list_agents(&self, _request: ListAgentsRequest) -> Result<ListAgentsResponse, Status> {
        Err(Status::unimplemented("list_agents"))
    }

    async fn get_agent(&self, _request: GetAgentRequest) -> Result<Agent, Status> {
        Err(Status::unimplemented("get_agent"))
    }

    async fn get_agent_avatar(
        &self,
        _request: GetAgentAvatarRequest,
    ) -> Result<GetAgentAvatarResponse, Status> {
        Err(Status::unimplemented("get_agent_avatar"))
    }

    async fn create_agent(&self, _request: CreateAgentRequest) -> Result<Agent, Status> {
        Err(Status::unimplemented("create_agent"))
    }

    async fn update_agent(&self, _request: UpdateAgentRequest) -> Result<Agent, Status> {
        Err(Status::unimplemented("update_agent"))
    }
    async fn delete_agent(
        &self,
        _request: DeleteAgentRequest,
    ) -> Result<DeleteAgentResponse, Status> {
        Err(Status::unimplemented("delete_agent"))
    }

    async fn get_provider_onboarding(
        &self,
        _request: GetProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingState, Status> {
        Err(Status::unimplemented("get_provider_onboarding"))
    }

    async fn advance_provider_onboarding(
        &self,
        _request: AdvanceProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingState, Status> {
        Err(Status::unimplemented("advance_provider_onboarding"))
    }

    async fn get_agent_onboarding(
        &self,
        _request: GetAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, Status> {
        Err(Status::unimplemented("get_agent_onboarding"))
    }

    async fn advance_agent_onboarding(
        &self,
        _request: AdvanceAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, Status> {
        Err(Status::unimplemented("advance_agent_onboarding"))
    }

    async fn get_provider_catalog(
        &self,
        _request: CatalogRequest,
    ) -> Result<ProviderCatalog, Status> {
        Err(Status::unimplemented("get_provider_catalog"))
    }

    async fn get_computer_catalog(
        &self,
        _request: CatalogRequest,
    ) -> Result<ComputerCatalog, Status> {
        Err(Status::unimplemented("get_computer_catalog"))
    }

    async fn get_harness_catalog(
        &self,
        _request: CatalogRequest,
    ) -> Result<HarnessCatalog, Status> {
        Err(Status::unimplemented("get_harness_catalog"))
    }

    async fn get_model_catalog(
        &self,
        _request: ModelCatalogRequest,
    ) -> Result<ModelCatalog, Status> {
        Err(Status::unimplemented("get_model_catalog"))
    }

    async fn get_reaction_catalog(
        &self,
        _request: CatalogRequest,
    ) -> Result<ReactionCatalog, Status> {
        Err(Status::unimplemented("get_reaction_catalog"))
    }

    async fn subscribe_management_events(
        &self,
        _request: ManagementSubscription,
    ) -> Result<ManagementEventStream, Status> {
        Err(Status::unimplemented("subscribe_management_events"))
    }

    async fn invoke_provider_interface(
        &self,
        _request: InvokeProviderInterfaceRequest,
    ) -> Result<InvokeProviderInterfaceResponse, Status> {
        Err(Status::unimplemented("invoke_provider_interface"))
    }

    async fn list_provider_interfaces(
        &self,
        _request: ListProviderInterfacesRequest,
    ) -> Result<ListProviderInterfacesResponse, Status> {
        Err(Status::unimplemented("list_provider_interfaces"))
    }

    async fn list_capabilities(&self, _context: AgentRunContext) -> Result<CapabilityList, Status> {
        Err(Status::unimplemented("list_capabilities"))
    }

    async fn get_self_profile(&self, _context: AgentRunContext) -> Result<AgentProfile, Status> {
        Err(Status::unimplemented("get_self_profile"))
    }

    async fn update_self_profile(
        &self,
        _request: UpdateSelfProfileRequest,
    ) -> Result<AgentProfile, Status> {
        Err(Status::unimplemented("update_self_profile"))
    }

    async fn invoke_capability(
        &self,
        _request: CapabilityInvokeRequest,
    ) -> Result<CapabilityInvokeResult, Status> {
        Err(Status::unimplemented("invoke_capability"))
    }

    async fn send_message(
        &self,
        _request: SendMessageRequest,
    ) -> Result<SendMessageResult, Status> {
        Err(Status::unimplemented("send_message"))
    }

    async fn get_content(&self, _request: ContentRequest) -> Result<ContentResult, Status> {
        Err(Status::unimplemented("get_content"))
    }
}

/// Generated-service adapter which rejects malformed wire input before it can
/// reach Runtime business code.
#[derive(Clone)]
pub struct ValidatedRuntimeService<P> {
    peer: P,
    config: TransportConfig,
}

impl<P> ValidatedRuntimeService<P> {
    pub const fn new(peer: P, config: TransportConfig) -> Self {
        Self { peer, config }
    }
}

#[tonic::async_trait]
impl<P> RuntimeProtocol for ValidatedRuntimeService<P>
where
    P: RuntimePeer,
{
    type ConnectComputerStream = RuntimeFrameStream;
    type DownloadResourceStream = ResourceChunkStream;
    type SubscribeManagementEventsStream = ManagementEventStream;

    async fn connect_computer(
        &self,
        request: Request<Streaming<ComputerFrame>>,
    ) -> Result<Response<Self::ConnectComputerStream>, Status> {
        let mut inbound = request.into_inner();
        let first = inbound
            .next()
            .await
            .ok_or_else(|| Status::invalid_argument("missing_computer_hello"))??;
        let hello = match first.frame {
            Some(computer_frame::Frame::Hello(hello)) => hello,
            Some(_) => {
                return Err(Status::invalid_argument(
                    "first_frame_must_be_computer_hello",
                ));
            }
            None => return Err(Status::invalid_argument("unknown_computer_frame")),
        };
        validate_computer_hello(&hello, self.config.protocol_version)?;

        let (sender, receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            let mut validator = ComputerFrameValidator::default();
            while let Some(next) = inbound.next().await {
                let validated = next.and_then(|frame| {
                    validator.validate(&frame)?;
                    Ok(frame)
                });
                let failed = validated.is_err();
                if sender.send(validated).await.is_err() || failed {
                    break;
                }
            }
        });
        let mut outgoing = self
            .peer
            .connect_computer(hello, Box::pin(ReceiverStream::new(receiver)))
            .await?;
        let (sender, receiver) = mpsc::channel(16);
        let expected_version = self.config.protocol_version;
        tokio::spawn(async move {
            let mut first = true;
            while let Some(next) = outgoing.next().await {
                let validated = next.and_then(|frame| {
                    validate_runtime_frame(&frame)?;
                    match (&frame.frame, first) {
                        (Some(runtime_frame::Frame::Hello(hello)), true)
                            if hello.protocol_version == expected_version => {}
                        (Some(runtime_frame::Frame::Hello(_)), true) => {
                            return Err(Status::failed_precondition(
                                "runtime_protocol_version_mismatch",
                            ));
                        }
                        (Some(runtime_frame::Frame::Hello(_)), false) => {
                            return Err(Status::invalid_argument("duplicate_runtime_hello"));
                        }
                        (_, true) => {
                            return Err(Status::invalid_argument(
                                "first_frame_must_be_runtime_hello",
                            ));
                        }
                        (_, false) => {}
                    }
                    first = false;
                    Ok(frame)
                });
                let failed = validated.is_err();
                if sender.send(validated).await.is_err() || failed {
                    return;
                }
            }
            if first {
                let _ = sender
                    .send(Err(Status::invalid_argument("missing_runtime_hello")))
                    .await;
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }

    async fn upload_resource(
        &self,
        request: Request<Streaming<ResourceChunk>>,
    ) -> Result<Response<ResourceRef>, Status> {
        let mut inbound = request.into_inner();
        let (sender, receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            let mut validator = ResourceUploadValidator::default();
            while let Some(next) = inbound.next().await {
                let validated = next.and_then(|chunk| {
                    validator.validate(&chunk)?;
                    Ok(chunk)
                });
                let failed = validated.is_err();
                if sender.send(validated).await.is_err() || failed {
                    break;
                }
            }
            if !validator.complete() {
                let _ = sender
                    .send(Err(Status::invalid_argument("resource_upload_missing_eof")))
                    .await;
            }
        });
        let resource = self
            .peer
            .upload_resource(Box::pin(ReceiverStream::new(receiver)))
            .await?;
        validate_resource_ref(&resource)?;
        Ok(Response::new(resource))
    }

    async fn download_resource(
        &self,
        request: Request<ResourceRequest>,
    ) -> Result<Response<Self::DownloadResourceStream>, Status> {
        validate_resource_request(request.get_ref())?;
        let request = request.into_inner();
        let mut chunks = self.peer.download_resource(request.clone()).await?;
        let (sender, receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            let mut validator =
                ResourceUploadValidator::expecting(request.transfer_id, request.resource_id);
            while let Some(next) = chunks.next().await {
                let validated = next.and_then(|chunk| {
                    validator.validate(&chunk)?;
                    Ok(chunk)
                });
                let failed = validated.is_err();
                if sender.send(validated).await.is_err() || failed {
                    return;
                }
            }
            if !validator.complete() {
                let _ = sender
                    .send(Err(Status::data_loss("resource_download_missing_eof")))
                    .await;
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }

    async fn list_workspaces(
        &self,
        request: Request<ListWorkspacesRequest>,
    ) -> Result<Response<ListWorkspacesResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        Ok(Response::new(
            self.peer.list_workspaces(request.into_inner()).await?,
        ))
    }

    async fn get_workspace_image(
        &self,
        request: Request<WorkspaceImageRequest>,
    ) -> Result<Response<WorkspaceImageResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        if request.get_ref().resource_id <= 0 {
            return Err(Status::invalid_argument("invalid image resource"));
        }
        let response = self.peer.get_workspace_image(request.into_inner()).await?;
        if response.bytes.len() > MAX_WORKSPACE_IMAGE_BYTES {
            return Err(Status::resource_exhausted("image exceeds size limit"));
        }
        Ok(Response::new(response))
    }

    async fn upload_workspace_image(
        &self,
        request: Request<UploadWorkspaceImageRequest>,
    ) -> Result<Response<UploadWorkspaceImageResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        require_text(&request.get_ref().file_name, "file_name")?;
        if request.get_ref().bytes.is_empty() {
            return Err(Status::invalid_argument("image is empty"));
        }
        if request.get_ref().bytes.len() > MAX_WORKSPACE_IMAGE_BYTES {
            return Err(Status::resource_exhausted("image exceeds size limit"));
        }
        if !matches!(
            request.get_ref().media_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err(Status::invalid_argument("unsupported image format"));
        }
        let response = self
            .peer
            .upload_workspace_image(request.into_inner())
            .await?;
        validate_resource_ref(
            response
                .resource
                .as_ref()
                .ok_or_else(|| Status::internal("image metadata missing"))?,
        )?;
        Ok(Response::new(response))
    }

    async fn get_workspace(
        &self,
        request: Request<GetWorkspaceRequest>,
    ) -> Result<Response<Workspace>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.get_workspace(request.into_inner()).await?,
        ))
    }

    async fn create_workspace(
        &self,
        request: Request<CreateWorkspaceRequest>,
    ) -> Result<Response<Workspace>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        lankide_core::WorkspaceId::new(input.workspace_id.clone())
            .map_err(|error| Status::invalid_argument(error.code()))?;
        require_text(&input.name, "workspace_name")?;
        require_text(&input.provider_id, "provider_id")?;
        validate_json(&input.provider_config_json, "provider_config_json")?;
        Ok(Response::new(
            self.peer.create_workspace(request.into_inner()).await?,
        ))
    }

    async fn update_workspace(
        &self,
        request: Request<UpdateWorkspaceRequest>,
    ) -> Result<Response<Workspace>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_workspace_ref(input.workspace.as_ref())?;
        if let Some(config) = &input.provider_config_json {
            validate_json(config, "provider_config_json")?;
        }
        Ok(Response::new(
            self.peer.update_workspace(request.into_inner()).await?,
        ))
    }

    async fn set_workspace_state(
        &self,
        request: Request<SetWorkspaceStateRequest>,
    ) -> Result<Response<Workspace>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_workspace_ref(input.workspace.as_ref())?;
        match DesiredState::try_from(input.desired_state) {
            Ok(DesiredState::Running | DesiredState::Paused) => {}
            _ => return Err(Status::invalid_argument("invalid_desired_state")),
        }
        Ok(Response::new(
            self.peer.set_workspace_state(request.into_inner()).await?,
        ))
    }

    async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.list_agents(request.into_inner()).await?,
        ))
    }

    async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<Agent>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer.get_agent(request.into_inner()).await?,
        ))
    }

    async fn get_agent_avatar(
        &self,
        request: Request<GetAgentAvatarRequest>,
    ) -> Result<Response<GetAgentAvatarResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        let response = self.peer.get_agent_avatar(request.into_inner()).await?;
        validate_request_id(&response.request_id)?;
        validate_agent_ref(response.agent.as_ref())?;
        validate_avatar_file(
            response
                .avatar
                .as_ref()
                .ok_or_else(|| Status::internal("avatar file missing"))?,
        )?;
        Ok(Response::new(response))
    }

    async fn create_agent(
        &self,
        request: Request<CreateAgentRequest>,
    ) -> Result<Response<Agent>, Status> {
        validate_agent_write(
            &request.get_ref().request_id,
            request.get_ref().agent.as_ref(),
            &request.get_ref().name,
            &request.get_ref().harness_id,
            &request.get_ref().computer_id,
            request.get_ref().attention.as_ref(),
        )?;
        Ok(Response::new(
            self.peer.create_agent(request.into_inner()).await?,
        ))
    }

    async fn update_agent(
        &self,
        request: Request<UpdateAgentRequest>,
    ) -> Result<Response<Agent>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        if let Some(attention) = request.get_ref().attention.as_ref() {
            validate_attention(attention)?;
        }
        if let Some(avatar) = request.get_ref().avatar.as_ref() {
            validate_avatar_upload(avatar)?;
        }
        Ok(Response::new(
            self.peer.update_agent(request.into_inner()).await?,
        ))
    }

    async fn delete_agent(
        &self,
        request: Request<DeleteAgentRequest>,
    ) -> Result<Response<DeleteAgentResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer.delete_agent(request.into_inner()).await?,
        ))
    }

    async fn get_provider_onboarding(
        &self,
        request: Request<GetProviderOnboardingRequest>,
    ) -> Result<Response<ProviderOnboardingState>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer
                .get_provider_onboarding(request.into_inner())
                .await?,
        ))
    }

    async fn advance_provider_onboarding(
        &self,
        request: Request<AdvanceProviderOnboardingRequest>,
    ) -> Result<Response<ProviderOnboardingState>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_workspace_ref(input.workspace.as_ref())?;
        require_text(&input.action, "onboarding_action")?;
        validate_json(&input.field_values_json, "field_values_json")?;
        Ok(Response::new(
            self.peer
                .advance_provider_onboarding(request.into_inner())
                .await?,
        ))
    }

    async fn get_agent_onboarding(
        &self,
        request: Request<GetAgentOnboardingRequest>,
    ) -> Result<Response<AgentOnboardingState>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer.get_agent_onboarding(request.into_inner()).await?,
        ))
    }

    async fn advance_agent_onboarding(
        &self,
        request: Request<AdvanceAgentOnboardingRequest>,
    ) -> Result<Response<AgentOnboardingState>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_agent_ref(input.agent.as_ref())?;
        require_text(&input.action, "onboarding_action")?;
        validate_json(&input.field_values_json, "field_values_json")?;
        Ok(Response::new(
            self.peer
                .advance_agent_onboarding(request.into_inner())
                .await?,
        ))
    }

    async fn get_provider_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<ProviderCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_provider_catalog(request.into_inner()).await?,
        ))
    }

    async fn get_computer_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<ComputerCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_computer_catalog(request.into_inner()).await?,
        ))
    }

    async fn get_harness_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<HarnessCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_harness_catalog(request.into_inner()).await?,
        ))
    }

    async fn get_model_catalog(
        &self,
        request: Request<ModelCatalogRequest>,
    ) -> Result<Response<ModelCatalog>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        if input.workspace.is_some() {
            validate_workspace_ref(input.workspace.as_ref())?;
        }
        require_text(&input.computer_id, "computer_id")?;
        require_text(&input.harness_id, "harness_id")?;
        Ok(Response::new(
            self.peer.get_model_catalog(request.into_inner()).await?,
        ))
    }

    async fn get_reaction_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<ReactionCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_reaction_catalog(request.into_inner()).await?,
        ))
    }

    async fn subscribe_management_events(
        &self,
        request: Request<ManagementSubscription>,
    ) -> Result<Response<Self::SubscribeManagementEventsStream>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        if input.workspace.is_some() {
            validate_workspace_ref(input.workspace.as_ref())?;
        }
        let after_sequence = input.after_sequence;
        let mut events = self
            .peer
            .subscribe_management_events(request.into_inner())
            .await?;
        let (sender, receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            let mut last_sequence = after_sequence;
            while let Some(next) = events.next().await {
                let validated = next.and_then(|event| {
                    if event.sequence <= last_sequence {
                        return Err(Status::out_of_range("management_sequence_regression"));
                    }
                    ManagementEventKind::try_from(event.kind)
                        .map_err(|_| Status::invalid_argument("unknown_management_event_kind"))?;
                    validate_workspace_ref(event.workspace.as_ref())?;
                    if event.agent.is_some() {
                        validate_agent_ref(event.agent.as_ref())?;
                    }
                    last_sequence = event.sequence;
                    Ok(event)
                });
                let failed = validated.is_err();
                if sender.send(validated).await.is_err() || failed {
                    return;
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }

    async fn list_capabilities(
        &self,
        request: Request<AgentRunContext>,
    ) -> Result<Response<CapabilityList>, Status> {
        validate_agent_run_context(Some(request.get_ref()))?;
        Ok(Response::new(
            self.peer.list_capabilities(request.into_inner()).await?,
        ))
    }

    async fn get_self_profile(
        &self,
        request: Request<AgentRunContext>,
    ) -> Result<Response<AgentProfile>, Status> {
        validate_agent_run_context(Some(request.get_ref()))?;
        let response = self.peer.get_self_profile(request.into_inner()).await?;
        validate_agent_profile(&response)?;
        Ok(Response::new(response))
    }

    async fn update_self_profile(
        &self,
        request: Request<UpdateSelfProfileRequest>,
    ) -> Result<Response<AgentProfile>, Status> {
        let input = request.get_ref();
        validate_agent_run_context(input.context.as_ref())?;
        if input.name.is_none()
            && input.role.is_none()
            && input.bio.is_none()
            && input.avatar.is_none()
        {
            return Err(Status::invalid_argument("profile update is empty"));
        }
        if input
            .name
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(Status::invalid_argument("agent name is empty"));
        }
        if let Some(avatar) = input.avatar.as_ref() {
            validate_avatar_upload(avatar)?;
        }
        let response = self.peer.update_self_profile(request.into_inner()).await?;
        validate_agent_profile(&response)?;
        Ok(Response::new(response))
    }

    async fn invoke_provider_interface(
        &self,
        request: Request<InvokeProviderInterfaceRequest>,
    ) -> Result<Response<InvokeProviderInterfaceResponse>, Status> {
        let input = request.get_ref();
        require_text(&input.request_id, "request_id")?;
        validate_workspace_ref(input.workspace.as_ref())?;
        require_text(&input.interface_id, "interface_id")?;
        validate_json(&input.input_json, "input_json")?;
        let response = self
            .peer
            .invoke_provider_interface(request.into_inner())
            .await?;
        require_text(&response.request_id, "response.request_id")?;
        validate_workspace_ref(response.workspace.as_ref())?;
        require_text(&response.interface_id, "response.interface_id")?;
        validate_json(&response.output_json, "response.output_json")?;
        Ok(Response::new(response))
    }

    async fn list_provider_interfaces(
        &self,
        request: Request<ListProviderInterfacesRequest>,
    ) -> Result<Response<ListProviderInterfacesResponse>, Status> {
        let input = request.get_ref();
        require_text(&input.request_id, "request_id")?;
        validate_workspace_ref(input.workspace.as_ref())?;
        let response = self
            .peer
            .list_provider_interfaces(request.into_inner())
            .await?;
        require_text(&response.request_id, "response.request_id")?;
        validate_workspace_ref(response.workspace.as_ref())?;
        let mut previous = None;
        for interface in &response.interfaces {
            require_text(&interface.interface_id, "interface_id")?;
            validate_json(&interface.input_schema_json, "input_schema_json")?;
            validate_json(&interface.output_schema_json, "output_schema_json")?;
            if previous.is_some_and(|value: &str| value >= interface.interface_id.as_str()) {
                return Err(Status::failed_precondition(
                    "provider_interfaces_not_sorted",
                ));
            }
            previous = Some(interface.interface_id.as_str());
        }
        Ok(Response::new(response))
    }

    async fn invoke_capability(
        &self,
        request: Request<CapabilityInvokeRequest>,
    ) -> Result<Response<CapabilityInvokeResult>, Status> {
        let input = request.get_ref();
        validate_agent_run_context(input.context.as_ref())?;
        require_text(&input.capability_name, "capability_name")?;
        require_text(&input.invocation_id, "invocation_id")?;
        validate_json(&input.arguments_json, "arguments_json")?;
        Ok(Response::new(
            self.peer.invoke_capability(request.into_inner()).await?,
        ))
    }

    async fn send_message(
        &self,
        request: Request<SendMessageRequest>,
    ) -> Result<Response<SendMessageResult>, Status> {
        let input = request.get_ref();
        validate_agent_run_context(input.context.as_ref())?;
        require_text(&input.channel_id, "channel_id")?;
        require_text(&input.chat_id, "chat_id")?;
        validate_send_message_content(&input.content_markdown, &input.resources)?;
        require_text(&input.idempotency_key, "idempotency_key")?;
        Ok(Response::new(
            self.peer.send_message(request.into_inner()).await?,
        ))
    }

    async fn get_content(
        &self,
        request: Request<ContentRequest>,
    ) -> Result<Response<ContentResult>, Status> {
        let input = request.get_ref();
        validate_agent_run_context(input.context.as_ref())?;
        ContentKind::try_from(input.kind)
            .map_err(|_| Status::invalid_argument("unknown_content_kind"))?;
        require_text(&input.content_id, "content_id")?;
        Ok(Response::new(
            self.peer.get_content(request.into_inner()).await?,
        ))
    }
}

/// Serve the validated Runtime protocol over one real Unix domain socket.
pub async fn serve_uds<P, F>(
    socket_path: impl AsRef<Path>,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), TransportError>
where
    P: RuntimePeer,
    F: Future<Output = ()> + Send + 'static,
{
    let socket_path = socket_path.as_ref().to_path_buf();
    prepare_socket_parent(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    let service = RuntimeProtocolServer::new(ValidatedRuntimeService::new(peer, config))
        .max_decoding_message_size(config.max_message_bytes)
        .max_encoding_message_size(config.max_message_bytes);
    let result = Server::builder()
        .add_service(service)
        .serve_with_incoming_shutdown(UnixListenerStream::new(listener), shutdown)
        .await;
    let _ = std::fs::remove_file(&socket_path);
    result.map_err(TransportError::from)
}

/// Connect a generated Runtime client to a Unix domain socket.
pub async fn connect_uds(
    socket_path: impl AsRef<Path>,
    max_message_bytes: usize,
) -> Result<RuntimeProtocolClient<Channel>, TransportError> {
    let path = socket_path.as_ref().to_path_buf();
    let channel = Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(service_fn(move |_| {
            let path = path.clone();
            async move { UnixStream::connect(path).await.map(TokioIo::new) }
        }))
        .await?;
    Ok(RuntimeProtocolClient::new(channel)
        .max_decoding_message_size(max_message_bytes)
        .max_encoding_message_size(max_message_bytes))
}

/// A reconnecting channel that can be created before the local Runtime starts.
/// Construct it inside a Tokio runtime, just like tonic's other lazy channels.
pub fn connect_uds_lazy(
    socket_path: impl AsRef<Path>,
    max_message_bytes: usize,
) -> RuntimeProtocolClient<Channel> {
    let path = socket_path.as_ref().to_path_buf();
    let channel = Endpoint::from_static("http://[::]:50051").connect_with_connector_lazy(
        service_fn(move |_| {
            let path = path.clone();
            async move { UnixStream::connect(path).await.map(TokioIo::new) }
        }),
    );
    RuntimeProtocolClient::new(channel)
        .max_decoding_message_size(max_message_bytes)
        .max_encoding_message_size(max_message_bytes)
}

fn prepare_socket_parent(socket_path: &Path) -> Result<(), std::io::Error> {
    let parent = socket_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "socket_path_has_no_parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn validate_computer_hello(hello: &ComputerHello, expected_version: u32) -> Result<(), Status> {
    if hello.protocol_version != expected_version {
        return Err(Status::failed_precondition(format!(
            "protocol_version_mismatch: expected {expected_version}, received {}",
            hello.protocol_version
        )));
    }
    require_text(&hello.computer_id, "computer_id")?;
    require_text(&hello.instance_id, "instance_id")?;
    require_text(&hello.computer_version, "computer_version")?;
    require_text(&hello.os, "os")?;
    require_text(&hello.arch, "arch")?;
    for harness in &hello.harnesses {
        require_text(&harness.harness_id, "harness_id")?;
        require_text(&harness.display_name, "harness_display_name")?;
        HarnessAvailability::try_from(harness.availability)
            .map_err(|_| Status::invalid_argument("unknown_harness_availability"))?;
    }
    Ok(())
}

pub fn validate_runtime_frame(frame: &RuntimeFrame) -> Result<(), Status> {
    match frame.frame.as_ref() {
        Some(runtime_frame::Frame::Hello(hello)) => {
            if hello.protocol_version == 0 {
                Err(Status::invalid_argument("invalid_protocol_version"))
            } else if hello.max_message_bytes == 0 {
                Err(Status::invalid_argument("invalid_max_message_bytes"))
            } else {
                require_text(&hello.runtime_instance_id, "runtime_instance_id")
            }
        }
        Some(runtime_frame::Frame::AssignExecution(assign)) => validate_assign_execution(assign),
        Some(runtime_frame::Frame::ExecutionEventAck(ack)) => {
            validate_execution_ref(ack.execution.as_ref())
        }
        Some(runtime_frame::Frame::ExecutionCommitted(committed)) => {
            validate_execution_ref(committed.execution.as_ref())
        }
        Some(runtime_frame::Frame::DeleteAgentFiles(request)) => {
            validate_request_id(&request.request_id)?;
            validate_agent_ref(request.agent.as_ref())
        }
        Some(_) => Ok(()),
        None => Err(Status::invalid_argument("unknown_runtime_frame")),
    }
}

#[derive(Default)]
struct ComputerFrameValidator {
    last_sequences: HashMap<(i64, u32), u64>,
}

impl ComputerFrameValidator {
    fn validate(&mut self, frame: &ComputerFrame) -> Result<(), Status> {
        match frame.frame.as_ref() {
            Some(computer_frame::Frame::Hello(_)) => {
                Err(Status::invalid_argument("duplicate_computer_hello"))
            }
            Some(computer_frame::Frame::ExecutionEvent(event)) => {
                let execution = event.execution.as_ref();
                validate_execution_ref(execution)?;
                if event.sequence == 0 {
                    return Err(Status::invalid_argument("event_sequence_must_be_positive"));
                }
                ExecutionEventKind::try_from(event.kind)
                    .map_err(|_| Status::invalid_argument("unknown_execution_event_kind"))?;
                let execution = execution.expect("validated execution ref");
                let key = (execution.execution_id, execution.attempt);
                if let Some(previous) = self.last_sequences.get(&key)
                    && event.sequence < *previous
                {
                    return Err(Status::out_of_range("event_sequence_regression"));
                }
                self.last_sequences.insert(key, event.sequence);
                Ok(())
            }
            Some(computer_frame::Frame::AssignmentResult(result)) => {
                validate_execution_ref(result.execution.as_ref())
            }
            Some(computer_frame::Frame::DeliveryResult(result)) => {
                validate_execution_ref(result.execution.as_ref())
            }
            Some(computer_frame::Frame::ToolCall(call)) => {
                validate_execution_ref(call.execution.as_ref())
            }
            Some(computer_frame::Frame::DeleteAgentFilesResult(result)) => {
                validate_request_id(&result.request_id)
            }
            Some(_) => Ok(()),
            None => Err(Status::invalid_argument("unknown_computer_frame")),
        }
    }
}

#[derive(Default)]
struct ResourceUploadValidator {
    transfer_id: Option<String>,
    resource_id: Option<i64>,
    descriptor: Option<ResourceDescriptor>,
    next_offset: u64,
    digest: Sha256,
    saw_eof: bool,
}

impl ResourceUploadValidator {
    fn expecting(transfer_id: String, resource_id: i64) -> Self {
        Self {
            transfer_id: Some(transfer_id),
            resource_id: Some(resource_id),
            ..Self::default()
        }
    }

    fn validate(&mut self, chunk: &ResourceChunk) -> Result<(), Status> {
        if self.saw_eof {
            return Err(Status::invalid_argument("resource_chunk_after_eof"));
        }
        require_text(&chunk.transfer_id, "transfer_id")?;
        if chunk.offset != self.next_offset {
            return Err(Status::invalid_argument(format!(
                "resource_offset_mismatch: expected {}, received {}",
                self.next_offset, chunk.offset
            )));
        }
        if let Some(transfer_id) = &self.transfer_id
            && transfer_id != &chunk.transfer_id
        {
            return Err(Status::invalid_argument("transfer_id_changed"));
        }
        if let Some(resource_id) = self.resource_id
            && resource_id != chunk.resource_id
        {
            return Err(Status::invalid_argument("resource_id_changed"));
        }
        match (&self.descriptor, &chunk.descriptor) {
            (None, Some(descriptor)) => {
                validate_resource_descriptor(descriptor)?;
                validate_agent_run_context(descriptor.context.as_ref())?;
                self.transfer_id.get_or_insert(chunk.transfer_id.clone());
                self.resource_id.get_or_insert(chunk.resource_id);
                self.descriptor = Some(descriptor.clone());
            }
            (None, None) => return Err(Status::invalid_argument("first_chunk_missing_descriptor")),
            (Some(_), Some(_)) => {
                return Err(Status::invalid_argument(
                    "descriptor_only_allowed_on_first_chunk",
                ));
            }
            (Some(_), None) => {}
        }
        self.digest.update(&chunk.bytes);
        self.next_offset = self
            .next_offset
            .checked_add(chunk.bytes.len() as u64)
            .ok_or_else(|| Status::out_of_range("resource_size_overflow"))?;
        if chunk.eof {
            let descriptor = self.descriptor.as_ref().expect("first chunk validated");
            if self.next_offset != descriptor.size_bytes {
                return Err(Status::invalid_argument("resource_size_mismatch"));
            }
            let actual = lowercase_hex(&self.digest.clone().finalize());
            if actual != descriptor.sha256 {
                return Err(Status::data_loss("resource_sha256_mismatch"));
            }
            self.saw_eof = true;
        }
        Ok(())
    }

    const fn complete(&self) -> bool {
        self.saw_eof
    }
}

fn validate_assign_execution(assign: &AssignExecution) -> Result<(), Status> {
    validate_execution_ref(assign.execution.as_ref())?;
    let workspace = assign
        .workspace
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("missing_workspace_ref"))?;
    lankide_core::WorkspaceId::new(workspace.workspace_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    let agent = assign
        .agent
        .as_ref()
        .and_then(|profile| profile.agent.as_ref())
        .ok_or_else(|| Status::invalid_argument("missing_agent_ref"))?;
    if agent.workspace_id != workspace.workspace_id {
        return Err(Status::invalid_argument("agent_workspace_mismatch"));
    }
    lankide_core::AgentId::new(agent.agent_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    if assign.harness.is_none() {
        return Err(Status::invalid_argument("missing_harness_config"));
    }
    if assign.session.is_none() {
        return Err(Status::invalid_argument("missing_session_directive"));
    }
    if assign.initial_delivery.is_none() {
        return Err(Status::invalid_argument("missing_initial_delivery"));
    }
    let mut tool_names = BTreeSet::new();
    for tool in &assign.tools {
        if !lankide_core::is_canonical_tool_name(&tool.name) {
            return Err(Status::invalid_argument("invalid_tool_name"));
        }
        if !tool_names.insert(&tool.name) {
            return Err(Status::invalid_argument("duplicate_tool_name"));
        }
        require_text(&tool.description, "tool_description")?;
        validate_json(&tool.input_schema_json, "tool_input_schema_json")?;
        validate_json(&tool.output_schema_json, "tool_output_schema_json")?;
    }
    require_text(&assign.execution_token, "execution_token")
}

fn validate_execution_ref(execution: Option<&ExecutionRef>) -> Result<(), Status> {
    let execution = execution.ok_or_else(|| Status::invalid_argument("missing_execution_ref"))?;
    lankide_core::ExecutionId::new(execution.execution_id)
        .map_err(|error| Status::invalid_argument(error.code()))?;
    if execution.attempt == 0 {
        return Err(Status::invalid_argument("attempt_must_be_positive"));
    }
    Ok(())
}

fn validate_resource_ref(resource: &ResourceRef) -> Result<(), Status> {
    lankide_core::ResourceId::new(resource.resource_id)
        .map_err(|error| Status::invalid_argument(error.code()))?;
    validate_resource_descriptor(&ResourceDescriptor {
        file_name: resource.file_name.clone(),
        media_type: resource.media_type.clone(),
        size_bytes: resource.size_bytes,
        sha256: resource.sha256.clone(),
        context: None,
    })
}

fn validate_resource_descriptor(descriptor: &ResourceDescriptor) -> Result<(), Status> {
    require_text(&descriptor.file_name, "file_name")?;
    require_text(&descriptor.media_type, "media_type")?;
    if descriptor.sha256.len() != 64
        || !descriptor
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Status::invalid_argument("invalid_resource_sha256"));
    }
    Ok(())
}

fn validate_resource_request(request: &ResourceRequest) -> Result<(), Status> {
    lankide_core::ResourceId::new(request.resource_id)
        .map_err(|error| Status::invalid_argument(error.code()))?;
    require_text(&request.transfer_id, "transfer_id")?;
    validate_agent_run_context(request.context.as_ref())
}

fn validate_request_id(request_id: &str) -> Result<(), Status> {
    require_text(request_id, "request_id")
}

fn validate_workspace_ref(workspace: Option<&WorkspaceRef>) -> Result<(), Status> {
    let workspace = workspace.ok_or_else(|| Status::invalid_argument("missing_workspace_ref"))?;
    lankide_core::WorkspaceId::new(workspace.workspace_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    Ok(())
}

fn validate_agent_ref(agent: Option<&AgentRef>) -> Result<(), Status> {
    let agent = agent.ok_or_else(|| Status::invalid_argument("missing_agent_ref"))?;
    lankide_core::WorkspaceId::new(agent.workspace_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    lankide_core::AgentId::new(agent.agent_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    Ok(())
}

fn validate_avatar_upload(avatar: &AgentAvatarUpload) -> Result<(), Status> {
    require_text(&avatar.media_type, "avatar_media_type")?;
    if avatar.bytes.is_empty() {
        return Err(Status::invalid_argument("avatar is empty"));
    }
    if avatar.bytes.len() > MAX_WORKSPACE_IMAGE_BYTES {
        return Err(Status::resource_exhausted("avatar exceeds size limit"));
    }
    if !matches!(
        avatar.media_type.as_str(),
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) {
        return Err(Status::invalid_argument("unsupported avatar format"));
    }
    Ok(())
}

fn validate_avatar_file(avatar: &AgentAvatarFile) -> Result<(), Status> {
    require_text(&avatar.path, "avatar_path")?;
    require_text(&avatar.media_type, "avatar_media_type")?;
    if avatar.bytes.is_empty() || avatar.bytes.len() > MAX_WORKSPACE_IMAGE_BYTES {
        return Err(Status::data_loss("avatar bytes are invalid"));
    }
    Ok(())
}

fn validate_agent_profile(profile: &AgentProfile) -> Result<(), Status> {
    validate_agent_ref(profile.agent.as_ref())?;
    require_text(&profile.name, "agent_name")?;
    if let Some(avatar) = profile.avatar.as_ref() {
        validate_avatar_file(avatar)?;
    }
    Ok(())
}

fn validate_agent_write(
    request_id: &str,
    agent: Option<&AgentRef>,
    name: &str,
    harness_id: &str,
    computer_id: &str,
    attention: Option<&AgentAttention>,
) -> Result<(), Status> {
    validate_request_id(request_id)?;
    validate_agent_ref(agent)?;
    require_text(name, "agent_name")?;
    require_text(harness_id, "harness_id")?;
    require_text(computer_id, "computer_id")?;
    if let Some(attention) = attention {
        validate_attention(attention)?;
    }
    Ok(())
}

fn validate_attention(attention: &AgentAttention) -> Result<(), Status> {
    let mut channels = BTreeSet::new();
    for value in &attention.channels {
        lankide_core::ChannelId::new(value.channel_id.clone())
            .map_err(|error| Status::invalid_argument(error.code()))?;
        if !channels.insert(value.channel_id.as_str()) {
            return Err(Status::invalid_argument("Choose each channel only once."));
        }
        match AttentionMode::try_from(value.mode).unwrap_or(AttentionMode::Unspecified) {
            AttentionMode::Mention | AttentionMode::Always | AttentionMode::Mute
                if value.interval_minutes == 0 => {}
            AttentionMode::Schedule
                if matches!(
                    value.interval_minutes,
                    5 | 15 | 30 | 60 | 120 | 240 | 480 | 1440
                ) => {}
            AttentionMode::Unspecified => {
                return Err(Status::invalid_argument(
                    "Choose how the agent should respond in each channel.",
                ));
            }
            _ => {
                return Err(Status::invalid_argument(
                    "Choose a supported schedule interval.",
                ));
            }
        }
    }
    Ok(())
}

fn validate_catalog_request(request: &CatalogRequest) -> Result<(), Status> {
    validate_request_id(&request.request_id)?;
    if request.workspace.is_some() {
        validate_workspace_ref(request.workspace.as_ref())?;
    }
    Ok(())
}

fn validate_agent_run_context(context: Option<&AgentRunContext>) -> Result<(), Status> {
    let context = context.ok_or_else(|| Status::unauthenticated("missing_agent_run_context"))?;
    validate_execution_ref(context.execution.as_ref())?;
    if context.execution_token.trim().is_empty() {
        return Err(Status::unauthenticated("missing_execution_token"));
    }
    Ok(())
}

fn validate_send_message_content(
    content_markdown: &str,
    resources: &[ResourceRef],
) -> Result<(), Status> {
    if content_markdown.trim().is_empty() && resources.is_empty() {
        Err(Status::invalid_argument("empty_content_markdown"))
    } else {
        Ok(())
    }
}

fn validate_json(value: &str, field: &'static str) -> Result<(), Status> {
    serde_json::from_str::<serde_json::Value>(value)
        .map(|_| ())
        .map_err(|_| Status::invalid_argument(format!("invalid_{field}")))
}

fn require_text(value: &str, field: &'static str) -> Result<(), Status> {
    if value.trim().is_empty() {
        Err(Status::invalid_argument(format!("empty_{field}")))
    } else {
        Ok(())
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

pub fn socket_path(home: impl AsRef<Path>) -> PathBuf {
    home.as_ref().join("run/runtime.sock")
}
