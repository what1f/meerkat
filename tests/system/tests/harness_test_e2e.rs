//! A4c tests enter through management HTTP and the real Machine link.
use futures_util::StreamExt;
use meerkat_system_e2e::{Result, RigConfig, Script, SystemRig};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
                .timeout(Duration::from_secs(35))
                .build()?,
            url: std::fs::read_to_string(rig.runtime_home().join("run/management.url"))?,
        })
    }
    async fn test(&self, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("{}/api/v1/test_harness", self.url))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        assert!(status.is_success(), "{status}: {text}");
        Ok(serde_json::from_str(&text)?)
    }
}
fn request(machine: &str, journals: &Path, script: Script) -> Value {
    json!({
        "machine_id":machine, "harness_id":"pi", "model":"plain", "thinking_level":"",
        "cli_path":env!("CARGO_BIN_EXE_system-harness"),
        "launch_arguments":["one argument", "--literal=$HOME;echo hi"],
        "environment_variables":[
            {"name":"MEERKAT_SYSTEM_CONTROLS", "value":journals.join("controls").to_str().unwrap()},
            {"name":"MEERKAT_SYSTEM_INPUTS", "value":journals.join("inputs").to_str().unwrap()},
            {"name":"MEERKAT_SYSTEM_SCRIPT", "value":serde_json::to_string(&script).unwrap()},
            {"name":"GREETING_VALUE", "value":"custom value"}
        ]
    })
}
fn snapshot(rig: &SystemRig) -> Result<Vec<i64>> {
    let db = rusqlite::Connection::open_with_flags(
        rig.runtime_home().join("runtime.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut values = Vec::new();
    for table in ["agents", "executions", "messages", "execution_events"] {
        values.push(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })?,
        );
    }
    values.push(rig.outbound_count() as i64);
    Ok(values)
}
fn controls(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path.join("controls"))?;
    Ok(serde_json::from_str(
        text.lines().next().ok_or("missing start")?,
    )?)
}
fn process_exists(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
async fn gone(pids: &[u32]) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !pids.iter().any(|pid| process_exists(*pid)) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| format!("process remains: {pids:?}").into())
}
async fn no_event(
    http: &Http,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let (events, _) = tokio_tungstenite::connect_async(format!(
        "{}/api/v1/events?after=0",
        http.url.replacen("http://", "ws://", 1)
    ))
    .await?;
    Ok(events)
}

#[tokio::test]
async fn a4c_1_2_4_targeted_greeting_configuration_failure_and_no_business_changes() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let _team = rig.workspace("team").await?;
        let remote = rig.add_machine().await?;
        let http = Http::new(&rig)?;
        let mut events = no_event(&http).await?;
        let before = snapshot(&rig)?;
        let local_journal = std::fs::read(rig.machine_home().join("harness-pids.tsv"))?;
        let journals = tempfile::tempdir()?;
        let body = request(&remote.id, journals.path(), Script::Greeting);
        let result = http.test(body.clone()).await?;
        assert_eq!(result["state"], "passed");
        let reply: Value = serde_json::from_str(result["reply"].as_str().ok_or("missing reply")?)?;
        assert_eq!(reply["greeting"], "Hello!");
        assert_eq!(reply["environment"], "custom value");
        assert_eq!(reply["argv"], body["launch_arguments"]);
        assert_eq!(reply["model"], "plain");
        assert_eq!(reply["thinking_level"], "");
        assert_eq!(reply["tools"], json!([]));
        assert_eq!(reply["runtime_environment"], false);
        let start = controls(journals.path())?;
        assert!(
            !Path::new(start["directory"].as_str().unwrap()).exists(),
            "temporary directory remains"
        );
        assert!(
            std::fs::read_to_string(remote.home.path().join("harness-pids.tsv"))?
                .contains(&start["pid"].to_string())
        );
        gone(&[
            start["pid"].as_u64().unwrap() as u32,
            reply["child_pid"].as_u64().unwrap() as u32,
        ])
        .await?;
        let mut selected = body.clone();
        selected["model"] = json!("scripted");
        selected["thinking_level"] = json!("high");
        selected.as_object_mut().unwrap().remove("cli_path");
        let fallback = http.test(selected).await?;
        assert_eq!(fallback["state"], "passed");
        let selected: Value = serde_json::from_str(fallback["reply"].as_str().unwrap())?;
        assert_eq!(selected["model"], "scripted");
        assert_eq!(selected["thinking_level"], "high");
        gone(&[selected["child_pid"].as_u64().unwrap() as u32]).await?;
        let mut invalid = body.clone();
        invalid["thinking_level"] = json!("high");
        let response = http
            .client
            .post(format!("{}/api/v1/test_harness", http.url))
            .json(&invalid)
            .send()
            .await?;
        assert_eq!(response.status(), 400);
        assert_eq!(response.json::<Value>().await?["code"], "invalid_argument");
        let mut failure = body.clone();
        failure["cli_path"] = json!(journals.path().join("missing-cli"));
        let failed = http.test(failure).await?;
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["error_code"], "spawn_failed");
        assert!(failed.get("reply").is_none());
        let mut missing = body.clone();
        missing["harness_id"] = json!("missing");
        let response = http
            .client
            .post(format!("{}/api/v1/test_harness", http.url))
            .json(&missing)
            .send()
            .await?;
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.json::<Value>().await?["detail"],
            "harness_unavailable_on_machine"
        );
        missing["machine_id"] = json!("missing");
        let response = http
            .client
            .post(format!("{}/api/v1/test_harness", http.url))
            .json(&missing)
            .send()
            .await?;
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.json::<Value>().await?["detail"],
            "machine_not_registered"
        );
        assert_eq!(snapshot(&rig)?, before);
        assert_eq!(
            std::fs::read(rig.machine_home().join("harness-pids.tsv"))?,
            local_journal
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(250), events.next())
                .await
                .is_err(),
            "test emitted an event"
        );
        drop(remote);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let catalog: Value = http
                    .client
                    .post(format!("{}/api/v1/get_machine_catalog", http.url))
                    .json(&json!({"request_id":"offline", "workspace":null}))
                    .send()
                    .await?
                    .json()
                    .await?;
                let offline = catalog["machines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["machine_id"] == body["machine_id"])
                    .unwrap()["observed_state"]
                    != "running";
                if offline {
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        let response = http
            .client
            .post(format!("{}/api/v1/test_harness", http.url))
            .json(&body)
            .send()
            .await?;
        assert_eq!(response.status(), 503);
        assert_eq!(response.json::<Value>().await?["code"], "unavailable");
        Ok(())
    }
    .await;
    rig.close().await?;
    scenario
}

