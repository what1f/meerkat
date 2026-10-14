//! Versioned JSON frames and the authenticated WebSocket Computer transport.
mod frames;
pub use frames::*;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{fmt, future::Future, marker::PhantomData, path::Path};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{
        self, Message,
        client::IntoClientRequest,
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    ProtocolVersionUnsupported,
    UnknownFrameType,
    MalformedJson,
    InvalidFrame,
    EventSequenceRegression,
    MessageTooLarge,
    Unauthorized,
    Shutdown,
    Timeout,
}
impl fmt::Display for CloseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            serde_json::to_value(self)
                .expect("close reason")
                .as_str()
                .expect("reason string"),
        )
    }
}
#[derive(Debug)]
pub enum LinkError {
    Closed(CloseReason),
    Io(std::io::Error),
    WebSocket(tungstenite::Error),
}
impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(reason) => write!(f, "{reason}"),
            Self::Io(e) => write!(f, "{e}"),
            Self::WebSocket(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for LinkError {}
impl From<std::io::Error> for LinkError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<tungstenite::Error> for LinkError {
    fn from(e: tungstenite::Error) -> Self {
        if matches!(e, tungstenite::Error::Capacity(_)) {
            Self::Closed(CloseReason::MessageTooLarge)
        } else if matches!(&e, tungstenite::Error::Http(response) if response.status() == 401) {
            Self::Closed(CloseReason::Unauthorized)
        } else {
            Self::WebSocket(e)
        }
    }
}

/// The business layer owns a single link. It has no dependency on a socket type.
#[tonic::async_trait]
pub trait ComputerLink: Send {
    type SendFrame: Send;
    type ReceiveFrame: Send;
    async fn send(&mut self, frame: Self::SendFrame) -> Result<(), LinkError>;
    async fn receive(&mut self) -> Result<Option<Self::ReceiveFrame>, LinkError>;
    async fn close(&mut self, reason: CloseReason) -> Result<(), LinkError>;
}

pub struct WebSocketLink<S, Out, In> {
    socket: WebSocketStream<S>,
    limit: usize,
    marker: PhantomData<(Out, In)>,
}
impl<S, Out, In> WebSocketLink<S, Out, In> {
    fn new(socket: WebSocketStream<S>, limit: usize) -> Self {
        Self {
            socket,
            limit,
            marker: PhantomData,
        }
    }
}
#[tonic::async_trait]
impl<S, Out, In> ComputerLink for WebSocketLink<S, Out, In>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
    Out: Serialize + Send,
    In: for<'a> Deserialize<'a> + Send,
{
    type SendFrame = Out;
    type ReceiveFrame = In;
    async fn send(&mut self, frame: Out) -> Result<(), LinkError> {
        let text = serde_json::to_string(&frame)
            .map_err(|_| LinkError::Closed(CloseReason::InvalidFrame))?;
        if text.len() > self.limit {
            self.close(CloseReason::MessageTooLarge).await?;
            return Err(LinkError::Closed(CloseReason::MessageTooLarge));
        }
        self.socket.send(Message::Text(text.into())).await?;
        Ok(())
    }
    async fn receive(&mut self) -> Result<Option<In>, LinkError> {
        while let Some(message) = self.socket.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    let reason = if matches!(error, tungstenite::Error::Capacity(_)) {
                        CloseReason::MessageTooLarge
                    } else {
                        return Err(error.into());
                    };
                    let _ = self.close(reason).await;
                    return Err(LinkError::Closed(reason));
                }
            };
            match message {
                Message::Text(text) => {
                    let parsed = serde_json::from_str::<serde_json::Value>(&text)
                        .map_err(|_| CloseReason::MalformedJson)
                        .and_then(|value| {
                            value
                                .get("type")
                                .and_then(|v| v.as_str())
                                .ok_or(CloseReason::MalformedJson)?;
                            // Serde distinguishes an unknown tagged variant from a malformed payload.
                            serde_json::from_value(value).map_err(|error| {
                                if error.to_string().starts_with("unknown variant") {
                                    CloseReason::UnknownFrameType
                                } else {
                                    CloseReason::MalformedJson
                                }
                            })
                        });
                    match parsed {
                        Ok(frame) => return Ok(Some(frame)),
                        Err(reason) => {
                            let _ = self.close(reason).await;
                            return Err(LinkError::Closed(reason));
                        }
                    }
                }
                Message::Close(frame) => {
                    if let Some(frame) = frame {
                        let reason = serde_json::from_value(serde_json::Value::String(
                            frame.reason.to_string(),
                        ))
                        .unwrap_or(CloseReason::Shutdown);
                        let _ = self.socket.flush().await;
                        return Err(LinkError::Closed(reason));
                    }
                    return Ok(None);
                }
                Message::Ping(_) | Message::Pong(_) => {
                    self.socket.flush().await?;
                }
                _ => {
                    self.close(CloseReason::InvalidFrame).await?;
                    return Err(LinkError::Closed(CloseReason::InvalidFrame));
                }
            }
        }
        Ok(None)
    }
    async fn close(&mut self, reason: CloseReason) -> Result<(), LinkError> {
        self.socket
            .close(Some(CloseFrame {
                code: CloseCode::Library(4000),
                reason: reason.to_string().into(),
            }))
            .await?;
        Ok(())
    }
}
fn ws_config(limit: usize) -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(limit))
        .max_frame_size(Some(limit))
}
pub type ClientLink = WebSocketLink<MaybeTlsStream<TcpStream>, ComputerFrame, RuntimeFrame>;
pub async fn connect(url: &str, secret: &str, limit: usize) -> Result<ClientLink, LinkError> {
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {secret}")
            .parse()
            .map_err(|_| LinkError::Closed(CloseReason::Unauthorized))?,
    );
    let (socket, _) =
        tokio_tungstenite::connect_async_with_config(request, Some(ws_config(limit)), false)
            .await?;
    Ok(WebSocketLink::new(socket, limit))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkSettings {
    pub heartbeat_interval: std::time::Duration,
    pub handshake_timeout: std::time::Duration,
}
impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            heartbeat_interval: std::time::Duration::from_secs(5),
            handshake_timeout: std::time::Duration::from_secs(10),
        }
    }
}

