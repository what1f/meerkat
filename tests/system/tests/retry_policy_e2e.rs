use lankide_runtime::RetryConfig;
use lankide_system_e2e::{OutboundKind, Result, RigConfig, Script, SystemRig};

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        retry: RetryConfig {
            base_ms: 40,
            max_ms: 160,
            notice_dedupe_ms: 900_000,
        },
        ..Default::default()
    }
}
fn sequence(texts: &[&str]) -> Script {
    Script::FailSequence {
        texts: texts.iter().map(|s| s.to_string()).collect(),
        reply: "done".into(),
    }
}

#[tokio::test]
async fn s5_1_configuration_pauses_queues_and_model_fix_resumes() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::FailSpawn {
            text: "model not found: /private/secret".into(),
        })
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent
        .wait_for(|e, a| e.len() == 1 && a.pause_reason.is_some())
        .await?;
    team.direct_message(&agent, "alice", "queued").await?;
    assert_eq!(agent.executions().await?.len(), 1);
    agent
        .change_script(Script::reply("fixed"), "fixed-model")
        .await?;
    rig.settle().await?;
    assert!(agent.view().await?.pause_reason.is_none());
    assert_eq!(agent.executions().await?.len(), 2);
    let sends = team.outbound(&agent, "alice");
    assert_eq!(sends.len(), 2);
    assert_eq!(
        sends[0].content,
        lankide_core::agent_failure_notice("hello")
    );
    assert_eq!(sends[0].kind, OutboundKind::Send);
    assert!(sends[0].reply_to.is_none());
    assert_eq!(sends[1].content, "fixed");
    rig.close().await
}

#[tokio::test]
async fn s5_2_transient_succeeds_on_third_attempt_without_notice() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(sequence(&["transient one", "transient two"]))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    let executions = agent.executions().await?;
    assert_eq!(executions.len(), 3);
    for (index, pair) in executions.windows(2).enumerate() {
        assert!(
            pair[1].created_at >= pair[0].completed_at.unwrap() + 40 * (1 << index),
            "backoff must increase for the frozen batch"
        );
    }
    assert_eq!(
        team.outbound(&agent, "alice")
            .iter()
            .map(|s| s.content.as_str())
            .collect::<Vec<_>>(),
        ["done"]
    );
    rig.close().await
}

#[tokio::test]
async fn s5_3_five_distinct_failures_end_the_batch() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(sequence(&[
            "transient 1",
            "transient 2",
            "transient 3",
            "transient 4",
            "transient 5",
        ]))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    let executions = agent.executions().await?;
    assert_eq!(executions.len(), 5);
    for (index, pair) in executions.windows(2).enumerate() {
        assert!(
            pair[1].created_at >= pair[0].completed_at.unwrap() + (40 * (1 << index)).min(160),
            "backoff must respect the configured cap"
        );
    }
    assert!(
        agent
            .executions()
            .await?
            .iter()
            .all(|e| e.state == "failed")
    );
    assert!(agent.view().await?.pause_reason.is_none());
    assert_eq!(agent.inbox_states()?, ["failed"]);
    assert_eq!(team.outbound(&agent, "alice").len(), 1);
    team.direct_message(&agent, "alice", "next batch").await?;
    rig.settle().await?;
    assert_eq!(agent.executions().await?.len(), 6);
    assert_eq!(team.outbound(&agent, "alice")[1].content, "done");
    rig.close().await
}

#[tokio::test]
async fn s5_4_identical_fingerprint_pauses_after_three_attempts() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::FailMidTurn {
            text: "rate limit unchanged".into(),
        })
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(agent.executions().await?.len(), 3);
    assert_eq!(
        agent.view().await?.pause_reason.as_deref(),
        Some("rate limit unchanged")
    );
    assert_eq!(team.outbound(&agent, "alice").len(), 1);
    rig.close().await
}

#[tokio::test]
async fn s5_5_ten_failures_dedupe_notices_durably() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    rig.workspace("team")
        .await?
        .agent("bruce")
        .harness(Script::FailSpawn {
            text: "model not found".into(),
        })
        .create()
        .await?;
    for n in 1..=10 {
        let team = rig.existing_workspace("team");
        let agent = team.existing_agent("bruce");
        team.direct_message(&agent, "alice", "hello").await?;
        rig.settle().await?;
        agent
            .wait_for(|e, a| e.len() == n && a.pause_reason.is_some())
            .await?;
        assert_eq!(team.outbound(&agent, "alice").len(), 1);
        agent.resume().await?;
        if n == 5 {
            rig.restart_runtime().await?;
        }
    }
    rig.close().await
}

#[tokio::test]
async fn s5_6_localized_notices_hide_diagnostics_and_use_own_identity() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    for (name, text) in [("bruce", "hello"), ("clark", "你好")] {
        let agent = team
            .agent(name)
            .harness(Script::FailSpawn {
                text: "model not found: /secret/path pi exit 127 E_BAD".into(),
            })
            .create()
            .await?;
        team.direct_message(&agent, "alice", text).await?;
        rig.settle().await?;
        let sends = team.outbound(&agent, "alice");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].content, lankide_core::agent_failure_notice(text));
        assert_eq!(sends[0].account, agent.account);
        assert_eq!(sends[0].kind, OutboundKind::Send);
        for secret in ["/secret", "pi", "127", "E_BAD", "model"] {
            assert!(!sends[0].content.contains(secret));
        }
    }
    rig.close().await
}

