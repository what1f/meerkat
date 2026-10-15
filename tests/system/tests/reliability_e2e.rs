use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;
use tokio::time::timeout;

fn config() -> RigConfig {
    RigConfig {
        local_discovery: true,
        link: lankide_protocol::link::LinkSettings {
            heartbeat_interval: Duration::from_millis(200),
            handshake_timeout: Duration::from_secs(1),
        },
        reconnect: lankide_computer::ReconnectPolicy {
            initial: Duration::from_millis(30),
            maximum: Duration::from_millis(100),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn wait(mut condition: impl AsyncFnMut() -> Result<bool>) -> Result<()> {
    timeout(Duration::from_secs(10), async {
        loop {
            if condition().await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "reliability state timed out")?
}

#[tokio::test]
async fn large_offline_event_backlog_replays_and_delivers_once() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: true,
        local_discovery: false,
        timeout: Duration::from_secs(45),
        attempt_lease: Duration::from_secs(60),
        ..config()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::OfflineEvents {
            count: 300,
            bytes: 2048,
            text: "done".into(),
        })
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    let proxy = rig.proxy.as_ref().unwrap();
    proxy.cut();
    wait(async || Ok(!rig.computer_online().await?)).await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    agent.release_run().await?;
    timeout(Duration::from_secs(30), async {
        loop {
            if rig.computer_state()?["pending_events"]
                .as_object()
                .is_some_and(|attempts| {
                    attempts
                        .values()
                        .any(|events| events.as_object().unwrap().len() >= 300)
                })
            {
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "offline backlog accumulation timed out")??;
    proxy.delay_computer_frames(Duration::from_millis(2));
    proxy.restore();
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert_eq!(agent.executions().await?.len(), 1);
    assert_eq!(agent.harness_inputs()?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn offline_queued_message_survives_runtime_restart() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    team.agent("bruce")
        .harness(Script::reply("done"))
        .create()
        .await?;
    rig.stop_computer()?;
    wait(async || Ok(!rig.computer_online().await?)).await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    team.direct_message(&agent, "alice", "waiting").await?;
    assert_eq!(agent.inbox_states()?, ["pending"]);
    assert!(agent.executions().await?.is_empty());
    rig.restart_runtime().await?;
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["waiting", "done"]);
    rig.close().await
}

#[tokio::test]
async fn management_cancel_stops_surviving_run_after_runtime_crash() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team.agent("bruce").harness(Script::Hang).create().await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    let id = agent.executions().await?[0].id;
    let pids = rig.harness_pids()?;
    rig.crash_and_restart_runtime().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    agent.wait_for(|runs, _| runs[0].state == "running").await?;
    assert_eq!(rig.harness_pids()?, pids);
    rig.cancel_execution(id).await?;
    agent
        .wait_for(|runs, _| runs[0].state == "cancelled")
        .await?;
    wait(async || {
        Ok(pids.iter().all(|pid| {
            !std::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        }))
    })
    .await?;
    assert!(team.outbound(&agent, "alice").is_empty());
    rig.close().await
}

#[tokio::test]
async fn repeated_computer_loss_uses_backoff_and_pauses_with_one_notice() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        attempt_lease: Duration::from_millis(350),
        retry: lankide_runtime::RetryConfig {
            base_ms: 500,
            max_ms: 1000,
            poll_ms: 10,
            ..Default::default()
        },
        ..config()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team.agent("bruce").harness(Script::Hang).create().await?;
    team.direct_message(&agent, "alice", "hello").await?;
    for n in 0..3 {
        let team = rig.existing_workspace("team");
        let agent = team.existing_agent("bruce");
        agent
            .wait_for(|runs, _| runs.len() == n + 1 && runs[n].state == "running")
            .await?;
        if n > 0 {
            let runs = agent.executions().await?;
            let delay = if n == 1 { 500 } else { 1000 };
            assert!(
                runs[n].created_at >= runs[n - 1].completed_at.unwrap() + delay,
                "Computer loss bypassed retry backoff"
            );
        }
        rig.stop_computer()?;
        let team = rig.existing_workspace("team");
        let agent = team.existing_agent("bruce");
        agent.wait_for(|runs, _| runs[n].state == "failed").await?;
        rig.start_paired_computer()?;
        rig.wait_computer_online().await?;
    }
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert_eq!(agent.executions().await?.len(), 3);
    assert!(agent.view().await?.pause_reason.is_some());
    assert_eq!(team.outbound(&agent, "alice").len(), 1);
    for run in agent.executions().await? {
        assert_eq!(run.error_code.as_deref(), Some("computer_lost"));
        let details = rig.execution_details(run.id).await?;
        assert_eq!(
            details.attempts[0]
                .failure
                .as_ref()
                .ok_or("lost failure missing")?
                .code,
            "computer_lost"
        );
        assert_eq!(
            details.attempts[0]
                .failure
                .as_ref()
                .ok_or("lost failure missing")?
                .category,
            "transient"
        );
    }
    rig.close().await
}

#[tokio::test]
async fn lost_initial_delivery_receipt_replays_completion_without_rerun() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: true,
        local_discovery: false,
        ..config()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::ReplyBeforeTerminal {
            text: "done".into(),
        })
        .create()
        .await?;
    let proxy = rig.proxy.as_ref().unwrap();
    proxy.discard_computer_frame("delivery_result");
    team.direct_message(&agent, "alice", "hello").await?;
    agent
        .wait_for(|runs, _| runs.len() == 1 && runs[0].state == "running")
        .await?;
    wait(async || {
        if agent.harness_inputs()?.is_empty() {
            return Ok(false);
        }
        Ok(agent
            .native_controls()?
            .iter()
            .any(|c| c["op"] == "reply_sent"))
    })
    .await?;
    proxy.cut();
    wait(async || Ok(!rig.computer_online().await?)).await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    agent.release_run().await?;
    wait(async || {
        Ok(rig.computer_state()?["pending_events"]
            .as_object()
            .unwrap()
            .values()
            .any(|events| {
                events
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|e| e["kind"] == lankide_protocol::ExecutionEventKind::Completed as i32)
            }))
    })
    .await?;
    proxy.restore();
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert_eq!(agent.executions().await?.len(), 1);
    assert_eq!(agent.harness_inputs()?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn runtime_crash_during_preparation_releases_batch_for_next_run() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let gate = rig.hold_preparation();
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("done"))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent
        .wait_for(|runs, _| runs.len() == 1 && runs[0].state == "queued")
        .await?;
    assert!(agent.harness_inputs()?.is_empty());
    rig.crash_and_restart_runtime().await?;
    std::fs::write(gate, b"release")?;
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert_eq!(agent.harness_inputs()?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn cut_after_acceptance_replays_receipt_before_live_reply() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: true,
        local_discovery: false,
        ..config()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("done").held())
        .create()
        .await?;
    let proxy = rig.proxy.as_ref().unwrap();
    proxy.cut_before_computer_frame("delivery_result");
    team.direct_message(&agent, "alice", "hello").await?;
    agent
        .wait_for(|runs, _| runs.len() == 1 && runs[0].state == "running")
        .await?;
    wait(async || Ok(!rig.computer_online().await?)).await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    agent.release_run().await?;
    wait(async || {
        Ok(rig.computer_state()?["pending_events"]
            .as_object()
            .unwrap()
            .values()
            .any(|events| {
                events
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|e| e["kind"] == lankide_protocol::ExecutionEventKind::Started as i32)
            }))
    })
    .await?;
    proxy.restore();
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert_eq!(agent.executions().await?.len(), 1);
    assert_eq!(agent.harness_inputs()?.len(), 1);
    rig.close().await
}
