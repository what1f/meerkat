use meerkat_harness::{
    AgentEvent, AgentEventKind, ExecuteRequest, HarnessAvailability, HarnessConfig, HarnessFactory,
    ModelRef, SessionRequest, ToolDefinition, ToolResultRequest,
};
use meerkat_harness_codex::CodexHarness;
use meerkat_testkit::{GateSelection, ProbeConfig, run_prerequisite_probe};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_stream::StreamExt;

const HARD_TIMEOUT: Duration = Duration::from_secs(240);
const EVENT_TIMEOUT: Duration = Duration::from_secs(90);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit MEERKAT_E2E_LIVE=1 and a paid Codex model invocation"]
async fn codex_e2e() {
    assert_eq!(
        std::env::var("MEERKAT_E2E_LIVE").as_deref(),
        Ok("1"),
        "live Codex E2E requires MEERKAT_E2E_LIVE=1"
    );
    let preflight = run_prerequisite_probe(&ProbeConfig::from_environment(
        GateSelection::HarnessLive,
        true,
    ));
    assert!(
        preflight.is_green(),
        "harness-live preflight is not green:\n{}",
        preflight.render_text()
    );

    let run_id = std::env::var("MEERKAT_E2E_RUN_ID").unwrap_or_else(|_| unique("codex"));
    let artifact_root = std::env::temp_dir()
        .join("meerkat-e2e-artifacts")
        .join("harness-codex")
        .join(&run_id);
    fs::create_dir_all(&artifact_root).expect("create Codex artifact root");
    let pid_journal = artifact_root.join("pids.tsv");
    let started_at = unix_millis();
    let started = std::time::Instant::now();

    let scenario = timeout(HARD_TIMEOUT, run_scenario(&artifact_root, &pid_journal))
        .await
        .map_err(|_| "hard timeout".to_owned())
        .and_then(|result| result);
    let cleanup = verify_pid_cleanup(&pid_journal);
    let outcome = match (scenario, cleanup) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(scenario), Ok(())) => Err(scenario),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(scenario), Err(cleanup)) => Err(format!("{scenario}; cleanup failed: {cleanup}")),
    };

    let artifact = json!({
        "run_id": run_id,
        "module": "harness-codex",
        "classification": "live",
        "model": "gpt-5.6-luna",
        "reasoning": "low",
        "started_at_unix_ms": started_at,
        "duration_ms": started.elapsed().as_millis(),
        "scenarios": [
            "probe_start_native_id_output",
            "close_resume_same_thread",
            "steer_dynamic_tool_result",
            "cancel_close_pid_cleanup"
        ],
        "summary": outcome.as_ref().ok(),
        "status": if outcome.is_ok() { "passed" } else { "failed" },
        "error": outcome.as_ref().err(),
    });
    let artifact_path = artifact_root.join("result.json");
    fs::write(
        &artifact_path,
        serde_json::to_vec_pretty(&artifact).expect("serialize Codex artifact"),
    )
    .expect("write Codex artifact");
    println!("Codex E2E artifact: {}", artifact_path.display());
    if let Err(error) = outcome {
        panic!("Codex live E2E failed: {error}");
    }
}

