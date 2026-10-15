use super::*;
use lankide_protocol::Status;
use lankide_protocol::{
    ComputerFrame, ComputerHello, ExecutionEventKind, HarnessInfo, RuntimeFrame,
    RuntimeFrameStream, computer_frame, runtime_frame,
};
use lankide_storage::{ExecutionEventDisposition, RecordExecutionEvent, SteerDelivery, Storage};
use std::collections::BTreeSet;
use std::sync::{
    Mutex as StdMutex, RwLock as StdRwLock,
    atomic::{AtomicI64, AtomicU64, Ordering},
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct ComputerBroker {
    inner: Arc<BrokerInner>,
}

struct BrokerInner {
    storage: database::WeakDatabase,
    attempt_lease_millis: AtomicI64,
    connections: StdRwLock<BTreeMap<String, ComputerConnection>>,
    next_connection_id: AtomicU64,
    link_rejections: StdRwLock<BTreeMap<String, lankide_protocol::link::CloseReason>>,
    pending: StdMutex<BTreeMap<i64, PendingExecution>>,
    pending_agent_deletions: StdMutex<BTreeMap<String, oneshot::Sender<Result<(), String>>>>,
}

#[derive(Clone)]
struct ComputerConnection {
    connection_id: u64,
    revoked: Arc<AtomicBool>,
    instance_id: String,
    sender: mpsc::Sender<Result<RuntimeFrame, Status>>,
    harnesses: Vec<HarnessInfo>,
    reconciled: bool,
    reconciling: bool,
    active_agents: BTreeSet<(String, String)>,
}

struct PendingExecution {
    computer_id: String,
    instance_id: String,
    attempt: i64,
    harness_id: String,
    completion: oneshot::Sender<Result<ComputerCompletion, ComputerError>>,
    assignment_accepted: bool,
    delivery_accepted: bool,
    max_saved_sequence: u64,
    output: String,
}

impl ComputerBroker {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self::with_database(Database::new(storage))
    }

    pub(crate) fn with_database(storage: Database) -> Self {
        Self {
            inner: Arc::new(BrokerInner {
                storage: storage.downgrade(),
                attempt_lease_millis: AtomicI64::new(15_000),
                connections: StdRwLock::new(BTreeMap::new()),
                next_connection_id: AtomicU64::new(0),
                link_rejections: StdRwLock::new(BTreeMap::new()),
                pending: StdMutex::new(BTreeMap::new()),
                pending_agent_deletions: StdMutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Configure how long an attempt can remain unreported by its Computer.
    /// The production default is fifteen seconds.
    pub fn set_attempt_lease(&self, duration: std::time::Duration) -> Result<(), RuntimeError> {
        let millis = i64::try_from(duration.as_millis())
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| RuntimeError::Application("invalid attempt lease duration".into()))?;
        self.inner
            .attempt_lease_millis
            .store(millis, Ordering::Relaxed);
        Ok(())
    }

    pub fn reject_link(
        &self,
        computer_id: String,
        reason: lankide_protocol::link::CloseReason,
    ) -> Result<(), RuntimeError> {
        self.inner
            .link_rejections
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .insert(computer_id, reason);
        Ok(())
    }

    pub fn rejected_links(
        &self,
    ) -> Result<BTreeMap<String, lankide_protocol::link::CloseReason>, RuntimeError> {
        Ok(self
            .inner
            .link_rejections
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .clone())
    }

    pub fn is_connected(&self, computer_id: &str) -> bool {
        self.inner
            .connections
            .read()
            .map(|values| values.contains_key(computer_id))
            .unwrap_or(false)
    }

    pub(crate) fn can_dispatch(
        &self,
        computer_id: &str,
        workspace: &WorkspaceId,
        agent: &AgentId,
    ) -> bool {
        self.inner
            .connections
            .read()
            .map(|connections| {
                connections.get(computer_id).is_some_and(|connection| {
                    connection.reconciled
                        && !connection.reconciling
                        && !connection
                            .active_agents
                            .contains(&(workspace.to_string(), agent.to_string()))
                })
            })
            .unwrap_or(false)
    }

    pub async fn expire_leases(&self, now: i64) -> Result<Vec<(i64, i64)>, RuntimeError> {
        let expired = self
            .database()?
            .expire_execution_attempt_leases(now)
            .await?;
        self.release_lost_attempts(&expired, "execution_lease_expired")?;
        Ok(expired)
    }

    fn release_lost_attempts(&self, lost: &[(i64, i64)], code: &str) -> Result<(), RuntimeError> {
        let mut pending = self
            .inner
            .pending
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        for (execution_id, attempt) in lost {
            if pending
                .get(execution_id)
                .is_some_and(|value| value.attempt == *attempt)
                && let Some(execution) = pending.remove(execution_id)
            {
                if let Ok(connections) = self.inner.connections.read()
                    && let Some(connection) = connections.get(&execution.computer_id)
                    && connection.instance_id == execution.instance_id
                {
                    let _ = connection.sender.try_send(Ok(RuntimeFrame {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        frame: Some(runtime_frame::Frame::CancelExecution(
                            lankide_protocol::CancelExecution {
                                force_close: !supports(connection, &execution.harness_id, "cancel"),
                                execution: Some(ExecutionRef {
                                    execution_id: *execution_id,
                                    attempt: *attempt as u32,
                                }),
                                reason: "Execution is no longer active".into(),
                            },
                        )),
                    }));
                }
                let _ = execution
                    .completion
                    .send(Err(ComputerError { code: code.into() }));
            }
        }
        Ok(())
    }

    fn database(&self) -> Result<Database, RuntimeError> {
        self.inner
            .storage
            .upgrade()
            .ok_or(RuntimeError::ApplicationStopped)
    }

    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        let connections = std::mem::take(
            &mut *self
                .inner
                .connections
                .write()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?,
        );
        for (_, connection) in connections {
            let _ = connection.sender.try_send(Ok(RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::Shutdown(lankide_protocol::Shutdown {
                    reason: "Runtime is stopping.".into(),
                })),
            }));
        }
        let pending = std::mem::take(
            &mut *self
                .inner
                .pending
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?,
        );
        for (_, pending) in pending {
            let _ = pending.completion.send(Err(ComputerError {
                code: "computer_disconnected".into(),
            }));
        }
        Ok(())
    }

    pub fn revoke(&self, computer_id: &str) -> Result<(), RuntimeError> {
        if let Some(connection) = self
            .inner
            .connections
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(computer_id)
        {
            // The stream checks this flag before sending any queued business frame.
            connection.revoked.store(true, Ordering::Release);
            let _ = connection
                .sender
                .try_send(Err(Status::unauthenticated("computer_revoked")));
        }
        Ok(())
    }

    pub async fn connect(&self, hello: ComputerHello) -> Result<RuntimeFrameStream, RuntimeError> {
        self.connect_tracked(hello).await.map(|(_, stream)| stream)
    }

    pub async fn connect_tracked(
        &self,
        hello: ComputerHello,
    ) -> Result<(u64, RuntimeFrameStream), RuntimeError> {
        self.inner
            .link_rejections
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(&hello.computer_id);
        let (sender, receiver) = mpsc::channel(256);
        let connection_id = self
            .inner
            .next_connection_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let revoked = Arc::new(AtomicBool::new(false));
        let connection = ComputerConnection {
            connection_id,
            revoked: revoked.clone(),
            instance_id: hello.instance_id.clone(),
            sender: sender.clone(),
            harnesses: hello.harnesses,
            reconciled: false,
            reconciling: false,
            active_agents: BTreeSet::new(),
        };
        if let Some(previous) = self
            .inner
            .connections
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .insert(hello.computer_id.clone(), connection)
        {
            let _ = previous.sender.try_send(Ok(RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::Shutdown(lankide_protocol::Shutdown {
                    reason: "replaced_by_reconnect".into(),
                })),
            }));
        }
        sender
            .try_send(Ok(RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::Hello(
                    lankide_protocol::RuntimeHello {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        runtime_instance_id: format!("runtime-{}", std::process::id()),
                        heartbeat_interval_ms: lankide_protocol::link::LinkSettings::default()
                            .heartbeat_interval
                            .as_millis() as u64,
                        max_message_bytes: lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES as u64,
                    },
                )),
            }))
            .map_err(|_| RuntimeError::Application("computer stream closed".into()))?;
        for delivery in self
            .database()?
            .pending_steer_deliveries(&hello.computer_id, &hello.instance_id)
            .await?
        {
            if !self.can_steer(&delivery)? {
                continue;
            }
            sender
                .try_send(Ok(steer_frame(delivery)))
                .map_err(|_| RuntimeError::Application("computer stream closed".into()))?;
        }
        Ok((
            connection_id,
            Box::pin(RegisteredComputerStream {
                receiver,
                revoked,
                closed: false,
            }),
        ))
    }

    pub async fn disconnect(
        &self,
        computer_id: &str,
        instance_id: &str,
    ) -> Result<(), RuntimeError> {
        self.disconnect_matching(computer_id, instance_id, None)
            .await
    }

    pub async fn disconnect_connection(
        &self,
        computer_id: &str,
        instance_id: &str,
        connection_id: u64,
    ) -> Result<(), RuntimeError> {
        self.disconnect_matching(computer_id, instance_id, Some(connection_id))
            .await
    }

    async fn disconnect_matching(
        &self,
        computer_id: &str,
        instance_id: &str,
        connection_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let removed = {
            let mut connections = self
                .inner
                .connections
                .write()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            if connections.get(computer_id).is_some_and(|connection| {
                connection.instance_id == instance_id
                    && connection_id.is_none_or(|id| connection.connection_id == id)
            }) {
                connections.remove(computer_id);
                true
            } else {
                false
            }
        };
        if !removed {
            return Ok(());
        }
        // A link loss is not an attempt loss. Keep the completion waiter and
        // its acceptance/output state until a terminal event or reconciliation.
        let execution_ids = {
            let pending = self
                .inner
                .pending
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            pending
                .iter()
                .filter(|(_, value)| {
                    value.computer_id == computer_id && value.instance_id == instance_id
                })
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        };
        for id in execution_ids {
            self.database()?
                .mark_execution_recovering(id, now_millis())
                .await?;
        }
        self.inner.storage.wake();
        Ok(())
    }

    pub async fn handle_frame(
        &self,
        computer_id: &str,
        instance_id: &str,
        frame: ComputerFrame,
    ) -> Result<(), RuntimeError> {
        if !self
            .inner
            .connections
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(computer_id)
            .is_some_and(|connection| connection.instance_id == instance_id)
        {
            return Err(RuntimeError::Application(
                "stale computer connection".into(),
            ));
        }
        let Some(frame) = frame.frame else {
            return Ok(());
        };
        if matches!(&frame, computer_frame::Frame::Heartbeat(_)) {
            if let Some(connection) = self
                .inner
                .connections
                .write()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
                .get_mut(computer_id)
            {
                connection.reconciling = true;
            }
        }
        let execution = match &frame {
            computer_frame::Frame::AssignmentResult(v) => v.execution.as_ref(),
            computer_frame::Frame::ExecutionEvent(v) => v.execution.as_ref(),
            computer_frame::Frame::DeliveryResult(v) => v.execution.as_ref(),
            _ => None,
        };
        if let Some(execution) = execution
            && !self
                .database()?
                .execution_placement_matches(
                    execution.execution_id,
                    i64::from(execution.attempt),
                    computer_id,
                )
                .await?
        {
            // Placement changes invalidate every event in the old attempt's
            // outbox, including progress saved before its acknowledgement arrived.
            // Stop any stale local run and let this Computer discard that outbox;
            // none of the rejected data reaches storage or a Provider.
            for frame in [
                runtime_frame::Frame::CancelExecution(lankide_protocol::CancelExecution {
                    execution: Some(*execution),
                    force_close: true,
                    reason: "placement_changed".into(),
                }),
                runtime_frame::Frame::ExecutionCommitted(lankide_protocol::ExecutionCommitted {
                    execution: Some(*execution),
                    committed_at: now_millis(),
                }),
            ] {
                self.send_to(
                    computer_id,
                    instance_id,
                    RuntimeFrame {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        frame: Some(frame),
                    },
                )
                .await?;
            }
            return Err(RuntimeError::InvalidExecutionToken);
        }
        match &frame {
            computer_frame::Frame::Heartbeat(_) => {
                self.database()?
                    .observe_computer(computer_id, now_millis(), None, None, None)
                    .await?;
            }
            computer_frame::Frame::InventoryChanged(inventory) => {
                self.database()?
                    .observe_computer(
                        computer_id,
                        now_millis(),
                        Some(
                            &serde_json::to_string(&inventory.harnesses)
                                .map_err(|e| RuntimeError::Application(e.to_string()))?,
                        ),
                        None,
                        None,
                    )
                    .await?;
            }
            _ => {}
        }
        match frame {
            computer_frame::Frame::AssignmentResult(result) => {
                let execution = result.execution.as_ref().ok_or_else(|| {
                    RuntimeError::Application("assignment missing execution".into())
                })?;
                let execution_id = execution.execution_id;
                if result.accepted {
                    self.database()?
                        .accept_execution_attempt(
                            execution_id,
                            i64::from(execution.attempt),
                            true,
                            now_millis(),
                        )
                        .await?;
                } else {
                    if result
                        .failure
                        .as_ref()
                        .is_some_and(|failure| failure.category == "conflict")
                    {
                        if let Some(connection) = self
                            .inner
                            .connections
                            .write()
                            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
                            .get_mut(computer_id)
                        {
                            connection.reconciled = false;
                        }
                    }
                    self.database()?
                        .reject_execution_attempt_with_failure(
                            execution_id,
                            i64::from(execution.attempt),
                            &result.error_code,
                            &result.error_message,
                            result
                                .failure
                                .as_ref()
                                .map(serde_json::to_value)
                                .transpose()
                                .map_err(|error| RuntimeError::Application(error.to_string()))?
                                .as_ref(),
                            now_millis(),
                        )
                        .await?;
                }
                let mut pending = self
                    .inner
                    .pending
                    .lock()
                    .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
                if result.accepted {
                    if let Some(value) = pending
                        .get_mut(&execution_id)
                        .filter(|value| value.computer_id == computer_id)
                    {
                        value.assignment_accepted = true;
                    }
                } else if pending
                    .get(&execution_id)
                    .is_some_and(|value| value.computer_id == computer_id)
                {
                    let value = pending.remove(&execution_id).expect("checked pending");
                    let _ = value.completion.send(Ok(ComputerCompletion {
                        delivery_accepted: false,
                        outcome: ComputerOutcome::Failed {
                            code: if result.error_code.is_empty() {
                                "assignment_rejected".into()
                            } else {
                                result.error_code
                            },
                        },
                    }));
                }
            }
            computer_frame::Frame::ExecutionEvent(event) => {
                let execution = event
                    .execution
                    .clone()
                    .ok_or_else(|| RuntimeError::Application("event missing execution".into()))?;
                let kind = ExecutionEventKind::try_from(event.kind)
                    .unwrap_or(ExecutionEventKind::Unspecified);
                // A restarted daemon keeps the durable per-attempt outbox but has
                // a new process identity. Authenticate its Computer ownership,
                // then let storage enforce the attempt/sequence commit boundary.
                let owner = self
                    .database()?
                    .execution_attempt(execution.execution_id, i64::from(execution.attempt))
                    .await?;
                let event_instance = owner
                    .as_ref()
                    .filter(|owner| owner.computer_id == computer_id)
                    .map(|owner| owner.computer_instance_id.as_str())
                    .unwrap_or(instance_id);
                let receipt = self
                    .database()?
                    .record_execution_event(RecordExecutionEvent {
                        execution_id: execution.execution_id,
                        attempt: i64::from(execution.attempt),
                        computer_id: computer_id.to_owned(),
                        computer_instance_id: event_instance.to_owned(),
                        sequence: event.sequence,
                        kind: execution_event_kind_name(kind).to_owned(),
                        payload: event.payload.clone(),
                        occurred_at: event.occurred_at,
                        now: event.occurred_at,
                    })
                    .await?;
                // A lost attempt cannot commit late output, but its Computer
                // still needs a terminal receipt to discard the durable outbox.
                let abandoned_terminal = receipt.disposition
                    == ExecutionEventDisposition::StaleAttempt
                    && matches!(
                        kind,
                        ExecutionEventKind::Completed
                            | ExecutionEventKind::Failed
                            | ExecutionEventKind::Cancelled
                    )
                    && owner.as_ref().is_some_and(|owner| {
                        owner.computer_id == computer_id
                            && matches!(
                                owner.state,
                                lankide_storage::ExecutionAttemptState::Lost
                                    | lankide_storage::ExecutionAttemptState::Rejected
                                    | lankide_storage::ExecutionAttemptState::Failed
                                    | lankide_storage::ExecutionAttemptState::Cancelled
                            )
                    });
                let mut completion = None;
                let max_saved_sequence = receipt.max_saved_sequence;
                {
                    let mut pending = self
                        .inner
                        .pending
                        .lock()
                        .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
                    if let Some(value) = pending
                        .get_mut(&execution.execution_id)
                        .filter(|value| value.computer_id == computer_id)
                    {
                        if receipt.disposition == ExecutionEventDisposition::Saved
                            && matches!(kind, ExecutionEventKind::AssistantDelta)
                        {
                            value
                                .output
                                .push_str(&String::from_utf8_lossy(&event.payload));
                        }
                        value.max_saved_sequence = max_saved_sequence;
                    }
                    if receipt.disposition.terminal()
                        && pending.get(&execution.execution_id).is_some_and(|value| {
                            value.computer_id == computer_id
                                && value.max_saved_sequence == event.sequence
                        })
                    {
                        completion = pending.remove(&execution.execution_id);
                    }
                }
                if let Some(value) = completion {
                    let outcome = match kind {
                        ExecutionEventKind::Completed => ComputerOutcome::Succeeded {
                            output: value.output,
                            reply: None,
                        },
                        ExecutionEventKind::Cancelled => ComputerOutcome::Cancelled,
                        _ => ComputerOutcome::Failed {
                            code: "computer_execution_failed".into(),
                        },
                    };
                    let _ = value.completion.send(Ok(ComputerCompletion {
                        delivery_accepted: value.assignment_accepted && value.delivery_accepted,
                        outcome,
                    }));
                }
                self.send_to(
                    computer_id,
                    instance_id,
                    RuntimeFrame {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        frame: Some(runtime_frame::Frame::ExecutionEventAck(
                            lankide_protocol::ExecutionEventAck {
                                execution: Some(execution),
                                max_saved_sequence,
                            },
                        )),
                    },
                )
                .await?;
                if receipt.disposition.terminal() || abandoned_terminal {
                    self.send_to(
                        computer_id,
                        instance_id,
                        RuntimeFrame {
                            protocol_version: lankide_protocol::PROTOCOL_VERSION,
                            frame: Some(runtime_frame::Frame::ExecutionCommitted(
                                lankide_protocol::ExecutionCommitted {
                                    execution: Some(execution.clone()),
                                    committed_at: event.occurred_at,
                                },
                            )),
                        },
                    )
                    .await?;
                }
            }
            computer_frame::Frame::DeliveryResult(result) => {
                let execution_id = result
                    .execution
                    .as_ref()
                    .map(|value| value.execution_id)
                    .ok_or_else(|| {
                        RuntimeError::Application("delivery result missing execution".into())
                    })?;
                let attempt = result
                    .execution
                    .as_ref()
                    .map(|value| value.attempt)
                    .unwrap_or_default();
                if result.delivery_id < 0 {
                    self.database()?
                        .mark_steer_delivery_result(
                            -result.delivery_id,
                            execution_id,
                            i64::from(attempt),
                            computer_id,
                            instance_id,
                            result.accepted,
                            now_millis(),
                        )
                        .await?;
                    return Ok(());
                }
                self.database()?
                    .mark_attempt_delivery(
                        execution_id,
                        i64::from(attempt),
                        result.delivery_id,
                        result.accepted,
                        now_millis(),
                    )
                    .await?;
                let mut pending = self
                    .inner
                    .pending
                    .lock()
                    .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
                if let Some(value) = pending
                    .get_mut(&execution_id)
                    .filter(|value| value.computer_id == computer_id)
                {
                    value.delivery_accepted = result.accepted;
                }
            }
            computer_frame::Frame::InventoryChanged(inventory) => {
                if let Some(connection) = self
                    .inner
                    .connections
                    .write()
                    .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
                    .get_mut(computer_id)
                {
                    connection.harnesses = inventory.harnesses;
                }
            }
            computer_frame::Frame::Heartbeat(heartbeat) => {
                if heartbeat.computer_id != computer_id || heartbeat.instance_id != instance_id {
                    return Err(RuntimeError::Application(
                        "heartbeat identity mismatch".into(),
                    ));
                }
                let active = heartbeat
                    .active_attempts
                    .iter()
                    .filter_map(|attempt| {
                        attempt
                            .execution
                            .as_ref()
                            .map(|execution| (execution.execution_id, i64::from(execution.attempt)))
                    })
                    .collect::<Vec<_>>();
                // Use Runtime receipt time for leases, independent of Computer clock skew.
                let now = now_millis();
                self.expire_leases(now).await?;
                let reconciled = self
                    .database()?
                    .reconcile_computer_heartbeat(
                        computer_id,
                        instance_id,
                        &active,
                        now,
                        now.saturating_add(self.inner.attempt_lease_millis.load(Ordering::Relaxed)),
                    )
                    .await?;
                self.release_lost_attempts(&reconciled.lost, "execution_attempt_lost")?;
                // Include reported runs even if their Runtime lease already expired.
                // The Computer must release the Agent before another dispatch.
                let storage = self.database()?;
                let mut active_agents = BTreeSet::new();
                for (id, attempt) in &active {
                    if !storage
                        .execution_placement_matches(*id, *attempt, computer_id)
                        .await?
                    {
                        self.send_to(
                            computer_id,
                            instance_id,
                            RuntimeFrame {
                                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                                frame: Some(runtime_frame::Frame::CancelExecution(
                                    lankide_protocol::CancelExecution {
                                        execution: Some(ExecutionRef {
                                            execution_id: *id,
                                            attempt: *attempt as u32,
                                        }),
                                        reason: "Execution is no longer active".into(),
                                        force_close: true,
                                    },
                                )),
                            },
                        )
                        .await?;
                        continue;
                    }
                    if let Some(context) = storage.execution_auth_context(*id, *attempt).await? {
                        active_agents.insert((
                            context.workspace_id.to_string(),
                            context.agent_id.to_string(),
                        ));
                        if matches!(
                            context.state,
                            lankide_storage::ExecutionState::Failed
                                | lankide_storage::ExecutionState::Cancelled
                        ) {
                            // Expiry can occur while offline, when cancellation
                            // cannot reach the Computer. Close the orphan on return.
                            self.send_to(
                                computer_id,
                                instance_id,
                                RuntimeFrame {
                                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                                    frame: Some(runtime_frame::Frame::CancelExecution(
                                        lankide_protocol::CancelExecution {
                                            execution: Some(ExecutionRef {
                                                execution_id: *id,
                                                attempt: *attempt as u32,
                                            }),
                                            reason: "Execution is no longer active".into(),
                                            force_close: true,
                                        },
                                    )),
                                },
                            )
                            .await?;
                        }
                    }
                }
                if let Some(connection) = self
                    .inner
                    .connections
                    .write()
                    .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
                    .get_mut(computer_id)
                    .filter(|connection| connection.instance_id == instance_id)
                {
                    connection.active_agents = active_agents;
                    connection.reconciled = true;
                    connection.reconciling = false;
                }
            }
            computer_frame::Frame::ResourceTransferResult(_) => {}
            computer_frame::Frame::DeleteAgentFilesResult(result) => {
                if let Some(sender) = self
                    .inner
                    .pending_agent_deletions
                    .lock()
                    .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
                    .remove(&result.request_id)
                {
                    let _ = sender.send(if result.success {
                        Ok(())
                    } else {
                        Err(result.error_code)
                    });
                }
            }
            computer_frame::Frame::CapabilityRequest(_) => {}
            computer_frame::Frame::Hello(_) => {
                return Err(RuntimeError::Application("duplicate computer hello".into()));
            }
        }
        self.inner.storage.wake();
        Ok(())
    }

    pub fn executor(&self, computer_id: impl Into<String>) -> BrokerComputerExecutor {
        BrokerComputerExecutor {
            broker: self.clone(),
            computer_id: computer_id.into(),
        }
    }

    pub async fn send_frame(
        &self,
        computer_id: &str,
        instance_id: &str,
        frame: RuntimeFrame,
    ) -> Result<(), RuntimeError> {
        self.send_to(computer_id, instance_id, frame).await
    }

    pub async fn delete_agent_files(
        &self,
        computer_id: &str,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<(), RuntimeError> {
        let connection = self
            .inner
            .connections
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(computer_id)
            .cloned()
            .ok_or_else(|| RuntimeError::Application("computer disconnected".into()))?;
        let request_id = format!("delete-agent-{}-{}", std::process::id(), now_millis());
        let (sender, receiver) = oneshot::channel();
        self.inner
            .pending_agent_deletions
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .insert(request_id.clone(), sender);
        if let Err(error) = connection
            .sender
            .send(Ok(RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::DeleteAgentFiles(
                    lankide_protocol::DeleteAgentFiles {
                        request_id: request_id.clone(),
                        agent: Some(lankide_protocol::AgentRef {
                            workspace_id: workspace_id.to_string(),
                            agent_id: agent_id.to_string(),
                        }),
                    },
                )),
            }))
            .await
        {
            self.inner
                .pending_agent_deletions
                .lock()
                .ok()
                .and_then(|mut values| values.remove(&request_id));
            return Err(RuntimeError::Application(error.to_string()));
        }
        let result = match tokio::time::timeout(std::time::Duration::from_secs(15), receiver).await
        {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(code))) => Err(RuntimeError::Application(code)),
            _ => {
                self.inner
                    .pending_agent_deletions
                    .lock()
                    .ok()
                    .and_then(|mut values| values.remove(&request_id));
                Err(RuntimeError::Application(
                    "agent file deletion timed out".into(),
                ))
            }
        };
        if result.is_ok() {
            self.database()?
                .complete_agent_file_deletion(workspace_id, agent_id)
                .await?;
        }
        result
    }

    pub(crate) async fn retry_agent_file_deletions(
        &self,
        computer_id: &str,
    ) -> Result<(), RuntimeError> {
        let pending = self
            .database()?
            .pending_agent_file_deletions(computer_id)
            .await?;
        for (workspace_id, agent_id) in pending {
            if let Err(error) = self
                .delete_agent_files(computer_id, &workspace_id, &agent_id)
                .await
            {
                log::error!(
                    "[runtime] workspace={workspace_id} agent={agent_id} deferred file cleanup failed: {error}"
                );
            }
        }
        Ok(())
    }

    /// Management cancellation retains the pending attempt until the Computer
    /// reports its terminal outcome after native process cleanup.
    pub async fn request_cancellation(&self, execution_id: i64) -> Result<(), RuntimeError> {
        let mut destination = self
            .inner
            .pending
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(&execution_id)
            .map(|pending| {
                (
                    pending.computer_id.clone(),
                    pending.instance_id.clone(),
                    pending.attempt,
                    pending.harness_id.clone(),
                )
            });
        // A surviving attempt has no in-memory waiter after Runtime restart.
        // Its durable ownership still identifies where cancellation must go.
        if destination.is_none() {
            let storage = self.database()?;
            if let Some(attempt) = storage
                .execution_attempts(execution_id)
                .await?
                .into_iter()
                .last()
                && matches!(
                    attempt.state,
                    lankide_storage::ExecutionAttemptState::Offered
                        | lankide_storage::ExecutionAttemptState::Accepted
                        | lankide_storage::ExecutionAttemptState::Running
                )
                && let Some(context) = storage
                    .execution_auth_context(execution_id, attempt.attempt)
                    .await?
                && matches!(
                    context.state,
                    lankide_storage::ExecutionState::Assigning
                        | lankide_storage::ExecutionState::Running
                        | lankide_storage::ExecutionState::Recovering
                )
            {
                let agent = storage
                    .agent_record(&context.workspace_id, &context.agent_id)
                    .await?;
                destination = Some((
                    attempt.computer_id,
                    attempt.computer_instance_id,
                    attempt.attempt,
                    agent.harness_id,
                ));
            }
        }
        if let Some((computer, instance, attempt, harness_id)) = destination {
            let force_close = self
                .inner
                .connections
                .read()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
                .get(&computer)
                .is_none_or(|connection| !supports(connection, &harness_id, "cancel"));
            self.send_to(
                &computer,
                &instance,
                RuntimeFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    frame: Some(runtime_frame::Frame::CancelExecution(
                        lankide_protocol::CancelExecution {
                            force_close,
                            execution: Some(ExecutionRef {
                                execution_id,
                                attempt: attempt as u32,
                            }),
                            reason: "Execution cancelled".into(),
                        },
                    )),
                },
            )
            .await?;
        }
        Ok(())
    }

    pub fn cancel_execution(&self, execution_id: i64, reason: &str) -> Result<(), RuntimeError> {
        let pending = self
            .inner
            .pending
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(&execution_id);
        if let Some(pending) = pending {
            if let Ok(connections) = self.inner.connections.read()
                && let Some(connection) = connections.get(&pending.computer_id)
                && connection.instance_id == pending.instance_id
            {
                let _ = connection.sender.try_send(Ok(RuntimeFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    frame: Some(runtime_frame::Frame::CancelExecution(
                        lankide_protocol::CancelExecution {
                            force_close: !supports(connection, &pending.harness_id, "cancel"),
                            execution: Some(ExecutionRef {
                                execution_id,
                                attempt: pending.attempt as u32,
                            }),
                            reason: reason.to_owned(),
                        },
                    )),
                }));
            }
            let _ = pending.completion.send(Err(ComputerError {
                code: "execution_cancelled".into(),
            }));
        }
        Ok(())
    }

    fn can_steer(&self, delivery: &SteerDelivery) -> Result<bool, RuntimeError> {
        let harness_id = self
            .inner
            .pending
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(&delivery.execution_id)
            .filter(|execution| {
                execution.attempt == delivery.attempt
                    && execution.computer_id == delivery.computer_id
                    && execution.instance_id == delivery.computer_instance_id
            })
            .map(|execution| execution.harness_id.clone());
        let Some(harness_id) = harness_id else {
            return Ok(false);
        };
        let connections = self
            .inner
            .connections
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        Ok(connections
            .get(&delivery.computer_id)
            .is_some_and(|connection| {
                connection.instance_id == delivery.computer_instance_id
                    && supports(connection, &harness_id, "steer")
            }))
    }

    pub async fn deliver_steer(&self, delivery: SteerDelivery) -> Result<(), RuntimeError> {
        if !self.can_steer(&delivery)? {
            return Ok(());
        }
        let computer_id = delivery.computer_id.clone();
        let computer_instance_id = delivery.computer_instance_id.clone();
        self.send_to(&computer_id, &computer_instance_id, steer_frame(delivery))
            .await
    }

    pub fn catalog(&self) -> Result<Vec<ComputerInventory>, RuntimeError> {
        Ok(self
            .inner
            .connections
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .iter()
            .filter(|(_, connection)| connection.reconciled)
            .map(|(computer_id, connection)| ComputerInventory {
                computer_id: computer_id.clone(),
                instance_id: connection.instance_id.clone(),
                harnesses: connection.harnesses.clone(),
            })
            .collect())
    }

    async fn send_to(
        &self,
        computer_id: &str,
        instance_id: &str,
        frame: RuntimeFrame,
    ) -> Result<(), RuntimeError> {
        let sender = self
            .inner
            .connections
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(computer_id)
            .filter(|connection| connection.instance_id == instance_id)
            .map(|connection| connection.sender.clone())
            .ok_or_else(|| RuntimeError::Application("computer disconnected".into()))?;
        sender
            .send(Ok(frame))
            .await
            .map_err(|_| RuntimeError::Application("computer stream closed".into()))
    }
}

