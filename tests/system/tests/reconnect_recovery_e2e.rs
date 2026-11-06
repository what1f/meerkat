use meerkat_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;
use tokio::time::timeout;

async fn wait(mut condition: impl AsyncFnMut() -> Result<bool>) -> Result<()> {
    timeout(Duration::from_secs(10), async {
        loop {
            if condition().await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "recovery state timed out")?
}

#[tokio::test]
async fn held_run_is_readopted_with_messages_during_repeated_disconnects() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: true,
        retry: meerkat_runtime::RetryConfig {
            base_ms: 40,
            max_ms: 160,
            ..Default::default()
        },
        link: meerkat_protocol::link::LinkSettings {
            heartbeat_interval: Duration::from_millis(100),
            handshake_timeout: Duration::from_secs(1),
        },
        ..Default::default()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("done").held())
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    let id = agent.executions().await?[0].id;
    let proxy = rig.proxy.as_ref().unwrap();
    for flap in 0..4 {
        proxy.cut();
        wait(async || Ok(!rig.machine_online().await?)).await?;
        agent
            .wait_for(|runs, _| runs.len() == 1 && runs[0].state == "recovering")
            .await?;
        team.direct_message(&agent, "bob", &format!("follow-up {flap}"))
            .await?;
        // Provider injection returns only after the real Runtime accepts ingress.
        assert_eq!(agent.inbox_count()?, flap as i64 + 2);
        assert_eq!(agent.executions().await?.len(), 1);
        assert!(agent.view().await?.pause_reason.is_none());
        proxy.restore();
        wait(async || rig.machine_online().await).await?;
        agent
            .wait_for(|runs, _| runs.len() == 1 && runs[0].state == "running")
            .await?;
        assert_eq!(agent.executions().await?[0].id, id);
    }
    agent.release_run().await?;
    rig.settle().await?;
    let runs = agent.executions().await?;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id, id);
    assert_eq!(runs[0].state, "succeeded");
    assert_eq!(runs[1].state, "succeeded");
    assert_eq!(
        runs[1].trigger_messages,
        ["follow-up 0", "follow-up 1", "follow-up 2", "follow-up 3"]
    );
    assert!(agent.view().await?.pause_reason.is_none());
    assert_eq!(
        team.outbound(&agent, "alice")
            .iter()
            .map(|send| send.content.as_str())
            .collect::<Vec<_>>(),
        ["done"]
    );
    assert_eq!(
        team.outbound(&agent, "bob")
            .iter()
            .map(|send| send.content.as_str())
            .collect::<Vec<_>>(),
        ["done"]
    );
    rig.close().await
}

#[tokio::test]
async fn expired_offline_attempt_is_closed_on_return_before_queued_work_runs() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        network_proxy: true,
        // Keep the normal lease for the retry as well as the offline attempt.
        // A short lease can expire healthy assignment preparation under load.
        timeout: Duration::from_secs(45),
        retry: meerkat_runtime::RetryConfig {
            base_ms: 40,
            max_ms: 160,
            ..Default::default()
        },
        link: meerkat_protocol::link::LinkSettings {
            heartbeat_interval: Duration::from_millis(100),
            handshake_timeout: Duration::from_secs(1),
        },
        ..Default::default()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("done").held())
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    rig.proxy.as_ref().unwrap().cut();
    agent
        .wait_for(|runs, _| runs.len() == 1 && runs[0].state == "failed")
        .await?;
    agent
        .change_script(Script::reply("done"), "recovered-model")
        .await?;
    rig.proxy.as_ref().unwrap().restore();
    rig.settle().await?;
    let runs = agent.executions().await?;
    assert_eq!(runs.len(), 2, "{runs:#?}");
    assert_eq!(runs[0].error_code.as_deref(), Some("machine_lost"));
    assert_eq!(runs[1].state, "succeeded");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert!(agent.view().await?.pause_reason.is_none());
    rig.close().await
}
