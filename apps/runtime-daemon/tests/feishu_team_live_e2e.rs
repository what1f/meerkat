use lankide_client::{
    AgentDraft, Avatar, ManagementClient, RuntimeManagementClient, ThinkingLevel, WorkspaceDraft,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    env, fs,
    fs::{File, OpenOptions},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, serde::Deserialize)]
struct FeishuAgentFixture {
    app_id: String,
    app_secret: String,
    domain: String,
    bot_open_id: String,
    bot_name: String,
}

#[path = "support/live.rs"]
mod live;
use lankide_provider_feishu::{Auth, Credentials, Domain, FeishuClient, Method, ReqwestTransport};
use lankide_testkit::{GateSelection, ScenarioArtifact};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct OwnedResources {
    daemons: Vec<live::Daemon>,
    actor_requests: BTreeMap<String, Option<String>>,
    apps: BTreeMap<String, FeishuAgentFixture>,
    profile: Option<(FeishuAgentFixture, Value)>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit LANKIDE_E2E_LIVE=1, real Feishu fixtures, and real Pi"]
async fn real_feishu_team_lifecycle_queue_steer_and_reply() {
    assert_eq!(env::var("LANKIDE_E2E_LIVE").as_deref(), Ok("1"));
    let run_id = env::var("LANKIDE_E2E_RUN_ID").expect("LANKIDE_E2E_RUN_ID is required");
    let artifact = Arc::new(
        ScenarioArtifact::new(
            &workspace_root().join("target/e2e-artifacts"),
            "runtime-feishu-team",
            &run_id,
        )
        .expect("reserve fresh team artifact"),
    );
    let artifact_dir = artifact.dir.clone();
    let runtime_home = tempfile::Builder::new()
        .prefix("lk-team-")
        .tempdir_in("/tmp")
        .expect("private short Runtime home");
    let home = runtime_home.path().to_path_buf();
    let owned = Arc::new(Mutex::new(OwnedResources::default()));
    let cleanup_owned = owned.clone();
    let cleanup_home = home.clone();
    let cleanup_artifact = artifact.clone();
    let result = live::scenario(async move {
    live::preflight(GateSelection::FeishuLive);
    let fixture_db = PathBuf::from(
        env::var("LANKIDE_FEISHU_E2E_APPS")
            .expect("LANKIDE_FEISHU_E2E_APPS is required"),
    );
    require_owner_only(&fixture_db).expect("Feishu fixture must be owner-only");
    let mut apps = load_feishu_apps(&fixture_db).expect("load two Feishu Agent applications");
    let lark_cli =
        PathBuf::from(env::var("LANKIDE_E2E_LARK_CLI").expect("LANKIDE_E2E_LARK_CLI is required"));
    let oauth_dir = PathBuf::from(
        env::var("LANKIDE_E2E_OAUTH_DIR").expect("LANKIDE_E2E_OAUTH_DIR is required"),
    );
    let chat_id = env::var("LANKIDE_E2E_CHAT_ID").expect("LANKIDE_E2E_CHAT_ID is required");
    hydrate_group_bots(&lark_cli, &oauth_dir, &chat_id, &mut apps)
        .expect("both Feishu Apps must already be members of the E2E group");

    let socket = home.join("run");
    let runtime_log = private_log(&artifact_dir.join("runtime.log"));
    let runtime = live::Daemon::spawn(Command::new(live::runtime_binary())
        .args(["--home", home.to_str().expect("utf-8 home")])
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            runtime_log.try_clone().expect("clone runtime log"),
        ))
        .stderr(Stdio::from(runtime_log)));
    owned.lock().unwrap().daemons.push(runtime);
    wait_for_path(&socket, Duration::from_secs(10))
        .await
        .expect("Runtime UDS did not become ready");

    let computer_bin = PathBuf::from(
        env::var("LANKIDE_E2E_COMPUTER_BIN").expect("LANKIDE_E2E_COMPUTER_BIN is required"),
    );
    let computer_log = private_log(&artifact_dir.join("computer.log"));
    let computer = live::Daemon::spawn(Command::new(computer_bin)
        .args(["--home", home.to_str().expect("utf-8 home")])
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            computer_log.try_clone().expect("clone computer log"),
        ))
        .stderr(Stdio::from(computer_log)));
    owned.lock().unwrap().daemons.push(computer);

    let client = RuntimeManagementClient::connect_local(&socket)
        .await
        .expect("connect real Runtime UDS");
    client
        .create_workspace(WorkspaceDraft {
            id: "feishu_team_e2e".into(),
            name: "Feishu Team E2E".into(),
            rules: "真实飞书端到端验收工作区".into(),
            provider_id: "feishu".into(),
            provider_config: json!({"task_system_enabled":false}),
            provider_secrets: json!({}),
        })
        .await
        .expect("create Feishu Workspace through management API");

    owned.lock().unwrap().apps = apps.clone();
    let evan_fixture = apps.remove("cli_aae5c090ee389d2e").expect("Evan fixture");
    let rocky_fixture = apps.remove("cli_aafa1b0f80b8dd24").expect("Rocky fixture");
    let evan = client
        .create_agent(
            "feishu_team_e2e",
            agent_draft("evan", "Evan", &evan_fixture),
        )
        .await
        .expect("create Evan through management API");
    let _rocky = client
        .create_agent(
            "feishu_team_e2e",
            agent_draft("rocky", "Rocky", &rocky_fixture),
        )
        .await
        .expect("create Rocky through management API");
    assert_eq!(
        client
            .list_agents("feishu_team_e2e")
            .await
            .expect("list created Agents")
            .len(),
        2,
        "Workspace must contain two real Feishu Agents"
    );

    // Feishu long connections are established during Agent creation. Give
    // both real Apps a brief registration window before the first group event.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let snapshot = application_profile(&evan_fixture).await.expect("snapshot actual Feishu profile");
    write_private_json(&artifact_dir.join("profile-before.json"), &snapshot).expect("persist profile recovery values");
    owned.lock().unwrap().profile = Some((evan_fixture.clone(), snapshot));
    let profile_marker = format!("Lankide live E2E {run_id}");
    let mut edited = evan.clone();
    edited.role = "E2E-RD".into();
    edited.bio = profile_marker.clone();
    let edited = client
        .update_agent(edited, None)
        .await
        .expect("edit Evan through management API and Feishu profile sync");
    assert_eq!(edited.role, "E2E-RD");
    assert_eq!(edited.bio, profile_marker);

    let database = home.join("runtime.db");
    let start_marker = format!("LK_{run_id}_START");
    let steer_one = format!("LK_{run_id}_S1");
    let steer_two = format!("LK_{run_id}_S2");
    let rocky_marker = format!("LK_{run_id}_ROCKY");
    let evan_final = format!("LK_{run_id}_EVAN_DONE");
    let rocky_final = format!("LK_{run_id}_ROCKY_DONE");

    send_tracked(&owned, &artifact,
        &lark_cli,
        &oauth_dir,
        &chat_id,
        &evan_fixture,
        &format!(
            "{start_marker} 请先用 bash 执行 sleep 15，等待随后发来的两条补充要求；然后使用 bash 调用正式能力命令 $LANKIDE_CLI send --channel group_chat --chat {chat_id} --force --content {evan_final}_{steer_one}_{steer_two}。只发送一次，不要改写 --content 的参数值，不要读取凭据或打印环境变量。"
        ),
        &format!("{run_id}-start"),
    )
    .expect("send Evan start mention as real user");
    let evan_execution = wait_for_running_execution(&database, "evan", Duration::from_secs(120))
        .await
        .expect("Evan did not enter a running execution");

    let steer_one_provider_message_id = send_tracked(&owned, &artifact,
        &lark_cli,
        &oauth_dir,
        &chat_id,
        &evan_fixture,
        &format!("{steer_one} 这是运行中的插入消息；最终回复还必须包含 {steer_one}。"),
        &format!("{run_id}-steer1"),
    )
    .expect("send first Evan steer mention");
    let steer_two_provider_message_id = send_tracked(&owned, &artifact,
        &lark_cli,
        &oauth_dir,
        &chat_id,
        &evan_fixture,
        &format!("{steer_two} 这是紧接着排队的第二条插入消息；最终回复还必须包含 {steer_two}。"),
        &format!("{run_id}-steer2"),
    )
    .expect("send second Evan steer mention");
    send_tracked(&owned, &artifact,
        &lark_cli,
        &oauth_dir,
        &chat_id,
        &rocky_fixture,
        &format!(
            "{rocky_marker} 请用 bash 调用正式能力命令 $LANKIDE_CLI send --channel group_chat --chat {chat_id} --force --content {rocky_final}，只发送一次。不要读取凭据或打印环境变量。"
        ),
        &format!("{run_id}-rocky"),
    )
    .expect("send Rocky mention in the same group");

    wait_for_steers(&database, evan_execution, 2, Duration::from_secs(120))
        .await
        .expect("two in-flight Evan messages were not persisted and delivered as steer");
    wait_for_agent_execution(&database, "rocky", Duration::from_secs(120))
        .await
        .expect("Rocky did not receive an independent execution");
    wait_for_outbound_marker(&database, &evan_final, Duration::from_secs(240))
        .await
        .expect("Evan final reply was not persisted");
    wait_for_outbound_marker(&database, &steer_one, Duration::from_secs(120))
        .await
        .expect("Evan reply did not incorporate the first in-flight message");
    wait_for_outbound_marker(&database, &steer_two, Duration::from_secs(120))
        .await
        .expect("Evan reply did not incorporate the queued in-flight message");
    wait_for_outbound_marker(&database, &rocky_final, Duration::from_secs(240))
        .await
        .expect("Rocky final reply was not persisted");
    wait_for_execution_finished(&database, evan_execution, Duration::from_secs(120))
        .await
        .expect("Evan execution did not finish");

    wait_for_count(&database, "SELECT COUNT(*) FROM executions WHERE state='succeeded'", &[], 2, Duration::from_secs(120), false)
        .await.expect("both native executions must finish");
    let evidence = collect_evidence(
        &database,
        evan_execution,
        &[
            steer_one_provider_message_id.as_str(),
            steer_two_provider_message_id.as_str(),
        ],
    )
    .expect("collect Runtime evidence");
    assert_eq!(evidence["scenario_accepted_steer_deliveries"], 2);
    assert!(
        evidence["accepted_steer_deliveries"]
            .as_i64()
            .is_some_and(|count| count >= 2),
        "the execution must retain both scenario steers even when the shared live chat has concurrent mentions"
    );
    assert_eq!(
        evidence["executions"], 2,
        "accepted steers must not start a second Evan execution"
    );
    assert_eq!(evidence["failed_executions"], 0);
    assert_eq!(evidence["unconsumed_inbox_items"], 0);
    let native = native_cli_evidence(
        &home,
        &database,
        &artifact_dir,
        &[("evan", evan_final.as_str()), ("rocky", rocky_final.as_str())],
    )
    .expect("both native sessions must successfully invoke the shipped CLI and persist its accepted message");
    write_private_json(&artifact_dir.join("evidence.json"), &json!({"runtime":evidence,"native":native}))
        .expect("persist composition evidence");
    }, Duration::from_secs(720)).await;
    cleanup_team(&cleanup_owned, &cleanup_home, &cleanup_artifact).await;
    cleanup_artifact.complete_cleanup();
    let result = cleanup_artifact.finish(result);
    if result.is_err() {
        let recovery_home = runtime_home.keep();
        write_private_json(
            &cleanup_artifact.dir.join("recovery-home.json"),
            &json!({"home":recovery_home}),
        )
        .expect("retain failed run home for recovery");
    }
    result.expect("team acceptance and cleanup");
}

