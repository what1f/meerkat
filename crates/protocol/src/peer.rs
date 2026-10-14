use crate::wire::*;
use std::pin::Pin;
use tokio_stream::Stream;
use tonic::Status;

pub type ComputerFrameStream =
    Pin<Box<dyn Stream<Item = Result<ComputerFrame, Status>> + Send + 'static>>;
pub type ResourceChunkStream =
    Pin<Box<dyn Stream<Item = Result<ResourceChunk, Status>> + Send + 'static>>;
pub type RuntimeFrameStream =
    Pin<Box<dyn Stream<Item = Result<RuntimeFrame, Status>> + Send + 'static>>;
pub type ManagementEventStream =
    Pin<Box<dyn Stream<Item = Result<ManagementEvent, Status>> + Send + 'static>>;

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

    async fn get_execution_details(
        &self,
        _request: GetExecutionDetailsRequest,
    ) -> Result<ExecutionDetails, Status> {
        Err(Status::unimplemented("get_execution_details"))
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
