//! Feishu persistent-connection source.
//!
//! The Lark SDK's websocket transport uses the small `pbbp2.Frame` protobuf
//! envelope rather than JSON websocket messages.  This module mirrors that
//! wire envelope and keeps the SDK-specific handshake/framing below the
//! provider event dispatcher.  It intentionally exposes only [`EventSource`]
//! to the adapter: decoded callback bodies remain ordinary JSON bytes.  The
//! adapter owns the source-factory retry loop so this source stays focused on
//! one connected socket and can be replaced by a hermetic peer.

use crate::client::{Credentials, Domain};
use crate::events::{EventSource, RawEvent, SourceError};
use crate::http::{Method, Request, ReqwestTransport, Transport};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const CONTROL_METHOD: i32 = 0;
const DATA_METHOD: i32 = 1;
const MESSAGE_TYPE_EVENT: &str = "event";
const MESSAGE_TYPE_PING: &str = "ping";
const MESSAGE_TYPE_PONG: &str = "pong";
const MAX_FRAGMENT_COUNT: usize = 4096;
const MAX_FRAGMENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, PartialEq, ProstMessage)]
struct FrameHeader {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(string, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct Frame {
    #[prost(uint64, tag = "1")]
    seq_id: u64,
    #[prost(uint64, tag = "2")]
    log_id: u64,
    #[prost(int32, tag = "3")]
    service: i32,
    #[prost(int32, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<FrameHeader>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct EndpointResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    data: Option<EndpointData>,
}

#[derive(Clone, Debug, Deserialize)]
struct EndpointData {
    #[serde(rename = "URL", default)]
    url: String,
    #[serde(rename = "ClientConfig", default)]
    client_config: ClientConfig,
}

#[derive(Clone, Debug, Deserialize)]
struct ClientConfig {
    #[serde(rename = "PingInterval", default = "default_ping_interval")]
    ping_interval: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            ping_interval: default_ping_interval(),
        }
    }
}

fn default_ping_interval() -> u64 {
    120
}

#[derive(Default)]
struct FragmentBuffer {
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
}

/// Native Feishu persistent-connection implementation used by the default
/// Provider.  `EventSourceFactory` remains injectable for hermetic peers and
/// hosts that already own an SDK websocket.
pub struct FeishuWebSocketSource {
    runtime: Runtime,
    socket: Option<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>,
    service_id: i32,
    ping_interval: Duration,
    last_ping: Instant,
    fragments: HashMap<String, FragmentBuffer>,
    pending_ack: Option<Frame>,
    closed: bool,
}

impl std::fmt::Debug for FeishuWebSocketSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FeishuWebSocketSource")
            .field("connected", &self.socket.is_some())
            .field("service_id", &self.service_id)
            .field("ping_interval", &self.ping_interval)
            .field("fragment_count", &self.fragments.len())
            .finish()
    }
}

impl FeishuWebSocketSource {
    /// Fetch the SDK websocket endpoint and establish its TLS connection.
    /// Secrets are only placed in the request body and never in an error or
    /// debug value.
    pub fn connect(credentials: &Credentials, domain: &Domain) -> Result<Self, SourceError> {
        let endpoint = format!("{}/callback/ws/endpoint", domain.base_url());
        let request = Request::new(Method::Post, endpoint)
            // The native SDK sends these gateway headers on the endpoint
            // request.  `locale` is consumed by the callback gateway and the
            // stable UA keeps this transport distinguishable without exposing
            // credentials.
            .header("locale", "zh")
            .header("User-Agent", "lankide-feishu-provider/0.1")
            .json(&json!({
                "AppID": credentials.app_id,
                "AppSecret": credentials.app_secret()
            }))
            .map_err(|error| {
                SourceError::Failed(format!("Feishu websocket handshake request: {error}"))
            })?;
        let response = ReqwestTransport::new().execute(request).map_err(|_| {
            SourceError::Failed("Feishu websocket endpoint request failed".to_owned())
        })?;
        if !(200..300).contains(&response.status) {
            return Err(SourceError::Failed(format!(
                "Feishu websocket endpoint returned HTTP status {}",
                response.status
            )));
        }
        let envelope: EndpointResponse = response.json().map_err(|error| {
            SourceError::Failed(format!("Feishu websocket endpoint decode: {error}"))
        })?;
        if envelope.code != 0 {
            return Err(SourceError::Failed(
                "Feishu websocket endpoint rejected the application".to_owned(),
            ));
        }
        let data = envelope.data.ok_or_else(|| {
            SourceError::Malformed("Feishu websocket endpoint has no data".to_owned())
        })?;
        if data.url.trim().is_empty() {
            return Err(SourceError::Malformed(
                "Feishu websocket endpoint has no URL".to_owned(),
            ));
        }
        let service_id = query_parameter(&data.url, "service_id")
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or_default();
        let ping_interval = Duration::from_secs(data.client_config.ping_interval.max(1));
        let runtime = Runtime::new()
            .map_err(|_| SourceError::Failed("Feishu websocket runtime failed".to_owned()))?;
        let url = data.url;
        let socket = runtime
            .block_on(async move { connect_async(url).await })
            .map_err(|_| SourceError::Disconnected("Feishu websocket connect failed".to_owned()))?
            .0;
        let mut source = Self {
            runtime,
            socket: Some(socket),
            service_id,
            ping_interval,
            last_ping: Instant::now(),
            fragments: HashMap::new(),
            pending_ack: None,
            closed: false,
        };
        // The official Feishu SDK sends its first control ping immediately
        // after the websocket is established.  The gateway does not consider
        // an otherwise-idle socket ready for event delivery until that
        // control exchange has started.
        source.send_ping()?;
        Ok(source)
    }

