use crate::execution::{abandon_execution, commit_execution_transaction, lose_attempt_transaction};
use crate::{ExecutionState, Result, Storage, StorageError};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAttemptState {
    Offered,
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    Lost,
    Rejected,
}
impl ExecutionAttemptState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offered => "offered",
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Lost => "lost",
            Self::Rejected => "rejected",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "offered" => Ok(Self::Offered),
            "accepted" => Ok(Self::Accepted),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "lost" => Ok(Self::Lost),
            "rejected" => Ok(Self::Rejected),
            _ => Err(StorageError::Invalid(format!(
                "execution attempt state {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAttemptView {
    pub execution_id: i64,
    pub attempt: i64,
    pub computer_id: String,
    pub computer_instance_id: String,
    pub state: ExecutionAttemptState,
    pub last_event_sequence: u64,
    pub native_session_id: Option<String>,
    pub lease_expires_at: i64,
    pub failure: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptCapability {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub kind: i32,
    pub input_schema_json: String,
    pub output_schema_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptAuthorization {
    pub execution_id: i64,
    pub attempt: i64,
    pub context_json: String,
    pub context_fingerprint: String,
    pub capabilities: Vec<AttemptCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEventRecord {
    pub execution_id: i64,
    pub attempt: i64,
    pub sequence: u64,
    pub kind: String,
    pub payload: Vec<u8>,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordExecutionEvent {
    pub execution_id: i64,
    pub attempt: i64,
    pub computer_id: String,
    pub computer_instance_id: String,
    pub sequence: u64,
    pub kind: String,
    pub payload: Vec<u8>,
    pub occurred_at: i64,
    pub now: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionEventDisposition {
    Saved,
    Duplicate,
    Committed,
    AlreadyCommitted,
    Abandoned,
    AlreadyAbandoned,
    StaleAttempt,
    SequenceGap,
}
impl ExecutionEventDisposition {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Committed | Self::AlreadyCommitted | Self::Abandoned | Self::AlreadyAbandoned
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEventReceipt {
    pub max_saved_sequence: u64,
    pub disposition: ExecutionEventDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct HeartbeatReconciliation {
    pub continued: Vec<(i64, i64)>,
    pub lost: Vec<(i64, i64)>,
}
fn load_execution_attempt(
    connection: &Connection,
    execution_id: i64,
    attempt: i64,
) -> Result<ExecutionAttemptView> {
    let row = connection.query_row(
        "SELECT computer_id,computer_instance_id,state,last_event_sequence,native_session_id,lease_expires_at,failure_json FROM execution_attempts WHERE execution_id=?1 AND attempt=?2",
        params![execution_id, attempt],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, i64>(5)?, row.get::<_, Option<String>>(6)?)),
    )?;
    Ok(ExecutionAttemptView {
        execution_id,
        attempt,
        computer_id: row.0,
        computer_instance_id: row.1,
        state: ExecutionAttemptState::parse(&row.2)?,
        last_event_sequence: row.3 as u64,
        native_session_id: row.4,
        lease_expires_at: row.5,
        failure: row
            .6
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| StorageError::Invalid(error.to_string()))?,
    })
}

fn load_attempt_capabilities(
    connection: &Connection,
    execution_id: i64,
    attempt: i64,
) -> Result<Vec<AttemptCapability>> {
    let mut statement = connection.prepare(
        "SELECT name,display_name,description,kind,input_schema_json,output_schema_json FROM execution_attempt_capabilities WHERE execution_id=?1 AND attempt=?2 ORDER BY position",
    )?;
    Ok(statement
        .query_map(params![execution_id, attempt], |row| {
            Ok(AttemptCapability {
                name: row.get(0)?,
                display_name: row.get(1)?,
                description: row.get(2)?,
                kind: row.get(3)?,
                input_schema_json: row.get(4)?,
                output_schema_json: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn durable_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "started"
            | "native_session_bound"
            | "tool_started"
            | "tool_finished"
            | "usage"
            | "warning"
            | "completed"
            | "failed"
            | "cancelled"
    )
}

fn event_final_output(payload: &[u8]) -> String {
    serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|value| {
            value
                .get("final_output")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(payload).into_owned())
}

impl Storage {
    pub fn freeze_attempt_authorization(&self, authorization: AttemptAuthorization) -> Result<()> {
        if authorization.attempt <= 0
            || authorization.context_json.trim().is_empty()
            || authorization.context_fingerprint.trim().is_empty()
        {
            return Err(StorageError::Invalid(
                "invalid attempt authorization".into(),
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        if authorization
            .capabilities
            .iter()
            .any(|capability| capability.name.trim().is_empty() || !names.insert(&capability.name))
        {
            return Err(StorageError::Invalid(
                "attempt capability names must be unique".into(),
            ));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let current_attempt: i64 = transaction.query_row(
            "SELECT attempt FROM executions WHERE execution_id=?1",
            [authorization.execution_id],
            |row| row.get(0),
        )?;
        if current_attempt != authorization.attempt {
            return Err(StorageError::Conflict("stale execution attempt".into()));
        }
        let existing: Option<(String, String)> = transaction
            .query_row(
                "SELECT context_json,context_fingerprint FROM execution_attempt_authorizations WHERE execution_id=?1 AND attempt=?2",
                params![authorization.execution_id, authorization.attempt],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((context_json, fingerprint)) = existing {
            let existing_capabilities = load_attempt_capabilities(
                &transaction,
                authorization.execution_id,
                authorization.attempt,
            )?;
            if context_json == authorization.context_json
                && fingerprint == authorization.context_fingerprint
                && existing_capabilities == authorization.capabilities
            {
                return Ok(());
            }
            return Err(StorageError::Conflict(
                "attempt authorization is immutable".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO execution_attempt_authorizations(execution_id,attempt,context_json,context_fingerprint) VALUES(?1,?2,?3,?4)",
            params![authorization.execution_id, authorization.attempt, authorization.context_json, authorization.context_fingerprint],
        )?;
        for (position, capability) in authorization.capabilities.into_iter().enumerate() {
            transaction.execute(
                "INSERT INTO execution_attempt_capabilities(execution_id,attempt,name,display_name,description,kind,input_schema_json,output_schema_json,position) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![authorization.execution_id, authorization.attempt, capability.name, capability.display_name, capability.description, capability.kind, capability.input_schema_json, capability.output_schema_json, position as i64],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn attempt_authorization(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Option<AttemptAuthorization>> {
        let connection = self.conn()?;
        let header: Option<(String, String)> = connection
            .query_row(
                "SELECT context_json,context_fingerprint FROM execution_attempt_authorizations WHERE execution_id=?1 AND attempt=?2",
                params![execution_id, attempt],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        header
            .map(|(context_json, context_fingerprint)| {
                Ok(AttemptAuthorization {
                    execution_id,
                    attempt,
                    context_json,
                    context_fingerprint,
                    capabilities: load_attempt_capabilities(&connection, execution_id, attempt)?,
                })
            })
            .transpose()
    }
    pub fn offer_execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        computer_id: &str,
        computer_instance_id: &str,
        lease_expires_at: i64,
        now: i64,
    ) -> Result<ExecutionAttemptView> {
        if attempt <= 0 || computer_id.is_empty() || computer_instance_id.is_empty() {
            return Err(StorageError::Invalid(
                "invalid execution attempt identity".into(),
            ));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let current: Option<(i64, String)> = transaction
            .query_row(
                "SELECT attempt,state FROM executions WHERE execution_id=?1",
                [execution_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_attempt, execution_state)) = current else {
            return Err(StorageError::Invalid("unknown execution".into()));
        };
        if current_attempt != attempt
            || !matches!(
                execution_state.as_str(),
                "queued" | "assigning" | "running" | "recovering"
            )
        {
            return Err(StorageError::Conflict("stale execution attempt".into()));
        }
        transaction.execute(
            "INSERT INTO execution_attempts(execution_id,attempt,computer_id,computer_instance_id,state,last_event_sequence,lease_expires_at,assigned_at) VALUES(?1,?2,?3,?4,'offered',0,?5,?6) ON CONFLICT(execution_id,attempt) DO NOTHING",
            params![execution_id, attempt, computer_id, computer_instance_id, lease_expires_at, now],
        )?;
        let view = load_execution_attempt(&transaction, execution_id, attempt)?;
        if view.computer_id != computer_id || view.computer_instance_id != computer_instance_id {
            return Err(StorageError::Conflict(
                "execution attempt belongs to another computer instance".into(),
            ));
        }
        transaction.execute(
            "UPDATE executions SET state='assigning',updated_at=?2 WHERE execution_id=?1 AND state IN('queued','recovering')",
            params![execution_id, now],
        )?;
        transaction.commit()?;
        Ok(view)
    }

    pub fn accept_execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        accepted: bool,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let state = if accepted { "accepted" } else { "rejected" };
        let changed = transaction.execute(
            "UPDATE execution_attempts SET state=?3,accepted_at=CASE WHEN ?4 THEN coalesce(accepted_at,?5) ELSE accepted_at END,finished_at=CASE WHEN ?4 THEN finished_at ELSE coalesce(finished_at,?5) END WHERE execution_id=?1 AND attempt=?2 AND state='offered'",
            params![execution_id, attempt, state, accepted, now],
        )? == 1;
        if changed {
            if accepted {
                transaction.execute("UPDATE executions SET state='running',started_at=coalesce(started_at,?2),updated_at=?2 WHERE execution_id=?1 AND attempt=?3", params![execution_id, now, attempt])?;
            } else {
                abandon_execution(
                    &transaction,
                    execution_id,
                    ExecutionState::Failed,
                    Some("assignment_rejected"),
                    now,
                )?;
            }
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn reject_execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        error_code: &str,
        error_message: &str,
        now: i64,
    ) -> Result<bool> {
        self.reject_execution_attempt_with_failure(
            execution_id,
            attempt,
            error_code,
            error_message,
            None,
            now,
        )
    }

    pub fn reject_execution_attempt_with_failure(
        &self,
        execution_id: i64,
        attempt: i64,
        error_code: &str,
        error_message: &str,
        failure: Option<&Value>,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let code = if error_code.trim().is_empty() {
            "assignment_rejected"
        } else {
            error_code.trim()
        };
        let changed = transaction.execute(
            "UPDATE execution_attempts SET state='rejected',error_code=?3,error_message=?4,failure_json=?6,finished_at=coalesce(finished_at,?5) WHERE execution_id=?1 AND attempt=?2 AND state='offered'",
            params![execution_id, attempt, code, error_message, now, failure.map(Value::to_string)],
        )? == 1;
        if changed {
            // A rejected assignment is generally caused by a stable local
            // configuration problem. Keep the Inbox pending, but avoid a hot
            // loop that immediately creates and rejects another execution.
            transaction.execute(
                "UPDATE agent_inbox_items SET available_at=max(available_at,?2) WHERE claimed_execution_id=?1 AND state!='consumed'",
                params![execution_id, now + 60_000],
            )?;
            abandon_execution(
                &transaction,
                execution_id,
                ExecutionState::Failed,
                Some(code),
                now,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn mark_attempt_delivery(
        &self,
        execution_id: i64,
        attempt: i64,
        delivery_id: i64,
        accepted: bool,
        now: i64,
    ) -> Result<bool> {
        let connection = self.conn()?;
        let current: i64 = connection.query_row(
            "SELECT count(*) FROM executions e JOIN execution_attempts a USING(execution_id) WHERE e.execution_id=?1 AND e.attempt=?2 AND a.attempt=?2 AND a.state IN('accepted','running')",
            params![execution_id, attempt],
            |row| row.get(0),
        )?;
        if current == 0 {
            return Ok(false);
        }
        drop(connection);
        if accepted {
            self.mark_delivery_delivered(delivery_id, now)
        } else {
            self.fail_attempt(execution_id, attempt, "delivery_rejected", now)
        }
    }

    pub fn execution_attempts(&self, execution_id: i64) -> Result<Vec<ExecutionAttemptView>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT attempt FROM execution_attempts WHERE execution_id=?1 ORDER BY attempt",
        )?;
        let attempts = statement
            .query_map([execution_id], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        attempts
            .into_iter()
            .map(|attempt| load_execution_attempt(&connection, execution_id, attempt))
            .collect()
    }

    pub fn execution_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Option<ExecutionAttemptView>> {
        let connection = self.conn()?;
        match load_execution_attempt(&connection, execution_id, attempt) {
            Ok(value) => Ok(Some(value)),
            Err(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn execution_events(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Vec<ExecutionEventRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare("SELECT sequence,kind,payload,occurred_at FROM execution_events WHERE execution_id=?1 AND attempt=?2 ORDER BY sequence")?;
        let rows = statement
            .query_map(params![execution_id, attempt], |row| {
                Ok(ExecutionEventRecord {
                    execution_id,
                    attempt,
                    sequence: row.get::<_, i64>(0)? as u64,
                    kind: row.get(1)?,
                    payload: row.get(2)?,
                    occurred_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn record_execution_event(
        &self,
        event: RecordExecutionEvent,
    ) -> Result<ExecutionEventReceipt> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let execution: Option<(i64, String)> = transaction
            .query_row(
                "SELECT attempt,state FROM executions WHERE execution_id=?1",
                [event.execution_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((current_attempt, execution_state)) = execution else {
            return Err(StorageError::Invalid("unknown execution".into()));
        };
        let attempt = transaction.query_row(
            "SELECT computer_id,computer_instance_id,state,last_event_sequence FROM execution_attempts WHERE execution_id=?1 AND attempt=?2",
            params![event.execution_id, event.attempt],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
        ).optional()?;
        let Some((computer_id, instance_id, attempt_state, last_sequence)) = attempt else {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: 0,
                disposition: ExecutionEventDisposition::StaleAttempt,
            });
        };
        if current_attempt != event.attempt
            || computer_id != event.computer_id
            || instance_id != event.computer_instance_id
            || matches!(attempt_state.as_str(), "lost" | "rejected")
        {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition: ExecutionEventDisposition::StaleAttempt,
            });
        }
        if event.sequence <= last_sequence as u64 {
            let disposition = match (execution_state.as_str(), event.kind.as_str()) {
                ("succeeded", "completed") => ExecutionEventDisposition::AlreadyCommitted,
                ("failed", "failed") | ("cancelled", "cancelled") => {
                    ExecutionEventDisposition::AlreadyAbandoned
                }
                _ => ExecutionEventDisposition::Duplicate,
            };
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition,
            });
        }
        if event.sequence != last_sequence as u64 + 1 {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition: ExecutionEventDisposition::SequenceGap,
            });
        }
        if !matches!(
            execution_state.as_str(),
            "assigning" | "running" | "recovering"
        ) {
            return Ok(ExecutionEventReceipt {
                max_saved_sequence: last_sequence as u64,
                disposition: ExecutionEventDisposition::StaleAttempt,
            });
        }
        if durable_event_kind(&event.kind) {
            transaction.execute(
                "INSERT INTO execution_events(execution_id,attempt,sequence,kind,payload,occurred_at) VALUES(?1,?2,?3,?4,?5,?6)",
                params![event.execution_id, event.attempt, event.sequence as i64, event.kind, event.payload, event.occurred_at],
            )?;
        }
        transaction.execute(
            "UPDATE execution_attempts SET last_event_sequence=?3,state=CASE WHEN ?4='started' THEN 'running' ELSE state END,native_session_id=CASE WHEN ?4='native_session_bound' THEN coalesce(json_extract(CAST(?5 AS TEXT),'$.native_session_id'),native_session_id) ELSE native_session_id END WHERE execution_id=?1 AND attempt=?2",
            params![event.execution_id, event.attempt, event.sequence as i64, event.kind, event.payload],
        )?;
        if matches!(event.kind.as_str(), "failed" | "cancelled") {
            let payload: Value = serde_json::from_slice(&event.payload).unwrap_or(Value::Null);
            if let Some(failure) = payload.get("failure").filter(|value| value.is_object()) {
                transaction.execute("UPDATE execution_attempts SET failure_json=?3,error_message=?4 WHERE execution_id=?1 AND attempt=?2", params![event.execution_id, event.attempt, failure.to_string(), failure.get("detail").and_then(Value::as_str)])?;
            }
        }
        let disposition = match event.kind.as_str() {
            "completed" => {
                let output = event_final_output(&event.payload);
                commit_execution_transaction(
                    &transaction,
                    event.execution_id,
                    event.attempt,
                    &output,
                    event.now,
                )?;
                ExecutionEventDisposition::Committed
            }
            "failed" => {
                transaction.execute("UPDATE execution_attempts SET state='failed',error_code=coalesce(json_extract(failure_json,'$.code'),'computer_execution_failed'),finished_at=?3 WHERE execution_id=?1 AND attempt=?2", params![event.execution_id, event.attempt, event.now])?;
                abandon_execution(
                    &transaction,
                    event.execution_id,
                    ExecutionState::Failed,
                    Some("computer_execution_failed"),
                    event.now,
                )?;
                ExecutionEventDisposition::Abandoned
            }
            "cancelled" => {
                transaction.execute("UPDATE execution_attempts SET state='cancelled',finished_at=?3 WHERE execution_id=?1 AND attempt=?2", params![event.execution_id, event.attempt, event.now])?;
                abandon_execution(
                    &transaction,
                    event.execution_id,
                    ExecutionState::Cancelled,
                    None,
                    event.now,
                )?;
                ExecutionEventDisposition::Abandoned
            }
            _ => ExecutionEventDisposition::Saved,
        };
        transaction.commit()?;
        Ok(ExecutionEventReceipt {
            max_saved_sequence: event.sequence,
            disposition,
        })
    }

    pub fn reconcile_computer_heartbeat(
        &self,
        computer_id: &str,
        instance_id: &str,
        active_attempts: &[(i64, i64)],
        now: i64,
        lease_expires_at: i64,
    ) -> Result<HeartbeatReconciliation> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare(
            "SELECT a.execution_id,a.attempt,a.computer_instance_id,a.state FROM execution_attempts a JOIN executions e USING(execution_id) WHERE a.computer_id=?1 AND e.attempt=a.attempt AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') ORDER BY a.execution_id",
        )?;
        let attempts = statement
            .query_map([computer_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let mut result = HeartbeatReconciliation::default();
        for (execution_id, attempt, stored_instance, state) in attempts {
            let reported = active_attempts.contains(&(execution_id, attempt));
            if stored_instance == instance_id && reported {
                transaction.execute("UPDATE execution_attempts SET lease_expires_at=?3 WHERE execution_id=?1 AND attempt=?2", params![execution_id, attempt, lease_expires_at])?;
                transaction.execute("UPDATE executions SET state='running',updated_at=?2 WHERE execution_id=?1 AND state='recovering'", params![execution_id, now])?;
                result.continued.push((execution_id, attempt));
            } else if stored_instance == instance_id && state == "offered" {
                // Assignment may still be in flight. Its original lease remains
                // authoritative until Computer accepts and reports it active.
            } else {
                lose_attempt_transaction(&transaction, execution_id, attempt, now)?;
                result.lost.push((execution_id, attempt));
            }
        }
        transaction.commit()?;
        Ok(result)
    }

    pub fn expire_execution_attempt_leases(&self, now: i64) -> Result<Vec<(i64, i64)>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare("SELECT a.execution_id,a.attempt FROM execution_attempts a JOIN executions e USING(execution_id) WHERE e.attempt=a.attempt AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') AND a.lease_expires_at<=?1 ORDER BY a.execution_id")?;
        let expired = statement
            .query_map([now], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        for (execution_id, attempt) in &expired {
            lose_attempt_transaction(&transaction, *execution_id, *attempt, now)?;
        }
        transaction.commit()?;
        Ok(expired)
    }

    pub fn fail_attempt(
        &self,
        execution_id: i64,
        attempt: i64,
        code: &str,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute("UPDATE execution_attempts SET state='failed',error_code=?3,finished_at=?4 WHERE execution_id=?1 AND attempt=?2 AND state IN('offered','accepted','running')", params![execution_id, attempt, code, now])? == 1;
        if changed {
            abandon_execution(
                &transaction,
                execution_id,
                ExecutionState::Failed,
                Some(code),
                now,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
}
