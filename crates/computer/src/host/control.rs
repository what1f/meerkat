use super::Inner;
use super::connection::send_frame;
use crate::resources::{download_resource, replace_resource_links};
use crate::state::assignment_key;
use lankide_harness::{ToolResultError, ToolResultRequest, ToolResultResource};
use lankide_protocol::{
    ActiveAttempt, ComputerFrame, DEFAULT_MAX_MESSAGE_BYTES, DeliveryResult, ExecutionRef,
    computer_frame,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc;

pub(super) fn prepare_tool_arguments(
    tool_name: &str,
    arguments: &serde_json::Value,
    agent_home: &Path,
) -> Result<serde_json::Value, (&'static str, &'static str)> {
    if tool_name != "message_send" {
        return Ok(arguments.clone());
    }
    let mut object = arguments.as_object().cloned().ok_or((
        "invalid_tool_arguments",
        "Message arguments must be an object",
    ))?;
    let Some(content_file) = object.remove("content_file") else {
        return Ok(serde_json::Value::Object(object));
    };
    let content_file = content_file
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or(("content_file_invalid", "Content file path is invalid"))?;
    let path = crate::resolve_content_file(agent_home, content_file)
        .map_err(|_| ("content_file_unreadable", "Content file could not be read"))?;
    let metadata = std::fs::metadata(&path)
        .map_err(|_| ("content_file_unreadable", "Content file could not be read"))?;
    if metadata.len() > DEFAULT_MAX_MESSAGE_BYTES as u64 {
        return Err(("content_file_too_large", "Content file is too large"));
    }
    let bytes = std::fs::read(path)
        .map_err(|_| ("content_file_unreadable", "Content file could not be read"))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| ("content_file_invalid_utf8", "Content file must be UTF-8"))?;
    object.insert("content".into(), serde_json::Value::String(content));
    Ok(serde_json::Value::Object(object))
}

pub(super) async fn handle_steer(inner: &Inner, steer: lankide_protocol::DeliverSteer) {
    let Some(execution) = steer.execution else {
        return;
    };
    let Some(delivery) = steer.delivery else {
        return;
    };
    let key = assignment_key(execution.execution_id, execution.attempt);
    let session = inner
        .active
        .lock()
        .await
        .get(&key)
        .filter(|active| {
            active
                .capabilities
                .contains(&lankide_harness::HarnessCapability::Steer)
        })
        .map(|active| active.session.clone());
    let result = match session {
        Some(session) => {
            session
                .steer(delivery.delivery_id.to_string(), delivery.rendered_markdown)
                .await
        }
        None => Err(lankide_harness::HarnessError::new(
            lankide_harness::HarnessErrorCode::InvalidState,
            "execution is not active",
        )),
    };
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::DeliveryResult(DeliveryResult {
                execution: Some(execution),
                delivery_id: delivery.delivery_id,
                accepted: result.as_ref().is_ok_and(|receipt| receipt.accepted),
                error_code: result
                    .as_ref()
                    .err()
                    .map(|error| format!("{:?}", error.code).to_lowercase())
                    .unwrap_or_default(),
                error_message: result.err().map(|error| error.message).unwrap_or_default(),
            })),
        },
    )
    .await;
}

pub(super) async fn handle_cancel(
    inner: &Inner,
    execution: Option<ExecutionRef>,
    reason: String,
    force_close: bool,
) {
    let Some(execution) = execution else { return };
    let key = assignment_key(execution.execution_id, execution.attempt);
    if let Some(starting) = inner
        .starting
        .lock()
        .await
        .values()
        .find(|starting| starting.assignment_key == key)
    {
        let _ = starting.cancel.send(Some(reason.clone()));
    }
    let session = inner.active.lock().await.get_mut(&key).map(|active| {
        active.accepting_tool_results = false;
        active.cancel_reason = Some(reason.clone());
        (
            active.session.clone(),
            active
                .capabilities
                .contains(&lankide_harness::HarnessCapability::Cancel),
        )
    });
    if let Some((session, can_cancel)) = session {
        if can_cancel && !force_close {
            let _ = tokio::time::timeout(Duration::from_secs(2), session.cancel(reason)).await;
        } else {
            let _ = session.close().await;
        }
    }
}

