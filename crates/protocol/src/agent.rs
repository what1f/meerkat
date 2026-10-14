//! Agent capability transport shared by the private local proxy and Computer link.
use crate::*;
use prost::Message;
use tokio_stream::StreamExt;
use tonic::Status;

pub const CHUNK_BYTES: usize = 32 * 1024;
pub const MAX_CALL_BYTES: usize = 8 * 1024 * 1024;
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[tonic::async_trait]
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
#[tonic::async_trait]
impl<T: CapabilityTransport> RuntimePeer for AgentProxy<T> {
    async fn list_capabilities(&self, request: AgentRunContext) -> Result<CapabilityList, Status> {
        let context = request.clone();
        let payload = self
            .0
            .call("list_capabilities", context, request.encode_to_vec())
            .await?;
        CapabilityList::decode(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn get_self_profile(&self, request: AgentRunContext) -> Result<AgentProfile, Status> {
        let context = request.clone();
        let payload = self
            .0
            .call("get_self_profile", context, request.encode_to_vec())
            .await?;
        AgentProfile::decode(payload.as_slice())
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
            .call("update_self_profile", context, request.encode_to_vec())
            .await?;
        AgentProfile::decode(payload.as_slice())
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
            .call("invoke_capability", context, request.encode_to_vec())
            .await?;
        CapabilityInvokeResult::decode(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn send_message(&self, request: SendMessageRequest) -> Result<SendMessageResult, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("send_message", context, request.encode_to_vec())
            .await?;
        SendMessageResult::decode(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn get_content(&self, request: ContentRequest) -> Result<ContentResult, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self
            .0
            .call("get_content", context, request.encode_to_vec())
            .await?;
        ContentResult::decode(payload.as_slice())
            .map_err(|_| Status::data_loss("invalid_capability_response"))
    }
    async fn upload_resource(
        &self,
        mut chunks: ResourceChunkStream,
    ) -> Result<ResourceRef, Status> {
        let mut payload = Vec::new();
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
            chunk
                .encode_length_delimited(&mut payload)
                .map_err(|_| Status::internal("encode_resource"))?;
            if payload.len() > MAX_CALL_BYTES {
                return Err(Status::resource_exhausted("capability_payload_too_large"));
            }
        }
        let context =
            context.ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let payload = self.0.call("upload_resource", context, payload).await?;
        ResourceRef::decode(payload.as_slice())
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
            .call("download_resource", context, request.encode_to_vec())
            .await?;
        Ok(Box::pin(tokio_stream::iter(
            decode_chunks(&payload)?.into_iter().map(Ok),
        )))
    }
}
pub fn decode_chunks(mut payload: &[u8]) -> Result<Vec<ResourceChunk>, Status> {
    let mut chunks = Vec::new();
    while !payload.is_empty() {
        chunks.push(
            ResourceChunk::decode_length_delimited(&mut payload)
                .map_err(|_| Status::invalid_argument("invalid_resource_chunks"))?,
        );
    }
    Ok(chunks)
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
            let request = AgentRunContext::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(Some(&request), context)?;
            Ok(peer.list_capabilities(request).await?.encode_to_vec())
        }
        "get_self_profile" => {
            let request = AgentRunContext::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(Some(&request), context)?;
            Ok(peer.get_self_profile(request).await?.encode_to_vec())
        }
        "update_self_profile" => {
            let request = UpdateSelfProfileRequest::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            Ok(peer.update_self_profile(request).await?.encode_to_vec())
        }
        "invoke_capability" => {
            let request = CapabilityInvokeRequest::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            Ok(peer.invoke_capability(request).await?.encode_to_vec())
        }
        "send_message" => {
            let request = SendMessageRequest::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            Ok(peer.send_message(request).await?.encode_to_vec())
        }
        "get_content" => {
            let request = ContentRequest::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            Ok(peer.get_content(request).await?.encode_to_vec())
        }
        "upload_resource" => {
            let chunks = decode_chunks(payload)?;
            let first = chunks
                .first()
                .and_then(|c| c.descriptor.as_ref())
                .and_then(|d| d.context.as_ref());
            check_context(first, context)?;
            Ok(peer
                .upload_resource(Box::pin(tokio_stream::iter(chunks.into_iter().map(Ok))))
                .await?
                .encode_to_vec())
        }
        "download_resource" => {
            let request = ResourceRequest::decode(payload)
                .map_err(|_| Status::invalid_argument("invalid_capability_request"))?;
            check_context(request.context.as_ref(), context)?;
            let mut stream = peer.download_resource(request).await?;
            let mut payload = Vec::new();
            while let Some(chunk) = stream.next().await {
                chunk?
                    .encode_length_delimited(&mut payload)
                    .map_err(|_| Status::internal("encode_resource"))?;
                if payload.len() > MAX_CALL_BYTES {
                    return Err(Status::resource_exhausted("capability_payload_too_large"));
                }
            }
            Ok(payload)
        }
        _ => Err(Status::unimplemented("unknown_capability_method")),
    }
}
