use lankide_provider_feishu::{
    Method, Request, Response, STANDARD_BOT_EVENTS, STANDARD_BOT_TENANT_SCOPES, Transport,
    TransportError,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
#[derive(Default)]
pub struct Feishu {
    /// 0 waiting for a scan, 1 scanned, 2 confirmed on the phone.
    pub login_stage: u8,
    pub qr_expired: bool,
    pub sends: Vec<Value>,
    pub message_chats: std::collections::BTreeMap<String, String>,
    pub chat_types: std::collections::BTreeMap<String, String>,
    qr_codes: usize,
    pub name: String,
    pub user: String,
    pub tenant: String,
    pub expired: bool,
    pub task_error: Option<i64>,
    pub task_error_suffix: Option<String>,
    pub task_http_status: Option<u16>,
    pub owner_task_calls: usize,
    pub tasks: Vec<Value>,
    pub task_subscriptions: usize,
    pub acknowledgements: usize,
    pub denied_tasks: Vec<String>,
    pub tokens: std::collections::BTreeMap<String, String>,
    pub events: std::collections::VecDeque<lankide_provider_feishu::events::RawEvent>,
    pub reject_app_create: bool,
    /// The owner has no profile picture.
    pub no_owner_avatar: bool,
    apps: Vec<String>,
    console_calls: Vec<String>,
    pub tasklist: Option<Value>,
    tasklist_creates: Vec<(String, Value)>,
    tasklist_member_adds: Vec<(String, Value)>,
}

#[derive(Clone, Default)]
pub struct FeishuWeb(pub Arc<Mutex<Feishu>>, Arc<Mutex<Option<Hold>>>);
struct Hold {
    suffix: String,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
impl FeishuWeb {
    pub fn hold(&self, suffix: &str) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *self.1.lock().unwrap() = Some(Hold {
            suffix: suffix.into(),
            entered: entered.clone(),
            release: release.clone(),
        });
        (entered, release)
    }
}

fn header<'a>(request: &'a Request, name: &str) -> &'a str {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .unwrap_or_default()
}

fn reply(body: Value, headers: &[(&str, &str)]) -> Result<Response, TransportError> {
    let mut all = vec![("content-type".to_owned(), "application/json".to_owned())];
    all.extend(
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned())),
    );
    Ok(Response {
        status: 200,
        headers: all,
        body: serde_json::to_vec(&body).unwrap(),
    })
}

fn data(value: Value) -> Result<Response, TransportError> {
    reply(json!({"code": 0, "data": value}), &[])
}

