use meerkat_client::ManagementClient;
use meerkat_machine::{MachineDaemonPaths, pair, read_credentials};
use meerkat_protocol::link::{self, CloseReason, LinkError, MachineLink, MachinePayload};
use meerkat_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        auto_start_machine: false,
        ..Default::default()
    }
}
async fn pair_rig(rig: &SystemRig) -> Result<meerkat_machine::DeviceCredentials> {
    let code = rig.management().create_pairing_code().await?;
    let (url, _) = rig.local_pairing_endpoint()?;
    Ok(pair(
        &MachineDaemonPaths::for_home(rig.machine_home())?,
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
                .get_machine_catalog(None)
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
    assert_ne!(credentials.machine_id, "fixture-machine");
    assert_eq!(credentials.device_credential.len(), 64);
    let file = rig.machine_home().join("machine-credentials.json");
    assert_eq!(
        std::fs::metadata(&file)?.permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        read_credentials(rig.machine_home())?.unwrap().runtime_url,
        credentials.runtime_url
    );
    let db = rusqlite::Connection::open(rig.runtime_home().join("runtime.db"))?;
    let hash: String = db.query_row(
        "SELECT credential_hash FROM machines WHERE machine_id=?1",
        [&credentials.machine_id],
        |r| r.get(0),
    )?;
    assert_ne!(hash, credentials.device_credential);
    drop(db);
    rig.start_paired_machine()?;
    rig.wait_machine_online().await?;
    let catalog = rig.management().get_machine_catalog(None).await?;
    let device = catalog
        .iter()
        .find(|c| c.id == credentials.machine_id)
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
            .machine_id,
        credentials.machine_id
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
        &MachineDaemonPaths::for_home(rig.machine_home())?,
        &url,
        &code.code,
    )
    .await?;
    let other = tempfile::tempdir()?;
    assert!(
        pair(
            &MachineDaemonPaths::for_home(other.path())?,
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
            &MachineDaemonPaths::for_home(other.path())?,
            &url,
            &expired.code
        )
        .await
        .is_err()
    );
    assert!(!other.path().join("machine-credentials.json").exists());
    assert_eq!(rig.management().get_machine_catalog(None).await?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn s12_3_stopped_machine_stays_registered_with_inventory() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.start_paired_machine()?;
    rig.wait_machine_online().await?;
    rig.stop_machine()?;
    wait_offline(&rig, &device.machine_id).await?;
    let catalog = rig.management().get_machine_catalog(None).await?;
    let machine = &catalog[0];
    assert!(machine.registered && !machine.available && machine.revoked_at.is_none());
    assert!(machine.last_seen_at.is_some());
    assert!(machine.inventory.iter().any(|h| h.harness_id == "pi"));
    rig.start_paired_machine()?;
    rig.wait_machine_online().await?;
    assert_eq!(rig.management().get_machine_catalog(None).await?.len(), 1);
    rig.close().await
}

#[tokio::test]
async fn s12_4_revocation_closes_link_and_stops_daemon() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.start_paired_machine()?;
    rig.wait_machine_online().await?;
    rig.management().revoke_machine(&device.machine_id).await?;
    assert!(
        rig.await_terminal_machine()
            .await?
            .contains("machine_revoked")
    );
    wait_offline(&rig, &device.machine_id).await?;
    let catalog = rig.management().get_machine_catalog(None).await?;
    assert!(catalog[0].registered && catalog[0].revoked_at.is_some() && !catalog[0].available);
    rig.close().await
}

#[tokio::test]
async fn s12_5_copied_revoked_home_cannot_connect() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.management().revoke_machine(&device.machine_id).await?;
    let copied = tempfile::tempdir()?;
    std::fs::copy(
        rig.machine_home().join("machine-credentials.json"),
        copied.path().join("machine-credentials.json"),
    )?;
    let mut daemon = std::process::Command::new(env!("CARGO_BIN_EXE_system-machine"))
        .arg(copied.path())
        .arg(&device.runtime_url)
        .arg(copied.path().join("unused-secret"))
        .arg(rig.runtime_run())
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
        String::from_utf8_lossy(&output.stderr).contains("machine_revoked"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!rig.machine_online().await?);
    rig.close().await
}

