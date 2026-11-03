//! HTTP resources and Machine WebSocket upgrades on one listener.
use crate::{
    AgentRunContext, ExecutionRef, MachinePeer, ResourceRequest, RuntimePeer, TransportConfig,
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{convert::Infallible, future::Future};
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

type Body = Full<Bytes>;

/// Application-owned HTTP fallback; the protocol does not depend on UI assets.
pub type Fallback = std::sync::Arc<dyn Fn(&str) -> Response<Full<Bytes>> + Send + Sync>;

/// Resource requests use the attempt's token, never the Machine link secret.
/// Agent resource calls use the same business authorization boundary.
pub async fn serve<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: TransportConfig,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + MachinePeer + Clone,
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
        None,
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
    P: RuntimePeer + MachinePeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(
        listener, secret, peer, config, settings, true, None, None, None, shutdown,
    )
    .await
}

/// Serve management, Machine links, and resources on the same TCP listener.
pub async fn serve_with_management<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: TransportConfig,
    settings: crate::link::LinkSettings,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + MachinePeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(
        listener, secret, peer, config, settings, true, None, None, None, shutdown,
    )
    .await
}

/// Local management with live remote access control, or a Machine-only TLS listener.
pub async fn serve_configured<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    settings: crate::link::LinkSettings,
    remote: Option<std::sync::Arc<dyn crate::management::RemoteAccessControl>>,
    tls: Option<tokio_rustls::TlsAcceptor>,
    fallback: Option<Fallback>,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + MachinePeer + Clone,
    F: Future<Output = ()> + Send,
{
    serve_inner(
        listener,
        secret,
        peer,
        TransportConfig::default(),
        settings,
        true,
        remote,
        tls,
        fallback,
        shutdown,
    )
    .await
}
trait Socket: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Socket for T {}

