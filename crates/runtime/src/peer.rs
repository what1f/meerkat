use super::*;
use lankide_protocol::Status;
use lankide_protocol::*;
use lankide_provider::ResourceUpload;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

#[derive(Clone)]
pub struct RuntimePeerAdapter {
    application: RuntimeApplication,
}
impl RuntimePeerAdapter {
    pub fn new(application: RuntimeApplication) -> Self {
        Self { application }
    }
    pub fn application(&self) -> &RuntimeApplication {
        &self.application
    }
}

#[async_trait::async_trait]
impl lankide_protocol::MachinePeer for RuntimePeerAdapter {
    async fn authorize_link(
        &self,
        credential: &str,
        _local_secret: &str,
    ) -> Result<(), lankide_protocol::link::CloseReason> {
        if credential.is_empty() {
            return Err(lankide_protocol::link::CloseReason::Unauthorized);
        }
        self.application
            .authenticate_machine(None, credential)
            .await
    }
    async fn pair_machine(
        &self,
        request: lankide_protocol::link::PairMachineRequest,
        local_secret: &str,
    ) -> Result<lankide_protocol::link::PairMachineResponse, Status> {
        self.application
            .pair_machine(request, local_secret)
            .await
            .map_err(status)
    }

    async fn link_rejected(
        &self,
        hello: MachineHello,
        reason: lankide_protocol::link::CloseReason,
    ) {
        log::error!("machine={} link_rejected={reason}", hello.machine_id);
        let _ = self
            .application
            .machine_broker()
            .reject_link(hello.machine_id, reason);
    }

