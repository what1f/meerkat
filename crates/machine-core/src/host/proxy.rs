use super::{Inner, MachineError};
use lankide_protocol::Status;
use lankide_protocol::{
    agent::{self, AgentProxy, CapabilityTransport},
    *,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{oneshot, watch};

pub(super) struct PendingCall {
    pub bytes: Vec<u8>,
    pub response: oneshot::Sender<Result<Vec<u8>, Status>>,
}
// Remove the correlation entry even when a caller drops its socket mid-call.
struct PendingGuard {
    inner: std::sync::Weak<Inner>,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            let id = self.id.clone();
            tokio::spawn(async move {
                inner.capability_calls.lock().await.remove(&id);
            });
        }
    }
}
pub(super) struct LocalTransport {
    agent_home: Option<PathBuf>,
    pub(super) attachment_lock: tokio::sync::Mutex<()>,
    inner: Arc<Inner>,
    context: AgentRunContext,
    stopped: watch::Receiver<bool>,
}
#[async_trait::async_trait]
impl CapabilityTransport for LocalTransport {
    async fn call(
        &self,
        method: &str,
        context: AgentRunContext,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Status> {
        if context != self.context || *self.stopped.borrow() {
            return Err(Status::unauthenticated("attempt_finished_or_invalid"));
        }
        if payload.len() > agent::MAX_CALL_BYTES {
            return Err(Status::resource_exhausted("capability_payload_too_large"));
        }
        let payload = if let Some(home) = &self.agent_home {
            super::files::prepare(self, method, &context, home, payload).await?
        } else {
            payload
        };
        self.forward(method, context, payload).await
    }
}
impl LocalTransport {
    pub(super) async fn cached_upload(&self, key: &str) -> Option<ResourceRef> {
        let execution = self.context.execution.as_ref()?;
        self.inner
            .state
            .lock()
            .await
            .uploaded_resources
            .get(&crate::state::assignment_key(
                execution.execution_id,
                execution.attempt,
            ))
            .and_then(|entries| entries.get(key))
            .cloned()
    }
    pub(super) async fn remember_upload(
        &self,
        key: String,
        resource: ResourceRef,
    ) -> Result<(), Status> {
        let execution = self
            .context
            .execution
            .as_ref()
            .ok_or_else(|| Status::unauthenticated("missing_execution"))?;
        let mut state = self.inner.state.lock().await;
        state
            .uploaded_resources
            .entry(crate::state::assignment_key(
                execution.execution_id,
                execution.attempt,
            ))
            .or_default()
            .insert(key, resource);
        state
            .save(&self.inner.state_path)
            .map_err(|_| Status::internal("persist_attachment"))
    }

