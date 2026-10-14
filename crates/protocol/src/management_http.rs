use crate::{RuntimePeer, TransportConfig, ValidatedRuntimeService, management as json};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode, body::Incoming};

pub(crate) fn response<T: serde::Serialize>(
    status: StatusCode,
    value: &T,
) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(Full::new(Bytes::from(
            serde_json::to_vec(value).expect("management JSON"),
        )))
        .expect("HTTP response")
}
pub(crate) fn error(status: tonic::Status) -> Response<Full<Bytes>> {
    let code = match status.code() {
        tonic::Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        tonic::Code::PermissionDenied => StatusCode::FORBIDDEN,
        tonic::Code::NotFound => StatusCode::NOT_FOUND,
        tonic::Code::AlreadyExists => StatusCode::CONFLICT,
        tonic::Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        tonic::Code::ResourceExhausted => StatusCode::PAYLOAD_TOO_LARGE,
        tonic::Code::Internal | tonic::Code::DataLoss | tonic::Code::Unknown => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    };
    response(code, &json::ErrorBody::from(status))
}
pub(crate) async fn dispatch<P: RuntimePeer>(
    request: Request<Incoming>,
    peer: P,
    config: TransportConfig,
) -> Response<Full<Bytes>> {
    if request.method() != hyper::Method::POST {
        return error(tonic::Status::invalid_argument("POST required"));
    }
    let operation = request
        .uri()
        .path()
        .trim_start_matches(json::PREFIX)
        .to_owned();
    let bytes = match http_body_util::Limited::new(
        request.into_body(),
        json::json_limit(config.max_message_bytes),
    )
    .collect()
    .await
    {
        Ok(value) => value.to_bytes(),
        Err(_) => {
            return error(tonic::Status::resource_exhausted(
                "management request too large",
            ));
        }
    };
    let service = ValidatedRuntimeService::new(peer, config);
    macro_rules! call {
        ($request:ty, $response:ty, $method:ident) => {{
            match serde_json::from_slice::<$request>(&bytes) {
                Ok(value) => match service.$method(tonic::Request::new(value.into())).await {
                    Ok(value) => response(StatusCode::OK, &<$response>::from(value.into_inner())),
                    Err(status) => error(status),
                },
                Err(_) => error(tonic::Status::invalid_argument("invalid management JSON")),
            }
        }};
    }
    match operation.as_str() {
        "list_workspaces" => call!(
            json::ListWorkspacesRequest,
            json::ListWorkspacesResponse,
            list_workspaces
        ),
        "get_workspace" => call!(json::GetWorkspaceRequest, json::Workspace, get_workspace),
        "get_workspace_image" => call!(
            json::WorkspaceImageRequest,
            json::WorkspaceImageResponse,
            get_workspace_image
        ),
        "upload_workspace_image" => call!(
            json::UploadWorkspaceImageRequest,
            json::UploadWorkspaceImageResponse,
            upload_workspace_image
        ),
        "create_workspace" => call!(
            json::CreateWorkspaceRequest,
            json::Workspace,
            create_workspace
        ),
        "update_workspace" => call!(
            json::UpdateWorkspaceRequest,
            json::Workspace,
            update_workspace
        ),
        "set_workspace_state" => call!(
            json::SetWorkspaceStateRequest,
            json::Workspace,
            set_workspace_state
        ),
        "list_agents" => call!(
            json::ListAgentsRequest,
            json::ListAgentsResponse,
            list_agents
        ),
        "get_agent" => call!(json::GetAgentRequest, json::Agent, get_agent),
        "cancel_attempt" => call!(
            json::CancelAttemptRequest,
            json::CancelAttemptResponse,
            cancel_attempt
        ),
        "get_execution_details" => call!(
            json::GetExecutionDetailsRequest,
            json::ExecutionDetails,
            get_execution_details
        ),
        "get_agent_avatar" => call!(
            json::GetAgentAvatarRequest,
            json::GetAgentAvatarResponse,
            get_agent_avatar
        ),
        "create_agent" => call!(json::CreateAgentRequest, json::Agent, create_agent),
        "update_agent" => call!(json::UpdateAgentRequest, json::Agent, update_agent),
        "resume_agent" => call!(json::GetAgentRequest, json::Agent, resume_agent),
        "delete_agent" => call!(
            json::DeleteAgentRequest,
            json::DeleteAgentResponse,
            delete_agent
        ),
        "get_provider_onboarding" => call!(
            json::GetProviderOnboardingRequest,
            json::ProviderOnboardingState,
            get_provider_onboarding
        ),
        "advance_provider_onboarding" => call!(
            json::AdvanceProviderOnboardingRequest,
            json::ProviderOnboardingState,
            advance_provider_onboarding
        ),
        "get_agent_onboarding" => call!(
            json::GetAgentOnboardingRequest,
            json::AgentOnboardingState,
            get_agent_onboarding
        ),
        "advance_agent_onboarding" => call!(
            json::AdvanceAgentOnboardingRequest,
            json::AgentOnboardingState,
            advance_agent_onboarding
        ),
        "get_provider_catalog" => call!(
            json::CatalogRequest,
            json::ProviderCatalog,
            get_provider_catalog
        ),
        "get_computer_catalog" => call!(
            json::CatalogRequest,
            json::ComputerCatalog,
            get_computer_catalog
        ),
        "create_pairing_code" => {
            call!(json::CatalogRequest, json::PairingCode, create_pairing_code)
        }
        "revoke_computer" => call!(
            json::RevokeComputerRequest,
            json::ComputerCatalog,
            revoke_computer
        ),
        "get_harness_catalog" => call!(
            json::CatalogRequest,
            json::HarnessCatalog,
            get_harness_catalog
        ),
        "get_model_catalog" => call!(
            json::ModelCatalogRequest,
            json::ModelCatalog,
            get_model_catalog
        ),
        "get_reaction_catalog" => call!(
            json::CatalogRequest,
            json::ReactionCatalog,
            get_reaction_catalog
        ),
        "list_provider_interfaces" => call!(
            json::ListProviderInterfacesRequest,
            json::ListProviderInterfacesResponse,
            list_provider_interfaces
        ),
        "invoke_provider_interface" => call!(
            json::InvokeProviderInterfaceRequest,
            json::InvokeProviderInterfaceResponse,
            invoke_provider_interface
        ),
        _ => error(tonic::Status::not_found("management route not found")),
    }
}

pub(crate) async fn serve_events(
    socket: hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>,
    mut events: crate::ManagementEventStream,
    limit: usize,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{
        Message,
        protocol::{Role, WebSocketConfig},
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(limit))
        .max_frame_size(Some(limit));
    let mut ws =
        tokio_tungstenite::WebSocketStream::from_raw_socket(socket, Role::Server, Some(config))
            .await;
    loop {
        tokio::select! {
            inbound = ws.next() => match inbound {
                Some(Ok(Message::Ping(bytes))) => { if ws.send(Message::Pong(bytes)).await.is_err() { break; } }
                Some(Ok(Message::Pong(_))) => {}
                _ => break,
            },
            event = events.next() => {
                let Some(event) = event else { let _ = ws.close(None).await; break; };
                let failed = event.is_err();
                let frame = match event {
                    Ok(event) => json::EventFrame::Event(event.into()),
                    Err(status) => json::EventFrame::Error(status.into()),
                };
                let text = serde_json::to_string(&frame).expect("management event JSON");
                if ws.send(Message::Text(text.into())).await.is_err() || failed { break; }
            }
        }
    }
}
