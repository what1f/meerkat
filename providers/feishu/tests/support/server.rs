use super::http::{Request, Response, Server};
use prost::Message;
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
#[derive(Clone, PartialEq, Message)]
struct Header {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(string, tag = "2")]
    value: String,
}
#[derive(Clone, PartialEq, Message)]
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
    headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
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
            assert_eq!(request.header("authorization"), "Bearer tenant-cli_beta");
            let data = match (request.method.as_str(), path) {
                ("GET", "/open-apis/bot/v3/info") => {
                    return Response::json(
                        json!({"code":0,"bot":{"open_id":"ou_beta","app_name":"Bot B"}}),
                    );
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
        state.parents.insert(
            event["event"]["message"]["message_id"]
                .as_str()
                .unwrap()
                .into(),
            event["event"]["message"]["chat_id"]
                .as_str()
                .unwrap()
                .into(),
        );
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
