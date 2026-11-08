use crate::Status;
use crate::wire::*;
use std::pin::Pin;
use tokio_stream::Stream;

pub type MachineFrameStream =
    Pin<Box<dyn Stream<Item = Result<MachineFrame, Status>> + Send + 'static>>;
pub type ResourceChunkStream =
    Pin<Box<dyn Stream<Item = Result<ResourceChunk, Status>> + Send + 'static>>;
pub type RuntimeFrameStream =
    Pin<Box<dyn Stream<Item = Result<RuntimeFrame, Status>> + Send + 'static>>;
pub type ManagementEventStream =
    Pin<Box<dyn Stream<Item = Result<ManagementEvent, Status>> + Send + 'static>>;

/// A Runtime-side peer behind the protocol validation and HTTP/JSON and private Agent socket boundaries.
#[async_trait::async_trait]
pub trait RuntimePeer: Send + Sync + 'static {
    /// Local management reads workspace-scoped resources without an execution token.
    async fn read_management_resource(
        &self,
        _workspace_id: String,
        _resource_id: i64,
    ) -> Result<(ResourceRef, Vec<u8>), Status> {
        Err(Status::unimplemented("read_management_resource"))
    }

    async fn upload_resource(&self, chunks: ResourceChunkStream) -> Result<ResourceRef, Status>;

    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status>;

    async fn import_figure_pack(
        &self,
        _request: ImportFigurePackRequest,
    ) -> Result<FigurePack, Status> {
        Err(Status::unimplemented("import_figure_pack"))
    }
    async fn list_figure_packs(
        &self,
        _request: ListFigurePacksRequest,
    ) -> Result<Vec<FigurePack>, Status> {
        Err(Status::unimplemented("list_figure_packs"))
    }
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

    async fn list_human_members(
        &self,
        _request: GetWorkspaceRequest,
    ) -> Result<Vec<HumanTeamMember>, Status> {
        Err(Status::unimplemented("list_human_members"))
    }
    async fn sync_human_profile(
        &self,
        _request: SyncHumanProfileRequest,
    ) -> Result<HumanTeamMember, Status> {
        Err(Status::unimplemented("sync_human_profile"))
    }
    async fn update_human_position(
        &self,
        _request: UpdateHumanPositionRequest,
    ) -> Result<Vec<HumanTeamMember>, Status> {
        Err(Status::unimplemented("update_human_position"))
    }

    async fn list_agents(&self, _request: ListAgentsRequest) -> Result<ListAgentsResponse, Status> {
        Err(Status::unimplemented("list_agents"))
    }

    async fn cancel_attempt(
        &self,
        _request: CancelAttemptRequest,
    ) -> Result<CancelAttemptResponse, Status> {
        Err(Status::unimplemented("cancel_attempt"))
    }

    async fn get_execution_details(
        &self,
        _request: GetExecutionDetailsRequest,
    ) -> Result<ExecutionDetails, Status> {
        Err(Status::unimplemented("get_execution_details"))
    }

    async fn get_agent_first_reply(
        &self,
        _request: GetAgentRequest,
    ) -> Result<AgentFirstReply, Status> {
        Err(Status::unimplemented("get_agent_first_reply"))
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

    async fn resume_agent(&self, _request: GetAgentRequest) -> Result<Agent, Status> {
        Err(Status::unimplemented("resume_agent"))
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

    async fn create_pairing_code(&self, _request: CatalogRequest) -> Result<PairingCode, Status> {
        Err(Status::unimplemented("create_pairing_code"))
    }
    async fn rename_machine(
        &self,
        _request: RenameMachineRequest,
    ) -> Result<MachineCatalog, Status> {
        Err(Status::unimplemented("rename_machine"))
    }
    async fn revoke_machine(
        &self,
        _request: RevokeMachineRequest,
    ) -> Result<MachineCatalog, Status> {
        Err(Status::unimplemented("revoke_machine"))
    }
    async fn get_machine_catalog(
        &self,
        _request: CatalogRequest,
    ) -> Result<MachineCatalog, Status> {
        Err(Status::unimplemented("get_machine_catalog"))
    }

    async fn get_harness_catalog(
        &self,
        _request: CatalogRequest,
    ) -> Result<HarnessCatalog, Status> {
        Err(Status::unimplemented("get_harness_catalog"))
    }

    async fn test_harness(
        &self,
        _request: TestHarnessRequest,
    ) -> Result<TestHarnessResponse, Status> {
        Err(Status::unimplemented("test_harness"))
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

    async fn builtin_agent_sign_in(
        &self,
        _request: BuiltinAgentSignInRequest,
    ) -> Result<BuiltinAgentSignInResponse, Status> {
        Err(Status::unimplemented("builtin_agent_sign_in"))
    }
    async fn builtin_agent_sign_in_status(
        &self,
        _request: BuiltinAgentSignInStatusRequest,
    ) -> Result<BuiltinAgentSignInStatus, Status> {
        Err(Status::unimplemented("builtin_agent_sign_in_status"))
    }
    async fn builtin_agent_sign_in_cancel(
        &self,
        _request: BuiltinAgentSignInStatusRequest,
    ) -> Result<BuiltinAgentSignInStatus, Status> {
        Err(Status::unimplemented("builtin_agent_sign_in_cancel"))
    }
    async fn builtin_agent_sign_out(
        &self,
        _request: BuiltinAgentSignOutRequest,
    ) -> Result<BuiltinAgentSignOutResponse, Status> {
        Err(Status::unimplemented("builtin_agent_sign_out"))
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

/// Accept a validated Machine connection, independent of the management transport.
#[async_trait::async_trait]
pub trait MachinePeer: Send + Sync + 'static {
    async fn authorize_link(
        &self,
        _credential: &str,
        _local_secret: &str,
    ) -> Result<(), crate::link::CloseReason> {
        Err(crate::link::CloseReason::Unauthorized)
    }
    async fn pair_machine(
        &self,
        _request: crate::link::PairMachineRequest,
        _local_secret: &str,
    ) -> Result<crate::link::PairMachineResponse, Status> {
        Err(Status::unimplemented("pair_machine"))
    }

    async fn link_rejected(&self, _hello: MachineHello, _reason: crate::link::CloseReason) {}

    async fn connect_machine(
        &self,
        hello: MachineHello,
        frames: MachineFrameStream,
    ) -> Result<RuntimeFrameStream, Status>;
}
