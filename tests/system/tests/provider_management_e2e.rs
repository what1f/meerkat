#[path = "support/feishu_web.rs"]
mod feishu_web;
use feishu_web::FeishuWeb;
use futures_util::StreamExt;
use lankide_provider::{Provider, ProviderHost};
use lankide_provider_feishu::FeishuProvider;
use lankide_runtime::ProviderFactoryRegistry;
use lankide_system_e2e::{Result, RigConfig, SystemRig};
use serde_json::{Value, json};
use std::time::Duration;

struct Http {
    url: String,
    client: reqwest::Client,
}
impl Http {
    fn new(rig: &SystemRig) -> Result<Self> {
        Ok(Self {
            url: std::fs::read_to_string(rig.runtime_home().join("run/management.url"))?,
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?,
        })
    }
    async fn post(&self, op: &str, body: Value) -> Result<Value> {
        let r = self
            .client
            .post(format!("{}/api/v1/{op}", self.url))
            .json(&body)
            .send()
            .await?;
        let status = r.status();
        let body: Value = r.json().await?;
        if !status.is_success() {
            return Err(format!("{op}: {status}: {body}").into());
        }
        Ok(body)
    }
    async fn invoke(&self, id: &str, action: &str, extra: Value) -> Result<Value> {
        let mut input = extra;
        input["action"] = json!(action);
        let result=self.post("invoke_provider_interface",json!({"request_id":"invoke","workspace":{"workspace_id":"team"},"interface_id":id,"input_json":input.to_string()})).await?;
        Ok(serde_json::from_str(
            result["output_json"].as_str().ok_or("output_json")?,
        )?)
    }
}
async fn event(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    kind: &str,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = ws.next().await.ok_or("events closed")??;
            let value: Value = serde_json::from_str(frame.to_text()?)?;
            if value["data"]["kind"] == kind {
                break Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
        }
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn a4b_feishu_owner_switch_and_task_list() -> Result<()> {
    let web = FeishuWeb::default();
    {
        let mut state = web.0.lock().unwrap();
        state.name = "Alice".into();
        state.user = "lead".into();
        state.tenant = "tenant-a".into();
        state.login_stage = 2;
    }
    let factories = ProviderFactoryRegistry::default();
    let transport = web.clone();
    factories.register_host_factory(FeishuProvider::new().info(), move |w, s| {
        ProviderHost::new(
            FeishuProvider::new()
                .with_transport(transport.clone())
                .with_event_source_factory({
                    let transport = transport.clone();
                    move |_, _, _, _| {
                        let source = feishu_web::Source(transport.clone());
                        Box::pin(async move {
                            Ok(Box::new(source)
                                as Box<dyn lankide_provider_feishu::events::EventSource>)
                        })
                    }
                }),
            w.id.clone(),
            w.provider_config.clone(),
            s,
        )
    })?;
    let config = RigConfig::default();
    let harness = config.harness_executable.clone();
    let rig = SystemRig::start_with_providers(config, factories).await?;
    let scenario=async {
        let http=Http::new(&rig)?;
        http.post("create_workspace",json!({"request_id":"create","workspace_id":"team","name":"Team","rules":"","provider_id":"feishu","provider_config_json":"{\"domain\":\"feishu\",\"task_system_enabled\":true}","provider_secrets_json":"{}"})).await?;
        let owner="feishu.workspace.owner";let login="feishu.workspace.console_login";
        assert_eq!(http.invoke(owner,"get",json!({})).await?["state"],"unbound");
        http.invoke(login,"start",json!({})).await?;http.invoke(login,"poll",json!({})).await?;
        assert_eq!(http.invoke(owner,"get",json!({})).await?["state"],"unbound");
        let machines=http.post("get_machine_catalog",json!({"request_id":"m","workspace":null})).await?;
        let request=lankide_protocol::CreateAgentRequest{request_id:"agent".into(),agent:Some(lankide_protocol::AgentRef{workspace_id:"team".into(),agent_id:"alpha".into()}),name:"Alpha".into(),machine_id:machines["machines"][0]["machine_id"].as_str().unwrap().into(),harness_id:"pi".into(),model:"scripted".into(),thinking_level:"low".into(),cli_path:harness.to_string_lossy().into(),provider_agent_config_json:json!({"mode":"new","domain":"feishu"}).to_string(),provider_agent_secrets_json:"{}".into(),..Default::default()};
        http.post("create_agent",serde_json::to_value(request)?).await?;
        for action in ["start","start"] {http.post("advance_agent_onboarding",json!({"request_id":action,"agent":{"workspace_id":"team","agent_id":"alpha"},"action":action,"field_values_json":"{}"})).await?;}
        let first=http.invoke(owner,"get",json!({})).await?;assert_eq!(first["name"],"Alice");assert!(first["avatar"]["resource_id"].is_number());
        let (mut events,_)=tokio_tungstenite::connect_async(format!("{}/api/v1/events?workspace=team",http.url.replacen("http://","ws://",1))).await?;
        web.0.lock().unwrap().name="Alice Updated".into();
        assert_eq!(http.invoke(owner,"refresh",json!({})).await?["name"],"Alice Updated");event(&mut events,"workspace_updated").await?;
        web.0.lock().unwrap().expired=true;let expired=http.invoke(owner,"refresh",json!({})).await?;assert_eq!(expired["state"],"reauth_required");assert_eq!(expired["name"],"Alice Updated");web.0.lock().unwrap().expired=false;
        let waiting=http.invoke(login,"switch_start",json!({})).await?;
        assert_eq!(waiting["state"],"waiting");
        assert!(waiting["poll_interval_seconds"].as_u64().is_some_and(|seconds| seconds > 0));
        http.invoke(login,"switch_cancel",json!({})).await?;assert_eq!(http.invoke(owner,"get",json!({})).await?["person_id"],first["person_id"]);
        http.invoke(login,"switch_start",json!({})).await?;{let mut state=web.0.lock().unwrap();state.user="other".into();state.tenant="tenant-b".into();}
        assert!(http.invoke(login,"switch_poll",json!({})).await.is_err());assert_eq!(http.invoke(owner,"get",json!({})).await?["person_id"],first["person_id"]);
        web.0.lock().unwrap().tenant="tenant-a".into();http.invoke(login,"switch_start",json!({})).await?;http.invoke(login,"switch_poll",json!({})).await?;
        assert_ne!(http.invoke(owner,"get",json!({})).await?["person_id"],first["person_id"]);
        web.0.lock().unwrap().tasks.push(json!({"guid":"existing-task","summary":"Existing item","completed_at":"0","tasklist_guid":"team_tasklist"}));
        let tasks="feishu.workspace.tasks";assert_eq!(http.invoke(tasks,"list",json!({})).await?["items"][0]["text"],"Existing item");
        let item=http.invoke(tasks,"create",json!({"text":"Ship it"})).await?;assert_eq!(item["text"],"Ship it");
        assert_eq!(http.invoke(tasks,"list",json!({})).await?["items"].as_array().unwrap().len(),2);
        for completed in [true,false] {assert_eq!(http.invoke(tasks,"set_completed",json!({"id":item["id"],"completed":completed})).await?["completed"],completed);}
        assert!(web.0.lock().unwrap().task_subscriptions > 0);
        // External callback ordering and negative cases have a dedicated scenario below.
        drop(events);
        for enabled in [false,true] {
            http.post("update_workspace",json!({"request_id":"toggle","workspace":{"workspace_id":"team"},"provider_config_json":json!({"domain":"feishu","task_system_enabled":enabled}).to_string()})).await?;
            let listed=http.invoke(tasks,"list",json!({})).await?;assert_eq!(listed["enabled"],enabled);if enabled{assert!(listed["items"].as_array().unwrap().iter().any(|t|t["id"]==item["id"]));}
        }
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;

    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
#[allow(dead_code)]
#[path = "support/im_http.rs"]
mod im_http;
#[path = "support/telegram_web.rs"]
mod telegram_web;
#[tokio::test]
async fn a4b_telegram_owner_and_first_reply_survive_restart() -> Result<()> {
    telegram_first_reply(false, false).await
}
#[tokio::test]
async fn a4b_first_reply_event_arrives_before_harness_finishes() -> Result<()> {
    telegram_first_reply(true, false).await
}
#[tokio::test]
async fn a4b_review_first_reply_across_executions() -> Result<()> {
    telegram_first_reply(false, true).await
}
async fn telegram_first_reply(held: bool, cross: bool) -> Result<()> {
    let web = telegram_web::TelegramWeb::start();
    let factories = ProviderFactoryRegistry::default();
    factories.register_host_factory(
        lankide_provider_telegram::TelegramProvider::new().info(),
        move |w, s| {
            ProviderHost::new(
                lankide_provider_telegram::TelegramProvider::new(),
                w.id.clone(),
                w.provider_config.clone(),
                s,
            )
        },
    )?;
    let config = RigConfig {
        retry: lankide_runtime::RetryConfig {
            base_ms: 20,
            max_ms: 50,
            ..Default::default()
        },
        ..Default::default()
    };
    let harness = config.harness_executable.clone();
    let mut rig = SystemRig::start_with_providers(config, factories).await?;
    let scenario=async {
        let http=Http::new(&rig)?;
        http.post("create_workspace",json!({"request_id":"team","workspace_id":"team","name":"Team","rules":"","provider_id":"telegram","provider_config_json":json!({"api_endpoint":web.http.endpoint,"poll_timeout_seconds":1}).to_string(),"provider_secrets_json":"{\"manager_bot_token\":\"100:manager\"}"})).await?;
        let machines=http.post("get_machine_catalog",json!({"request_id":"m","workspace":null})).await?;
        let home=rig.machine_home();
        let mut env=vec![("LANKIDE_SYSTEM_SCRIPT".to_string(),serde_json::to_string(&if cross {lankide_system_e2e::Script::DeferredPrivateReply{text:"Later reply".into()}} else if held {lankide_system_e2e::Script::ReplyBeforeTerminal{text:"Welcome".into()}}else{lankide_system_e2e::Script::reply("Welcome")})?)];
        for (key,name) in [("LANKIDE_SYSTEM_INPUTS","inputs.jsonl"),("LANKIDE_SYSTEM_CONTROLS","controls.jsonl"),("LANKIDE_SYSTEM_STEERS","steers.jsonl"),("LANKIDE_SYSTEM_RELEASE","release")] {env.push((key.into(),home.join(name).to_string_lossy().into()));}
        let request=lankide_protocol::CreateAgentRequest{request_id:"alpha".into(),agent:Some(lankide_protocol::AgentRef{workspace_id:"team".into(),agent_id:"alpha".into()}),name:"Alpha".into(),machine_id:machines["machines"][0]["machine_id"].as_str().unwrap().into(),harness_id:"pi".into(),model:"scripted".into(),thinking_level:"low".into(),cli_path:harness.to_string_lossy().into(),environment_variables:env.into_iter().map(|(key,value)|lankide_protocol::EnvironmentVariable{name:key,value}).collect(),provider_agent_config_json:"{\"mode\":\"new\",\"bot_username\":\"alpha_bot\"}".into(),provider_agent_secrets_json:"{}".into(),..Default::default()};
        http.post("create_agent",serde_json::to_value(request)?).await?;
        let owner="telegram.workspace.owner";
        assert_eq!(http.invoke(owner,"get",json!({})).await?["state"],"unbound");
        for action in ["start","poll"] {http.post("advance_agent_onboarding",json!({"request_id":action,"agent":{"workspace_id":"team","agent_id":"alpha"},"action":action,"field_values_json":"{}"})).await?;}
        let card=http.invoke(owner,"get",json!({})).await?;assert_eq!(card["name"],"Alice");assert!(card["avatar"]["resource_id"].is_number());
        web.state.lock().unwrap().name="Alice Updated".into();assert_eq!(http.invoke(owner,"refresh",json!({})).await?["name"],"Alice Updated");
        web.state.lock().unwrap().expired=true;let expired=http.invoke(owner,"refresh",json!({})).await?;assert_eq!(expired["state"],"reauth_required");assert_eq!(expired["avatar"],http.invoke(owner,"get",json!({})).await?["avatar"]);web.state.lock().unwrap().expired=false;
        let reply_request=json!({"request_id":"reply","agent":{"workspace_id":"team","agent_id":"alpha"}});
        assert_eq!(http.post("get_agent_first_reply",reply_request.clone()).await?["state"],"waiting");
        if cross {
            web.message(2,true,300);
            tokio::time::timeout(Duration::from_secs(10),async{loop{if rig.machine_home().join("inputs.jsonl.read").exists(){break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await?;
            rig.settle().await?;
            assert!(web.state.lock().unwrap().sends.is_empty(),"first execution must only read");
            assert_eq!(http.post("get_agent_first_reply",reply_request.clone()).await?["state"],"waiting");
        }
        if !held && !cross {
        web.message(2,false,300);
        tokio::time::timeout(Duration::from_secs(15),async{loop{if !web.state.lock().unwrap().sends.is_empty(){break;}tokio::time::sleep(Duration::from_millis(20)).await;}}).await?;rig.settle().await?;
        assert_eq!(http.post("get_agent_first_reply",reply_request.clone()).await?["state"],"waiting");
        web.state.lock().unwrap().fail_send=true;web.message(3,true,300);
        tokio::time::timeout(Duration::from_secs(20),async{loop{if !http.post("get_agent",reply_request.clone()).await?["pause_reason"].is_null(){break Ok::<_,Box<dyn std::error::Error+Send+Sync>>(());}tokio::time::sleep(Duration::from_millis(20)).await;}}).await??;
        assert_eq!(http.post("get_agent_first_reply",reply_request.clone()).await?["state"],"waiting");
        web.state.lock().unwrap().fail_send=false;
        http.post("resume_agent", reply_request.clone()).await?;
        }
        let (mut events,_)=tokio_tungstenite::connect_async(format!("{}/api/v1/events?workspace=team",http.url.replacen("http://","ws://",1))).await?;
        if cross { web.message(4,false,400); } else { web.message(4,true,300); }
        if cross { tokio::time::timeout(Duration::from_secs(10),async{loop{if web.state.lock().unwrap().sends.iter().any(|s|s["text"]=="Later reply"){break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await?; }
        tokio::time::timeout(Duration::from_secs(20),async{loop{if http.post("get_agent_first_reply",reply_request.clone()).await?["state"]=="replied"{break Ok::<_,Box<dyn std::error::Error+Send+Sync>>(());}tokio::time::sleep(Duration::from_millis(20)).await;}}).await??;
        event(&mut events,"agent_updated").await?;drop(events);
        if held { std::fs::write(rig.machine_home().join("release"),b"release")?;rig.settle().await?; }
        rig.restart_runtime().await?;
        assert_eq!(Http::new(&rig)?.post("get_agent_first_reply",reply_request).await?["state"],"replied");
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;

    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

async fn feishu_unbound_rig(enabled: bool) -> Result<(SystemRig, FeishuWeb, Http)> {
    let web = FeishuWeb::default();
    {
        let mut state = web.0.lock().unwrap();
        state.name = "Alice".into();
        state.user = "lead".into();
        state.tenant = "tenant-a".into();
        state.login_stage = 2;
    }
    let factories = ProviderFactoryRegistry::default();
    let transport = web.clone();
    factories.register_host_factory(FeishuProvider::new().info(), move |w, s| {
        ProviderHost::new(
            FeishuProvider::new()
                .with_transport(transport.clone())
                .with_event_source_factory({
                    let transport = transport.clone();
                    move |_, _, _, _| {
                        let source = feishu_web::Source(transport.clone());
                        Box::pin(async move {
                            Ok(Box::new(source)
                                as Box<dyn lankide_provider_feishu::events::EventSource>)
                        })
                    }
                }),
            w.id.clone(),
            w.provider_config.clone(),
            s,
        )
    })?;
    let config = RigConfig::default();
    let rig = SystemRig::start_with_providers(config, factories).await?;
    let http = Http::new(&rig)?;
    http.post("create_workspace",json!({"request_id":"create","workspace_id":"team","name":"Team","rules":"","provider_id":"feishu","provider_config_json":json!({"domain":"feishu","task_system_enabled":enabled}).to_string(),"provider_secrets_json":"{}"})).await?;
    Ok((rig, web, http))
}

async fn feishu_review_rig(enabled: bool) -> Result<(SystemRig, FeishuWeb, Http)> {
    let (rig, web, http) = feishu_unbound_rig(enabled).await?;
    http.invoke("feishu.workspace.console_login", "start", json!({}))
        .await?;
    http.invoke("feishu.workspace.console_login", "poll", json!({}))
        .await?;
    let (mut binding_events, _) = tokio_tungstenite::connect_async(format!(
        "{}/api/v1/events?workspace=team",
        http.url.replacen("http://", "ws://", 1)
    ))
    .await?;
    feishu_review_member(&http, "alpha").await?;
    event(&mut binding_events, "workspace_updated").await?;
    drop(binding_events);
    Ok((rig, web, http))
}

#[tokio::test]
async fn tasks_without_owner_authorization_fail_promptly_log_once_and_recover() -> Result<()> {
    // The daemon logger is process-wide. Run this complete business scenario in its own
    // process so concurrent rigs neither capture its diagnostics nor share its log file.
    const CHILD: &str = "LANKIDE_TASKS_LOG_SCENARIO";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "provider_management_e2e::tasks_without_owner_authorization_fail_promptly_log_once_and_recover",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return Ok(());
    }
    let (rig, web, http) = feishu_unbound_rig(true).await?;
    lankide_daemon_runtime::init_logging(rig.runtime_home(), "lankided")?;
    let scenario = async {
        let started = std::time::Instant::now();
        let response = http.client.post(format!("{}/api/v1/invoke_provider_interface", http.url))
            .json(&json!({"request_id":"tasks","workspace":{"workspace_id":"team"},"interface_id":"feishu.workspace.tasks","input_json":json!({"action":"list","text":"task-secret-canary"}).to_string()}))
            .send().await?;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!response.status().is_success());
        let body: Value = response.json().await?;
        assert_eq!(body, json!({"code":"failed_precondition","message":"Connect your account to continue.","detail":"authorization_required"}));
        assert_eq!(http.invoke("feishu.workspace.owner", "get", json!({})).await?["state"], "unbound");
        {
            let state = web.0.lock().unwrap();
            assert!(state.tasklist.is_none());
            assert_eq!(state.task_subscriptions, 0);
        }
        let log = std::fs::read_to_string(rig.runtime_home().join("logs/lankided.log"))?;
        let failures: Vec<_> = log.lines().filter(|line| line.contains("provider interface failed")).collect();
        assert_eq!(failures.len(), 1, "{log}");
        assert!(failures[0].contains("workspace=team category=authorization_required"));
        assert!(!log.contains("task-secret-canary"));
        assert!(!log.contains("Bearer"));
        // Use the actual supported account-grant flow, then retry the same list.
        http.invoke("feishu.workspace.console_login", "start", json!({})).await?;
        http.invoke("feishu.workspace.console_login", "poll", json!({})).await?;
        feishu_review_member(&http, "alpha").await?;
        let tasks = "feishu.workspace.tasks";
        assert_eq!(http.invoke(tasks, "list", json!({})).await?["enabled"], true);
        let task = http.invoke(tasks, "create", json!({"text":"Recovered task list"})).await?;
        assert_eq!(http.invoke(tasks, "set_completed", json!({"id":task["id"],"completed":true})).await?["completed"], true);
        assert_eq!(http.invoke(tasks, "list", json!({})).await?["items"][0]["id"], task["id"]);
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a4b_review_cancel_inflight_switch() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(true).await?;
    let scenario = async {
        let login = "feishu.workspace.console_login";
        let owner = "feishu.workspace.owner";
        let mut old = http.invoke(owner, "get", json!({})).await?;
        for (suffix, stage) in [
            ("/accounts/qrlogin/polling", 0),
            ("/open-apis/authen/v2/oauth/token", 2),
            ("/owner.jpg", 2),
        ] {
            {
                let mut state = web.0.lock().unwrap();
                state.user = "other".into();
                state.login_stage = stage;
            }
            http.invoke(login, "switch_start", json!({})).await?;
            let (entered, release) = web.hold(suffix);
            let poll_http = Http::new(&rig)?;
            let poll = tokio::spawn(async move {
                poll_http
                    .invoke("feishu.workspace.console_login", "switch_poll", json!({}))
                    .await
            });
            tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
            http.invoke(login, "switch_cancel", json!({})).await?;
            release.notify_one();
            let _ = tokio::time::timeout(Duration::from_secs(5), poll).await??;
            assert_eq!(
                http.invoke(owner, "get", json!({})).await?,
                old,
                "cancel during {suffix} changed the owner"
            );
            assert_eq!(
                http.invoke(login, "switch_poll", json!({})).await?["state"],
                "required",
                "cancelled pending was restored during {suffix}"
            );
            assert_eq!(
                http.invoke(owner, "refresh", json!({})).await?["person_id"],
                old["person_id"],
                "cancel replaced the user token"
            );
            old = http.invoke(owner, "get", json!({})).await?;
        }
        // A subsequent member authorization must still use the original console session.
        feishu_review_member(&http, "beta").await?;
        assert_eq!(
            http.invoke(owner, "get", json!({})).await?["person_id"],
            old["person_id"],
            "cancel replaced the active console session"
        );
        web.0.lock().unwrap().login_stage = 2;
        http.invoke(login, "switch_start", json!({})).await?;
        http.invoke(login, "switch_poll", json!({})).await?;
        assert_ne!(
            http.invoke(owner, "get", json!({})).await?["person_id"],
            old["person_id"]
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a4b_review_workspace_grant_survives_member_deletion() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(true).await?;
    let scenario = async {
        feishu_review_member(&http, "beta").await?;
        http.post(
            "delete_agent",
            json!({"request_id":"delete","agent":{"workspace_id":"team","agent_id":"beta"}}),
        )
        .await?;
        let owner = "feishu.workspace.owner";
        let tasks = "feishu.workspace.tasks";
        let login = "feishu.workspace.console_login";
        assert_eq!(
            http.invoke(owner, "refresh", json!({})).await?["state"],
            "ready"
        );
        assert_eq!(
            http.invoke(tasks, "list", json!({})).await?["enabled"],
            true
        );
        let task = http
            .invoke(tasks, "create", json!({"text":"After deletion"}))
            .await?;
        assert_eq!(
            http.invoke(
                tasks,
                "set_completed",
                json!({"id":task["id"],"completed":true})
            )
            .await?["completed"],
            true
        );
        let old = http.invoke(owner, "get", json!({})).await?;
        web.0.lock().unwrap().user = "other".into();
        http.invoke(login, "switch_start", json!({})).await?;
        http.invoke(login, "switch_poll", json!({})).await?;
        assert_ne!(
            http.invoke(owner, "get", json!({})).await?["person_id"],
            old["person_id"]
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a4b_review_first_task_enable_initializes_list() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(false).await?;
    let scenario=async {
        assert!(web.0.lock().unwrap().tasklist.is_none());
        assert_eq!(http.invoke("feishu.workspace.tasks","list",json!({})).await?["enabled"],false);
        http.post("update_workspace",json!({"request_id":"enable","workspace":{"workspace_id":"team"},"provider_config_json":"{\"domain\":\"feishu\",\"task_system_enabled\":true}"})).await?;
        let tasks="feishu.workspace.tasks";
        assert_eq!(http.invoke(tasks,"list",json!({})).await?["enabled"],true);
        let task=http.invoke(tasks,"create",json!({"text":"First task"})).await?;
        assert_eq!(http.invoke(tasks,"set_completed",json!({"id":task["id"],"completed":true})).await?["completed"],true);
        assert_eq!(http.invoke(tasks,"list",json!({})).await?["items"][0]["id"],task["id"]);
        assert_eq!(web.0.lock().unwrap().tasklist.as_ref().unwrap()["guid"],"team_tasklist");
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

type Events =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn no_workspace_event(events: &mut Events) -> Result<()> {
    assert!(
        tokio::time::timeout(Duration::from_millis(200), async {
            while let Some(frame) = events.next().await {
                let value: Value = serde_json::from_str(frame?.to_text()?)?;
                if value["data"]["kind"] == "workspace_updated" {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
            }
            Err("event stream closed".into())
        })
        .await
        .is_err(),
        "unrelated input emitted WorkspaceUpdated"
    );
    Ok(())
}
async fn task_callback(web: &FeishuWeb, id: &str, sequence: usize, kind: &str) -> Result<()> {
    let acknowledgements = {
        let mut state = web.0.lock().unwrap();
        let n = state.acknowledgements;
        state.events.push_back(lankide_provider_feishu::events::RawEvent::new("task.task.update_user_access_v2",json!({"schema":"2.0","header":{"event_id":format!("review-{sequence}"),"event_type":"task.task.update_user_access_v2","app_id":"cli_console_1","tenant_key":"tenant-a"},"event":{"task_guid":id,"event_types":[kind]}}).to_string().into_bytes()));
        n
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if web.0.lock().unwrap().acknowledgements > acknowledgements {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}
#[tokio::test]
async fn a4b_review_task_callbacks_follow_enabled_state() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(true).await?;
    let scenario=async {
        let (mut events,_)=tokio_tungstenite::connect_async(format!("{}/api/v1/events?workspace=team",http.url.replacen("http://","ws://",1))).await?;
        // Consume this exact local mutation before injecting any external event.
        let task=http.invoke("feishu.workspace.tasks","create",json!({"text":"Tracked"})).await?;
        event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        {let mut state=web.0.lock().unwrap();state.tasks.push(json!({"guid":"foreign","summary":"Other list","tasklist_guid":"other-list"}));state.denied_tasks.push("denied".into());}
        assert_eq!(http.invoke("feishu.workspace.tasks","list",json!({})).await?["items"][0]["id"],task["id"]);
        task_callback(&web,"foreign",1,"task_completed_update").await?;event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        task_callback(&web,"denied",2,"task_assignees_update").await?;event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        task_callback(&web,task["id"].as_str().unwrap(),3,"task_completed_update").await?;event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        // A displayed item disappears from the list before its callback arrives.
        {let mut state=web.0.lock().unwrap();let moved=state.tasks.iter_mut().find(|t|t["guid"]==task["id"]).unwrap();moved["tasklist_guid"]=json!("other-list");moved["tasklists"]=json!([{ "tasklist_guid":"other-list" }]);}
        task_callback(&web,task["id"].as_str().unwrap(),4,"task_tasklist_update").await?;event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        assert!(http.invoke("feishu.workspace.tasks","list",json!({})).await?["items"].as_array().unwrap().is_empty());
        web.0.lock().unwrap().denied_tasks.push(task["id"].as_str().unwrap().into());
        task_callback(&web,task["id"].as_str().unwrap(),5,"task_assignees_update").await?;event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        {let mut state=web.0.lock().unwrap();state.denied_tasks.retain(|id|id!=task["id"].as_str().unwrap());state.tasks.retain(|t|t["guid"]!=task["id"]);}
        task_callback(&web,task["id"].as_str().unwrap(),6,"task_deleted").await?;event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        let acknowledged=web.0.lock().unwrap().acknowledgements;
        web.0.lock().unwrap().events.push_back(lankide_provider_feishu::events::RawEvent::new("im.message.receive_v1",json!({"schema":"2.0","header":{"event_id":"ordinary-message","event_type":"im.message.receive_v1","app_id":"cli_console_1","tenant_key":"tenant-a"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_lead","union_id":"on_lead"}},"message":{"message_id":"ordinary","chat_id":"oc_team","chat_type":"group","create_time":"1000","message_type":"text","content":json!({"text":"Hello team"}).to_string()}}}).to_string().into_bytes()));
        tokio::time::timeout(Duration::from_secs(5),async {loop{if web.0.lock().unwrap().acknowledgements>acknowledged{break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await?;
        no_workspace_event(&mut events).await?;
        http.post("update_workspace",json!({"request_id":"disable","workspace":{"workspace_id":"team"},"provider_config_json":"{\"domain\":\"feishu\",\"task_system_enabled\":false}"})).await?;
        event(&mut events,"workspace_updated").await?;no_workspace_event(&mut events).await?;
        for (index,id) in ["foreign","denied",task["id"].as_str().unwrap()].into_iter().enumerate() {
            task_callback(&web,id,7+index,"task_deleted").await?;no_workspace_event(&mut events).await?;
        }
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

async fn feishu_review_member(http: &Http, id: &str) -> Result<()> {
    let harness = RigConfig::default().harness_executable;
    let machines = http
        .post(
            "get_machine_catalog",
            json!({"request_id":"m","workspace":null}),
        )
        .await?;
    let request = lankide_protocol::CreateAgentRequest {
        request_id: "agent".into(),
        agent: Some(lankide_protocol::AgentRef {
            workspace_id: "team".into(),
            agent_id: id.into(),
        }),
        name: "Alpha".into(),
        machine_id: machines["machines"][0]["machine_id"]
            .as_str()
            .unwrap()
            .into(),
        harness_id: "pi".into(),
        model: "scripted".into(),
        thinking_level: "low".into(),
        cli_path: harness.to_string_lossy().into(),
        provider_agent_config_json: "{\"mode\":\"new\",\"domain\":\"feishu\"}".into(),
        provider_agent_secrets_json: "{}".into(),
        ..Default::default()
    };
    http.post("create_agent", serde_json::to_value(request)?)
        .await?;
    for action in ["start", "start"] {
        http.post("advance_agent_onboarding",json!({"request_id":action,"agent":{"workspace_id":"team","agent_id":id},"action":action,"field_values_json":"{}"})).await?;
    }
    Ok(())
}
