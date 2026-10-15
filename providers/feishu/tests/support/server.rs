use super::http::{Request, Response, Server};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{
    self,
    handshake::server::{Request as UpgradeRequest, Response as UpgradeResponse},
};

pub const FILE: &[u8] = b"attachment bytes\0\xff\r\nfrom Alice";
// Platform-owned pbbp2 wire contract, independent of the Provider's private frame type.
#[derive(Clone, Default, PartialEq)]
struct Header {
    key: String,
    value: String,
}
#[derive(Clone, Default, PartialEq)]
struct Frame {
    seq_id: u64,
    log_id: u64,
    service: i32,
    method: i32,
    headers: Vec<Header>,
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
                    let mut header = Header::default();
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

#[derive(Clone, Debug)]
pub struct Delivery {
    pub chat: String,
    pub parent: Option<String>,
    pub body: Value,
}
#[derive(Default)]
struct State {
    events: BTreeMap<String, VecDeque<Value>>,
    requests: Vec<Request>,
    acknowledgements: Vec<Value>,
    next_message: usize,
    deliveries: Vec<Delivery>,
    parents: BTreeMap<String, String>,
    messages: BTreeMap<(String, String), Value>,
    chats: BTreeMap<String, String>,
}
pub struct FeishuServer {
    pub http: Server,
    state: Arc<Mutex<State>>,
    stopped: Arc<AtomicBool>,
    websocket: Option<JoinHandle<()>>,
}
impl FeishuServer {
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let stopped = Arc::new(AtomicBool::new(false));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let ws_address = listener.local_addr().unwrap();
        let shared = state.clone();
        let stop = stopped.clone();
        let websocket = thread::spawn(move || {
            let mut peers = vec![];
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let shared = shared.clone();
                        let stop = stop.clone();
                        peers.push(thread::spawn(move || {
                            // The non-blocking listener can pass its mode to this socket.
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(30)))
                                .unwrap();
                            stream
                                .set_write_timeout(Some(Duration::from_secs(30)))
                                .unwrap();
                            let mut app = String::new();
                            let mut socket = tungstenite::accept_hdr(
                                stream,
                                |request: &UpgradeRequest, response: UpgradeResponse| {
                                    app = request.uri().path().trim_start_matches('/').to_owned();
                                    Ok(response)
                                },
                            )
                            .unwrap();
                            socket
                                .get_mut()
                                .set_read_timeout(Some(Duration::from_millis(20)))
                                .unwrap();
                            let mut ready = false;
                            while !stop.load(Ordering::Acquire) {
                                if ready {
                                    let next = shared
                                        .lock()
                                        .unwrap()
                                        .events
                                        .entry(app.clone())
                                        .or_default()
                                        .pop_front();
                                    if let Some(event) = next {
                                        let headers = [
                                            ("type", "event"),
                                            (
                                                "message_id",
                                                event["header"]["event_id"].as_str().unwrap(),
                                            ),
                                            ("sum", "1"),
                                            ("seq", "0"),
                                        ]
                                        .into_iter()
                                        .map(|(key, value)| Header {
                                            key: key.into(),
                                            value: value.into(),
                                        })
                                        .collect();
                                        let frame = Frame {
                                            seq_id: 1,
                                            log_id: 1,
                                            service: 7,
                                            method: 1,
                                            headers,
                                            payload_encoding: None,
                                            payload_type: None,
                                            payload: Some(serde_json::to_vec(&event).unwrap()),
                                            log_id_new: None,
                                        };
                                        socket
                                            .send(tungstenite::Message::binary(
                                                frame.encode_to_vec(),
                                            ))
                                            .unwrap();
                                    }
                                }
                                match socket.read() {
                                    Ok(tungstenite::Message::Binary(bytes)) => {
                                        let frame = Frame::decode(bytes.as_ref()).unwrap();
                                        if frame.method == 0 {
                                            assert!(
                                                frame
                                                    .headers
                                                    .iter()
                                                    .any(|h| h.key == "type" && h.value == "ping")
                                            );
                                            ready = true;
                                        } else {
                                            let ack: Value = serde_json::from_slice(
                                                frame.payload.as_ref().unwrap(),
                                            )
                                            .unwrap();
                                            assert_eq!(ack["code"], 200);
                                            shared.lock().unwrap().acknowledgements.push(ack);
                                        }
                                    }
                                    Ok(tungstenite::Message::Close(_))
                                    | Err(tungstenite::Error::ConnectionClosed) => break,
                                    Err(tungstenite::Error::Io(error))
                                        if matches!(
                                            error.kind(),
                                            std::io::ErrorKind::WouldBlock
                                                | std::io::ErrorKind::TimedOut
                                        ) => {}
                                    other => panic!("unexpected websocket frame: {other:?}"),
                                }
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("WS accept: {error}"),
                }
            }
            for peer in peers {
                peer.join().unwrap();
            }
        });
        let shared = state.clone();
        let http = Server::start(move |request| {
            let mut state = shared.lock().unwrap();
            state.requests.push(request.clone());
            let path = request.path.split('?').next().unwrap();
            if path == "/callback/ws/endpoint" {
                let body = request.json();
                return Response::json(
                    json!({"code":0,"data":{"URL":format!("ws://{ws_address}/{}?service_id=7",body["AppID"].as_str().unwrap()),"ClientConfig":{"PingInterval":120}}}),
                );
            }
            if path.ends_with("/auth/v3/tenant_access_token/internal") {
                return Response::json(
                    json!({"code":0,"tenant_access_token":format!("tenant-{}", request.json()["app_id"].as_str().unwrap()),"expire":7200}),
                );
            }
            let app = request.header("authorization");
            let app = app.strip_prefix("Bearer tenant-").unwrap();
            assert!(matches!(app, "cli_alpha" | "cli_beta"));
            let data = match (request.method.as_str(), path) {
                ("GET", "/open-apis/bot/v3/info") => {
                    return Response::json(
                        json!({"code":0,"bot":{"open_id":if app == "cli_alpha" { "ou_alpha" } else { "ou_beta" },"app_name":app}}),
                    );
                }
                ("GET", path)
                    if path.starts_with("/open-apis/im/v1/messages/")
                        && !path.contains("/resources/") =>
                {
                    let id = path.trim_start_matches("/open-apis/im/v1/messages/");
                    let message = state
                        .messages
                        .get(&(app.into(), id.into()))
                        .expect("target belongs to receiving App");
                    // The real endpoint reports the chat but not its type.
                    json!({"items":[{"message_id":id,"chat_id":message["chat_id"],"msg_type":message["message_type"]}]})
                }
                ("GET", path) if path.starts_with("/open-apis/im/v1/chats/") => {
                    let id = path.trim_start_matches("/open-apis/im/v1/chats/");
                    let mode = state.chats.get(id).expect("chat was delivered");
                    json!({"chat_id":id,"chat_mode":mode,"chat_type":"private"})
                }
                ("GET", "/open-apis/contact/v3/users/ou_alice") => {
                    json!({"user":{"open_id":"ou_alice","union_id":"on_alice","name":"Alice"}})
                }
                ("GET", "/open-apis/im/v1/messages/om_file/resources/inbound-file") => {
                    return Response::file(FILE);
                }
                ("POST", "/open-apis/im/v1/files") => {
                    assert_eq!(request.part("file"), FILE);
                    json!({"file_key":"uploaded-file"})
                }
                ("POST", "/open-apis/im/v1/images") => json!({"image_key":"uploaded-image"}),
                ("POST", "/open-apis/im/v1/messages") => {
                    state.next_message += 1;
                    let body = request.json();
                    state.deliveries.push(Delivery {
                        chat: body["receive_id"].as_str().unwrap().into(),
                        parent: None,
                        body: body.clone(),
                    });
                    json!({"message_id":format!("om_sent_{}",state.next_message),"chat_id":body["receive_id"]})
                }
                ("POST", "/open-apis/im/v1/messages/om_file/reply") => {
                    let chat = state
                        .parents
                        .get("om_file")
                        .expect("reply must refer to a delivered message")
                        .clone();
                    state.next_message += 1;
                    state.deliveries.push(Delivery {
                        chat: chat.clone(),
                        parent: Some("om_file".into()),
                        body: request.json(),
                    });
                    json!({"message_id":format!("om_sent_{}",state.next_message),"chat_id":chat})
                }
                ("POST", "/open-apis/im/v1/messages/om_file/reactions") => {
                    json!({"reaction_id":"reaction-1"})
                }
                ("DELETE", path) if path.starts_with("/open-apis/im/v1/messages/om_sent_") => {
                    json!({})
                }
                _ => panic!("unexpected Feishu request: {request:?}"),
            };
            Response::json(json!({"code":0,"data":data}))
        });
        Self {
            http,
            state,
            stopped,
            websocket: Some(websocket),
        }
    }
    /// Deliver to an application account; direct-message callbacks use the same entry point.
    pub fn deliver(&self, app_id: &str, event: Value) {
        let mut state = self.state.lock().unwrap();
        if let Some(message) = event["event"].get("message") {
            let id = message["message_id"].as_str().unwrap();
            state
                .parents
                .insert(id.into(), message["chat_id"].as_str().unwrap().into());
            state
                .messages
                .insert((app_id.into(), id.into()), message.clone());
            state.chats.insert(
                message["chat_id"].as_str().unwrap().into(),
                message["chat_type"].as_str().unwrap().into(),
            );
        }
        state
            .events
            .entry(app_id.into())
            .or_default()
            .push_back(event);
    }
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }
    pub fn deliveries(&self) -> Vec<Delivery> {
        self.state.lock().unwrap().deliveries.clone()
    }
    pub fn acknowledgements(&self) -> usize {
        self.state.lock().unwrap().acknowledgements.len()
    }
}
impl Drop for FeishuServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let result = self.websocket.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
