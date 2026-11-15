use meerkat_system_e2e::{Agent, Result, RigConfig, Script, SystemRig};
use std::{
    process::{Command, Stdio},
    time::Duration,
};
use tokio::time::{Instant, timeout};

fn config(idle: Duration, total: Duration) -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        execution_liveness: meerkat_machine_core::ExecutionLiveness {
            idle_timeout: idle,
            total_budget: total,
            cancel_grace: Duration::from_millis(200),
        },
        retry: meerkat_runtime::RetryConfig {
            base_ms: 30_000,
            max_ms: 30_000,
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn terminal(rig: &SystemRig, id: i64) -> Result<meerkat_client::ExecutionDetails> {
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
    let mut config = config(Duration::from_millis(700), Duration::from_secs(4));
    config.retry.base_ms = 80;
    config.retry.max_ms = 320;
    let rig = SystemRig::start(config).await?;
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
    rig.settle().await?;
    let executions = bruce.executions().await?;
    assert_eq!(executions.len(), 3);
    assert_eq!(executions[0].state, "failed");
    assert_eq!(executions[1].state, "succeeded");
    assert_eq!(executions[1].trigger_messages, ["first"]);
    assert!(executions[1].created_at >= executions[0].completed_at.unwrap() + 80);
    assert_eq!(executions[2].state, "succeeded");
    assert_eq!(executions[2].trigger_messages, ["second"]);
    let sends = team.outbound(&bruce, "chat");
    assert_eq!(sends.len(), 2);
    assert!(sends.iter().all(|send| send.content == "next reply"));
    assert!(bruce.view().await?.pause_reason.is_none());
    rig.close().await
}

#[tokio::test]
async fn s6_2_progress_resets_idle_but_cannot_extend_the_total_budget() -> Result<()> {
    let rig = SystemRig::start(config(
        Duration::from_millis(600),
        Duration::from_millis(2400),
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
        bruce.progress_sequence().await? >= 8,
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

#[tokio::test]
async fn timeout_without_cancel_closes_and_kills_without_native_cancel() -> Result<()> {
    without_native_cancel(false).await
}

#[tokio::test]
async fn management_cancel_without_cancel_closes_and_kills_within_grace() -> Result<()> {
    without_native_cancel(true).await
}

async fn without_native_cancel(management_cancel: bool) -> Result<()> {
    let mut config = config(
        if management_cancel {
            Duration::from_secs(10)
        } else {
            Duration::from_millis(700)
        },
        Duration::from_secs(30),
    );
    config
        .harness_capabilities
        .retain(|capability| capability != "cancel");
    // Keep a retry from starting before process-cleanup assertions finish.
    config.retry.base_ms = 30_000;
    config.retry.max_ms = 30_000;
    let rig = SystemRig::start(config).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::SpawnChild {
            text: String::new(),
            delay_ms: 0,
        })
        .create()
        .await?;
    team.mention(&agent, "chat", "alice", "work").await?;
    let id = first_execution(&agent).await?;
    let child = agent.child_pid().await?;
    let mut pids = rig.harness_pids()?;
    pids.push(child);
    let started = Instant::now();
    if management_cancel {
        rig.cancel_execution(id).await?;
    }
    let details = terminal(&rig, id).await?;
    gone(&pids).await?;
    if management_cancel {
        assert!(started.elapsed() <= Duration::from_millis(1200));
    }
    assert_eq!(
        details.state,
        if management_cancel {
            "cancelled"
        } else {
            "failed"
        }
    );
    assert_eq!(
        details.attempts[0]
            .failure
            .as_ref()
            .expect("failure")
            .category,
        if management_cancel {
            "cancelled"
        } else {
            "timeout"
        }
    );
    let controls = agent.native_controls()?;
    assert!(
        controls.iter().all(|command| command["op"] != "cancel"),
        "{controls:?}"
    );
    assert!(team.outbound(&agent, "chat").is_empty());
    rig.close().await
}
