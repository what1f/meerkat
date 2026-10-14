//! HTTP resources and Computer WebSocket upgrades on one listener.
use crate::{
    AgentRunContext, ComputerPeer, ExecutionRef, ResourceRequest, RuntimePeer, TransportConfig,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
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
    serve_inner(
        listener,
        secret,
        peer,
        config,
        crate::link::LinkSettings::default(),
        false,
        None,
        shutdown,
    )
    .await
}

/// Serve HTTP resources and authenticated links with configured delivery deadlines.
pub async fn serve_with_settings<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: TransportConfig,
    settings: crate::link::LinkSettings,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + ComputerPeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(
        listener, secret, peer, config, settings, true, None, shutdown,
    )
    .await
}

/// Serve management, Computer links, and resources on the same TCP listener.
pub async fn serve_with_management<P, F>(
    listener: TcpListener,
    secret: String,
    owner: crate::management::OwnerCredential,
    peer: P,
    config: TransportConfig,
    settings: crate::link::LinkSettings,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + ComputerPeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(
        listener,
        secret,
        peer,
        config,
        settings,
        true,
        Some(owner),
        shutdown,
    )
    .await
}

async fn serve_inner<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: TransportConfig,
    settings: crate::link::LinkSettings,
    override_heartbeat: bool,
    owner: Option<crate::management::OwnerCredential>,
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
                let owner = owner.clone();
                let peer = peer.clone();
                let secret = secret.clone();
                let settings = settings.clone();
                tasks.spawn(async move {
                    let (upgrades, mut upgraded) = tokio::sync::mpsc::channel(1);
                    let ws_peer = peer.clone();
                    let service = service_fn(move |mut request: Request<Incoming>| {
                        let owner = owner.clone();
                        let peer = peer.clone();
                        let secret = secret.clone();
                        let upgrades = upgrades.clone();
                        async move {
                            let response = if request.uri().path().starts_with(crate::management::PREFIX) {
                                let credential = request.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
                                if !owner.as_ref().is_some_and(|owner| owner.authorizes(credential)) {
                                    crate::management_http::error(tonic::Status::unauthenticated("owner credential required"))
                                } else if request.uri().path() == crate::management::EVENTS_PATH {
                                    let subscription = request.headers().get("x-lankide-subscription").and_then(|v| v.to_str().ok()).and_then(|v| serde_json::from_str::<crate::management::ManagementSubscription>(v).ok());
                                    match subscription {
                                        None => crate::management_http::error(tonic::Status::invalid_argument("invalid event subscription")),
                                        Some(subscription) => {
                                            let service = crate::ValidatedRuntimeService::new(peer, config);
                                            match service.subscribe_management_events(tonic::Request::new(subscription.into())).await {
                                                Err(status) => crate::management_http::error(status),
                                                Ok(events) => {
                                                    let mut handshake = Request::builder().method(request.method()).uri(request.uri()).body(()).expect("HTTP request");
                                                    *handshake.headers_mut() = request.headers().clone();
                                                    match tokio_tungstenite::tungstenite::handshake::server::create_response(&handshake) {
                                                        Ok(response) => {
                                                            let _ = upgrades.send((hyper::upgrade::on(&mut request), Some(events.into_inner()))).await;
                                                            response.map(|_| Full::new(Bytes::new()))
                                                        }
                                                        Err(_) => empty(StatusCode::BAD_REQUEST),
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else { crate::management_http::dispatch(request, peer, config).await }
                            } else if request.uri().path().starts_with("/resources/") {
                                download(request, peer).await
                            } else if request.uri().path() == "/computers/pair" {
                                pair(request, peer, &secret, config.max_message_bytes).await
                            } else if request.uri().path() == "/" {
                                let credential = request.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
                                if let Err(reason) = peer.authorize_link(credential, &secret).await {
                                    empty(match reason {
                                        crate::link::CloseReason::ComputerRevoked => StatusCode::FORBIDDEN,
                                        crate::link::CloseReason::ComputerUnknown => StatusCode::NOT_FOUND,
                                        _ => StatusCode::UNAUTHORIZED,
                                    })
                                } else {
                                    let handshake = Request::builder().method(request.method()).uri(request.uri());
                                    let mut handshake = handshake.body(()).expect("HTTP request");
                                    *handshake.headers_mut() = request.headers().clone();
                                    match tokio_tungstenite::tungstenite::handshake::server::create_response(&handshake) {
                                        Ok(response) => {
                                            let _ = upgrades.send((hyper::upgrade::on(&mut request), None)).await;
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
                        .timer(TokioTimer::new())
                        .header_read_timeout(settings.handshake_timeout)
                        .max_buf_size(32 * 1024)
                        .serve_connection(TokioIo::new(socket), service).with_upgrades();
                    if connection.await.is_ok()
                        && let Ok((upgrade, events)) = upgraded.try_recv()
                        && let Ok(socket) = upgrade.await {
                        if let Some(events) = events {
                            crate::management_http::serve_events(TokioIo::new(socket), events, config.max_message_bytes).await;
                        } else {
                            crate::link::serve_upgraded(TokioIo::new(socket), ws_peer, config, settings, override_heartbeat).await;
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

async fn pair<P: ComputerPeer>(
    request: Request<Incoming>,
    peer: P,
    secret: &str,
    limit: usize,
) -> Response<Body> {
    if request.method() != hyper::Method::POST {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let bytes = match http_body_util::Limited::new(request.into_body(), limit)
        .collect()
        .await
    {
        Ok(body) => body.to_bytes(),
        Err(_) => return empty(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let request = match serde_json::from_slice(&bytes) {
        Ok(request) => request,
        Err(_) => return empty(StatusCode::BAD_REQUEST),
    };
    match peer.pair_computer(request, secret).await {
        Ok(value) => Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(
                serde_json::to_vec(&value).expect("pair response"),
            )))
            .expect("HTTP response"),
        Err(_) => empty(StatusCode::UNAUTHORIZED),
    }
}