    async fn connect_machine(
        &self,
        hello: MachineHello,
        mut frames: MachineFrameStream,
    ) -> Result<RuntimeFrameStream, Status> {
        let machine_id = hello.machine_id.clone();
        let instance_id = hello.instance_id.clone();
        let (connection_id, output) = {
            let _guard = self.application.inner.machine_registration.lock().await;
            self.application
                .authenticate_machine(Some(&machine_id), &hello.device_credential)
                .await
                .map_err(|reason| Status::unauthenticated(reason.to_string()))?;
            self.application
                .runtime()
                .map_err(status)?
                .database()
                .observe_machine(
                    &machine_id,
                    application::now_millis(),
                    Some(
                        &serde_json::to_string(&hello.harnesses)
                            .map_err(|_| Status::internal("inventory invalid"))?,
                    ),
                    Some(&hello.os),
                    Some(&hello.machine_version),
                )
                .await
                .map_err(|error| status(error.into()))?;
            self.application
                .machine_broker()
                .connect_tracked(hello)
                .await
                .map_err(status)?
        };
        self.application.set_machine_connected(true);
        self.application.emit_machine_catalog_changed();
        let cleanup_broker = self.application.machine_broker();
        let cleanup_machine_id = machine_id.clone();
        let application = self.application.clone();
        self.application
            .spawn_background(async move {
                let mut capability_calls = crate::capability_link::CapabilityCalls::default();
                let mut online = false;
                while let Some(frame) = frames.next().await {
                    match frame {
                        Ok(frame) => {
                            if let Some(machine_frame::Frame::CapabilityRequest(chunk)) =
                                frame.frame.clone()
                            {
                                let request_id = chunk.request_id.clone();
                                match capability_calls.accept(chunk) {
                                    Ok(Some((request, permit))) => {
                                        let application = application.clone();
                                        let machine = machine_id.clone();
                                        let instance = instance_id.clone();
                                        let owner = application.clone();
                                        if owner
                                            .spawn_background(async move {
                                                let _permit = permit;
                                                crate::capability_link::respond(
                                                    application,
                                                    machine,
                                                    instance,
                                                    request,
                                                )
                                                .await;
                                            })
                                            .is_err()
                                        {
                                            break;
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        let _ = application
                                            .machine_broker()
                                            .send_frame(
                                                &machine_id,
                                                &instance_id,
                                                RuntimeFrame {
                                                    protocol_version:
                                                        lankide_protocol::PROTOCOL_VERSION,
                                                    frame: Some(
                                                        runtime_frame::Frame::CapabilityResponse(
                                                            CapabilityResponse {
                                                                request_id,
                                                                eof: true,
                                                                error_code: error.code() as i32,
                                                                error_message: error
                                                                    .message()
                                                                    .into(),
                                                                ..Default::default()
                                                            },
                                                        ),
                                                    ),
                                                },
                                            )
                                            .await;
                                    }
                                }
                                continue;
                            }
                            let first_heartbeat = !online
                                && matches!(&frame.frame, Some(machine_frame::Frame::Heartbeat(_)));
                            let inventory_changed = matches!(
                                &frame.frame,
                                Some(machine_frame::Frame::InventoryChanged(_))
                            );
                            if let Err(error) = application
                                .machine_broker()
                                .handle_frame(&machine_id, &instance_id, frame)
                                .await
                            {
                                // Reject this stale attempt without dropping the authenticated
                                // Machine link before its outbox-discard response can arrive.
                                if !matches!(error, RuntimeError::InvalidExecutionToken) {
                                    log::warn!("machine={machine_id} link frame failed: {error}");
                                    break;
                                }
                            } else {
                                if first_heartbeat {
                                    online = true;
                                }
                                if inventory_changed || first_heartbeat {
                                    application.emit_machine_catalog_changed();
                                }
                            }
                        }
                        Err(error) => {
                            log::warn!("machine={machine_id} link failed: {error}");
                            break;
                        }
                    }
                }
                let _ = application
                    .machine_broker()
                    .disconnect_connection(&machine_id, &instance_id, connection_id)
                    .await;
                application.emit_machine_catalog_changed();
                application.set_machine_connected(
                    application
                        .machine_broker()
                        .catalog()
                        .is_ok_and(|values| !values.is_empty()),
                );
            })
            .map_err(status)?;
        self.application.spawn_background(async move {
            if let Err(error) = cleanup_broker
                .retry_agent_file_deletions(&cleanup_machine_id)
                .await
            {
                log::error!(
                    "[runtime] machine={cleanup_machine_id} deferred Agent file cleanup could not start: {error}"
                );
            }
        }).map_err(status)?;
        Ok(output)
    }
}

#[async_trait::async_trait]
impl RuntimePeer for RuntimePeerAdapter {
    async fn read_management_resource(
        &self,
        workspace_id: String,
        resource_id: i64,
    ) -> Result<(ResourceRef, Vec<u8>), Status> {
        let workspace_id = WorkspaceId::new(workspace_id)
            .map_err(|_| Status::invalid_argument("Choose a valid workspace"))?;
        let resource_id = ResourceId::new(resource_id)
            .map_err(|_| Status::invalid_argument("Choose a valid resource"))?;
        let runtime = self.application.runtime().map_err(status)?;
        runtime
            .database()
            .resource_metadata(&workspace_id, resource_id)
            .await
            .map_err(|error| status(error.into()))?
            .ok_or_else(|| Status::not_found("Resource not found"))?;
        let value = self
            .application
            .resource_store()
            .map_err(status)?
            .read(&workspace_id, resource_id)
            .await
            .map_err(status)?;
        Ok((value.resource, value.bytes))
    }

