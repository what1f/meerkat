use super::connection::send_frame;
use super::control::prepare_tool_arguments;
use super::{Inner, now_millis};
use crate::state::{AssignmentLifecycle, PersistedEvent, SessionRecovery};
use lankide_harness::{
    AgentEvent, AgentEventKind, ExecuteRequest, HarnessSession, ToolResultError, ToolResultRequest,
};
use lankide_protocol::{
    AgentRunContext, ComputerFrame, ExecutionEvent, ExecutionEventKind, ExecutionRef, ToolCall,
    computer_frame,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio_stream::StreamExt;

const COMPLETION_ACKNOWLEDGEMENT: &str = "OK";
const MAX_COMPLETION_CORRECTIONS: usize = 3;
const COMPLETION_PROTOCOL_CORRECTION: &str = r#"<system-reminder>
上一条回复不符合结束协议。需要回复或执行操作时，请使用相应工具；不要重复已成功执行的操作。确认本轮全部完成后，只回复 OK。
</system-reminder>"#;

pub(super) async fn run_events(
    inner: Arc<Inner>,
    key: String,
    session_key: String,
    execution: ExecutionRef,
    context: AgentRunContext,
    agent_home: PathBuf,
    session: Arc<dyn HarnessSession>,
    mut stream: lankide_harness::AgentEventStream,
) {
    let mut last_sequence = 0_u64;
    let mut stream_sequence = 0_u64;
    let mut corrections = 0_usize;
    let mut terminal_event = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(mut event) if event.sequence > stream_sequence => {
                stream_sequence = event.sequence;
                if let AgentEventKind::Completed { final_output } = &event.kind
                    && final_output != COMPLETION_ACKNOWLEDGEMENT
                {
                    if corrections >= MAX_COMPLETION_CORRECTIONS {
                        terminal_event = Some(synthetic_event(
                            last_sequence + 1,
                            AgentEventKind::Failed {
                                code: "completion_protocol".into(),
                                message: "Agent did not finish this run correctly".into(),
                            },
                        ));
                        break;
                    }
                    corrections += 1;
                    match session
                        .execute(ExecuteRequest {
                            input: COMPLETION_PROTOCOL_CORRECTION.into(),
                        })
                        .await
                    {
                        Ok(next_stream) => {
                            stream = next_stream;
                            stream_sequence = 0;
                            continue;
                        }
                        Err(error) => {
                            terminal_event = Some(synthetic_event(
                                last_sequence + 1,
                                AgentEventKind::Failed {
                                    code: format!("{:?}", error.code).to_lowercase(),
                                    message: error.message,
                                },
                            ));
                            break;
                        }
                    }
                }

                last_sequence += 1;
                event.sequence = last_sequence;
                if let Some(active) = inner.active.lock().await.get_mut(&key) {
                    active.last_event_sequence = last_sequence;
                }
                if let AgentEventKind::ToolStarted {
                    tool_call_id,
                    tool_name,
                    arguments,
                } = &event.kind
                {
                    match prepare_tool_arguments(tool_name, arguments, &agent_home) {
                        Ok(arguments) => {
                            inner
                                .outstanding_tools
                                .lock()
                                .await
                                .insert((key.clone(), tool_call_id.clone()));
                            send_frame(
                                &inner,
                                ComputerFrame {
                                    frame: Some(computer_frame::Frame::ToolCall(ToolCall {
                                        execution: Some(execution.clone()),
                                        tool_call_id: tool_call_id.clone(),
                                        tool_name: tool_name.clone(),
                                        arguments_json: arguments.to_string(),
                                        execution_token: context.execution_token.clone(),
                                    })),
                                },
                            )
                            .await;
                        }
                        Err((code, message)) => {
                            let _ = session
                                .tool_result(ToolResultRequest {
                                    tool_call_id: tool_call_id.clone(),
                                    success: false,
                                    content_markdown: String::new(),
                                    structured_content: serde_json::json!({}),
                                    resources: Vec::new(),
                                    error: Some(ToolResultError {
                                        code: code.into(),
                                        message: message.into(),
                                        retryable: false,
                                    }),
                                })
                                .await;
                        }
                    }
                }
                if let AgentEventKind::ToolFinished { tool_call_id, .. } = &event.kind {
                    inner
                        .outstanding_tools
                        .lock()
                        .await
                        .remove(&(key.clone(), tool_call_id.clone()));
                }
                if event.is_terminal() {
                    terminal_event = Some(event);
                    break;
                }
                if let Err(error) = emit_event(&inner, &key, &execution, &event).await {
                    stop_after_persistence_failure(&inner, &error);
                    return;
                }
            }
            Ok(_) => {
                terminal_event = Some(synthetic_event(
                    last_sequence + 1,
                    AgentEventKind::Failed {
                        code: "invalid_sequence".into(),
                        message: "Harness event sequence was not monotonic".into(),
                    },
                ));
                break;
            }
            Err(error) => {
                terminal_event = Some(synthetic_event(
                    last_sequence + 1,
                    AgentEventKind::Failed {
                        code: format!("{:?}", error.code).to_lowercase(),
                        message: error.message,
                    },
                ));
                break;
            }
        }
    }
    let terminal_event = terminal_event.unwrap_or_else(|| {
        synthetic_event(
            last_sequence + 1,
            AgentEventKind::Failed {
                code: "stream_interrupted".into(),
                message: "Harness stream ended before a terminal event".into(),
            },
        )
    });
    let _ = session.close().await;
    // Persist and enqueue the terminal event before removing the execution
    // from the active inventory. Otherwise a heartbeat can report the attempt
    // missing while Runtime is still waiting to commit its terminal event.
    if let Err(error) = emit_event(&inner, &key, &execution, &terminal_event).await {
        stop_after_persistence_failure(&inner, &error);
        return;
    }
    inner.active.lock().await.remove(&key);
    inner
        .outstanding_tools
        .lock()
        .await
        .retain(|(execution_key, _)| execution_key != &key);
    inner
        .deferred_tool_results
        .lock()
        .await
        .retain(|(execution_key, _), _| execution_key != &key);
    let mut state = inner.state.lock().await;
    if let Some(assignment) = state.accepted_assignments.get_mut(&key) {
        assignment.lifecycle = AssignmentLifecycle::Terminal;
    }
    if let Some(entry) = state.sessions.get_mut(&session_key) {
        entry.recovery = SessionRecovery::Ready;
    }
    if let Err(error) = state.save(&inner.state_path) {
        stop_after_persistence_failure(&inner, &error);
    }
}

