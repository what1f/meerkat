#[path = "support/process.rs"]
mod process_support;
use process_support::{ProcessHarnessFactory, ProcessHarnessSpec};

use lankide_harness::{
    AgentEvent, AgentEventKind, ExecuteRequest, HarnessAvailability, HarnessConfig,
    HarnessErrorCode, HarnessFactory, HarnessInfo, ModelRef, SessionHealth, SessionRequest,
    ToolDefinition, ToolResultError, ToolResultRequest, ToolResultResource,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_stream::StreamExt;

const STEP_TIMEOUT: Duration = Duration::from_secs(3);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn harness_e2e() {
    let run_id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    );
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let started = std::time::Instant::now();
    let work_root = std::env::temp_dir().join(format!("lankide-harness-{run_id}"));
    let artifact_root = std::env::temp_dir()
        .join("lankide-e2e-artifacts")
        .join("harness")
        .join(&run_id);
    fs::create_dir_all(&work_root).expect("create isolated work root");
    fs::create_dir_all(&artifact_root).expect("create artifact root");
    let pid_journal = artifact_root.join("pids.tsv");

    let outcome = timeout(
        Duration::from_secs(20),
        run_scenarios(&work_root, &pid_journal),
    )
    .await
    .map_err(|_| "hard timeout".to_owned())
    .and_then(|result| result)
    .and_then(|_| verify_pid_journal(&pid_journal));

    let cleanup = fs::remove_dir_all(&work_root)
        .map(|_| "passed".to_owned())
        .unwrap_or_else(|error| format!("failed: {error}"));
    let result = json!({
        "run_id": run_id,
        "module": "harness",
        "classification": "hermetic",
        "started_at_unix_ms": started_at,
        "duration_ms": started.elapsed().as_millis(),
        "prerequisite_check": {"os_child_process": true},
        "scenarios": [
            "probe_start_execute_resume",
            "tool_result_roundtrip_idempotency",
            "steer_cancel_close",
            "bind_exit",
            "invalid_event",
            "stream_cut",
            "forced_cancel_reap"
        ],
        "cleanup": cleanup,
        "status": if outcome.is_ok() && cleanup == "passed" { "passed" } else { "failed" },
        "error": outcome.as_ref().err(),
    });
    let result_path = artifact_root.join("result.json");
    fs::write(
        &result_path,
        serde_json::to_vec_pretty(&result).expect("serialize result artifact"),
    )
    .expect("write result artifact");
    println!("harness e2e artifact: {}", result_path.display());
    assert_eq!(cleanup, "passed", "cleanup must succeed");
    if let Err(error) = outcome {
        panic!("harness conformance failed: {error}");
    }
}

async fn run_scenarios(work_root: &Path, pid_journal: &Path) -> Result<(), String> {
    require(
        HarnessInfo::new("dcc", "DCC").is_err(),
        "dcc must not become a harness ID",
    )?;
    let mut invalid_dcc = config("none");
    invalid_dcc.harness_id = "dcc".to_owned();
    require(
        invalid_dcc.validate().is_err(),
        "dcc HarnessConfig must be rejected",
    )?;
    let factory = factory(pid_journal)?;
    let config = config("none");
    let probe = timeout(STEP_TIMEOUT, factory.probe(&config))
        .await
        .map_err(|_| "probe timeout".to_owned())?
        .map_err(display)?;
    require(
        probe.availability == HarnessAvailability::Available,
        "probe unavailable",
    )?;
    require(
        probe.version.as_deref() == Some("conformance-1.0.0"),
        "probe version",
    )?;
    require(probe.capabilities.len() == 6, "probe capabilities")?;

    let session = timeout(
        STEP_TIMEOUT,
        factory.start(session_request(&config, work_root, None)),
    )
    .await
    .map_err(|_| "start timeout".to_owned())?
    .map_err(display)?;
    let native_id = session.entry().native_session_id;
    require(!native_id.is_empty(), "native session ID missing")?;
    let first = collect(
        session
            .execute(ExecuteRequest {
                input: "first".to_owned(),
            })
            .await
            .map_err(display)?,
    )
    .await?;
    assert_execution(&first, &native_id, "first")?;
    session.close().await.map_err(display)?;
    require(
        session.entry().health == SessionHealth::Closed,
        "close health",
    )?;

    let resumed = factory
        .resume(session_request(&config, work_root, Some(native_id.clone())))
        .await
        .map_err(display)?;
    require(
        resumed.entry().native_session_id == native_id,
        "resume ID changed",
    )?;
    let second = collect(
        resumed
            .execute(ExecuteRequest {
                input: "second".to_owned(),
            })
            .await
            .map_err(display)?,
    )
    .await?;
    assert_execution(&second, &native_id, "second")?;
    tool_result_scenario(resumed.as_ref(), work_root).await?;
    resumed.close().await.map_err(display)?;
    require(
        resumed
            .tool_result(tool_result_request(work_root))
            .await
            .is_err(),
        "tool result accepted after close",
    )?;

    cancel_scenario("none", work_root, pid_journal).await?;
    cancel_scenario("ignore_cancel", work_root, pid_journal).await?;
    failure_scenarios(work_root, pid_journal).await?;
    Ok(())
}

