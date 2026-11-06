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
use crate::events::EventFuture;
use crate::events::{EventSource, RawEvent, SourceError};
use crate::http::{Method, Request, Transport};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

const CONTROL_METHOD: i32 = 0;
const DATA_METHOD: i32 = 1;
const MESSAGE_TYPE_EVENT: &str = "event";
const MESSAGE_TYPE_PING: &str = "ping";
const MESSAGE_TYPE_PONG: &str = "pong";
const MAX_FRAGMENT_COUNT: usize = 4096;
const MAX_FRAGMENT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Default, PartialEq)]
struct FrameHeader {
    key: String,
    value: String,
}

#[derive(Clone, Default, PartialEq)]
struct Frame {
    seq_id: u64,
    log_id: u64,
    service: i32,
    method: i32,
    headers: Vec<FrameHeader>,
    payload_encoding: Option<String>,
    payload_type: Option<String>,
    payload: Option<Vec<u8>>,
    log_id_new: Option<String>,
}

// Feishu's fixed pbbp2 envelope. Payloads are opaque JSON; unknown envelope
// fields are skipped so additions by the platform remain compatible.
#[derive(Debug)]
struct FrameDecodeError;
impl std::fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid Feishu frame")
    }
}
fn read_varint(input: &mut &[u8]) -> Result<u64, FrameDecodeError> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = input.split_first().ok_or(FrameDecodeError)?;
        *input = rest;
        if shift == 63 && byte > 1 {
            return Err(FrameDecodeError);
        }
        value |= ((byte & 127) as u64) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err(FrameDecodeError)
}
fn read_bytes<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], FrameDecodeError> {
    let length = usize::try_from(read_varint(input)?).map_err(|_| FrameDecodeError)?;
    if length > input.len() {
        return Err(FrameDecodeError);
    }
    let (value, rest) = input.split_at(length);
    *input = rest;
    Ok(value)
}
fn read_string(input: &mut &[u8]) -> Result<String, FrameDecodeError> {
    String::from_utf8(read_bytes(input)?.to_vec()).map_err(|_| FrameDecodeError)
}
fn skip_field(input: &mut &[u8], kind: u64) -> Result<(), FrameDecodeError> {
    match kind {
        0 => {
            read_varint(input)?;
        }
        2 => {
            read_bytes(input)?;
        }
        1 | 5 => {
            let length = if kind == 1 { 8 } else { 4 };
            if input.len() < length {
                return Err(FrameDecodeError);
            }
            *input = &input[length..];
        }
        _ => return Err(FrameDecodeError),
    }
    Ok(())
}
fn write_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    output.push(value as u8);
}
fn write_number(tag: u64, value: u64, output: &mut Vec<u8>) {
    if value != 0 {
        write_varint(tag << 3, output);
        write_varint(value, output);
    }
}
fn write_bytes(tag: u64, value: &[u8], output: &mut Vec<u8>) {
    write_varint((tag << 3) | 2, output);
    write_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}
