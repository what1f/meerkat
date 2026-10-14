use lankide_computer::{Computer, ComputerConfig, OutputRecordPolicy, ReconnectPolicy};
use std::fs::{self, File, FileTimes};
use std::time::{Duration, SystemTime};

#[tokio::test]
async fn computer_start_expires_records_and_preserves_sessions()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let expired = root.path().join(".computer/diagnostics/1-1");
    let recent = root.path().join(".computer/diagnostics/2-1");
    let legacy = root.path().join("team/agents/agent/.lankide/pi/sessions");
    for directory in [&expired, &recent, &legacy] {
        fs::create_dir_all(directory)?;
    }
    let old = SystemTime::now() - Duration::from_secs(60);
    for path in [
        expired.join("rpc.jsonl"),
        expired.join("rpc.jsonl.1"),
        expired.join("diagnostics.json"),
        legacy.join("pi-old.rpc.jsonl"),
    ] {
        fs::write(&path, "old record")?;
        File::open(path)?.set_times(FileTimes::new().set_modified(old))?;
    }
    let native = legacy.join("native-session.jsonl");
    fs::write(&native, "native session")?;
    File::open(&native)?.set_times(FileTimes::new().set_modified(old))?;
    fs::write(recent.join("rpc.jsonl"), "recent record")?;
    // Cleanup must not follow links to unrelated data.
    let outside = tempfile::tempdir()?;
    let external = outside.path().join("rpc.jsonl");
    fs::write(&external, "external record")?;
    File::open(&external)?.set_times(FileTimes::new().set_modified(old))?;
    std::os::unix::fs::symlink(
        outside.path(),
        root.path().join(".computer/diagnostics/link"),
    )?;
    let linked_agent = root.path().join("other/agents/linked");
    fs::create_dir_all(&linked_agent)?;
    let external_sessions = outside.path().join("pi/sessions");
    fs::create_dir_all(&external_sessions)?;
    let external_legacy = external_sessions.join("pi-old.rpc.jsonl");
    fs::write(&external_legacy, "external legacy record")?;
    File::open(&external_legacy)?.set_times(FileTimes::new().set_modified(old))?;
    std::os::unix::fs::symlink(outside.path(), linked_agent.join(".lankide"))?;
    let handle = Computer::new(ComputerConfig {
        computer_id: "retention-computer".into(),
        instance_id: "retention-instance".into(),
        workspace_root: root.path().display().to_string(),
        reconnect: ReconnectPolicy::default(),
    })
    .with_output_record_policy(OutputRecordPolicy {
        max_bytes: 2048,
        retention: Duration::from_secs(30),
    })
    .spawn(root.path().join("runtime.sock"))
    .await?;
    assert!(!expired.exists());
    assert!(!legacy.join("pi-old.rpc.jsonl").exists());
    assert_eq!(
        fs::read_to_string(recent.join("rpc.jsonl"))?,
        "recent record"
    );
    assert_eq!(fs::read_to_string(native)?, "native session");
    assert_eq!(fs::read_to_string(external)?, "external record");
    assert_eq!(
        fs::read_to_string(external_legacy)?,
        "external legacy record"
    );
    handle.shutdown("scenario_complete").await;
    handle.join().await?;
    Ok(())
}
