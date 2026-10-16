//! New Agents through the Feishu developer console: the team lead scans one
//! sign-in QR code, then each new Agent gets its own App, published and
//! joined to the lead's Team Tasklist without further confirmations. Only the
//! Feishu web and OpenAPI boundary is simulated.

#[allow(dead_code)]
#[path = "support/host.rs"]
mod host;

use host::Services;
use lankide_core::AgentId;
use lankide_provider::*;
use lankide_provider_feishu::{
    CONSOLE_LOGIN_INTERFACE, FeishuProvider, Method, Request, Response, STANDARD_BOT_EVENTS,
    STANDARD_BOT_TENANT_SCOPES, Transport, TransportError,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Secrets(Mutex<BTreeMap<String, String>>);

impl SecretReader for Secrets {
    fn read<'a>(&'a self, name: &'a str) -> ProviderFuture<'a, SecretValue> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .get(name)
                .map(SecretValue::new)
                .ok_or(ProviderError::SecretAccessDenied)
        })
    }
}

impl SecretWriter for Secrets {
    fn write<'a>(&'a self, name: &'a str, value: SecretValue) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .insert(name.to_owned(), value.expose_secret().to_owned());
            Ok(())
        })
    }
}

#[derive(Default)]
struct Feishu {
    /// 0 waiting for a scan, 1 scanned, 2 confirmed on the phone.
    login_stage: u8,
    qr_codes: usize,
    reject_app_create: bool,
    apps: Vec<String>,
    console_calls: Vec<String>,
    tasklist: Option<Value>,
    tasklist_creates: Vec<(String, Value)>,
    tasklist_member_adds: Vec<(String, Value)>,
}

