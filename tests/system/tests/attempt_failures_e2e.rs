use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        ..Default::default()
    }
}

async fn failed_details(
    rig: &SystemRig,
    agent: &lankide_system_e2e::Agent<'_>,
) -> Result<lankide_client::ExecutionDetails> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            for execution in agent.executions().await? {
                let details = rig.execution_details(execution.id).await?;
                if details.state == "failed"
                    && details
                        .attempts
                        .iter()
                        .any(|attempt| attempt.failure.is_some())
                {
                    assert_eq!(details.state, "failed");
                    return Ok(details);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "failure did not reach execution details")?
}

#[tokio::test]
async fn s4_1_spawn_failure_keeps_original_detail_and_identifies_the_attempt_in_machine_logs()
-> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let text = "model not found: unavailable-model";
    let bruce = team
        .agent("bruce")
        .harness(Script::FailSpawn { text: text.into() })
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "hello").await?;
    let details = failed_details(&rig, &bruce).await?;
    assert_eq!(details.attempts.len(), 1);
    let attempt = &details.attempts[0];
    assert_eq!(attempt.state, "rejected");
    let failure = attempt.failure.as_ref().expect("spawn failure");
    assert_eq!(failure.category, "configuration");
    assert!(!failure.retryable);
    assert_eq!(failure.detail, text);
    assert_eq!(failure.fingerprint.len(), 64);
    let logs = rig.machine_failure_log()?;
    let records = logs
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert!(
        records
            .iter()
            .any(|record| record["execution_id"] == details.execution_id
                && record["attempt"] == attempt.attempt
                && record["agent_id"] == "bruce"
                && record["machine_id"] == attempt.machine_id
                && record["failure"]["detail"] == text
                && record["failure"]["fingerprint"] == failure.fingerprint)
    );
    assert!(
        team.outbound(&bruce, "team-chat")
            .iter()
            .all(|send| send.content == lankide_core::agent_failure_notice("hello"))
    );
    rig.close().await
}

#[tokio::test]
async fn s4_2_mid_turn_rate_limit_is_stored_with_the_failed_attempt() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let text = "rate limit exceeded: too many requests (429)";
    let bruce = team
        .agent("bruce")
        .harness(Script::FailMidTurn { text: text.into() })
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "hello").await?;
    let details = failed_details(&rig, &bruce).await?;
    let attempt = &details.attempts[0];
    assert_eq!(attempt.state, "failed");
    let failure = attempt.failure.as_ref().expect("mid-turn failure");
    assert_eq!(failure.category, "rate_limit");
    assert!(failure.retryable);
    assert_eq!(failure.detail, text);
    assert_eq!(failure.code, "scripted_failure");
    assert_eq!(failure.fingerprint.len(), 64);
    assert!(
        team.outbound(&bruce, "team-chat")
            .iter()
            .all(|send| send.content == lankide_core::agent_failure_notice("hello"))
    );
    rig.close().await
}

#[tokio::test]
async fn s4_3_spawn_and_mid_turn_diagnostics_never_reach_any_chat() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::FailSpawn {
            text: "model not found: private configuration detail".into(),
        })
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .harness(Script::FailMidTurn {
            text: "rate limit: private diagnostic detail".into(),
        })
        .create()
        .await?;
    team.mention(&bruce, "first-chat", "alice", "hello bruce")
        .await?;
    team.mention(&clark, "second-chat", "alice", "hello clark")
        .await?;
    failed_details(&rig, &bruce).await?;
    failed_details(&rig, &clark).await?;
    assert!(
        rig.outbound()
            .iter()
            .all(|send| send.content == lankide_core::agent_failure_notice("hello")),
        "only the localized notice can reach a chat"
    );
    rig.close().await
}