#[tokio::test]
async fn local_self_pairing_places_new_agents_and_reuses_credentials() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        local_discovery: true,
        ..config()
    })
    .await?;
    rig.workspace("team").await?;
    rig.restart_runtime().await?;
    let team = rig.existing_workspace("team");
    let credentials = read_credentials(rig.machine_home())?.unwrap();
    assert!(credentials.local_discovery);
    assert!(matches!(
        rig.management()
            .revoke_machine(&credentials.machine_id)
            .await,
        Err(meerkat_client::ClientError::LocalMachineCannotBeRemoved)
    ));
    assert!(rig.management().get_machine_catalog(None).await?[0].available);
    let agent = team
        .agent("bruce")
        .harness(Script::reply("hi"))
        .create()
        .await?;
    assert_eq!(
        rig.management()
            .get_agent("team", "bruce")
            .await?
            .machine_id,
        credentials.machine_id
    );
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "hi"]);
    rig.restart_runtime().await?;
    assert_eq!(
        read_credentials(rig.machine_home())?
            .unwrap()
            .device_credential,
        credentials.device_credential
    );
    assert_eq!(rig.management().get_machine_catalog(None).await?.len(), 1);
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
            meerkat_protocol::DEFAULT_MAX_MESSAGE_BYTES
        )
        .await,
        Err(LinkError::Closed(CloseReason::MachineUnknown))
    ));
    let mut socket = link::connect(
        &url,
        &device.device_credential,
        meerkat_protocol::DEFAULT_MAX_MESSAGE_BYTES,
    )
    .await?;
    socket
        .send(link::MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(MachinePayload::Hello(link::MachineHello {
                protocol_version: meerkat_protocol::PROTOCOL_VERSION,
                machine_id: "unknown".into(),
                device_credential: device.device_credential.clone(),
                instance_id: "one".into(),
                machine_version: "1".into(),
                os: "test".into(),
                arch: "test".into(),
                ..Default::default()
            })),
        })
        .await?;
    assert!(matches!(
        timeout(Duration::from_secs(3), socket.receive()).await?,
        Err(LinkError::Closed(CloseReason::MachineUnknown))
    ));
    let unknown_home = tempfile::tempdir()?;
    let unknown = meerkat_machine::DeviceCredentials {
        cert_sha256: None,
        machine_id: "unknown".into(),
        ..device
    };
    std::fs::write(
        unknown_home.path().join("machine-credentials.json"),
        serde_json::to_vec(&unknown)?,
    )?;
    let config = meerkat_machine::DaemonConfig::for_home(unknown_home.path())?;
    let error = timeout(
        Duration::from_secs(3),
        meerkat_machine::run_with_harnesses(config, vec![]),
    )
    .await?
    .expect_err("unknown Machine must stop");
    assert!(error.to_string().contains("machine_unknown"));
    rig.close().await
}

#[tokio::test]
async fn owner_renames_machine_and_name_survives_daemon_and_runtime_restart() -> Result<()> {
    use meerkat_client::{ClientError, ManagementEventKind};
    let mut rig = SystemRig::start(config()).await?;
    let device = pair_rig(&rig).await?;
    rig.start_paired_machine()?;
    rig.wait_machine_online().await?;
    let client = rig.management().clone();
    let mut events = client.subscribe_management_events(None, 0).await?;
    client
        .rename_machine(&device.machine_id, "  Studio Mac  ")
        .await?;
    loop {
        let event = timeout(Duration::from_secs(5), events.next())
            .await?
            .ok_or("events ended")??;
        if event.kind == ManagementEventKind::MachineCatalogChanged {
            break;
        }
    }
    assert_eq!(
        client.get_machine_catalog(None).await?[0].name,
        "Studio Mac"
    );
    for name in [" ".to_owned(), "a".repeat(81)] {
        assert!(matches!(
            client.rename_machine(&device.machine_id, &name).await,
            Err(ClientError::MachineNameInvalid)
        ));
    }
    rig.stop_machine()?;
    wait_offline(&rig, &device.machine_id).await?;
    client
        .rename_machine(&device.machine_id, "Office Mac")
        .await?;
    rig.restart_runtime().await?;
    rig.wait_machine_online().await?;
    let catalog = client.get_machine_catalog(None).await?;
    assert_eq!(catalog[0].name, "Office Mac");
    client.revoke_machine(&device.machine_id).await?;
    assert!(
        rig.await_terminal_machine()
            .await?
            .contains("machine_revoked")
    );
    assert!(
        client
            .rename_machine(&device.machine_id, "Removed")
            .await
            .is_err()
    );
    rig.close().await
}