async fn run_scenario(
    artifact_root: &Path,
    pid_journal: &Path,
) -> Result<serde_json::Value, String> {
    let cli_path = resolve_codex_path()?;
    let work_root = tempfile::Builder::new()
        .prefix("meerkat-codex-live-")
        .tempdir()
        .map_err(display)?;
    let codex_home = work_root.path().join("codex-home");
    prepare_overlay(&codex_home)?;
    let nonce = unique("nonce");
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "codex".to_owned(),
        model: ModelRef::new("gpt-5.6-luna", "GPT-5.6 Luna"),
        thinking_level: "low".to_owned(),
        cli_path,
        environment: BTreeMap::from([(
            "CODEX_HOME".to_owned(),
            codex_home.to_string_lossy().into_owned(),
        )]),
        config_fingerprint: format!("codex-live-{nonce}"),
    };
    let factory = CodexHarness::new().with_pid_journal(pid_journal);
    let probe = factory.probe(&config).await.map_err(display)?;
    require(
        probe.availability == HarnessAvailability::Available,
        probe.reason.as_deref().unwrap_or("Codex probe unavailable"),
    )?;

    let session = factory
        .start(session_request(&config, work_root.path(), None))
        .await
        .map_err(display)?;
    let native_id = session.entry().native_session_id;
    require(!native_id.is_empty(), "Codex native thread ID is empty")?;
    let first_events = collect_terminal(
        session
            .execute(ExecuteRequest {
                input: format!("Reply with exactly this nonce and nothing else: {nonce}"),
            })
            .await
            .map_err(display)?,
    )
    .await?;
    require_successful_turn(&first_events)?;
    require(
        has_native_id(&first_events, &native_id),
        "native ID event missing",
    )?;
    let first_output = final_output(&first_events).unwrap_or_default();
    require(
        first_output.contains(&nonce),
        format!("initial output did not contain the nonce: {first_events:?}"),
    )?;
    session.close().await.map_err(display)?;

    let resumed = factory
        .resume(session_request(
            &config,
            work_root.path(),
            Some(native_id.clone()),
        ))
        .await
        .map_err(display)?;
    require(
        resumed.entry().native_session_id == native_id,
        "Codex resume changed the native thread ID",
    )?;

    let mut tool_events = resumed
        .execute(ExecuteRequest {
            input: format!(
                "Call the feishu_reaction_catalog tool exactly once with {{\"nonce\":\"{nonce}\"}}, then include the returned nonce in the final answer."
            ),
        })
        .await
        .map_err(display)?;
    let (tool_call_id, mut observed) = wait_for_tool_call(&mut tool_events, &nonce).await?;
    let steer = resumed
        .steer(
            format!("steer-{nonce}"),
            format!("After the tool returns, keep the exact nonce {nonce} in the answer."),
        )
        .await
        .map_err(display)?;
    require(steer.accepted, "Codex steer was not accepted")?;
    resumed
        .tool_result(ToolResultRequest {
            tool_call_id: tool_call_id.clone(),
            success: true,
            content_markdown: nonce.clone(),
            structured_content: json!({"nonce": nonce}),
            resources: Vec::new(),
            error: None,
        })
        .await
        .map_err(display)?;
    observed.extend(collect_terminal(tool_events).await?);
    require_successful_turn(&observed)?;
    require(
        observed.iter().any(|event| {
            matches!(&event.kind, AgentEventKind::ToolFinished { tool_call_id: id, success: true, .. } if id == &tool_call_id)
        }),
        "Codex ToolFinished event missing",
    )?;
    require(
        final_output(&observed).is_some_and(|output| output.contains(&nonce)),
        "tool turn final output did not contain nonce",
    )?;

    let mut cancel_events = resumed
        .execute(ExecuteRequest {
            input: format!(
                "Call feishu_reaction_catalog with {{\"nonce\":\"cancel-{nonce}\"}} and wait for its result."
            ),
        })
        .await
        .map_err(display)?;
    let (_, cancel_prefix) =
        wait_for_tool_call(&mut cancel_events, &format!("cancel-{nonce}")).await?;
    let cancel = resumed
        .cancel("live E2E cancellation".to_owned())
        .await
        .map_err(display)?;
    require(cancel.accepted, "Codex cancel was not accepted")?;
    let mut cancelled = cancel_prefix;
    cancelled.extend(collect_terminal(cancel_events).await?);
    require(
        cancelled
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::Cancelled { .. })),
        "Codex cancel terminal event missing",
    )?;
    resumed.close().await.map_err(display)?;

    let session_artifacts = count_jsonl(&codex_home);
    require(
        session_artifacts > 0,
        "isolated Codex home has no session JSONL",
    )?;
    fs::write(
        artifact_root.join("session-summary.json"),
        serde_json::to_vec_pretty(&json!({
            "native_thread_id": native_id,
            "isolated_session_jsonl_count": session_artifacts,
            "nonce_verified": true,
            "tool_call_id": tool_call_id,
        }))
        .map_err(display)?,
    )
    .map_err(display)?;
    Ok(json!({
        "native_thread_id_present": true,
        "isolated_session_jsonl_count": session_artifacts,
        "nonce_verified": true,
        "tool_result_verified": true,
        "cancel_verified": true,
    }))
}