async fn cancel_scenario(mode: &str, work_root: &Path, pid_journal: &Path) -> Result<(), String> {
    let factory = factory(pid_journal)?;
    let config = config(mode);
    let session = factory
        .start(session_request(&config, work_root, None))
        .await
        .map_err(display)?;
    let mut events = session
        .execute(ExecuteRequest {
            input: "long".to_owned(),
        })
        .await
        .map_err(display)?;
    for _ in 0..3 {
        timeout(STEP_TIMEOUT, events.next())
            .await
            .map_err(|_| "long execution prefix timeout".to_owned())?
            .ok_or_else(|| "long execution ended early".to_owned())?
            .map_err(display)?;
    }
    let steer = session
        .steer("delivery-1".to_owned(), "urgent".to_owned())
        .await
        .map_err(display)?;
    require(steer.accepted, "steer rejected")?;
    let steer_event = timeout(STEP_TIMEOUT, events.next())
        .await
        .map_err(|_| "steer event timeout".to_owned())?
        .ok_or_else(|| "missing steer event".to_owned())?
        .map_err(display)?;
    require(
        matches!(steer_event.kind, AgentEventKind::AssistantDelta { ref text } if text == "steer:urgent"),
        "steer delta missing",
    )?;
    require(
        session
            .cancel("requested".to_owned())
            .await
            .map_err(display)?
            .accepted,
        "cancel rejected",
    )?;
    let terminal = timeout(STEP_TIMEOUT, events.next())
        .await
        .map_err(|_| "cancel terminal timeout".to_owned())?
        .ok_or_else(|| "missing cancel terminal".to_owned())?
        .map_err(display)?;
    require(
        matches!(terminal.kind, AgentEventKind::Cancelled { .. }),
        "not cancelled",
    )?;
    require(events.next().await.is_none(), "multiple terminal events")?;
    require(
        session
            .tool_result(tool_result_request(work_root))
            .await
            .is_err(),
        "tool result accepted after cancel",
    )?;
    session.close().await.map_err(display)?;
    require(
        session.entry().health == SessionHealth::Closed,
        "cancel close health",
    )
}

async fn tool_result_scenario(
    session: &dyn lankide_harness::HarnessSession,
    work_root: &Path,
) -> Result<(), String> {
    let mut events = session
        .execute(ExecuteRequest {
            input: "tool_roundtrip".to_owned(),
        })
        .await
        .map_err(display)?;
    let mut prefix = Vec::new();
    for _ in 0..4 {
        prefix.push(
            timeout(STEP_TIMEOUT, events.next())
                .await
                .map_err(|_| "tool call prefix timeout".to_owned())?
                .ok_or_else(|| "tool call stream ended early".to_owned())?
                .map_err(display)?,
        );
    }
    require(
        matches!(&prefix[3].kind, AgentEventKind::ToolStarted { tool_call_id, .. } if tool_call_id == "call-1"),
        "tool call was not observed",
    )?;

    let mut unknown = tool_result_request(work_root);
    unknown.tool_call_id = "unknown-call".to_owned();
    require(
        session
            .tool_result(unknown)
            .await
            .is_err_and(|error| error.code == HarnessErrorCode::InvalidState),
        "unobserved tool call result was accepted",
    )?;

    let request = tool_result_request(work_root);
    let first = session
        .tool_result(request.clone())
        .await
        .map_err(display)?;
    require(!first.already_applied, "first tool result marked duplicate")?;
    let duplicate = session
        .tool_result(request.clone())
        .await
        .map_err(display)?;
    require(
        duplicate.already_applied,
        "duplicate tool result was rewritten",
    )?;
    let mut conflict = request.clone();
    conflict.content_markdown = "conflicting content".to_owned();
    require(
        session
            .tool_result(conflict)
            .await
            .is_err_and(|error| error.code == HarnessErrorCode::InvalidState),
        "conflicting tool result was accepted",
    )?;

    session
        .steer("finish-tool".to_owned(), "finish-tool".to_owned())
        .await
        .map_err(display)?;
    let finished = timeout(STEP_TIMEOUT, events.next())
        .await
        .map_err(|_| "tool finished timeout".to_owned())?
        .ok_or_else(|| "tool finished missing".to_owned())?
        .map_err(display)?;
    require(finished.sequence == 5, "tool finished sequence")?;
    let AgentEventKind::ToolFinished { output, .. } = finished.kind else {
        return Err("child did not emit ToolFinished".to_owned());
    };
    require(
        output["received"] == serde_json::to_value(&request).map_err(display)?,
        "child did not receive complete tool result payload",
    )?;
    require(
        output["peer_write_count"] == 1,
        "duplicate tool result reached child",
    )?;
    let terminal = timeout(STEP_TIMEOUT, events.next())
        .await
        .map_err(|_| "tool terminal timeout".to_owned())?
        .ok_or_else(|| "tool terminal missing".to_owned())?
        .map_err(display)?;
    require(
        matches!(terminal.kind, AgentEventKind::Completed { ref final_output } if final_output == "tool-result-applied"),
        "tool execution did not complete",
    )?;
    require(events.next().await.is_none(), "tool event after terminal")?;
    require(
        session
            .tool_result(request)
            .await
            .is_err_and(|error| error.code == HarnessErrorCode::InvalidState),
        "tool result accepted after terminal",
    )
}