fn supports(connection: &ComputerConnection, harness_id: &str, capability: &str) -> bool {
    connection.harnesses.iter().any(|harness| {
        harness.harness_id == harness_id
            && harness.capabilities.iter().any(|value| value == capability)
    })
}

fn steer_frame(delivery: SteerDelivery) -> RuntimeFrame {
    RuntimeFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(runtime_frame::Frame::DeliverSteer(
            lankide_protocol::DeliverSteer {
                execution: Some(lankide_protocol::ExecutionRef {
                    execution_id: delivery.execution_id,
                    attempt: delivery.attempt as u32,
                }),
                delivery: Some(lankide_protocol::Delivery {
                    delivery_id: delivery.delivery_id,
                    kind: lankide_protocol::DeliveryKind::Steer as i32,
                    rendered_markdown: delivery.rendered_markdown,
                    snapshot_id: 0,
                    trigger_reasons: Vec::new(),
                    resources: Vec::new(),
                    created_at: delivery.created_at,
                }),
            },
        )),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComputerInventory {
    pub computer_id: String,
    pub instance_id: String,
    pub harnesses: Vec<HarnessInfo>,
}

#[derive(Clone)]
pub struct BrokerComputerExecutor {
    broker: ComputerBroker,
    computer_id: String,
}
impl ComputerExecutor for BrokerComputerExecutor {
    fn execute(&self, mut assignment: AssignExecution) -> ComputerFuture<'_, ComputerCompletion> {
        Box::pin(async move {
            let execution_id = assignment
                .execution
                .as_ref()
                .map(|value| value.execution_id)
                .ok_or_else(|| ComputerError {
                    code: "assignment_missing_execution".into(),
                })?;
            let connection = self
                .broker
                .inner
                .connections
                .read()
                .map_err(|_| ComputerError {
                    code: "broker_lock_poisoned".into(),
                })?
                .get(&self.computer_id)
                .cloned()
                .ok_or_else(|| ComputerError {
                    code: "computer_unavailable".into(),
                })?;
            let harness_id = assignment
                .harness
                .as_ref()
                .map(|harness| harness.harness_id.clone())
                .unwrap_or_default();
            if !supports(&connection, &harness_id, "resume")
                && let Some(session) = assignment.session.as_mut()
            {
                session.action = lankide_protocol::SessionAction::StartNew as i32;
                session.native_session_id = None;
            }
            let execution = assignment.execution.as_ref().expect("validated execution");
            let offered_at = now_millis();
            if !self
                .broker
                .database()
                .map_err(|e| ComputerError {
                    code: e.to_string(),
                })?
                .execution_computer_matches(
                    execution_id,
                    i64::from(execution.attempt),
                    &self.computer_id,
                )
                .await
                .map_err(|e| ComputerError {
                    code: e.to_string(),
                })?
            {
                return Err(ComputerError {
                    code: "computer_placement_changed".into(),
                });
            }
            self.broker
                .database()
                .map_err(|error| ComputerError {
                    code: error.to_string(),
                })?
                .offer_execution_attempt(
                    execution_id,
                    i64::from(execution.attempt),
                    &self.computer_id,
                    &connection.instance_id,
                    offered_at.saturating_add(
                        self.broker
                            .inner
                            .attempt_lease_millis
                            .load(Ordering::Relaxed),
                    ),
                    offered_at,
                )
                .await
                .map_err(|error| ComputerError {
                    code: error.to_string(),
                })?;
            let (completion, receiver) = oneshot::channel();
            {
                let mut pending = self
                    .broker
                    .inner
                    .pending
                    .lock()
                    .map_err(|_| ComputerError {
                        code: "broker_lock_poisoned".into(),
                    })?;
                if pending.contains_key(&execution_id) {
                    return Err(ComputerError {
                        code: "assignment_already_pending".into(),
                    });
                }
                pending.insert(
                    execution_id,
                    PendingExecution {
                        computer_id: self.computer_id.clone(),
                        instance_id: connection.instance_id.clone(),
                        attempt: i64::from(execution.attempt),
                        harness_id,
                        completion,
                        assignment_accepted: false,
                        delivery_accepted: false,
                        max_saved_sequence: 0,
                        output: String::new(),
                    },
                );
            }
            if connection
                .sender
                .send(Ok(RuntimeFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    frame: Some(runtime_frame::Frame::AssignExecution(assignment)),
                }))
                .await
                .is_err()
            {
                // The transport cannot prove whether the Computer received the
                // assignment. Let the offered attempt's lease/reconciliation decide.
                self.broker
                    .database()
                    .map_err(|error| ComputerError {
                        code: error.to_string(),
                    })?
                    .mark_execution_recovering(execution_id, now_millis())
                    .await
                    .map_err(|error| ComputerError {
                        code: error.to_string(),
                    })?;
            }
            receiver.await.unwrap_or_else(|_| {
                Err(ComputerError {
                    code: "computer_disconnected".into(),
                })
            })
        })
    }
}

