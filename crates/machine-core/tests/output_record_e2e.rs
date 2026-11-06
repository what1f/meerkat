use meerkat_machine_core::{Machine, MachineConfig, OutputRecordPolicy, ReconnectPolicy};
use std::fs::{self, File, FileTimes};
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn machine_start_expires_records_and_preserves_sessions()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let expired = root.path().join(".machine/diagnostics/1-1");
    let recent = root.path().join(".machine/diagnostics/2-1");
    for directory in [&expired, &recent] {
        fs::create_dir_all(directory)?;
    }
    let old = SystemTime::now() - Duration::from_secs(60);
    for path in [
        expired.join("rpc.jsonl"),
        expired.join("rpc.jsonl.1"),
        expired.join("diagnostics.json"),
    ] {
        fs::write(&path, "old record")?;
        File::open(path)?.set_times(FileTimes::new().set_modified(old))?;
    }
    fs::write(recent.join("rpc.jsonl"), "recent record")?;
    let session = root.path().join(".meerkat/pi/sessions/current.jsonl");
    fs::create_dir_all(session.parent().unwrap())?;
    fs::write(&session, "native session")?;
    File::open(&session)?.set_times(FileTimes::new().set_modified(old))?;
    // Cleanup must not follow links to unrelated data.
    let outside = tempfile::tempdir()?;
    let external = outside.path().join("rpc.jsonl");
    fs::write(&external, "external record")?;
    File::open(&external)?.set_times(FileTimes::new().set_modified(old))?;
    std::os::unix::fs::symlink(
        outside.path(),
        root.path().join(".machine/diagnostics/link"),
    )?;
    let handle = Machine::new(MachineConfig {
        machine_id: "retention-machine".into(),
        instance_id: "retention-instance".into(),
        workspace_root: root.path().display().to_string(),
        reconnect: ReconnectPolicy::default(),
    })
    .with_output_record_policy(OutputRecordPolicy {
        max_bytes: 2048,
        retention: Duration::from_secs(30),
    })
    .spawn(root.path().join("run"))
    .await?;
    assert!(!expired.exists());
    assert_eq!(
        fs::read_to_string(recent.join("rpc.jsonl"))?,
        "recent record"
    );
    assert_eq!(fs::read_to_string(external)?, "external record");
    assert_eq!(fs::read_to_string(session)?, "native session");
    handle.shutdown("scenario_complete").await;
    handle.join().await?;
    Ok(())
}