fn stop_after_persistence_failure(inner: &Inner, error: &std::io::Error) {
    eprintln!("Computer stopped because its execution state could not be saved: {error}");
    inner.persistence_failed.store(true, Ordering::Release);
    let _ = inner.stop_tx.send(Some("state_persistence_failed".into()));
}

async fn emit_event(
    inner: &Inner,
    key: &str,
    execution: &ExecutionRef,
    event: &AgentEvent,
) -> std::io::Result<()> {
    let (kind, payload) = protocol_event(event);
    let event = ExecutionEvent {
        execution: Some(execution.clone()),
        sequence: event.sequence,
        occurred_at: now_millis(),
        kind: kind.into(),
        payload,
    };
    let persisted = PersistedEvent {
        execution_id: execution.execution_id,
        attempt: execution.attempt,
        sequence: event.sequence,
        occurred_at: event.occurred_at,
        kind: event.kind,
        payload: event.payload.clone(),
    };
    let mut state = inner.state.lock().await;
    state
        .pending_events
        .entry(key.to_owned())
        .or_default()
        .insert(event.sequence, persisted);
    state.save(&inner.state_path)?;
    drop(state);
    inner
        .pending_events
        .lock()
        .await
        .entry(key.to_owned())
        .or_default()
        .insert(event.sequence, event.clone());

    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::ExecutionEvent(event)),
        },
    )
    .await;
    Ok(())
}

pub(super) fn restore_event(event: &PersistedEvent) -> ExecutionEvent {
    ExecutionEvent {
        execution: Some(ExecutionRef {
            execution_id: event.execution_id,
            attempt: event.attempt,
        }),
        sequence: event.sequence,
        occurred_at: event.occurred_at,
        kind: event.kind,
        payload: event.payload.clone(),
    }
}

fn protocol_event(event: &AgentEvent) -> (ExecutionEventKind, Vec<u8>) {
    let kind = match &event.kind {
        AgentEventKind::Started => ExecutionEventKind::Started,
        AgentEventKind::NativeSessionBound { .. } => ExecutionEventKind::NativeSessionBound,
        AgentEventKind::AssistantDelta { .. } => ExecutionEventKind::AssistantDelta,
        AgentEventKind::ReasoningDelta { .. } => ExecutionEventKind::ReasoningDelta,
        AgentEventKind::ToolStarted { .. } => ExecutionEventKind::ToolStarted,
        AgentEventKind::ToolFinished { .. } => ExecutionEventKind::ToolFinished,
        AgentEventKind::Usage { .. } => ExecutionEventKind::Usage,
        AgentEventKind::Warning { .. } => ExecutionEventKind::Warning,
        AgentEventKind::Completed { .. } => ExecutionEventKind::Completed,
        AgentEventKind::Failed { .. } => ExecutionEventKind::Failed,
        AgentEventKind::Cancelled { .. } => ExecutionEventKind::Cancelled,
    };
    (
        kind,
        serde_json::to_vec(&event.kind).unwrap_or_else(|_| b"{}".to_vec()),
    )
}

pub(super) fn terminal_protocol_event(kind: i32) -> bool {
    matches!(
        ExecutionEventKind::try_from(kind).ok(),
        Some(
            ExecutionEventKind::Completed
                | ExecutionEventKind::Failed
                | ExecutionEventKind::Cancelled
        )
    )
}

fn synthetic_event(sequence: u64, kind: AgentEventKind) -> AgentEvent {
    AgentEvent { sequence, kind }
}
