//! HTTP resources and Computer WebSocket upgrades on one listener.
use crate::{
    AgentRunContext, ComputerPeer, ExecutionRef, ResourceRequest, RuntimePeer, TransportConfig,
};
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::rt::TokioIo;
use std::{convert::Infallible, future::Future};
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

type Body = Full<Bytes>;

/// Resource requests use the attempt's token, never the Computer link secret.
/// Legacy Agent resource RPCs retain the same business authorization boundary.
pub async fn serve<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + ComputerPeer + Clone,
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
                let peer = peer.clone();
                let secret = secret.clone();
                tasks.spawn(async move {
                    let (upgrades, mut upgraded) = tokio::sync::mpsc::channel(1);
                    let ws_peer = peer.clone();
                    let service = service_fn(move |mut request: Request<Incoming>| {
                        let peer = peer.clone();
                        let secret = secret.clone();
                        let upgrades = upgrades.clone();
                        async move {
                            let response = if request.uri().path().starts_with("/resources/") {
                                download(request, peer).await
                            } else if request.uri().path() == "/" {
                                if request.headers().get("authorization").and_then(|v| v.to_str().ok()) != Some(format!("Bearer {secret}").as_str()) {
                                    empty(StatusCode::UNAUTHORIZED)
                                } else {
                                    let handshake = Request::builder().method(request.method()).uri(request.uri());
                                    let mut handshake = handshake.body(()).expect("HTTP request");
                                    *handshake.headers_mut() = request.headers().clone();
                                    match tokio_tungstenite::tungstenite::handshake::server::create_response(&handshake) {
                                        Ok(response) => {
                                            let _ = upgrades.send(hyper::upgrade::on(&mut request)).await;
                                            response.map(|_| Full::new(Bytes::new()))
                                        }
                                        Err(_) => empty(StatusCode::BAD_REQUEST),
                                    }
                                }
                            } else { empty(StatusCode::NOT_FOUND) };
                            Ok::<_, Infallible>(response)
                        }
                    });
                    let connection = hyper::server::conn::http1::Builder::new()
                        .max_buf_size(32 * 1024)
                        .serve_connection(TokioIo::new(socket), service).with_upgrades();
                    if connection.await.is_ok()
                        && let Ok(upgrade) = upgraded.try_recv()
                        && let Ok(socket) = upgrade.await {
                        crate::link::serve_upgraded(TokioIo::new(socket), ws_peer, config).await;
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

fn empty(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .expect("HTTP response")
}

async fn download<P: RuntimePeer>(request: Request<Incoming>, peer: P) -> Response<Body> {
    if request.method() != hyper::Method::GET {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let Some(token) = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
    else {
        return empty(StatusCode::UNAUTHORIZED);
    };
    let Some(resource_id) = request
        .uri()
        .path()
        .strip_prefix("/resources/")
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|id| *id > 0)
    else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let execution = request
        .headers()
        .get("x-lankide-execution")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    let attempt = request
        .headers()
        .get("x-lankide-attempt")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok());
    let (Some(execution_id), Some(attempt)) = (execution, attempt) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let mut chunks = match peer
        .download_resource(ResourceRequest {
            resource_id,
            transfer_id: uuid::Uuid::new_v4().to_string(),
            context: Some(AgentRunContext {
                execution: Some(ExecutionRef {
                    execution_id,
                    attempt,
                }),
                execution_token: token.into(),
            }),
        })
        .await
    {
        Ok(chunks) => chunks,
        Err(error) => {
            return empty(match error.code() {
                tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                    StatusCode::FORBIDDEN
                }
                tonic::Code::NotFound => StatusCode::NOT_FOUND,
                tonic::Code::InvalidArgument => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            });
        }
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks.next().await {
        match chunk {
            Ok(chunk) => bytes.extend_from_slice(&chunk.bytes),
            Err(_) => return empty(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
    let size = bytes.len();
    let mut response = Response::builder()
        .header("accept-ranges", "bytes")
        .header("content-type", "application/octet-stream")
        .header("cache-control", "no-store");
    let body = if let Some(range) = request.headers().get("range") {
        let range = range
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| v.split_once('-'))
            .and_then(|(start, end)| {
                if start.is_empty() {
                    let suffix = end.parse::<usize>().ok()?;
                    (suffix > 0 && size > 0)
                        .then_some((size.saturating_sub(suffix), size.saturating_sub(1)))
                } else {
                    let start = start.parse::<usize>().ok()?;
                    let end = if end.is_empty() {
                        size.checked_sub(1)?
                    } else {
                        end.parse::<usize>().ok()?.min(size.checked_sub(1)?)
                    };
                    (start <= end && start < size).then_some((start, end))
                }
            });
        let Some((start, end)) = range else {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("content-range", format!("bytes */{size}"))
                .body(Full::new(Bytes::new()))
                .expect("HTTP response");
        };
        response = response
            .status(StatusCode::PARTIAL_CONTENT)
            .header("content-range", format!("bytes {start}-{end}/{size}"));
        Bytes::from(bytes).slice(start..=end)
    } else {
        Bytes::from(bytes)
    };
    response
        .header("content-length", body.len())
        .body(Full::new(body))
        .expect("HTTP response")
}