    fn receive_message(&mut self, message: Message) -> Result<Option<RawEvent>, SourceError> {
        match message {
            Message::Binary(bytes) => self.receive_frame(&bytes),
            Message::Ping(payload) => {
                self.send_socket_message(Message::Pong(payload))?;
                Ok(None)
            }
            Message::Pong(_) => Ok(None),
            Message::Close(_) => {
                self.closed = true;
                self.socket = None;
                Err(SourceError::Disconnected(
                    "Feishu websocket closed by peer".to_owned(),
                ))
            }
            Message::Text(_) | Message::Frame(_) => Err(SourceError::Malformed(
                "Feishu websocket returned a non-binary SDK frame".to_owned(),
            )),
        }
    }

    fn receive_frame(&mut self, bytes: &[u8]) -> Result<Option<RawEvent>, SourceError> {
        let frame = Frame::decode(bytes)
            .map_err(|error| SourceError::Malformed(format!("Feishu pbbp2 frame: {error}")))?;
        let message_type = header_value(&frame.headers, "type");
        if frame.method == CONTROL_METHOD {
            if message_type.as_deref() == Some(MESSAGE_TYPE_PONG) {
                self.update_ping_interval(frame.payload.as_deref());
            }
            return Ok(None);
        }
        if frame.method != DATA_METHOD || message_type.as_deref() != Some(MESSAGE_TYPE_EVENT) {
            return Ok(None);
        }
        let message_id = header_value(&frame.headers, "message_id")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| SourceError::Malformed("Feishu event has no message_id".to_owned()))?;
        let sum = header_value(&frame.headers, "sum")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| SourceError::Malformed("Feishu event has invalid sum".to_owned()))?;
        let seq = header_value(&frame.headers, "seq")
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| SourceError::Malformed("Feishu event has invalid seq".to_owned()))?;
        if sum == 0 || sum > MAX_FRAGMENT_COUNT || seq >= sum {
            return Err(SourceError::Malformed(
                "Feishu event fragment index is out of bounds".to_owned(),
            ));
        }
        let payload = frame.payload.as_deref().unwrap_or_default().to_vec();
        let entry = self
            .fragments
            .entry(message_id.clone())
            .or_insert_with(|| FragmentBuffer {
                parts: vec![None; sum],
                bytes: 0,
            });
        if entry.parts.len() != sum {
            self.fragments.remove(&message_id);
            return Err(SourceError::Malformed(
                "Feishu event fragment sum changed".to_owned(),
            ));
        }
        if entry.parts[seq].is_none() {
            entry.bytes = entry.bytes.saturating_add(payload.len());
        }
        if entry.bytes > MAX_FRAGMENT_BYTES {
            self.fragments.remove(&message_id);
            return Err(SourceError::Malformed(
                "Feishu event exceeds the fragment size limit".to_owned(),
            ));
        }
        entry.parts[seq] = Some(payload);
        if entry.parts.iter().any(Option::is_none) {
            return Ok(None);
        }
        let entry = self.fragments.remove(&message_id).ok_or_else(|| {
            SourceError::Malformed("Feishu event fragment disappeared".to_owned())
        })?;
        let body = entry
            .parts
            .into_iter()
            .flatten()
            .flatten()
            .collect::<Vec<_>>();
        let event_type = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("header")
                    .and_then(|header| header.get("event_type"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| SourceError::Malformed("Feishu event has no event_type".to_owned()))?;
        self.pending_ack = Some(frame);
        Ok(Some(RawEvent::new(event_type, body)))
    }

    fn send_event_ack(&mut self, frame: &Frame) -> Result<(), SourceError> {
        let mut headers = frame.headers.clone();
        headers.push(FrameHeader {
            key: "biz_rt".to_owned(),
            value: "0".to_owned(),
        });
        self.send_frame(Frame {
            seq_id: frame.seq_id,
            log_id: frame.log_id,
            service: frame.service,
            method: frame.method,
            headers,
            payload_encoding: None,
            payload_type: None,
            payload: Some(json!({ "code": 200 }).to_string().into_bytes()),
            log_id_new: frame.log_id_new.clone(),
        })
    }

    fn send_ping(&mut self) -> Result<(), SourceError> {
        self.send_frame(Frame {
            seq_id: 0,
            log_id: 0,
            service: self.service_id,
            method: CONTROL_METHOD,
            headers: vec![FrameHeader {
                key: "type".to_owned(),
                value: MESSAGE_TYPE_PING.to_owned(),
            }],
            payload_encoding: None,
            payload_type: None,
            payload: None,
            log_id_new: None,
        })?;
        self.last_ping = Instant::now();
        Ok(())
    }

    fn send_frame(&mut self, frame: Frame) -> Result<(), SourceError> {
        self.send_socket_message(Message::binary(frame.encode_to_vec()))
    }

    fn send_socket_message(&mut self, message: Message) -> Result<(), SourceError> {
        let socket = self
            .socket
            .as_mut()
            .ok_or_else(|| SourceError::Disconnected("Feishu websocket is closed".to_owned()))?;
        self.runtime
            .block_on(async { socket.send(message).await })
            .map_err(|_| SourceError::Disconnected("Feishu websocket send failed".to_owned()))
    }

    fn update_ping_interval(&mut self, payload: Option<&[u8]>) {
        let Some(payload) = payload else {
            return;
        };
        let Ok(config) = serde_json::from_slice::<ClientConfig>(payload) else {
            return;
        };
        self.ping_interval = Duration::from_secs(config.ping_interval.max(1));
    }
}

