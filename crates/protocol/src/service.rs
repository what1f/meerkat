use crate::validation::*;
use crate::wire::*;
use crate::{
    MAX_WORKSPACE_IMAGE_BYTES, ManagementEventStream, ResourceChunkStream, RuntimePeer,
    TransportConfig,
};
use crate::{Request, Response, Status};
use tokio::sync::mpsc;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

/// Service adapter which rejects malformed wire input before it can
/// reach Runtime business code.
#[derive(Clone)]
pub struct ValidatedRuntimeService<P> {
    peer: P,
    max_message_bytes: usize,
}

impl<P> ValidatedRuntimeService<P> {
    pub const fn new(peer: P, config: TransportConfig) -> Self {
        Self {
            peer,
            max_message_bytes: config.max_message_bytes,
        }
    }
}

#[async_trait::async_trait]
impl<P> RuntimePeer for ValidatedRuntimeService<P>
where
    P: RuntimePeer,
{
    async fn upload_resource(&self, request: ResourceChunkStream) -> Result<ResourceRef, Status> {
        let mut inbound = request;
        let (sender, receiver) = mpsc::channel(16);
        let max_message_bytes = self.max_message_bytes;
        tokio::spawn(async move {
            let mut validator = ResourceUploadValidator::default();
            while let Some(next) = inbound.next().await {
                let validated = next.and_then(|chunk| {
                    if chunk.bytes.len() > max_message_bytes {
                        return Err(Status::resource_exhausted("agent_message_too_large"));
                    }
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
        Ok(resource)
    }

    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status> {
        validate_resource_request(&request)?;
        let mut chunks = self.peer.download_resource(request.clone()).await?;
        let (sender, receiver) = mpsc::channel(16);
        let max_message_bytes = self.max_message_bytes;
        tokio::spawn(async move {
            let mut validator =
                ResourceUploadValidator::expecting(request.transfer_id, request.resource_id);
            while let Some(next) = chunks.next().await {
                let validated = next.and_then(|chunk| {
                    if chunk.bytes.len() > max_message_bytes {
                        return Err(Status::resource_exhausted("agent_message_too_large"));
                    }
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
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }

    async fn list_capabilities(&self, request: AgentRunContext) -> Result<CapabilityList, Status> {
        validate_agent_run_context(Some(&request))?;
        Ok(self.peer.list_capabilities(request).await?)
    }

    async fn get_self_profile(&self, request: AgentRunContext) -> Result<AgentProfile, Status> {
        validate_agent_run_context(Some(&request))?;
        let response = self.peer.get_self_profile(request).await?;
        validate_agent_profile(&response)?;
        Ok(response)
    }

    async fn update_self_profile(
        &self,
        request: UpdateSelfProfileRequest,
    ) -> Result<AgentProfile, Status> {
        let input = &request;
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
        let response = self.peer.update_self_profile(request).await?;
        validate_agent_profile(&response)?;
        Ok(response)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvokeRequest,
    ) -> Result<CapabilityInvokeResult, Status> {
        let input = &request;
        validate_agent_run_context(input.context.as_ref())?;
        require_text(&input.capability_name, "capability_name")?;
        require_text(&input.invocation_id, "invocation_id")?;
        validate_json(&input.arguments_json, "arguments_json")?;
        Ok(self.peer.invoke_capability(request).await?)
    }

    async fn send_message(&self, request: SendMessageRequest) -> Result<SendMessageResult, Status> {
        let input = &request;
        validate_agent_run_context(input.context.as_ref())?;
        require_text(&input.channel_id, "channel_id")?;
        require_text(&input.chat_id, "chat_id")?;
        validate_send_message_content(&input.content_markdown, &input.resources)?;
        require_text(&input.idempotency_key, "idempotency_key")?;
        Ok(self.peer.send_message(request).await?)
    }

    async fn get_content(&self, request: ContentRequest) -> Result<ContentResult, Status> {
        let input = &request;
        validate_agent_run_context(input.context.as_ref())?;
        ContentKind::try_from(input.kind)
            .map_err(|_| Status::invalid_argument("unknown_content_kind"))?;
        require_text(&input.content_id, "content_id")?;
        Ok(self.peer.get_content(request).await?)
    }
}

impl<P: RuntimePeer> ValidatedRuntimeService<P> {
    pub async fn list_workspaces(
        &self,
        request: Request<ListWorkspacesRequest>,
    ) -> Result<Response<ListWorkspacesResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        Ok(Response::new(
            self.peer.list_workspaces(request.into_inner()).await?,
        ))
    }

    pub async fn import_figure_pack(
        &self,
        request: Request<ImportFigurePackRequest>,
    ) -> Result<Response<FigurePack>, Status> {
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.import_figure_pack(request.into_inner()).await?,
        ))
    }
    pub async fn list_figure_packs(
        &self,
        request: Request<ListFigurePacksRequest>,
    ) -> Result<Response<Vec<FigurePack>>, Status> {
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.list_figure_packs(request.into_inner()).await?,
        ))
    }
    pub async fn get_workspace(
        &self,
        request: Request<GetWorkspaceRequest>,
    ) -> Result<Response<Workspace>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.get_workspace(request.into_inner()).await?,
        ))
    }

    pub async fn get_workspace_image(
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

    pub async fn upload_workspace_image(
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

    pub async fn create_workspace(
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

    pub async fn update_workspace(
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

    pub async fn set_workspace_state(
        &self,
        request: Request<SetWorkspaceStateRequest>,
    ) -> Result<Response<Workspace>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_workspace_ref(input.workspace.as_ref())?;
        match input.desired_state {
            DesiredState::Running | DesiredState::Paused => {}
            _ => return Err(Status::invalid_argument("invalid_desired_state")),
        }
        Ok(Response::new(
            self.peer.set_workspace_state(request.into_inner()).await?,
        ))
    }

    pub async fn list_human_members(
        &self,
        request: Request<GetWorkspaceRequest>,
    ) -> Result<Response<Vec<HumanTeamMember>>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.list_human_members(request.into_inner()).await?,
        ))
    }
    pub async fn sync_human_profile(
        &self,
        request: Request<SyncHumanProfileRequest>,
    ) -> Result<Response<HumanTeamMember>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.sync_human_profile(request.into_inner()).await?,
        ))
    }
    pub async fn update_human_position(
        &self,
        request: Request<UpdateHumanPositionRequest>,
    ) -> Result<Response<Vec<HumanTeamMember>>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer
                .update_human_position(request.into_inner())
                .await?,
        ))
    }

    pub async fn list_agents(
        &self,
        request: Request<ListAgentsRequest>,
    ) -> Result<Response<ListAgentsResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        Ok(Response::new(
            self.peer.list_agents(request.into_inner()).await?,
        ))
    }

    pub async fn get_agent_first_reply(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<AgentFirstReply>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer
                .get_agent_first_reply(request.into_inner())
                .await?,
        ))
    }
    pub async fn get_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<Agent>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer.get_agent(request.into_inner()).await?,
        ))
    }

    pub async fn cancel_attempt(
        &self,
        request: Request<CancelAttemptRequest>,
    ) -> Result<Response<CancelAttemptResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        lankide_core::ExecutionId::new(request.get_ref().execution_id)
            .map_err(|_| Status::invalid_argument("invalid_execution_id"))?;
        Ok(Response::new(
            self.peer.cancel_attempt(request.into_inner()).await?,
        ))
    }

    pub async fn get_execution_details(
        &self,
        request: Request<GetExecutionDetailsRequest>,
    ) -> Result<Response<ExecutionDetails>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        lankide_core::ExecutionId::new(request.get_ref().execution_id)
            .map_err(|_| Status::invalid_argument("invalid_execution_id"))?;
        Ok(Response::new(
            self.peer
                .get_execution_details(request.into_inner())
                .await?,
        ))
    }

    pub async fn get_agent_avatar(
        &self,
        request: Request<GetAgentAvatarRequest>,
    ) -> Result<Response<GetAgentAvatarResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        let response = self.peer.get_agent_avatar(request.into_inner()).await?;
        validate_request_id(&response.request_id)?;
        validate_agent_ref(response.agent.as_ref())?;
        if let Some(avatar) = response.avatar.as_ref() {
            validate_avatar_file(avatar)?;
        }
        Ok(Response::new(response))
    }

    pub async fn create_agent(
        &self,
        request: Request<CreateAgentRequest>,
    ) -> Result<Response<Agent>, Status> {
        validate_agent_write(
            &request.get_ref().request_id,
            request.get_ref().agent.as_ref(),
            &request.get_ref().name,
            &request.get_ref().harness_id,
            &request.get_ref().machine_id,
            request.get_ref().attention.as_ref(),
        )?;
        Ok(Response::new(
            self.peer.create_agent(request.into_inner()).await?,
        ))
    }

    pub async fn update_agent(
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

    pub async fn resume_agent(
        &self,
        request: Request<GetAgentRequest>,
    ) -> Result<Response<Agent>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer.resume_agent(request.into_inner()).await?,
        ))
    }

    pub async fn delete_agent(
        &self,
        request: Request<DeleteAgentRequest>,
    ) -> Result<Response<DeleteAgentResponse>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        Ok(Response::new(
            self.peer.delete_agent(request.into_inner()).await?,
        ))
    }

    pub async fn get_provider_onboarding(
        &self,
        request: Request<GetProviderOnboardingRequest>,
    ) -> Result<Response<ProviderOnboardingState>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_workspace_ref(request.get_ref().workspace.as_ref())?;
        let state = self
            .peer
            .get_provider_onboarding(request.into_inner())
            .await?;
        validate_json(&state.payload_json, "payload_json")?;
        Ok(Response::new(state))
    }

    pub async fn advance_provider_onboarding(
        &self,
        request: Request<AdvanceProviderOnboardingRequest>,
    ) -> Result<Response<ProviderOnboardingState>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_workspace_ref(input.workspace.as_ref())?;
        require_text(&input.action, "onboarding_action")?;
        validate_json(&input.field_values_json, "field_values_json")?;
        let state = self
            .peer
            .advance_provider_onboarding(request.into_inner())
            .await?;
        validate_json(&state.payload_json, "payload_json")?;
        Ok(Response::new(state))
    }

    pub async fn get_agent_onboarding(
        &self,
        request: Request<GetAgentOnboardingRequest>,
    ) -> Result<Response<AgentOnboardingState>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        validate_agent_ref(request.get_ref().agent.as_ref())?;
        let state = self.peer.get_agent_onboarding(request.into_inner()).await?;
        validate_json(&state.payload_json, "payload_json")?;
        Ok(Response::new(state))
    }

    pub async fn advance_agent_onboarding(
        &self,
        request: Request<AdvanceAgentOnboardingRequest>,
    ) -> Result<Response<AgentOnboardingState>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        validate_agent_ref(input.agent.as_ref())?;
        require_text(&input.action, "onboarding_action")?;
        validate_json(&input.field_values_json, "field_values_json")?;
        let state = self
            .peer
            .advance_agent_onboarding(request.into_inner())
            .await?;
        validate_json(&state.payload_json, "payload_json")?;
        Ok(Response::new(state))
    }

    pub async fn get_provider_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<ProviderCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_provider_catalog(request.into_inner()).await?,
        ))
    }

    pub async fn get_machine_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<MachineCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_machine_catalog(request.into_inner()).await?,
        ))
    }

    pub async fn create_pairing_code(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<PairingCode>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.create_pairing_code(request.into_inner()).await?,
        ))
    }
    pub async fn rename_machine(
        &self,
        request: Request<RenameMachineRequest>,
    ) -> Result<Response<MachineCatalog>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        require_text(&request.get_ref().machine_id, "machine_id")?;
        Ok(Response::new(
            self.peer.rename_machine(request.into_inner()).await?,
        ))
    }

    pub async fn revoke_machine(
        &self,
        request: Request<RevokeMachineRequest>,
    ) -> Result<Response<MachineCatalog>, Status> {
        validate_request_id(&request.get_ref().request_id)?;
        require_text(&request.get_ref().machine_id, "machine_id")?;
        Ok(Response::new(
            self.peer.revoke_machine(request.into_inner()).await?,
        ))
    }

    pub async fn get_harness_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<HarnessCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_harness_catalog(request.into_inner()).await?,
        ))
    }

    pub async fn test_harness(
        &self,
        request: Request<TestHarnessRequest>,
    ) -> Result<Response<TestHarnessResponse>, Status> {
        require_text(&request.get_ref().machine_id, "machine_id")?;
        require_text(&request.get_ref().harness_id, "harness_id")?;
        Ok(Response::new(
            self.peer.test_harness(request.into_inner()).await?,
        ))
    }
    pub async fn get_model_catalog(
        &self,
        request: Request<ModelCatalogRequest>,
    ) -> Result<Response<ModelCatalog>, Status> {
        let input = request.get_ref();
        validate_request_id(&input.request_id)?;
        if input.workspace.is_some() {
            validate_workspace_ref(input.workspace.as_ref())?;
        }
        require_text(&input.machine_id, "machine_id")?;
        require_text(&input.harness_id, "harness_id")?;
        Ok(Response::new(
            self.peer.get_model_catalog(request.into_inner()).await?,
        ))
    }

    pub async fn get_reaction_catalog(
        &self,
        request: Request<CatalogRequest>,
    ) -> Result<Response<ReactionCatalog>, Status> {
        validate_catalog_request(request.get_ref())?;
        Ok(Response::new(
            self.peer.get_reaction_catalog(request.into_inner()).await?,
        ))
    }

    pub async fn subscribe_management_events(
        &self,
        request: Request<ManagementSubscription>,
    ) -> Result<Response<ManagementEventStream>, Status> {
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
            loop {
                let next = tokio::select! {
                    _ = sender.closed() => return,
                    next = events.next() => next,
                };
                let Some(next) = next else {
                    return;
                };
                let validated = next.and_then(|event| {
                    if event.sequence <= last_sequence {
                        return Err(Status::out_of_range("management_sequence_regression"));
                    }
                    let kind = event.kind;
                    if kind != ManagementEventKind::MachineCatalogChanged {
                        validate_workspace_ref(event.workspace.as_ref())?;
                    }
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

    pub async fn list_provider_interfaces(
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

    pub async fn invoke_provider_interface(
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
}
