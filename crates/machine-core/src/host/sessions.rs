use super::Inner;
use meerkat_harness::{HarnessSession, SessionHealth, SessionRequest};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct IdleSession {
    pub session: Arc<dyn HarnessSession>,
    pub identity: String,
    idle_since: tokio::time::Instant,
}

pub(super) fn identity(request: &SessionRequest, session_key: &str, soul: &str) -> String {
    let mut request = request.clone();
    request.native_session_id = None;
    request.config.environment.retain(|key, _| {
        !key.starts_with("MEERKAT_EXECUTION_")
            && !key.starts_with("MEERKAT_PI_RECORD_")
            && key != "MEERKAT_PI_ARTIFACT_DIR"
            && key != "MEERKAT_AGENT_SOCKET"
    });
    serde_json::json!([session_key, soul, request]).to_string()
}

pub(super) async fn take(
    inner: &Inner,
    agent_key: &str,
    identity: &str,
    native_id: Option<&str>,
    request: &SessionRequest,
) -> Option<Arc<dyn HarnessSession>> {
    let cached = inner.idle_sessions.lock().await.remove(agent_key)?;
    let entry = cached.session.entry();
    if cached.identity == identity
        && native_id == Some(entry.native_session_id.as_str())
        && entry.health == SessionHealth::Healthy
        && cached.session.rebind(request.clone()).await.is_ok()
    {
        Some(cached.session)
    } else {
        let _ = cached.session.close().await;
        None
    }
}

pub(super) async fn retain(
    inner: &Arc<Inner>,
    agent_key: String,
    identity: String,
    session: Arc<dyn HarnessSession>,
) {
    let idle_since = tokio::time::Instant::now();
    let previous = inner.idle_sessions.lock().await.insert(
        agent_key.clone(),
        IdleSession {
            session,
            identity,
            idle_since,
        },
    );
    if let Some(previous) = previous {
        let _ = previous.session.close().await;
    }
    let weak = Arc::downgrade(inner);
    let timeout = inner.session_idle_timeout;
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        let Some(inner) = weak.upgrade() else { return };
        let expired = {
            let mut sessions = inner.idle_sessions.lock().await;
            if sessions
                .get(&agent_key)
                .is_some_and(|entry| entry.idle_since == idle_since)
            {
                sessions.remove(&agent_key)
            } else {
                None
            }
        };
        if let Some(expired) = expired {
            let _ = expired.session.close().await;
        }
    });
}

pub(super) async fn close(inner: &Inner, agent_key: &str) {
    let cached = inner.idle_sessions.lock().await.remove(agent_key);
    if let Some(cached) = cached {
        let _ = cached.session.close().await;
    }
}

pub(super) async fn close_all(inner: &Inner) {
    let sessions = std::mem::take(&mut *inner.idle_sessions.lock().await);
    for cached in sessions.into_values() {
        let _ = cached.session.close().await;
    }
}

pub(super) const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