fn agent_draft(id: &str, name: &str, fixture: &FeishuAgentFixture) -> AgentDraft {
    AgentDraft {
        id: id.into(),
        name: name.into(),
        role: "RD".into(),
        bio: String::new(),
        avatar: Avatar::Custom(name.into()),
        soul: "完成群聊中的协作请求，简洁回复。".into(),
        computer_id: "fixture-computer".into(),
        harness_id: "pi".into(),
        model: "deepseek-v4-flash".into(),
        thinking_level: ThinkingLevel::Low,
        auto_reaction: None,
        cli_path: String::new(),
        environment_variables: Vec::new(),
        provider_handle: fixture.app_id.clone(),
        provider_agent_config: json!({
            "mode":"existing",
            "app_id":fixture.app_id,
            "domain":fixture.domain,
            "external_id":fixture.bot_open_id,
            "open_id":fixture.bot_open_id,
            "display_name":fixture.bot_name,
        }),
        provider_agent_secrets: json!({"app_secret":fixture.app_secret}),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn private_log(path: &Path) -> File {
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .expect("create daemon log");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("protect daemon log");
    file
}

fn write_private_json(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let mut file = private_log(path);
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|_| "encode evidence")?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "persist evidence".into())
}

fn app_client(fixture: &FeishuAgentFixture) -> Result<FeishuClient<ReqwestTransport>, String> {
    FeishuClient::new(
        ReqwestTransport::new(),
        Credentials::app(&fixture.app_id, &fixture.app_secret),
    )
    .map(|client| {
        client.with_domain(if fixture.domain == "lark" {
            Domain::Lark
        } else {
            Domain::Feishu
        })
    })
    .map_err(|_| "create authenticated Feishu client".into())
}

