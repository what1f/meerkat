use crate::backend;
use std::time::{Duration, Instant};

/// Helper lifecycle scenarios each start a full set of local services; one at
/// a time keeps the timing of the other Desktop scenarios intact.
static HELPER_LIFECYCLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run the public Desktop bootstrap and service cleanup with real helpers.
/// A subprocess provides private environment variables without mutating this test process.
#[test]
fn owned_helpers_stop_on_cleanup_while_reused_helpers_stay_running() {
    const CHILD: &str = "LANKIDE_QUIT_E2E_CHILD";
    const NAME: &str = concat!(
        module_path!(),
        "::owned_helpers_stop_on_cleanup_while_reused_helpers_stay_running"
    );
    if std::env::var_os(CHILD).is_none() {
        let _serial = HELPER_LIFECYCLE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let directory = tempfile::Builder::new()
            .prefix("lkq-")
            .tempdir_in("/tmp")
            .unwrap();
        let current = std::env::current_exe().unwrap();
        let binaries = current.parent().unwrap().parent().unwrap();
        let executable = directory.path().join("quit-e2e");
        std::fs::hard_link(&current, &executable).unwrap();
        for name in ["lankided", "lankide-machine"] {
            std::fs::hard_link(binaries.join(name), directory.path().join(name)).unwrap();
        }
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("runtime.json"),
            r#"{"machine_link_listen":"127.0.0.1:0","remote_access_port":0}"#,
        )
        .unwrap();
        let output = std::process::Command::new(executable)
            .args(["--exact", NAME.split_once("::").unwrap().1, "--nocapture"])
            .env(CHILD, "1")
            .env("LANKIDE_HOME", &home)
            .env("HOME", &home)
            .env_remove("LANKIDE_RUNTIME_RUN")
            .env("SHELL", "/bin/sh")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let owner = backend::DesktopBackend::bootstrap();
    assert!(owner.startup_error.is_none());
    let pids: Vec<String> = ["runtime.lock", "machine.lock"]
        .into_iter()
        .map(|name| {
            let lock = std::fs::read_to_string(owner.local_home.join("run").join(name)).unwrap();
            lock.split_whitespace()
                .find_map(|part| part.strip_prefix("pid="))
                .unwrap()
                .to_owned()
        })
        .collect();
    let reused = backend::DesktopBackend::bootstrap();
    assert!(reused.startup_error.is_none());
    reused.stop_local_services();
    for pid in &pids {
        assert!(
            std::process::Command::new("kill")
                .args(["-0", pid])
                .status()
                .unwrap()
                .success(),
            "stopped a reused helper"
        );
    }
    owner.stop_local_services();
    for pid in &pids {
        assert!(
            !std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success(),
            "owned helper survived cleanup"
        );
    }
    assert!(!owner.local_home.join("run/runtime.lock").exists());
    assert!(!owner.local_home.join("run/machine.lock").exists());
}

/// Reopening Desktop after an update replaces helpers still running the old
/// build instead of reusing them.
#[test]
fn reopening_after_an_update_replaces_outdated_helpers() {
    const CHILD: &str = "LANKIDE_UPDATE_E2E_CHILD";
    const NAME: &str = concat!(
        module_path!(),
        "::reopening_after_an_update_replaces_outdated_helpers"
    );
    if std::env::var_os(CHILD).is_none() {
        let _serial = HELPER_LIFECYCLE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let directory = tempfile::Builder::new()
            .prefix("lku-")
            .tempdir_in("/tmp")
            .unwrap();
        let current = std::env::current_exe().unwrap();
        let binaries = current.parent().unwrap().parent().unwrap();
        let executable = directory.path().join("update-e2e");
        std::fs::hard_link(&current, &executable).unwrap();
        for name in ["lankided", "lankide-machine"] {
            std::fs::hard_link(binaries.join(name), directory.path().join(name)).unwrap();
        }
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("runtime.json"),
            r#"{"machine_link_listen":"127.0.0.1:0","remote_access_port":0}"#,
        )
        .unwrap();
        let output = std::process::Command::new(executable)
            .args(["--exact", NAME.split_once("::").unwrap().1, "--nocapture"])
            .env(CHILD, "1")
            .env("LANKIDE_HOME", &home)
            .env("HOME", &home)
            .env_remove("LANKIDE_RUNTIME_RUN")
            .env("SHELL", "/bin/sh")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let runtime_pid = |home: &std::path::Path| {
        std::fs::read_to_string(home.join("run/runtime.lock"))
            .unwrap()
            .split_whitespace()
            .find_map(|part| part.strip_prefix("pid=").map(str::to_owned))
            .unwrap()
    };
    // Helpers run on their own, as launchd keeps them, not as Desktop children.
    let directory = std::env::current_exe().unwrap();
    let directory = directory.parent().unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("LANKIDE_HOME").unwrap());
    for name in ["lankided", "lankide-machine"] {
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("\"$0\" --home \"$1\" >/dev/null 2>&1 &")
            .arg(directory.join(name))
            .arg(&home)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !home.join("run/runtime.lock").exists()
        || !home.join("run/management.url").exists()
        || !home.join("run/machine.lock").exists()
    {
        assert!(Instant::now() < deadline, "the Runtime did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let outdated = runtime_pid(&home);
    struct StopDetached(String);
    impl Drop for StopDetached {
        fn drop(&mut self) {
            let _ = std::process::Command::new("kill").arg(&self.0).status();
        }
    }
    let _old_runtime = StopDetached(outdated.clone());
    let _old_machine = StopDetached(
        std::fs::read_to_string(home.join("run/machine.lock"))
            .unwrap_or_default()
            .split_whitespace()
            .find_map(|part| part.strip_prefix("pid=").map(str::to_owned))
            .unwrap_or_default(),
    );

    // An update replaces the bundled helpers while the old Runtime keeps running.
    std::thread::sleep(Duration::from_millis(20));
    for name in ["lankided", "lankide-machine"] {
        let staged = directory.join(format!(".{name}.update"));
        std::fs::write(&staged, std::fs::read(directory.join(name)).unwrap()).unwrap();
        std::fs::set_permissions(
            &staged,
            std::fs::metadata(directory.join(name))
                .unwrap()
                .permissions(),
        )
        .unwrap();
        std::fs::rename(&staged, directory.join(name)).unwrap();
    }

    let reopened = backend::DesktopBackend::bootstrap();
    assert!(
        reopened.startup_error.is_none(),
        "{:?}",
        reopened.startup_error
    );
    assert_ne!(runtime_pid(&reopened.local_home), outdated);
    assert!(
        !std::process::Command::new("kill")
            .args(["-0", &outdated])
            .status()
            .unwrap()
            .success(),
        "the outdated Runtime kept running"
    );
    reopened.stop_local_services();
}