#[derive(Clone, Default)]
struct FeishuWeb(Arc<Mutex<Feishu>>);

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
                return match feishu.login_stage {
                    0 | 1 => data(json!({
                        "next_step": "qr_login_polling",
                        "step_info": {"status": feishu.login_stage + 1},
                    })),
                    _ => reply(
                        json!({"code": 0, "data": {"next_step": "enter_app"}}),
                        &[("set-cookie", "session=lead; Domain=.feishu.cn; Path=/")],
                    ),
                };
            }
            if url == "https://open.feishu.cn/app" {
                assert!(cookie.contains("session=lead"));
                return Ok(Response {
                    status: 200,
                    headers: vec![],
                    body: br#"<script>window.csrfToken = "console-csrf";</script>"#.to_vec(),
                });
            }
            if let Some(call) = url.strip_prefix("https://open.feishu.cn/developers/v1/") {
                if !cookie.contains("session=lead")
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
                assert!(cookie.contains("session=lead"));
                let page = r#"<script>window.serverInjectRes = {"data":{"current_user":{"user_id":"lead","all_scope_key_list":["task:tasklist:read","task:tasklist:write"]}},"X-Auth-Flow-Key":"consent"};
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
                    "redirect_uri": "http://127.0.0.1/lankide/feishu-oauth?code=granted&state=lankide",
                }));
            }
            if url.ends_with("/open-apis/authen/v2/oauth/token") {
                assert_eq!(body["code"], "granted");
                return reply(
                    json!({"code": 0, "access_token": "lead-token", "expires_in": 7200}),
                    &[],
                );
            }
            if url.ends_with("/open-apis/authen/v1/user_info") {
                assert_eq!(header(&request, "authorization"), "Bearer lead-token");
                return data(
                    json!({"open_id": "ou_lead", "union_id": "on_lead", "user_id": "lead"}),
                );
            }
            if url.ends_with("/oauth/v1/app/registration") {
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
            if url.ends_with("/open-apis/bot/v3/info") {
                return reply(json!({"code": 0, "bot": {"open_id": "ou_bot"}}), &[]);
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

struct Workspace {
    host: ProviderHost<FeishuProvider>,
    feishu: FeishuWeb,
    _services: Arc<Services>,
}

async fn workspace(task_system_enabled: bool) -> Workspace {
    let feishu = FeishuWeb::default();
    let services = Services::new("feishu");
    let secrets = Arc::new(Secrets::default());
    let mut host_services = services.host();
    host_services.secret_reader = secrets.clone();
    host_services.secret_writer = secrets;
    let host = ProviderHost::new(
        FeishuProvider::new()
            .without_event_source()
            .with_transport(feishu.clone()),
        services.workspace.clone(),
        json!({"domain": "feishu", "task_system_enabled": task_system_enabled}),
        host_services,
    )
    .unwrap();
    host.init().await.unwrap();
    Workspace {
        host,
        feishu,
        _services: services,
    }
}

impl Workspace {
    async fn sign_in(&self, action: &str) -> Value {
        self.host
            .invoke_interface(CONSOLE_LOGIN_INTERFACE, json!({"action": action}))
            .await
            .unwrap()
    }

    async fn onboard(&self, agent: &str, action: &str, payload: Value) -> ProviderOnboarding {
        self.host
            .agent_onboarding(
                AgentId::new(agent).unwrap(),
                agent.to_uppercase(),
                json!({"mode": "new", "domain": "feishu"}),
                action.into(),
                payload,
            )
            .await
            .unwrap()
    }

    fn stage(&self, stage: u8) {
        self.feishu.0.lock().unwrap().login_stage = stage;
    }
}

#[tokio::test]
async fn one_sign_in_creates_every_agent_app_and_shares_the_leads_tasklist() {
    let workspace = workspace(true).await;
    assert_eq!(workspace.sign_in("status").await["state"], "required");

    let shown = workspace.sign_in("start").await;
    assert_eq!(shown["state"], "waiting");
    assert_eq!(shown["qr_code"], r#"{"qrlogin":{"token":"qr-1"}}"#);
    workspace.stage(1);
    assert_eq!(workspace.sign_in("poll").await["state"], "scanned");
    workspace.stage(2);
    assert_eq!(workspace.sign_in("poll").await["state"], "ready");
    assert_eq!(workspace.sign_in("status").await["state"], "ready");

    let creating = workspace.onboard("alpha", "start", json!({})).await;
    assert_eq!(creating.step, "permissions");
    assert_eq!(creating.status, ProviderOnboardingStatus::Verifying);
    assert!(creating.verification_uri.is_none());
    assert!(creating.poll_interval_seconds.is_some());
    let alpha = workspace
        .onboard("alpha", "poll", creating.opaque_payload)
        .await;
    assert_eq!(alpha.status, ProviderOnboardingStatus::Ready);
    assert_eq!(alpha.agent_config_updates["app_id"], "cli_console_1");
    assert_eq!(alpha.agent_config_updates["creator_open_id"], "ou_lead");
    assert!(alpha.agent_config_updates["creator_person_id"].is_string());

    let beta = workspace.onboard("beta", "start", json!({})).await;
    let beta = workspace.onboard("beta", "poll", beta.opaque_payload).await;
    assert_eq!(beta.status, ProviderOnboardingStatus::Ready);
    assert_eq!(beta.agent_config_updates["app_id"], "cli_console_2");

    let feishu = workspace.feishu.0.lock().unwrap();
    assert_eq!(feishu.qr_codes, 1);
    assert_eq!(feishu.apps, ["ALPHA", "BETA"]);
    for step in ["app/upload/image", "publish/commit/cli_console_1/v1"] {
        assert!(feishu.console_calls.iter().any(|call| call == step));
    }
    assert_eq!(
        feishu.tasklist_creates,
        [(
            "Bearer lead-token".to_owned(),
            json!({"name": "团队任务", "members": [{"id": "cli_console_1", "type": "app", "role": "editor"}]}),
        )]
    );
    assert_eq!(
        feishu.tasklist_member_adds,
        [(
            "Bearer lead-token".to_owned(),
            json!({"members": [{"id": "cli_console_2", "type": "app", "role": "editor"}]}),
        )]
    );
}

#[tokio::test]
async fn creating_an_agent_asks_for_the_sign_in_it_still_needs() {
    let workspace = workspace(false).await;

    let shown = workspace.onboard("alpha", "start", json!({})).await;
    assert_eq!(shown.step, "console_login");
    assert_eq!(shown.status, ProviderOnboardingStatus::ActionRequired);
    assert_eq!(
        shown.opaque_payload["qr_code"],
        r#"{"qrlogin":{"token":"qr-1"}}"#
    );
    assert!(shown.poll_interval_seconds.is_some());
    workspace.stage(1);
    let scanned = workspace
        .onboard("alpha", "poll", shown.opaque_payload)
        .await;
    assert_eq!(scanned.step, "console_login");
    assert_eq!(scanned.status, ProviderOnboardingStatus::Verifying);
    workspace.stage(2);
    let creating = workspace
        .onboard("alpha", "poll", scanned.opaque_payload)
        .await;
    assert_eq!(creating.step, "permissions");
    // Reopening the dialog continues with the App already created.
    let resumed = workspace.onboard("alpha", "start", json!({})).await;
    assert_eq!(resumed.status, ProviderOnboardingStatus::Ready);
    assert_eq!(resumed.agent_config_updates["creator_open_id"], "ou_lead");

    let feishu = workspace.feishu.0.lock().unwrap();
    assert_eq!(feishu.apps, ["ALPHA"]);
    assert!(feishu.tasklist_creates.is_empty());
}

#[tokio::test]
async fn a_console_failure_falls_back_to_confirming_in_feishu() {
    let workspace = workspace(false).await;
    workspace.stage(2);
    assert_eq!(workspace.sign_in("start").await["state"], "waiting");
    assert_eq!(workspace.sign_in("poll").await["state"], "ready");
    workspace.feishu.0.lock().unwrap().reject_app_create = true;

    let confirm = workspace.onboard("alpha", "start", json!({})).await;
    assert_eq!(confirm.step, "create_app");
    assert_eq!(
        confirm.verification_uri.as_deref(),
        Some("https://open.feishu.cn/page/cli?code=device")
    );
    assert!(workspace.feishu.0.lock().unwrap().apps.is_empty());
}