fn tool_result_request(work_root: &Path) -> ToolResultRequest {
    ToolResultRequest {
        tool_call_id: "call-1".to_owned(),
        success: false,
        content_markdown: "tool could not finish".to_owned(),
        structured_content: json!({"attempted": true, "items": [1, 2]}),
        resources: vec![ToolResultResource {
            local_path: work_root.join("tool-output.txt"),
            file_name: "tool-output.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            size_bytes: 17,
            sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        }],
        error: Some(ToolResultError {
            code: "fixture_error".to_owned(),
            message: "fixture failure".to_owned(),
            retryable: false,
        }),
    }
}

async fn failure_scenarios(work_root: &Path, pid_journal: &Path) -> Result<(), String> {
    let bind_factory = factory(pid_journal)?;
    let bind_error = match bind_factory
        .start(session_request(&config("bind_exit"), work_root, None))
        .await
    {
        Ok(_) => return Err("bind-exit process unexpectedly started".to_owned()),
        Err(error) => error,
    };
    require(
        bind_error.code == HarnessErrorCode::BindFailed,
        "bind-exit code",
    )?;

    for (mode, expected) in [
        ("invalid_event", HarnessErrorCode::InvalidEvent),
        ("stream_cut", HarnessErrorCode::StreamInterrupted),
    ] {
        let factory = factory(pid_journal)?;
        let session = factory
            .start(session_request(&config(mode), work_root, None))
            .await
            .map_err(display)?;
        let mut events = session
            .execute(ExecuteRequest {
                input: "fault".to_owned(),
            })
            .await
            .map_err(display)?;
        let observed = loop {
            let item = timeout(STEP_TIMEOUT, events.next())
                .await
                .map_err(|_| format!("{mode} timeout"))?
                .ok_or_else(|| format!("{mode} stream ended without error"))?;
            if let Err(error) = item {
                break error;
            }
        };
        require(observed.code == expected, &format!("{mode} error code"))?;
        session.close().await.map_err(display)?;
    }
    Ok(())
}

fn factory(pid_journal: &Path) -> Result<Box<dyn HarnessFactory>, String> {
    Ok(Box::new(ProcessHarnessFactory::new(
        HarnessInfo::new("conformance", "Conformance Harness").map_err(display)?,
        ProcessHarnessSpec {
            arguments: Vec::new(),
            bind_timeout: STEP_TIMEOUT,
            cancel_grace: Duration::from_millis(80),
            close_grace: Duration::from_millis(80),
            pid_journal: Some(pid_journal.to_owned()),
        },
    )))
}

fn config(mode: &str) -> HarnessConfig {
    HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: "conformance".to_owned(),
        model: ModelRef::new("fixture-model", "Fixture Model"),
        thinking_level: "low".to_owned(),
        cli_path: PathBuf::from(env!("CARGO_BIN_EXE_harness-peer")),
        environment: BTreeMap::from([("HARNESS_PEER_FAULT".to_owned(), mode.to_owned())]),
        config_fingerprint: format!("fingerprint-{mode}"),
    }
}

