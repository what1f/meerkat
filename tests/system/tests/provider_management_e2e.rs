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
                    move |_, credentials, _, _| {
                        let source =
                            feishu_web::Source(transport.clone(), credentials.app_id.clone());
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

async fn feishu_review_member(http: &Http, id: &str) -> Result<()> {
    feishu_member_flow(http, id, &["start", "start"]).await
}

async fn feishu_member_flow(http: &Http, id: &str, actions: &[&str]) -> Result<()> {
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
    let mut payload = "{}".to_owned();
    for action in actions {
        let progress = http.post("advance_agent_onboarding",json!({"request_id":action,"agent":{"workspace_id":"team","agent_id":id},"action":action,"field_values_json":payload})).await
            .map_err(|error| format!("member onboarding {action}: {error}"))?;
        payload = progress["payload_json"]
            .as_str()
            .ok_or("onboarding payload")?
            .to_owned();
    }
    Ok(())
}

fn database(rig: &SystemRig) -> Result<rusqlite::Connection> {
    Ok(rusqlite::Connection::open_with_flags(
        rig.runtime_home().join("runtime.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)
}

#[tokio::test]
async fn console_profile_multiple_apps_and_read_only_team_tasks() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(true).await?;
    let scenario = async {
        let owner = http.invoke("feishu.workspace.owner", "get", json!({})).await?;
        assert_eq!(owner["name"], "Alice");
        assert!(owner["avatar"]["resource_id"].is_number());
        let person = owner["person_id"].as_str().ok_or("person")?;
        feishu_review_member(&http, "beta").await?;
        assert_eq!(http.invoke("feishu.workspace.owner", "get", json!({})).await?["person_id"], person);
        let db = database(&rig)?;
        let (external, role, identities): (String,String,String) = db.query_row("SELECT external_id,team_role,provider_identity_state_json FROM people WHERE person_id=?1", [person], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        assert_eq!((external.as_str(),role.as_str()), ("on_lead","lead"));
        let identities: Value = serde_json::from_str(&identities)?;
        for app in ["cli_console_1", "cli_console_2"] {
            assert_eq!(identities["open_ids_by_app"][app], format!("ou_lead_{app}"));
        }
        {
            let mut state = web.0.lock().unwrap();
            assert_eq!(state.qr_codes,1);
            assert_eq!(state.tasklist_creates.len(),1);
            assert_eq!(state.tasklist_creates[0].0,"Bearer tenant-cli_console_1");
            assert_eq!(state.tasklist_transfers.len(),1);
            assert_eq!(state.tasklist.as_ref().unwrap()["owner"]["id"], "ou_lead_cli_console_1");
            assert!(state.tasklist_member_adds.iter().any(|(_,body)|body["members"].as_array().unwrap().iter().any(|m|m["type"]=="app" && m["id"]=="cli_console_2")));
            state.tasks.push(json!({"guid":"external","summary":"Remote task","completed_at":"0","tasklist_guid":"team_tasklist"}));
        }
        let tasks = "feishu.workspace.tasks";
        assert_eq!(http.invoke(tasks,"list",json!({})).await?["items"][0]["text"], "Remote task");
        for action in ["create","set_completed"] { assert!(http.invoke(tasks,action,json!({"text":"No", "id":"external", "completed":true})).await.is_err()); }
        web.0.lock().unwrap().expired=true;
        assert_eq!(http.invoke(tasks,"list",json!({})).await?["items"][0]["text"], "Remote task");
        let refreshed=http.invoke("feishu.workspace.owner","refresh",json!({})).await?;
        assert_eq!(refreshed["person_id"],owner["person_id"]);
        assert_eq!(refreshed["state"],"ready");
        let state = web.0.lock().unwrap();
        assert_eq!(state.console_calls.iter().filter(|v|v.starts_with("scope/update")).count(),2);
        assert_eq!(state.console_calls.iter().filter(|v|v.starts_with("app_version/create")).count(),2);
        assert!(!state.requests.iter().any(|v|v.contains("authen") || v.contains("safe_setting")));
        let secrets:i64 = db.query_row("SELECT count(*) FROM workspace_named_secrets WHERE name LIKE 'feishu_owner_%'",[],|r|r.get(0))?;
        assert_eq!(secrets,0);
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn console_session_expiry_network_csrf_and_paused_account_switch() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(false).await?;
    let scenario=async {
        let login="feishu.workspace.console_login";
        let owner=http.invoke("feishu.workspace.owner","get",json!({})).await?;
        for status in [429,500,503] {
            web.0.lock().unwrap().profile_status=Some(status);
            assert!(http.invoke(login,"start",json!({})).await.is_err());
            let current=http.invoke("feishu.workspace.owner","get",json!({})).await?;
        for key in ["person_id","name","state"] {assert_eq!(current[key],owner[key]);}
            assert_eq!(web.0.lock().unwrap().qr_codes,1);
        }
        {let mut s=web.0.lock().unwrap();s.profile_status=None;s.profile_network_failure=true;}
        assert!(http.invoke(login,"start",json!({})).await.is_err());
        assert_eq!(web.0.lock().unwrap().qr_codes,1);
        {let mut s=web.0.lock().unwrap();s.profile_network_failure=false;s.csrf_failure=true;}

        feishu_review_member(&http,"beta").await?;
        assert_eq!(web.0.lock().unwrap().qr_codes,1);
        web.0.lock().unwrap().expired=true;
        assert_eq!(http.invoke(login,"start",json!({})).await?["state"],"waiting");
        web.0.lock().unwrap().expired=false;
        assert_eq!(http.invoke(login,"poll",json!({})).await?["state"],"ready");
        assert_eq!(http.invoke("feishu.workspace.owner","get",json!({})).await?["person_id"],owner["person_id"]);
        web.0.lock().unwrap().profile_redirect=true;
        assert_eq!(http.invoke(login,"start",json!({})).await?["state"],"waiting");
        web.0.lock().unwrap().profile_redirect=false;
        assert_eq!(http.invoke(login,"poll",json!({})).await?["state"],"ready");
        http.post("set_workspace_state",json!({"request_id":"pause","workspace":{"workspace_id":"team"},"desired_state":"paused"})).await?;
        let before=web.0.lock().unwrap().event_polls;
        http.invoke(login,"switch_start",json!({})).await?;
        http.invoke(login,"switch_cancel",json!({})).await?;
        assert_eq!(http.invoke(login,"switch_poll",json!({})).await?["state"],"required");
        http.invoke(login,"switch_start",json!({})).await?;
        web.0.lock().unwrap().tenant="other-tenant".into();
        assert!(http.invoke(login,"switch_poll",json!({})).await.is_err());
        assert_eq!(http.invoke("feishu.workspace.owner","get",json!({})).await?["person_id"],owner["person_id"]);
        web.0.lock().unwrap().tenant="tenant-a".into();
        http.invoke(login,"switch_start",json!({})).await?;
        assert_eq!(http.invoke(login,"switch_poll",json!({})).await?["state"],"ready");
        assert!(web.0.lock().unwrap().event_polls <= before+1);
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn console_creator_mismatch_preserves_lead() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(false).await?;
    let scenario = async {
        let owner = http
            .invoke("feishu.workspace.owner", "get", json!({}))
            .await?;
        web.0.lock().unwrap().creator_mismatch = true;
        assert!(feishu_review_member(&http, "beta").await.is_err());
        let current = http
            .invoke("feishu.workspace.owner", "get", json!({}))
            .await?;
        for key in ["person_id", "name", "state"] {
            assert_eq!(current[key], owner[key]);
        }
        let db = database(&rig)?;
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM people WHERE team_role='lead'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

async fn inbound(
    web: &FeishuWeb,
    rig: &SystemRig,
    app: &str,
    user: &str,
    id: &str,
) -> Result<String> {
    {
        let mut state = web.0.lock().unwrap();
        state
            .chat_types
            .insert("owner_dm".into(), "private_chat".into());
        state.message_chats.insert(id.into(), "owner_dm".into());
        state.events.push_back(lankide_provider_feishu::events::RawEvent::new("im.message.receive_v1",json!({"schema":"2.0","header":{"event_id":id,"event_type":"im.message.receive_v1","app_id":app,"tenant_key":"tenant-a"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":format!("ou_{user}_{app}"),"union_id":format!("on_{user}")}},"message":{"message_id":id,"chat_id":"owner_dm","chat_type":"p2p","create_time":"1900000000000","message_type":"text","content":json!({"text":"Hello"}).to_string()}}}).to_string().into_bytes()));
    }
    let person = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let db = database(rig)?;
            use rusqlite::OptionalExtension;
            if let Some(id) = db
                .query_row(
                    "SELECT sender_id FROM messages WHERE provider_message_id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                break Ok::<_, Box<dyn std::error::Error + Send + Sync>>(id);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|e| format!("inbound {id}: {e}"))??;
    Ok(person)
}

#[tokio::test]
async fn stranger_cannot_claim_lead_and_union_identity_survives_multiple_apps() -> Result<()> {
    let (rig, web, http) = feishu_unbound_rig(false).await?;
    let scenario = async {
        let machines = http
            .post(
                "get_machine_catalog",
                json!({"request_id":"m","workspace":null}),
            )
            .await?;
        let request = lankide_protocol::CreateAgentRequest {
            request_id: "bootstrap".into(),
            agent: Some(lankide_protocol::AgentRef {
                workspace_id: "team".into(),
                agent_id: "bootstrap".into(),
            }),
            name: "Bootstrap".into(),
            machine_id: machines["machines"][0]["machine_id"]
                .as_str()
                .unwrap()
                .into(),
            harness_id: "pi".into(),
            model: "scripted".into(),
            thinking_level: "low".into(),
            cli_path: RigConfig::default()
                .harness_executable
                .to_string_lossy()
                .into(),
            provider_agent_config_json:
                json!({"mode":"existing","app_id":"cli_existing","domain":"feishu"}).to_string(),
            provider_agent_secrets_json: json!({"app_secret":"existing-secret"}).to_string(),
            ..Default::default()
        };
        http.post("create_agent", serde_json::to_value(request)?)
            .await?;
        let stranger = inbound(&web, &rig, "cli_existing", "stranger", "stranger-first").await?;
        let existing = inbound(&web, &rig, "cli_existing", "lead", "lead-before-binding").await?;
        {
            let db = database(&rig)?;
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM people WHERE team_role='lead'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                0
            );
            assert_ne!(stranger, existing);
        }
        let login = "feishu.workspace.console_login";
        http.invoke(login, "start", json!({})).await?;
        http.invoke(login, "poll", json!({})).await?;
        assert_eq!(
            http.invoke("feishu.workspace.owner", "get", json!({}))
                .await?["name"],
            "Alice"
        );
        feishu_review_member(&http, "alpha").await?;
        feishu_review_member(&http, "beta").await?;
        let owner = http
            .invoke("feishu.workspace.owner", "get", json!({}))
            .await?;
        assert_eq!(owner["person_id"], existing);
        for (app, id) in [
            ("cli_console_1", "alpha-message"),
            ("cli_console_2", "beta-message"),
        ] {
            assert_eq!(inbound(&web, &rig, app, "lead", id).await?, existing);
        }
        let db = database(&rig)?;
        assert_eq!(
            db.query_row(
                "SELECT team_role FROM people WHERE person_id=?1",
                [stranger],
                |r| r.get::<_, String>(0)
            )?,
            "external"
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn same_tenant_account_switch_rebinds_and_cancelled_profile_does_not_commit() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(false).await?;
    let scenario = async {
        let login = "feishu.workspace.console_login";
        let owner = http
            .invoke("feishu.workspace.owner", "get", json!({}))
            .await?;
        http.invoke(login, "switch_start", json!({})).await?;
        let (entered, release) = web.hold("/napi/profile");
        let pending = {
            let http = Http::new(&rig)?;
            tokio::spawn(async move {
                http.invoke("feishu.workspace.console_login", "switch_poll", json!({}))
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(5), entered.notified()).await?;
        http.invoke(login, "switch_cancel", json!({})).await?;
        release.notify_one();
        assert_eq!(pending.await??["state"], "required");
        assert_eq!(
            http.invoke("feishu.workspace.owner", "get", json!({}))
                .await?,
            owner
        );
        {
            let mut state = web.0.lock().unwrap();
            state.user = "other".into();
            state.name = "Bob".into();
            state
                .app_owners
                .insert("cli_console_1".into(), "other".into());
        }
        http.invoke(login, "switch_start", json!({})).await?;
        assert_eq!(
            http.invoke(login, "switch_poll", json!({})).await?["state"],
            "ready"
        );
        let switched = http
            .invoke("feishu.workspace.owner", "get", json!({}))
            .await?;
        assert_eq!(switched["name"], "Bob");
        assert_ne!(switched["person_id"], owner["person_id"]);
        let db = database(&rig)?;
        let lead: String = db.query_row(
            "SELECT external_id FROM people WHERE team_role='lead'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(lead, "on_other");
        assert_eq!(
            db.query_row(
                "SELECT team_role FROM people WHERE person_id=?1",
                [owner["person_id"].as_str().unwrap()],
                |r| r.get::<_, String>(0)
            )?,
            "member"
        );
        assert_eq!(
            web.0
                .lock()
                .unwrap()
                .console_calls
                .iter()
                .filter(|v| v.starts_with("scope/update") || v.starts_with("app_version/create"))
                .count(),
            2
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
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
        let task=json!({"id":"external"});
        web.0.lock().unwrap().tasks.push(json!({"guid":"external","summary":"Tracked","completed_at":"0","tasklist_guid":"team_tasklist"}));
        no_workspace_event(&mut events).await?;
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
        web.0.lock().unwrap().events.push_back(lankide_provider_feishu::events::RawEvent::new("im.message.receive_v1",json!({"schema":"2.0","header":{"event_id":"ordinary-message","event_type":"im.message.receive_v1","app_id":"cli_console_1","tenant_key":"tenant-a"},"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_lead_cli_console_1","union_id":"on_lead"}},"message":{"message_id":"ordinary","chat_id":"oc_team","chat_type":"group","create_time":"1000","message_type":"text","content":json!({"text":"Hello team"}).to_string()}}}).to_string().into_bytes()));
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

#[tokio::test]
async fn console_create_failure_keeps_registration_fallback() -> Result<()> {
    let (rig, web, http) = feishu_unbound_rig(false).await?;
    let scenario = async {
        http.invoke("feishu.workspace.console_login", "start", json!({}))
            .await?;
        http.invoke("feishu.workspace.console_login", "poll", json!({}))
            .await?;
        web.0.lock().unwrap().reject_app_create = true;
        feishu_member_flow(&http, "alpha", &["start"]).await?;
        let progress = http
            .post(
                "get_agent_onboarding",
                json!({"request_id":"get","agent":{"workspace_id":"team","agent_id":"alpha"}}),
            )
            .await?;
        assert_ne!(progress["status"], "ready");
        let state = web.0.lock().unwrap();
        assert!(state.apps.is_empty());
        assert!(
            state
                .requests
                .iter()
                .any(|p| p.ends_with("/oauth/v1/app/registration"))
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn first_task_enable_initializes_with_app_and_deleting_app_keeps_owner() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(false).await?;
    let scenario=async {
        assert!(web.0.lock().unwrap().tasklist.is_none());
        let owner=http.invoke("feishu.workspace.owner","get",json!({})).await?;
        http.post("update_workspace",json!({"request_id":"enable","workspace":{"workspace_id":"team"},"provider_config_json":"{\"domain\":\"feishu\",\"task_system_enabled\":true}"})).await?;
        assert_eq!(http.invoke("feishu.workspace.tasks","list",json!({})).await?["enabled"],true);
        {let state=web.0.lock().unwrap();assert_eq!(state.tasklist_creates.len(),1);assert_eq!(state.tasklist_creates[0].0,"Bearer tenant-cli_console_1");}
        http.post("delete_agent",json!({"request_id":"delete","agent":{"workspace_id":"team","agent_id":"alpha"}})).await?;
        let refreshed=http.invoke("feishu.workspace.owner","refresh",json!({})).await?;
        assert_eq!(refreshed["person_id"],owner["person_id"]);assert_eq!(refreshed["state"],"ready");
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