/// Serve authenticated Computer links. Shutdown drops and joins every connection.
pub async fn serve<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: crate::TransportConfig,
    shutdown: F,
) -> Result<(), LinkError>
where
    P: crate::ComputerPeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(
        listener,
        secret,
        peer,
        config,
        LinkSettings::default(),
        false,
        shutdown,
    )
    .await
}

pub async fn serve_with_settings<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: crate::TransportConfig,
    settings: LinkSettings,
    shutdown: F,
) -> Result<(), LinkError>
where
    P: crate::ComputerPeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(listener, secret, peer, config, settings, true, shutdown).await
}

async fn serve_inner<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: crate::TransportConfig,
    settings: LinkSettings,
    override_heartbeat: bool,
    shutdown: F,
) -> Result<(), LinkError>
where
    P: crate::ComputerPeer + Clone,
    F: Future<Output = ()> + Send,
{
    let mut tasks = tokio::task::JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            result = tasks.join_next(), if !tasks.is_empty() => { let _ = result; }
            accepted = listener.accept() => {
                let (socket, _) = accepted?;
                let secret = secret.clone(); let peer = peer.clone(); let settings = settings.clone();
                tasks.spawn(async move {
                    let upgraded = tokio::time::timeout(settings.handshake_timeout, tokio_tungstenite::accept_hdr_async_with_config(socket, move |request: &tungstenite::handshake::server::Request, response: tungstenite::handshake::server::Response| {
                        if request.headers().get("authorization").and_then(|v| v.to_str().ok()) != Some(format!("Bearer {secret}").as_str()) {
                            let mut denied = tungstenite::http::Response::new(Some("unauthorized".to_owned()));
                            *denied.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED; return Err(denied);
                        }
                        Ok(response)
                    }, Some(ws_config(config.max_message_bytes)))).await;
                    if let Ok(Ok(socket)) = upgraded {
                        let mut link = WebSocketLink::new(socket, config.max_message_bytes);
                        match accept_link(&mut link, peer, config, settings.clone(), override_heartbeat).await {
                            Ok(()) => { let _ = tokio::time::timeout(settings.handshake_timeout, link.close(CloseReason::Shutdown)).await; }
                            Err(LinkError::Closed(reason)) => { let _ = tokio::time::timeout(settings.handshake_timeout, link.close(reason)).await; }
                            Err(_) => {}
                        }
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

async fn accept_link<
    L: ComputerLink<SendFrame = RuntimeFrame, ReceiveFrame = ComputerFrame>,
    P: crate::ComputerPeer,
>(
    link: &mut L,
    peer: P,
    config: crate::TransportConfig,
    settings: LinkSettings,
    override_heartbeat: bool,
) -> Result<(), LinkError> {
    let first = tokio::time::timeout(settings.handshake_timeout, link.receive())
        .await
        .map_err(|_| LinkError::Closed(CloseReason::Timeout))??
        .ok_or(LinkError::Closed(CloseReason::InvalidFrame))?;
    let ComputerPayload::Hello(hello) = first.frame else {
        return Err(LinkError::Closed(CloseReason::InvalidFrame));
    };
    if first.protocol_version != config.protocol_version
        || hello.protocol_version != config.protocol_version
    {
        let hello = hello.into();
        crate::validate_computer_hello(&hello, hello.protocol_version)
            .map_err(|_| LinkError::Closed(CloseReason::InvalidFrame))?;
        peer.link_rejected(hello, CloseReason::ProtocolVersionUnsupported)
            .await;
        return Err(LinkError::Closed(CloseReason::ProtocolVersionUnsupported));
    }
    let hello = hello.into();
    crate::validate_computer_hello(&hello, config.protocol_version)
        .map_err(|_| LinkError::Closed(CloseReason::InvalidFrame))?;
    let (sender, receiver) = tokio::sync::mpsc::channel(64);
    let mut output = peer
        .connect_computer(
            hello,
            Box::pin(tokio_stream::wrappers::ReceiverStream::new(receiver)),
        )
        .await
        .map_err(|_| LinkError::Closed(CloseReason::InvalidFrame))?;
    let mut validator = crate::validation::ComputerFrameValidator::default();
    let mut first = true;
    let mut interval = settings
        .heartbeat_interval
        .max(std::time::Duration::from_millis(1));
    let mut heartbeat = tokio::time::interval(interval);
    heartbeat.tick().await;
    let deadline = tokio::time::sleep(interval * 3);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Err(LinkError::Closed(CloseReason::Timeout)),
            _ = heartbeat.tick(), if !first => {
                tokio::time::timeout(interval, link.send(RuntimeFrame { protocol_version: config.protocol_version, frame: RuntimePayload::Heartbeat }))
                    .await.map_err(|_| LinkError::Closed(CloseReason::Timeout))??;
            }
            frame = link.receive() => {
                let Some(frame) = frame? else { break };
                deadline.as_mut().reset(tokio::time::Instant::now() + interval * 3);
                if frame.protocol_version != config.protocol_version { return Err(LinkError::Closed(CloseReason::ProtocolVersionUnsupported)); }
                let frame = frame.into();
                validator.validate(&frame).map_err(|error| LinkError::Closed(if error.code() == tonic::Code::OutOfRange { CloseReason::EventSequenceRegression } else { CloseReason::InvalidFrame }))?;
                if sender.send(Ok(frame)).await.is_err() { break; }
            }
            frame = output.next() => {
                let Some(frame) = frame else { break };
                let mut frame = frame.map_err(|_| LinkError::Closed(CloseReason::InvalidFrame))?;
                crate::validate_runtime_frame(&frame).map_err(|_| LinkError::Closed(CloseReason::InvalidFrame))?;
                match (&frame.frame, first) {
                    (Some(crate::runtime_frame::Frame::Hello(hello)), true)
                        if hello.protocol_version == config.protocol_version => {},
                    (Some(crate::runtime_frame::Frame::Hello(_)), _) | (_, true) => {
                        return Err(LinkError::Closed(CloseReason::InvalidFrame));
                    }
                    _ => {},
                }
                if let Some(crate::runtime_frame::Frame::Hello(hello)) = frame.frame.as_mut() {
                    if override_heartbeat {
                        hello.heartbeat_interval_ms = interval.as_millis() as u64;
                    } else {
                        interval = std::time::Duration::from_millis(hello.heartbeat_interval_ms.max(1));
                        heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
                    }
                    deadline.as_mut().reset(tokio::time::Instant::now() + interval * 3);
                }
                first = false;
                tokio::time::timeout(interval, link.send(frame.try_into().map_err(LinkError::Closed)?)).await.map_err(|_| LinkError::Closed(CloseReason::Timeout))??;
            }
        }
    }
    Ok(())
}

/// Local discovery uses the same private run directory as the management socket.
pub fn discover(socket: &Path) -> Result<(String, String), std::io::Error> {
    let run = socket
        .parent()
        .ok_or_else(|| std::io::Error::other("missing_run_directory"))?;
    Ok((
        std::fs::read_to_string(run.join("computer-link.url"))?
            .trim()
            .to_owned(),
        std::fs::read_to_string(run.join("computer-link.secret"))?
            .trim()
            .to_owned(),
    ))
}
pub fn publish(run: &Path, url: &str) -> Result<String, std::io::Error> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(run)?;
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    for (name, value) in [
        ("computer-link.secret", secret.as_str()),
        ("computer-link.url", url),
    ] {
        // Replace the entry instead of following an existing symlink.
        let path = run.join(name);
        let temporary = run.join(format!(".{name}-{}", uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(temporary, path)?;
    }
    Ok(secret)
}