#[tokio::test]
async fn s5_7_restart_preserves_backoff_and_does_not_repeat_notice() -> Result<()> {
    let mut cfg = config();
    cfg.retry.base_ms = 500;
    cfg.retry.max_ms = 500;
    let mut rig = SystemRig::start(cfg).await?;
    {
        let team = rig.workspace("team").await?;
        let agent = team
            .agent("bruce")
            .harness(sequence(&["transient once"]))
            .create()
            .await?;
        team.direct_message(&agent, "alice", "hello").await?;
        agent
            .wait_for(|e, _| e.len() == 1 && e[0].state == "failed")
            .await?;
    }
    rig.restart_runtime().await?;
    rig.settle().await?;
    {
        let team = rig.existing_workspace("team");
        let agent = team.existing_agent("bruce");
        let executions = agent.executions().await?;
        assert_eq!(executions.len(), 2);
        assert!(
            executions[1].created_at >= executions[0].completed_at.unwrap() + 500,
            "restart must preserve the retry deadline"
        );
        assert_eq!(team.outbound(&agent, "alice").len(), 1);
        agent
            .change_script(
                Script::FailSpawn {
                    text: "model not found".into(),
                },
                "bad-model",
            )
            .await?;
        team.direct_message(&agent, "alice", "fail").await?;
        rig.settle().await?;
        assert_eq!(team.outbound(&agent, "alice").len(), 2);
    }
    rig.restart_runtime().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert!(agent.view().await?.pause_reason.is_some());
    assert_eq!(team.outbound(&agent, "alice").len(), 2);
    rig.close().await
}

#[tokio::test]
async fn s5_8_client_resume_runs_queued_messages() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(sequence(&[
            "transient same",
            "transient same",
            "transient same",
        ]))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "first").await?;
    rig.settle().await?;
    assert!(agent.view().await?.pause_reason.is_some());
    team.direct_message(&agent, "alice", "queued").await?;
    agent.resume().await?;
    rig.settle().await?;
    assert_eq!(agent.executions().await?.len(), 4);
    assert!(agent.view().await?.pause_reason.is_none());
    assert_eq!(team.outbound(&agent, "alice")[1].content, "done");
    rig.close().await
}

#[tokio::test]
async fn authentication_pauses_and_cancelled_does_not_retry_or_notify() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    for (name, text, paused, notices) in [
        ("bruce", "authentication invalid", true, 1),
        ("clark", "cancelled", false, 0),
    ] {
        let agent = team
            .agent(name)
            .harness(Script::FailMidTurn { text: text.into() })
            .create()
            .await?;
        team.direct_message(&agent, "alice", "hello").await?;
        rig.settle().await?;
        assert_eq!(agent.executions().await?.len(), 1);
        assert_eq!(agent.view().await?.pause_reason.is_some(), paused);
        assert_eq!(team.outbound(&agent, "alice").len(), notices);
    }
    rig.close().await
}

#[tokio::test]
async fn agent_and_timeout_failures_retry_even_if_adapter_marks_nonretryable() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    for (name, text) in [("bruce", "agent failed"), ("clark", "timeout expired")] {
        let agent = team.agent(name).harness(sequence(&[text])).create().await?;
        team.direct_message(&agent, "alice", "hello").await?;
        rig.settle().await?;
        assert_eq!(agent.executions().await?.len(), 2);
        assert_eq!(team.outbound(&agent, "alice")[0].content, "done");
    }
    rig.close().await
}

#[tokio::test]
async fn messages_during_backoff_keep_their_own_batch() -> Result<()> {
    let mut cfg = config();
    cfg.retry.base_ms = 300;
    cfg.retry.max_ms = 300;
    let rig = SystemRig::start(cfg).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(sequence(&["transient once"]))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "first").await?;
    agent
        .wait_for(|e, _| e.len() == 1 && e[0].state == "failed")
        .await?;
    team.direct_message(&agent, "alice", "queued").await?;
    rig.settle().await?;
    let executions = agent.executions().await?;
    assert_eq!(executions.len(), 3);
    assert_eq!(executions[0].trigger_messages, ["first"]);
    assert_eq!(executions[1].trigger_messages, ["first"]);
    assert_eq!(executions[2].trigger_messages, ["queued"]);
    rig.close().await
}

// The native Harness boundary supplies the structured category. The real
// Computer busy rejection is covered by computer_e2e's parallel-run scenario.
#[tokio::test]
async fn ownership_conflicts_do_not_spend_the_failure_budget_or_send_notices() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(sequence(&[
            "ownership conflict",
            "ownership conflict",
            "ownership conflict",
            "ownership conflict",
            "ownership conflict",
            "ownership conflict",
            "transient unavailable",
        ]))
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    let runs = agent.executions().await?;
    assert_eq!(runs.len(), 8);
    assert!(agent.view().await?.pause_reason.is_none());
    for run in &runs[..6] {
        let details = rig.execution_details(run.id).await?;
        assert_eq!(
            details.attempts[0].failure.as_ref().unwrap().category,
            "conflict"
        );
    }
    assert_eq!(runs[7].state, "succeeded");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    rig.close().await
}
