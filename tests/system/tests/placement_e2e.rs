use meerkat_client::{AgentDraft, ClientError, ManagementClient};
use meerkat_protocol::{
    link::{self, MachineLink},
    *,
};
use meerkat_system_e2e::{Result, RigConfig, Script, SystemRig};

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
async fn s13_1_placement_requires_registered_non_revoked_machine_and_reported_harness() -> Result<()>
{
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let remote = rig.add_machine().await?;
    let machine = remote.id.clone();
    let draft = AgentDraft {
        id: "bruce".into(),
        name: "Bruce".into(),
        machine_id: "unknown".into(),
        harness_id: "pi".into(),
        ..Default::default()
    };
    assert!(matches!(
        rig.management().create_agent("team", draft.clone()).await,
        Err(ClientError::MachineNotRegistered)
    ));
    let mut unavailable = draft;
    unavailable.machine_id = machine.clone();
    unavailable.harness_id = "absent".into();
    assert!(matches!(
        rig.management().create_agent("team", unavailable).await,
        Err(ClientError::HarnessUnavailableOnMachine)
    ));
    let agent = team
        .agent("bruce")
        .machine(&machine)
        .harness(Script::reply("hi"))
        .create()
        .await?;
    let original = agent.view().await?;
    let mut changed = original.clone();
    changed.machine_id = "unknown".into();
    assert!(matches!(
        rig.management().update_agent(changed, None).await,
        Err(ClientError::MachineNotRegistered)
    ));
    let mut changed = original.clone();
    changed.harness_id = "absent".into();
    assert!(matches!(
        rig.management().update_agent(changed, None).await,
        Err(ClientError::HarnessUnavailableOnMachine)
    ));
    assert_eq!(agent.view().await?.machine_id, original.machine_id);
    assert!(
        rig.management().revoke_machine(&machine).await.is_err(),
        "must move assigned Agents before removing their Machine"
    );
    let destination = rig.add_machine().await?;
    let mut moved = original.clone();
    moved.machine_id = destination.id.clone();
    rig.management().update_agent(moved, None).await?;
    // The move carries the Agent home first; the Machine is released once it lands.
    timeout(Duration::from_secs(20), async {
        while agent.view().await?.machine_id != destination.id {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    rig.management().revoke_machine(&machine).await?;
    let revoked = AgentDraft {
        id: "clark".into(),
        name: "Clark".into(),
        machine_id: machine,
        harness_id: "pi".into(),
        ..Default::default()
    };
    assert!(matches!(
        rig.management().create_agent("team", revoked).await,
        Err(ClientError::MachineRevoked)
    ));
    assert!(matches!(
        rig.management().update_agent(original, None).await,
        Err(ClientError::MachineRevoked)
    ));
    assert!(
        rig.management()
            .get_machine_catalog(None)
            .await?
            .iter()
            .any(|c| c.id == remote.id && c.revoked_at.is_some() && !c.available)
    );
    drop(remote);
    drop(destination);
    rig.close().await
}

#[tokio::test]
async fn s13_2_each_agent_runs_only_on_its_own_machine() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let first = rig
        .management()
        .get_machine_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_machine().await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .machine(&first)
        .harness(Script::reply("first"))
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .machine(&second.id)
        .harness(Script::reply("second"))
        .create()
        .await?;
    team.direct_message(&bruce, "alice", "one").await?;
    team.direct_message(&clark, "alice", "two").await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&bruce, "alice"), ["one", "first"]);
    assert_eq!(team.dm_transcript(&clark, "alice"), ["two", "second"]);
    for (home, present, absent) in [
        (rig.machine_home(), "bruce", "clark"),
        (second.home.path(), "clark", "bruce"),
    ] {
        let root = home.join("workspaces");
        assert!(meerkat_machine_core::agent_directory(&root, "team", present)?.exists());
        assert!(!meerkat_machine_core::agent_directory(&root, "team", absent)?.exists());
    }
    drop(second);
    rig.close().await
}