    async fn upload_resource(
        &self,
        mut chunks: ResourceChunkStream,
    ) -> Result<ResourceRef, Status> {
        let mut descriptor = None;
        let mut context = None;
        let mut transfer_id = None;
        let mut bytes = Vec::new();
        let mut saw_eof = false;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            if saw_eof {
                return Err(Status::invalid_argument(
                    "Resource upload is already complete.",
                ));
            }
            if transfer_id
                .as_ref()
                .is_some_and(|id| id != &chunk.transfer_id)
            {
                return Err(Status::invalid_argument("Resource upload ID changed."));
            }
            transfer_id.get_or_insert(chunk.transfer_id);
            if chunk.offset != bytes.len() as u64 {
                return Err(Status::invalid_argument("Resource upload is incomplete."));
            }
            if let Some(value) = chunk.descriptor {
                if descriptor.is_some() {
                    return Err(Status::invalid_argument("Resource details were repeated."));
                }
                context = value.context.clone();
                descriptor = Some(value);
            }
            bytes.extend_from_slice(&chunk.bytes);
            if bytes.len() > lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES {
                return Err(Status::resource_exhausted("Resource is too large."));
            }
            saw_eof = chunk.eof;
        }
        if !saw_eof {
            return Err(Status::invalid_argument("Resource upload is incomplete."));
        }
        let descriptor =
            descriptor.ok_or_else(|| Status::invalid_argument("Resource details are required."))?;
        let context = context
            .ok_or_else(|| Status::permission_denied("This resource upload is not authorized."))?;
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if descriptor.size_bytes != bytes.len() as u64 || descriptor.sha256 != digest {
            return Err(Status::invalid_argument("Resource integrity check failed."));
        }
        self.application
            .save_resource(
                &context,
                ResourceUpload {
                    file_name: descriptor.file_name,
                    media_type: descriptor.media_type,
                    bytes,
                },
            )
            .await
            .map_err(status)
    }

    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status> {
        let context = request.context.as_ref().ok_or_else(|| {
            Status::permission_denied("This resource download is not authorized.")
        })?;
        let resource_id = ResourceId::new(request.resource_id)
            .map_err(|_| Status::invalid_argument("Choose a valid resource."))?;
        let value = self
            .application
            .read_resource(context, resource_id)
            .await
            .map_err(status)?;
        let descriptor = ResourceDescriptor {
            file_name: value.resource.file_name,
            media_type: value.resource.media_type,
            size_bytes: value.resource.size_bytes,
            sha256: value.resource.sha256,
            context: request.context.clone(),
        };
        let chunk_size = 64 * 1024;
        let mut output = Vec::new();
        if value.bytes.is_empty() {
            output.push(Ok(ResourceChunk {
                transfer_id: request.transfer_id,
                resource_id: request.resource_id,
                offset: 0,
                bytes: Vec::new(),
                eof: true,
                descriptor: Some(descriptor),
            }));
        } else {
            let total = value.bytes.len();
            for (index, chunk) in value.bytes.chunks(chunk_size).enumerate() {
                let offset = index * chunk_size;
                output.push(Ok(ResourceChunk {
                    transfer_id: request.transfer_id.clone(),
                    resource_id: request.resource_id,
                    offset: offset as u64,
                    bytes: chunk.to_vec(),
                    eof: offset + chunk.len() == total,
                    descriptor: (index == 0).then(|| descriptor.clone()),
                }));
            }
        }
        Ok(Box::pin(tokio_stream::iter(output)))
    }

    async fn get_workspace_image(
        &self,
        request: WorkspaceImageRequest,
    ) -> Result<WorkspaceImageResponse, Status> {
        let workspace_id = request
            .workspace
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Choose a workspace"))?;
        let workspace_id = WorkspaceId::new(workspace_id.workspace_id.clone())
            .map_err(|_| Status::invalid_argument("Choose a valid workspace"))?;
        let resource_id = ResourceId::new(request.resource_id)
            .map_err(|_| Status::invalid_argument("Choose a valid image"))?;
        let runtime = self.application.runtime().map_err(status)?;
        let storage = runtime.database();
        let workspace = storage
            .workspace_record(&workspace_id)
            .await
            .map_err(|error| status(error.into()))?;
        let figure_reference = storage
            .list_figure_packs(&workspace_id)
            .await
            .map_err(|error| status(error.into()))?
            .iter()
            .any(|pack| {
                pack["frames"].as_object().is_some_and(|frames| {
                    frames
                        .values()
                        .any(|frame| frame["resource_id"].as_i64() == Some(resource_id.get()))
                })
            });
        let referenced = workspace.cover_resource_id == Some(resource_id) || figure_reference;
        if !referenced {
            return Err(Status::permission_denied(
                "This image is not part of the workspace profile",
            ));
        }
        let metadata = storage
            .resource_metadata(&workspace_id, resource_id)
            .await
            .map_err(|error| status(error.into()))?
            .ok_or_else(|| Status::not_found("Image not found"))?;
        if metadata.size_bytes > MAX_WORKSPACE_IMAGE_BYTES as u64 {
            return Err(Status::resource_exhausted("Image exceeds the size limit"));
        }
        if !matches!(
            metadata.media_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err(Status::invalid_argument("Unsupported image format"));
        }
        let image = self
            .application
            .resource_store()
            .map_err(status)?
            .read(&workspace_id, resource_id)
            .await
            .map_err(status)?;
        Ok(WorkspaceImageResponse {
            request_id: request.request_id,
            workspace: request.workspace,
            resource: Some(image.resource),
            bytes: image.bytes,
        })
    }

