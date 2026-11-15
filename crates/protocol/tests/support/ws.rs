//! Shared test-only wiring for the real WS, HTTP, and local JSON endpoints.
#![allow(dead_code)]
use meerkat_protocol::link::{self, CloseReason, LinkError, MachineLink};
use meerkat_protocol::{MachinePeer, RuntimeFrame, RuntimePeer, TransportConfig, TransportError};
use meerkat_protocol::{Response, Status};
use std::{
    future::Future,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc;
use tokio_stream::{Stream, StreamExt};

pub async fn serve_local<P, F>(
    socket: impl AsRef<Path>,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), TransportError>
where
    P: RuntimePeer + MachinePeer + Clone,
    F: Future<Output = ()> + Send + 'static,
{
    let socket = socket.as_ref().to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("ws://{}", listener.local_addr()?);
    let secret = link::publish(&socket, &url)?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    std::fs::write(
        socket.join(meerkat_protocol::management::URL_FILE),
        url.replacen("ws://", "http://", 1),
    )?;
    let ws = tokio::spawn(meerkat_protocol::resources::serve_with_management(
        listener,
        secret,
        peer.clone(),
        config,
        Default::default(),
        async {
            let _ = stopped.await;
        },
    ));
    let result =
        meerkat_protocol::serve_local(socket.join("agent.sock"), peer, config, shutdown).await;
    let _ = stop.send(());
    let _ = ws.await;
    result
}

fn status(error: LinkError) -> Status {
    match error {
        LinkError::Closed(CloseReason::ProtocolVersionUnsupported) => {
            Status::failed_precondition("protocol_version_unsupported")
        }
        LinkError::Closed(CloseReason::EventSequenceRegression) => {
            Status::out_of_range("event_sequence_regression")
        }
        LinkError::Closed(CloseReason::MessageTooLarge) => {
            Status::out_of_range("message_too_large")
        }
        LinkError::Closed(reason) => Status::invalid_argument(reason.to_string()),
        error => Status::unavailable(error.to_string()),
    }
}
#[derive(Debug)]
pub struct Messages(mpsc::Receiver<Result<RuntimeFrame, Status>>);
impl Messages {
    pub async fn message(&mut self) -> Result<Option<RuntimeFrame>, Status> {
        self.0.recv().await.transpose()
    }
}
impl Stream for Messages {
    type Item = Result<RuntimeFrame, Status>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
pub async fn connect_stream(
    socket: &Path,
    mut frames: impl Stream<Item = meerkat_protocol::MachineFrame> + Unpin + Send + 'static,
) -> Result<Response<Messages>, Status> {
    let (url, secret) = link::discover(socket).map_err(|e| Status::unavailable(e.to_string()))?;
    let hello = frames
        .next()
        .await
        .ok_or_else(|| Status::invalid_argument("missing_hello"))?;
    let credential = match &hello.frame {
        Some(meerkat_protocol::machine_frame::Frame::Hello(hello))
            if !hello.device_credential.is_empty() =>
        {
            hello.device_credential.as_str()
        }
        _ => &secret,
    };
    let mut link = link::connect(
        &url,
        credential,
        meerkat_protocol::DEFAULT_MAX_MESSAGE_BYTES,
    )
    .await
    .map_err(status)?;
    link.send(hello).await.map_err(status)?;
    let first = link
        .receive()
        .await
        .map_err(status)?
        .ok_or_else(|| Status::unavailable("closed"))?;
    let (sender, receiver) = mpsc::channel(64);
    sender.send(Ok(first)).await.expect("first frame");
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = sender.closed() => break,
                frame = frames.next() => {
                    let Some(frame) = frame else { break };
                    let result = link.send(frame).await;
                    if let Err(error) = result { let _ = sender.send(Err(status(error))).await; break; }
                }
                frame = link.receive() => match frame {
                    Ok(Some(frame)) => { if sender.send(Ok(frame)).await.is_err() { break; } }
                    Ok(None) => break,
                    Err(LinkError::Closed(CloseReason::Shutdown)) => break,
                    Err(error) => { let _ = sender.send(Err(status(error))).await; break; }
                }
            }
        }
    });
    Ok(Response::new(Messages(receiver)))
}
