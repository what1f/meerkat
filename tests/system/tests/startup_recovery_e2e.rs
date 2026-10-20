use lankide_client::{ManagementClient, WorkspaceObservedState, WorkspaceStatus};
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};

async fn create_team(rig: &SystemRig) -> Result<()> {
    rig.workspace("team")
        .await?
        .agent("bruce")
        .harness(Script::reply("done"))
        .create()
        .await?;
    Ok(())
}

async fn assert_setup_failed(rig: &SystemRig) -> Result<()> {
    let team = rig.management().get_workspace("team").await?;
    assert_eq!(team.status, WorkspaceStatus::Active);
    assert_eq!(team.observed_state, WorkspaceObservedState::Error);
    assert!(!team.status_message.is_empty());
    Ok(())
}

async fn assert_working(rig: &SystemRig) -> Result<()> {
    let state = rig.management().get_workspace("team").await?;
    assert_eq!(state.status, WorkspaceStatus::Active);
    assert_eq!(state.observed_state, WorkspaceObservedState::Running);
    assert!(state.status_message.is_empty());
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    assert_eq!(agent.executions().await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn provider_startup_failure_keeps_shift_and_recovers_on_restart() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        create_team(&rig).await?;
        rig.workspace("off_shift").await?;
        rig.management()
            .set_workspace_state("off_shift", WorkspaceStatus::Paused)
            .await?;
        rig.fail_next_provider_startup("team");
        rig.restart_runtime().await?;
        assert_setup_failed(&rig).await?;
        assert_eq!(
            rig.management().get_workspace("off_shift").await?.status,
            WorkspaceStatus::Paused
        );
        rig.restart_runtime().await?;
        assert_eq!(
            rig.management().get_workspace("off_shift").await?.status,
            WorkspaceStatus::Paused
        );
        assert_working(&rig).await
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn missing_agent_column_keeps_shift_and_recovers_after_repair() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        create_team(&rig).await?;
        rig.stop_runtime().await?;
        // Reproduce the real fault on the isolated database, with real SQLite.
        let path = rig.runtime_home().join("runtime.db");
        rusqlite::Connection::open(&path)?.execute_batch(
            "ALTER TABLE agents RENAME COLUMN launch_arguments_json TO unavailable_arguments",
        )?;
        rig.start_runtime().await?;
        assert_setup_failed(&rig).await?;
        rig.stop_runtime().await?;
        rusqlite::Connection::open(&path)?.execute_batch(
            "ALTER TABLE agents RENAME COLUMN unavailable_arguments TO launch_arguments_json",
        )?;
        rig.start_runtime().await?;
        assert_working(&rig).await
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn provider_reconfiguration_failure_keeps_shift_and_retry_clears_error() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        create_team(&rig).await?;
        let mut team = rig.management().get_workspace("team").await?;
        team.provider_config = serde_json::json!({"connection": "changed"});
        rig.fail_next_provider_startup("team");
        assert!(rig.management().update_workspace(team).await.is_err());
        assert_setup_failed(&rig).await?;
        let mut team = rig.management().get_workspace("team").await?;
        team.provider_config = serde_json::json!({"connection": "repaired"});
        let recovered = rig.management().update_workspace(team).await?;
        assert_eq!(recovered.observed_state, WorkspaceObservedState::Running);
        assert!(recovered.status_message.is_empty());
        assert_working(&rig).await
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn requested_shift_survives_provider_failure_and_restart() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        create_team(&rig).await?;
        rig.management()
            .set_workspace_state("team", WorkspaceStatus::Paused)
            .await?;
        rig.fail_next_provider_startup("team");
        assert!(
            rig.management()
                .set_workspace_state("team", WorkspaceStatus::Active)
                .await
                .is_err()
        );
        assert_setup_failed(&rig).await?;
        rig.restart_runtime().await?;
        assert_working(&rig).await
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
