use lankide_system_e2e::{OutboundKind, Result, RigConfig, Script, SystemRig};
use std::time::Duration;

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
    let mut rig = SystemRig::start(RigConfig {
        retry: lankide_runtime::RetryConfig {
            base_ms: 50,
            max_ms: 100,
            poll_ms: 10,
            ..Default::default()
        },
        ..config()
    })
    .await?;
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
    assert!(executions[1].created_at >= executions[0].completed_at.unwrap() + 50);
    assert_eq!(executions[1].trigger_messages, ["hello"]);
    rig.close().await
}

#[tokio::test]
async fn s1_1_direct_message_runs_only_the_receiving_agent() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::reply("hi Alice"))
        .create()
        .await?;
    let clark = team.agent("clark").create().await?;
    let diana = team.agent("diana").create().await?;
    team.direct_message(&bruce, "alice", "private to bruce")
        .await?;
    rig.settle().await?;
    assert_eq!(
        team.dm_transcript(&bruce, "alice"),
        ["private to bruce", "hi Alice"]
    );
    assert_eq!(bruce.executions().await?.len(), 1);
    assert_eq!(team.outbound(&bruce, "alice").len(), 1);
    for other in [&clark, &diana] {
        assert!(other.executions().await?.is_empty());
        assert_eq!(other.inbox_count()?, 0);
        assert!(other.harness_inputs()?.is_empty());
        assert!(team.outbound(other, "alice").is_empty());
        assert!(team.dm_transcript(other, "alice").is_empty());
    }
    rig.close().await
}

#[tokio::test]
async fn s1_2_same_person_has_separate_conversations_with_two_bots() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::reply("Bruce reply"))
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .harness(Script::reply("Clark reply"))
        .create()
        .await?;
    team.direct_message(&bruce, "alice", "Bruce secret").await?;
    rig.settle().await?;
    team.direct_message(&clark, "alice", "Clark secret").await?;
    rig.settle().await?;
    // A later Bruce run must still exclude Clark's conversation.
    team.direct_message(&bruce, "alice", "Bruce followup")
        .await?;
    rig.settle().await?;
    assert_eq!(
        team.dm_transcript(&bruce, "alice"),
        [
            "Bruce secret",
            "Bruce reply",
            "Bruce followup",
            "Bruce reply"
        ]
    );
    assert_eq!(
        team.dm_transcript(&clark, "alice"),
        ["Clark secret", "Clark reply"]
    );
    let bruce_inputs = bruce.harness_inputs()?;
    assert_eq!(bruce_inputs.len(), 2);
    assert!(bruce_inputs[0].contains("Bruce secret"));
    assert!(bruce_inputs[1].contains("Bruce followup"));
    assert!(
        bruce_inputs
            .iter()
            .all(|input| !input.contains("Clark secret") && !input.contains("Clark reply"))
    );
    let clark_inputs = clark.harness_inputs()?;
    assert_eq!(clark_inputs.len(), 1);
    assert!(clark_inputs[0].contains("Clark secret"));
    assert!(!clark_inputs[0].contains("Bruce secret"));
    assert!(!clark_inputs[0].contains("Bruce reply"));
    let bruce_replies = team.outbound(&bruce, "alice");
    let clark_replies = team.outbound(&clark, "alice");
    assert!(
        bruce_replies
            .iter()
            .chain(&clark_replies)
            .all(|send| send.kind == OutboundKind::Reply && send.chat == "alice")
    );
    // Platform IDs are scoped to each bot's conversation and can be identical.
    assert_eq!(bruce_replies[0].reply_to.as_deref(), Some("incoming-1"));
    assert_eq!(clark_replies[0].reply_to.as_deref(), Some("incoming-1"));
    assert_eq!(bruce_replies[1].reply_to.as_deref(), Some("incoming-2"));
    rig.close().await
}

#[tokio::test]
async fn s1_3_unmentioned_group_message_keeps_each_agents_attention_policy() -> Result<()> {
    use lankide_protocol::AttentionMode;
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team.agent("bruce").create().await?;
    let clark = team
        .agent("clark")
        .harness(Script::reply("following"))
        .create()
        .await?;
    let diana = team.agent("diana").create().await?;
    bruce.group_attention(AttentionMode::Mention).await?;
    clark.group_attention(AttentionMode::Always).await?;
    diana.group_attention(AttentionMode::Mute).await?;
    team.group_message("team-chat", "alice", "hello everyone")
        .await?;
    rig.settle().await?;
    assert_eq!(
        team.transcript(&clark, "team-chat"),
        ["hello everyone", "following"]
    );
    assert_eq!(clark.executions().await?.len(), 1);
    for other in [&bruce, &diana] {
        assert!(other.executions().await?.is_empty());
        assert_eq!(other.inbox_count()?, 0);
        assert!(team.outbound(other, "team-chat").is_empty());
    }
    rig.close().await
}

#[tokio::test]
async fn unaddressed_direct_message_is_dropped_without_an_inbox_or_execution() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team.agent("bruce").create().await?;
    let clark = team.agent("clark").create().await?;
    team.unaddressed_direct_message("alice", "no receiver")
        .await?;
    rig.settle().await?;
    for agent in [&bruce, &clark] {
        assert_eq!(agent.inbox_count()?, 0);
        assert!(agent.executions().await?.is_empty());
        assert!(team.outbound(agent, "alice").is_empty());
    }
    rig.close().await
}

#[tokio::test]
async fn direct_reactions_with_the_same_event_id_stay_in_the_receiving_bots_chat() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::reply("Bruce reply"))
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .harness(Script::reply("Clark reply"))
        .create()
        .await?;
    let diana = team.agent("diana").create().await?;
    team.direct_message(&bruce, "alice", "Bruce secret").await?;
    team.direct_message(&clark, "alice", "Clark secret").await?;
    rig.settle().await?;
    let bruce_target = team.outbound(&bruce, "alice")[0].message_id.clone();
    let clark_target = team.outbound(&clark, "alice")[0].message_id.clone();
    team.direct_reaction(&bruce, "alice", &bruce_target, "reaction-1")
        .await?;
    rig.settle().await?;
    assert_eq!(bruce.executions().await?.len(), 2);
    assert_eq!(clark.executions().await?.len(), 1);
    team.direct_reaction(&clark, "alice", &clark_target, "reaction-1")
        .await?;
    rig.settle().await?;
    assert_eq!(clark.executions().await?.len(), 2);
    // Replaying a delivery must not schedule another run.
    team.direct_reaction(&bruce, "alice", &bruce_target, "reaction-1")
        .await?;
    rig.settle().await?;
    assert_eq!(bruce.executions().await?.len(), 2);
    assert_eq!(diana.inbox_count()?, 0);
    assert!(diana.executions().await?.is_empty());
    assert!(team.outbound(&diana, "alice").is_empty());
    assert!(
        bruce
            .harness_inputs()?
            .iter()
            .all(|input| !input.contains("Clark secret") && !input.contains("Clark reply"))
    );
    assert!(
        clark
            .harness_inputs()?
            .iter()
            .all(|input| !input.contains("Bruce secret") && !input.contains("Bruce reply"))
    );
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
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()?
                .status
                .success()
            {
                return Ok::<_, std::io::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("deleted Agent process is still running")?;
    rig.close().await
}
