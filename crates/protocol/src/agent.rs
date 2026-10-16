//! Agent capability transport shared by the private local proxy and Machine link.
use crate::Status;
use crate::*;
use serde::{Serialize, de::DeserializeOwned};
use tokio_stream::StreamExt;

pub const CHUNK_BYTES: usize = 32 * 1024;
pub const MAX_CALL_BYTES: usize = 8 * 1024 * 1024;
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[async_trait::async_trait]
pub trait CapabilityTransport: Send + Sync + 'static {
    async fn call(
        &self,
        method: &str,
        context: AgentRunContext,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Status>;
}
/// Only Agent operations are implemented; management is unavailable on this socket.
pub struct AgentProxy<T>(pub T);
#[async_trait::async_trait]
impl<T: CapabilityTransport> RuntimePeer for AgentProxy<T> {
    async fn list_capabilities(&self, request: AgentRunContext) -> Result<CapabilityList, Status> {
        let context = request.clone();
        let payload = self
            .0
            .call("list_capabilities", context, encode(&request)?)
            .await?;
        decode::<CapabilityList>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn get_self_profile(&self, request: AgentRunContext) -> Result<AgentProfile, Status> {
        let context = request.clone();
        let payload = self
            .0
            .call("get_self_profile", context, encode(&request)?)
            .await?;
        decode::<AgentProfile>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn update_self_profile(
        &self,
        request: UpdateSelfProfileRequest,
    ) -> Result<AgentProfile, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("update_self_profile", context, encode(&request)?)
            .await?;
        decode::<AgentProfile>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn invoke_capability(
        &self,
        request: CapabilityInvokeRequest,
    ) -> Result<CapabilityInvokeResult, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("invoke_capability", context, encode(&request)?)
            .await?;
        decode::<CapabilityInvokeResult>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn send_message(&self, request: SendMessageRequest) -> Result<SendMessageResult, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("send_message", context, encode(&request)?)
            .await?;
        decode::<SendMessageResult>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn get_content(&self, request: ContentRequest) -> Result<ContentResult, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("get_content", context, encode(&request)?)
            .await?;
        decode::<ContentResult>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn upload_resource(
        &self,
        mut chunks: ResourceChunkStream,
    ) -> Result<ResourceRef, Status> {
        let mut collected = Vec::new();
        let mut payload_size = 0;
        let mut context = None;
        let deadline = tokio::time::Instant::now() + CALL_TIMEOUT;
        while let Some(chunk) = tokio::time::timeout_at(deadline, chunks.next())
            .await
            .map_err(|_| Status::deadline_exceeded("capability_timeout"))?
        {
            let chunk = chunk?;
            if context.is_none() {
                context = chunk.descriptor.as_ref().and_then(|d| d.context.clone());
            }
            payload_size += encode(&chunk)?.len();
            collected.push(chunk);
            if payload_size > MAX_CALL_BYTES {
                return Err(Status::resource_exhausted("capability_payload_too_large"));
            }
        }
        let context =
            context.ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("upload_resource", context, encode(&collected)?)
            .await?;
        decode::<ResourceRef>(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn download_resource(
        &self,
        request: ResourceRequest,
    ) -> Result<ResourceChunkStream, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("download_resource", context, encode(&request)?)
            .await?;
        Ok(Box::pin(tokio_stream::iter(
            decode_chunks(&payload)?.into_iter().map(Ok),
        )))
    }
}
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, Status> {
    serde_json::to_vec(value).map_err(|_| Status::internal("encode_capability_json"))
}
pub fn decode<T: DeserializeOwned>(payload: &[u8]) -> Result<T, Status> {
    serde_json::from_slice(payload).map_err(|_| Status::invalid_argument("invalid_capability_json"))
}
pub fn decode_chunks(payload: &[u8]) -> Result<Vec<ResourceChunk>, Status> {
    decode(payload)
}
fn check_context(
    actual: Option<&AgentRunContext>,
    expected: &AgentRunContext,
) -> Result<(), Status> {
    if actual != Some(expected) {
        return Err(Status::unauthenticated("execution_context_mismatch"));
    }
    Ok(())
}
/// Dispatch through the existing public peer API, preserving its token validation.
pub async fn dispatch<P: RuntimePeer>(
    peer: &P,
    method: &str,
    payload: &[u8],
    context: &AgentRunContext,
) -> Result<Vec<u8>, Status> {
    match method {
        "list_capabilities" => {
            let request = decode::<AgentRunContext>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(Some(&request), context)?;
            encode(&peer.list_capabilities(request).await?)
        }
        "get_self_profile" => {
            let request = decode::<AgentRunContext>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(Some(&request), context)?;
            encode(&peer.get_self_profile(request).await?)
        }
        "update_self_profile" => {
            let request = decode::<UpdateSelfProfileRequest>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            encode(&peer.update_self_profile(request).await?)
        }
        "invoke_capability" => {
            let request = decode::<CapabilityInvokeRequest>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            encode(&peer.invoke_capability(request).await?)
        }
        "send_message" => {
            let request = decode::<SendMessageRequest>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            encode(&peer.send_message(request).await?)
        }
        "get_content" => {
            let request = decode::<ContentRequest>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            encode(&peer.get_content(request).await?)
        }
        "upload_resource" => {
            let chunks = decode_chunks(payload)?;
            let first = chunks
                .first()
                .and_then(|c| c.descriptor.as_ref())
                .and_then(|d| d.context.as_ref());
            check_context(first, context)?;
            encode(
                &peer
                    .upload_resource(Box::pin(tokio_stream::iter(chunks.into_iter().map(Ok))))
                    .await?,
            )
        }
        "download_resource" => {
            let request = decode::<ResourceRequest>(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            let mut stream = peer.download_resource(request).await?;
            let mut collected = Vec::new();
            let mut payload_size = 0;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                payload_size += encode(&chunk)?.len();
                if payload_size > MAX_CALL_BYTES {
                    return Err(Status::resource_exhausted("capability_payload_too_large"));
                }
                collected.push(chunk);
            }
            encode(&collected)
        }
        _ => Err(Status::unimplemented("unknown_capability_method")),
    }
}
