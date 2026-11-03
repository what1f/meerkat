use meerkat_daemon_runtime::{DaemonRuntimeError, InstanceGuard, create_private_directory};
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn daemon_filesystem_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let run = temporary.path().join("run");
    let lock = run.join("runtime.lock");
    create_private_directory(&run)?;
    #[cfg(unix)]
    assert_eq!(fs::metadata(&run)?.permissions().mode() & 0o777, 0o700);

    let guard = InstanceGuard::acquire(&run, &lock, "first")?;
    assert!(matches!(
        InstanceGuard::acquire(&run, &lock, "second"),
        Err(DaemonRuntimeError::AlreadyRunning)
    ));
    drop(guard);
    assert!(!lock.exists());

    #[cfg(unix)]
    {
        let mut exited = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()?;
        let stale_pid = exited.id();
        exited.wait()?;
        fs::write(&lock, format!("pid={stale_pid} run_id=stale\n"))?;
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o600))?;
        drop(InstanceGuard::acquire(&run, &lock, "recovered")?);
        assert!(!lock.exists());
    }

    fs::create_dir(&lock)?;
    assert!(matches!(
        InstanceGuard::acquire(&run, &lock, "unsafe"),
        Err(DaemonRuntimeError::UnsafeInstanceLock)
    ));
    Ok(())
}
