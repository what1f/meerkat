use super::*;
use lankide_protocol::*;
use lankide_provider::ResourceUpload;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tonic::Status;

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

#[tonic::async_trait]
impl RuntimePeer for RuntimePeerAdapter {
    async fn connect_computer(
        &self,
        hello: ComputerHello,
        mut frames: ComputerFrameStream,
    ) -> Result<RuntimeFrameStream, Status> {
        let computer_id = hello.computer_id.clone();
        let instance_id = hello.instance_id.clone();
        let (connection_id, output) = self
            .application
            .computer_broker()
            .connect_tracked(hello)
            .map_err(status)?;
        self.application.set_computer_connected(true);
        let cleanup_broker = self.application.computer_broker();
        let cleanup_computer_id = computer_id.clone();
        let application = self.application.clone();
        self.application
            .spawn_background(async move {
                while let Some(frame) = frames.next().await {
                    match frame {
                        Ok(frame) => {
                            if let Some(computer_frame::Frame::ToolCall(call)) = frame.frame.clone()
                            {
                                let application = application.clone();
                                let computer_id = computer_id.clone();
                                let instance_id = instance_id.clone();
                                let owner = application.clone();
                                if owner
                                    .spawn_background(async move {
                                        let result = application
                                            .invoke_capability(CapabilityInvokeRequest {
                                                context: Some(AgentRunContext {
                                                    execution: call.execution.clone(),
                                                    execution_token: call.execution_token,
                                                }),
                                                capability_name: call.tool_name.clone(),
                                                arguments_json: call.arguments_json,
                                                invocation_id: call.tool_call_id.clone(),
                                            })
                                            .await;
                                        let tool_result = match result {
                                            Ok(result) => ToolResult {
                                                execution: call.execution,
                                                tool_call_id: call.tool_call_id,
                                                success: result.success,
                                                content_markdown: result.content_markdown,
                                                structured_content_json: result
                                                    .structured_content_json,
                                                resources: result.resources,
                                                error: result.error,
                                            },
                                            Err(_) => ToolResult {
                                                execution: call.execution,
                                                tool_call_id: call.tool_call_id,
                                                success: false,
                                                content_markdown: String::new(),
                                                structured_content_json: "{}".into(),
                                                resources: Vec::new(),
                                                error: Some(ToolError {
                                                    code: "tool_unavailable".into(),
                                                    message:
                                                        "This tool is not available right now."
                                                            .into(),
                                                    retryable: true,
                                                }),
                                            },
                                        };
                                        if application
                                            .computer_broker()
                                            .send_frame(
                                                &computer_id,
                                                &instance_id,
                                                RuntimeFrame {
                                                    frame: Some(runtime_frame::Frame::ToolResult(
                                                        tool_result,
                                                    )),
                                                },
                                            )
                                            .await
                                            .is_err()
                                        {
                                            eprintln!(
                                                "Runtime tool result could not reach the Computer"
                                            );
                                        }
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                            if application
                                .computer_broker()
                                .handle_frame(&computer_id, &instance_id, frame)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let _ = application.computer_broker().disconnect_connection(
                    &computer_id,
                    &instance_id,
                    connection_id,
                );
                application.set_computer_connected(
                    application
                        .computer_broker()
                        .catalog()
                        .is_ok_and(|values| !values.is_empty()),
                );
            })
            .map_err(status)?;
        self.application.spawn_background(async move {
            if let Err(error) = cleanup_broker
                .retry_agent_file_deletions(&cleanup_computer_id)
                .await
            {
                eprintln!(
                    "[runtime] computer={cleanup_computer_id} deferred Agent file cleanup could not start: {error}"
                );
            }
        }).map_err(status)?;
        Ok(output)
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
        let storage = runtime.storage();
        let workspace = storage
            .workspace_record(&workspace_id)
            .map_err(|error| status(error.into()))?;
        let referenced = workspace.cover_resource_id == Some(resource_id);
        if !referenced {
            return Err(Status::permission_denied(
                "This image is not part of the workspace profile",
            ));
        }
        let metadata = storage
            .resource_metadata(&workspace_id, resource_id)
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
            .storage()
            .workspace_record(&workspace_id)
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
    async fn get_computer_catalog(
        &self,
        request: CatalogRequest,
    ) -> Result<ComputerCatalog, Status> {
        self.application.computer_catalog(request).map_err(status)
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
                    let event = match receiver.recv().await {
                        Ok(event) => event,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if event.sequence <= after
                        || workspace.as_ref().is_some_and(|workspace| {
                            event
                                .workspace
                                .as_ref()
                                .is_none_or(|value| &value.workspace_id != workspace)
                        })
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

fn status(error: RuntimeError) -> Status {
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
        RuntimeError::InvalidCore(_) => {
            Status::invalid_argument("Check the request values and try again.")
        }
        RuntimeError::ActionRequired(message) => Status::failed_precondition(message),
        RuntimeError::Application(_) => Status::internal("The request could not be completed."),
        RuntimeError::WorkspacePaused(_) => Status::failed_precondition("Workspace is paused."),
        RuntimeError::InvalidExecutionToken => {
            Status::permission_denied("This action is no longer authorized.")
        }
        RuntimeError::ProviderCall(lankide_provider::ProviderError::NotFound { .. })
        | RuntimeError::ProviderCall(lankide_provider::ProviderError::UnknownRegistration {
            ..
        }) => Status::not_found("The provider interface was not found."),
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