async fn application_profile(fixture: &FeishuAgentFixture) -> Result<Value, String> {
    let language = if fixture.domain == "lark" {
        "en_us"
    } else {
        "zh_cn"
    };
    let data = app_client(fixture)?
        .task_json(
            Method::Get,
            &format!("/open-apis/application/v6/applications/{}", fixture.app_id),
            &[("lang".into(), language.into())],
            None,
            Auth::Tenant,
            "snapshot application profile",
        )
        .await
        .map_err(|_| "read application profile")?;
    let name = data
        .pointer("/app/app_name")
        .and_then(Value::as_str)
        .ok_or("profile has no name")?;
    let description = data
        .pointer("/app/description")
        .and_then(Value::as_str)
        .ok_or("profile has no description")?;
    Ok(json!({"app_id":fixture.app_id,"name":name,"description":description,"language":language}))
}

async fn restore_profile(fixture: &FeishuAgentFixture, before: &Value) -> Result<(), String> {
    let client = app_client(fixture)?;
    client
        .update_application_profile(
            before["name"].as_str().ok_or("missing prior name")?,
            before["description"]
                .as_str()
                .ok_or("missing prior description")?,
            None,
            before["language"]
                .as_str()
                .ok_or("missing prior language")?,
            Auth::Tenant,
        )
        .await
        .map_err(|_| "restore actual application profile")?;
    client
        .publish_application(
            "Restore profile after authorized acceptance",
            "Restore prior name and description",
            Auth::Tenant,
        )
        .await
        .map_err(|_| "publish restored application profile")?;
    for _ in 0..10 {
        if application_profile(fixture).await? == *before {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err("restored application profile read-back differs".into())
}

async fn cleanup_team(owned: &Mutex<OwnedResources>, home: &Path, artifact: &ScenarioArtifact) {
    let mut owned = std::mem::take(
        &mut *owned
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    for daemon in owned.daemons.iter_mut().rev() {
        daemon.cleanup(artifact, "daemon_shutdown");
    }
    // The private database belongs only to this run. Stop writers before enumerating
    // its outgoing messages, so cleanup cannot race a later native send.
    let database = home.join("runtime.db");
    if database.exists() {
        let outgoing = (|| -> Result<Vec<(String, String)>, String> {
            let db = runtime_db(&database)?;
            let mut statement = db.prepare("SELECT DISTINCT sender_id,provider_message_id FROM messages WHERE direction='outbound' AND provider_message_id IS NOT NULL")
                .map_err(|_| "prepare owned outgoing messages")?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|_| "query outgoing messages")?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "read outgoing messages".into())
        })();
        match outgoing {
            Ok(messages) => {
                let saved = write_private_json(
                    &artifact.dir.join("outgoing-recovery.json"),
                    &json!(messages),
                )
                .is_ok();
                artifact.record_cleanup("persist_outgoing_recovery", "message_ids", saved);
                for (sender, id) in messages {
                    let app_id = match sender.as_str() {
                        "evan" => "cli_aae5c090ee389d2e",
                        "rocky" => "cli_aafa1b0f80b8dd24",
                        _ => "",
                    };
                    let result = owned
                        .apps
                        .get(app_id)
                        .ok_or("unknown owned sender".into())
                        .and_then(app_client);
                    let result = match result {
                        Ok(client) => client
                            .recall_message(&id, Auth::Tenant)
                            .await
                            .map_err(|_| "recall bot message".into()),
                        Err(error) => Err(error),
                    };
                    artifact.record_cleanup("bot_recall", &id, result.is_ok());
                }
            }
            Err(_) => artifact.record_cleanup("enumerate_bot_messages", "private_database", false),
        }
    }
    let cli = env::var_os("LANKIDE_E2E_LARK_CLI").map(PathBuf::from);
    let oauth = env::var_os("LANKIDE_E2E_OAUTH_DIR")
        .map(PathBuf::from)
        .unwrap_or_default();
    for (request_key, id) in &owned.actor_requests {
        let Some(id) = id else {
            artifact.record_cleanup("actor_send_outcome_unknown", request_key, false);
            continue;
        };
        let parameters = json!({"message_id":id}).to_string();
        let success = cli.as_deref().is_some_and(|cli| {
            lark_json(
                cli,
                &oauth,
                &[
                    "im",
                    "messages",
                    "delete",
                    "--as",
                    "user",
                    "--params",
                    &parameters,
                    "--yes",
                    "--json",
                ],
            )
            .is_ok()
        });
        artifact.record_cleanup("actor_recall", id, success);
    }
    if let Some((fixture, profile)) = &owned.profile {
        let success = restore_profile(fixture, profile).await.is_ok();
        artifact.record_cleanup("profile_restore_and_readback", &fixture.app_id, success);
    }
}

fn native_cli_evidence(
    home: &Path,
    database: &Path,
    artifact_dir: &Path,
    expected_markers: &[(&str, &str)],
) -> Result<Value, String> {
    let db = runtime_db(database)?;
    let cli = live::runtime_binary().with_file_name("lankide");
    let hash = Sha256::digest(fs::read(&cli).map_err(|_| "read shipped capability CLI")?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut evidence = Vec::new();
    for &(agent, expected_marker) in expected_markers {
        let session_id: String = db.query_row(
            "SELECT a.native_session_id FROM execution_attempts a JOIN executions e ON e.execution_id=a.execution_id WHERE e.agent_id=?1 AND e.state='succeeded'",
            [agent], |row| row.get(0)).map_err(|_| "read succeeded native session binding")?;
        let directory = home
            .join("workspaces/feishu_team_e2e/agents")
            .join(agent)
            .join(".lankide/pi/sessions");
        let mut matched = None;
        for entry in fs::read_dir(&directory).map_err(|_| "native session directory missing")? {
            let path = entry.map_err(|_| "read session directory")?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|_| "read native session")?;
            let entries = String::from_utf8_lossy(&bytes)
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "decode native session")?;
            if !entries
                .iter()
                .any(|entry| entry["type"] == "session" && entry["id"] == session_id)
            {
                continue;
            }
            let mut successful = Vec::new();
            for entry in &entries {
                let Some(blocks) = entry.pointer("/message/content").and_then(Value::as_array)
                else {
                    continue;
                };
                for call in blocks {
                    let command = call
                        .pointer("/arguments/command")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if call["type"] != "toolCall"
                        || call["name"] != "bash"
                        || !is_direct_shipped_cli_send(command, &cli)
                    {
                        continue;
                    }
                    let Some(result) = entries.iter().find(|result| {
                        result.pointer("/message/role").and_then(Value::as_str)
                            == Some("toolResult")
                            && result.pointer("/message/toolCallId") == call.get("id")
                    }) else {
                        continue;
                    };
                    let Some((message_id, provider_message_id)) = accepted_cli_send(result) else {
                        continue;
                    };
                    let persisted = db
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM messages WHERE message_id=?1 AND provider_message_id=?2 AND direction='outbound' AND sender_kind='agent' AND sender_id=?3 AND send_status='sent' AND instr(content_markdown,?4)>0)",
                            params![message_id, provider_message_id, agent, expected_marker],
                            |row| row.get::<_, bool>(0),
                        )
                        .map_err(|_| "correlate accepted CLI result with Runtime message")?;
                    if persisted {
                        successful.push(json!({
                            "tool_call_id": call["id"],
                            "message_id": message_id,
                            "provider_message_id": provider_message_id,
                        }));
                    }
                }
            }
            if successful.len() != 1 {
                return Err(format!(
                    "{agent} native session must contain exactly one accepted shipped CLI send correlated with its persisted Runtime message"
                ));
            }
            let copy = artifact_dir.join(format!("{agent}-native.jsonl"));
            use std::io::Write;
            private_log(&copy)
                .write_all(&bytes)
                .map_err(|_| "retain native session evidence")?;
            matched = Some(
                json!({"agent":agent,"session_id":session_id,"accepted_cli_sends":successful,
                "native_sha256":Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),"native_evidence":copy}),
            );
        }
        evidence.push(matched.ok_or_else(|| format!("{agent} bound native session file missing"))?);
    }
    Ok(json!({"cli_path":cli,"cli_sha256":hash,"sessions":evidence}))
}

