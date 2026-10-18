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
    journals: std::path::PathBuf,
}
impl Http {
    fn new(rig: &SystemRig) -> Result<Self> {
        Ok(Self {
            journals: rig.machine_home().to_owned(),
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
async fn telegram_human_and_first_reply_survive_restart() -> Result<()> {
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
        assert!(http.post("list_human_members",json!({"request_id":"humans","workspace":{"workspace_id":"team"}})).await?.as_array().unwrap().is_empty());
        for action in ["start","poll"] {http.post("advance_agent_onboarding",json!({"request_id":action,"agent":{"workspace_id":"team","agent_id":"alpha"},"action":action,"field_values_json":"{}"})).await?;}
        let card = http.post("list_human_members",json!({"request_id":"humans","workspace":{"workspace_id":"team"}})).await?;
        assert_eq!(card[0]["name"],"Alice"); assert_eq!(card[0]["team_role"],"lead"); assert_eq!(card[0]["position"],"Team Leader"); assert!(card[0]["avatar"]["resource_id"].is_number());
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
    feishu_member_flow(http, id, &["start", "poll", "start"]).await
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
        environment_variables: vec![
            lankide_protocol::EnvironmentVariable {
                name: "LANKIDE_SYSTEM_SCRIPT".into(),
                value: serde_json::to_string(&lankide_system_e2e::Script::reply("Hello"))?,
            },
            lankide_protocol::EnvironmentVariable {
                name: "LANKIDE_SYSTEM_INPUTS".into(),
                value: http
                    .journals
                    .join(format!("inputs-{id}"))
                    .to_string_lossy()
                    .into(),
            },
            lankide_protocol::EnvironmentVariable {
                name: "LANKIDE_SYSTEM_CONTROLS".into(),
                value: http
                    .journals
                    .join(format!("controls-{id}"))
                    .to_string_lossy()
                    .into(),
            },
            lankide_protocol::EnvironmentVariable {
                name: "LANKIDE_SYSTEM_STEERS".into(),
                value: http
                    .journals
                    .join(format!("steers-{id}"))
                    .to_string_lossy()
                    .into(),
            },
        ],
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
        feishu_review_member(&http, "alpha").await?;
        feishu_review_member(&http, "beta").await?;
        let roster = humans(&http).await?;
        assert_eq!(roster[0]["person_id"], existing);
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

async fn humans(http: &Http) -> Result<Value> {
    http.post(
        "list_human_members",
        json!({"request_id":"humans","workspace":{"workspace_id":"team"}}),
    )
    .await
}

#[tokio::test]
async fn bot_creators_join_once_with_real_profiles_and_custom_positions() -> Result<()> {
    let (rig, web, http) = feishu_unbound_rig(true).await?;
    let scenario = async {
        feishu_review_member(&http,"alpha").await?;
        let first = humans(&http).await?;
        assert_eq!(first[0]["name"], "Alice");
        assert_eq!(first[0]["position"], "Team Leader");
        assert_eq!(first[0]["team_role"], "lead");
        assert!(first[0]["avatar"]["resource_id"].is_number());
        {let mut state = web.0.lock().unwrap();state.user="bob".into();state.name="Bob".into();}
        feishu_review_member(&http,"beta").await?;
        let roster = humans(&http).await?;
        assert_eq!(roster.as_array().unwrap().len(),2);
        let bob = roster.as_array().unwrap().iter().find(|p|p["name"]=="Bob").unwrap();
        assert_eq!(bob["team_role"],"member");assert_eq!(bob["position"],"");
        let request=json!({"request_id":"position","workspace":{"workspace_id":"team"},"person_id":bob["person_id"],"position":"Team Leader"});
        http.post("update_human_position",request).await?;
        let roster=humans(&http).await?;
        assert_eq!(roster.as_array().unwrap().iter().find(|p|p["person_id"]==bob["person_id"]).unwrap()["team_role"],"member");
        inbound(&web,&rig,"cli_console_2","bob","position-prompt").await?;
        let started=tokio::time::timeout(Duration::from_secs(10),async {loop {
            if let Ok(contents)=std::fs::read_to_string(http.journals.join("controls-beta")) {
                if let Some(first)=contents.lines().next() { if let Ok(value)=serde_json::from_str::<Value>(first) { break value; } }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await?;
        let prompt = started["system_prompt"].as_str().ok_or("system prompt")?;
        assert!(prompt.contains(&format!("Name: Bob；Role: Team Member；Custom position: Team Leader；ID: {}",bob["person_id"].as_str().unwrap())));
        assert!(prompt.contains(&format!("Name: Alice；Role: Team Lead；Custom position: Team Leader；ID: {}", first[0]["person_id"].as_str().unwrap())));
        feishu_review_member(&http,"gamma").await?;
        assert_eq!(humans(&http).await?.as_array().unwrap().len(),2);
        assert_eq!(web.0.lock().unwrap().qr_codes,3);
        let db=database(&rig)?;
        let secrets:i64=db.query_row("SELECT count(*) FROM workspace_named_secrets WHERE name LIKE '%console%'",[],|r|r.get(0))?;
        assert_eq!(secrets,0);
        let state:String=db.query_row("SELECT provider_state_json FROM workspaces WHERE workspace_id='team'",[],|r|r.get(0))?;
        let state:Value=serde_json::from_str(&state)?;assert!(state.get("owner").is_none() && state.get("console_user_id").is_none() && state.get("console_tenant_id").is_none());
        let requests=web.0.lock().unwrap().requests.clone();assert!(requests.iter().all(|url| !url.contains("/napi/profile") && !url.contains("oauth")));
        Ok::<_,Box<dyn std::error::Error+Send+Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn enterprise_mismatch_rejects_and_missing_contact_still_joins() -> Result<()> {
    let (rig, web, http) = feishu_review_rig(false).await?;
    let scenario = async {
        {
            let mut state = web.0.lock().unwrap();
            state.user = "outside".into();
            state.tenant = "other-enterprise".into();
        }
        assert!(feishu_review_member(&http, "outside").await.is_err());
        assert_eq!(humans(&http).await?.as_array().unwrap().len(), 1);
        {
            let mut state = web.0.lock().unwrap();
            state.user = "charlie".into();
            state.tenant = "tenant-a".into();
            state.contact_unavailable = true;
        }
        feishu_review_member(&http, "charlie").await?;
        let roster = humans(&http).await?;
        assert_eq!(roster.as_array().unwrap().len(), 2);
        let fallback = roster
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["team_role"] == "member")
            .unwrap();
        assert!(fallback["name"].is_null());
        assert!(fallback["avatar"].is_null());
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