impl EventSource for FeishuWebSocketSource {
    fn next(&mut self) -> Result<Option<RawEvent>, SourceError> {
        if self.closed {
            return Ok(None);
        }
        if self.pending_ack.is_some() {
            return Err(SourceError::Failed(
                "previous Feishu event is not acknowledged".into(),
            ));
        }
        loop {
            if self.last_ping.elapsed() >= self.ping_interval {
                self.send_ping()?;
            }
            let timeout = self
                .ping_interval
                .saturating_sub(self.last_ping.elapsed())
                .min(Duration::from_secs(1))
                .max(Duration::from_millis(100));
            let socket = self.socket.as_mut().ok_or_else(|| {
                SourceError::Disconnected("Feishu websocket is closed".to_owned())
            })?;
            let result = self
                .runtime
                .block_on(async { tokio::time::timeout(timeout, socket.next()).await });
            match result {
                Err(_) => {
                    // The short read timeout keeps shutdown responsive even
                    // when the SDK's advertised interval is long.  It is not
                    // itself a ping deadline: sending on every one-second
                    // timeout would turn a 120-second heartbeat into a busy
                    // control stream.
                    if self.last_ping.elapsed() >= self.ping_interval {
                        self.send_ping()?;
                    }
                    // Hand control back to the Provider dispatcher so it can
                    // observe shutdown and lifecycle changes even when the
                    // gateway has no data frames to deliver.
                    return Ok(None);
                }
                Ok(None) => {
                    self.closed = true;
                    self.socket = None;
                    return Err(SourceError::Disconnected(
                        "Feishu websocket stream ended".to_owned(),
                    ));
                }
                Ok(Some(Err(_error))) => {
                    self.closed = true;
                    return Err(SourceError::Disconnected(
                        "Feishu websocket receive failed".to_owned(),
                    ));
                }
                Ok(Some(Ok(message))) => {
                    if let Some(event) = self.receive_message(message)? {
                        return Ok(Some(event));
                    }
                }
            }
        }
    }

    fn acknowledge(&mut self) -> Result<(), SourceError> {
        if let Some(frame) = self.pending_ack.clone() {
            self.send_event_ack(&frame)?;
            self.pending_ack = None;
        }
        Ok(())
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(socket) = self.socket.as_mut() {
            let _ = self.runtime.block_on(async {
                let _ = socket.send(Message::Close(None)).await;
            });
        }
        self.socket = None;
    }
}

/// Factory function suitable for [`crate::adapter::EventSourceFactory`].
pub fn native_event_source_factory(
    _workspace_id: &lankide_core::WorkspaceId,
    credentials: &Credentials,
    domain: &Domain,
) -> Result<Box<dyn EventSource>, SourceError> {
    Ok(Box::new(FeishuWebSocketSource::connect(
        credentials,
        domain,
    )?))
}

fn header_value(headers: &[FrameHeader], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.clone())
}

fn query_parameter(url: &str, key: &str) -> Option<String> {
    url.split_once('?')?
        .1
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find_map(|(name, value)| (name == key).then_some(value.to_owned()))
}