async fn serve_inner<P, F>(
    listener: TcpListener,
    secret: String,
    peer: P,
    config: TransportConfig,
    settings: crate::link::LinkSettings,
    override_heartbeat: bool,
    remote: Option<std::sync::Arc<dyn crate::management::RemoteAccessControl>>,
    tls: Option<tokio_rustls::TlsAcceptor>,
    fallback: Option<Fallback>,
    shutdown: F,
) -> Result<(), crate::link::LinkError>
where
    P: RuntimePeer + MachinePeer + Clone,
    F: Future<Output = ()> + Send,
{
    let local_port = listener.local_addr()?.port();
    let mut tasks = tokio::task::JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            result = tasks.join_next(), if !tasks.is_empty() => { let _ = result; }
            accepted = listener.accept() => {
                let (socket, address) = accepted?;
                let peer = peer.clone();
                let secret = secret.clone();
                let settings = settings.clone();
                let remote = remote.clone();
                let tls = tls.clone();
                let fallback = fallback.clone();
                tasks.spawn(async move {
                    let machine_only = tls.is_some();
                    let socket: Box<dyn Socket> = if let Some(tls) = tls {
                        match tokio::time::timeout(settings.handshake_timeout, tls.accept(socket)).await {
                            Ok(Ok(socket)) => Box::new(socket),
                            _ => return,
                        }
                    } else { Box::new(socket) };
                    let (upgrades, mut upgraded) = tokio::sync::mpsc::channel(1);
                    let ws_peer = peer.clone();
                    let service = service_fn(move |mut request: Request<Incoming>| {
                        let fallback = fallback.clone();
                        let remote = remote.clone();
                        let peer = peer.clone();
                        let secret = secret.clone();
                        let upgrades = upgrades.clone();
                        async move {
                            let response = if !machine_only && !address.ip().is_loopback() {
                                crate::management_http::error(crate::Status::permission_denied("request forbidden"))
                            } else if !machine_only && let Some(response) = local_access(&request, local_port) {
                                response
                            } else if machine_only && request.uri().path() != "/" && request.uri().path() != "/machines/pair" {
                                empty(StatusCode::NOT_FOUND)
                            } else if request.uri().path().starts_with("/api/v1/resources/") {
                                management_resource(request, peer).await
                            } else if request.uri().path().starts_with(crate::management::PREFIX) {
                                if matches!(request.uri().path(), "/api/v1/get_remote_access" | "/api/v1/set_remote_access" | "/api/v1/enable_remote_access") {
                                    remote_access(request, remote, config.max_message_bytes).await
                                } else if request.uri().path() == crate::management::EVENTS_PATH {
                                    let subscription = event_subscription(request.uri().query());
                                    match subscription {
                                        Err(()) => crate::management_http::error(crate::Status::invalid_argument("invalid event subscription")),
                                        Ok(subscription) => {
                                            let service = crate::ValidatedRuntimeService::new(peer, config);
                                            match service.subscribe_management_events(crate::Request::new(subscription)).await {
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
                            } else if request.uri().path() == "/machines/pair" {
                                pair(request, peer, &secret, config.max_message_bytes).await
                            } else if request.uri().path() == "/" && (machine_only || request.headers().get("upgrade").is_some()) {
                                let credential = request.headers().get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
                                if let Err(reason) = peer.authorize_link(credential, &secret).await {
                                    empty(match reason {
                                        crate::link::CloseReason::MachineRevoked => StatusCode::FORBIDDEN,
                                        crate::link::CloseReason::MachineUnknown => StatusCode::NOT_FOUND,
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
                            } else if !machine_only && matches!(*request.method(), hyper::Method::GET | hyper::Method::HEAD) && let Some(fallback) = fallback {
                                let mut response = fallback(request.uri().path());
                                if request.method() == hyper::Method::HEAD {
                                    *response.body_mut() = Full::new(Bytes::new());
                                }
                                response
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

// Apply before routing, including Machine links, resources, and unknown paths.
fn local_access(request: &Request<Incoming>, port: u16) -> Option<Response<Body>> {
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let host_ok = request.headers().get_all("host").iter().count() == 1
        && request
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|host| hosts.iter().any(|allowed| allowed == host));
    let origins = request.headers().get_all("origin");
    let origin_ok = origins.iter().count() <= 1
        && origins.iter().all(|origin| {
            origin
                .to_str()
                .ok()
                .is_some_and(|origin| hosts.iter().any(|host| origin == format!("http://{host}")))
        });
    if !host_ok || !origin_ok {
        return Some(crate::management_http::error(
            crate::Status::permission_denied("request forbidden"),
        ));
    }
    if request.method() == hyper::Method::POST {
        let values = request.headers().get_all("content-type");
        let json = values.iter().count() == 1
            && values.iter().all(|value| {
                value.to_str().ok().is_some_and(|value| {
                    let mut parts = value.split(';');
                    parts
                        .next()
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
                        && parts
                            .next()
                            .is_none_or(|v| v.trim().eq_ignore_ascii_case("charset=utf-8"))
                        && parts.next().is_none()
                })
            });
        if !json {
            return Some(crate::management_http::response(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                &crate::management::ErrorBody::from(crate::Status::invalid_argument(
                    "JSON required",
                )),
            ));
        }
    }
    None
}

fn event_subscription(
    query: Option<&str>,
) -> Result<crate::management::ManagementSubscription, ()> {
    let mut subscription = crate::management::ManagementSubscription {
        request_id: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    };
    let mut workspace_seen = false;
    let mut after_seen = false;
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "workspace" if !workspace_seen => {
                workspace_seen = true;
                subscription.workspace = Some(crate::management::WorkspaceRef {
                    workspace_id: value.into_owned(),
                });
            }
            "after" if !after_seen => {
                after_seen = true;
                subscription.after_sequence = value.parse().map_err(|_| ())?;
            }
            "workspace" | "after" => return Err(()),
            _ => {}
        }
    }
    Ok(subscription)
}

fn empty(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .expect("HTTP response")
}

async fn management_resource<P: RuntimePeer>(
    request: Request<Incoming>,
    peer: P,
) -> Response<Body> {
    if request.method() != hyper::Method::GET {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let resource_id = request
        .uri()
        .path()
        .strip_prefix("/api/v1/resources/")
        .and_then(|id| id.parse::<i64>().ok())
        .filter(|id| *id > 0);
    let workspaces: Vec<_> =
        url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            .filter(|(key, _)| key == "workspace")
            .map(|(_, value)| value.into_owned())
            .collect();
    let (Some(resource_id), [workspace]) = (resource_id, workspaces.as_slice()) else {
        return crate::management_http::error(crate::Status::invalid_argument(
            "invalid resource request",
        ));
    };
    match peer
        .read_management_resource(workspace.clone(), resource_id)
        .await
    {
        Ok((resource, _))
            if !matches!(
                resource.media_type.as_str(),
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            ) =>
        {
            crate::management_http::error(crate::Status::permission_denied(
                "Unsupported image format",
            ))
        }
        Ok((resource, bytes)) => Response::builder()
            .header("content-type", resource.media_type)
            .header("content-length", bytes.len())
            .header("cache-control", "no-store")
            .header("x-content-type-options", "nosniff")
            .header("content-security-policy", "default-src 'none'; sandbox")
            .body(Full::new(Bytes::from(bytes)))
            .unwrap_or_else(|_| empty(StatusCode::INTERNAL_SERVER_ERROR)),
        Err(error) => crate::management_http::error(error),
    }
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
        .get("x-meerkat-execution")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok());
    let attempt = request
        .headers()
        .get("x-meerkat-attempt")
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
                crate::Code::PermissionDenied | crate::Code::Unauthenticated => {
                    StatusCode::FORBIDDEN
                }
                crate::Code::NotFound => StatusCode::NOT_FOUND,
                crate::Code::InvalidArgument => StatusCode::BAD_REQUEST,
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

async fn pair<P: MachinePeer>(
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
    match peer.pair_machine(request, secret).await {
        Ok(value) => Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(
                serde_json::to_vec(&value).expect("pair response"),
            )))
            .expect("HTTP response"),
        Err(_) => empty(StatusCode::UNAUTHORIZED),
    }
}

async fn remote_access(
    request: Request<Incoming>,
    remote: Option<std::sync::Arc<dyn crate::management::RemoteAccessControl>>,
    limit: usize,
) -> Response<Body> {
    let Some(remote) = remote else {
        return empty(StatusCode::NOT_FOUND);
    };
    if request.method() != hyper::Method::POST {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }
    let enable = request.uri().path().ends_with("enable_remote_access");
    let set = request.uri().path().ends_with("set_remote_access");
    let result = if enable {
        remote.enable().await
    } else if set {
        match http_body_util::Limited::new(request.into_body(), limit)
            .collect()
            .await
        {
            Ok(bytes) => match serde_json::from_slice::<crate::management::SetRemoteAccessRequest>(
                &bytes.to_bytes(),
            ) {
                Ok(value) => remote.set(value.address).await,
                Err(_) => Err(crate::Status::invalid_argument("invalid remote address")),
            },
            Err(_) => Err(crate::Status::resource_exhausted("request too large")),
        }
    } else {
        remote.get().await
    };
    match result {
        Ok(value) => crate::management_http::response(StatusCode::OK, &value),
        Err(error) => crate::management_http::error(error),
    }
}
