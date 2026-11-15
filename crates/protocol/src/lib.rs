#![forbid(unsafe_code)]

//! Versioned wire contracts shared by Runtime, Machine, and clients.
//!
//! Machine links use JSON/WebSocket through [`link::MachineLink`]. Machine
//! resources and management use HTTP on the same listener; management events use
//! one WebSocket. Agent capabilities use private length-prefixed JSON local endpoints through [`RuntimePeer`].

mod error;
pub use error::{Code, Request, Response, Status};
pub mod agent;
pub mod link;
pub mod local;
mod peer;
pub mod resources;
mod service;
pub mod tls;
mod transport;
mod validation;

pub const PROTOCOL_VERSION: u32 = 2;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WORKSPACE_IMAGE_BYTES: usize = 3 * 1024 * 1024;
pub use meerkat_core::PROVIDER_CAPABILITY_TASK_SYSTEM;

pub mod wire;

pub use wire::*;

pub use peer::{
    MachineFrameStream, MachinePeer, ManagementEventStream, ResourceChunkStream,
    RuntimeFrameStream, RuntimePeer,
};
pub use service::ValidatedRuntimeService;
pub use transport::{
    AgentClient, TransportConfig, TransportError, connect_local, connect_local_lazy,
    serve_agent_listener, serve_local,
};
pub use validation::{validate_machine_hello, validate_runtime_frame};

impl From<meerkat_core::ResourceRef> for ResourceRef {
    fn from(value: meerkat_core::ResourceRef) -> Self {
        Self {
            resource_id: value.resource_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}

impl From<ResourceRef> for meerkat_core::ResourceRef {
    fn from(value: ResourceRef) -> Self {
        Self {
            resource_id: value.resource_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}

pub mod management;
mod management_http;