fn is_direct_shipped_cli_send(command: &str, cli: &Path) -> bool {
    let command = command.trim();
    let prefixes = [
        "$LANKIDE_CLI send ".to_owned(),
        "${LANKIDE_CLI} send ".to_owned(),
        format!("{} send ", cli.display()),
    ];
    prefixes.iter().any(|prefix| command.starts_with(prefix))
        && command.contains(" --content ")
        && !command.contains(['\n', '\r', '|', ';', '&', '`'])
        && !command.contains("$(")
}

fn accepted_cli_send(result: &Value) -> Option<(i64, String)> {
    if result.pointer("/message/isError") != Some(&Value::Bool(false)) {
        return None;
    }
    let text_blocks = result.pointer("/message/content")?.as_array()?;
    if text_blocks.len() != 1 || text_blocks[0]["type"] != "text" {
        return None;
    }
    let output: Value = serde_json::from_str(text_blocks[0]["text"].as_str()?.trim()).ok()?;
    if output["accepted"] != true || !output["error"].is_null() {
        return None;
    }
    let message_id = output["message_id"].as_i64()?;
    let provider_message_id = output["provider_message_id"].as_str()?;
    if message_id <= 0 || provider_message_id.is_empty() {
        return None;
    }
    Some((message_id, provider_message_id.to_owned()))
}