    async fn upload_workspace_image(
        &self,
        request: UploadWorkspaceImageRequest,
    ) -> Result<UploadWorkspaceImageResponse, Status> {
        let workspace = request
            .workspace
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Choose a workspace"))?;
        let workspace_id = WorkspaceId::new(workspace.workspace_id.clone())
            .map_err(|_| Status::invalid_argument("Choose a valid workspace"))?;
        let runtime = self.application.runtime().map_err(status)?;
        runtime
            .database()
            .workspace_record(&workspace_id)
            .await
            .map_err(|error| status(error.into()))?;
        let resource = self
            .application
            .resource_store()
            .map_err(status)?
            .save(
                &workspace_id,
                ResourceUpload {
                    file_name: request.file_name,
                    media_type: request.media_type,
                    bytes: request.bytes,
                },
            )
            .await
            .map_err(status)?;
        Ok(UploadWorkspaceImageResponse {
            request_id: request.request_id,
            workspace: request.workspace,
            resource: Some(resource),
        })
    }

    async fn import_figure_pack(
        &self,
        request: lankide_protocol::ImportFigurePackRequest,
    ) -> Result<lankide_protocol::FigurePack, Status> {
        self.application
            .import_figure_pack(request)
            .await
            .map_err(status)
    }
    async fn list_figure_packs(
        &self,
        request: lankide_protocol::ListFigurePacksRequest,
    ) -> Result<Vec<lankide_protocol::FigurePack>, Status> {
        self.application
            .list_figure_packs(request)
            .await
            .map_err(status)
    }
    async fn list_workspaces(
        &self,
        request: ListWorkspacesRequest,
    ) -> Result<ListWorkspacesResponse, Status> {
        self.application
            .list_workspaces(request)
            .await
            .map_err(status)
    }
    async fn get_workspace(&self, request: GetWorkspaceRequest) -> Result<Workspace, Status> {
        self.application
            .get_workspace(request)
            .await
            .map_err(status)
    }
    async fn create_workspace(&self, request: CreateWorkspaceRequest) -> Result<Workspace, Status> {
        self.application
            .create_workspace(request)
            .await
            .map_err(status)
    }
    async fn update_workspace(&self, request: UpdateWorkspaceRequest) -> Result<Workspace, Status> {
        self.application
            .update_workspace(request)
            .await
            .map_err(status)
    }
    async fn set_workspace_state(
        &self,
        request: SetWorkspaceStateRequest,
    ) -> Result<Workspace, Status> {
        self.application
            .set_workspace_state(request)
            .await
            .map_err(status)
    }
    async fn list_human_members(
        &self,
        request: GetWorkspaceRequest,
    ) -> Result<Vec<lankide_protocol::HumanTeamMember>, Status> {
        self.application
            .list_human_members(request)
            .await
            .map_err(status)
    }
    async fn update_human_position(
        &self,
        request: lankide_protocol::UpdateHumanPositionRequest,
    ) -> Result<Vec<lankide_protocol::HumanTeamMember>, Status> {
        self.application
            .update_human_position(request)
            .await
            .map_err(status)
    }

    async fn list_agents(&self, request: ListAgentsRequest) -> Result<ListAgentsResponse, Status> {
        self.application.list_agents(request).await.map_err(status)
    }
    async fn cancel_attempt(
        &self,
        request: lankide_protocol::CancelAttemptRequest,
    ) -> Result<lankide_protocol::CancelAttemptResponse, Status> {
        self.application
            .cancel_attempt(request)
            .await
            .map_err(status)
    }

