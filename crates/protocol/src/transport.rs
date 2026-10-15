//! Owner-local, length-prefixed JSON transport for Agent capabilities.
use crate::{agent, *};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fmt,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};
use tokio_stream::{Stream, StreamExt};

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
pub struct TransportError(pub std::io::Error);
impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "agent_socket: {}", self.0)
    }
}
impl std::error::Error for TransportError {}
impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        Self(value)
    }
}

#[derive(Serialize, Deserialize)]
struct Call {
    protocol_version: u32,
    method: String,
    context: AgentRunContext,
    payload: serde_json::Value,
}
type Reply = Result<serde_json::Value, Status>;
fn io_status(error: std::io::Error) -> Status {
    Status::unavailable(error.to_string())
}
async fn read_json<T: DeserializeOwned>(
    stream: &mut UnixStream,
    limit: usize,
) -> Result<T, Status> {
    let length = stream.read_u32().await.map_err(io_status)? as usize;
    if length > limit {
        return Err(Status::resource_exhausted("agent_message_too_large"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await.map_err(io_status)?;
    serde_json::from_slice(&bytes).map_err(|_| Status::invalid_argument("invalid_agent_json"))
}
async fn write_json<T: Serialize>(
    stream: &mut UnixStream,
    value: &T,
    limit: usize,
) -> Result<(), Status> {
    let bytes = agent::encode(value)?;
    if bytes.len() > limit {
        return Err(Status::resource_exhausted("agent_message_too_large"));
    }
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(io_status)?;
    stream.write_all(&bytes).await.map_err(io_status)
}
/// Serve an already bound, private attempt socket. Dropping this future closes all calls.
pub async fn serve_agent_listener<P, F>(
    listener: UnixListener,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), TransportError>
where
    P: RuntimePeer,
    F: Future<Output = ()> + Send,
{
    let peer = Arc::new(ValidatedRuntimeService::new(peer, config));
    let slots = Arc::new(tokio::sync::Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            accepted = listener.accept() => {
                let (mut stream, _) = accepted?;
                let Ok(permit) = slots.clone().try_acquire_owned() else { continue };
                let peer = peer.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let work = async {
                        let result: Reply = async {
                            let call: Call = read_json(&mut stream, agent::MAX_CALL_BYTES).await?;
                            if call.protocol_version != config.protocol_version { return Err(Status::failed_precondition("protocol_version_unsupported")); }
                            let payload = agent::encode(&call.payload)?;
                            if call.method == "upload_resource" {
                                for chunk in agent::decode_chunks(&payload)? {
                                    if chunk.bytes.len() > config.max_message_bytes { return Err(Status::resource_exhausted("agent_message_too_large")); }
                                }
                            } else if payload.len() > config.max_message_bytes {
                                return Err(Status::resource_exhausted("agent_message_too_large"));
                            }
                            let response = agent::dispatch(peer.as_ref(), &call.method, &payload, &call.context).await?;
                            if call.method != "download_resource" && response.len() > config.max_message_bytes {
                                return Err(Status::resource_exhausted("agent_message_too_large"));
                            }
                            agent::decode(&response)
                        }.await;
                        let _ = write_json(&mut stream, &result, agent::MAX_CALL_BYTES).await;
                    };
                    let _ = tokio::time::timeout(agent::CALL_TIMEOUT, work).await;
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}
pub async fn serve_uds<P, F>(
    path: impl AsRef<Path>,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), TransportError>
where
    P: RuntimePeer,
    F: Future<Output = ()> + Send + 'static,
{
    use std::os::unix::fs::PermissionsExt;
    let path = path.as_ref();
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("socket_path_has_no_parent"))?;
    std::fs::create_dir_all(parent)?;
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let result = serve_agent_listener(listener, peer, config, shutdown).await;
    let _ = std::fs::remove_file(path);
    result
}
#[derive(Clone, Debug)]
pub struct AgentClient {
    path: PathBuf,
    limit: usize,
}
pub async fn connect_uds(
    path: impl AsRef<Path>,
    limit: usize,
) -> Result<AgentClient, TransportError> {
    let path = path.as_ref().to_owned();
    let _ = UnixStream::connect(&path).await?;
    Ok(AgentClient { path, limit })
}
pub fn connect_uds_lazy(path: impl AsRef<Path>, limit: usize) -> AgentClient {
    AgentClient {
        path: path.as_ref().to_owned(),
        limit,
    }
}
impl AgentClient {
    async fn call<I: Serialize, O: DeserializeOwned>(
        &self,
        method: &str,
        context: AgentRunContext,
        input: &I,
    ) -> Result<Response<O>, Status> {
        tokio::time::timeout(agent::CALL_TIMEOUT, async {
            let payload =
                serde_json::to_value(input).map_err(|_| Status::internal("encode_agent_json"))?;
            if method != "upload_resource" && agent::encode(input)?.len() > self.limit {
                return Err(Status::resource_exhausted("agent_message_too_large"));
            }
            let call = Call {
                protocol_version: PROTOCOL_VERSION,
                method: method.into(),
                context,
                payload,
            };
            let mut stream = UnixStream::connect(&self.path).await.map_err(io_status)?;
            write_json(&mut stream, &call, agent::MAX_CALL_BYTES).await?;
            let reply: Reply = read_json(&mut stream, agent::MAX_CALL_BYTES).await?;
            let value = reply?;
            if method != "download_resource" && agent::encode(&value)?.len() > self.limit {
                return Err(Status::resource_exhausted("agent_message_too_large"));
            }
            serde_json::from_value(value)
                .map(Response::new)
                .map_err(|_| Status::data_loss("invalid_agent_response"))
        })
        .await
        .map_err(|_| Status::deadline_exceeded("capability_timeout"))?
    }
    pub async fn list_capabilities(
        &mut self,
        request: AgentRunContext,
    ) -> Result<Response<CapabilityList>, Status> {
        self.call("list_capabilities", request.clone(), &request)
            .await
    }
    pub async fn get_self_profile(
        &mut self,
        request: AgentRunContext,
    ) -> Result<Response<AgentProfile>, Status> {
        self.call("get_self_profile", request.clone(), &request)
            .await
    }
    pub async fn upload_resource(
        &mut self,
        chunks: impl Stream<Item = ResourceChunk> + Unpin,
    ) -> Result<Response<ResourceRef>, Status> {
        let mut chunks = chunks;
        let mut all = Vec::new();
        let mut size = 0;
        let deadline = tokio::time::Instant::now() + agent::CALL_TIMEOUT;
        while let Some(chunk) = tokio::time::timeout_at(deadline, chunks.next())
            .await
            .map_err(|_| Status::deadline_exceeded("capability_timeout"))?
        {
            if chunk.bytes.len() > self.limit {
                return Err(Status::resource_exhausted("agent_message_too_large"));
            }
            size += agent::encode(&chunk)?.len();
            if size > agent::MAX_CALL_BYTES {
                return Err(Status::resource_exhausted("capability_payload_too_large"));
            }
            all.push(chunk);
        }
        let context = all
            .first()
            .and_then(|c| c.descriptor.as_ref())
            .and_then(|d| d.context.clone())
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        self.call("upload_resource", context, &all).await
    }
    pub async fn download_resource(
        &mut self,
        request: ResourceRequest,
    ) -> Result<Response<Messages<ResourceChunk>>, Status> {
        let context = request
            .context
            .clone()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        let values: Response<Vec<ResourceChunk>> =
            self.call("download_resource", context, &request).await?;
        let values = values.into_inner();
        if values.iter().any(|chunk| chunk.bytes.len() > self.limit) {
            return Err(Status::resource_exhausted("agent_message_too_large"));
        }
        Ok(Response::new(Messages(tokio_stream::iter(
            values.into_iter().map(Ok).collect::<Vec<_>>(),
        ))))
    }
}
macro_rules! agent_methods { ($($method:ident($input:ty) -> $output:ty),* $(,)?) => { impl AgentClient { $(pub async fn $method(&mut self, request: $input) -> Result<Response<$output>, Status> { let context = request.context.clone().ok_or_else(|| Status::unauthenticated("missing_execution_context"))?; self.call(stringify!($method), context, &request).await })* } }; }
agent_methods! { update_self_profile(UpdateSelfProfileRequest) -> AgentProfile, invoke_capability(CapabilityInvokeRequest) -> CapabilityInvokeResult, send_message(SendMessageRequest) -> SendMessageResult, get_content(ContentRequest) -> ContentResult }
#[derive(Debug)]
pub struct Messages<T>(tokio_stream::Iter<std::vec::IntoIter<Result<T, Status>>>);
impl<T: Unpin> Messages<T> {
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        self.next().await.transpose()
    }
}
impl<T: Unpin> Stream for Messages<T> {
    type Item = Result<T, Status>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}
