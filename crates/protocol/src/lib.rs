#![forbid(unsafe_code)]

//! The single versioned gRPC contract shared by Runtime, Computer, and clients.
//!
//! This crate owns wire data and Unix-domain-socket transport only. Runtime and
//! Computer business behavior is supplied through [`RuntimePeer`].

mod peer;
mod service;
mod transport;
mod validation;

pub const PROTOCOL_VERSION: u32 = 1;
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WORKSPACE_IMAGE_BYTES: usize = 3 * 1024 * 1024;
pub const PROVIDER_CAPABILITY_TASK_SYSTEM: &str = "task_system";

pub mod wire {
    tonic::include_proto!("lankide.protocol.v1");
}

pub use wire::runtime_protocol_client::RuntimeProtocolClient;
pub use wire::runtime_protocol_server::{RuntimeProtocol, RuntimeProtocolServer};
pub use wire::*;

pub use peer::{
    ComputerFrameStream, ManagementEventStream, ResourceChunkStream, RuntimeFrameStream,
    RuntimePeer,
};
pub use service::ValidatedRuntimeService;
pub use transport::{
    TransportConfig, TransportError, connect_uds, connect_uds_lazy, serve_uds, socket_path,
};
pub use validation::{validate_computer_hello, validate_runtime_frame};