    async fn get_execution_details(
        &self,
        request: GetExecutionDetailsRequest,
    ) -> Result<ExecutionDetails, Status> {
        self.application
            .get_execution_details(request)
            .await
            .map_err(status)
    }
    async fn get_agent_first_reply(
        &self,
        request: GetAgentRequest,
    ) -> Result<lankide_protocol::AgentFirstReply, Status> {
        self.application
            .get_agent_first_reply(request)
            .await
            .map_err(status)
    }
    async fn get_agent(&self, request: GetAgentRequest) -> Result<Agent, Status> {
        self.application.get_agent(request).await.map_err(status)
    }
    async fn get_agent_avatar(
        &self,
        request: GetAgentAvatarRequest,
    ) -> Result<GetAgentAvatarResponse, Status> {
        self.application
            .get_agent_avatar(request)
            .await
            .map_err(status)
    }
    async fn create_agent(&self, request: CreateAgentRequest) -> Result<Agent, Status> {
        self.application.create_agent(request).await.map_err(status)
    }
    async fn resume_agent(&self, request: GetAgentRequest) -> Result<Agent, Status> {
        self.application.resume_agent(request).await.map_err(status)
    }
    async fn update_agent(&self, request: UpdateAgentRequest) -> Result<Agent, Status> {
        self.application.update_agent(request).await.map_err(status)
    }

    async fn get_self_profile(&self, context: AgentRunContext) -> Result<AgentProfile, Status> {
        self.application
            .get_self_profile(context)
            .await
            .map_err(status)
    }

