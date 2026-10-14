use super::assignment::{handle_assignment, handle_delete_agent_files};
use super::control::{
    active_attempts, handle_cancel, handle_steer, replay_pending, route_tool_result,
};
use super::events::terminal_protocol_event;
use super::{ComputerError, ComputerHandle, Inner, now_millis};
use crate::STANDARD_HARNESS_IDS;
use crate::state::assignment_key;
use lankide_harness::HarnessAvailability as NativeHarnessAvailability;
use lankide_protocol::{
    ComputerFrame, ComputerHeartbeat, ComputerHello, HarnessAvailability, HarnessInfo,
    RuntimeFrame, computer_frame, runtime_frame,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

pub(super) async fn run_loop(
    inner: Arc<Inner>,
    mut stop_rx: watch::Receiver<Option<String>>,
) -> Result<(), ComputerError> {
    let mut delay = inner.config.reconnect.initial;
    loop {
        // A reconnect backoff may already have consumed the watch notification.
        // Check the value as well as awaiting future notifications.
        if stop_rx.borrow().is_some() {
            let reason = stop_rx
                .borrow()
                .clone()
                .unwrap_or_else(|| "computer_shutdown".into());
            ComputerHandle::cancel_inner(&inner, &reason).await;
            return if inner.persistence_failed.load(Ordering::Acquire) {
                Err(ComputerError::PersistenceFailed)
            } else {
                Ok(())
            };
        }
        tokio::select! {
            _ = stop_rx.changed() => continue,
            result = run_connection(inner.clone()) => {
                *inner.connection.write().await = None;
                let established = inner.connected.swap(false, Ordering::AcqRel);
                if let Err(error) = result {
                    if matches!(error, ComputerError::InvalidAssignment(_)) {
                        return Err(error);
                    }
                }
                if stop_rx.borrow().is_some() {
                    continue;
                }
                    if established {
                        delay = inner.config.reconnect.initial;
                    }
                    let sleep_delay = delay;
                    tokio::select! {
                        _ = stop_rx.changed() => {}
                        _ = tokio::time::sleep(sleep_delay) => {
                            if !established {
                                delay = std::cmp::min(
                                    inner.config.reconnect.maximum,
                                    delay.saturating_mul(inner.config.reconnect.multiplier),
                                );
                            }
                        }
                    }
            }
        }
    }
}

async fn run_connection(inner: Arc<Inner>) -> Result<(), ComputerError> {
    let mut client = lankide_protocol::connect_uds(
        &inner.socket_path,
        lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES,
    )
    .await?;
    let (tx, rx) = mpsc::channel(64);
    tx.send(ComputerFrame {
        frame: Some(computer_frame::Frame::Hello(make_hello(&inner).await)),
    })
    .await
    .map_err(|_| ComputerError::InvalidAssignment("computer_stream_closed".into()))?;
    let mut inbound = client
        .connect_computer(ReceiverStream::new(rx))
        .await?
        .into_inner();
    let first = inbound.next().await.ok_or_else(|| {
        ComputerError::InvalidAssignment("runtime_stream_closed_before_hello".into())
    })??;
    let heartbeat_interval = match first.frame {
        Some(runtime_frame::Frame::Hello(hello))
            if hello.protocol_version == lankide_protocol::PROTOCOL_VERSION =>
        {
            Duration::from_millis(hello.heartbeat_interval_ms.max(1))
        }
        _ => {
            return Err(ComputerError::InvalidAssignment(
                "invalid_runtime_hello".into(),
            ));
        }
    };
    *inner.connection.write().await = Some(tx.clone());
    inner.connected.store(true, Ordering::Release);
    publish_inventory(&inner).await;
    replay_pending(&inner, &tx).await;
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut stop_rx = inner.stop_tx.subscribe();
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                tx.send(ComputerFrame {
                    frame: Some(computer_frame::Frame::Heartbeat(ComputerHeartbeat {
                        computer_id: inner.config.computer_id.clone(),
                        instance_id: inner.config.instance_id.clone(),
                        sent_at: now_millis(),
                        active_attempts: active_attempts(&inner).await,
                    })),
                }).await.map_err(|_| ComputerError::InvalidAssignment("computer_stream_closed".into()))?;
            }
            changed = stop_rx.changed() => {
                let _ = changed;
                return Ok(());
            }
            frame = inbound.next() => match frame {
                Some(frame) => dispatch(inner.clone(), frame?).await?,
                None => return Ok(()),
            }
        }
    }
}

