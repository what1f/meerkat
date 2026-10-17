//! A4a public management and mock-IM business scenarios.
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use lankide_protocol::management;
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use serde_json::{Value, json};
use std::{
    io::{Cursor, Write},
    time::Duration,
};

struct Http {
    client: reqwest::Client,
    url: String,
}
impl Http {
    fn new(rig: &SystemRig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?,
            url: std::fs::read_to_string(
                rig.runtime_home().join("run").join(management::URL_FILE),
            )?,
        })
    }
    async fn response(&self, op: &str, body: Value) -> Result<reqwest::Response> {
        Ok(self
            .client
            .post(format!("{}/api/v1/{op}", self.url))
            .json(&body)
            .send()
            .await?)
    }
    async fn post(&self, op: &str, body: Value) -> Result<Value> {
        let response = self.response(op, body).await?;
        let status = response.status();
        let text = response.text().await?;
        assert!(status.is_success(), "{op}: {status} {text}");
        Ok(serde_json::from_str(&text)?)
    }
    async fn agent(&self) -> Result<Value> {
        self.post(
            "get_agent",
            json!({"request_id":"get", "agent":agent_ref()}),
        )
        .await
    }
    async fn update(&self, values: Value) -> Result<Value> {
        let mut body = json!({"request_id":"update", "agent":agent_ref()});
        body.as_object_mut()
            .unwrap()
            .extend(values.as_object().unwrap().clone());
        self.post("update_agent", body).await
    }
}
fn agent_ref() -> Value {
    json!({"workspace_id":"team", "agent_id":"bruce"})
}
fn workspace() -> Value {
    json!({"workspace_id":"team"})
}
fn png() -> Result<Vec<u8>> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}
fn pack(missing: bool, traversal: bool, broken_png: bool) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options)?;
    zip.write_all(br#"{"format":1,"name":"Beret","author":"Artist","scale":2}"#)?;
    for frame in ["work.png", "sleep.png", "carry.png", "stand.png"] {
        if missing && frame == "sleep.png" {
            continue;
        }
        zip.start_file(frame, options)?;
        zip.write_all(&if broken_png {
            b"broken".to_vec()
        } else {
            png()?
        })?;
    }
    if traversal {
        zip.start_file("../outside.png", options)?;
        zip.write_all(&png()?)?;
    }
    Ok(zip.finish()?.into_inner())
}

#[tokio::test]
async fn a4a_1_figure_pack_survives_restart_and_invalid_imports_leave_no_data() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        {
            let team = rig.workspace("team").await?;
            team.agent("bruce").harness(Script::reply("done")).create().await?;
        }
        let http = Http::new(&rig)?;
        assert_eq!(http.agent().await?["figure"], "default");
        let figure = http.post("import_figure_pack", json!({"workspace":workspace(),"bytes":STANDARD.encode(pack(false,false,false)?)})).await?;
        assert_eq!(figure["name"], "Beret");
        assert_eq!(figure["scale"], 2.0);
        http.update(json!({"figure":figure["id"]})).await?;
        for bytes in [pack(true,false,false)?, pack(false,true,false)?, pack(false,false,true)?] {
            let response = http.response("import_figure_pack",json!({"workspace":workspace(),"bytes":STANDARD.encode(bytes)})).await?;
            assert_eq!(response.status(),400);
            assert_eq!(http.agent().await?["figure"],figure["id"]);
        }
        assert_eq!(http.post("list_figure_packs",json!({"workspace":workspace()})).await?,json!([figure]));
        let bad_id = http.response("update_agent",json!({"request_id":"bad-id","agent":agent_ref(),"figure":"missing-pack"})).await?;
        assert_eq!(bad_id.status(),400);
        {
            let c = rusqlite::Connection::open_with_flags(rig.runtime_home().join("runtime.db"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let count: i64 = c.query_row("SELECT count(*) FROM resource_metadata WHERE workspace_id='team'",[],|row| row.get(0))?;
            assert_eq!(count,4);
            assert_eq!(std::fs::read_dir(rig.runtime_home().join("resources/team"))?.count(),4);
        }
        rig.restart_runtime().await?;
        let http = Http::new(&rig)?;
        assert_eq!(http.agent().await?["figure"],figure["id"]);
        assert_eq!(http.post("list_figure_packs",json!({"workspace":workspace()})).await?,json!([figure]));
        for frame in ["work","sleep","carry","stand"] {
            let image = http.post("get_workspace_image",json!({"request_id":frame,"workspace":workspace(),"resource_id":figure["frames"][frame]["resource_id"]})).await?;
            assert_eq!(STANDARD.decode(image["bytes"].as_str().unwrap())?,png()?);
        }
        let selected = http.update(json!({"figure":"builtin:unknown-to-runtime"})).await?;
        assert_eq!(selected["figure"],"builtin:unknown-to-runtime");
        Ok::<_,Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    let close = rig.close().await;
    scenario?;
    close
}

#[tokio::test]
async fn a4a_2_arguments_reach_native_argv_and_new_sessions_after_edit() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let team = rig.workspace("team").await?;
        let agent = team
            .agent("bruce")
            .harness(Script::reply("done"))
            .create()
            .await?;
        let http = Http::new(&rig)?;
        let marker = rig.machine_home().join("shell-was-run");
        let args = json!([
            "with spaces",
            format!("; touch {}", marker.display()),
            "$(echo unsafe)",
            "|",
            "",
            "\"quoted\""
        ]);
        assert_eq!(
            http.update(json!({"launch_arguments":args})).await?["launch_arguments"],
            args
        );
        team.mention(&agent, "args", "alice", "first").await?;
        rig.settle().await?;
        let starts = agent.native_controls()?;
        let first = starts
            .iter()
            .find(|row| row["op"] == "start")
            .ok_or("missing start")?;
        assert_eq!(first["argv"], args);
        assert_eq!(first["config"]["launch_arguments"], args);
        let old_fingerprint = first["config"]["config_fingerprint"].clone();
        let next = json!(["new value", ">file"]);
        http.update(json!({"launch_arguments":next})).await?;
        team.mention(&agent, "args", "alice", "second").await?;
        rig.settle().await?;
        let starts = agent.native_controls()?;
        let last = starts
            .iter()
            .rev()
            .find(|row| row["op"] == "start")
            .ok_or("missing replacement session")?;
        assert_eq!(last["argv"], next);
        assert_ne!(last["config"]["config_fingerprint"], old_fingerprint);
        assert!(!marker.exists());
        assert_eq!(
            team.transcript(&agent, "args"),
            ["first", "done", "second", "done"]
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let close = rig.close().await;
    scenario?;
    close
}

#[tokio::test]
async fn a4a_3_busy_event_arrives_before_held_harness_is_released() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let team = rig.workspace("team").await?;
        let agent = team
            .agent("bruce")
            .harness(Script::reply("done").held())
            .create()
            .await?;
        let http = Http::new(&rig)?;
        let (mut events, _) = tokio_tungstenite::connect_async(format!(
            "{}/api/v1/events?workspace=team&after=0",
            http.url.replacen("http://", "ws://", 1)
        ))
        .await?;
        assert_eq!(http.agent().await?["activity"], "idle");
        team.mention(&agent, "busy", "alice", "work").await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = events.next().await.ok_or("event stream ended")??;
                let event: Value = serde_json::from_str(frame.to_text()?)?;
                if event["data"]["kind"] == "agent_updated" {
                    break;
                }
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await??;
        assert_eq!(http.agent().await?["activity"], "busy");
        let agents = http
            .post(
                "list_agents",
                json!({"request_id":"list","workspace":workspace()}),
            )
            .await?;
        assert_eq!(agents["agents"][0]["activity"], "busy");
        agent.release_run().await?;
        rig.settle().await?;
        assert_eq!(http.agent().await?["activity"], "idle");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = events.next().await.ok_or("event stream ended")??;
                let event: Value = serde_json::from_str(frame.to_text()?)?;
                if event["data"]["kind"] == "agent_updated" {
                    break;
                }
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await??;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let close = rig.close().await;
    scenario?;
    close
}