impl Frame {
    fn decode(mut input: &[u8]) -> Result<Self, FrameDecodeError> {
        let mut frame = Self::default();
        while !input.is_empty() {
            let key = read_varint(&mut input)?;
            let (tag, kind) = (key >> 3, key & 7);
            if tag == 0 {
                return Err(FrameDecodeError);
            }
            match (tag, kind) {
                (1, 0) => frame.seq_id = read_varint(&mut input)?,
                (2, 0) => frame.log_id = read_varint(&mut input)?,
                (3, 0) => frame.service = read_varint(&mut input)? as i32,
                (4, 0) => frame.method = read_varint(&mut input)? as i32,
                (5, 2) => {
                    let mut data = read_bytes(&mut input)?;
                    let mut header = FrameHeader::default();
                    while !data.is_empty() {
                        let key = read_varint(&mut data)?;
                        match (key >> 3, key & 7) {
                            (1, 2) => header.key = read_string(&mut data)?,
                            (2, 2) => header.value = read_string(&mut data)?,
                            (0, _) => return Err(FrameDecodeError),
                            (_, kind) => skip_field(&mut data, kind)?,
                        }
                    }
                    frame.headers.push(header);
                }
                (6, 2) => frame.payload_encoding = Some(read_string(&mut input)?),
                (7, 2) => frame.payload_type = Some(read_string(&mut input)?),
                (8, 2) => frame.payload = Some(read_bytes(&mut input)?.to_vec()),
                (9, 2) => frame.log_id_new = Some(read_string(&mut input)?),
                (_, kind) => skip_field(&mut input, kind)?,
            }
        }
        Ok(frame)
    }
    fn encode_to_vec(&self) -> Vec<u8> {
        let mut output = Vec::new();
        write_number(1, self.seq_id, &mut output);
        write_number(2, self.log_id, &mut output);
        write_number(3, self.service as u64, &mut output);
        write_number(4, self.method as u64, &mut output);
        for header in &self.headers {
            let mut data = Vec::new();
            if !header.key.is_empty() {
                write_bytes(1, header.key.as_bytes(), &mut data);
            }
            if !header.value.is_empty() {
                write_bytes(2, header.value.as_bytes(), &mut data);
            }
            write_bytes(5, &data, &mut output);
        }
        if let Some(value) = &self.payload_encoding {
            write_bytes(6, value.as_bytes(), &mut output);
        }
        if let Some(value) = &self.payload_type {
            write_bytes(7, value.as_bytes(), &mut output);
        }
        if let Some(value) = &self.payload {
            write_bytes(8, value, &mut output);
        }
        if let Some(value) = &self.log_id_new {
            write_bytes(9, value.as_bytes(), &mut output);
        }
        output
    }
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
    pub async fn connect(
        credentials: &Credentials,
        domain: &Domain,
        transport: &dyn Transport,
    ) -> Result<Self, SourceError> {
        let endpoint = format!("{}/callback/ws/endpoint", domain.base_url());
        let request = Request::new(Method::Post, endpoint)
            // The native SDK sends these gateway headers on the endpoint
            // request.  `locale` is consumed by the callback gateway and the
            // stable UA keeps this transport distinguishable without exposing
            // credentials.
            .header("locale", "zh")
            .header("User-Agent", "meerkat-feishu-provider/0.1")
            .json(&json!({
                "AppID": credentials.app_id,
                "AppSecret": credentials.app_secret()
            }))
            .map_err(|error| {
                SourceError::Failed(format!("Feishu websocket handshake request: {error}"))
            })?;
        let response = transport.execute(request).await.map_err(|_| {
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
        let socket = connect_async(data.url)
            .await
            .map_err(|_| SourceError::Disconnected("Feishu websocket connect failed".to_owned()))?
            .0;
        let mut source = Self {
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
        source.send_ping().await?;
        Ok(source)
    }

    async fn receive_message(&mut self, message: Message) -> Result<Option<RawEvent>, SourceError> {
        match message {
            Message::Binary(bytes) => self.receive_frame(&bytes),
            Message::Ping(payload) => {
                self.send_socket_message(Message::Pong(payload)).await?;
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
        self.pending_ack = Some(frame.clone());
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

    async fn send_event_ack(&mut self, frame: &Frame) -> Result<(), SourceError> {
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
        .await
    }

    async fn send_ping(&mut self) -> Result<(), SourceError> {
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
        })
        .await?;
        self.last_ping = Instant::now();
        Ok(())
    }

    async fn send_frame(&mut self, frame: Frame) -> Result<(), SourceError> {
        self.send_socket_message(Message::binary(frame.encode_to_vec()))
            .await
    }

    async fn send_socket_message(&mut self, message: Message) -> Result<(), SourceError> {
        let socket = self
            .socket
            .as_mut()
            .ok_or_else(|| SourceError::Disconnected("Feishu websocket is closed".to_owned()))?;
        socket
            .send(message)
            .await
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
    fn next(&mut self) -> EventFuture<'_, Result<Option<RawEvent>, SourceError>> {
        Box::pin(async move {
            if self.closed {
                return Ok(None);
            }
            if self.pending_ack.is_some() {
                return Err(SourceError::Failed(
                    "previous Feishu event is not acknowledged".into(),
                ));
            }
            loop {
                let delay = self.ping_interval.saturating_sub(self.last_ping.elapsed());
                let socket = self.socket.as_mut().ok_or_else(|| {
                    SourceError::Disconnected("Feishu websocket is closed".into())
                })?;
                tokio::select! {
                    _ = tokio::time::sleep(delay) => self.send_ping().await?,
                    message = socket.next() => match message {
                        Some(Ok(message)) => if let Some(event) = self.receive_message(message).await? { return Ok(Some(event)); },
                        _ => return Err(SourceError::Disconnected("Feishu websocket receive failed".into())),
                    }
                }
            }
        })
    }
    fn acknowledge(&mut self) -> EventFuture<'_, Result<(), SourceError>> {
        Box::pin(async move {
            if let Some(frame) = self.pending_ack.clone() {
                self.send_event_ack(&frame).await?;
                self.pending_ack = None;
            }
            Ok(())
        })
    }
    fn close(&mut self) -> EventFuture<'_, ()> {
        Box::pin(async move {
            self.closed = true;
            if let Some(mut socket) = self.socket.take() {
                let _ = socket.close(None).await;
            }
        })
    }
}

/// Connect with the Provider's shared HTTP client.
pub fn native_event_source_factory<'a>(
    _workspace_id: &'a meerkat_core::WorkspaceId,
    credentials: &'a Credentials,
    domain: &'a Domain,
    transport: &'a dyn Transport,
) -> EventFuture<'a, Result<Box<dyn EventSource>, SourceError>> {
    Box::pin(async move {
        Ok(
            Box::new(FeishuWebSocketSource::connect(credentials, domain, transport).await?)
                as Box<dyn EventSource>,
        )
    })
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