fn execution_event_kind_name(kind: ExecutionEventKind) -> &'static str {
    match kind {
        ExecutionEventKind::Started => "started",
        ExecutionEventKind::NativeSessionBound => "native_session_bound",
        ExecutionEventKind::AssistantDelta => "assistant_delta",
        ExecutionEventKind::ReasoningDelta => "reasoning_delta",
        ExecutionEventKind::ToolStarted => "tool_started",
        ExecutionEventKind::ToolFinished => "tool_finished",
        ExecutionEventKind::Usage => "usage",
        ExecutionEventKind::Warning => "warning",
        ExecutionEventKind::Completed => "completed",
        ExecutionEventKind::Failed => "failed",
        ExecutionEventKind::Cancelled => "cancelled",
        ExecutionEventKind::Unspecified => "unspecified",
    }
}

// Revocation wins over queued business frames, even when the output queue is full.
struct RegisteredComputerStream {
    receiver: mpsc::Receiver<Result<RuntimeFrame, Status>>,
    revoked: Arc<AtomicBool>,
    closed: bool,
}
impl tokio_stream::Stream for RegisteredComputerStream {
    type Item = Result<RuntimeFrame, Status>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.closed {
            return std::task::Poll::Ready(None);
        }
        if self.revoked.load(Ordering::Acquire) {
            self.closed = true;
            return std::task::Poll::Ready(Some(Err(Status::unauthenticated("computer_revoked"))));
        }
        self.receiver.poll_recv(cx)
    }
}