#[tokio::test]
async fn agent_moves_to_another_machine_with_its_home() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let first = rig
        .management()
        .get_machine_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_machine().await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .machine(&first)
        .harness(Script::reply("hi"))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "one").await?;
    rig.settle().await?;
    let home = |root: &std::path::Path| {
        meerkat_machine_core::agent_directory(&root.join("workspaces"), "team", "bruce")
    };
    let old_home = home(rig.machine_home())?;
    std::fs::create_dir_all(old_home.join("memory"))?;
    std::fs::write(
        old_home.join("memory/MEMORY.md"),
        "- Alice takes her tea green\n",
    )?;
    std::fs::create_dir_all(old_home.join("notes/drafts"))?;
    std::fs::write(old_home.join("notes/drafts/plan.md"), "ship friday")?;
    std::fs::create_dir_all(old_home.join("resources"))?;
    std::fs::write(old_home.join("resources/cache.bin"), [1, 2, 3])?;

    let mut moved = agent.view().await?;
    moved.machine_id = second.id.clone();
    rig.management().update_agent(moved, None).await?;
    timeout(Duration::from_secs(20), async {
        while agent.view().await?.machine_id != second.id || old_home.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    let new_home = home(second.home.path())?;
    assert_eq!(
        std::fs::read_to_string(new_home.join("memory/MEMORY.md"))?,
        "- Alice takes her tea green\n"
    );
    assert_eq!(
        std::fs::read_to_string(new_home.join("notes/drafts/plan.md"))?,
        "ship friday"
    );
    assert!(!new_home.join("resources").exists());

    team.direct_message(&agent, "alice", "two").await?;
    rig.settle().await?;
    assert_eq!(
        team.dm_transcript(&agent, "alice"),
        ["one", "hi", "two", "hi"]
    );
    let first_run = agent.harness_inputs()?.pop().unwrap_or_default();
    assert!(
        first_run.contains("Alice takes her tea green"),
        "{first_run}"
    );
    assert!(first_run.contains("另一台电脑"), "{first_run}");
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
    rig.stop_machine()?;
    timeout(Duration::from_secs(5), async {
        while rig.machine_online().await? {
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
    rig.start_paired_machine()?;
    rig.wait_machine_online().await?;
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["waiting", "back"]);
    assert_eq!(agent.executions().await?.len(), 1);
    assert!(agent.view().await?.pause_reason.is_none());
    rig.close().await
}

#[tokio::test]
async fn s13_4_assigned_machine_removal_is_refused_and_its_result_is_preserved() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: true,
        ..config()
    })
    .await?;
    let device = meerkat_machine::read_credentials(rig.machine_home())?.unwrap();
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .machine(&device.machine_id)
        .harness(Script::Reply {
            text: "late".into(),
            held: true,
            delay_ms: 0,
            deltas: vec![],
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
    let machine = agent.view().await?.machine_id;
    rig.proxy.as_ref().unwrap().pause();
    agent.release_run().await?;
    assert!(matches!(
        rig.management().revoke_machine(&machine).await,
        Err(ClientError::MachineHasAssignedAgents)
    ));
    rig.proxy.as_ref().unwrap().restore();
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    timeout(Duration::from_secs(5), async {
        while team.dm_transcript(&agent, "alice") != ["hello", "late"] {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "late"]);
    assert_eq!(agent.executions().await?[0].output.as_deref(), Some("OK"));
    assert!(
        agent
            .executions()
            .await?
            .iter()
            .all(|r| r.state == "succeeded")
    );
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
    let (url, _) = rig.machine_endpoint()?;
    let cutter = cutter::Cutter::start(&url, None, false).await?;
    rig.route_machine_through(&cutter.url).await?;
    let first = rig
        .management()
        .get_machine_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_machine().await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .machine(&first)
        .harness(Script::Hang)
        .create()
        .await?;
    let resource = team
        .image_message(&agent, "images", vec![1, 2, 3, 4])
        .await?;
    agent.wait_until_running().await?;
    let auth = cutter.downloads().remove(0);
    assert!(auth.range.is_none());
    let context = meerkat_protocol::AgentRunContext {
        execution: Some(meerkat_protocol::ExecutionRef {
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
            .header("x-meerkat-execution", &auth.execution)
            .header("x-meerkat-attempt", &auth.attempt)
    };
    // Replay through an authenticated Machine link, without heartbeat-driven
    // cancellation masking the placement authorization being checked here.
    rig.stop_machine()?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    let mut link = capability_link(&url, rig.machine_home()).await?;
    let capability = || meerkat_protocol::CapabilityInvokeRequest {
        context: Some(context.clone()),
        capability_name: "unread_current".into(),
        arguments_json: "{}".into(),
        invocation_id: "placement-read".into(),
    };
    assert_eq!(download().send().await?.status(), 200);
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(response.error_code, 0, "{response:?}");
    assert!(
        meerkat_protocol::agent::decode::<meerkat_protocol::CapabilityInvokeResult>(
            response.payload.as_slice()
        )?
        .success
    );
    // A valid token must also be rejected on another registered Machine.
    let second_id = second.id.clone();
    let second_home = tempfile::tempdir()?;
    std::fs::copy(
        second.home.path().join("machine-credentials.json"),
        second_home.path().join("machine-credentials.json"),
    )?;
    drop(second);
    let mut other_link = capability_link(&url, second_home.path()).await?;
    let response = invoke(&mut other_link, capability()).await?;
    assert_eq!(
        response.error_code,
        meerkat_protocol::Code::PermissionDenied as i32,
        "{response:?}"
    );
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(response.error_code, 0, "{response:?}");
    assert!(
        meerkat_protocol::agent::decode::<meerkat_protocol::CapabilityInvokeResult>(
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
        meerkat_protocol::agent::decode::<meerkat_protocol::CapabilityInvokeResult>(
            response.payload.as_slice()
        )?
        .success
    );
    // Both Machines are offline, so the Agent moves without its files.
    for (offset, machine) in [&second_id, &original.machine_id].into_iter().enumerate() {
        rig.management()
            .move_agent_without_files("team", "bruce", machine)
            .await?;
        assert_eq!(generation()?, initial + offset as i64 + 1);
        assert_eq!(download().send().await?.status(), 403);
        let response = invoke(&mut link, capability()).await?;
        assert_eq!(
            response.error_code,
            meerkat_protocol::Code::PermissionDenied as i32,
            "{response:?}"
        );
    }
    // The generation and invalid token survive a Runtime restart.
    drop(database);
    drop(other_link);
    drop(link);
    rig.restart_runtime().await?;
    rig.stop_machine()?;
    let (restarted_url, _) = rig.machine_endpoint()?;
    assert_eq!(
        client
            .get(format!(
                "{}/resources/{}",
                restarted_url.replacen("ws://", "http://", 1),
                resource.resource_id
            ))
            .bearer_auth(&auth.token)
            .header("x-meerkat-execution", &auth.execution)
            .header("x-meerkat-attempt", &auth.attempt)
            .send()
            .await?
            .status(),
        403
    );
    let mut link = capability_link(&restarted_url, rig.machine_home()).await?;
    let response = invoke(&mut link, capability()).await?;
    assert_eq!(
        response.error_code,
        meerkat_protocol::Code::PermissionDenied as i32,
        "{response:?}"
    );
    rig.close().await
}

async fn capability_link(url: &str, home: &std::path::Path) -> Result<link::ClientLink> {
    let credentials: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("machine-credentials.json"))?)?;
    let secret = credentials["device_credential"].as_str().unwrap();
    let mut link = link::connect(url, secret, DEFAULT_MAX_MESSAGE_BYTES).await?;
    link.send(link::MachineFrame {
        protocol_version: PROTOCOL_VERSION,
        frame: Some(link::MachinePayload::Hello(link::MachineHello {
            device_credential: secret.into(),
            machine_id: credentials["machine_id"].as_str().unwrap().into(),
            instance_id: "placement-capabilities".into(),
            machine_version: "1".into(),
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
    link.send(link::MachineFrame {
        protocol_version: PROTOCOL_VERSION,
        frame: Some(link::MachinePayload::CapabilityRequest(
            link::CapabilityRequest {
                request_id: request.invocation_id.clone(),
                method: "invoke_capability".into(),
                context: request.context.clone(),
                payload: meerkat_protocol::agent::encode(&request).expect("capability JSON"),
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
        .get_machine_catalog(None)
        .await?
        .remove(0)
        .id;
    let second = rig.add_machine().await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .machine(&first)
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
            if !rig.machine_state()?["pending_events"]
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
    rig.stop_machine()?;
    let management = rig.management();
    management
        .move_agent_without_files("team", "bruce", &second.id)
        .await?;
    management
        .move_agent_without_files("team", "bruce", &original.machine_id)
        .await?;
    rig.proxy.as_ref().unwrap().restore();
    // A new listener replaces the old proxy target. Use direct local discovery
    // for the next Machine, retaining the same paired home and durable outbox.
    rig.proxy = None;
    drop(second);
    rig.restart_runtime().await?;
    timeout(Duration::from_secs(5), async {
        loop {
            if rig.machine_state()?["pending_events"]
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
    assert!(rig.machine_online().await?);
    let agent = rig.existing_workspace("team").existing_agent("bruce");
    assert_eq!(
        rig.existing_workspace("team")
            .dm_transcript(&agent, "alice"),
        ["work"]
    );
    rig.close().await
}
