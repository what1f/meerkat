use super::assignment::{handle_assignment, handle_delete_agent_files};
use super::control::{active_attempts, handle_cancel, handle_steer, replay_pending};
use super::events::terminal_protocol_event;
use super::{Inner, MachineError, MachineHandle, now_millis};
use crate::STANDARD_HARNESS_IDS;
use crate::state::assignment_key;
use lankide_harness::HarnessAvailability as NativeHarnessAvailability;
use lankide_protocol::link::{CloseReason, LinkError, MachineLink};
use lankide_protocol::{
    HarnessAvailability, HarnessInfo, MachineFrame, MachineHeartbeat, MachineHello, RuntimeFrame,
    machine_frame, runtime_frame,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub(super) async fn run_loop(
    inner: Arc<Inner>,
    mut stop_rx: watch::Receiver<Option<String>>,
) -> Result<(), MachineError> {
    let mut delay = inner.config.reconnect.initial;
    loop {
        // A reconnect backoff may already have consumed the watch notification.
        // Check the value as well as awaiting future notifications.
        if stop_rx.borrow().is_some() {
            let reason = stop_rx
                .borrow()
                .clone()
                .unwrap_or_else(|| "machine_shutdown".into());
            MachineHandle::cancel_inner(&inner, &reason).await;
            return if inner.persistence_failed.load(Ordering::Acquire) {
                Err(MachineError::PersistenceFailed)
            } else {
                Ok(())
            };
        }
        tokio::select! {
            _ = stop_rx.changed() => continue,
            result = run_connection(inner.clone()) => {
                *inner.connection.write().await = None;
                inner.connected.send_replace(false);
                super::proxy::disconnected(&inner).await;
                if let Err(error) = &result {
                    log::error!("machine={} link failed: {error}", inner.config.machine_id);
                    if matches!(error, MachineError::InvalidAssignment(_) | MachineError::ProtocolVersionUnsupported | MachineError::Link(LinkError::Closed(CloseReason::MachineRevoked | CloseReason::MachineUnknown))) {
                        MachineHandle::cancel_inner(&inner, "machine_link_rejected").await;
                        return result.map(|_| ());
                    }
                }
                if stop_rx.borrow().is_some() {
                    continue;
                }
                if result.as_ref().is_ok_and(|healthy| *healthy >= inner.config.reconnect.healthy_duration) {
                    delay = inner.config.reconnect.initial;
                }
                // Independent per-attempt jitter in the range 75%-125%.
                let entropy = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos();
                let sleep_delay = delay.mul_f64(0.75 + f64::from(entropy % 501) / 1000.0);
                tokio::select! {
                    _ = stop_rx.changed() => {}
                    _ = tokio::time::sleep(sleep_delay) => {
                        delay = std::cmp::min(inner.config.reconnect.maximum,
                            delay.saturating_mul(inner.config.reconnect.multiplier));
                    }
                }
            }
        }
    }
}

async fn run_connection(inner: Arc<Inner>) -> Result<Duration, MachineError> {
    let (url, secret) = match &inner.link_endpoint {
        Some(endpoint) => endpoint.clone(),
        None => {
            let (url, _) = lankide_protocol::link::discover(&inner.runtime_run)?;
            (url, inner.device_credential.clone())
        }
    };
    let mut link = tokio::time::timeout(
        inner.config.reconnect.handshake_timeout,
        lankide_protocol::link::connect_pinned(
            &url,
            &secret,
            lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES,
            inner.cert_sha256.as_deref(),
        ),
    )
    .await
    .map_err(|_| LinkError::Closed(CloseReason::Timeout))??;
    let (tx, rx) = mpsc::channel::<MachineFrame>(64);
    tokio::time::timeout(
        inner.config.reconnect.handshake_timeout,
        link.send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::Hello(make_hello(&inner).await)),
        }),
    )
    .await
    .map_err(|_| LinkError::Closed(CloseReason::Timeout))??;
    let first = tokio::time::timeout(inner.config.reconnect.handshake_timeout, link.receive())
        .await
        .map_err(|_| LinkError::Closed(CloseReason::Timeout))??
        .ok_or(LinkError::Closed(CloseReason::Shutdown))?;
    if first.protocol_version != lankide_protocol::PROTOCOL_VERSION {
        return Err(MachineError::ProtocolVersionUnsupported);
    }
    let first: RuntimeFrame = first;
    lankide_protocol::validate_runtime_frame(&first)?;
    let heartbeat_interval = match first.frame {
        Some(runtime_frame::Frame::Hello(hello))
            if hello.protocol_version == lankide_protocol::PROTOCOL_VERSION =>
        {
            Duration::from_millis(hello.heartbeat_interval_ms.max(1))
        }
        _ => {
            return Err(MachineError::InvalidAssignment(
                "invalid_runtime_hello".into(),
            ));
        }
    };
    let started = tokio::time::Instant::now();
    let (inbound_tx, mut inbound) = mpsc::channel(64);
    let _link_task = LinkTask(tokio::spawn(pump_link(
        link,
        rx,
        inbound_tx,
        heartbeat_interval,
    )));
    *inner.connection.write().await = Some(tx.clone());
    inner.connected.send_replace(true);
    publish_inventory(&inner).await;
    let replay = replay_pending(&inner, &tx);
    tokio::pin!(replay);
    let mut replay_done = false;
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick is immediate: short reconnects must report live attempts
    // before the link can flap again, rather than waiting a full interval.
    let mut stop_rx = inner.stop_tx.subscribe();
    let deadline = tokio::time::sleep(heartbeat_interval * 3);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut replay, if !replay_done => replay_done = true,
            _ = &mut deadline => {
                log::warn!("machine={} link heartbeat timed out", inner.config.machine_id);
                return Ok(started.elapsed());
            },
            _ = heartbeat.tick() => {
                let frame = MachineFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    frame: Some(machine_frame::Frame::Heartbeat(MachineHeartbeat {
                        machine_id: inner.config.machine_id.clone(),
                        instance_id: inner.config.instance_id.clone(),
                        sent_at: now_millis(),
                        active_attempts: active_attempts(&inner).await,
                    })),
                };
                // Replay can fill the send queue. Keep draining ACKs instead
                // of waiting for another slot; the next tick retries heartbeat.
                match tx.try_send(frame) {
                    Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => return Err(LinkError::Closed(CloseReason::Shutdown).into()),
                }
            }
            changed = stop_rx.changed() => {
                let _ = changed;
                return Ok(started.elapsed());
            }
            frame = inbound.recv() => match frame {
                Some(Ok(frame)) => {
                    if frame.protocol_version != lankide_protocol::PROTOCOL_VERSION { return Err(MachineError::ProtocolVersionUnsupported); }
                    deadline.as_mut().reset(tokio::time::Instant::now() + heartbeat_interval * 3);
                    if !matches!(frame.frame, Some(lankide_protocol::link::RuntimePayload::Heartbeat)) {
                        dispatch(inner.clone(), frame).await?
                    }
                }
                Some(Err(error @ LinkError::Closed(CloseReason::MachineRevoked | CloseReason::MachineUnknown))) => return Err(error.into()),
                Some(Err(error)) => {
                    log::warn!("machine={} link closed: {error}", inner.config.machine_id);
                    return Ok(started.elapsed());
                },
                None => return Ok(started.elapsed()),
            }
        }
    }
}

