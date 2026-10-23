const DEFAULT_MODEL: &str = "deepseek-v4-flash";
use lankide_harness::{
    AgentEvent, AgentEventKind, ExecuteRequest, HarnessAvailability, HarnessConfig, HarnessFactory,
    ModelRef, SessionRequest, ToolDefinition, ToolResultRequest,
};
use lankide_harness_claude_code::{ClaudeCodeHarness, HARNESS_ID};
use lankide_testkit::{GateSelection, ProbeConfig, run_prerequisite_probe};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_stream::StreamExt;

const HARD_TIMEOUT: Duration = Duration::from_secs(240);
const EVENT_TIMEOUT: Duration = Duration::from_secs(90);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires dcc, deepseek-v4-flash, and explicit live opt-in"]
async fn claude_code_e2e() {
    assert_eq!(
        std::env::var("LANKIDE_E2E_LIVE").as_deref(),
        Ok("1"),
        "live Claude Code E2E requires LANKIDE_E2E_LIVE=1"
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
    let cli_path = std::env::var_os("LANKIDE_CLAUDE_CODE_PATH")
        .map(PathBuf::from)
        .expect("LANKIDE_CLAUDE_CODE_PATH is required");

    let run_id = std::env::var("LANKIDE_E2E_RUN_ID").unwrap_or_else(|_| unique("claude-code"));
    let root = std::env::temp_dir()
        .join("lankide-e2e-artifacts")
        .join("harness-claude-code")
        .join(&run_id);
    let workspace = root.join("workspace");
    let config_dir = root.join("config");
    let artifact_dir = root.join("artifacts");
    let pid_journal = artifact_dir.join("pids.tsv");
    for path in [&workspace, &config_dir, &artifact_dir] {
        fs::create_dir_all(path).expect("create Claude Code E2E directory");
    }
    let started = std::time::Instant::now();
    let outcome = timeout(
        HARD_TIMEOUT,
        run_scenario(
            &workspace,
            &config_dir,
            &artifact_dir,
            &pid_journal,
            &run_id,
            &cli_path,
        ),
    )
    .await
    .map_err(|_| "hard timeout".to_owned())
    .and_then(|result| result);
    let cleanup = verify_pid_cleanup(&pid_journal);
    let outcome = match (outcome, cleanup) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(format!("{first}; {second}")),
    };
    let report = json!({
        "run_id":run_id,
        "module":"harness-claude-code",
        "classification":"live",
        "harness_id":HARNESS_ID,
        "cli_path":cli_path,
        "model":DEFAULT_MODEL,
        "duration_ms":started.elapsed().as_millis(),
        "scenarios":["probe_start_native_id_output","resume_same_session","mcp_tool_result_and_steer","cancel_close_pid_cleanup"],
        "summary":outcome.as_ref().ok(),
        "status":if outcome.is_ok() { "passed" } else { "failed" },
        "error":outcome.as_ref().err(),
    });
    let report_path = artifact_dir.join("result.json");
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).expect("serialize report"),
    )
    .expect("write report");
    println!("Claude Code E2E artifact: {}", report_path.display());
    if let Err(error) = outcome {
        panic!("Claude Code live E2E failed: {error}");
    }
}