async fn timeout_scenario(script: Script, short_budget: bool) -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: short_budget,
        ..Default::default()
    })
    .await?;
    let scenario = async {
        let remote = if short_budget {
            None
        } else {
            Some(rig.add_machine().await?)
        };
        let machine_id = if let Some(remote) = &remote {
            remote.id.clone()
        } else {
            rig.proxy
                .as_ref()
                .expect("network fixture")
                .set_harness_test_budget(Duration::from_millis(1200));
            meerkat_machine::read_credentials(rig.machine_home())?
                .expect("credentials")
                .machine_id
        };
        let http = Http::new(&rig)?;
        let mut events = no_event(&http).await?;
        let before = snapshot(&rig)?;
        let journals = tempfile::tempdir()?;
        let started = std::time::Instant::now();
        let response = http.test(request(&machine_id, journals.path(), script));
        tokio::pin!(response);
        let (directory, pid, child) = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    result = &mut response => panic!("finished before child started: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
                if let Ok(start) = controls(journals.path()) {
                    let directory = PathBuf::from(start["directory"].as_str().unwrap());
                    if let Ok(child) = std::fs::read_to_string(directory.join("system-child.pid"))
                        && let Ok(child) = child.trim().parse::<u32>()
                    {
                        return (directory, start["pid"].as_u64().unwrap() as u32, child);
                    }
                }
            }
        })
        .await?;
        assert_eq!(response.await?["state"], "timed_out");
        assert!(started.elapsed() < Duration::from_secs(if short_budget { 2 } else { 30 }));
        assert!(!directory.exists());
        if short_budget {
            assert!(
                !process_exists(pid),
                "startup CLI remains when timed_out is returned"
            );
            assert!(
                !process_exists(child),
                "startup child remains when timed_out is returned"
            );
        }
        gone(&[pid, child]).await?;
        assert_eq!(snapshot(&rig)?, before);
        assert!(
            tokio::time::timeout(Duration::from_millis(250), events.next())
                .await
                .is_err()
        );
        Ok(())
    }
    .await;
    rig.close().await?;
    scenario
}

#[tokio::test]
async fn a4c_3_4_hung_harness_times_out_with_process_tree_and_directory_removed() -> Result<()> {
    timeout_scenario(
        Script::SpawnChild {
            text: String::new(),
            delay_ms: 0,
        },
        false,
    )
    .await
}

#[tokio::test]
async fn a4c_3_4_hung_startup_times_out_with_process_tree_and_directory_removed() -> Result<()> {
    timeout_scenario(Script::HangOnStart, false).await
}

#[tokio::test]
async fn a4c_review_short_budget_cleans_a_still_binding_process_group() -> Result<()> {
    timeout_scenario(Script::HangOnStart, true).await
}
