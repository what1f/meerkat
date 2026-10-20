use lankide_harness::{
    AgentEventKind, ExecuteRequest, HarnessConfig, HarnessFactory, ModelRef, SessionRequest,
};
use lankide_harness_pi::PiHarness;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;
use tokio_stream::StreamExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_turn_keeps_bounded_final_record() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let cli = root.path().join("pi");
    fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/pi_record_peer.py"
        ),
        &cli,
    )?;
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700))?;
    let agent = root.path().join("agent");
    fs::create_dir(&agent)?;
    for name in ["auth.json", "models.json", "settings.json"] {
        fs::write(agent.join(name), "{}")?;
    }
    let record = root.path().join("rpc.jsonl");
    let factory = PiHarness::new();
    let session = factory
        .start(SessionRequest {
            permission_policy: Default::default(),
            config: HarnessConfig {
                launch_arguments: Default::default(),
                harness_id: "pi".into(),
                model: ModelRef::new("local", "Local"),
                thinking_level: "off".into(),
                cli_path: cli,
                config_fingerprint: "record-scenario".into(),
                environment: BTreeMap::from([
                    ("PI_CODING_AGENT_DIR".into(), agent.display().to_string()),
                    (
                        "LANKIDE_PI_ARTIFACT_DIR".into(),
                        root.path().display().to_string(),
                    ),
                    (
                        "LANKIDE_PI_RECORD_PATH".into(),
                        record.display().to_string(),
                    ),
                    ("LANKIDE_PI_RECORD_MAX_BYTES".into(), "2048".into()),
                ]),
            },
            working_directory: root.path().to_owned(),
            system_prompt: "Respond.".into(),
            tools: vec![],
            native_session_id: None,
        })
        .await?;
    let mut stream = session
        .execute(ExecuteRequest {
            input: "Stream then finish.".into(),
        })
        .await?;
    let mut deltas = 0;
    let mut completed = false;
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = stream.next().await {
            match event?.kind {
                AgentEventKind::Started => {}
                AgentEventKind::NativeSessionBound { native_session_id } => {
                    assert_eq!(native_session_id, "record-session");
                }
                AgentEventKind::AssistantDelta { .. } => deltas += 1,
                AgentEventKind::Completed { final_output } => {
                    assert_eq!(final_output, "Final retained message");
                    completed = true;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
        Ok::<_, lankide_harness::HarnessError>(())
    })
    .await??;
    session.close().await?;
    assert_eq!(deltas, 10000);
    assert!(completed);
    let previous = record.with_extension("jsonl.1");
    assert!(previous.exists(), "events must trigger rotation");
    for path in [&record, &previous] {
        assert!(fs::metadata(path)?.len() <= 2048);
        for line in fs::read_to_string(path)?.lines() {
            let event: Value = serde_json::from_str(line)?;
            assert_ne!(event["type"], "message_update");
            assert_ne!(event["type"], "tool_execution_update");
        }
    }
    let current = fs::read_to_string(record)?;
    assert!(current.contains("message_end"));
    assert!(current.contains("agent_end"));
    assert!(current.contains("Final retained message"));
    Ok(())
}