async fn run_scenario(
    workspace: &Path,
    config_dir: &Path,
    artifact_dir: &Path,
    pid_journal: &Path,
    run_id: &str,
    cli_path: &Path,
) -> Result<serde_json::Value, String> {
    let nonce = format!("lankide-claude-{run_id}");
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: HARNESS_ID.to_owned(),
        model: ModelRef::new(DEFAULT_MODEL, "DeepSeek V4 Flash"),
        thinking_level: "low".to_owned(),
        cli_path: cli_path.to_path_buf(),
        environment: BTreeMap::from([
            (
                "LANKIDE_CLAUDE_CONFIG_DIR".to_owned(),
                config_dir.display().to_string(),
            ),
            (
                "LANKIDE_CLAUDE_ARTIFACT_DIR".to_owned(),
                artifact_dir.display().to_string(),
            ),
        ]),
        config_fingerprint: format!("claude-code-live-{run_id}"),
    };
    require(config.harness_id == "claude_code", "unexpected harness id")?;
    require(config.cli_path == cli_path, "unexpected CLI path")?;
    require(
        config.model.model_id == "deepseek-v4-flash",
        "unexpected model",
    )?;
    let factory = ClaudeCodeHarness::new().with_pid_journal(pid_journal);
    let probe = factory.probe(&config).await.map_err(display)?;
    require(
        probe.availability == HarnessAvailability::Available,
        probe.reason.as_deref().unwrap_or("probe unavailable"),
    )?;
    require(
        probe
            .version
            .as_deref()
            .is_some_and(|value| value.contains("Claude Code")),
        "probe omitted Claude Code version",
    )?;

    let base = session_request(&config, workspace, None);
    let session = factory.start(base.clone()).await.map_err(display)?;
    let native_id = session.entry().native_session_id;
    require(!native_id.is_empty(), "native session id is empty")?;
    let initial = collect_terminal(
        session
            .execute(ExecuteRequest {
                input: format!("Use the native Bash tool to run printf to write exactly {nonce} to native-command.txt in the current directory. Then reply with exactly this nonce: {nonce}. Do not read other files or environment variables."),
            })
            .await
            .map_err(display)?,
    )
    .await?;
    require(initial.iter().any(|event| matches!(&event.kind, AgentEventKind::NativeSessionBound { native_session_id } if native_session_id == &native_id)), "native session event missing")?;
    require(
        final_output(&initial).is_some_and(|output| output.contains(&nonce)),
        "initial output omitted nonce",
    )?;
    require(
        fs::read_to_string(workspace.join("native-command.txt"))
            .map_err(|_| "native Claude command did not create output")?
            .trim()
            == nonce,
        "native Claude shell output differs from requested nonce",
    )?;
    session.close().await.map_err(display)?;
    require(
        find_session(config_dir, &native_id),
        "isolated Claude session artifact missing",
    )?;

    let resumed = factory
        .resume(SessionRequest {
            native_session_id: Some(native_id.clone()),
            ..base
        })
        .await
        .map_err(display)?;
    require(
        resumed.entry().native_session_id == native_id,
        "resume changed native session id",
    )?;
    let mut events = resumed.execute(ExecuteRequest {
        input: format!("Call echo_nonce exactly once with nonce {nonce}. Then include its result in the final answer."),
    }).await.map_err(display)?;
    let (call_id, mut observed) = wait_for_tool(&mut events, &nonce).await?;
    let steer = resumed
        .steer(
            format!("steer-{run_id}"),
            format!("Keep the exact marker {nonce} in the final answer."),
        )
        .await
        .map_err(display)?;
    require(steer.accepted, "steer was rejected")?;
    resumed
        .tool_result(ToolResultRequest {
            tool_call_id: call_id.clone(),
            success: true,
            content_markdown: nonce.clone(),
            structured_content: json!({"nonce":nonce}),
            resources: Vec::new(),
            error: None,
        })
        .await
        .map_err(display)?;
    observed.extend(collect_terminal(events).await?);
    require(observed.iter().any(|event| matches!(&event.kind, AgentEventKind::ToolFinished { tool_call_id, success:true, .. } if tool_call_id == &call_id)), "ToolFinished missing")?;
    require(
        final_output(&observed).is_some_and(|output| output.contains(&nonce)),
        "tool turn output omitted nonce",
    )?;
    resumed.close().await.map_err(display)?;

    // Cancellation is a separate real session. The steer above is delivered
    // through Claude's streaming input and may be persisted as a queued
    // follow-up after the Tool turn's terminal result; reusing that process
    // would let the queued completion race the cancellation turn.
    let cancellation = factory
        .start(session_request(&config, workspace, None))
        .await
        .map_err(display)?;
    let mut cancel_events = cancellation
        .execute(ExecuteRequest {
            input: format!("Call echo_nonce with nonce cancel-{nonce} and wait for its result."),
        })
        .await
        .map_err(display)?;
    let (_, prefix) = wait_for_tool(&mut cancel_events, &format!("cancel-{nonce}")).await?;
    require(!prefix.is_empty(), "cancel turn emitted no events")?;
    let cancel = cancellation
        .cancel("live E2E cancellation".to_owned())
        .await
        .map_err(display)?;
    require(cancel.accepted, "cancel was rejected")?;
    let cancelled = collect_terminal(cancel_events).await?;
    require(
        cancelled
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::Cancelled { .. })),
        "Cancelled terminal missing",
    )?;
    cancellation.close().await.map_err(display)?;
    Ok(
        json!({"native_session_id_present":true,"nonce_verified":true,"resume_verified":true,"tool_result_verified":true,"steer_verified":true,"cancel_verified":true}),
    )
}

