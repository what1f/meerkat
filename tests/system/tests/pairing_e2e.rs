use lankide_client::ManagementClient;
use lankide_computer_daemon::{ComputerDaemonPaths, pair, read_credentials};
use lankide_protocol::link::{self, CloseReason, ComputerLink, ComputerPayload, LinkError};
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        auto_start_computer: false,
        ..Default::default()
    }
}
async fn pair_rig(rig: &SystemRig) -> Result<lankide_computer_daemon::DeviceCredentials> {
    let code = rig.management().create_pairing_code().await?;
    let (url, _) = rig.local_pairing_endpoint()?;
    Ok(pair(
        &ComputerDaemonPaths::for_home(rig.computer_home())?,
        &url,
        &code.code,
    )
    .await?)
}
async fn wait_offline(rig: &SystemRig, id: &str) -> Result<()> {
    timeout(Duration::from_secs(5), async {
        loop {
            if rig
                .management()
                .get_computer_catalog(None)
                .await?
                .iter()
                .any(|c| c.id == id && !c.available)
            {
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn s12_1_pair_fresh_code_register_and_connect() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut rig = SystemRig::start(config()).await?;
    let credentials = pair_rig(&rig).await?;
    assert_ne!(credentials.computer_id, "local");
    assert_eq!(credentials.device_credential.len(), 64);
    let file = rig.computer_home().join("computer-credentials.json");
    assert_eq!(
        std::fs::metadata(&file)?.permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        read_credentials(rig.computer_home())?.unwrap().runtime_url,
        credentials.runtime_url
    );
    let db = rusqlite::Connection::open(rig.runtime_home().join("runtime.db"))?;
    let hash: String = db.query_row(
        "SELECT credential_hash FROM computers WHERE computer_id=?1",
        [&credentials.computer_id],
        |r| r.get(0),
    )?;
    assert_ne!(hash, credentials.device_credential);
    drop(db);
    rig.start_paired_computer()?;
    rig.wait_computer_online().await?;
    let catalog = rig.management().get_computer_catalog(None).await?;
    let device = catalog
        .iter()
        .find(|c| c.id == credentials.computer_id)
        .unwrap();
    assert!(device.registered && device.available && device.revoked_at.is_none());
    assert!(device.last_seen_at.is_some());
    assert!(device.inventory.iter().any(|h| h.harness_id == "pi"));
    assert_eq!(device.os, std::env::consts::OS);
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::reply("hi"))
        .create()
        .await?;
    assert_eq!(
        rig.management()
            .get_agent("team", "bruce")
            .await?
            .computer_id,
        credentials.computer_id
    );
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "hi"]);
    rig.close().await
}

#[tokio::test]
async fn s12_2_codes_are_single_use_and_expire() -> Result<()> {
    let rig = SystemRig::start(RigConfig {
        pairing_validity: Duration::from_secs(1),
        ..config()
    })
    .await?;
    let code = rig.management().create_pairing_code().await?;
    let (url, _) = rig.local_pairing_endpoint()?;
    pair(
        &ComputerDaemonPaths::for_home(rig.computer_home())?,
        &url,
        &code.code,
    )
    .await?;
    let other = tempfile::tempdir()?;
    assert!(
        pair(
            &ComputerDaemonPaths::for_home(other.path())?,
            &url,
            &code.code
        )
        .await
        .is_err()
    );
    let expired = rig.management().create_pairing_code().await?;
    timeout(Duration::from_secs(5), async {
        loop {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            if now >= expired.expires_at {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert!(
        pair(
            &ComputerDaemonPaths::for_home(other.path())?,
            &url,
            &expired.code
        )
        .await
        .is_err()
    );
    assert!(!other.path().join("computer-credentials.json").exists());
    assert_eq!(rig.management().get_computer_catalog(None).await?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn s12_3_stopped_computer_stays_registered_with_inventory() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.start_paired_computer()?;
    rig.wait_computer_online().await?;
    rig.stop_computer()?;
    wait_offline(&rig, &device.computer_id).await?;
    let catalog = rig.management().get_computer_catalog(None).await?;
    let computer = &catalog[0];
    assert!(computer.registered && !computer.available && computer.revoked_at.is_none());
    assert!(computer.last_seen_at.is_some());
    assert!(computer.inventory.iter().any(|h| h.harness_id == "pi"));
    rig.start_paired_computer()?;
    rig.wait_computer_online().await?;
    assert_eq!(rig.management().get_computer_catalog(None).await?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn s12_4_revocation_closes_link_and_stops_daemon() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.start_paired_computer()?;
    rig.wait_computer_online().await?;
    rig.management()
        .revoke_computer(&device.computer_id)
        .await?;
    assert!(
        rig.await_terminal_computer()
            .await?
            .contains("computer_revoked")
    );
    wait_offline(&rig, &device.computer_id).await?;
    let catalog = rig.management().get_computer_catalog(None).await?;
    assert!(catalog[0].registered && catalog[0].revoked_at.is_some() && !catalog[0].available);
    rig.close().await
}

#[tokio::test]
async fn s12_5_copied_revoked_home_cannot_connect() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.management()
        .revoke_computer(&device.computer_id)
        .await?;
    let copied = tempfile::tempdir()?;
    std::fs::copy(
        rig.computer_home().join("computer-credentials.json"),
        copied.path().join("computer-credentials.json"),
    )?;
    let mut daemon = std::process::Command::new(env!("CARGO_BIN_EXE_system-computer"))
        .arg(copied.path())
        .arg(&device.runtime_url)
        .arg(copied.path().join("unused-secret"))
        .arg(rig.runtime_socket())
        .arg(env!("CARGO_BIN_EXE_system-harness"))
        .arg(r#"["system_prompt","streaming","resume","cancel","tools"]"#)
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    timeout(Duration::from_secs(5), async {
        loop {
            if daemon.try_wait()?.is_some() {
                return Ok::<_, std::io::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let output = daemon.wait_with_output()?;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("computer_revoked"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!rig.computer_online().await?);
    rig.close().await
}

#[tokio::test]
async fn local_self_pairing_moves_legacy_agents_and_reuses_credentials() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        local_discovery: true,
        ..config()
    })
    .await?;
    rig.workspace("team").await?;
    // Seed a pre-registration Agent through Storage. New management requests
    // must no longer be able to create this legacy placement.
    let storage = lankide_storage::Storage::open(lankide_storage::DatabaseConfig::new(
        rig.runtime_home().join("runtime.db"),
    ))?;
    let workspace = lankide_core::WorkspaceId::new("team")?;
    let agent = lankide_core::AgentId::new("bruce")?;
    storage.create_agent(lankide_storage::AgentSpec {
        workspace_id: workspace.clone(),
        id: agent.clone(),
        name: "bruce".into(),
        now: 1,
    })?;
    let mut record = storage.agent_record(&workspace, &agent)?;
    record.computer_id = "local".into();
    record.harness_id = "pi".into();
    record.model = "scripted".into();
    record.thinking = "low".into();
    record.cli_path = Some(env!("CARGO_BIN_EXE_system-harness").into());
    record.env.insert(
        "LANKIDE_SYSTEM_SCRIPT".into(),
        serde_json::to_string(&Script::reply("hi"))?,
    );
    for (key, file) in [
        ("LANKIDE_SYSTEM_CONTROLS", "controls-team-bruce.jsonl"),
        ("LANKIDE_SYSTEM_INPUTS", "inputs-team-bruce.jsonl"),
        ("LANKIDE_SYSTEM_STEERS", "steers-team-bruce.jsonl"),
    ] {
        record.env.insert(
            key.into(),
            rig.computer_home().join(file).display().to_string(),
        );
    }
    record.provider_agent_config = serde_json::json!({"account":"bot-bruce"});
    storage.update_agent_record(record)?;
    storage.close()?;
    assert_eq!(
        rig.management()
            .get_agent("team", "bruce")
            .await?
            .computer_id,
        "local"
    );
    rig.restart_runtime().await?;
    let credentials = read_credentials(rig.computer_home())?.unwrap();
    assert!(credentials.local_discovery);
    assert_ne!(credentials.computer_id, "local");
    assert_eq!(
        rig.management()
            .get_agent("team", "bruce")
            .await?
            .computer_id,
        credentials.computer_id
    );
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "hi"]);
    rig.restart_runtime().await?;
    assert_eq!(
        read_credentials(rig.computer_home())?
            .unwrap()
            .device_credential,
        credentials.device_credential
    );
    assert_eq!(rig.management().get_computer_catalog(None).await?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn local_secret_and_unknown_or_mismatched_devices_cannot_authorize_links() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    let (url, local_secret) = rig.local_pairing_endpoint()?;
    assert!(matches!(
        link::connect(
            &url,
            &local_secret,
            lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES
        )
        .await,
        Err(LinkError::Closed(CloseReason::ComputerUnknown))
    ));
    let mut socket = link::connect(
        &url,
        &device.device_credential,
        lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES,
    )
    .await?;
    socket
        .send(link::ComputerFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(ComputerPayload::Hello(link::ComputerHello {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                computer_id: "unknown".into(),
                device_credential: device.device_credential.clone(),
                instance_id: "one".into(),
                computer_version: "1".into(),
                os: "test".into(),
                arch: "test".into(),
                ..Default::default()
            })),
        })
        .await?;
    assert!(matches!(
        timeout(Duration::from_secs(3), socket.receive()).await?,
        Err(LinkError::Closed(CloseReason::ComputerUnknown))
    ));
    let unknown_home = tempfile::tempdir()?;
    let unknown = lankide_computer_daemon::DeviceCredentials {
        computer_id: "unknown".into(),
        ..device
    };
    std::fs::write(
        unknown_home.path().join("computer-credentials.json"),
        serde_json::to_vec(&unknown)?,
    )?;
    let config = lankide_computer_daemon::DaemonConfig::for_home(unknown_home.path())?;
    let error = timeout(
        Duration::from_secs(3),
        lankide_computer_daemon::run_with_harnesses(config, vec![]),
    )
    .await?
    .expect_err("unknown Computer must stop");
    assert!(error.to_string().contains("computer_unknown"));
    rig.close().await
}