#[tokio::test]
async fn a4a_4_model_catalog_is_machine_scoped_and_plain_model_runs_without_thinking() -> Result<()>
{
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let http = Http::new(&rig)?;
        let initial = http.post("get_machine_catalog",json!({"request_id":"initial","workspace":null})).await?;
        let local_id = initial["machines"][0]["machine_id"].as_str().ok_or("missing machine")?;
        let remote = rig.add_machine_with_models(Some(vec![lankide_harness::NativeModel {
            id: "remote-only".into(), name: "Remote".into(), description: String::new(), thinking_levels: vec!["max".into()],
        }])).await?;
        let team = rig.workspace("team").await?;
        let agent = team.agent("bruce").machine(local_id).harness(Script::reply("done")).create().await?;
        let http = Http::new(&rig)?;
        let machines = http.post("get_machine_catalog",json!({"request_id":"machines","workspace":null})).await?;
        assert_eq!(machines["machines"].as_array().unwrap().len(),2);
        for machine in machines["machines"].as_array().unwrap() {
            let catalog = http.post("get_model_catalog",json!({"request_id":"models","workspace":workspace(),"machine_id":machine["machine_id"],"harness_id":"pi"})).await?;
            if machine["machine_id"] == remote.id {
                assert_eq!(catalog["models"].as_array().unwrap().len(),1);
                assert_eq!(catalog["models"][0]["model_id"],"remote-only");
                assert_eq!(catalog["models"][0]["supported_thinking_levels"],json!(["max"]));
            } else {
            assert_eq!(catalog["models"][0]["supported_thinking_levels"],json!(["low","high"]));
            assert_eq!(catalog["models"][1]["supported_thinking_levels"],json!(["medium"]));
            assert_eq!(catalog["models"][2]["supported_thinking_levels"],json!([]));
            assert_eq!(machine["harnesses"][0]["model_catalog"],catalog);
            }
            let unavailable = http.response("get_model_catalog",json!({"request_id":"other-harness","workspace":workspace(),"machine_id":machine["machine_id"],"harness_id":"codex"})).await?;
            assert_eq!(unavailable.status(),503);

        }
        let invalid = http.response("update_agent",json!({"request_id":"invalid","agent":agent_ref(),"model":"short","thinking_level":"high"})).await?;
        assert_eq!(invalid.status(),400);
        http.update(json!({"model":"short","thinking_level":"medium"})).await?;
        team.mention(&agent,"models","alice","short").await?; rig.settle().await?;
        http.update(json!({"model":"plain","thinking_level":""})).await?;
        team.mention(&agent,"models","alice","plain").await?; rig.settle().await?;
        let starts = agent.native_controls()?;
        let last = starts.iter().rev().find(|row| row["op"] == "start").ok_or("missing plain-model session")?;
        assert_eq!(last["config"]["thinking_level"],"");
        assert_eq!(last["config"]["model"]["model_id"],"plain");
        assert_eq!(team.transcript(&agent,"models"),["short","done","plain","done"]);
        let absent = http.response("get_model_catalog",json!({"request_id":"absent","machine_id":"absent","harness_id":"pi"})).await?;
        assert_eq!(absent.status(),404);
        Ok::<_,Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    let close = rig.close().await;
    scenario?;
    close
}