async fn make_hello(inner: &Inner) -> ComputerHello {
    let harnesses = STANDARD_HARNESS_IDS
        .iter()
        .map(|id| {
            let factory = inner.factories.get(*id);
            let probe = inner.probes.get(*id);
            HarnessInfo {
                capabilities: probe
                    .map(|probe| {
                        probe
                            .capabilities
                            .iter()
                            .map(|capability| capability.as_str().to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
                model_catalog: None,
                model_catalog_error: String::new(),
                harness_id: (*id).to_owned(),
                display_name: factory
                    .map(|factory| factory.info().display_name.clone())
                    .unwrap_or_else(|| match *id {
                        "pi" => "Pi".into(),
                        "codex" => "Codex".into(),
                        _ => "Claude Code".into(),
                    }),
                availability: probe
                    .map(|probe| match probe.availability {
                        NativeHarnessAvailability::Available => HarnessAvailability::Available,
                        NativeHarnessAvailability::Unavailable => HarnessAvailability::Unavailable,
                        NativeHarnessAvailability::Unknown => HarnessAvailability::Unknown,
                    })
                    .unwrap_or(HarnessAvailability::Unavailable)
                    .into(),
                version: probe
                    .and_then(|probe| probe.version.clone())
                    .unwrap_or_default(),
                reason: probe
                    .and_then(|probe| probe.reason.clone())
                    .unwrap_or_else(|| {
                        if factory.is_some() {
                            String::new()
                        } else {
                            "not installed".into()
                        }
                    }),
            }
        })
        .collect();
    ComputerHello {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        computer_id: inner.config.computer_id.clone(),
        instance_id: inner.config.instance_id.clone(),
        computer_version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        harnesses,
    }
}

async fn publish_inventory(inner: &Inner) {
    send_frame(
        inner,
        ComputerFrame {
            frame: Some(computer_frame::Frame::InventoryChanged(
                lankide_protocol::InventoryChanged {
                    harnesses: make_hello(inner).await.harnesses,
                },
            )),
        },
    )
    .await;
}

async fn dispatch(inner: Arc<Inner>, frame: RuntimeFrame) -> Result<(), ComputerError> {
    match frame.frame {
        Some(runtime_frame::Frame::AssignExecution(assign)) => {
            let task_inner = inner.clone();
            let task = tokio::spawn(async move {
                handle_assignment(task_inner, assign).await;
            });
            let mut tasks = inner.assignment_tasks.lock().await;
            tasks.retain(|task| !task.is_finished());
            tasks.push(task);
        }
        Some(runtime_frame::Frame::DeliverSteer(steer)) => {
            handle_steer(&inner, steer).await;
        }
        Some(runtime_frame::Frame::CancelExecution(cancel)) => {
            handle_cancel(&inner, cancel.execution, cancel.reason, cancel.force_close).await;
        }
        Some(runtime_frame::Frame::DeleteAgentFiles(request)) => {
            handle_delete_agent_files(&inner, request).await;
        }
        Some(runtime_frame::Frame::ExecutionEventAck(ack)) => {
            if let Some(execution) = ack.execution {
                let key = assignment_key(execution.execution_id, execution.attempt);
                let mut pending = inner.pending_events.lock().await;
                if let Some(events) = pending.get_mut(&key) {
                    events.retain(|sequence, event| {
                        *sequence > ack.max_saved_sequence || terminal_protocol_event(event.kind)
                    });
                    if events.is_empty() {
                        pending.remove(&key);
                    }
                }
                drop(pending);
                let mut state = inner.state.lock().await;
                if let Some(events) = state.pending_events.get_mut(&key) {
                    events.retain(|sequence, event| {
                        *sequence > ack.max_saved_sequence || terminal_protocol_event(event.kind)
                    });
                    if events.is_empty() {
                        state.pending_events.remove(&key);
                    }
                    state.save(&inner.state_path)?;
                }
            }
        }
        Some(runtime_frame::Frame::ToolResult(result)) => {
            route_tool_result(&inner, result).await;
        }
        Some(runtime_frame::Frame::ExecutionCommitted(committed)) => {
            if let Some(execution) = committed.execution {
                let key = assignment_key(execution.execution_id, execution.attempt);
                inner.pending_events.lock().await.remove(&key);
                let mut state = inner.state.lock().await;
                state.pending_events.remove(&key);
                state.accepted_assignments.remove(&key);
                state.save(&inner.state_path)?;
            }
        }
        Some(runtime_frame::Frame::Hello(_)) => {}
        Some(runtime_frame::Frame::Shutdown(shutdown)) => {
            ComputerHandle::cancel_inner(&inner, &shutdown.reason).await;
            let _ = inner.stop_tx.send(Some(shutdown.reason));
        }
        None => {
            return Err(ComputerError::InvalidAssignment(
                "unknown_runtime_frame".into(),
            ));
        }
    }
    Ok(())
}

pub(super) async fn send_frame(inner: &Inner, frame: ComputerFrame) {
    if let Some(sender) = inner.connection.read().await.clone() {
        let _ = sender.send(frame).await;
    }
}
