//! HTTP transport adapter for the management contract.
use futures_util::{SinkExt, StreamExt};
use lankide_protocol::{self as wire, management as json};
use lankide_protocol::{Response, Status};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
};
use tokio_stream::Stream;

#[derive(Clone)]
pub struct HttpManagementTransport {
    run: PathBuf,
    http: reqwest::Client,
    limit: usize,
}
impl HttpManagementTransport {
    pub fn local(run: impl AsRef<Path>, limit: usize) -> Self {
        Self {
            run: run.as_ref().to_owned(),
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("HTTP client"),
            limit,
        }
    }
    fn endpoint(&self) -> Result<(String, String), Status> {
        let url = std::fs::read_to_string(self.run.join(json::URL_FILE))
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let token = std::fs::read_to_string(self.run.join(json::OWNER_FILE))
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let parsed =
            reqwest::Url::parse(url.trim()).map_err(|e| Status::internal(e.to_string()))?;
        if parsed.scheme() != "http" || parsed.host_str().is_none() {
            return Err(Status::internal("invalid management listener URL"));
        }
        Ok((url.trim_end_matches('/').trim().to_owned(), token))
    }
    async fn call<I: serde::Serialize, O: serde::de::DeserializeOwned>(
        &self,
        operation: &str,
        input: &I,
    ) -> Result<O, Status> {
        let (url, token) = self.endpoint()?;
        let bytes = serde_json::to_vec(input).map_err(|e| Status::internal(e.to_string()))?;
        if bytes.len() > json::json_limit(self.limit) {
            return Err(Status::resource_exhausted("management request too large"));
        }
        let mut response = self
            .http
            .post(format!("{url}{}{operation}", json::PREFIX))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Status::unavailable(e.to_string()))?
        {
            if bytes.len().saturating_add(chunk.len()) > json::json_limit(self.limit) {
                return Err(Status::resource_exhausted("management response too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(serde_json::from_slice::<json::ErrorBody>(&bytes)
                .map_err(|e| Status::internal(e.to_string()))?
                .into_status());
        }
        serde_json::from_slice(&bytes).map_err(|e| Status::internal(e.to_string()))
    }

    pub async fn enable_remote_access(&self) -> Result<json::RemoteAccess, Status> {
        self.call("enable_remote_access", &()).await
    }
    pub async fn get_remote_access(&self) -> Result<json::RemoteAccess, Status> {
        self.call("get_remote_access", &()).await
    }
    pub async fn set_remote_access(
        &self,
        address: Option<String>,
    ) -> Result<json::RemoteAccess, Status> {
        self.call(
            "set_remote_access",
            &json::SetRemoteAccessRequest { address },
        )
        .await
    }
    pub async fn list_workspaces(
        &self,
        request: wire::ListWorkspacesRequest,
    ) -> Result<Response<wire::ListWorkspacesResponse>, Status> {
        let response: json::ListWorkspacesResponse = self.call("list_workspaces", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_workspace(
        &self,
        request: wire::GetWorkspaceRequest,
    ) -> Result<Response<wire::Workspace>, Status> {
        let response: json::Workspace = self.call("get_workspace", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_workspace_image(
        &self,
        request: wire::WorkspaceImageRequest,
    ) -> Result<Response<wire::WorkspaceImageResponse>, Status> {
        let response: json::WorkspaceImageResponse =
            self.call("get_workspace_image", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn upload_workspace_image(
        &self,
        request: wire::UploadWorkspaceImageRequest,
    ) -> Result<Response<wire::UploadWorkspaceImageResponse>, Status> {
        let response: json::UploadWorkspaceImageResponse =
            self.call("upload_workspace_image", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn create_workspace(
        &self,
        request: wire::CreateWorkspaceRequest,
    ) -> Result<Response<wire::Workspace>, Status> {
        let response: json::Workspace = self.call("create_workspace", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn update_workspace(
        &self,
        request: wire::UpdateWorkspaceRequest,
    ) -> Result<Response<wire::Workspace>, Status> {
        let response: json::Workspace = self.call("update_workspace", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn set_workspace_state(
        &self,
        request: wire::SetWorkspaceStateRequest,
    ) -> Result<Response<wire::Workspace>, Status> {
        let response: json::Workspace = self.call("set_workspace_state", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn list_agents(
        &self,
        request: wire::ListAgentsRequest,
    ) -> Result<Response<wire::ListAgentsResponse>, Status> {
        let response: json::ListAgentsResponse = self.call("list_agents", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_agent(
        &self,
        request: wire::GetAgentRequest,
    ) -> Result<Response<wire::Agent>, Status> {
        let response: json::Agent = self.call("get_agent", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn cancel_attempt(
        &self,
        request: wire::CancelAttemptRequest,
    ) -> Result<Response<wire::CancelAttemptResponse>, Status> {
        let response: json::CancelAttemptResponse = self.call("cancel_attempt", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_execution_details(
        &self,
        request: wire::GetExecutionDetailsRequest,
    ) -> Result<Response<wire::ExecutionDetails>, Status> {
        let response: json::ExecutionDetails = self.call("get_execution_details", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_agent_avatar(
        &self,
        request: wire::GetAgentAvatarRequest,
    ) -> Result<Response<wire::GetAgentAvatarResponse>, Status> {
        let response: json::GetAgentAvatarResponse =
            self.call("get_agent_avatar", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn create_agent(
        &self,
        request: wire::CreateAgentRequest,
    ) -> Result<Response<wire::Agent>, Status> {
        let response: json::Agent = self.call("create_agent", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn update_agent(
        &self,
        request: wire::UpdateAgentRequest,
    ) -> Result<Response<wire::Agent>, Status> {
        let response: json::Agent = self.call("update_agent", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn resume_agent(
        &self,
        request: wire::GetAgentRequest,
    ) -> Result<Response<wire::Agent>, Status> {
        let response: json::Agent = self.call("resume_agent", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn delete_agent(
        &self,
        request: wire::DeleteAgentRequest,
    ) -> Result<Response<wire::DeleteAgentResponse>, Status> {
        let response: json::DeleteAgentResponse = self.call("delete_agent", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_provider_onboarding(
        &self,
        request: wire::GetProviderOnboardingRequest,
    ) -> Result<Response<wire::ProviderOnboardingState>, Status> {
        let response: json::ProviderOnboardingState =
            self.call("get_provider_onboarding", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn advance_provider_onboarding(
        &self,
        request: wire::AdvanceProviderOnboardingRequest,
    ) -> Result<Response<wire::ProviderOnboardingState>, Status> {
        let response: json::ProviderOnboardingState =
            self.call("advance_provider_onboarding", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_agent_onboarding(
        &self,
        request: wire::GetAgentOnboardingRequest,
    ) -> Result<Response<wire::AgentOnboardingState>, Status> {
        let response: json::AgentOnboardingState =
            self.call("get_agent_onboarding", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn advance_agent_onboarding(
        &self,
        request: wire::AdvanceAgentOnboardingRequest,
    ) -> Result<Response<wire::AgentOnboardingState>, Status> {
        let response: json::AgentOnboardingState =
            self.call("advance_agent_onboarding", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_provider_catalog(
        &self,
        request: wire::CatalogRequest,
    ) -> Result<Response<wire::ProviderCatalog>, Status> {
        let response: json::ProviderCatalog = self.call("get_provider_catalog", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_computer_catalog(
        &self,
        request: wire::CatalogRequest,
    ) -> Result<Response<wire::ComputerCatalog>, Status> {
        let response: json::ComputerCatalog = self.call("get_computer_catalog", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn create_pairing_code(
        &self,
        request: wire::CatalogRequest,
    ) -> Result<Response<wire::PairingCode>, Status> {
        let response: json::PairingCode = self.call("create_pairing_code", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn rename_computer(
        &self,
        request: wire::RenameComputerRequest,
    ) -> Result<Response<wire::ComputerCatalog>, Status> {
        let response: json::ComputerCatalog = self.call("rename_computer", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn revoke_computer(
        &self,
        request: wire::RevokeComputerRequest,
    ) -> Result<Response<wire::ComputerCatalog>, Status> {
        let response: json::ComputerCatalog = self.call("revoke_computer", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_harness_catalog(
        &self,
        request: wire::CatalogRequest,
    ) -> Result<Response<wire::HarnessCatalog>, Status> {
        let response: json::HarnessCatalog = self.call("get_harness_catalog", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_model_catalog(
        &self,
        request: wire::ModelCatalogRequest,
    ) -> Result<Response<wire::ModelCatalog>, Status> {
        let response: json::ModelCatalog = self.call("get_model_catalog", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn get_reaction_catalog(
        &self,
        request: wire::CatalogRequest,
    ) -> Result<Response<wire::ReactionCatalog>, Status> {
        let response: json::ReactionCatalog = self.call("get_reaction_catalog", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn list_provider_interfaces(
        &self,
        request: wire::ListProviderInterfacesRequest,
    ) -> Result<Response<wire::ListProviderInterfacesResponse>, Status> {
        let response: json::ListProviderInterfacesResponse =
            self.call("list_provider_interfaces", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn invoke_provider_interface(
        &self,
        request: wire::InvokeProviderInterfaceRequest,
    ) -> Result<Response<wire::InvokeProviderInterfaceResponse>, Status> {
        let response: json::InvokeProviderInterfaceResponse =
            self.call("invoke_provider_interface", &request).await?;
        Ok(Response::new(response))
    }
    pub async fn subscribe_management_events(
        &self,
        request: wire::ManagementSubscription,
    ) -> Result<Response<ManagementMessages>, Status> {
        use tokio_tungstenite::tungstenite::{
            Message, client::IntoClientRequest, protocol::WebSocketConfig,
        };
        let (url, token) = self.endpoint()?;
        let mut handshake = format!(
            "{}{}",
            url.replacen("http://", "ws://", 1),
            json::EVENTS_PATH
        )
        .into_client_request()
        .map_err(|e| Status::internal(e.to_string()))?;
        handshake.headers_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| Status::internal("invalid credential"))?,
        );
        handshake.headers_mut().insert(
            "x-lankide-subscription",
            serde_json::to_string(&request)
                .map_err(|e| Status::internal(e.to_string()))?
                .parse()
                .map_err(|_| Status::invalid_argument("invalid subscription"))?,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(self.limit))
            .max_frame_size(Some(self.limit));
        let (socket, _) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio_tungstenite::connect_async_with_config(handshake, Some(config), false),
        )
        .await
        .map_err(|_| Status::unavailable("event handshake timed out"))?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                if let Some(error) = response
                    .body()
                    .as_ref()
                    .and_then(|body| serde_json::from_slice::<json::ErrorBody>(body).ok())
                {
                    error.into_status()
                } else {
                    match response.status().as_u16() {
                        401 => Status::unauthenticated("owner credential required"),
                        403 => Status::permission_denied("management access denied"),
                        400 => Status::invalid_argument("invalid event subscription"),
                        _ => Status::unavailable(format!(
                            "event upgrade refused: {}",
                            response.status()
                        )),
                    }
                }
            }
            error => Status::unavailable(error.to_string()),
        })?;
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            let mut socket = socket;
            loop {
                tokio::select! {
                    _ = sender.closed() => { let _ = socket.close(None).await; break; }
                    frame = socket.next() => {
                        let event = match frame {
                            Some(Ok(Message::Text(text))) => match serde_json::from_str::<json::EventFrame>(&text) {
                                Ok(json::EventFrame::Event(event)) => Ok(event),
                                Ok(json::EventFrame::Error(error)) => Err(error.into_status()),
                                Err(error) => Err(Status::internal(error.to_string())),
                            },
                            Some(Ok(Message::Ping(bytes))) => { if socket.send(Message::Pong(bytes)).await.is_err() { break; } continue; }
                            Some(Ok(Message::Pong(_))) => continue,
                            Some(Ok(Message::Close(_))) | None => break,
                            Some(Err(error)) => Err(Status::unavailable(error.to_string())),
                            _ => Err(Status::internal("invalid management event frame")),
                        };
                        let failed = event.is_err();
                        if sender.send(event).await.is_err() || failed { break; }
                    }
                }
            }
        });
        Ok(Response::new(ManagementMessages {
            inner: Box::pin(tokio_stream::wrappers::ReceiverStream::new(receiver)),
        }))
    }
}
pub struct ManagementMessages {
    inner: Pin<Box<dyn Stream<Item = Result<wire::ManagementEvent, Status>> + Send>>,
}
impl std::fmt::Debug for ManagementMessages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagementMessages")
    }
}
impl ManagementMessages {
    pub async fn message(&mut self) -> Result<Option<wire::ManagementEvent>, Status> {
        self.inner.next().await.transpose()
    }
}
impl Stream for ManagementMessages {
    type Item = Result<wire::ManagementEvent, Status>;
    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}