async fn wait_for_path(path: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("path did not appear: {}", path.display()))
}

fn load_feishu_apps(path: &Path) -> Result<BTreeMap<String, FeishuAgentFixture>, String> {
    use std::os::unix::fs::PermissionsExt;
    if fs::metadata(path)
        .map_err(|e| e.to_string())?
        .permissions()
        .mode()
        & 0o077
        != 0
    {
        return Err("Feishu fixture permissions are too broad".into());
    }
    let apps: Vec<FeishuAgentFixture> =
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|_| "invalid Feishu applications fixture".to_owned())?;
    if apps.len() != 2 {
        return Err("two Feishu Agent Apps are required".into());
    }
    Ok(apps
        .into_iter()
        .map(|app| (app.app_id.clone(), app))
        .collect())
}

fn hydrate_group_bots(
    lark_cli: &Path,
    oauth_dir: &Path,
    chat_id: &str,
    apps: &mut BTreeMap<String, FeishuAgentFixture>,
) -> Result<(), String> {
    let output = lark_json(
        lark_cli,
        oauth_dir,
        &[
            "im",
            "+chat-members-list",
            "--as",
            "user",
            "--chat-id",
            chat_id,
            "--member-types",
            "bot",
            "--page-all",
            "--json",
        ],
    )?;
    let bots = output
        .pointer("/data/bots")
        .and_then(Value::as_array)
        .ok_or_else(|| "Feishu group member response has no bots".to_owned())?;
    for bot in bots {
        let Some(app_id) = bot.get("app_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(fixture) = apps.get_mut(app_id) else {
            continue;
        };
        fixture.bot_open_id = bot
            .get("member_id")
            .and_then(Value::as_str)
            .unwrap_or(&fixture.bot_open_id)
            .to_owned();
        fixture.bot_name = bot
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&fixture.bot_name)
            .to_owned();
    }
    if apps.values().any(|fixture| {
        !bots
            .iter()
            .any(|bot| bot.get("app_id").and_then(Value::as_str) == Some(fixture.app_id.as_str()))
    }) {
        return Err("both Feishu Apps must be members of the E2E group".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn send_tracked(
    owned: &Mutex<OwnedResources>,
    artifact: &ScenarioArtifact,
    lark_cli: &Path,
    oauth_dir: &Path,
    chat_id: &str,
    agent: &FeishuAgentFixture,
    text: &str,
    request_key: &str,
) -> Result<String, String> {
    let mut owned = owned
        .lock()
        .map_err(|_| "owned mutation registry unavailable")?;
    owned.actor_requests.insert(request_key.into(), None);
    let recovery = artifact.dir.join("actor-recovery.json");
    write_private_json(
        &recovery,
        &json!({"chat_id":chat_id,"requests":owned.actor_requests}),
    )?;
    let id = send_user_mention(lark_cli, oauth_dir, chat_id, agent, text, request_key)?;
    owned
        .actor_requests
        .insert(request_key.into(), Some(id.clone()));
    write_private_json(
        &recovery,
        &json!({"chat_id":chat_id,"requests":owned.actor_requests}),
    )?;
    artifact.record("actor_message_created", &id, "ok");
    Ok(id)
}

fn send_user_mention(
    lark_cli: &Path,
    oauth_dir: &Path,
    chat_id: &str,
    agent: &FeishuAgentFixture,
    text: &str,
    request_key: &str,
) -> Result<String, String> {
    let content = json!({
        "text": format!(
            "<at user_id=\"{}\">{}</at> {}",
            agent.bot_open_id, agent.bot_name, text
        )
    })
    .to_string();
    let output = lark_json(
        lark_cli,
        oauth_dir,
        &[
            "im",
            "+messages-send",
            "--as",
            "user",
            "--chat-id",
            chat_id,
            "--msg-type",
            "text",
            "--content",
            &content,
            "--idempotency-key",
            request_key,
            "--json",
        ],
    )?;
    find_string(&output, "message_id")
        .ok_or_else(|| "Feishu user send returned no message_id".to_owned())
}

fn lark_json(lark_cli: &Path, oauth_dir: &Path, args: &[&str]) -> Result<Value, String> {
    use std::os::unix::process::CommandExt;
    let output_file = tempfile::tempfile().map_err(|_| "create private actor output")?;
    let mut child = Command::new(lark_cli)
        .args(args)
        .env("LARKSUITE_CLI_CONFIG_DIR", oauth_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            output_file.try_clone().map_err(|_| "clone actor output")?,
        ))
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| "start lark-cli")?;
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| "wait actor CLI")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.wait();
            return Err("lark-cli timed out; mutation outcome requires inspection".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    if !status.success() {
        return Err(format!(
            "lark-cli failed with exit {}",
            status.code().unwrap_or(-1)
        ));
    }
    use std::io::{Read, Seek};
    let mut output_file = output_file;
    output_file.rewind().map_err(|_| "rewind actor output")?;
    let mut stdout = String::new();
    output_file
        .take(8 * 1024 * 1024)
        .read_to_string(&mut stdout)
        .map_err(|_| "read actor output")?;
    let start = stdout.find('{').ok_or("lark-cli returned no JSON")?;
    let end = stdout
        .rfind('}')
        .ok_or("lark-cli returned incomplete JSON")?;
    serde_json::from_str(&stdout[start..=end]).map_err(|_| "decode lark-cli JSON".into())
}