fn session_request(
    config: &HarnessConfig,
    work_root: &Path,
    native_session_id: Option<String>,
) -> SessionRequest {
    SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config: config.clone(),
        working_directory: work_root.to_owned(),
        system_prompt: "conformance system prompt".to_owned(),
        tools: vec![ToolDefinition {
            name: "echo".to_owned(),
            description: "Echo input".to_owned(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
        }],
        native_session_id,
    }
}

async fn collect(mut stream: lankide_harness::AgentEventStream) -> Result<Vec<AgentEvent>, String> {
    let mut events = Vec::new();
    loop {
        let next = timeout(STEP_TIMEOUT, stream.next())
            .await
            .map_err(|_| "event stream timeout".to_owned())?;
        let Some(event) = next else { break };
        let event = event.map_err(display)?;
        let terminal = event.is_terminal();
        events.push(event);
        if terminal {
            require(stream.next().await.is_none(), "event after terminal")?;
            break;
        }
    }
    Ok(events)
}

fn assert_execution(events: &[AgentEvent], native_id: &str, input: &str) -> Result<(), String> {
    require(events.len() == 6, "unexpected event count")?;
    require(
        events
            .iter()
            .enumerate()
            .all(|(index, event)| event.sequence == index as u64 + 1),
        "event sequence is not monotonic",
    )?;
    require(
        matches!(&events[1].kind, AgentEventKind::NativeSessionBound { native_session_id } if native_session_id == native_id),
        "native session event missing",
    )?;
    require(
        events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolStarted { .. })),
        "tool event missing",
    )?;
    require(
        matches!(&events[5].kind, AgentEventKind::Completed { final_output } if final_output == &format!("final:{input}")),
        "final output missing",
    )
}