fn session_request(
    config: &HarnessConfig,
    cwd: &Path,
    native_session_id: Option<String>,
) -> SessionRequest {
    SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config: config.clone(),
        working_directory: cwd.to_path_buf(),
        system_prompt: "You are running a protocol test. Do not inspect files, the repository, memory, environment variables, or external services. Follow only the nonce and test-tool instructions in the user message.".to_owned(),
        tools: vec![ToolDefinition {
            name: "feishu_reaction_catalog".to_owned(),
            description: "Return the supplied test nonce without modification.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {"nonce": {"type": "string"}},
                "required": ["nonce"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {"nonce": {"type": "string"}},
                "required": ["nonce"]
            }),
        }],
        native_session_id,
    }
}

async fn wait_for_tool_call(
    events: &mut meerkat_harness::AgentEventStream,
    expected_nonce: &str,
) -> Result<(String, Vec<AgentEvent>), String> {
    let mut observed = Vec::new();
    loop {
        let event = timeout(EVENT_TIMEOUT, events.next())
            .await
            .map_err(|_| "timed out waiting for Codex tool call".to_owned())?
            .ok_or_else(|| "Codex stream ended before tool call".to_owned())?
            .map_err(display)?;
        if let AgentEventKind::ToolStarted {
            tool_call_id,
            tool_name,
            arguments,
        } = &event.kind
        {
            require(
                tool_name == "feishu_reaction_catalog",
                "Codex tool name was not restored",
            )?;
            require(
                arguments.get("nonce").and_then(|value| value.as_str()) == Some(expected_nonce),
                "Codex tool arguments did not contain expected nonce",
            )?;
            let id = tool_call_id.clone();
            observed.push(event);
            return Ok((id, observed));
        }
        if event.is_terminal() {
            return Err("Codex turn ended before tool call".to_owned());
        }
        observed.push(event);
    }
}

async fn collect_terminal(
    mut events: meerkat_harness::AgentEventStream,
) -> Result<Vec<AgentEvent>, String> {
    let mut observed = Vec::new();
    loop {
        let event = timeout(EVENT_TIMEOUT, events.next())
            .await
            .map_err(|_| "timed out waiting for Codex terminal event".to_owned())?
            .ok_or_else(|| "Codex event stream ended without terminal event".to_owned())?
            .map_err(display)?;
        let terminal = event.is_terminal();
        observed.push(event);
        if terminal {
            require(events.next().await.is_none(), "event after Codex terminal")?;
            return Ok(observed);
        }
    }
}

fn prepare_overlay(destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(display)?;
    let source = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".codex"));
    let auth_source = source.join("auth.json");
    require(auth_source.is_file(), "Codex auth.json is unavailable")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&auth_source, destination.join("auth.json")).map_err(display)?;
    #[cfg(not(unix))]
    fs::copy(&auth_source, destination.join("auth.json")).map_err(display)?;
    fs::write(
        destination.join("config.toml"),
        "model = \"gpt-5.6-luna\"\nmodel_reasoning_effort = \"low\"\nservice_tier = \"default\"\n",
    )
    .map_err(display)
}

fn resolve_codex_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("MEERKAT_CODEX_PATH").map(PathBuf::from) {
        return Ok(path);
    }
    let output = std::process::Command::new("sh")
        .args(["-c", "command -v codex"])
        .output()
        .map_err(display)?;
    require(output.status.success(), "Codex CLI is unavailable")?;
    Ok(PathBuf::from(
        String::from_utf8(output.stdout).map_err(display)?.trim(),
    ))
}

