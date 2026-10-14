use lankide_client::{AgentDraft, ClientError, ManagementClient};
use lankide_protocol::{
    link::{self, ComputerLink},
    *,
};
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};

use std::time::Duration;
use tokio::time::timeout;

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        attempt_lease: Duration::from_secs(15),
        ..Default::default()
    }
}

#[tokio::test]
async fn s13_1_placement_requires_registered_non_revoked_computer_and_reported_harness()
-> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let computer = rig
        .management()
        .get_computer_catalog(None)
        .await?
        .remove(0)
        .id;
    let draft = AgentDraft {
        id: "bruce".into(),
        name: "Bruce".into(),
        computer_id: "unknown".into(),
        harness_id: "pi".into(),
        ..Default::default()
    };
    assert!(matches!(
        rig.management().create_agent("team", draft.clone()).await,
        Err(ClientError::ComputerNotRegistered)
    ));
    let mut unavailable = draft;
    unavailable.computer_id = computer.clone();
    unavailable.harness_id = "absent".into();
    assert!(matches!(
        rig.management().create_agent("team", unavailable).await,
        Err(ClientError::HarnessUnavailableOnComputer)
    ));
    let agent = team
        .agent("bruce")
        .harness(Script::reply("hi"))
        .create()
        .await?;
    let original = agent.view().await?;
    let mut changed = original.clone();
    changed.computer_id = "unknown".into();
    assert!(matches!(
        rig.management().update_agent(changed, None).await,
        Err(ClientError::ComputerNotRegistered)
    ));
    let mut changed = original.clone();
    changed.harness_id = "absent".into();
    assert!(matches!(
        rig.management().update_agent(changed, None).await,
        Err(ClientError::HarnessUnavailableOnComputer)
    ));
    assert_eq!(agent.view().await?.computer_id, original.computer_id);
    rig.management().revoke_computer(&computer).await?;
    let revoked = AgentDraft {
        id: "clark".into(),
        name: "Clark".into(),
        computer_id: computer,
        harness_id: "pi".into(),
        ..Default::default()
    };
    assert!(matches!(
        rig.management().create_agent("team", revoked).await,
        Err(ClientError::ComputerRevoked)
    ));
    assert!(matches!(
        rig.management().update_agent(original, None).await,
        Err(ClientError::ComputerRevoked)
    ));
    assert!(
        rig.await_terminal_computer()
            .await?
            .contains("computer_revoked")
    );
    rig.close().await
}

#[tokio::test]
async fn s13_2_each_agent_runs_only_on_its_own_computer() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let first = rig
        .management()
        .get_computer_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_computer().await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .computer(&first)
        .harness(Script::reply("first"))
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .computer(&second.id)
        .harness(Script::reply("second"))
        .create()
        .await?;
    team.direct_message(&bruce, "alice", "one").await?;
    team.direct_message(&clark, "alice", "two").await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&bruce, "alice"), ["one", "first"]);
    assert_eq!(team.dm_transcript(&clark, "alice"), ["two", "second"]);
    for (home, present, absent) in [
        (rig.computer_home(), "bruce", "clark"),
        (second.home.path(), "clark", "bruce"),
    ] {
        let root = home.join("workspaces");
        assert!(lankide_computer::agent_directory(&root, "team", present)?.exists());
        assert!(!lankide_computer::agent_directory(&root, "team", absent)?.exists());
    }
    drop(second);
    rig.close().await
}