fn session_request(
    config: &HarnessConfig,
    workspace: &Path,
    native_session_id: Option<String>,
) -> SessionRequest {
    SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config:config.clone(), working_directory:workspace.to_path_buf(),
        system_prompt:"Protocol test only. Only perform the explicitly requested local file operation. Do not inspect unrelated files, environment, memory, or external services. Only call a Tool when the current user message explicitly asks for that Tool; otherwise answer with text only.".to_owned(),
        tools:vec![ToolDefinition {
            name:"echo_nonce".to_owned(), description:"Return the supplied test nonce unchanged.".to_owned(),
            input_schema:json!({"type":"object","properties":{"nonce":{"type":"string"}},"required":["nonce"],"additionalProperties":false}),
            output_schema:json!({"type":"object","properties":{"nonce":{"type":"string"}},"required":["nonce"]}),
        }], native_session_id,
    }
}

async fn wait_for_tool(
    stream: &mut lankide_harness::AgentEventStream,
    nonce: &str,
) -> Result<(String, Vec<AgentEvent>), String> {
    let mut events = Vec::new();
    loop {
        let event = timeout(EVENT_TIMEOUT, stream.next())
            .await
            .map_err(|_| "tool wait timed out".to_owned())?
            .ok_or_else(|| "stream ended before tool call".to_owned())?
            .map_err(display)?;
        if let AgentEventKind::ToolStarted {
            tool_call_id,
            tool_name,
            arguments,
        } = &event.kind
        {
            require(tool_name == "echo_nonce", "unexpected tool name")?;
            require(
                arguments.get("nonce").and_then(|value| value.as_str()) == Some(nonce),
                "unexpected tool nonce",
            )?;
            let id = tool_call_id.clone();
            events.push(event);
            return Ok((id, events));
        }
        if event.is_terminal() {
            return Err(format!("terminal before tool call: {event:?}"));
        }
        events.push(event);
    }
}

async fn collect_terminal(
    mut stream: lankide_harness::AgentEventStream,
) -> Result<Vec<AgentEvent>, String> {
    let mut events = Vec::new();
    loop {
        let event = timeout(EVENT_TIMEOUT, stream.next())
            .await
            .map_err(|_| "terminal wait timed out".to_owned())?
            .ok_or_else(|| "stream ended before terminal".to_owned())?
            .map_err(display)?;
        let terminal = event.is_terminal();
        events.push(event);
        if terminal {
            return Ok(events);
        }
    }
}

fn final_output(events: &[AgentEvent]) -> Option<&str> {
    events.iter().rev().find_map(|event| {
        if let AgentEventKind::Completed { final_output } = &event.kind {
            Some(final_output.as_str())
        } else {
            None
        }
    })
}

fn find_session(root: &Path, native_id: &str) -> bool {
    let target = format!("{native_id}.jsonl");
    let mut paths = vec![root.to_path_buf()];
    while let Some(path) = paths.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                paths.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some(&target) {
                return true;
            }
        }
    }
    false
}

fn verify_pid_cleanup(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(display)?;
    let mut spawned = BTreeSet::new();
    let mut reaped = BTreeSet::new();
    for line in content.lines() {
        let mut fields = line.split('\t');
        let state = fields.next().unwrap_or_default();
        let pid = fields.next().unwrap_or_default();
        match state {
            "spawned" => {
                spawned.insert(pid);
            }
            "reaped" => {
                reaped.insert(pid);
            }
            _ => {}
        }
    }
    require(!spawned.is_empty(), "PID journal recorded no children")?;
    require(
        spawned == reaped,
        format!("unreaped Claude children: spawned={spawned:?} reaped={reaped:?}"),
    )
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

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

#[tokio::test]
#[ignore = "requires installed native CLI; reads model directory without a model turn"]
async fn native_model_catalog() {
    let config = HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "claude_code".into(),
        cli_path: "claude".into(),
        model: ModelRef::new("", ""),
        thinking_level: String::new(),
        environment: BTreeMap::new(),
        config_fingerprint: String::new(),
    };
    let models = ClaudeCodeHarness::new()
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