fn find_string(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(object) => object
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| object.values().find_map(|value| find_string(value, key))),
        Value::Array(values) => values.iter().find_map(|value| find_string(value, key)),
        _ => None,
    }
}

fn runtime_db(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("open Runtime DB: {error}"))
}

async fn wait_for_running_execution(
    path: &Path,
    agent_id: &str,
    timeout: Duration,
) -> Result<i64, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(db) = runtime_db(path) {
            let result = db
                .query_row(
                    "SELECT execution_id FROM executions WHERE agent_id=?1 AND state='running' ORDER BY execution_id DESC LIMIT 1",
                    params![agent_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| format!("query running execution: {error}"))?;
            if let Some(execution_id) = result {
                return Ok(execution_id);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(format!("Agent {agent_id} did not reach running state"))
}

async fn wait_for_agent_execution(
    path: &Path,
    agent_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_count(
        path,
        "SELECT COUNT(*) FROM executions WHERE agent_id=?1",
        &[agent_id],
        1,
        timeout,
        false,
    )
    .await
}

async fn wait_for_steers(
    path: &Path,
    execution_id: i64,
    minimum: i64,
    timeout: Duration,
) -> Result<(), String> {
    let id = execution_id.to_string();
    wait_for_count(
        path,
        "SELECT COUNT(*) FROM steer_deliveries WHERE execution_id=CAST(?1 AS INTEGER) AND state='accepted'",
        &[&id],
        minimum,
        timeout,
        false,
    )
    .await
}

async fn wait_for_outbound_marker(
    path: &Path,
    marker: &str,
    timeout: Duration,
) -> Result<(), String> {
    let pattern = format!("%{marker}%");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let db = runtime_db(path)?;
        let sent: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE direction='outbound' AND content_markdown LIKE ?1 AND send_status='sent')",
            [&pattern], |row| row.get(0)).map_err(|_| "read sent reply evidence")?;
        if sent {
            return Ok(());
        }
        let settled: bool = db.query_row(
            "SELECT count(*)>0 AND sum(CASE WHEN state IN ('succeeded','failed','cancelled') THEN 0 ELSE 1 END)=0 FROM executions",
            [], |row| row.get(0)).map_err(|_| "read execution completion evidence")?;
        if settled {
            return Err(format!(
                "all executions finished without required reply marker {marker}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(format!("required reply marker timed out: {marker}"))
}

async fn wait_for_execution_finished(
    path: &Path,
    execution_id: i64,
    timeout: Duration,
) -> Result<(), String> {
    let id = execution_id.to_string();
    wait_for_count(
        path,
        "SELECT COUNT(*) FROM executions WHERE execution_id=CAST(?1 AS INTEGER) AND state='succeeded'",
        &[&id],
        1,
        timeout,
        false,
    )
    .await
}

async fn wait_for_count(
    path: &Path,
    sql: &str,
    arguments: &[&str],
    minimum: i64,
    timeout: Duration,
    accept_zero_after_database_ready: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(db) = runtime_db(path) {
            let count = match arguments {
                [] => db.query_row(sql, [], |row| row.get::<_, i64>(0)),
                [one] => db.query_row(sql, params![one], |row| row.get::<_, i64>(0)),
                _ => return Err("unsupported E2E SQL argument count".into()),
            };
            match count {
                Ok(value)
                    if value >= minimum && (!accept_zero_after_database_ready || value >= 0) =>
                {
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) if error.to_string().contains("no such table") => {}
                Err(error) => return Err(format!("query Runtime count: {error}")),
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(format!("Runtime evidence did not reach {minimum}: {sql}"))
}

fn collect_evidence(
    path: &Path,
    execution_id: i64,
    scenario_steer_provider_message_ids: &[&str; 2],
) -> Result<Value, String> {
    let db = runtime_db(path)?;
    let inbound_messages: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE direction='inbound'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("count inbound messages: {error}"))?;
    let outbound_messages: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE direction='outbound' AND send_status='sent'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("count outbound messages: {error}"))?;
    let steer_deliveries: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM steer_deliveries WHERE execution_id=?1 AND state='accepted'",
            params![execution_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("count accepted steers: {error}"))?;
    let scenario_steer_deliveries: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM steer_deliveries s JOIN agent_inbox_items i ON i.inbox_item_id=s.inbox_item_id JOIN messages m ON m.message_id=i.source_id WHERE s.execution_id=?1 AND s.state='accepted' AND m.provider_message_id IN (?2,?3)",
            params![
                execution_id,
                scenario_steer_provider_message_ids[0],
                scenario_steer_provider_message_ids[1]
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("count scenario-correlated accepted steers: {error}"))?;
    let execution_count: i64 = db
        .query_row("SELECT COUNT(*) FROM executions", [], |row| row.get(0))
        .map_err(|error| format!("count executions: {error}"))?;
    let failed_executions: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM executions WHERE state='failed'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("count failed executions: {error}"))?;
    let unconsumed_inbox_items: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_inbox_items WHERE state!='consumed'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("count unconsumed Inbox items: {error}"))?;
    Ok(json!({
        "inbound_messages":inbound_messages,
        "outbound_messages":outbound_messages,
        "accepted_steer_deliveries":steer_deliveries,
        "scenario_accepted_steer_deliveries":scenario_steer_deliveries,
        "executions":execution_count,
        "failed_executions":failed_executions,
        "unconsumed_inbox_items":unconsumed_inbox_items
    }))
}

fn require_owner_only(path: &Path) -> Result<(), String> {
    let mode = fs::metadata(path)
        .map_err(|error| format!("fixture metadata: {error}"))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err("fixture permissions are too broad".into());
    }
    Ok(())
}
