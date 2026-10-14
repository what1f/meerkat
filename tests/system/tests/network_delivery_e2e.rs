use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;
use tokio::time::{Instant, timeout};

fn config() -> RigConfig {
    RigConfig {
        network_proxy: true,
        link: lankide_protocol::link::LinkSettings {
            heartbeat_interval: Duration::from_millis(200),
            handshake_timeout: Duration::from_millis(400),
        },
        reconnect: lankide_computer::ReconnectPolicy {
            initial: Duration::from_millis(50),
            maximum: Duration::from_millis(800),
            healthy_duration: Duration::from_secs(3),
            handshake_timeout: Duration::from_millis(400),
            multiplier: 2,
        },
        ..Default::default()
    }
}
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
    .map_err(|_| "observable network state timed out")?
}

#[tokio::test]
async fn s10_1_cut_mid_turn_replays_one_reply() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("delivered").held())
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    let proxy = rig.proxy.as_ref().unwrap();
    let attempts = proxy.attempts().len();
    proxy.cut();
    agent.release_run().await?;
    wait(async || Ok(!rig.computer_online().await? && proxy.attempts().len() >= attempts + 4))
        .await?;
    assert!(
        proxy
            .attempts()
            .last()
            .unwrap()
            .duration_since(proxy.attempts()[attempts - 1])
            > rig_config_deadline()
    );
    proxy.restore();
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "delivered"]);
    assert_eq!(agent.executions().await?.len(), 1);
    rig.close().await
}
fn rig_config_deadline() -> Duration {
    Duration::from_millis(600)
}

#[tokio::test]
async fn s10_2_stalled_frames_close_both_sides() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let proxy = rig.proxy.as_ref().unwrap();
    let attempts = proxy.attempts().len();
    let paused = Instant::now();
    proxy.pause();
    wait(async || Ok(!rig.computer_online().await?)).await?;
    let runtime_detected = paused.elapsed();
    wait(async || Ok(proxy.attempts().len() > attempts)).await?;
    let computer_detected = proxy.attempts()[attempts] - paused;
    // Scheduling allowance and first reconnect jitter are separate from liveness.
    assert!(
        runtime_detected < Duration::from_millis(750),
        "Runtime detection: {runtime_detected:?}"
    );
    assert!(
        computer_detected < Duration::from_millis(850),
        "Computer reconnect: {computer_detected:?}"
    );
    proxy.restore();
    wait(async || rig.computer_online().await).await?;
    rig.close().await
}