    pub(super) async fn forward(
        &self,
        method: &str,
        context: AgentRunContext,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Status> {
        if payload.len() > agent::MAX_CALL_BYTES {
            return Err(Status::resource_exhausted("capability_payload_too_large"));
        }
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = format!(
            "{}-{}",
            self.inner.config.instance_id,
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let _pending = PendingGuard {
            inner: Arc::downgrade(&self.inner),
            id: id.clone(),
        };
        let mut stopped = self.stopped.clone();
        let result = tokio::time::timeout(agent::CALL_TIMEOUT, async {
            let sender = self
                .inner
                .connection
                .read()
                .await
                .clone()
                .ok_or_else(|| Status::unavailable("machine_link_down"))?;
            let (tx, rx) = oneshot::channel();
            {
                let mut pending = self.inner.capability_calls.lock().await;
                if pending.len() >= 16 {
                    return Err(Status::resource_exhausted("too_many_capability_calls"));
                }
                pending.insert(
                    id.clone(),
                    PendingCall {
                        bytes: Vec::new(),
                        response: tx,
                    },
                );
            }
            let count = payload.len().max(1).div_ceil(agent::CHUNK_BYTES);
            for index in 0..count {
                let offset = index * agent::CHUNK_BYTES;
                sender
                    .send(MachineFrame {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        frame: Some(machine_frame::Frame::CapabilityRequest(CapabilityRequest {
                            request_id: id.clone(),
                            method: method.to_owned(),
                            context: Some(context.clone()),
                            payload: payload
                                [offset..payload.len().min(offset + agent::CHUNK_BYTES)]
                                .to_vec(),
                            offset: offset as u64,
                            eof: index + 1 == count,
                        })),
                    })
                    .await
                    .map_err(|_| Status::unavailable("machine_link_down"))?;
            }
            rx.await
                .map_err(|_| Status::unavailable("machine_link_down"))?
        });
        let response = tokio::select! {
            _ = stopped.changed() => Err(Status::unauthenticated("attempt_finished")),
            response = result => response.unwrap_or_else(|_| Err(Status::deadline_exceeded("capability_timeout"))),
        };
        self.inner.capability_calls.lock().await.remove(&id);
        response
    }
}
pub(super) struct AttemptProxy {
    pub path: PathBuf,
    stopped: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl AttemptProxy {
    pub fn start(
        inner: Arc<Inner>,
        context: AgentRunContext,
        agent_home: PathBuf,
    ) -> Result<Self, MachineError> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let execution = context
            .execution
            .as_ref()
            .ok_or_else(|| MachineError::InvalidAssignment("missing_execution".into()))?;
        let path = attempt_socket(&inner, execution);
        let directory = path.parent().expect("attempt directory");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        // A stale socket can only belong to this exact attempt in our private run directory.
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let (stopped, rx) = watch::channel(false);
        let transport = LocalTransport {
            agent_home: Some(agent_home),
            attachment_lock: tokio::sync::Mutex::new(()),
            inner,
            context,
            stopped: rx,
        };
        let task = tokio::spawn(async move {
            let _ = serve_agent_listener(
                listener,
                AgentProxy(transport),
                TransportConfig::default(),
                std::future::pending(),
            )
            .await;
        });
        Ok(Self {
            path,
            stopped,
            task,
        })
    }
}
impl Drop for AttemptProxy {
    fn drop(&mut self) {
        self.stopped.send_replace(true);
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}
pub(super) async fn response(inner: &Inner, chunk: CapabilityResponse) {
    let finished = {
        let mut calls = inner.capability_calls.lock().await;
        let Some(call) = calls.get_mut(&chunk.request_id) else {
            return;
        };
        let error = if chunk.error_code != 0 {
            Some(Status::new(
                lankide_protocol::Code::from_i32(chunk.error_code),
                chunk.error_message,
            ))
        } else if chunk.offset != call.bytes.len() as u64
            || chunk.payload.len() > agent::CHUNK_BYTES
            || call.bytes.len() + chunk.payload.len() > agent::MAX_CALL_BYTES
        {
            Some(Status::data_loss("invalid_capability_response"))
        } else {
            None
        };
        if let Some(error) = error {
            calls
                .remove(&chunk.request_id)
                .map(|call| (call.response, Err(error)))
        } else {
            call.bytes.extend_from_slice(&chunk.payload);
            if chunk.eof {
                calls
                    .remove(&chunk.request_id)
                    .map(|call| (call.response, Ok(call.bytes)))
            } else {
                None
            }
        }
    };
    if let Some((target, result)) = finished {
        let _ = target.send(result);
    }
}
pub(super) async fn disconnected(inner: &Inner) {
    for (_, call) in std::mem::take(&mut *inner.capability_calls.lock().await) {
        let _ = call
            .response
            .send(Err(Status::unavailable("machine_link_down")));
    }
}
/// Native MCP and Pi calls enter through the public Harness event contract.
/// Their bridge on the Machine uses the same attempt socket as the CLI.
pub(super) async fn harness_call(
    inner: Arc<Inner>,
    context: AgentRunContext,
    call_id: String,
    name: String,
    arguments: String,
) {
    let Some(execution) = context.execution.clone() else {
        return;
    };
    let path = attempt_socket(&inner, &execution);
    tokio::spawn(async move {
        let result = tokio::time::timeout(agent::CALL_TIMEOUT, async {
            let mut client = connect_uds(path, DEFAULT_MAX_MESSAGE_BYTES)
                .await
                .map_err(|_| Status::unavailable("agent_socket_unavailable"))?;
            Ok::<_, Status>(
                client
                    .invoke_capability(CapabilityInvokeRequest {
                        context: Some(context),
                        capability_name: name,
                        arguments_json: arguments,
                        invocation_id: call_id.clone(),
                    })
                    .await?
                    .into_inner(),
            )
        })
        .await
        .unwrap_or_else(|_| Err(Status::deadline_exceeded("capability_timeout")));
        let result = match result {
            Ok(result) => ToolResult {
                execution: Some(execution),
                tool_call_id: call_id,
                success: result.success,
                content_markdown: result.content_markdown,
                structured_content_json: result.structured_content_json,
                resources: result.resources,
                error: result.error,
            },
            Err(error) => ToolResult {
                execution: Some(execution),
                tool_call_id: call_id,
                success: false,
                structured_content_json: "{}".into(),
                error: Some(ToolError {
                    code: format!("{:?}", error.code()).to_lowercase(),
                    message: error.message().into(),
                    retryable: matches!(
                        error.code(),
                        lankide_protocol::Code::Unavailable
                            | lankide_protocol::Code::DeadlineExceeded
                    ),
                }),
                ..Default::default()
            },
        };
        super::control::route_tool_result(&inner, result).await;
    });
}
fn attempt_socket(inner: &Inner, execution: &ExecutionRef) -> PathBuf {
    inner
        .root
        .join(".machine/run")
        .join(format!("{}-{}", execution.execution_id, execution.attempt))
        .join("agent.sock")
}

/// Secure listeners expose only the Machine link. Reuse its existing resource
/// capability for assignment and Tool-result attachments, with the same local
/// path and integrity checks as loopback HTTP downloads.
pub(super) async fn download_resource(
    inner: Arc<Inner>,
    resource: &ResourceRef,
    context: AgentRunContext,
    directory: &std::path::Path,
) -> Result<PathBuf, MachineError> {
    if !inner
        .link_endpoint
        .as_ref()
        .is_some_and(|(url, _)| url.starts_with("wss://"))
    {
        return crate::resources::download_resource(
            &inner.resource_http,
            &inner.runtime_run,
            inner.link_endpoint.as_ref().map(|(url, _)| url.as_str()),
            resource,
            context,
            directory,
        )
        .await;
    }
    use tokio_stream::StreamExt as _;
    let (path, temporary) = crate::resources::download_paths(resource, directory)?;
    let (_stop, stopped) = watch::channel(false);
    let peer = AgentProxy(LocalTransport {
        agent_home: None,
        attachment_lock: tokio::sync::Mutex::new(()),
        inner,
        context: context.clone(),
        stopped,
    });
    let mut chunks = peer
        .download_resource(ResourceRequest {
            resource_id: resource.resource_id,
            transfer_id: uuid::Uuid::new_v4().to_string(),
            context: Some(context),
        })
        .await
        .map_err(MachineError::Rpc)?;
    let mut file = std::fs::File::create(&temporary)?;
    let mut size = 0;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(MachineError::Rpc)?;
        if chunk.offset != size || size + chunk.bytes.len() as u64 > resource.size_bytes {
            drop(file);
            let _ = std::fs::remove_file(&temporary);
            return Err(MachineError::ResourceIntegrityMismatch);
        }
        std::io::Write::write_all(&mut file, &chunk.bytes)?;
        size += chunk.bytes.len() as u64;
    }
    file.sync_all()?;
    drop(file);
    crate::resources::finish_download(resource, &temporary, &path)
}
