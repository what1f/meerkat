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
    pub qr_codes: usize,
    pub name: String,
    pub user: String,
    pub tenant: String,
    pub contact_unavailable: bool,
    pub requests: Vec<String>,
    pub app_creators: std::collections::BTreeMap<String, String>,
    pub tasklist_transfers: Vec<Value>,
    pub tasklist_renames: Vec<(String, Value)>,
    pub tasklist_rename_error: bool,
    pub task_error: Option<i64>,
    pub tasks: Vec<Value>,
    pub task_subscriptions: usize,
    pub acknowledgements: usize,
    pub denied_tasks: Vec<String>,
    pub events: std::collections::VecDeque<lankide_provider_feishu::events::RawEvent>,
    /// Contact profile has no picture.
    pub no_member_avatar: bool,
    pub apps: Vec<String>,
    pub console_calls: Vec<String>,
    pub configuration_error: Option<i64>,
    pub version_status: Option<i64>,
    pub event_polls: usize,
    pub app_authorization: std::collections::BTreeMap<String, AppAuthorization>,
    pub tasklist: Option<Value>,
    pub tasklist_creates: Vec<(String, Value)>,
    pub tasklist_member_adds: Vec<(String, Value)>,
}

#[derive(Default)]
pub struct AppAuthorization {
    pub scopes: Vec<Value>,
    pub published: bool,
    pub versions: Vec<String>,
}