fn verify_pid_cleanup(path: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(path).map_err(display)?;
    let mut states = BTreeMap::<u32, Vec<&str>>::new();
    for line in contents.lines() {
        let mut fields = line.split('\t');
        let state = fields.next().unwrap_or_default();
        let pid = fields
            .next()
            .ok_or_else(|| "PID journal row is incomplete".to_owned())?
            .parse::<u32>()
            .map_err(display)?;
        states.entry(pid).or_default().push(state);
    }
    require(!states.is_empty(), "PID journal is empty")?;
    for (pid, states) in states {
        require(states.contains(&"spawned"), "PID journal missing spawn")?;
        require(states.contains(&"reaped"), "PID journal missing reap")?;
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        require(!alive, "Codex app-server orphan remains after close")?;
    }
    Ok(())
}

fn count_jsonl(root: &Path) -> usize {
    let mut count = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
                count += 1;
            }
        }
    }
    count
}

fn has_native_id(events: &[AgentEvent], native_id: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.kind, AgentEventKind::NativeSessionBound { native_session_id } if native_session_id == native_id)
    })
}

fn final_output(events: &[AgentEvent]) -> Option<&str> {
    events.iter().rev().find_map(|event| match &event.kind {
        AgentEventKind::Completed { final_output } => Some(final_output.as_str()),
        _ => None,
    })
}

fn require_successful_turn(events: &[AgentEvent]) -> Result<(), String> {
    for event in events {
        if let AgentEventKind::Failed { code, message, .. } = &event.kind {
            return Err(format!("Codex turn failed ({code}): {message}"));
        }
    }
    require(
        events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::Completed { .. })),
        "Codex turn did not complete successfully",
    )
}

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    )
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis()
}

fn require(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires installed Codex CLI; starts native sessions without running model turns"]
async fn codex_default_home_is_agent_local() {
    let temp = tempfile::tempdir().unwrap();
    let agent_socket = temp.path().join("agent.sock");
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "codex".into(),
        model: ModelRef::new("gpt-5.6-luna", "Codex"),
        thinking_level: "low".into(),
        cli_path: resolve_codex_path().unwrap(),
        environment: BTreeMap::from([(
            "MEERKAT_AGENT_SOCKET".into(),
            agent_socket.display().to_string(),
        )]),
        config_fingerprint: "isolation".into(),
    };
    let factory = CodexHarness::new();
    let mut ids = Vec::new();
    for name in ["alpha", "beta"] {
        let cwd = temp.path().join(name);
        fs::create_dir_all(&cwd).unwrap();
        let session = timeout(
            Duration::from_secs(30),
            factory.start(SessionRequest {
                cli_version: None,
                permission_policy: Default::default(),
                config: config.clone(),
                working_directory: cwd.clone(),
                system_prompt: String::new(),
                tools: Vec::new(),
                native_session_id: None,
            }),
        )
        .await
        .expect("native Codex startup timeout")
        .expect("native Codex startup");
        ids.push(session.entry().native_session_id);
        session.close().await.unwrap();
        let home = cwd.join(".meerkat/codex");
        assert!(home.is_dir(), "default config must be Agent-local");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&home).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for file in ["auth.json", "config.toml"] {
                if home.join(file).exists() {
                    assert_eq!(
                        fs::metadata(home.join(file)).unwrap().permissions().mode() & 0o077,
                        0
                    );
                }
            }
        }
        assert!(
            fs::read_dir(&home).unwrap().any(|entry| {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                name.ends_with(".sqlite") || name.ends_with(".db") || name == "logs"
            }),
            "native process did not write its state inside Agent-local home"
        );
    }
    assert_ne!(ids[0], ids[1], "Agents must not share native sessions");
}

#[tokio::test]
#[ignore = "requires installed native CLI; reads model directory without a model turn"]
async fn native_model_catalog() {
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "codex".into(),
        cli_path: "codex".into(),
        model: ModelRef::new("", ""),
        thinking_level: String::new(),
        environment: BTreeMap::new(),
        config_fingerprint: String::new(),
    };
    let models = CodexHarness::new()
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
