use lankide_client::ManagementClient;
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;

#[tokio::test]
async fn concurrent_workspaces_deliver_each_reply_once_while_management_lists_agents() -> Result<()>
{
    let rig = SystemRig::start(RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        ..Default::default()
    })
    .await?;
    let mut teams = Vec::new();
    for name in ["north", "south", "west"] {
        teams.push(rig.workspace(name).await?);
    }
    let mut agents = Vec::new();
    for team in &teams {
        for name in ["bruce", "diana"] {
            agents.push(
                team.agent(name)
                    .harness(Script::reply("received").held())
                    .create()
                    .await?,
            );
        }
    }
    let sends = agents.iter().enumerate().map(|(index, agent)| {
        let team = &teams[index / 2];
        async move { team.direct_message(agent, "alice", "hello").await }
    });
    for result in futures_util::future::join_all(sends).await {
        result?;
    }
    for agent in &agents {
        agent.wait_until_running().await?;
    }
    // Management remains usable while every Agent has an active conversation.
    tokio::time::timeout(Duration::from_secs(2), async {
        let workspaces = rig.management().list_workspaces().await?;
        assert_eq!(workspaces.len(), 3);
        let computers = rig.management().get_computer_catalog(None).await?;
        assert!(computers.iter().any(|computer| computer.available));
        for name in ["north", "south", "west"] {
            assert_eq!(rig.management().list_agents(name).await?.len(), 2);
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    let release = async {
        for agent in &agents {
            agent.release_run().await?;
        }
        rig.settle().await
    };
    let reads = async {
        for _ in 0..12 {
            assert_eq!(rig.management().list_workspaces().await?.len(), 3);
            rig.management().get_computer_catalog(None).await?;
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    };
    let (released, listed) = tokio::join!(release, reads);
    released?;
    listed?;
    for (index, agent) in agents.iter().enumerate() {
        let team = &teams[index / 2];
        assert_eq!(team.dm_transcript(agent, "alice"), ["hello", "received"]);
        assert_eq!(team.outbound(agent, "alice").len(), 1);
        let executions = agent.executions().await?;
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].state, "succeeded");
    }
    rig.close().await
}
