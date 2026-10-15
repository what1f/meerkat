#![forbid(unsafe_code)]

//! Versioned wire contracts shared by Runtime, Computer, and clients.
//!
//! Computer links use JSON/WebSocket through [`link::ComputerLink`]. Computer
//! resources and management use HTTP on the same listener; management events use
//! one WebSocket. Agent capabilities use private length-prefixed JSON sockets through [`RuntimePeer`].

mod error;
pub use error::{Code, Request, Response, Status};
pub mod agent;
pub mod link;
mod peer;
pub mod resources;
mod service;
pub mod tls;
mod transport;
mod validation;

pub const PROTOCOL_VERSION: u32 = 2;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WORKSPACE_IMAGE_BYTES: usize = 3 * 1024 * 1024;
pub use lankide_core::PROVIDER_CAPABILITY_TASK_SYSTEM;

pub mod wire;

pub use wire::*;

pub use peer::{
    ComputerFrameStream, ComputerPeer, ManagementEventStream, ResourceChunkStream,
    RuntimeFrameStream, RuntimePeer,
};
pub use service::ValidatedRuntimeService;
pub use transport::{
    AgentClient, TransportConfig, TransportError, connect_uds, connect_uds_lazy,
    serve_agent_listener, serve_uds,
};
pub use validation::{validate_computer_hello, validate_runtime_frame};

impl From<lankide_core::ResourceRef> for ResourceRef {
    fn from(value: lankide_core::ResourceRef) -> Self {
        Self {
            resource_id: value.resource_id,
            file_name: value.file_name,
            media_type: value.media_type,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}

impl From<ResourceRef> for lankide_core::ResourceRef {
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
