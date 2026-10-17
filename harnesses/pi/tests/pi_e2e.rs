const DEFAULT_MODEL: &str = "deepseek-v4-flash";
#[path = "../../../crates/harness/tests/support/process.rs"]
mod process_support;
use process_support::{ProcessHarnessFactory, ProcessHarnessSpec};

use lankide_harness::{
    AgentEventKind, ExecuteRequest, HarnessConfig, HarnessErrorCode, HarnessFactory, HarnessInfo,
    ModelRef, SessionRequest, ToolDefinition, ToolResultRequest,
};
use lankide_harness_pi::PiHarness;
use lankide_testkit::{GateSelection, ProbeConfig, run_prerequisite_probe};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_stream::StreamExt;

const TURN_TIMEOUT: Duration = Duration::from_secs(120);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Pi CLI, deepseek-v4-flash, and explicit live opt-in"]
async fn pi_e2e() -> Result<(), String> {
    if std::env::var("LANKIDE_E2E_LIVE").as_deref() != Ok("1") {
        return Err("set LANKIDE_E2E_LIVE=1 for the Pi live gate".to_owned());
    }
    let prerequisite = run_prerequisite_probe(&ProbeConfig::from_environment(
        GateSelection::HarnessLive,
        true,
    ));
    if !prerequisite.is_green() {
        return Err(format!(
            "PREREQUISITE_MISSING\n{}",
            prerequisite.render_text()
        ));
    }

    let run_id = std::env::var("LANKIDE_E2E_RUN_ID").unwrap_or_else(|_| {
        format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    });
    let nonce = format!("lankide-pi-e2e-{run_id}");
    let root = std::env::temp_dir()
        .join("lankide-pi-e2e-artifacts")
        .join(&run_id);
    let workspace = root.join("workspace");
    let agent_dir = root.join("overlay/agent");
    let session_dir = root.join("overlay/sessions");
    let artifact_dir = root.join("artifacts");
    let pid_journal = artifact_dir.join("pids.tsv");
    for path in [&workspace, &agent_dir, &session_dir, &artifact_dir] {
        fs::create_dir_all(path).map_err(|error| error.to_string())?;
    }
    copy_auth_overlay(&agent_dir)?;
    let _auth_cleanup = AuthCleanup(agent_dir.clone());
    fs::write(
        artifact_dir.join("run.json"),
        serde_json::to_vec_pretty(&json!({
            "run_id":run_id,
            "status":"running",
            "workspace":workspace,
            "session_dir":session_dir,
            "pid_journal":pid_journal
        }))
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    eprintln!("Pi E2E artifact: {}", artifact_dir.display());

    let cli_path = std::env::var_os("LANKIDE_PI_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/homebrew/bin/pi"));
    let mut environment = BTreeMap::new();
    environment.insert(
        "PI_CODING_AGENT_DIR".to_owned(),
        agent_dir.display().to_string(),
    );
    environment.insert(
        "PI_CODING_AGENT_SESSION_DIR".to_owned(),
        session_dir.display().to_string(),
    );
    environment.insert(
        "LANKIDE_PI_PID_JOURNAL".to_owned(),
        pid_journal.display().to_string(),
    );
    environment.insert(
        "LANKIDE_PI_ARTIFACT_DIR".to_owned(),
        artifact_dir.display().to_string(),
    );
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "pi".to_owned(),
        model: ModelRef::new(DEFAULT_MODEL, "DeepSeek V4 Flash"),
        thinking_level: "low".to_owned(),
        cli_path,
        environment,
        config_fingerprint: format!("pi-live-{run_id}"),
    };
    let tool = ToolDefinition {
        name: "e2e_echo".to_owned(),
        description: "Return the supplied test nonce. Use only when explicitly requested."
            .to_owned(),
        input_schema: json!({
            "type":"object",
            "properties":{"nonce":{"type":"string"}},
            "required":["nonce"],
            "additionalProperties":false
        }),
        output_schema: json!({"type":"object"}),
    };
    let factory = PiHarness::new();
    let probe = factory
        .probe(&config)
        .await
        .map_err(|error| error.to_string())?;
    if probe.version.as_deref().is_none_or(str::is_empty) {
        return Err("Pi probe omitted version".to_owned());
    }

    let base_request = SessionRequest {
        permission_policy: Default::default(),
        config: config.clone(),
        working_directory: workspace.clone(),
        system_prompt: "Follow the user request. Use only the provided tools.".to_owned(),
        tools: vec![tool.clone()],
        native_session_id: None,
    };
    let session = factory
        .start(base_request.clone())
        .await
        .map_err(|error| error.to_string())?;
    mark_phase(&artifact_dir, "tool_turn")?;
    let native_id = session.entry().native_session_id;
    if native_id.is_empty() {
        return Err("Pi start did not bind native session".to_owned());
    }
    let stream = session
        .execute(ExecuteRequest {
            input: format!(
                "Call e2e_echo exactly once with nonce {nonce}. After the tool result, include that nonce in the final answer."
            ),
        })
        .await
        .map_err(|error| error.to_string())?;
    drive_tool_turn(session.as_ref(), stream, &nonce).await?;
    session.close().await.map_err(|error| error.to_string())?;
    require_session_artifact(&session_dir, &native_id)?;

    let resumed = factory
        .resume(SessionRequest {
            native_session_id: Some(native_id.clone()),
            ..base_request.clone()
        })
        .await
        .map_err(|error| error.to_string())?;
    mark_phase(&artifact_dir, "resume_turn")?;
    if resumed.entry().native_session_id != native_id {
        return Err("Pi resume rebound another native session".to_owned());
    }
    let mut resume_stream = resumed
        .execute(ExecuteRequest {
            input: "Use the native bash tool to write the nonce from the previous turn to native-command.txt using printf, then state only that nonce. Do not call e2e_echo again or read other files or environment variables.".to_owned(),
        })
        .await
        .map_err(|error| error.to_string())?;
    let resume_output = collect_terminal(&mut resume_stream).await?;
    if !resume_output.contains(&nonce) {
        return Err("resumed Pi turn did not retain the tool nonce".to_owned());
    }
    if fs::read_to_string(workspace.join("native-command.txt"))
        .map_err(|_| "native Pi shell did not create its output")?
        .trim()
        != nonce
    {
        return Err("native Pi command output differs from the tool nonce".into());
    }
    resumed.close().await.map_err(|error| error.to_string())?;

    let cancellable = factory
        .start(SessionRequest {
            permission_policy: Default::default(),
            config,
            working_directory: workspace,
            system_prompt: "Follow the user request.".to_owned(),
            tools: Vec::new(),
            native_session_id: None,
        })
        .await
        .map_err(|error| error.to_string())?;
    mark_phase(&artifact_dir, "long_turn")?;
    let mut long_stream = cancellable
        .execute(ExecuteRequest {
            input: format!(
                "Write at least 4000 numbered lines containing {nonce}; do not summarize."
            ),
        })
        .await
        .map_err(|error| error.to_string())?;
    wait_for_delta(&mut long_stream).await?;
    mark_phase(&artifact_dir, "steer")?;
    let steer = cancellable
        .steer(format!("steer-{run_id}"), format!("Use marker {nonce}"))
        .await
        .map_err(|error| error.to_string())?;
    if !steer.accepted {
        return Err("Pi steer was not accepted".to_owned());
    }
    mark_phase(&artifact_dir, "cancel")?;
    let cancel = cancellable
        .cancel("live gate cancellation".to_owned())
        .await
        .map_err(|error| error.to_string())?;
    if !cancel.accepted {
        return Err("Pi cancel was not accepted".to_owned());
    }
    wait_for_cancelled(&mut long_stream).await?;
    cancellable
        .close()
        .await
        .map_err(|error| error.to_string())?;

    verify_abort_without_response(&root, &pid_journal).await?;
    verify_pid_cleanup(&pid_journal)?;
    mark_phase(&artifact_dir, "scripted_fault")?;
    verify_scripted_fault_boundary(&root).await?;
    fs::write(
        artifact_dir.join("run.json"),
        serde_json::to_vec_pretty(&json!({
            "run_id":run_id,
            "status":"passed",
            "native_session_id":native_id,
            "session_artifact":true,
            "tool_roundtrip":true,
            "resume":true,
            "steer":true,
            "cancel":true,
            "pid_cleanup":true
        }))
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

struct AuthCleanup(PathBuf);

impl Drop for AuthCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mark_phase(artifact_dir: &Path, phase: &str) -> Result<(), String> {
    fs::write(artifact_dir.join("phase.txt"), format!("{phase}\n"))
        .map_err(|error| error.to_string())
}

async fn verify_scripted_fault_boundary(root: &Path) -> Result<(), String> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pi_fault_peer.py");
    let factory = ProcessHarnessFactory::new(
        HarnessInfo::new("pi", "Pi fault peer").map_err(|error| error.to_string())?,
        ProcessHarnessSpec {
            arguments: vec![fixture.display().to_string()],
            bind_timeout: Duration::from_secs(3),
            cancel_grace: Duration::from_millis(100),
            close_grace: Duration::from_millis(100),
            pid_journal: None,
        },
    );
    let session = factory
        .start(SessionRequest {
            permission_policy: Default::default(),
            config: HarnessConfig {
                launch_arguments: Default::default(),
                harness_id: "pi".to_owned(),
                model: ModelRef::new(DEFAULT_MODEL, "DeepSeek V4 Flash"),
                thinking_level: "low".to_owned(),
                cli_path: PathBuf::from("/usr/bin/python3"),
                environment: BTreeMap::new(),
                config_fingerprint: "pi-scripted-fault".to_owned(),
            },
            working_directory: root.to_owned(),
            system_prompt: "fault fixture".to_owned(),
            tools: Vec::new(),
            native_session_id: None,
        })
        .await
        .map_err(|error| error.to_string())?;
    let mut stream = session
        .execute(ExecuteRequest {
            input: "malformed".to_owned(),
        })
        .await
        .map_err(|error| error.to_string())?;
    let error = timeout(Duration::from_secs(3), stream.next())
        .await
        .map_err(|_| "scripted Pi fault timed out".to_owned())?
        .ok_or_else(|| "scripted Pi fault stream ended".to_owned())?
        .expect_err("scripted peer must emit an invalid event");
    if error.code != HarnessErrorCode::InvalidEvent {
        return Err(format!("unexpected scripted fault code: {:?}", error.code));
    }
    session.close().await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn verify_abort_without_response(root: &Path, pid_journal: &Path) -> Result<(), String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pi_native_peer.py");
    let executable = root.join("pi-native-peer");
    fs::copy(source, &executable).map_err(|error| error.to_string())?;
    let mut permissions = fs::metadata(&executable)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).map_err(|error| error.to_string())?;

    for mode in ["agent_end", "no_terminal"] {
        let cwd = root.join(format!("native-fault-{mode}"));
        let session_dir = cwd.join("sessions");
        fs::create_dir_all(&session_dir).map_err(|error| error.to_string())?;
        let mut environment = BTreeMap::new();
        environment.insert("PI_NATIVE_FAULT".to_owned(), mode.to_owned());
        environment.insert(
            "PI_CODING_AGENT_DIR".to_owned(),
            cwd.join("agent").display().to_string(),
        );
        environment.insert(
            "PI_CODING_AGENT_SESSION_DIR".to_owned(),
            session_dir.display().to_string(),
        );
        environment.insert(
            "LANKIDE_PI_PID_JOURNAL".to_owned(),
            pid_journal.display().to_string(),
        );
        let session = PiHarness::new()
            .start(SessionRequest {
                permission_policy: Default::default(),
                config: HarnessConfig {
                    launch_arguments: Default::default(),
                    harness_id: "pi".to_owned(),
                    model: ModelRef::new(DEFAULT_MODEL, "DeepSeek V4 Flash"),
                    thinking_level: "low".to_owned(),
                    cli_path: executable.clone(),
                    environment,
                    config_fingerprint: format!("pi-native-fault-{mode}"),
                },
                working_directory: cwd,
                system_prompt: "native fault fixture".to_owned(),
                tools: Vec::new(),
                native_session_id: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut stream = session
            .execute(ExecuteRequest {
                input: "long".to_owned(),
            })
            .await
            .map_err(|error| error.to_string())?;
        session
            .steer(format!("fault-steer-{mode}"), "steer".to_owned())
            .await
            .map_err(|error| error.to_string())?;
        session
            .cancel(format!("fault-cancel-{mode}"))
            .await
            .map_err(|error| error.to_string())?;
        wait_for_cancelled(&mut stream).await?;
        session.close().await.map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn drive_tool_turn(
    session: &dyn lankide_harness::HarnessSession,
    mut stream: lankide_harness::AgentEventStream,
    nonce: &str,
) -> Result<(), String> {
    let deadline = timeout(TURN_TIMEOUT, async {
        let mut saw_started = false;
        let mut saw_finished = false;
        while let Some(event) = stream.next().await {
            let event = event.map_err(|error| error.to_string())?;
            match event.kind {
                AgentEventKind::ToolStarted {
                    tool_call_id,
                    tool_name,
                    arguments,
                } => {
                    if tool_name != "e2e_echo"
                        || arguments.get("nonce").and_then(|value| value.as_str()) != Some(nonce)
                    {
                        return Err("Pi tool call did not preserve the nonce contract".to_owned());
                    }
                    saw_started = true;
                    session
                        .tool_result(ToolResultRequest {
                            tool_call_id,
                            success: true,
                            content_markdown: nonce.to_owned(),
                            structured_content: json!({"nonce":nonce}),
                            resources: Vec::new(),
                            error: None,
                        })
                        .await
                        .map_err(|error| error.to_string())?;
                }
                AgentEventKind::ToolFinished {
                    success, output, ..
                } => {
                    if !success
                        || output
                            .pointer("/structured_content/nonce")
                            .and_then(|value| value.as_str())
                            != Some(nonce)
                    {
                        return Err("Pi ToolFinished did not preserve ToolResult".to_owned());
                    }
                    saw_finished = true;
                }
                AgentEventKind::Completed { final_output } => {
                    if !saw_started || !saw_finished || !final_output.contains(nonce) {
                        return Err("Pi tool turn completed without protocol facts".to_owned());
                    }
                    return Ok(());
                }
                AgentEventKind::Failed { code, message, .. } => {
                    return Err(format!("Pi tool turn failed: {code}: {message}"));
                }
                AgentEventKind::Cancelled { reason } => {
                    return Err(format!("Pi tool turn cancelled: {reason}"));
                }
                _ => {}
            }
        }
        Err("Pi tool stream ended before terminal event".to_owned())
    })
    .await;
    deadline.map_err(|_| "Pi tool turn timed out".to_owned())?
}

async fn collect_terminal(
    stream: &mut lankide_harness::AgentEventStream,
) -> Result<String, String> {
    timeout(TURN_TIMEOUT, async {
        while let Some(event) = stream.next().await {
            match event.map_err(|error| error.to_string())?.kind {
                AgentEventKind::Completed { final_output } => return Ok(final_output),
                AgentEventKind::Failed { code, message, .. } => {
                    return Err(format!("Pi turn failed: {code}: {message}"));
                }
                AgentEventKind::Cancelled { reason } => {
                    return Err(format!("Pi turn cancelled: {reason}"));
                }
                _ => {}
            }
        }
        Err("Pi stream ended before terminal event".to_owned())
    })
    .await
    .map_err(|_| "Pi turn timed out".to_owned())?
}

async fn wait_for_delta(stream: &mut lankide_harness::AgentEventStream) -> Result<(), String> {
    timeout(TURN_TIMEOUT, async {
        while let Some(event) = stream.next().await {
            match event.map_err(|error| error.to_string())?.kind {
                AgentEventKind::AssistantDelta { .. } | AgentEventKind::ReasoningDelta { .. } => {
                    return Ok(());
                }
                AgentEventKind::Completed { .. } => {
                    return Err("Pi long turn completed before steer/cancel".to_owned());
                }
                AgentEventKind::Failed { code, message, .. } => {
                    return Err(format!("Pi long turn failed: {code}: {message}"));
                }
                _ => {}
            }
        }
        Err("Pi long stream ended before delta".to_owned())
    })
    .await
    .map_err(|_| "Pi long turn timed out before delta".to_owned())?
}

async fn wait_for_cancelled(stream: &mut lankide_harness::AgentEventStream) -> Result<(), String> {
    timeout(Duration::from_secs(15), async {
        while let Some(event) = stream.next().await {
            match event.map_err(|error| error.to_string())?.kind {
                AgentEventKind::Cancelled { .. } => return Ok(()),
                AgentEventKind::Completed { .. } => {
                    return Err("Pi completed after cancel".to_owned());
                }
                AgentEventKind::Failed { code, message, .. } => {
                    return Err(format!("Pi failed after cancel: {code}: {message}"));
                }
                _ => {}
            }
        }
        Err("Pi cancel stream ended without Cancelled".to_owned())
    })
    .await
    .map_err(|_| "Pi cancellation timed out".to_owned())?
}

fn copy_auth_overlay(agent_dir: &Path) -> Result<(), String> {
    let source = std::env::var_os("LANKIDE_PI_MODELS_PATH")
        .map(PathBuf::from)
        .ok_or("LANKIDE_PI_MODELS_PATH is required")?;
    fs::copy(&source, agent_dir.join("models.json"))
        .map(|_| ())
        .map_err(|error| format!("copy isolated Pi model config: {error}"))
}

fn require_session_artifact(session_dir: &Path, native_id: &str) -> Result<(), String> {
    let suffix = format!("_{native_id}.jsonl");
    let count = fs::read_dir(session_dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))
        .count();
    if count == 1 {
        Ok(())
    } else {
        Err(format!("expected one Pi JSONL artifact, found {count}"))
    }
}

fn verify_pid_cleanup(journal: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(journal).map_err(|error| error.to_string())?;
    let mut spawned = Vec::new();
    let mut reaped = Vec::new();
    for line in contents.lines() {
        let mut fields = line.split('\t');
        let action = fields.next().unwrap_or("");
        let pid = fields
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| "invalid Pi PID journal".to_owned())?;
        match action {
            "spawned" => spawned.push(pid),
            "reaped" => reaped.push(pid),
            _ => {}
        }
    }
    for pid in spawned {
        if !reaped.contains(&pid) {
            return Err(format!("Pi PID {pid} was not reaped"));
        }
        let status = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            return Err(format!("Pi PID {pid} is still alive"));
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires installed native CLI; reads model directory without a model turn"]
async fn native_model_catalog() {
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "pi".into(),
        cli_path: "pi".into(),
        model: ModelRef::new("", ""),
        thinking_level: String::new(),
        environment: BTreeMap::new(),
        config_fingerprint: String::new(),
    };
    let models = PiHarness::new()
        .models(&config)
        .await
        .expect("native model directory");
    assert!(!models.is_empty(), "authenticated CLI should offer models");
    let ids: std::collections::BTreeSet<_> = models.iter().map(|model| &model.id).collect();
    assert_eq!(ids.len(), models.len(), "model IDs must be unambiguous");
    assert!(
        models
            .iter()
            .all(|model| !model.id.is_empty() && !model.name.is_empty())
    );
    println!(
        "native directory: {} models; no model turn requested",
        models.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "S7.3 requires Pi CLI, isolated model config, and LANKIDE_E2E_LIVE=1"]
async fn s7_3_real_pi_uses_a_skill_authored_in_its_agent_home() -> Result<(), String> {
    if std::env::var("LANKIDE_E2E_LIVE").as_deref() != Ok("1") {
        return Err("set LANKIDE_E2E_LIVE=1 for the Pi live gate".into());
    }
    let prerequisite = run_prerequisite_probe(&ProbeConfig::from_environment(
        GateSelection::HarnessLive,
        true,
    ));
    if !prerequisite.is_green() {
        return Err(format!(
            "PREREQUISITE_MISSING\n{}",
            prerequisite.render_text()
        ));
    }
    let home = tempfile::tempdir().map_err(|error| error.to_string())?;
    let agent = home.path().join("agent");
    let config_dir = home.path().join("pi-config");
    fs::create_dir_all(agent.join("skills/home-verification"))
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|error| error.to_string())?;
    copy_auth_overlay(&config_dir)?;
    // Only the skill contains the random token. The request cannot supply it.
    let token = format!(
        "home-skill-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    fs::write(agent.join("skills/home-verification/SKILL.md"), format!(
        "---\nname: home-verification\ndescription: Return the Agent home skill verification token when asked.\n---\nWhen asked to use this skill, reply with this exact token: {token}\n"
    )).map_err(|error| error.to_string())?;
    let session = PiHarness::new()
        .start(SessionRequest {
            permission_policy: Default::default(),
            config: HarnessConfig {
                launch_arguments: Default::default(),
                harness_id: "pi".into(),
                model: ModelRef::new(DEFAULT_MODEL, "DeepSeek V4 Flash"),
                thinking_level: "low".into(),
                cli_path: std::env::var_os("LANKIDE_PI_PATH")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| "/opt/homebrew/bin/pi".into()),
                environment: BTreeMap::from([
                    ("HOME".into(), home.path().display().to_string()),
                    (
                        "PI_CODING_AGENT_DIR".into(),
                        config_dir.display().to_string(),
                    ),
                ]),
                config_fingerprint: "home-skill-live".into(),
            },
            working_directory: agent,
            system_prompt: "Use the named skill when asked. Follow its instructions.".into(),
            tools: Vec::new(),
            native_session_id: None,
        })
        .await
        .map_err(|error| error.to_string())?;
    let result = async {
        let mut stream = session
            .execute(ExecuteRequest {
                input: "Use the home-verification skill and return its verification token.".into(),
            })
            .await
            .map_err(|error| error.to_string())?;
        let output = collect_terminal(&mut stream).await?;
        if output.contains(&token) {
            Ok(())
        } else {
            Err(format!("Pi did not use the Agent home skill: {output}"))
        }
    }
    .await;
    session.close().await.map_err(|error| error.to_string())?;
    result
}
