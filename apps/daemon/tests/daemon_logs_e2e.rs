//! Startup failures remain available even when launchd discards both streams.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};

#[test]
fn failed_daemon_start_keeps_private_bounded_diagnostics() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("runtime.json"), "invalid configuration").unwrap();
    let directory = home.path().join("logs");
    fs::create_dir(&directory).unwrap();
    let log = directory.join("meerkatd.log");
    // A previous run has filled the log. The next diagnostic rotates it once.
    fs::write(&log, vec![b'x'; 4 * 1024 * 1024]).unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_meerkatd"))
        .arg("--home")
        .arg(home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let text = fs::read_to_string(&log).unwrap();
    assert!(text.contains("runtime-daemon could not start"), "{text}");
    assert!(text.contains("json"), "{text}");
    assert!(text.lines().all(|line| {
        line.split_whitespace()
            .next()
            .unwrap()
            .trim_end_matches('Z')
            .parse::<f64>()
            .is_ok()
    }));
    for path in [&log, &log.with_extension("log.1")] {
        let metadata = fs::metadata(path).unwrap();
        assert!(metadata.len() <= 4 * 1024 * 1024);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }
}