impl Transport for FeishuWeb {
    fn execute(&self, request: Request) -> lankide_provider_feishu::http::TransportFuture<'_> {
        Box::pin(async move {
            let url = request.path.clone();
            let hold = {
                let mut gate = self.1.lock().unwrap();
                if gate.as_ref().is_some_and(|h| url.ends_with(&h.suffix)) {
                    gate.take()
                } else {
                    None
                }
            };
            if let Some(hold) = hold {
                hold.entered.notify_one();
                hold.release.notified().await;
            }
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let cookie = header(&request, "cookie").to_owned();
            let mut feishu = self.0.lock().unwrap();
            if url.starts_with("https://accounts.feishu.cn/accounts/page/login") {
                return reply(
                    json!({}),
                    &[("set-cookie", "passport_web_did=device; Domain=.feishu.cn")],
                );
            }
            if url == "https://accounts.feishu.cn/accounts/qrlogin/init" {
                assert!(cookie.contains("passport_web_did=device"));
                feishu.qr_codes += 1;
                let token = format!("qr-{}", feishu.qr_codes);
                return reply(
                    json!({"code": 0, "data": {"step_info": {"token": token}}}),
                    &[("x-flow-key", "flow")],
                );
            }
            if url == "https://accounts.feishu.cn/accounts/qrlogin/polling" {
                assert_eq!(header(&request, "x-flow-key"), "flow");
                if feishu.qr_expired {
                    return data(json!({"next_step":"qr_login_polling","step_info":{"status":3}}));
                }
                return match feishu.login_stage {
                    0 | 1 => data(json!({
                        "next_step": "qr_login_polling",
                        "step_info": {"status": feishu.login_stage + 1},
                    })),
                    _ => reply(
                        json!({"code": 0, "data": {"next_step": "enter_app"}}),
                        &[(
                            "set-cookie",
                            &format!("session={}; Domain=.feishu.cn; Path=/", feishu.user),
                        )],
                    ),
                };
            }
            if url == "https://open.feishu.cn/app" {
                assert!(cookie.contains("session="));
                return Ok(Response {
                    status: 200,
                    headers: vec![],
                    body: br#"<script>window.csrfToken = "console-csrf";</script>"#.to_vec(),
                });
            }
            if let Some(call) = url.strip_prefix("https://open.feishu.cn/developers/v1/") {
                if !cookie.contains("session=")
                    || header(&request, "x-csrf-token") != "console-csrf"
                {
                    return Ok(Response {
                        status: 302,
                        headers: vec![("location".into(), "https://accounts.feishu.cn/".into())],
                        body: vec![],
                    });
                }
                let call = call.split('?').next().unwrap_or_default().to_owned();
                feishu.console_calls.push(call.clone());
                let app = format!("cli_console_{}", feishu.apps.len());
                return match call.split('/').next().unwrap_or_default() {
                    "app" if call == "app/list" => data(json!({"apps": []})),
                    "app" if call == "app/create" => {
                        if feishu.reject_app_create {
                            return reply(json!({"code": 10013, "msg": "denied"}), &[]);
                        }
                        let app = format!("cli_console_{}", feishu.apps.len() + 1);
                        feishu.apps.push(body["name"].as_str().unwrap().to_owned());
                        data(json!({"ClientID": app}))
                    }
                    "app" if call == "app/upload/image" => data(json!({"url": "https://icon"})),
                    "secret" => data(json!({"secret": format!("{app}-secret")})),
                    "event" if call.starts_with("event/update") => {
                        assert_eq!(body["events"], json!(STANDARD_BOT_EVENTS));
                        data(json!({}))
                    }
                    "scope" if call.starts_with("scope/all") => data(json!({
                        "scopes": STANDARD_BOT_TENANT_SCOPES
                            .iter()
                            .chain(["task:tasklist:read", "task:tasklist:write"].iter())
                            .enumerate()
                            .map(|(id, name)| json!({"id": id.to_string(), "name": name}))
                            .collect::<Vec<_>>(),
                    })),
                    "app_version" => data(json!({"versionId": "v1"})),
                    "publish" => data(json!({"isOk": true})),
                    "robot" | "event" | "scope" | "safe_setting" | "base_info" => data(json!({})),
                    _ => panic!("unexpected console call {call}"),
                };
            }
            if url.starts_with("https://accounts.feishu.cn/open-apis/authen/v1/authorize") {
                assert!(cookie.contains("session="));
                let page = r#"<script>window.serverInjectRes = {"data":{"current_user":{"user_id":"lead","all_scope_key_list":["task:tasklist:read","task:tasklist:write","task:task:read","task:task:write"]}},"X-Auth-Flow-Key":"consent"};
</script>"#;
                return Ok(Response {
                    status: 200,
                    headers: vec![(
                        "set-cookie".into(),
                        "open_csrf_token=consent-csrf; Path=/".into(),
                    )],
                    body: page.as_bytes().to_vec(),
                });
            }
            if url == "https://accounts.feishu.cn/open-apis/authen/v1/confirm" {
                assert_eq!(header(&request, "open-csrf-token"), "consent-csrf");
                assert_eq!(header(&request, "x-auth-flow-key"), "consent");
                return data(json!({
                    "redirect_uri": format!("http://127.0.0.1/lankide/feishu-oauth?code=granted-{}&state=lankide",cookie.split("session=").nth(1).unwrap().split(';').next().unwrap()),
                }));
            }
            if url.ends_with("/open-apis/authen/v2/oauth/token") {
                let user = body["code"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("granted-")
                    .unwrap()
                    .to_owned();
                let token = format!("token-{user}-{}", body["client_id"].as_str().unwrap());
                feishu.tokens.insert(token.clone(), user);
                return reply(
                    json!({"code": 0, "access_token": token, "expires_in": 7200}),
                    &[],
                );
            }
            if url.ends_with("/open-apis/authen/v1/user_info") {
                if feishu.expired {
                    return reply(json!({"code":99991663,"msg":"expired"}), &[]);
                }
                let user = feishu
                    .tokens
                    .get(
                        header(&request, "authorization")
                            .strip_prefix("Bearer ")
                            .unwrap(),
                    )
                    .unwrap();
                let mut info = json!({"open_id":format!("ou_{}",user),"union_id":format!("on_{}",user),"user_id":user,"name":feishu.name,"tenant_key":feishu.tenant});
                if !feishu.no_owner_avatar {
                    info["avatar_url"] = json!("https://avatar/owner.jpg");
                }
                return data(info);
            }
            if url.ends_with("/oauth/v1/app/registration") {
                if request.body.starts_with(b"action=poll") {
                    return reply(
                        json!({"client_id":"cli_registered", "client_secret":"registered-secret",
                        "user_info":{"open_id":"ou_lead"}}),
                        &[],
                    );
                }
                return reply(
                    json!({
                        "device_code": "device",
                        "verification_uri_complete": "https://open.feishu.cn/page/cli?code=device",
                        "interval": 5,
                        "expires_in": 600,
                    }),
                    &[],
                );
            }
            if url.ends_with("/auth/v3/tenant_access_token/internal") {
                return reply(
                    json!({"code": 0, "tenant_access_token": "tenant", "expire": 7200}),
                    &[],
                );
            }
            // Profile sync of an Agent app: avatar upload, base update and publish.
            if url.ends_with("/open-apis/application/v7/app_avatar/upload") {
                return data(json!({"url": "https://avatar/app.jpg"}));
            }
            if url.contains("/open-apis/application/v7/applications/") && url.ends_with("/base") {
                return data(json!({}));
            }
            if url.contains("/open-apis/application/v7/applications/") && url.ends_with("/publish")
            {
                return data(json!({"version_id": "v2", "version": "1.0.1"}));
            }
            if url.contains("/app_versions") {
                return data(json!({"items": [{
                    "version": "1.0.0",
                    "status": 1,
                    "publish_time": "1700000000",
                    "scopes": STANDARD_BOT_TENANT_SCOPES
                        .iter()
                        .map(|scope| json!({"scope": scope, "token_types": ["tenant"]}))
                        .collect::<Vec<_>>(),
                    "event_infos": STANDARD_BOT_EVENTS
                        .iter()
                        .map(|event| json!({"event_type": event}))
                        .collect::<Vec<_>>(),
                }]}));
            }
            if url.contains("/open-apis/contact/v3/users/") {
                return data(
                    json!({"user":{"open_id":"ou_lead","union_id":"on_lead","name":"Alice"}}),
                );
            }
            if url.contains("/open-apis/im/v1/messages") && request.method == Method::Post {
                let original = url
                    .split("/messages/")
                    .nth(1)
                    .and_then(|v| v.split('/').next());
                let chat = original
                    .and_then(|id| feishu.message_chats.get(id))
                    .cloned()
                    .or_else(|| body["receive_id"].as_str().map(str::to_owned))
                    .ok_or_else(|| TransportError::Failed("unknown mock message target".into()))?;
                let content: Value =
                    serde_json::from_str(body["content"].as_str().unwrap_or("{}")).unwrap();
                let channel_type = feishu
                    .chat_types
                    .get(&chat)
                    .cloned()
                    .ok_or_else(|| TransportError::Failed("unknown mock chat type".into()))?;
                feishu
                    .sends
                    .push(json!({"chat":chat,"content":content,"channel_type":channel_type}));
                return data(
                    json!({"message_id":format!("web-reply-{}",feishu.sends.len()),"chat_id":chat}),
                );
            }
            if url.ends_with("/open-apis/im/v1/chats/owner_dm") {
                return data(json!({"name":"Owner","chat_mode":"p2p"}));
            }
            if url.ends_with("/open-apis/im/v1/chats/oc_team") {
                return data(json!({"name":"Team"}));
            }
            if url.ends_with("/open-apis/bot/v3/info") {
                return reply(json!({"code": 0, "bot": {"open_id": "ou_bot"}}), &[]);
            }
            if url == "https://avatar/owner.jpg" {
                return Ok(Response {
                    status: 200,
                    headers: vec![("content-type".into(), "image/jpeg".into())],
                    body: b"avatar".to_vec(),
                });
            }
            if url.contains("/open-apis/task/")
                && header(&request, "authorization").starts_with("Bearer token-")
            {
                feishu.owner_task_calls += 1;
                if let Some(code) = feishu.task_error
                    && feishu
                        .task_error_suffix
                        .as_ref()
                        .is_none_or(|suffix| url.ends_with(suffix))
                {
                    return reply(json!({"code":code,"msg":"mock task failure"}), &[]);
                }
                if let Some(status) = feishu.task_http_status {
                    return Ok(Response {
                        status,
                        headers: vec![],
                        body: vec![],
                    });
                }
            }
            if url.ends_with("/task/v2/task_v2/task_subscription") {
                assert_eq!(request.method, Method::Post);
                feishu.task_subscriptions += 1;
                return data(json!({}));
            }
            if url.ends_with("/team_tasklist/tasks") {
                let items: Vec<_> = feishu
                    .tasks
                    .iter()
                    .filter(|task| {
                        !feishu.denied_tasks.iter().any(|id| task["guid"] == *id)
                            && (task["tasklist_guid"] == "team_tasklist"
                                || task["tasklists"].as_array().is_some_and(|lists| {
                                    lists
                                        .iter()
                                        .any(|list| list["tasklist_guid"] == "team_tasklist")
                                }))
                    })
                    .collect();
                return data(json!({"items":items,"has_more":false}));
            }
            if url.ends_with("/task/v2/tasks") && request.method == Method::Post {
                let mut task = body.clone();
                task["guid"] = json!(format!("task-{}", feishu.tasks.len()));
                task["completed_at"] = json!("0");
                feishu.tasks.push(task.clone());
                return data(json!({"task":task}));
            }
            if let Some(id) = url.split("/task/v2/tasks/").nth(1) {
                if feishu.denied_tasks.iter().any(|t| t == id) {
                    return reply(json!({"code":1470403,"msg":"denied"}), &[]);
                }
                let Some(task) = feishu.tasks.iter_mut().find(|t| t["guid"] == id) else {
                    return reply(json!({"code":1470404,"msg":"deleted"}), &[]);
                };
                if request.method == Method::Patch {
                    task["completed_at"] = body["task"]["completed_at"].clone();
                }
                return data(json!({"task":task}));
            }
            if url.contains("/open-apis/task/v2/tasklists") {
                let caller = header(&request, "authorization").to_owned();
                if request.method == Method::Post && url.ends_with("/task/v2/tasklists") {
                    feishu.tasklist_creates.push((caller, body.clone()));
                    let tasklist = json!({
                        "guid": "team_tasklist",
                        "name": body["name"],
                        "owner": {"id": "ou_lead", "type": "user", "role": "owner"},
                        "members": body["members"],
                    });
                    feishu.tasklist = Some(tasklist.clone());
                    return data(json!({"tasklist": tasklist}));
                }
                if url.ends_with("/team_tasklist/add_members") {
                    feishu.tasklist_member_adds.push((caller, body.clone()));
                    let tasklist = feishu.tasklist.as_mut().unwrap();
                    let members = tasklist["members"].as_array_mut().unwrap();
                    members.extend(body["members"].as_array().unwrap().iter().cloned());
                    return data(json!({"tasklist": tasklist}));
                }
                if url.ends_with("/team_tasklist") {
                    return data(json!({"tasklist": feishu.tasklist}));
                }
            }
            Err(TransportError::Failed(format!(
                "unexpected Feishu request {url}"
            )))
        })
    }
}

pub struct Source(pub FeishuWeb);
impl lankide_provider_feishu::events::EventSource for Source {
    fn next(
        &mut self,
    ) -> lankide_provider_feishu::events::EventFuture<
        '_,
        Result<
            Option<lankide_provider_feishu::events::RawEvent>,
            lankide_provider_feishu::events::SourceError,
        >,
    > {
        Box::pin(async move {
            loop {
                let raw = self.0.0.lock().unwrap().events.pop_front();
                if let Some(raw) = raw {
                    return Ok(Some(raw));
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
    }
    fn acknowledge(
        &mut self,
    ) -> lankide_provider_feishu::events::EventFuture<
        '_,
        Result<(), lankide_provider_feishu::events::SourceError>,
    > {
        Box::pin(async {
            self.0.0.lock().unwrap().acknowledgements += 1;
            Ok(())
        })
    }
    fn close(&mut self) -> lankide_provider_feishu::events::EventFuture<'_, ()> {
        Box::pin(async {})
    }
}
