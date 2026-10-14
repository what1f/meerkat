//! Capability calls received on an authenticated Computer connection.
use crate::{RuntimeApplication, RuntimePeerAdapter};
use lankide_protocol::{agent, *};
use std::sync::Arc;
use std::{collections::HashMap, time::Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tonic::Status;

pub(crate) struct CapabilityCalls {
    slots: Arc<Semaphore>,
    pending: HashMap<String, (Instant, CapabilityRequest)>,
}
impl Default for CapabilityCalls {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            slots: Arc::new(Semaphore::new(16)),
        }
    }
}
impl CapabilityCalls {
    fn ready(
        &self,
        request: CapabilityRequest,
    ) -> Result<Option<(CapabilityRequest, OwnedSemaphorePermit)>, Status> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too_many_capability_calls"))?;
        Ok(Some((request, permit)))
    }

    pub(crate) fn accept(
        &mut self,
        mut chunk: CapabilityRequest,
    ) -> Result<Option<(CapabilityRequest, OwnedSemaphorePermit)>, Status> {
        self.pending
            .retain(|_, (at, _)| at.elapsed() < agent::CALL_TIMEOUT);
        if chunk.request_id.is_empty() || chunk.payload.len() > agent::CHUNK_BYTES {
            return Err(Status::invalid_argument("invalid_capability_chunk"));
        }
        if chunk.offset == 0 {
            if self.pending.len() >= 16 || self.pending.contains_key(&chunk.request_id) {
                return Err(Status::resource_exhausted("too_many_capability_calls"));
            }
            if chunk.eof {
                return self.ready(chunk);
            }
            self.pending
                .insert(chunk.request_id.clone(), (Instant::now(), chunk));
            return Ok(None);
        }
        let (_, request) = self
            .pending
            .get_mut(&chunk.request_id)
            .ok_or_else(|| Status::invalid_argument("unknown_capability_call"))?;
        if chunk.offset != request.payload.len() as u64
            || chunk.method != request.method
            || chunk.context != request.context
        {
            self.pending.remove(&chunk.request_id);
            return Err(Status::invalid_argument("invalid_capability_offset"));
        }
        if request.payload.len() + chunk.payload.len() > agent::MAX_CALL_BYTES {
            self.pending.remove(&chunk.request_id);
            return Err(Status::resource_exhausted("capability_payload_too_large"));
        }
        request.payload.append(&mut chunk.payload);
        if chunk.eof {
            if let Some((_, request)) = self.pending.remove(&chunk.request_id) {
                return self.ready(request);
            }
        }
        Ok(None)
    }
}
pub(crate) async fn respond(
    application: RuntimeApplication,
    computer: String,
    instance: String,
    request: CapabilityRequest,
) {
    let result = tokio::time::timeout(agent::CALL_TIMEOUT, async {
        let context = request
            .context
            .as_ref()
            .ok_or_else(|| Status::unauthenticated("missing_execution_context"))?;
        agent::dispatch(
            &RuntimePeerAdapter::new(application.clone()),
            &request.method,
            &request.payload,
            context,
        )
        .await
    })
    .await
    .unwrap_or_else(|_| Err(Status::deadline_exceeded("capability_timeout")));
    let (payload, error_code, error_message) = match result {
        Ok(bytes) if bytes.len() <= agent::MAX_CALL_BYTES => (bytes, 0, String::new()),
        Ok(_) => (
            Vec::new(),
            tonic::Code::ResourceExhausted as i32,
            "capability_payload_too_large".into(),
        ),
        Err(error) => (Vec::new(), error.code() as i32, error.message().to_owned()),
    };
    let count = payload.len().max(1).div_ceil(agent::CHUNK_BYTES);
    for index in 0..count {
        let offset = index * agent::CHUNK_BYTES;
        let response = CapabilityResponse {
            request_id: request.request_id.clone(),
            payload: payload[offset..payload.len().min(offset + agent::CHUNK_BYTES)].to_vec(),
            offset: offset as u64,
            eof: index + 1 == count,
            error_code,
            error_message: error_message.clone(),
        };
        if application
            .computer_broker()
            .send_frame(
                &computer,
                &instance,
                RuntimeFrame {
                    frame: Some(runtime_frame::Frame::CapabilityResponse(response)),
                },
            )
            .await
            .is_err()
        {
            break;
        }
    }
}
