use super::ExecutionLiveness;
use lankide_harness::{AgentEventKind, FailureCategory, HarnessFailure, HarnessSession};
use std::sync::Arc;
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};

/// Runs independently of event persistence and Tool Result delivery.
pub(super) async fn monitor(
    policy: ExecutionLiveness,
    started: Instant,
    session: Arc<dyn HarnessSession>,
    mut progress: watch::Receiver<Instant>,
    mut cancel: watch::Receiver<Option<String>>,
    cancelling: watch::Sender<Option<String>>,
    terminated: watch::Sender<Option<AgentEventKind>>,
) {
    let total = started + policy.total_budget;
    let (reason, timeout) = loop {
        let idle = *progress.borrow_and_update() + policy.idle_timeout;
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() { return; }
                if let Some(reason) = cancel.borrow().clone() { break (reason, false); }
            }
            _ = sleep_until(total) => break ("Attempt total budget exceeded".into(), true),
            changed = progress.changed() => { if changed.is_err() { return; } }
            _ = sleep_until(idle) => break ("Harness idle timeout exceeded".into(), true),
        }
    };
    cancelling.send_replace(Some(reason.clone()));
    let group = session.process_group_id();
    // The cancel call shares the grace deadline; a stuck adapter cannot extend it.
    let deadline = Instant::now() + policy.cancel_grace;
    let _ = tokio::time::timeout_at(deadline, session.cancel(reason.clone())).await;
    sleep_until(deadline).await;
    #[cfg(unix)]
    if let Some(group) = group.and_then(|group| rustix::process::Pid::from_raw(group as i32)) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    #[cfg(not(unix))]
    let _ = group;
    let _ = tokio::time::timeout(policy.cancel_grace, session.close()).await;
    let kind = if timeout {
        AgentEventKind::Failed {
            code: "harness_timeout".into(),
            message: reason.clone(),
            failure: Some(HarnessFailure {
                code: "harness_timeout".into(),
                category: FailureCategory::Timeout,
                retryable: true,
                detail: reason,
            }),
        }
    } else {
        AgentEventKind::Cancelled { reason }
    };
    terminated.send_replace(Some(kind));
}

/// Cleanup is complete before a terminal outcome is published.
pub(super) async fn wait_for_stop(
    mut terminated: watch::Receiver<Option<AgentEventKind>>,
    grace: std::time::Duration,
) {
    if terminated.borrow().is_none() {
        let _ = tokio::time::timeout(
            grace + std::time::Duration::from_secs(1),
            terminated.changed(),
        )
        .await;
    }
}
