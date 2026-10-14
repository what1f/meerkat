use crate::{
    DEFAULT_MAX_MESSAGE_BYTES, PROTOCOL_VERSION, RuntimePeer, RuntimeProtocolClient,
    RuntimeProtocolServer, ValidatedRuntimeService,
};
use hyper_util::rt::TokioIo;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tower::service_fn;

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
pub enum TransportError {
    Io(std::io::Error),
    Tonic(tonic::transport::Error),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "uds_io: {error}"),
            Self::Tonic(error) => write!(formatter, "grpc_transport: {error}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<tonic::transport::Error> for TransportError {
    fn from(value: tonic::transport::Error) -> Self {
        Self::Tonic(value)
    }
}

/// Serve the validated Runtime protocol over one real Unix domain socket.
pub async fn serve_uds<P, F>(
    socket_path: impl AsRef<Path>,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), TransportError>
where
    P: RuntimePeer,
    F: Future<Output = ()> + Send + 'static,
{
    serve_uds_inner(socket_path, peer, config, shutdown, false).await
}

/// Legacy Agent-disabled listener. Management now uses HTTP/WS, so this
/// listener has no callable RPCs. Agent RPCs use private Computer proxies.
pub async fn serve_management_uds<P, F>(
    socket_path: impl AsRef<Path>,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), TransportError>
where
    P: RuntimePeer,
    F: Future<Output = ()> + Send + 'static,
{
    serve_uds_inner(socket_path, peer, config, shutdown, true).await
}
async fn serve_uds_inner<P, F>(
    socket_path: impl AsRef<Path>,
    peer: P,
    config: TransportConfig,
    shutdown: F,
    management_only: bool,
) -> Result<(), TransportError>
where
    P: RuntimePeer,
    F: Future<Output = ()> + Send + 'static,
{
    let socket_path = socket_path.as_ref().to_path_buf();
    prepare_socket_parent(&socket_path)?;
    let listener = UnixListener::bind(&socket_path)?;
    let service = ValidatedRuntimeService::new(peer, config);
    let service = if management_only {
        service.management_only()
    } else {
        service
    };
    let service = RuntimeProtocolServer::new(service)
        .max_decoding_message_size(config.max_message_bytes)
        .max_encoding_message_size(config.max_message_bytes);
    let result = Server::builder()
        .add_service(service)
        .serve_with_incoming_shutdown(UnixListenerStream::new(listener), shutdown)
        .await;
    let _ = std::fs::remove_file(&socket_path);
    result.map_err(TransportError::from)
}

/// Connect a generated Runtime client to a Unix domain socket.
pub async fn connect_uds(
    socket_path: impl AsRef<Path>,
    max_message_bytes: usize,
) -> Result<RuntimeProtocolClient<Channel>, TransportError> {
    let path = socket_path.as_ref().to_path_buf();
    let channel = Endpoint::from_static("http://[::]:50051")
        .connect_with_connector(service_fn(move |_| {
            let path = path.clone();
            async move { UnixStream::connect(path).await.map(TokioIo::new) }
        }))
        .await?;
    Ok(RuntimeProtocolClient::new(channel)
        .max_decoding_message_size(max_message_bytes)
        .max_encoding_message_size(max_message_bytes))
}

/// A reconnecting channel that can be created before the local Runtime starts.
/// Construct it inside a Tokio runtime, just like tonic's other lazy channels.
pub fn connect_uds_lazy(
    socket_path: impl AsRef<Path>,
    max_message_bytes: usize,
) -> RuntimeProtocolClient<Channel> {
    let path = socket_path.as_ref().to_path_buf();
    let channel = Endpoint::from_static("http://[::]:50051").connect_with_connector_lazy(
        service_fn(move |_| {
            let path = path.clone();
            async move { UnixStream::connect(path).await.map(TokioIo::new) }
        }),
    );
    RuntimeProtocolClient::new(channel)
        .max_decoding_message_size(max_message_bytes)
        .max_encoding_message_size(max_message_bytes)
}

fn prepare_socket_parent(socket_path: &Path) -> Result<(), std::io::Error> {
    let parent = socket_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "socket_path_has_no_parent",
        )
    })?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn socket_path(home: impl AsRef<Path>) -> PathBuf {
    home.as_ref().join("run/runtime.sock")
}