#[tokio::test]
async fn s10_3_sigkill_after_finish_before_ack_replays_once() -> Result<()> {
    let mut cfg = config();
    cfg.computer_process = Some(env!("CARGO_BIN_EXE_system-computer").into());
    let mut rig = SystemRig::start(cfg).await?;
    {
        let team = rig.workspace("team").await?;
        let agent = team
            .agent("bruce")
            .harness(Script::reply("durable").held())
            .create()
            .await?;
        team.direct_message(&agent, "alice", "hello").await?;
        agent.wait_until_running().await?;
        rig.proxy.as_ref().unwrap().pause_replies();
        agent.release_run().await?;
        wait(async || {
            let state = rig.computer_state()?;
            Ok(state["pending_events"]
                .as_object()
                .unwrap()
                .values()
                .any(|events| {
                    events.as_object().unwrap().values().any(|e| {
                        e["kind"] == lankide_protocol::ExecutionEventKind::Completed as i32
                    })
                }))
        })
        .await?;
        rig.settle().await?;
        assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "durable"]);
    }
    rig.kill_computer_process()?; // Child::kill sends SIGKILL on Unix.
    rig.proxy.as_ref().unwrap().cut();
    wait(async || Ok(!rig.computer_online().await?)).await?;
    rig.proxy.as_ref().unwrap().restore();
    rig.restart_computer_process()?;
    wait(async || {
        Ok(rig.computer_online().await?
            && rig.computer_state()?["pending_events"]
                .as_object()
                .unwrap()
                .is_empty())
    })
    .await?;
    rig.settle().await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "durable"]);
    assert_eq!(agent.executions().await?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn s10_4_ten_flaps_follow_backoff_and_complete() -> Result<()> {
    let mut cfg = config();
    cfg.reconnect.maximum = Duration::from_secs(2);
    let rig = SystemRig::start(cfg).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("done").held())
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    let proxy = rig.proxy.as_ref().unwrap();
    let start = Instant::now();
    let mut nominal = Duration::from_millis(50);
    for flap in 0..10 {
        let attempts = proxy.attempts().len();
        let cut_at = Instant::now();
        proxy.cut();
        wait(async || Ok(proxy.attempts().len() > attempts)).await?;
        let reconnect_gap = proxy.attempts()[attempts] - cut_at;
        assert!(
            reconnect_gap >= nominal.mul_f64(0.70),
            "flap {flap}: {reconnect_gap:?} expected nominal {nominal:?}"
        );
        nominal = (nominal * 4).min(Duration::from_secs(2));
        wait(async || Ok(!rig.computer_online().await?)).await?;
        proxy.restore();
        wait(async || rig.computer_online().await).await?;
    }
    assert!(start.elapsed() < Duration::from_secs(60));
    let attempts = proxy.attempts();
    // Failed short connections must not reset exponential backoff.
    let gaps: Vec<_> = attempts.windows(2).skip(2).map(|p| p[1] - p[0]).collect();
    assert!(
        gaps.iter()
            .skip(3)
            .all(|gap| *gap >= Duration::from_millis(550)),
        "connection gaps: {gaps:?}"
    );
    // Observe a healthy connection for the configured reset duration.
    let healthy_since = Instant::now();
    wait(async || {
        Ok(rig.computer_online().await? && healthy_since.elapsed() >= Duration::from_millis(3200))
    })
    .await?;
    let before = proxy.attempts().len();
    let cut_at = Instant::now();
    proxy.cut();
    wait(async || Ok(proxy.attempts().len() > before)).await?;
    // Without a reset the next attempt waits at least 75% of the 2 s maximum.
    let gap = proxy.attempts()[before] - cut_at;
    assert!(
        gap < Duration::from_millis(1000),
        "healthy connection did not reset backoff: {gap:?}"
    );
    proxy.restore();
    wait(async || rig.computer_online().await).await?;
    agent.release_run().await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    let executions = agent.executions().await?;
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].state, "succeeded");
    rig.close().await
}

#[tokio::test]
async fn offline_past_attempt_lease_fails_the_run() -> Result<()> {
    let mut cfg = config();
    cfg.attempt_lease = Duration::from_secs(2);
    cfg.retry.poll_ms = 50;
    let rig = SystemRig::start(cfg).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("late").held())
        .create()
        .await?;
    team.direct_message(&agent, "alice", "hello").await?;
    agent.wait_until_running().await?;
    rig.proxy.as_ref().unwrap().cut();
    wait(async || {
        let executions = agent.executions().await?;
        Ok(executions[0].state == "failed")
    })
    .await?;
    let executions = agent.executions().await?;
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].error_code.as_deref(), Some("computer_lost"));
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello"]);
    rig.close().await
}

#[tokio::test]
async fn silent_tcp_and_ws_peers_release_runtime_connections() -> Result<()> {
    use lankide_protocol::link::{CloseReason, ComputerLink, LinkError};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rig = SystemRig::start(config()).await?;
    let (url, secret) = rig.computer_endpoint()?;
    // Both an empty connection and an incomplete HTTP head must release the socket.
    for head in [
        b"".as_slice(),
        b"GET / HTTP/1.1\r\nHost: localhost\r\n".as_slice(),
    ] {
        let mut tcp = tokio::net::TcpStream::connect(url.trim_start_matches("ws://")).await?;
        tcp.write_all(head).await?;
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_secs(1), tcp.read(&mut byte)).await??,
            0
        );
    }
    let mut ws =
        lankide_protocol::link::connect(&url, &secret, lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES)
            .await?;
    assert!(matches!(
        timeout(Duration::from_secs(2), ws.receive()).await?,
        Err(LinkError::Closed(CloseReason::Timeout))
    ));
    assert!(rig.computer_online().await?);
    rig.close().await
}
