use super::http::{Request, Response, Server};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

pub const FILE: &[u8] = b"attachment bytes\0\xff\r\nfrom Alice";
#[derive(Default)]
struct State {
    updates: BTreeMap<String, VecDeque<Value>>,
    requests: Vec<Request>,
    failures: BTreeMap<String, (u16, usize)>,
    next_message: i64,
}
pub struct TelegramServer {
    pub http: Server,
    state: Arc<Mutex<State>>,
}
impl TelegramServer {
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let http = Server::start(move |request| {
            let mut state = shared.lock().unwrap();
            state.requests.push(request.clone());
            if let Some((status, remaining)) = state.failures.get_mut(&request.path) {
                if *remaining > 0 {
                    *remaining -= 1;
                    let mut response = Response::json(if *status == 429 {
                        json!({"ok":false,"error_code":429,"description":"rate limit","parameters":{"retry_after":1}})
                    } else {
                        json!({})
                    });
                    response.status = *status;
                    return response;
                }
            }
            if request.path.starts_with("/file/bot") {
                assert_eq!(request.method, "GET");
                assert_eq!(request.path, "/file/bot200:local/documents/inbound.bin");
                return Response::file(FILE);
            }
            assert_eq!(request.method, "POST");
            let (bot, method) = request
                .path
                .trim_start_matches("/bot")
                .split_once('/')
                .unwrap();
            let result = match method {
                "getMe" => {
                    json!({"id":bot.split(':').next().unwrap().parse::<i64>().unwrap(),"is_bot":true,"first_name":"Bot B","username":match bot { "201:local" => "alpha_bot", "202:local" => "gamma_bot", _ => "beta_bot" },"can_manage_bots":true})
                }
                "getUpdates" => {
                    let offset = request.json()["offset"].as_i64().unwrap();
                    let queue = state.updates.entry(bot.into()).or_default();
                    queue.retain(|update| update["update_id"].as_i64().unwrap() >= offset);
                    let values: Vec<_> = queue.iter().cloned().collect();
                    drop(state);
                    if values.is_empty() {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    return Response::json(json!({"ok":true,"result":values}));
                }
                "sendMessage" | "sendDocument" | "sendPhoto" => {
                    state.next_message += 1;
                    let chat = if method == "sendMessage" {
                        request.json()["chat_id"].as_i64().unwrap()
                    } else {
                        String::from_utf8(request.part("chat_id"))
                            .unwrap()
                            .parse::<i64>()
                            .unwrap()
                    };
                    json!({"message_id":100 + state.next_message,"chat":{"id":chat,"type":"supergroup"},"date":1})
                }
                "getFile" => {
                    assert_eq!(request.json()["file_id"], "inbound-file");
                    json!({"file_id":"inbound-file","file_path":"documents/inbound.bin","file_size":FILE.len()})
                }
                "setMyName" | "setMyDescription" | "setMessageReaction" | "deleteMessage" => {
                    json!(true)
                }
                _ => panic!("unexpected Telegram request: {request:?}"),
            };
            Response::json(json!({"ok":true,"result":result}))
        });
        Self { http, state }
    }
    /// Keep event delivery account-specific so direct-message scenarios can use the same server.
    pub fn deliver(&self, bot_token: &str, update: Value) {
        self.state
            .lock()
            .unwrap()
            .updates
            .entry(bot_token.into())
            .or_default()
            .push_back(update);
    }
    pub fn fail_requests(&self, path: &str, status: u16, count: usize) {
        self.state
            .lock()
            .unwrap()
            .failures
            .insert(path.into(), (status, count));
    }
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }
}
