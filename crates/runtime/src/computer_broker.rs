use super::*;
use lankide_protocol::{
    ComputerFrame, ComputerHello, ExecutionEventKind, HarnessInfo, RuntimeFrame,
    RuntimeFrameStream, computer_frame, runtime_frame,
};
use lankide_storage::{ExecutionEventDisposition, RecordExecutionEvent, SteerDelivery, Storage};
use std::sync::{
    Mutex as StdMutex, RwLock as StdRwLock, Weak,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;

#[derive(Clone)]
pub struct ComputerBroker {
    inner: Arc<BrokerInner>,
}

struct BrokerInner {
    storage: Weak<Storage>,
    connections: StdRwLock<BTreeMap<String, ComputerConnection>>,
    next_connection_id: AtomicU64,
    pending: StdMutex<BTreeMap<i64, PendingExecution>>,
    pending_agent_deletions: StdMutex<BTreeMap<String, oneshot::Sender<Result<(), String>>>>,
}

#[derive(Clone)]
struct ComputerConnection {
    connection_id: u64,
    instance_id: String,
    sender: mpsc::Sender<Result<RuntimeFrame, Status>>,
    harnesses: Vec<HarnessInfo>,
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
    const ATTEMPT_LEASE_MILLIS: i64 = 15_000;

    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            inner: Arc::new(BrokerInner {
                storage: Arc::downgrade(&storage),
                connections: StdRwLock::new(BTreeMap::new()),
                next_connection_id: AtomicU64::new(0),
                pending: StdMutex::new(BTreeMap::new()),
                pending_agent_deletions: StdMutex::new(BTreeMap::new()),
            }),
        }
    }

    pub fn is_connected(&self, computer_id: &str) -> bool {
        self.inner
            .connections
            .read()
            .map(|values| values.contains_key(computer_id))
            .unwrap_or(false)
    }

    pub fn expire_leases(&self, now: i64) -> Result<Vec<(i64, i64)>, RuntimeError> {
        let expired = self.storage()?.expire_execution_attempt_leases(now)?;
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

    fn storage(&self) -> Result<Arc<Storage>, RuntimeError> {
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

    pub fn connect(&self, hello: ComputerHello) -> Result<RuntimeFrameStream, RuntimeError> {
        self.connect_tracked(hello).map(|(_, stream)| stream)
    }

    pub fn connect_tracked(
        &self,
        hello: ComputerHello,
    ) -> Result<(u64, RuntimeFrameStream), RuntimeError> {
        let (sender, receiver) = mpsc::channel(256);
        let connection_id = self
            .inner
            .next_connection_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        let connection = ComputerConnection {
            connection_id,
            instance_id: hello.instance_id.clone(),
            sender: sender.clone(),
            harnesses: hello.harnesses,
        };
        if let Some(previous) = self
            .inner
            .connections
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .insert(hello.computer_id.clone(), connection)
        {
            let _ = previous.sender.try_send(Ok(RuntimeFrame {
                frame: Some(runtime_frame::Frame::Shutdown(lankide_protocol::Shutdown {
                    reason: "replaced_by_reconnect".into(),
                })),
            }));
        }
        sender
            .try_send(Ok(RuntimeFrame {
                frame: Some(runtime_frame::Frame::Hello(
                    lankide_protocol::RuntimeHello {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        runtime_instance_id: format!("runtime-{}", std::process::id()),
                        heartbeat_interval_ms: 5_000,
                        max_message_bytes: lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES as u64,
                    },
                )),
            }))
            .map_err(|_| RuntimeError::Application("computer stream closed".into()))?;
        for delivery in self
            .storage()?
            .pending_steer_deliveries(&hello.computer_id, &hello.instance_id)?
        {
            if !self.can_steer(&delivery)? {
                continue;
            }
            sender
                .try_send(Ok(steer_frame(delivery)))
                .map_err(|_| RuntimeError::Application("computer stream closed".into()))?;
        }
        Ok((connection_id, Box::pin(ReceiverStream::new(receiver))))
    }

    pub fn disconnect(&self, computer_id: &str, instance_id: &str) -> Result<(), RuntimeError> {
        self.disconnect_matching(computer_id, instance_id, None)
    }

    pub fn disconnect_connection(
        &self,
        computer_id: &str,
        instance_id: &str,
        connection_id: u64,
    ) -> Result<(), RuntimeError> {
        self.disconnect_matching(computer_id, instance_id, Some(connection_id))
    }

    fn disconnect_matching(
        &self,
        computer_id: &str,
        instance_id: &str,
        connection_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let mut connections = self
            .inner
            .connections
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        let removed = if connections.get(computer_id).is_some_and(|connection| {
            connection.instance_id == instance_id
                && connection_id.is_none_or(|id| connection.connection_id == id)
        }) {
            connections.remove(computer_id);
            true
        } else {
            false
        };
        drop(connections);
        if !removed {
            return Ok(());
        }
        let mut pending = self
            .inner
            .pending
            .lock()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        let execution_ids = pending
            .iter()
            .filter_map(|(execution_id, value)| {
                (value.computer_id == computer_id).then_some(*execution_id)
            })
            .collect::<Vec<_>>();
        for execution_id in execution_ids {
            let Some(pending) = pending.remove(&execution_id) else {
                continue;
            };
            let _ = pending.completion.send(Err(ComputerError {
                code: "computer_disconnected".into(),
            }));
            let _ = self
                .storage()?
                .mark_execution_recovering(execution_id, now_millis());
        }
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
        match frame {
            computer_frame::Frame::AssignmentResult(result) => {
                let execution = result.execution.as_ref().ok_or_else(|| {
                    RuntimeError::Application("assignment missing execution".into())
                })?;
                let execution_id = execution.execution_id;
                if result.accepted {
                    self.storage()?.accept_execution_attempt(
                        execution_id,
                        i64::from(execution.attempt),
                        true,
                        now_millis(),
                    )?;
                } else {
                    self.storage()?.reject_execution_attempt_with_failure(
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
                    )?;
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
                let receipt = self
                    .storage()?
                    .record_execution_event(RecordExecutionEvent {
                        execution_id: execution.execution_id,
                        attempt: i64::from(execution.attempt),
                        computer_id: computer_id.to_owned(),
                        computer_instance_id: instance_id.to_owned(),
                        sequence: event.sequence,
                        kind: execution_event_kind_name(kind).to_owned(),
                        payload: event.payload.clone(),
                        occurred_at: event.occurred_at,
                        now: event.occurred_at,
                    })?;
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
                self.send_to(
                    computer_id,
                    instance_id,
                    RuntimeFrame {
                        frame: Some(runtime_frame::Frame::ExecutionEventAck(
                            lankide_protocol::ExecutionEventAck {
                                execution: Some(execution),
                                max_saved_sequence,
                            },
                        )),
                    },
                )
                .await?;
                if receipt.disposition.terminal() {
                    self.send_to(
                        computer_id,
                        instance_id,
                        RuntimeFrame {
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
                    self.storage()?.mark_steer_delivery_result(
                        -result.delivery_id,
                        execution_id,
                        i64::from(attempt),
                        computer_id,
                        instance_id,
                        result.accepted,
                        now_millis(),
                    )?;
                    return Ok(());
                }
                self.storage()?.mark_attempt_delivery(
                    execution_id,
                    i64::from(attempt),
                    result.delivery_id,
                    result.accepted,
                    now_millis(),
                )?;
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
                let reconciled = self.storage()?.reconcile_computer_heartbeat(
                    computer_id,
                    instance_id,
                    &active,
                    heartbeat.sent_at,
                    heartbeat.sent_at.saturating_add(Self::ATTEMPT_LEASE_MILLIS),
                )?;
                self.release_lost_attempts(&reconciled.lost, "execution_attempt_lost")?;
                self.expire_leases(heartbeat.sent_at)?;
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
            computer_frame::Frame::ToolCall(_) => {}
            computer_frame::Frame::Hello(_) => {
                return Err(RuntimeError::Application("duplicate computer hello".into()));
            }
        }
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
            self.storage()?
                .complete_agent_file_deletion(workspace_id, agent_id)?;
        }
        result
    }

    pub(crate) async fn retry_agent_file_deletions(
        &self,
        computer_id: &str,
    ) -> Result<(), RuntimeError> {
        let pending = self.storage()?.pending_agent_file_deletions(computer_id)?;
        for (workspace_id, agent_id) in pending {
            if let Err(error) = self
                .delete_agent_files(computer_id, &workspace_id, &agent_id)
                .await
            {
                eprintln!(
                    "[runtime] workspace={workspace_id} agent={agent_id} deferred file cleanup failed: {error}"
                );
            }
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
            self.broker
                .storage()
                .map_err(|error| ComputerError {
                    code: error.to_string(),
                })?
                .offer_execution_attempt(
                    execution_id,
                    i64::from(execution.attempt),
                    &self.computer_id,
                    &connection.instance_id,
                    offered_at.saturating_add(ComputerBroker::ATTEMPT_LEASE_MILLIS),
                    offered_at,
                )
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
                    frame: Some(runtime_frame::Frame::AssignExecution(assignment)),
                }))
                .await
                .is_err()
            {
                self.broker
                    .inner
                    .pending
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&execution_id));
                return Err(ComputerError {
                    code: "computer_disconnected".into(),
                });
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