// Keep outbound delivery active while business handlers persist or replay work.
// Dropping the connection cancels its pump and releases both channel halves.
struct LinkTask(tokio::task::JoinHandle<()>);
impl Drop for LinkTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn pump_link<
    L: MachineLink<
            SendFrame = lankide_protocol::link::MachineFrame,
            ReceiveFrame = lankide_protocol::link::RuntimeFrame,
        >,
>(
    mut link: L,
    mut outgoing: mpsc::Receiver<MachineFrame>,
    incoming: mpsc::Sender<Result<lankide_protocol::link::RuntimeFrame, LinkError>>,
    interval: Duration,
) {
    let result: Result<(), LinkError> = async {
        loop {
            tokio::select! {
                _ = incoming.closed() => return Ok(()),
                frame = outgoing.recv() => {
                    let Some(frame) = frame else { return Ok(()) };
                    tokio::time::timeout(interval, link.send(frame)).await.map_err(|_| LinkError::Closed(CloseReason::Timeout))??;
                }
                frame = link.receive() => {
                    let Some(frame) = frame? else { return Ok(()) };
                    if incoming.send(Ok(frame)).await.is_err() { return Ok(()); }
                }
            }
        }
    }
    .await;
    if let Err(error) = result {
        let _ = incoming.send(Err(error)).await;
    }
}

async fn make_hello(inner: &Inner) -> MachineHello {
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
    MachineHello {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        device_credential: inner.device_credential.clone(),
        machine_id: inner.config.machine_id.clone(),
        instance_id: inner.config.instance_id.clone(),
        machine_version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        harnesses,
    }
}

async fn publish_inventory(inner: &Inner) {
    send_frame(
        inner,
        MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::InventoryChanged(
                lankide_protocol::InventoryChanged {
                    harnesses: make_hello(inner).await.harnesses,
                },
            )),
        },
    )
    .await;
}

async fn dispatch(inner: Arc<Inner>, frame: RuntimeFrame) -> Result<(), MachineError> {
    match frame.frame {
        Some(runtime_frame::Frame::CapabilityResponse(response)) => {
            super::proxy::response(&inner, response).await
        }
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
        Some(runtime_frame::Frame::Hello(_) | runtime_frame::Frame::Heartbeat) => {}
        Some(runtime_frame::Frame::Shutdown(shutdown)) => {
            MachineHandle::cancel_inner(&inner, &shutdown.reason).await;
            let _ = inner.stop_tx.send(Some(shutdown.reason));
        }
        None => {
            return Err(MachineError::InvalidAssignment(
                "unknown_runtime_frame".into(),
            ));
        }
    }
    Ok(())
}

pub(super) async fn send_frame(inner: &Inner, frame: MachineFrame) {
    if let Some(sender) = inner.connection.read().await.clone() {
        let _ = sender.send(frame).await;
    }
}