#[tokio::test]
async fn s13_3_offline_message_stays_queued_and_runs_after_reconnect() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    team.agent("bruce")
        .harness(Script::reply("back"))
        .create()
        .await?;
    rig.stop_computer()?;
    timeout(Duration::from_secs(5), async {
        while rig.computer_online().await? {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    team.direct_message(&agent, "alice", "waiting").await?;
    timeout(Duration::from_secs(5), async {
        while agent.inbox_count()? == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    assert_eq!(agent.inbox_states()?, ["pending"]);
    assert!(agent.executions().await?.is_empty());
    assert!(team.outbound(&agent, "alice").is_empty());
    rig.start_paired_computer()?;
    rig.wait_computer_online().await?;
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["waiting", "back"]);
    assert_eq!(agent.executions().await?.len(), 1);
    assert!(agent.view().await?.pause_reason.is_none());
    rig.close().await
}

#[tokio::test]
async fn s13_4_revoked_computer_cannot_deliver_a_late_result() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        network_proxy: true,
        ..config()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::BufferedReply {
            text: "late".into(),
        })
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_for(|runs, _| !runs.is_empty()).await?;
    timeout(Duration::from_secs(5), async {
        while agent.harness_inputs()?.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    let computer = agent.view().await?.computer_id;
    rig.proxy.as_ref().unwrap().pause();
    agent.release_run().await?;
    // The finished result is durable on the Computer but has not reached Runtime.
    timeout(Duration::from_secs(5), async {
        loop {
            let state = rig.computer_state()?;
            if state["pending_events"]
                .as_object()
                .unwrap()
                .values()
                .any(|events| {
                    events.as_object().unwrap().values().any(|e| {
                        e["kind"] == lankide_protocol::ExecutionEventKind::Completed as i32
                    })
                })
            {
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    rig.management().revoke_computer(&computer).await?;
    rig.proxy.as_ref().unwrap().restore();
    // Wait for the observable terminal link rejection and lease outcome.
    assert!(
        rig.await_terminal_computer()
            .await?
            .contains("computer_revoked")
    );
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    agent
        .wait_for(|runs, _| runs.iter().all(|r| r.state == "failed"))
        .await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello"]);
    assert!(agent.executions().await?.iter().all(|r| r.output.is_none()));
    rig.close().await
}

#[path = "support/cutter.rs"]
mod cutter;

#[tokio::test]
async fn placement_changes_invalidate_tokens_even_after_moving_back() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        attempt_lease: Duration::from_secs(30),
        ..config()
    })
    .await?;
    let (url, _) = rig.computer_endpoint()?;
    let cutter = cutter::Cutter::start(&url, None, false).await?;
    rig.route_computer_through(&cutter.url).await?;
    let first = rig
        .management()
        .get_computer_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_computer().await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .computer(&first)
        .harness(Script::Hang)
        .create()
        .await?;
    let resource = team
        .image_message(&agent, "images", vec![1, 2, 3, 4])
        .await?;
    agent.wait_until_running().await?;
    let auth = cutter.downloads().remove(0);
    assert!(auth.range.is_none());
    let context = lankide_protocol::AgentRunContext {
        execution: Some(lankide_protocol::ExecutionRef {
            execution_id: auth.execution.parse()?,
            attempt: auth.attempt.parse()?,
        }),
        execution_token: auth.token.clone(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    let download = || {
        client
            .get(format!(
                "{}/resources/{}",
                url.replacen("ws://", "http://", 1),
                resource.resource_id
            ))
            .bearer_auth(&auth.token)
            .header("x-lankide-execution", &auth.execution)
            .header("x-lankide-attempt", &auth.attempt)
    };
    // Replay through an authenticated Computer link, without heartbeat-driven
    // cancellation masking the placement authorization being checked here.
    rig.stop_computer()?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    let mut link = capability_link(&url, rig.computer_home()).await?;
    let capability = || lankide_protocol::CapabilityInvokeRequest {
        context: Some(context.clone()),
        capability_name: "unread_current".into(),
        arguments_json: "{}".into(),
        invocation_id: "placement-read".into(),
    };
    assert_eq!(download().send().await?.status(), 200);
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(response.error_code, 0, "{response:?}");
    assert!(
        lankide_protocol::agent::decode::<lankide_protocol::CapabilityInvokeResult>(
            response.payload.as_slice()
        )?
        .success
    );
    // A valid token must also be rejected on another registered Computer.
    let second_id = second.id.clone();
    let second_home = tempfile::tempdir()?;
    std::fs::copy(
        second.home.path().join("computer-credentials.json"),
        second_home.path().join("computer-credentials.json"),
    )?;
    drop(second);
    let mut other_link = capability_link(&url, second_home.path()).await?;
    let response = invoke(&mut other_link, capability()).await?;
    assert_eq!(
        response.error_code,
        lankide_protocol::Code::PermissionDenied as i32,
        "{response:?}"
    );
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(response.error_code, 0, "{response:?}");
    assert!(
        lankide_protocol::agent::decode::<lankide_protocol::CapabilityInvokeResult>(
            response.payload.as_slice()
        )?
        .success
    );
    let original = agent.view().await?;
    let database = rusqlite::Connection::open(rig.runtime_home().join("runtime.db"))?;
    let generation = || {
        database.query_row("SELECT placement_generation FROM agents WHERE workspace_id='team' AND agent_id='bruce'", [], |r| r.get::<_, i64>(0))
    };
    let initial = generation()?;
    let mut edited = original.clone();
    edited.model = "another-model".into();
    rig.management().update_agent(edited, None).await?;
    assert_eq!(generation()?, initial);
    assert_eq!(download().send().await?.status(), 200);
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(response.error_code, 0, "{response:?}");
    assert!(
        lankide_protocol::agent::decode::<lankide_protocol::CapabilityInvokeResult>(
            response.payload.as_slice()
        )?
        .success
    );
    for (offset, computer) in [&second_id, &original.computer_id].into_iter().enumerate() {
        let mut moved = agent.view().await?;
        moved.computer_id = computer.clone();
        rig.management().update_agent(moved, None).await?;
        assert_eq!(generation()?, initial + offset as i64 + 1);
        assert_eq!(download().send().await?.status(), 403);
        let response = invoke(&mut link, capability()).await?;
        assert_eq!(
            response.error_code,
            lankide_protocol::Code::PermissionDenied as i32,
            "{response:?}"
        );
    }
    // The generation and invalid token survive a Runtime restart.
    drop(database);
    drop(other_link);
    drop(link);
    rig.restart_runtime().await?;
    rig.stop_computer()?;
    let (restarted_url, _) = rig.computer_endpoint()?;
    assert_eq!(
        client
            .get(format!(
                "{}/resources/{}",
                restarted_url.replacen("ws://", "http://", 1),
                resource.resource_id
            ))
            .bearer_auth(&auth.token)
            .header("x-lankide-execution", &auth.execution)
            .header("x-lankide-attempt", &auth.attempt)
            .send()
            .await?
            .status(),
        403
    );
    let mut link = capability_link(&restarted_url, rig.computer_home()).await?;
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(
        response.error_code,
        lankide_protocol::Code::PermissionDenied as i32,
        "{response:?}"
    );
    rig.close().await
}

async fn capability_link(url: &str, home: &std::path::Path) -> Result<link::ClientLink> {
    let credentials: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("computer-credentials.json"))?)?;
    let secret = credentials["device_credential"].as_str().unwrap();
    let mut link = link::connect(url, secret, DEFAULT_MAX_MESSAGE_BYTES).await?;
    link.send(link::ComputerFrame {
        protocol_version: PROTOCOL_VERSION,
        frame: Some(link::ComputerPayload::Hello(link::ComputerHello {
            device_credential: secret.into(),
            computer_id: credentials["computer_id"].as_str().unwrap().into(),
            instance_id: "placement-capabilities".into(),
            computer_version: "1".into(),
            os: "test".into(),
            arch: "test".into(),
            protocol_version: PROTOCOL_VERSION,
            harnesses: vec![link::HarnessInfo {
                harness_id: "pi".into(),
                display_name: "Pi".into(),
                availability: HarnessAvailability::Available as i32,
                ..Default::default()
            }],
        })),
    })
    .await?;
    Ok(link)
}

async fn invoke(
    link: &mut link::ClientLink,
    request: CapabilityInvokeRequest,
) -> Result<link::CapabilityResponse> {
    link.send(link::ComputerFrame {
        protocol_version: PROTOCOL_VERSION,
        frame: Some(link::ComputerPayload::CapabilityRequest(
            link::CapabilityRequest {
                request_id: request.invocation_id.clone(),
                method: "invoke_capability".into(),
                context: request.context.clone(),
                payload: lankide_protocol::agent::encode(&request).expect("capability JSON"),
                offset: 0,
                eof: true,
            },
        )),
    })
    .await?;
    Ok(timeout(Duration::from_secs(3), async {
        loop {
            let frame = link
                .receive()
                .await?
                .ok_or("link ended before capability response")?;
            if let Some(link::RuntimePayload::CapabilityResponse(response)) = frame.frame {
                assert_eq!(response.request_id, request.invocation_id);
                assert!(response.eof);
                assert_eq!(response.offset, 0);
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(response);
            }
        }
    })
    .await??)
}

#[tokio::test]
async fn stale_unacknowledged_progress_is_discarded_without_disconnect_after_placement_change()
-> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        network_proxy: true,
        ..config()
    })
    .await?;
    let first = rig
        .management()
        .get_computer_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_computer().await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .computer(&first)
        .harness(Script::Progress {
            duration_ms: 60_000,
            interval_ms: 20,
        })
        .create()
        .await?;
    team.direct_message(&agent, "alice", "work").await?;
    agent.wait_until_running().await?;
    rig.proxy.as_ref().unwrap().pause_replies();
    timeout(Duration::from_secs(5), async {
        loop {
            if !rig.computer_state()?["pending_events"]
                .as_object()
                .unwrap()
                .is_empty()
            {
                break Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let original = agent.view().await?;
    rig.stop_computer()?;
    let mut moved = original.clone();
    moved.computer_id = second.id.clone();
    rig.management().update_agent(moved, None).await?;
    rig.management().update_agent(original, None).await?;
    rig.proxy.as_ref().unwrap().restore();
    // A new listener replaces the old proxy target. Use direct local discovery
    // for the next Computer, retaining the same paired home and durable outbox.
    rig.proxy = None;
    drop(second);
    rig.restart_runtime().await?;
    timeout(Duration::from_secs(5), async {
        loop {
            if rig.computer_state()?["pending_events"]
                .as_object()
                .unwrap()
                .is_empty()
            {
                break Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    assert!(rig.computer_online().await?);
    let agent = rig.existing_workspace("team").existing_agent("bruce");
    assert_eq!(
        rig.existing_workspace("team")
            .dm_transcript(&agent, "alice"),
        ["work"]
    );
    rig.close().await
}
