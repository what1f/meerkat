use super::*;
use lankide_harness::{AgentEventKind, ExecuteRequest, PermissionPolicy, SessionRequest};
use lankide_protocol::{HarnessTestState, TestHarnessResponse, machine_frame};
use tokio_stream::StreamExt;

pub(super) async fn run(
    inner: Arc<Inner>,
    request: lankide_protocol::link::TestHarness,
    received_at: tokio::time::Instant,
) {
    let budget = Duration::from_millis(request.budget_millis).min(Duration::from_secs(29));
    let deadline = received_at + budget.saturating_sub(Duration::from_millis(500));
    let result = if tokio::time::Instant::now() >= deadline {
        outcome(HarnessTestState::TimedOut, None, None)
    } else {
        test(&inner, &request, deadline).await
    };
    let sender = inner.connection.read().await.clone();
    if let Some(sender) = sender {
        let _ = sender
            .send(MachineFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::TestHarnessResult(
                    lankide_protocol::link::TestHarnessResult {
                        request_id: request.request_id,
                        result,
                    },
                )),
            })
            .await;
    }
}

fn outcome(
    state: HarnessTestState,
    reply: Option<String>,
    error_code: Option<String>,
) -> TestHarnessResponse {
    TestHarnessResponse {
        state,
        reply,
        error_code,
    }
}

async fn test(
    inner: &Inner,
    request: &lankide_protocol::link::TestHarness,
    deadline: tokio::time::Instant,
) -> TestHarnessResponse {
    use HarnessTestState::*;
    let mut session: Option<Box<dyn HarnessSession>> = None;
    let mut directory = None;
    let operation = async {
        let wire = &request.harness;
        let factory = inner
            .factories
            .get(&wire.harness_id)
            .ok_or_else(|| MachineError::InvalidAssignment("harness_unavailable".into()))?;
        let defaults = inner
            .defaults
            .get(&wire.harness_id)
            .ok_or_else(|| MachineError::InvalidAssignment("harness_unavailable".into()))?;
        let mut config = super::assignment::native_config(wire, "harness-test", defaults)?;
        config
            .environment
            .retain(|key, _| !key.starts_with("LANKIDE_RUNTIME_"));
        directory = Some(
            tempfile::Builder::new()
                .prefix("lankide-harness-test-")
                .tempdir()?,
        );
        session = Some(
            factory
                .start_before(
                    SessionRequest {
                        permission_policy: PermissionPolicy::FullAccess,
                        config,
                        working_directory: directory
                            .as_ref()
                            .expect("test directory")
                            .path()
                            .to_path_buf(),
                        system_prompt: "Reply briefly to the greeting. Do not use tools.".into(),
                        tools: Vec::new(),
                        native_session_id: None,
                    },
                    deadline.into_std(),
                )
                .await?,
        );
        if tokio::time::Instant::now() >= deadline {
            return Ok(outcome(TimedOut, None, None));
        }
        let mut events = session
            .as_ref()
            .expect("test session")
            .execute(ExecuteRequest {
                input: "Hello! Please reply with a short greeting.".into(),
            })
            .await?;
        while let Some(event) = events.next().await {
            match event?.kind {
                AgentEventKind::Completed { final_output } if !final_output.trim().is_empty() => {
                    return Ok(outcome(Passed, Some(final_output), None));
                }
                AgentEventKind::Completed { .. } => {
                    return Ok(outcome(Failed, None, Some("empty_reply".into())));
                }
                AgentEventKind::Failed { code, .. } => {
                    return Ok(outcome(Failed, None, Some(code)));
                }
                AgentEventKind::Cancelled { .. } => {
                    return Ok(outcome(Failed, None, Some("cancelled".into())));
                }
                _ => {}
            }
        }
        Ok::<_, MachineError>(outcome(Failed, None, Some("stream_interrupted".into())))
    };
    let result = match tokio::time::timeout_at(deadline, operation).await {
        Ok(Ok(result)) => result,
        Ok(Err(MachineError::Harness(error))) => {
            if error.code == lankide_harness::HarnessErrorCode::Timeout {
                outcome(TimedOut, None, None)
            } else {
                let code = serde_json::to_value(error.code).expect("error code");
                outcome(
                    Failed,
                    None,
                    Some(code.as_str().expect("error code string").into()),
                )
            }
        }
        Ok(Err(_)) => outcome(Failed, None, Some("invalid_config".into())),
        Err(_) => outcome(TimedOut, None, None),
    };
    if let Some(session) = session {
        // Kill the entire group even when its leader already exited. Native close
        // then reaps the leader and releases adapter resources.
        if let Some(group) = session.process_group_id() {
            lankide_harness::kill_process_group(group);
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), session.close()).await;
    }
    drop(directory);
    result
}