fn verify_pid_journal(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path).map_err(display)?;
    let mut balances = BTreeMap::<String, (u32, u32)>::new();
    for line in content.lines() {
        let (action, pid) = line
            .split_once('\t')
            .ok_or_else(|| format!("invalid PID journal line: {line}"))?;
        let counts = balances.entry(pid.to_owned()).or_default();
        match action {
            "spawned" => counts.0 += 1,
            "reaped" => counts.1 += 1,
            "forced" => {}
            _ => return Err(format!("unknown PID journal action: {action}")),
        }
    }
    require(!balances.is_empty(), "PID journal is empty")?;
    require(
        balances.values().all(|(spawned, reaped)| spawned == reaped),
        &format!("orphaned or multiply reaped child in {balances:?}"),
    )?;
    require(
        content.lines().any(|line| line.starts_with("forced\t")),
        "forced cancel was not observed",
    )?;
    for pid in balances.keys() {
        let alive = std::process::Command::new("kill")
            .args(["-0", pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        require(
            !alive,
            &format!("child PID {pid} still exists after cleanup"),
        )?;
    }
    Ok(())
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    condition.then_some(()).ok_or_else(|| message.to_owned())
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tokio::test]
async fn javascript_plugin_defaults_resume_rotation_and_explicit_test_plan() {
    use lankide_harness::JsHarnessFactory;
    let factory = JsHarnessFactory::new("custom", "Custom CLI", r#"
        defineHarness({apiVersion:1,id:'custom',defaults:{environment:{WINDOW:'80'},launch_arguments:['--window','80']},
          start(){return {native_session_id:'original'};},
          resume(request){if(request.native_session_id!=='original')failure('invalid_state','Missing prior binding');return {native_session_id:'replacement'};},
          execute(){ctx.bindSession('late-binding');ctx.emit({kind:'completed',final_output:'OK'});},
          close(){},test(request){request.config.model.model_id='fast';return {request,input:'Probe'};}
        });
    "#).unwrap();
    assert_eq!(factory.defaults().environment["WINDOW"], "80");
    let directory =
        std::env::temp_dir().join(format!("lankide-plugin-contract-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let request = SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config: HarnessConfig {
            harness_id: "custom".into(),
            cli_path: "/bin/true".into(),
            environment: BTreeMap::new(),
            launch_arguments: Vec::new(),
            model: ModelRef::new("selected", "Selected"),
            thinking_level: String::new(),
            config_fingerprint: "binding".into(),
        },
        working_directory: directory.clone(),
        system_prompt: "Instructions".into(),
        tools: Vec::new(),
        native_session_id: None,
    };
    let plan = factory.test(request.clone()).await.unwrap();
    assert_eq!(plan.request.config.model.model_id, "fast");
    assert_eq!(plan.input, "Probe");
    let first = factory.start(request.clone()).await.unwrap();
    assert_eq!(first.entry().native_session_id, "original");
    first.close().await.unwrap();
    let resumed = factory
        .resume(SessionRequest {
            native_session_id: Some("original".into()),
            ..request
        })
        .await
        .unwrap();
    assert_eq!(resumed.entry().native_session_id, "replacement");
    let mut events = resumed
        .execute(ExecuteRequest {
            input: "Continue".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        events.next().await.unwrap().unwrap().kind,
        AgentEventKind::Started
    ));
    assert!(
        matches!(events.next().await.unwrap().unwrap().kind,AgentEventKind::NativeSessionBound{native_session_id} if native_session_id=="replacement")
    );
    assert!(
        matches!(events.next().await.unwrap().unwrap().kind,AgentEventKind::NativeSessionBound{native_session_id} if native_session_id=="late-binding")
    );
    assert!(matches!(
        events.next().await.unwrap().unwrap().kind,
        AgentEventKind::Completed { .. }
    ));
    assert_eq!(resumed.entry().native_session_id, "late-binding");
    resumed.close().await.unwrap();
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn javascript_plugin_deadline_interrupts_a_stuck_hook_and_rejects_global_writes() {
    use lankide_harness::JsHarnessFactory;
    let directory =
        std::env::temp_dir().join(format!("lankide-plugin-guard-{}", std::process::id()));
    fs::create_dir_all(directory.join("agent")).unwrap();
    fs::create_dir_all(directory.join("user")).unwrap();
    fs::write(directory.join("user/config"), "keep").unwrap();
    let request = SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config: HarnessConfig {
            harness_id: "guard".into(),
            cli_path: "/bin/true".into(),
            model: ModelRef::new("m", "m"),
            thinking_level: String::new(),
            environment: BTreeMap::new(),
            launch_arguments: vec![],
            config_fingerprint: "guard".into(),
        },
        working_directory: directory.join("agent"),
        system_prompt: String::new(),
        tools: Vec::new(),
        native_session_id: None,
    };
    let stuck = JsHarnessFactory::new(
        "guard",
        "Guard",
        "defineHarness({apiVersion:1,id:'guard',start(){while(true){}}})",
    )
    .unwrap();
    let started = std::time::Instant::now();
    let result = stuck
        .start_before(request.clone(), started + Duration::from_millis(60))
        .await;
    assert!(matches!(result,Err(error) if error.code==HarnessErrorCode::Timeout));
    assert!(started.elapsed() < Duration::from_secs(2));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(directory.join("user"), directory.join("agent/escape")).unwrap();
        let guarded=JsHarnessFactory::new("guard","Guard","defineHarness({apiVersion:1,id:'guard',start(r){ctx.write(r.working_directory+'/escape/config','changed');return {native_session_id:'x'};}})").unwrap();
        assert!(guarded.start(request).await.is_err());
        assert_eq!(
            fs::read_to_string(directory.join("user/config")).unwrap(),
            "keep"
        );
    }
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn changing_the_native_config_source_refreshes_inherited_resources() {
    use lankide_harness::JsHarnessFactory;
    let directory =
        std::env::temp_dir().join(format!("lankide-plugin-resources-{}", std::process::id()));
    let agent = directory.join("agent");
    fs::create_dir_all(&agent).unwrap();
    for source in ["first", "second"] {
        fs::create_dir_all(directory.join(source).join("skills")).unwrap();
        fs::write(directory.join(source).join("skills/marker"), source).unwrap();
    }
    let plugin=JsHarnessFactory::new("resources","Resources",r#"defineHarness({apiVersion:1,id:'resources',start(r){ctx.prepare({directory:r.working_directory+'/private',sourceVariable:'PLUGIN_BASE',sourceRelative:'unused',files:[],resources:['skills']});return {native_session_id:'id'};},close(){}})"#).unwrap();
    for source in ["first", "second"] {
        let session = plugin
            .start(SessionRequest {
                cli_version: None,
                permission_policy: Default::default(),
                config: HarnessConfig {
                    harness_id: "resources".into(),
                    cli_path: "/bin/true".into(),
                    model: ModelRef::new("m", "m"),
                    thinking_level: String::new(),
                    environment: BTreeMap::from([(
                        "PLUGIN_BASE".into(),
                        directory.join(source).display().to_string(),
                    )]),
                    launch_arguments: vec![],
                    config_fingerprint: source.into(),
                },
                working_directory: agent.clone(),
                system_prompt: String::new(),
                tools: vec![],
                native_session_id: None,
            })
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(agent.join("private/skills/marker")).unwrap(),
            source
        );
        session.close().await.unwrap();
    }
    assert_eq!(
        fs::read_to_string(directory.join("first/skills/marker")).unwrap(),
        "first"
    );
    fs::remove_dir_all(directory).unwrap();
}
