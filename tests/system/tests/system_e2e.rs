use lankide_system_e2e::{OutboundKind, Result, RigConfig, Script, SystemRig};

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn mentioning_an_agent_replies_once_in_the_group_without_running_other_agents() -> Result<()>
{
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::reply("hi").streaming(["h".into(), "i".into()]))
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .harness(Script::reply("hey"))
        .create()
        .await?;
    let diana = team
        .agent("diana")
        .harness(Script::reply("hello"))
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "hello bruce")
        .await?;
    rig.settle().await?;
    assert_eq!(team.transcript(&bruce, "team-chat"), ["hello bruce", "hi"]);
    let sends = team.outbound(&bruce, "team-chat");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].kind, OutboundKind::Reply);
    let executions = bruce.executions().await?;
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].state, "succeeded");
    for other in [&clark, &diana] {
        assert!(other.executions().await?.is_empty());
        assert!(team.outbound(other, "team-chat").is_empty());
    }
    rig.close().await
}

#[tokio::test]
async fn s7_1_a_message_received_without_steer_waits_for_the_next_run() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::reply("hi").held())
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "first").await?;
    bruce.wait_until_running().await?;
    team.mention(&bruce, "team-chat", "alice", "second").await?;
    assert_eq!(bruce.executions().await?.len(), 1);
    assert!(team.outbound(&bruce, "team-chat").is_empty());
    bruce.release_run().await?;
    rig.settle().await?;
    assert_eq!(
        team.transcript(&bruce, "team-chat"),
        ["first", "second", "hi", "hi"]
    );
    let replies = team.outbound(&bruce, "team-chat");
    assert_eq!(replies.len(), 2);
    assert!(
        replies
            .iter()
            .all(|reply| reply.kind == OutboundKind::Reply)
    );
    assert_ne!(replies[0].reply_to, replies[1].reply_to);
    let executions = bruce.executions().await?;
    assert_eq!(executions.len(), 2, "{executions:?}");
    assert_eq!(executions[0].trigger_messages, ["first"]);
    assert_eq!(executions[1].trigger_messages, ["second"]);
    assert!(
        bruce.received_steers()?.is_empty(),
        "native Harness received a steer"
    );
    assert!(
        executions
            .iter()
            .all(|execution| execution.state == "succeeded")
    );
    rig.close().await
}

#[tokio::test]
async fn restarting_the_computer_during_a_run_delivers_the_reply_exactly_once() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    {
        let team = rig.workspace("team").await?;
        let bruce = team
            .agent("bruce")
            .harness(Script::reply("survived").held())
            .create()
            .await?;
        team.mention(&bruce, "team-chat", "alice", "hello").await?;
        bruce.wait_until_running().await?;
        assert!(team.outbound(&bruce, "team-chat").is_empty());
    }
    rig.restart_computer().await?;
    rig.existing_workspace("team")
        .existing_agent("bruce")
        .release_run()
        .await?;
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let bruce = team.existing_agent("bruce");
    assert_eq!(team.transcript(&bruce, "team-chat"), ["hello", "survived"]);
    assert_eq!(team.outbound(&bruce, "team-chat").len(), 1);
    let executions = bruce.executions().await?;
    assert_eq!(executions.len(), 2, "{executions:?}");
    assert_eq!(executions[0].state, "failed");
    assert_eq!(executions[0].error_code.as_deref(), Some("computer_lost"));
    assert_eq!(executions[1].state, "succeeded");
    assert_eq!(executions[1].trigger_messages, ["hello"]);
    rig.close().await
}

#[tokio::test]
async fn an_agent_without_resume_or_cancel_starts_fresh_and_can_be_deleted_during_a_run()
-> Result<()> {
    let rig = SystemRig::start(RigConfig {
        harness_capabilities: ["system_prompt", "streaming", "tools"]
            .map(str::to_owned)
            .to_vec(),
        ..config()
    })
    .await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::reply("hi"))
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "first").await?;
    rig.settle().await?;
    team.mention(&bruce, "team-chat", "alice", "second").await?;
    rig.settle().await?;
    let controls = bruce.native_controls()?;
    let starts: Vec<_> = controls
        .iter()
        .filter(|command| command["op"] == "start")
        .collect();
    assert_eq!(starts.len(), 2, "{controls:?}");
    assert!(controls.iter().all(|command| command["op"] != "resume"));
    assert!(
        starts
            .iter()
            .all(|command| command["native_session_id"].is_null())
    );

    let clark = team
        .agent("clark")
        .harness(Script::reply("held").held())
        .create()
        .await?;
    team.mention(&clark, "team-chat", "alice", "work").await?;
    clark.wait_until_running().await?;
    clark.delete().await?;
    assert!(team.outbound(&clark, "team-chat").is_empty());
    let controls = clark.native_controls()?;
    assert!(
        controls.iter().all(|command| command["op"] != "cancel"),
        "unsupported native cancel: {controls:?}"
    );
    let pid = controls[0]["pid"].as_u64().expect("native PID");
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()?
            .status
            .success(),
        "deleted Agent process is still running"
    );
    rig.close().await
}