pub(super) async fn route_tool_result(inner: &Inner, result: lankide_protocol::ToolResult) {
    let Some(execution) = result.execution.clone() else {
        return;
    };
    let execution_key = assignment_key(execution.execution_id, execution.attempt);
    let route_key = (execution_key.clone(), result.tool_call_id.clone());
    let active = inner.active.lock().await.get(&execution_key).map(|active| {
        (
            active.session.clone(),
            active.agent_home.clone(),
            active.context.clone(),
            active.accepting_tool_results,
        )
    });
    let Some((session, agent_home, context, accepting)) = active else {
        send_tool_result_receipt(
            inner,
            &execution,
            &result.tool_call_id,
            false,
            "execution_not_active",
            "Execution is no longer active",
        )
        .await;
        return;
    };
    if !accepting {
        send_tool_result_receipt(
            inner,
            &execution,
            &result.tool_call_id,
            false,
            "execution_cancelling",
            "Execution is being cancelled",
        )
        .await;
        return;
    }
    if !inner.outstanding_tools.lock().await.contains(&route_key) {
        send_tool_result_receipt(
            inner,
            &execution,
            &result.tool_call_id,
            false,
            "unknown_tool_call",
            "Tool call is not owned by this execution",
        )
        .await;
        return;
    }
    let mut local_resources = Vec::new();
    for resource in &result.resources {
        match download_resource(
            &inner.socket_path,
            resource,
            context.clone(),
            &agent_home.join("tool-results"),
        )
        .await
        {
            Ok(local_path) => local_resources.push(ToolResultResource {
                local_path,
                file_name: resource.file_name.clone(),
                media_type: resource.media_type.clone(),
                size_bytes: resource.size_bytes,
                sha256: resource.sha256.clone(),
            }),
            Err(_) => {
                send_tool_result_receipt(
                    inner,
                    &execution,
                    &result.tool_call_id,
                    false,
                    "tool_resource_invalid",
                    "Tool result resource could not be verified",
                )
                .await;
                return;
            }
        }
    }
    let local_links = local_resources
        .iter()
        .zip(result.resources.iter())
        .map(|(local, resource)| (resource.resource_id, local.local_path.clone()))
        .collect::<Vec<_>>();
    let structured_content = match serde_json::from_str(&result.structured_content_json) {
        Ok(value) => value,
        Err(_) => {
            send_tool_result_receipt(
                inner,
                &execution,
                &result.tool_call_id,
                false,
                "invalid_tool_result_json",
                "Tool result data is invalid",
            )
            .await;
            return;
        }
    };
    let request = ToolResultRequest {
        tool_call_id: result.tool_call_id.clone(),
        success: result.success,
        content_markdown: replace_resource_links(&result.content_markdown, &local_links),
        structured_content,
        resources: local_resources,
        error: result.error.as_ref().map(|error| ToolResultError {
            code: error.code.clone(),
            message: error.message.clone(),
            retryable: error.retryable,
        }),
    };
    match session.tool_result(request).await {
        Ok(receipt) => {
            inner
                .deferred_tool_results
                .lock()
                .await
                .insert(route_key, result);
            send_tool_result_receipt(
                inner,
                &execution,
                &receipt.tool_call_id,
                true,
                "",
                if receipt.already_applied {
                    "already_applied"
                } else {
                    "applied"
                },
            )
            .await;
        }
        Err(error) => {
            send_tool_result_receipt(
                inner,
                &execution,
                &result.tool_call_id,
                false,
                "tool_result_rejected",
                &error.message,
            )
            .await;
        }
    }
}

async fn send_tool_result_receipt(
    inner: &Inner,
    execution: &ExecutionRef,
    tool_call_id: &str,
    success: bool,
    error_code: &str,
    message: &str,
) {
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::ResourceTransferResult(
                lankide_protocol::ResourceTransferResult {
                    transfer_id: format!(
                        "tool_result:{}:{}:{tool_call_id}",
                        execution.execution_id, execution.attempt
                    ),
                    success,
                    error_code: error_code.to_owned(),
                    error_message: message.to_owned(),
                },
            )),
        },
    )
    .await;
}

pub(super) async fn replay_pending(inner: &Inner, sender: &mpsc::Sender<ComputerFrame>) {
    let pending = inner.pending_events.lock().await;
    for events in pending.values() {
        for event in events.values() {
            let _ = sender
                .send(ComputerFrame {
                    frame: Some(computer_frame::Frame::ExecutionEvent(event.clone())),
                })
                .await;
        }
    }
}

pub(super) async fn replay_assignment_pending(inner: &Inner, key: &str) {
    let events = inner
        .pending_events
        .lock()
        .await
        .get(key)
        .cloned()
        .unwrap_or_default();
    for event in events.into_values() {
        send_frame(
            inner,
            ComputerFrame {
                frame: Some(computer_frame::Frame::ExecutionEvent(event)),
            },
        )
        .await;
    }
}

pub(super) async fn active_attempts(inner: &Inner) -> Vec<ActiveAttempt> {
    let active = inner.active.lock().await;
    let mut attempts = active
        .iter()
        .filter_map(|(key, value)| {
            let (execution_id, attempt) = key.split_once(':')?;
            Some(ActiveAttempt {
                execution: Some(ExecutionRef {
                    execution_id: execution_id.parse().ok()?,
                    attempt: attempt.parse().ok()?,
                }),
                last_event_sequence: value.last_event_sequence,
                harness_process_state: "active".into(),
            })
        })
        .map(|attempt| {
            let execution = attempt.execution.as_ref().expect("active attempt");
            ((execution.execution_id, execution.attempt), attempt)
        })
        .collect::<BTreeMap<_, _>>();
    drop(active);

    // A terminal Harness event remains an active Runtime attempt until Runtime
    // acknowledges the commit. Include that durable outbox in heartbeats so a
    // normal completion is never mistaken for a lost Computer process.
    for (key, events) in inner.pending_events.lock().await.iter() {
        let Some((execution_id, attempt)) = key.split_once(':') else {
            continue;
        };
        let (Ok(execution_id), Ok(attempt)) = (execution_id.parse(), attempt.parse()) else {
            continue;
        };
        attempts
            .entry((execution_id, attempt))
            .or_insert_with(|| ActiveAttempt {
                execution: Some(ExecutionRef {
                    execution_id,
                    attempt,
                }),
                last_event_sequence: events.keys().next_back().copied().unwrap_or_default(),
                harness_process_state: "terminal_pending_commit".into(),
            });
    }
    attempts.into_values().collect()
}
