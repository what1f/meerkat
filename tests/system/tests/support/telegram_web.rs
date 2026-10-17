use super::im_http::{Response, Server};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
pub struct State {
    pub name: String,
    pub expired: bool,
    pub fail_send: bool,
    pub updates: VecDeque<Value>,
    pub sends: Vec<Value>,
    pub managed: bool,
}
pub struct TelegramWeb {
    pub http: Server,
    pub state: Arc<Mutex<State>>,
}
impl TelegramWeb {
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(State {
            name: "Alice".into(),
            ..Default::default()
        }));
        let shared = state.clone();
        let http = Server::start(move |r| {
            let mut state = shared.lock().unwrap();
            if r.path.starts_with("/file/") {
                return Response::file(b"avatar");
            }
            let method = r.path.rsplit('/').next().unwrap();
            let body = if r.body.is_empty() {
                Value::Null
            } else {
                r.json()
            };
            if state.expired && matches!(method, "getChat" | "getUserProfilePhotos") {
                return Response::json(
                    json!({"ok":false,"error_code":401,"description":"expired"}),
                );
            }
            let result = match method {
                "getMe" => {
                    json!({"id":if r.path.contains("100:manager"){100}else{200},"is_bot":true,"first_name":"Alpha","username":if r.path.contains("100:manager"){"manager_bot"}else{"alpha_bot"},"can_manage_bots":true})
                }
                "getManagedBotToken" => json!("200:agent"),
                "getUserProfilePhotos" => {
                    json!({"total_count":1,"photos":[[{"file_id":"photo","width":64,"height":64}]]})
                }
                "getFile" => json!({"file_id":"photo","file_path":"owner.jpg","file_size":6}),
                "getChat" => json!({"id":300,"type":"private","first_name":state.name}),
                "setMyName" | "setMyDescription" | "setMessageReaction" => json!(true),
                "getUpdates" => {
                    if r.path.contains("100:manager") {
                        if state.managed {
                            json!([])
                        } else {
                            state.managed = true;
                            json!([{"update_id":1,"managed_bot":{"user":{"id":300,"first_name":state.name},"bot":{"id":200,"is_bot":true,"first_name":"Alpha","username":"alpha_bot"}}}])
                        }
                    } else {
                        let offset = body["offset"].as_i64().unwrap_or(0);
                        state
                            .updates
                            .retain(|u| u["update_id"].as_i64().unwrap() >= offset);
                        let values: Vec<_> = state.updates.iter().cloned().collect();
                        drop(state);
                        if values.is_empty() {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        return Response::json(json!({"ok":true,"result":values}));
                    }
                }
                "sendMessage" => {
                    if state.fail_send {
                        return Response::json(
                            json!({"ok":false,"error_code":403,"description":"blocked"}),
                        );
                    }
                    state.sends.push(body.clone());
                    json!({"message_id":1000+state.sends.len(),"chat":{"id":body["chat_id"],"type":"private"},"date":1})
                }
                _ => panic!("unexpected IM request {}", r.path),
            };
            Response::json(json!({"ok":true,"result":result}))
        });
        Self { http, state }
    }
    pub fn message(&self, id: i64, private: bool, person: i64) {
        self.state.lock().unwrap().updates.push_back(json!({"update_id":id,"message":{"message_id":id,"date":1900000000,"chat":{"id":if private{person}else{-100},"type":if private{"private"}else{"supergroup"}},"from":{"id":person,"first_name":"Alice"},"text":if private{"hello"}else{"@alpha_bot hello"},"entities":if private{json!([])}else{json!([{"type":"mention","offset":0,"length":10}])}}}));
    }
}