    async fn update_self_profile(
        &self,
        request: UpdateSelfProfileRequest,
    ) -> Result<AgentProfile, Status> {
        self.application
            .update_self_profile(request)
            .await
            .map_err(status)
    }
    async fn delete_agent(
        &self,
        request: DeleteAgentRequest,
    ) -> Result<DeleteAgentResponse, Status> {
        self.application.delete_agent(request).await.map_err(status)
    }
    async fn get_provider_onboarding(
        &self,
        request: GetProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingState, Status> {
        self.application
            .get_provider_onboarding(request)
            .await
            .map_err(status)
    }
    async fn advance_provider_onboarding(
        &self,
        request: AdvanceProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingState, Status> {
        self.application
            .advance_provider_onboarding(request)
            .await
            .map_err(status)
    }
    async fn get_agent_onboarding(
        &self,
        request: GetAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, Status> {
        self.application
            .get_agent_onboarding(request)
            .await
            .map_err(status)
    }
    async fn advance_agent_onboarding(
        &self,
        request: AdvanceAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, Status> {
        self.application
            .advance_agent_onboarding(request)
            .await
            .map_err(status)
    }
    async fn get_provider_catalog(
        &self,
        request: CatalogRequest,
    ) -> Result<ProviderCatalog, Status> {
        self.application.provider_catalog(request).map_err(status)
    }
    async fn create_pairing_code(&self, _request: CatalogRequest) -> Result<PairingCode, Status> {
        self.application.create_pairing_code().await.map_err(status)
    }
    async fn rename_machine(
        &self,
        request: RenameMachineRequest,
    ) -> Result<MachineCatalog, Status> {
        self.application
            .rename_machine(&request.machine_id, &request.name)
            .await
            .map_err(status)?;
        self.application
            .machine_catalog(CatalogRequest {
                request_id: request.request_id,
                workspace: None,
            })
            .await
            .map_err(status)
    }
    async fn revoke_machine(
        &self,
        request: RevokeMachineRequest,
    ) -> Result<MachineCatalog, Status> {
        self.application
            .revoke_machine(&request.machine_id)
            .await
            .map_err(status)?;
        self.application
            .machine_catalog(CatalogRequest {
                request_id: request.request_id,
                workspace: None,
            })
            .await
            .map_err(status)
    }
    async fn get_machine_catalog(&self, request: CatalogRequest) -> Result<MachineCatalog, Status> {
        self.application
            .machine_catalog(request)
            .await
            .map_err(status)
    }
    async fn test_harness(
        &self,
        request: lankide_protocol::TestHarnessRequest,
    ) -> Result<lankide_protocol::TestHarnessResponse, Status> {
        self.application
            .machine_broker()
            .test_harness(request)
            .await
    }
    async fn get_model_catalog(
        &self,
        request: lankide_protocol::ModelCatalogRequest,
    ) -> Result<lankide_protocol::ModelCatalog, Status> {
        let catalog = self
            .application
            .machine_catalog(CatalogRequest::default())
            .await
            .map_err(status)?;
        let machine = catalog
            .machines
            .into_iter()
            .find(|machine| machine.machine_id == request.machine_id)
            .ok_or_else(|| Status::not_found("Machine not found"))?;
        if machine.observed_state != lankide_protocol::ObservedState::Running
            || machine.revoked_at.is_some()
        {
            return Err(Status::unavailable("Machine is offline"));
        }
        let harness = machine
            .harnesses
            .into_iter()
            .find(|harness| harness.harness_id == request.harness_id)
            .ok_or_else(|| Status::not_found("Harness not found"))?;
        if !harness.model_catalog_error.is_empty() {
            return Err(Status::unavailable("Models could not be loaded"));
        }
        harness
            .model_catalog
            .ok_or_else(|| Status::unavailable("Models are not ready"))
    }
    async fn get_harness_catalog(&self, request: CatalogRequest) -> Result<HarnessCatalog, Status> {
        self.application.harness_catalog(request).map_err(status)
    }
    async fn get_reaction_catalog(
        &self,
        request: CatalogRequest,
    ) -> Result<ReactionCatalog, Status> {
        self.application
            .reaction_catalog(request)
            .await
            .map_err(status)
    }
    async fn subscribe_management_events(
        &self,
        request: ManagementSubscription,
    ) -> Result<ManagementEventStream, Status> {
        let mut receiver = self.application.subscribe_management();
        let workspace = request.workspace.map(|value| value.workspace_id);
        let after = request.after_sequence;
        let (sender, output) = mpsc::channel(128);
        self.application
            .spawn_background(async move {
                loop {
                    let received = tokio::select! {
                        _ = sender.closed() => break,
                        event = receiver.recv() => event,
                    };
                    let event = match received {
                        Ok(event) => event,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = sender
                                .send(Err(Status::unavailable("management events lagged")))
                                .await;
                            break;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if event.sequence <= after
                        || (event.kind
                            != lankide_protocol::ManagementEventKind::MachineCatalogChanged
                            && workspace.as_ref().is_some_and(|workspace| {
                                event
                                    .workspace
                                    .as_ref()
                                    .is_none_or(|value| &value.workspace_id != workspace)
                            }))
                    {
                        continue;
                    }
                    if sender.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
            })
            .map_err(status)?;
        Ok(Box::pin(ReceiverStream::new(output)))
    }

    async fn list_capabilities(&self, context: AgentRunContext) -> Result<CapabilityList, Status> {
        self.application
            .list_capabilities(context)
            .await
            .map_err(status)
    }
    async fn invoke_provider_interface(
        &self,
        request: InvokeProviderInterfaceRequest,
    ) -> Result<InvokeProviderInterfaceResponse, Status> {
        let workspace = request
            .workspace
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Workspace is required."))?;
        let workspace_id = WorkspaceId::new(&workspace.workspace_id)
            .map_err(|_| Status::invalid_argument("Workspace is invalid."))?;
        let input = serde_json::from_str(&request.input_json)
            .map_err(|_| Status::invalid_argument("Input must be valid JSON."))?;
        let output = self
            .application
            .runtime()
            .map_err(status)?
            .invoke_provider_interface(&workspace_id, request.interface_id.clone(), input)
            .await
            .map_err(status)?;
        Ok(InvokeProviderInterfaceResponse {
            request_id: request.request_id,
            workspace: request.workspace,
            interface_id: request.interface_id,
            output_json: output.to_string(),
        })
    }
    async fn list_provider_interfaces(
        &self,
        request: ListProviderInterfacesRequest,
    ) -> Result<ListProviderInterfacesResponse, Status> {
        let workspace = request
            .workspace
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("Workspace is required."))?;
        let workspace_id = WorkspaceId::new(&workspace.workspace_id)
            .map_err(|_| Status::invalid_argument("Workspace is invalid."))?;
        let interfaces = self
            .application
            .runtime()
            .map_err(status)?
            .list_provider_interfaces(&workspace_id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|entry| ProviderInterface {
                interface_id: entry.id,
                description: entry.description,
                input_schema_json: entry.input_schema.to_string(),
                output_schema_json: entry.output_schema.to_string(),
            })
            .collect();
        Ok(ListProviderInterfacesResponse {
            request_id: request.request_id,
            workspace: request.workspace,
            interfaces,
        })
    }
    async fn invoke_capability(
        &self,
        request: CapabilityInvokeRequest,
    ) -> Result<CapabilityInvokeResult, Status> {
        self.application
            .invoke_capability(request)
            .await
            .map_err(status)
    }
    async fn send_message(
        &self,
        request: SendMessageRequest,
    ) -> Result<lankide_protocol::SendMessageResult, Status> {
        self.application
            .send_message_gateway(request)
            .await
            .map_err(status)
    }
    async fn get_content(&self, request: ContentRequest) -> Result<ContentResult, Status> {
        self.application
            .get_content_gateway(request)
            .await
            .map_err(status)
    }
}

pub(crate) fn status(error: RuntimeError) -> Status {
    match error {
        RuntimeError::UnknownWorkspace(_) => Status::not_found("Workspace not found."),
        RuntimeError::UnknownAgent { .. } => Status::not_found("Agent not found."),
        RuntimeError::UnknownChannel { .. } => Status::not_found("Channel not found."),
        RuntimeError::UnknownProviderType(_) => Status::not_found("Provider is not available."),
        RuntimeError::DuplicateWorkspace(_) => Status::already_exists("Workspace already exists."),
        RuntimeError::DuplicateAgent { .. } => Status::already_exists("Agent already exists."),
        RuntimeError::DuplicateProviderType(_) => {
            Status::already_exists("Provider is already available.")
        }
        RuntimeError::MachineHasAssignedAgents => {
            Status::failed_precondition("Move this machine's Agents to another machine first.")
                .with_detail(Some("machine_has_assigned_agents".into()))
        }
        RuntimeError::LocalMachineCannotBeRemoved => {
            Status::failed_precondition("This machine cannot be removed.")
                .with_detail(Some("local_machine_cannot_be_removed".into()))
        }
        RuntimeError::MachineNameInvalid => {
            Status::invalid_argument("Enter a name with 1 to 80 characters.")
                .with_detail(Some("machine_name_invalid".into()))
        }
        RuntimeError::MachineNotRegistered
        | RuntimeError::MachineRevoked
        | RuntimeError::HarnessUnavailableOnMachine => {
            let code = error.to_string();
            Status::failed_precondition("Check the request values and try again.")
                .with_detail(Some(code))
        }
        RuntimeError::InvalidCore(_) => {
            Status::invalid_argument("Check the request values and try again.")
        }
        RuntimeError::ActionRequired(message) => Status::failed_precondition(message),
        RuntimeError::Application(_) => Status::internal("The request could not be completed."),
        RuntimeError::WorkspacePaused(_) => Status::failed_precondition("Workspace is paused."),
        RuntimeError::InvalidExecutionToken => {
            Status::permission_denied("This action is no longer authorized.")
        }
        RuntimeError::ProviderCall(lankide_provider::ProviderError::AppConfigurationFailed {
            ..
        }) => Status::failed_precondition("The connection could not be completed. Try again.")
            .with_detail(Some("app_configuration_failed".into())),
        RuntimeError::ProviderCall(lankide_provider::ProviderError::NotFound { .. })
        | RuntimeError::ProviderCall(lankide_provider::ProviderError::UnknownRegistration {
            ..
        }) => Status::not_found("The provider interface was not found."),
        RuntimeError::ProviderCall(lankide_provider::ProviderError::InvalidInput {
            field: "team_enterprise",
        }) => Status::failed_precondition("This person belongs to another organization.")
            .with_detail(Some("team_enterprise".into())),
        RuntimeError::ProviderCall(lankide_provider::ProviderError::InvalidInput { .. })
        | RuntimeError::ProviderCall(lankide_provider::ProviderError::InvalidSchema { .. })
        | RuntimeError::ProviderCall(lankide_provider::ProviderError::SchemaMismatch { .. }) => {
            Status::invalid_argument("The provider interface input is invalid.")
        }
        RuntimeError::Provider { .. } | RuntimeError::ProviderCall(_) => {
            Status::failed_precondition("The provider is unavailable. Try again.")
        }
        _ => Status::internal("The request could not be completed."),
    }
}