#[derive(Clone, Default)]
pub struct FeishuWeb(pub Arc<Mutex<Feishu>>);

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
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let cookie = header(&request, "cookie").to_owned();
            let mut feishu = self.0.lock().unwrap();
            feishu.requests.push(url.clone());
            assert!(
                !url.contains("/authen/v1/authorize") && !url.contains("/authen/v2/oauth/token")
            );
            assert!(!url.contains("safe_setting"));
            assert!(!url.contains("/napi/profile") && !url.contains("/oauth/"));
            if url == "https://open.feishu.cn/napi/check/login" {
                assert!(cookie.contains("session="));
                return data(json!({"tenantId":feishu.tenant}));
            }
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
                if call.starts_with("scope/update")
                    && let Some(code) = feishu.configuration_error
                {
                    return reply(
                        json!({"code":code,"msg":"denied sensitive-cookie sensitive-secret sensitive-token"}),
                        &[],
                    );
                }
                let app = format!("cli_console_{}", feishu.apps.len());
                return match call.split('/').next().unwrap_or_default() {
                    "app" if call == "app/create" => {
                        let app = format!("cli_console_{}", feishu.apps.len() + 1);
                        let user = cookie
                            .split("session=")
                            .nth(1)
                            .unwrap()
                            .split(';')
                            .next()
                            .unwrap()
                            .to_owned();
                        feishu.app_creators.insert(app.clone(), user);
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
                            .enumerate()
                            .map(|(id, name)| json!({"id": id.to_string(), "name": name}))
                            .collect::<Vec<_>>(),
                    })),
                    "scope" if call.starts_with("scope/update") => {
                        assert_eq!(body["userScopeIDs"], json!([]));
                        let app = call.split('/').nth(2).unwrap();
                        let config = feishu.app_authorization.entry(app.into()).or_default();
                        for scope in body["appScopeIDs"].as_array().unwrap() {
                            if !config.scopes.contains(scope) {
                                config.scopes.push(scope.clone());
                            }
                        }
                        config.published = false;
                        data(json!({}))
                    }
                    "app_version" => {
                        let app = call.split('/').nth(2).unwrap();
                        let status = feishu.version_status;
                        let config = feishu.app_authorization.entry(app.into()).or_default();
                        if call.starts_with("app_version/list") {
                            return data(
                                json!({"versions":config.versions.iter().enumerate().map(|(i, version)| json!({"appVersion":version,"versionId":format!("v{}",i+1),"versionStatus":status.unwrap_or(if config.published { 2 } else { 1 })})).collect::<Vec<_>>()}),
                            );
                        }
                        let name = body["appVersion"].as_str().unwrap().to_owned();
                        assert!(!config.versions.contains(&name), "version must be new");
                        config.versions.push(name);
                        data(json!({"versionId":format!("v{}",config.versions.len())}))
                    }
                    "publish" => {
                        let app = call.split('/').nth(2).unwrap();
                        let config = feishu.app_authorization.entry(app.into()).or_default();
                        assert_eq!(config.scopes.len(), STANDARD_BOT_TENANT_SCOPES.len());
                        config.published = true;
                        data(json!({"isOk": true}))
                    }
                    "robot" | "event" | "scope" | "base_info" => data(json!({})),
                    _ => panic!("unexpected console call {call}"),
                };
            }
            if url.ends_with("/auth/v3/tenant_access_token/internal") {
                return reply(
                    json!({"code": 0, "tenant_access_token": format!("tenant-{}",body["app_id"].as_str().unwrap()), "expire": 7200}),
                    &[],
                );
            }
            if url.contains("/open-apis/application/v6/applications/me") {
                let app = header(&request, "authorization")
                    .strip_prefix("Bearer tenant-")
                    .unwrap();
                let user = feishu
                    .app_creators
                    .get(app)
                    .map(String::as_str)
                    .unwrap_or("lead");
                let prefix = if request
                    .query
                    .iter()
                    .any(|(k, v)| k == "user_id_type" && v == "union_id")
                {
                    "on"
                } else {
                    "ou"
                };
                let id = if prefix == "on" {
                    format!("on_{user}")
                } else {
                    format!("ou_{user}_{app}")
                };
                return data(json!({"app":{"creator_id":id}}));
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
                assert!(header(&request, "authorization").starts_with("Bearer tenant-"));
                if feishu.contact_unavailable {
                    return reply(json!({"code":99991672,"msg":"no contact access"}), &[]);
                }
                let app = header(&request, "authorization").trim_start_matches("Bearer tenant-");
                let id = url.rsplit('/').next().unwrap();
                let user_id = id
                    .strip_prefix("on_")
                    .or_else(|| id.strip_prefix("ou_").map(|s| s.split('_').next().unwrap()))
                    .unwrap_or("lead");
                let mut user = json!({"open_id":format!("ou_{user_id}_{app}"),"union_id":format!("on_{user_id}"),"name":feishu.name});
                if !feishu.no_member_avatar {
                    user["avatar"] = json!({"avatar_origin":"https://avatar/member.jpg"});
                }
                return data(json!({"user":user}));
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
                let app = header(&request, "authorization")
                    .strip_prefix("Bearer tenant-")
                    .unwrap();
                return reply(
                    json!({"code": 0, "bot": {"open_id": format!("ou_bot_{app}")}}),
                    &[],
                );
            }
            if url == "https://avatar/member.jpg" {
                return Ok(Response {
                    status: 200,
                    headers: vec![("content-type".into(), "image/png".into())],
                    body: include_bytes!("../../../../providers/feishu/assets/app-icon.png")
                        .to_vec(),
                });
            }
            if url.contains("/open-apis/task/") {
                assert!(header(&request, "authorization").starts_with("Bearer tenant-"));
                if let Some(code) = feishu.task_error {
                    return reply(json!({"code":code}), &[]);
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
                if let Some(members) = body["members"].as_array() {
                    for member in members.iter().filter(|v| v["type"] == "user") {
                        assert!(
                            member["id"]
                                .as_str()
                                .unwrap()
                                .ends_with(caller.trim_start_matches("Bearer tenant-")),
                            "user IDs belong to the calling App"
                        );
                    }
                }
                if request.method == Method::Post && url.ends_with("/task/v2/tasklists") {
                    feishu.tasklist_creates.push((caller.clone(), body.clone()));
                    let tasklist = json!({
                        "guid": "team_tasklist",
                        "name": body["name"],
                        "owner": {"id": caller.trim_start_matches("Bearer tenant-"), "type": "app", "role": "owner"},
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
                if url.ends_with("/task/v2/tasklists") && request.method == Method::Get {
                    return data(
                        json!({"items":feishu.tasklist.iter().collect::<Vec<_>>(),"has_more":false}),
                    );
                }
                if url.ends_with("/team_tasklist") {
                    if request.method == Method::Patch {
                        if body["update_fields"] == json!(["name"]) {
                            feishu.tasklist_renames.push((caller.clone(), body.clone()));
                            if feishu.tasklist_rename_error {
                                return reply(json!({"code": 403, "msg": "rename denied"}), &[]);
                            }
                            feishu.tasklist.as_mut().unwrap()["name"] =
                                body["tasklist"]["name"].clone();
                        }
                        if body["update_fields"]
                            .as_array()
                            .is_some_and(|v| v.contains(&json!("owner")))
                        {
                            assert_eq!(body["origin_owner_to_role"], "editor");
                            feishu.tasklist_transfers.push(body.clone());
                            feishu.tasklist.as_mut().unwrap()["owner"] =
                                body["tasklist"]["owner"].clone();
                        }
                    }
                    return data(json!({"tasklist": feishu.tasklist}));
                }
            }
            Err(TransportError::Failed(format!(
                "unexpected Feishu request {url}"
            )))
        })
    }
}

pub struct Source(pub FeishuWeb, pub String);
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
                let raw = {
                    let mut state = self.0.0.lock().unwrap();
                    state.event_polls += 1;
                    let index = state.events.iter().position(|raw| {
                        serde_json::from_slice::<Value>(&raw.body)
                            .ok()
                            .is_some_and(|v| v["header"]["app_id"] == self.1)
                    });
                    index.and_then(|index| state.events.remove(index))
                };
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
