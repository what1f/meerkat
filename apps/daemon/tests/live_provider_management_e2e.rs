use meerkat_client::{
    AgentDraft, Avatar, ManagementClient, RuntimeManagementClient, ThinkingLevel, WorkspaceDraft,
};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Deserialize)]
struct FeishuProvider {
    app_id: String,
    app_secret: String,
    #[serde(default)]
    domain: String,
}

struct Fixture {
    feishu_app_id: String,
    feishu_app_secret: String,
    feishu_domain: String,
    telegram_account_id: String,
    telegram_username: String,
    telegram_token: String,
    telegram_manager_token: String,
}

#[path = "support/live.rs"]
mod live;
use meerkat_testkit::{GateSelection, ScenarioArtifact};

#[tokio::test]
#[ignore = "requires the owner-only Feishu and Telegram live fixtures"]
async fn real_runtime_creates_feishu_and_telegram_agents() {
    let run_id = std::env::var("MEERKAT_E2E_RUN_ID").expect("unique run id required");
    let artifact = ScenarioArtifact::new(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/e2e-artifacts"),
        "runtime-provider-management",
        &run_id,
    )
    .expect("reserve management artifact");
    let daemons = Arc::new(Mutex::new(Vec::<live::Daemon>::new()));
    let owned = daemons.clone();
    let home = tempfile::Builder::new()
        .prefix("meerkat-live-runtime-")
        .tempdir_in("/tmp")
        .expect("create private runtime home");
    let home_path = home.path().to_path_buf();
    let result = live::scenario(
        async move {
            live::preflight(GateSelection::FeishuLive);
            live::preflight(GateSelection::TelegramLiveMutating);
            let fixture = load_fixture().expect("load owner-only live fixtures");
            let socket = home_path.join("run");
            let child = live::Daemon::spawn(
                Command::new(live::runtime_binary())
                    .args(["--home", home_path.to_str().expect("utf-8 home")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            );
            owned.lock().unwrap().push(child);
            for _ in 0..250 {
                if socket.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(socket.exists(), "Runtime UDS did not become ready");

            let client = RuntimeManagementClient::connect_local(&socket)
                .await
                .expect("connect real Runtime UDS");
            client
                .create_workspace(WorkspaceDraft {
                    id: "live_feishu".into(),
                    name: "Live Feishu".into(),
                    rules: String::new(),
                    provider_id: "feishu".into(),
                    provider_config: json!({"task_system_enabled":false}),
                    provider_secrets: json!({}),
                })
                .await
                .expect("create credential-free Feishu Workspace");
            let feishu = client
                .create_agent(
                    "live_feishu",
                    agent(
                        "feishu_agent",
                        json!({
                            "mode":"existing",
                            "app_id":fixture.feishu_app_id,
                            "domain":fixture.feishu_domain,
                        }),
                        json!({"app_secret":fixture.feishu_app_secret}),
                    ),
                )
                .await
                .expect("bind Feishu Agent through Runtime");
            assert_eq!(feishu.workspace_id, "live_feishu");

            client
                .create_workspace(WorkspaceDraft {
                    id: "live_telegram".into(),
                    name: "Live Telegram".into(),
                    rules: String::new(),
                    provider_id: "telegram".into(),
                    provider_config: json!({}),
                    provider_secrets: json!({"manager_bot_token":fixture.telegram_manager_token}),
                })
                .await
                .expect("create Telegram Workspace with Manager Bot");
            let telegram = client
                .create_agent(
                    "live_telegram",
                    agent(
                        "telegram_agent",
                        json!({
                            "mode":"existing",
                            "account_id":fixture.telegram_account_id,
                            "bot_username":fixture.telegram_username,
                            "skip_profile_sync":true,
                        }),
                        json!({"bot_token":fixture.telegram_token}),
                    ),
                )
                .await
                .expect("bind Telegram Agent through Runtime");
            assert_eq!(telegram.workspace_id, "live_telegram");
            assert_eq!(client.list_agents("live_feishu").await.unwrap().len(), 1);
            assert_eq!(client.list_agents("live_telegram").await.unwrap().len(), 1);

            let runtime_db = Connection::open_with_flags(
                home_path.join("runtime.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .expect("open Runtime database for acceptance evidence");
            let telegram_workspace_state: String = runtime_db
                .query_row(
                    "SELECT provider_state_json FROM workspaces WHERE workspace_id='live_telegram'",
                    [],
                    |row| row.get(0),
                )
                .expect("Telegram Workspace Provider state");
            let telegram_workspace_state: Value = serde_json::from_str(&telegram_workspace_state)
                .expect("Telegram Workspace state JSON");
            assert_eq!(telegram_workspace_state["manager_bot_ready"], json!(true));
            assert!(
                telegram_workspace_state["manager_bot_id"]
                    .as_i64()
                    .is_some_and(|value| value > 0)
            );
            for (workspace_id, agent_id, required_key) in [
                ("live_feishu", "feishu_agent", "open_id"),
                ("live_telegram", "telegram_agent", "bot_username"),
            ] {
                let state: String = runtime_db
            .query_row(
                "SELECT provider_state_json FROM agents WHERE workspace_id=?1 AND agent_id=?2",
                [workspace_id, agent_id],
                |row| row.get(0),
            )
            .expect("Agent Provider state");
                let state: Value = serde_json::from_str(&state).expect("Agent Provider state JSON");
                assert!(
                    state[required_key]
                        .as_str()
                        .is_some_and(|value| !value.trim().is_empty()),
                    "{workspace_id}/{agent_id} did not persist {required_key}"
                );
            }
        },
        Duration::from_secs(240),
    )
    .await;
    for daemon in daemons.lock().unwrap().iter_mut().rev() {
        daemon.cleanup(&artifact, "runtime_shutdown");
    }
    artifact.complete_cleanup();
    let result = artifact.finish(result);
    if result.is_err() {
        let recovery_home = home.keep();
        fs::write(
            artifact.dir.join("recovery-home.json"),
            json!({"home":recovery_home}).to_string(),
        )
        .expect("retain failed management home");
    }
    result.expect("management acceptance and cleanup");
}

fn agent(id: &str, config: Value, secrets: Value) -> AgentDraft {
    AgentDraft {
        id: id.into(),
        name: "Meerkat E2E".into(),
        role: "QA".into(),
        bio: String::new(),
        avatar: Avatar::Custom("E2E".into()),
        soul: String::new(),
        machine_id: "fixture-machine".into(),
        harness_id: "pi".into(),
        model: "deepseek/deepseek-v4-flash".into(),
        thinking_level: ThinkingLevel::Low,
        auto_reaction: None,
        cli_path: String::new(),
        environment_variables: Vec::new(),
        provider_handle: String::new(),
        provider_agent_config: config,
        provider_agent_secrets: secrets,
    }
}

fn load_fixture() -> Result<Fixture, String> {
    let feishu_root = std::env::var_os("MEERKAT_E2E_TEAM_ROOT")
        .map(PathBuf::from)
        .ok_or("MEERKAT_E2E_TEAM_ROOT is required")?;
    let bootstrap_path = feishu_root.join("feishu-app.json");
    require_owner_only(&bootstrap_path)?;
    let feishu: FeishuProvider = serde_json::from_slice(
        &fs::read(&bootstrap_path).map_err(|_| "Feishu fixture is unavailable")?,
    )
    .map_err(|_| "Feishu fixture is invalid")?;
    let path = std::env::var_os("MEERKAT_TELEGRAM_E2E_FIXTURE")
        .map(PathBuf::from)
        .ok_or("MEERKAT_TELEGRAM_E2E_FIXTURE is required")?;
    let telegram = meerkat_testkit::read_telegram_fixture(&path)?;
    Ok(Fixture {
        feishu_app_id: feishu.app_id,
        feishu_app_secret: feishu.app_secret,
        feishu_domain: if feishu.domain.trim().is_empty() {
            "feishu".into()
        } else {
            feishu.domain
        },
        telegram_account_id: telegram.account_id,
        telegram_username: telegram.username,
        telegram_token: telegram.token,
        telegram_manager_token: telegram.manager_token,
    })
}

fn require_owner_only(path: &Path) -> Result<(), String> {
    let mode = fs::metadata(path)
        .map_err(|_| "fixture metadata is unavailable")?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err("fixture permissions are too broad".into());
    }
    Ok(())
}
