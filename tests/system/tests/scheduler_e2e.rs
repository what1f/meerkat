use meerkat_client::{ManagementClient, WorkspaceStatus};
use meerkat_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;

#[tokio::test]
async fn a_paused_workspace_loses_its_offline_attempt_and_retries_only_after_resume() -> Result<()>
{
    let rig = SystemRig::start(RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        network_proxy: true,
        attempt_lease: Duration::from_millis(800),
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
    rig.management()
        .set_workspace_state("team", WorkspaceStatus::Paused)
        .await?;
    rig.proxy.as_ref().unwrap().cut();
    // The owner's execution details must show Machine loss even while every
    // Workspace is paused and no Provider is receiving another event.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if rig.execution_details(id).await?.state == "failed" {
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "offline attempt remained active while its Workspace was paused")??;
    assert_eq!(agent.executions().await?.len(), 1);
    assert!(team.outbound(&agent, "alice").is_empty());
    agent
        .change_script(Script::reply("done"), "recovered-model")
        .await?;
    rig.proxy.as_ref().unwrap().restore();
    rig.management()
        .set_workspace_state("team", WorkspaceStatus::Active)
        .await?;
    rig.settle().await?;
    let runs = agent.executions().await?;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].error_code.as_deref(), Some("machine_lost"));
    assert_eq!(runs[1].state, "succeeded");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert_eq!(team.outbound(&agent, "alice").len(), 1);
    rig.close().await
}
