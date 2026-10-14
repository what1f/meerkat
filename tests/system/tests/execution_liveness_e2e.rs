use lankide_system_e2e::{Agent, Result, RigConfig, Script, SystemRig};
use std::{
    process::{Command, Stdio},
    time::Duration,
};
use tokio::time::{Instant, timeout};

fn config(idle: Duration, total: Duration) -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        execution_liveness: lankide_computer::ExecutionLiveness {
            idle_timeout: idle,
            total_budget: total,
            cancel_grace: Duration::from_millis(200),
        },
        ..Default::default()
    }
}

async fn terminal(rig: &SystemRig, id: i64) -> Result<lankide_client::ExecutionDetails> {
    timeout(Duration::from_secs(8), async {
        loop {
            let details = rig.execution_details(id).await?;
            if matches!(details.state.as_str(), "failed" | "cancelled" | "succeeded") {
                return Ok(details);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "attempt did not terminate")?
}

async fn first_execution(agent: &Agent<'_>) -> Result<i64> {
    agent.wait_until_running().await?;
    Ok(agent.executions().await?[0].id)
}

async fn gone(pids: &[u32]) -> Result<()> {
    timeout(Duration::from_secs(3), async {
        loop {
            let alive = pids.iter().any(|pid| {
                Command::new("/bin/kill")
                    .args(["-0", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
            });
            if !alive {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| format!("Harness process or child remains alive: {pids:?}").into())
}

#[tokio::test]
async fn s6_1_idle_timeout_releases_the_agent_for_its_next_queued_message() -> Result<()> {
    let rig = SystemRig::start(config(Duration::from_millis(700), Duration::from_secs(4))).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::HangOnce {
            text: "next reply".into(),
        })
        .create()
        .await?;
    team.mention(&bruce, "chat", "alice", "first").await?;
    let first = first_execution(&bruce).await?;
    team.mention(&bruce, "chat", "alice", "second").await?;
    let details = terminal(&rig, first).await?;
    let failure = details.attempts[0]
        .failure
        .as_ref()
        .expect("timeout failure");
    assert_eq!(failure.category, "timeout");
    assert!(failure.detail.contains("idle"));
    timeout(Duration::from_secs(8), async {
        loop {
            let executions = bruce.executions().await?;
            if executions.len() >= 2 && executions[1].state == "succeeded" {
                assert!(
                    executions[1]
                        .trigger_messages
                        .iter()
                        .any(|message| message == "second")
                );
                assert_eq!(team.outbound(&bruce, "chat").len(), 1);
                assert_eq!(team.outbound(&bruce, "chat")[0].content, "next reply");
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "next queued message did not run")??;
    // The Runtime retains its current requeue/retry behavior for the first batch.
    rig.close().await
}

#[tokio::test]
async fn s6_2_progress_resets_idle_but_cannot_extend_the_total_budget() -> Result<()> {
    let rig = SystemRig::start(config(
        Duration::from_millis(500),
        Duration::from_millis(1800),
    ))
    .await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::Progress {
            duration_ms: 30_000,
            interval_ms: 30,
        })
        .create()
        .await?;
    team.mention(&bruce, "chat", "alice", "work").await?;
    let id = first_execution(&bruce).await?;
    let details = terminal(&rig, id).await?;
    let failure = details.attempts[0]
        .failure
        .as_ref()
        .expect("timeout failure");
    assert_eq!(failure.category, "timeout");
    assert!(failure.detail.contains("total budget"), "{failure:?}");
    assert!(
        bruce.progress_sequence().await? >= 20,
        "progress was not observed across idle intervals"
    );
    assert!(team.outbound(&bruce, "chat").is_empty());
    gone(&rig.harness_pids()?).await?;
    rig.close().await
}

#[tokio::test]
async fn s6_3_timeout_kills_the_harness_and_its_spawned_child() -> Result<()> {
    let rig = SystemRig::start(config(Duration::from_millis(700), Duration::from_secs(4))).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::SpawnChild {
            text: String::new(),
            delay_ms: 0,
        })
        .create()
        .await?;
    team.mention(&bruce, "chat", "alice", "work").await?;
    let id = first_execution(&bruce).await?;
    let child = bruce.child_pid().await?;
    let details = terminal(&rig, id).await?;
    assert_eq!(
        details.attempts[0]
            .failure
            .as_ref()
            .expect("failure")
            .category,
        "timeout"
    );
    let mut pids = rig.harness_pids()?;
    pids.push(child);
    gone(&pids).await?;
    rig.close().await
}

#[tokio::test]
async fn s6_4_management_cancel_stops_the_process_tree_within_grace_plus_one_second() -> Result<()>
{
    let rig = SystemRig::start(config(Duration::from_secs(10), Duration::from_secs(30))).await?;
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::SpawnChild {
            text: String::new(),
            delay_ms: 0,
        })
        .create()
        .await?;
    team.mention(&bruce, "chat", "alice", "work").await?;
    let id = first_execution(&bruce).await?;
    let child = bruce.child_pid().await?;
    let mut pids = rig.harness_pids()?;
    pids.push(child);
    let started = Instant::now();
    rig.cancel_execution(id).await?;
    let details = terminal(&rig, id).await?;
    gone(&pids).await?;
    assert!(
        started.elapsed() <= Duration::from_millis(1200),
        "cancel took {:?}",
        started.elapsed()
    );
    assert_eq!(details.state, "cancelled");
    assert_eq!(
        details.attempts[0]
            .failure
            .as_ref()
            .expect("failure")
            .category,
        "cancelled"
    );
    assert!(team.outbound(&bruce, "chat").is_empty());
    rig.close().await
}
