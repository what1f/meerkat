use lankide_testkit::{GateSelection, ProbeConfig, ScenarioArtifact, run_prerequisite_probe};
use std::{
    future::Future,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub fn preflight(gate: GateSelection) {
    assert_eq!(std::env::var("LANKIDE_E2E_LIVE").as_deref(), Ok("1"));
    let config = ProbeConfig::from_environment(gate, true);
    let report = run_prerequisite_probe(&config);
    assert!(report.is_green(), "{}", report.render_text());
}

pub fn runtime_binary() -> PathBuf {
    if let Some(directory) = std::env::var_os("LANKIDE_RELEASE_BIN_DIR") {
        PathBuf::from(directory).join("lankided")
    } else {
        assert_ne!(
            std::env::var("LANKIDE_RELEASE_REQUIRED").as_deref(),
            Ok("1")
        );
        PathBuf::from(env!("CARGO_BIN_EXE_lankided"))
    }
}

/// Join failures (including panics) and timeouts still reach the caller's cleanup.
pub async fn scenario(
    future: impl Future<Output = ()> + Send + 'static,
    timeout: Duration,
) -> Result<(), String> {
    let mut task = tokio::spawn(future);
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err("live composition scenario panicked; see private log".into()),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err("live composition scenario timed out".into())
        }
    }
}

pub struct Daemon {
    child: Child,
    stopped: bool,
}

impl Daemon {
    pub fn spawn(command: &mut Command) -> Self {
        use std::os::unix::process::CommandExt;
        Self {
            child: command
                .process_group(0)
                .spawn()
                .expect("start owned daemon"),
            stopped: false,
        }
    }

    fn stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        let group = format!("-{}", self.child.id());
        let _ = Command::new("kill")
            .args(["-TERM", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() && !group_exists(&group) {
                self.stopped = true;
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = Command::new("kill")
            .args(["-KILL", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.child.wait();
        self.stopped = true;
        // A forced kill is a cleanup failure even when it removes the process.
        false
    }

    pub fn cleanup(&mut self, artifact: &ScenarioArtifact, label: &str) {
        let succeeded = self.stop();
        artifact.record_cleanup(label, &self.child.id().to_string(), succeeded);
    }
}

fn group_exists(group: &str) -> bool {
    Command::new("kill")
        .args(["-0", "--", group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(true)
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}
