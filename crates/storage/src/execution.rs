use crate::agent::aid;
use crate::message::load_snapshot;
use crate::workspace::wid;
use crate::{DispatchReason, Result, SnapshotChatView, SnapshotView, Storage, StorageError};
use meerkat_core::{AgentId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::fmt;

type RecoverableExecutionRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    i64,
    String,
    Option<String>,
);
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SteerDelivery {
    pub delivery_id: i64,
    pub execution_id: i64,
    pub attempt: i64,
    pub machine_id: String,
    pub machine_instance_id: String,
    pub resources: Vec<crate::ResourceView>,
    pub rendered_markdown: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedExecution {
    pub execution_id: i64,
    pub snapshot_id: i64,
    pub delivery_id: i64,
    pub inbox_item_ids: Vec<i64>,
    pub chats: Vec<SnapshotChatView>,
    pub snapshot: SnapshotView,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Queued,
    Assigning,
    Running,
    Recovering,
    Succeeded,
    Failed,
    Cancelled,
}
impl ExecutionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Assigning => "assigning",
            Self::Running => "running",
            Self::Recovering => "recovering",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    fn parse(s: &str) -> Result<Self> {
        match s {
            "queued" => Ok(Self::Queued),
            "assigning" => Ok(Self::Assigning),
            "running" => Ok(Self::Running),
            "recovering" => Ok(Self::Recovering),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(StorageError::Invalid(format!("execution state {s}"))),
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionView {
    pub execution_id: i64,
    pub snapshot_id: i64,
    pub state: ExecutionState,
    pub final_output: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAuthSpec {
    pub attempt: i64,
    pub token_hash: String,
    pub expires_at: i64,
}
#[derive(Clone, Eq, PartialEq)]
pub struct ExecutionAuthContext {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub execution_id: i64,
    pub attempt: i64,
    pub token_hash: String,
    pub expires_at: i64,
    pub state: ExecutionState,
}
impl fmt::Debug for ExecutionAuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionAuthContext")
            .field("workspace_id", &self.workspace_id)
            .field("agent_id", &self.agent_id)
            .field("execution_id", &self.execution_id)
            .field("attempt", &self.attempt)
            .field("token_hash", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("state", &self.state)
            .finish()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitResult {
    Committed,
    AlreadyCommitted,
    NotReady,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableExecutionView {
    pub execution: ExecutionView,
    pub snapshot: SnapshotView,
    pub delivery_id: i64,
    pub delivery_state: String,
}
fn steer_delivery_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SteerDelivery> {
    Ok(SteerDelivery {
        delivery_id: -row.get::<_, i64>(0)?,
        execution_id: row.get(1)?,
        attempt: row.get(2)?,
        machine_id: row.get(3)?,
        machine_instance_id: row.get(4)?,
        rendered_markdown: row.get(5)?,
        created_at: row.get(6)?,
        resources: serde_json::from_str(&row.get::<_, String>(7)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}
fn state(c: &Connection, id: i64) -> Result<ExecutionState> {
    let s: String = c.query_row(
        "SELECT state FROM executions WHERE execution_id=?1",
        [id],
        |r| r.get(0),
    )?;
    ExecutionState::parse(&s)
}

pub(super) fn expand_execution_chat_boundary(
    transaction: &Transaction<'_>,
    execution_id: i64,
    snapshot_id: i64,
    chat_id: i64,
    read_to_message_id: i64,
    now: i64,
) -> Result<()> {
    let (read_from_message_id, current_boundary): (Option<i64>, i64) = transaction.query_row(
        "SELECT read_from_message_id,read_to_message_id FROM unread_snapshot_chats WHERE snapshot_id=?1 AND chat_id=?2",
        params![snapshot_id, chat_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let read_to_message_id = read_to_message_id.max(current_boundary);
    let unread_count: i64 = transaction.query_row(
        "SELECT count(*) FROM messages WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent')",
        params![chat_id, read_from_message_id.unwrap_or(0), read_to_message_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE unread_snapshot_chats SET read_to_message_id=max(read_to_message_id,?3),unread_count=?4 WHERE snapshot_id=?1 AND chat_id=?2",
        params![snapshot_id, chat_id, read_to_message_id, unread_count],
    )?;

    transaction.execute(
        "DELETE FROM unread_snapshot_messages WHERE snapshot_id=?1 AND chat_id=?2",
        params![snapshot_id, chat_id],
    )?;
    let mut recent = transaction.prepare(
        "SELECT message_id FROM messages WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent') ORDER BY message_id DESC LIMIT 3",
    )?;
    let mut recent_ids = recent
        .query_map(
            params![
                chat_id,
                read_from_message_id.unwrap_or(0),
                read_to_message_id
            ],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    recent_ids.reverse();
    drop(recent);
    for (position, message_id) in recent_ids.into_iter().enumerate() {
        transaction.execute(
            "INSERT INTO unread_snapshot_messages(snapshot_id,chat_id,message_id,position) VALUES(?1,?2,?3,?4)",
            params![snapshot_id, chat_id, message_id, position as i64],
        )?;
    }

    let (delivery_id, delivery_state): (i64, String) = transaction.query_row(
        "SELECT delivery_id,state FROM execution_deliveries WHERE execution_id=?1 ORDER BY delivery_id DESC LIMIT 1",
        [execution_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    transaction.execute(
        "INSERT INTO execution_delivery_chats(delivery_id,chat_id,read_to_message_id) VALUES(?1,?2,?3) ON CONFLICT(delivery_id,chat_id) DO UPDATE SET read_to_message_id=max(read_to_message_id,excluded.read_to_message_id)",
        params![delivery_id, chat_id, read_to_message_id],
    )?;
    let inbox_state = if delivery_state == "delivered" {
        "delivered"
    } else {
        "claimed"
    };

    let mut pending = transaction.prepare(
        "SELECT inbox_item_id FROM agent_inbox_items WHERE workspace_id=(SELECT workspace_id FROM executions WHERE execution_id=?1) AND agent_id=(SELECT agent_id FROM executions WHERE execution_id=?1) AND chat_id=?2 AND state='pending' AND available_at<=?3 AND ((source_kind='message' AND source_id<=?4) OR (source_kind='reaction' AND (SELECT target_message_id FROM reactions WHERE reaction_id=-source_id)<=?4)) ORDER BY inbox_item_id",
    )?;
    let pending_ids = pending
        .query_map(
            params![execution_id, chat_id, now, read_to_message_id],
            |row| row.get::<_, i64>(0),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(pending);
    for inbox_item_id in pending_ids {
        transaction.execute(
            "INSERT OR IGNORE INTO execution_delivery_items(delivery_id,inbox_item_id) VALUES(?1,?2)",
            params![delivery_id, inbox_item_id],
        )?;
        transaction.execute(
            "UPDATE agent_inbox_items SET state=?4,claimed_execution_id=?2,updated_at=?3 WHERE inbox_item_id=?1 AND state='pending'",
            params![inbox_item_id, execution_id, now, inbox_state],
        )?;
    }
    Ok(())
}

// Sparse receipts must not claim messages in the gaps between delivered inputs.
pub(super) fn record_delivered_messages(
    transaction: &Transaction<'_>,
    execution_id: i64,
    chat_id: i64,
    message_ids: &[i64],
    now: i64,
) -> Result<()> {
    let delivery_id: i64 = transaction.query_row(
        "SELECT delivery_id FROM execution_deliveries WHERE execution_id=?1 ORDER BY delivery_id DESC LIMIT 1",
        [execution_id], |row| row.get(0),
    )?;
    for message_id in message_ids {
        transaction.execute(
            "INSERT OR IGNORE INTO execution_delivery_items(delivery_id,inbox_item_id) SELECT ?1,inbox_item_id FROM agent_inbox_items WHERE workspace_id=(SELECT workspace_id FROM executions WHERE execution_id=?2) AND agent_id=(SELECT agent_id FROM executions WHERE execution_id=?2) AND chat_id=?3 AND source_kind='message' AND source_id=?4 AND state='pending'",
            params![delivery_id,execution_id,chat_id,message_id],
        )?;
        transaction.execute(
            "UPDATE agent_inbox_items SET state='delivered',claimed_execution_id=?2,updated_at=?3 WHERE inbox_item_id IN(SELECT inbox_item_id FROM execution_delivery_items WHERE delivery_id=?1) AND source_kind='message' AND source_id=?4 AND state='pending'",
            params![delivery_id,execution_id,now,message_id],
        )?;
        transaction.execute(
            "INSERT INTO execution_delivery_chats(delivery_id,chat_id,read_to_message_id) VALUES(?1,?2,?3) ON CONFLICT(delivery_id,chat_id) DO UPDATE SET read_to_message_id=max(read_to_message_id,excluded.read_to_message_id)",
            params![delivery_id,chat_id,message_id],
        )?;
    }
    Ok(())
}

pub(super) fn commit_execution_transaction(
    transaction: &Transaction<'_>,
    execution_id: i64,
    attempt: i64,
    output: &str,
    now: i64,
) -> Result<()> {
    transaction.execute("UPDATE agents SET failure_fingerprint=NULL,failure_streak=0 WHERE (workspace_id,agent_id)=(SELECT workspace_id,agent_id FROM executions WHERE execution_id=?1)",[execution_id])?;

    let (workspace_id, agent_id, snapshot_id, current_attempt, state): (String, String, i64, i64, String) = transaction.query_row(
        "SELECT workspace_id,agent_id,snapshot_id,attempt,state FROM executions WHERE execution_id=?1",
        [execution_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    if attempt > 0 && current_attempt != attempt {
        return Err(StorageError::Conflict("stale execution attempt".into()));
    }
    if state == "succeeded" {
        return Ok(());
    }
    if !matches!(state.as_str(), "assigning" | "running" | "recovering") {
        return Err(StorageError::Conflict(
            "execution is not committable".into(),
        ));
    }
    let delivery_ready: i64 = transaction.query_row(
        "SELECT count(*) FROM execution_deliveries WHERE execution_id=?1 AND state='delivered'",
        [execution_id],
        |row| row.get(0),
    )?;
    if delivery_ready == 0 {
        return Err(StorageError::Conflict(
            "execution delivery is not durable".into(),
        ));
    }
    let mut statement = transaction.prepare("SELECT dc.chat_id,max(dc.read_to_message_id) FROM execution_delivery_chats dc JOIN execution_deliveries d USING(delivery_id) WHERE d.execution_id=?1 AND d.state='delivered' GROUP BY dc.chat_id")?;
    let cursors = statement
        .query_map([execution_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for (chat_id, read_to_message_id) in cursors {
        let read_to_message_id: i64 = transaction.query_row(
            "SELECT min(?4,coalesce(min(source_id)-1,?4)) FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 AND chat_id=?3 AND source_kind='message' AND state='pending'",
            params![workspace_id,agent_id,chat_id,read_to_message_id], |row| row.get(0),
        )?;
        transaction.execute("INSERT INTO agent_chat_cursors VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,agent_id,chat_id) DO UPDATE SET read_to_message_id=max(read_to_message_id,excluded.read_to_message_id),updated_at=excluded.updated_at", params![workspace_id,agent_id,chat_id,read_to_message_id,now])?;
    }
    transaction.execute("UPDATE agent_inbox_items SET state='consumed',updated_at=?2 WHERE claimed_execution_id=?1 AND state='delivered'", params![execution_id,now])?;
    transaction.execute("UPDATE execution_deliveries SET state='consumed' WHERE execution_id=?1 AND state='delivered'", [execution_id])?;
    transaction.execute(
        "UPDATE unread_snapshots SET state='consumed',completed_at=?2 WHERE snapshot_id=?1",
        params![snapshot_id, now],
    )?;
    if attempt > 0 {
        transaction.execute("UPDATE execution_attempts SET state='completed',finished_at=?3 WHERE execution_id=?1 AND attempt=?2", params![execution_id,attempt,now])?;
    }
    transaction.execute("UPDATE executions SET state='succeeded',final_output=?2,completed_at=?3,updated_at=?3 WHERE execution_id=?1", params![execution_id,output,now])?;
    Ok(())
}

pub(super) fn abandon_execution(
    transaction: &Transaction<'_>,
    execution_id: i64,
    end: ExecutionState,
    error: Option<&str>,
    now: i64,
) -> Result<()> {
    let snapshot_id: i64 = transaction.query_row(
        "SELECT snapshot_id FROM executions WHERE execution_id=?1",
        [execution_id],
        |row| row.get(0),
    )?;
    transaction.execute("UPDATE agent_inbox_items SET state='pending',claimed_execution_id=NULL,updated_at=?2 WHERE claimed_execution_id=?1 AND state!='consumed'", params![execution_id,now])?;
    transaction.execute(
        "UPDATE unread_snapshots SET state='abandoned',completed_at=?2 WHERE snapshot_id=?1",
        params![snapshot_id, now],
    )?;
    transaction.execute(
        "UPDATE execution_deliveries SET state='abandoned' WHERE execution_id=?1 AND state='pending'",
        [execution_id],
    )?;
    transaction.execute("UPDATE executions SET state=?2,error_code=?3,completed_at=?4,updated_at=?4 WHERE execution_id=?1", params![execution_id,end.as_str(),error,now])?;
    Ok(())
}

pub(super) fn lose_attempt_transaction(
    transaction: &Transaction<'_>,
    execution_id: i64,
    attempt: i64,
    now: i64,
) -> Result<()> {
    let failure = serde_json::json!({
        "code": "machine_lost",
        "category": "transient",
        "retryable": true,
        "fingerprint": "machine_lost",
        "detail": "Machine disconnected before the attempt completed",
    });
    let changed = transaction.execute("UPDATE execution_attempts SET state='lost',error_code='machine_lost',failure_json=?4,error_message=json_extract(?4,'$.detail'),finished_at=?3 WHERE execution_id=?1 AND attempt=?2 AND state IN('offered','accepted','running')", params![execution_id,attempt,now,failure.to_string()])? == 1;
    if changed {
        abandon_execution(
            transaction,
            execution_id,
            ExecutionState::Failed,
            Some("machine_lost"),
            now,
        )?;
    }
    Ok(())
}
pub(super) fn load_recoverable_executions(
    c: &Connection,
    workspace_id: Option<&WorkspaceId>,
) -> Result<Vec<RecoverableExecutionView>> {
    let mut sql="SELECT e.execution_id,e.snapshot_id,e.state,e.final_output,e.error_code,d.delivery_id,d.state,e.error_message FROM executions e JOIN execution_deliveries d USING(execution_id) WHERE e.state IN('queued','assigning','running','recovering')".to_owned();
    if workspace_id.is_some() {
        sql.push_str(" AND e.workspace_id=?1");
    }
    sql.push_str(" ORDER BY e.execution_id");
    let mut s = c.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RecoverableExecutionRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    };
    let rows = if let Some(w) = workspace_id {
        s.query_map([w.as_str()], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        s.query_map([], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    drop(s);
    let mut out = Vec::new();
    for (
        execution_id,
        snapshot_id,
        state,
        final_output,
        error_code,
        delivery_id,
        delivery_state,
        error_message,
    ) in rows
    {
        out.push(RecoverableExecutionView {
            execution: ExecutionView {
                execution_id,
                snapshot_id,
                state: ExecutionState::parse(&state)?,
                final_output,
                error_code,
                error_message,
            },
            snapshot: load_snapshot(c, snapshot_id)?,
            delivery_id,
            delivery_state,
        });
    }
    Ok(out)
}

impl Storage {
    pub fn create_execution(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
    ) -> Result<Option<CreatedExecution>> {
        self.create_execution_inner(w, a, summary, now, None)
    }
    pub fn create_execution_with_auth(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
        auth: ExecutionAuthSpec,
    ) -> Result<Option<CreatedExecution>> {
        if auth.attempt <= 0 {
            return Err(StorageError::Invalid(
                "execution attempt must be positive".into(),
            ));
        }
        if auth.token_hash.trim().is_empty() {
            return Err(StorageError::Invalid(
                "execution token hash is empty".into(),
            ));
        }
        if auth.expires_at <= now {
            return Err(StorageError::Invalid(
                "execution token already expired".into(),
            ));
        }
        self.create_execution_inner(w, a, summary, now, Some(&auth))
    }
    fn create_execution_inner(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        summary: &str,
        now: i64,
        auth: Option<&ExecutionAuthSpec>,
    ) -> Result<Option<CreatedExecution>> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let n:i64=t.query_row("SELECT count(*) FROM executions WHERE workspace_id=?1 AND agent_id=?2 AND state IN('queued','assigning','running','recovering')",params![w.as_str(),a.as_str()],|r|r.get(0))?;
        if n > 0 {
            return Err(StorageError::Invalid("active execution".into()));
        }
        let paused: bool = t.query_row(
            "SELECT pause_reason IS NOT NULL OR EXISTS(SELECT 1 FROM agent_relocations r WHERE r.workspace_id=?1 AND r.agent_id=?2 AND r.state IN('waiting','staged')) FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
            |r| r.get(0),
        )?;
        if paused {
            return Ok(None);
        }
        let mut s=t.prepare("SELECT inbox_item_id,chat_id,source_id,source_kind,trigger_reason_json FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 AND state='pending' AND available_at<=?3 AND coalesce(retry_batch_id,0)=coalesce((SELECT retry_batch_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 AND state='pending' ORDER BY inbox_item_id LIMIT 1),0) ORDER BY inbox_item_id")?;
        let p = s
            .query_map(params![w.as_str(), a.as_str(), now], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(s);
        if p.is_empty() {
            return Ok(None);
        }
        t.execute("INSERT INTO unread_snapshots(workspace_id,agent_id,state,created_at)VALUES(?1,?2,'open',?3)",params![w.as_str(),a.as_str(),now])?;
        let sid = t.last_insert_rowid();
        let mut map = std::collections::BTreeMap::<i64, Vec<i64>>::new();
        for (_, ch, source, source_kind, _) in &p {
            let message_id = trigger_message_id(&t, *ch, *source, source_kind)?;
            map.entry(*ch).or_default().push(message_id)
        }
        let mut chats = vec![];
        for (ch, _trigger_message_ids) in map {
            let to: i64 = t.query_row(
                "SELECT last_message_id FROM chats WHERE chat_id=?1",
                [ch],
                |r| r.get(0),
            )?;
            let from:Option<i64>=t.query_row("SELECT read_to_message_id FROM agent_chat_cursors WHERE workspace_id=?1 AND agent_id=?2 AND chat_id=?3",params![w.as_str(),a.as_str(),ch],|r|r.get(0)).optional()?.flatten();
            let unread_count:i64=t.query_row("SELECT count(*) FROM messages m WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent') AND NOT EXISTS(SELECT 1 FROM agent_inbox_items i WHERE i.workspace_id=?4 AND i.agent_id=?5 AND i.source_kind='message' AND i.source_id=m.message_id AND i.state='consumed')",params![ch,from.unwrap_or(0),to,w.as_str(),a.as_str()],|r|r.get(0))?;
            t.execute(
                "INSERT INTO unread_snapshot_chats VALUES(?1,?2,?3,?4,?5,?6)",
                params![sid, ch, from, to, unread_count, summary],
            )?;
            let mentioned_message_id = p.iter().rev().find_map(
                |(_, item_chat_id, source_id, source_kind, reason_json)| {
                    if *item_chat_id != ch
                        || serde_json::from_str::<DispatchReason>(reason_json).ok()
                            != Some(DispatchReason::Mention)
                    {
                        return None;
                    }
                    trigger_message_id(&t, ch, *source_id, source_kind).ok()
                },
            );
            let mut recent=t.prepare("SELECT message_id FROM messages m WHERE chat_id=?1 AND message_id>?2 AND message_id<=?3 AND send_status IN ('received','sent') AND NOT EXISTS(SELECT 1 FROM agent_inbox_items i WHERE i.workspace_id=?5 AND i.agent_id=?6 AND i.source_kind='message' AND i.source_id=m.message_id AND i.state='consumed') ORDER BY message_id DESC LIMIT ?4")?;
            let mut recent_ids = recent
                .query_map(
                    params![ch, from.unwrap_or(0), to, 3, w.as_str(), a.as_str()],
                    |r| r.get::<_, i64>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            recent_ids.reverse();
            drop(recent);
            if let Some(mentioned_message_id) = mentioned_message_id {
                if !recent_ids.contains(&mentioned_message_id) {
                    recent_ids = recent_ids.into_iter().rev().take(2).collect::<Vec<_>>();
                    recent_ids.reverse();
                    recent_ids.insert(0, mentioned_message_id);
                }
            }
            for (pos, m) in recent_ids.iter().enumerate() {
                t.execute(
                    "INSERT INTO unread_snapshot_messages VALUES(?1,?2,?3,?4)",
                    params![sid, ch, m, pos as i64],
                )?;
            }
            chats.push(SnapshotChatView {
                chat_id: ch,
                read_to_message_id: to,
                unread_count: unread_count as usize,
            })
        }
        for (position, (_, chat_id, source_id, source_kind, reason)) in p.iter().enumerate() {
            let message_id = trigger_message_id(&t, *chat_id, *source_id, source_kind)?;
            t.execute("INSERT INTO unread_snapshot_triggers(snapshot_id,chat_id,message_id,reason_json,position)VALUES(?1,?2,?3,?4,?5)",params![sid,chat_id,message_id,reason,position as i64])?;
        }
        let (attempt, token_hash, token_expires_at) = auth
            .map(|value| {
                (
                    value.attempt,
                    Some(value.token_hash.as_str()),
                    Some(value.expires_at),
                )
            })
            .unwrap_or((1, None, None));
        t.execute("INSERT INTO executions(workspace_id,agent_id,snapshot_id,state,trigger_summary,created_at,updated_at,attempt,token_hash,token_expires_at,placement_machine_id,placement_generation)VALUES(?1,?2,?3,'queued',?4,?5,?5,?6,?7,?8,(SELECT machine_id FROM agents WHERE workspace_id=?1 AND agent_id=?2),(SELECT placement_generation FROM agents WHERE workspace_id=?1 AND agent_id=?2))",params![w.as_str(),a.as_str(),sid,summary,now,attempt,token_hash,token_expires_at])?;
        let eid = t.last_insert_rowid();
        t.execute("INSERT INTO execution_deliveries(execution_id,state,created_at)VALUES(?1,'pending',?2)",params![eid,now])?;
        let did = t.last_insert_rowid();
        let ids = p.iter().map(|x| x.0).collect::<Vec<_>>();
        for id in &ids {
            t.execute(
                "INSERT INTO execution_delivery_items VALUES(?1,?2)",
                params![did, id],
            )?;
            t.execute("UPDATE agent_inbox_items SET state='claimed',claimed_execution_id=?1,updated_at=?2 WHERE inbox_item_id=?3",params![eid,now,id])?;
        }
        for ch in &chats {
            t.execute(
                "INSERT INTO execution_delivery_chats VALUES(?1,?2,?3)",
                params![did, ch.chat_id, ch.read_to_message_id],
            )?;
        }
        let snapshot = load_snapshot(&t, sid)?;
        t.commit()?;
        Ok(Some(CreatedExecution {
            execution_id: eid,
            snapshot_id: sid,
            delivery_id: did,
            inbox_item_ids: ids,
            chats,
            snapshot,
        }))
    }
    pub fn execution_auth_context(
        &self,
        execution_id: i64,
        attempt: i64,
    ) -> Result<Option<ExecutionAuthContext>> {
        let row: Option<(String, String, i64, String, i64, String)> = self
            .read_conn()?
            .query_row(
                "SELECT e.workspace_id,e.agent_id,e.attempt,e.token_hash,e.token_expires_at,e.state FROM executions e JOIN agents a ON a.workspace_id=e.workspace_id AND a.agent_id=e.agent_id WHERE e.execution_id=?1 AND e.attempt=?2 AND e.token_hash IS NOT NULL AND e.token_expires_at IS NOT NULL AND e.placement_machine_id=a.machine_id AND e.placement_generation=a.placement_generation AND NOT EXISTS(SELECT 1 FROM machines c WHERE c.machine_id=e.placement_machine_id AND c.revoked_at IS NOT NULL)",
                params![execution_id, attempt],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .optional()?;
        row.map(
            |(workspace, agent, stored_attempt, token_hash, expires_at, state)| {
                Ok(ExecutionAuthContext {
                    workspace_id: wid(workspace)?,
                    agent_id: aid(agent)?,
                    execution_id,
                    attempt: stored_attempt,
                    token_hash,
                    expires_at,
                    state: ExecutionState::parse(&state)?,
                })
            },
        )
        .transpose()
    }
    /// Verifies the assignment target against the frozen and current placement.
    pub fn execution_machine_matches(
        &self,
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
    ) -> Result<bool> {
        Ok(self.read_conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions e JOIN agents a ON a.workspace_id=e.workspace_id AND a.agent_id=e.agent_id WHERE e.execution_id=?1 AND e.attempt=?2 AND e.placement_machine_id=?3 AND a.machine_id=?3 AND e.placement_generation=a.placement_generation AND NOT EXISTS(SELECT 1 FROM machines c WHERE c.machine_id=?3 AND c.revoked_at IS NOT NULL))",
            params![execution_id, attempt, machine_id], |r| r.get(0))?)
    }

    /// Checks the frozen placement before accepting any Machine-side mutation.
    pub fn execution_placement_matches(
        &self,
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
    ) -> Result<bool> {
        Ok(self.read_conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions e JOIN agents a ON a.workspace_id=e.workspace_id AND a.agent_id=e.agent_id JOIN execution_attempts t ON t.execution_id=e.execution_id AND t.attempt=e.attempt WHERE e.execution_id=?1 AND e.attempt=?2 AND t.machine_id=?3 AND e.placement_machine_id=t.machine_id AND e.placement_machine_id=a.machine_id AND e.placement_generation=a.placement_generation AND NOT EXISTS(SELECT 1 FROM machines c WHERE c.machine_id=?3 AND c.revoked_at IS NOT NULL))",
            params![execution_id, attempt, machine_id], |r| r.get(0))?)
    }

    pub fn mark_execution_running(&self, id: i64, now: i64) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE executions SET state='running',started_at=coalesce(started_at,?2),updated_at=?2 WHERE execution_id=?1 AND state IN('queued','assigning','recovering')",params![id,now])?==1)
    }
    pub fn mark_execution_recovering(&self, id: i64, now: i64) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE executions SET state='recovering',updated_at=?2 WHERE execution_id=?1 AND state IN('queued','assigning','running','recovering')",params![id,now])?==1)
    }
    pub fn mark_delivery_delivered(&self, id: i64, now: i64) -> Result<bool> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let ok=t.execute("UPDATE execution_deliveries SET state='delivered',delivered_at=?2 WHERE delivery_id=?1 AND state='pending'",params![id,now])?==1;
        if ok {
            t.execute("UPDATE agent_inbox_items SET state='delivered',updated_at=?2 WHERE inbox_item_id IN(SELECT inbox_item_id FROM execution_delivery_items WHERE delivery_id=?1) AND state='claimed'",params![id,now])?;
        }
        t.commit()?;
        Ok(ok)
    }
    pub fn create_steer_delivery(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        inbox_item_id: i64,
        rendered_markdown: &str,
        now: i64,
    ) -> Result<Option<SteerDelivery>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let active: Option<(i64, i64, String, String)> = transaction
            .query_row(
                "SELECT e.execution_id,e.attempt,a.machine_id,a.machine_instance_id FROM executions e JOIN execution_attempts a ON a.execution_id=e.execution_id AND a.attempt=e.attempt WHERE e.workspace_id=?1 AND e.agent_id=?2 AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') ORDER BY e.execution_id DESC LIMIT 1",
                params![workspace_id.as_str(), agent_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((execution_id, attempt, machine_id, machine_instance_id)) = active else {
            return Ok(None);
        };
        transaction.execute(
            "INSERT INTO steer_deliveries(execution_id,attempt,inbox_item_id,machine_id,machine_instance_id,rendered_markdown,state,created_at) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7) ON CONFLICT(execution_id,attempt,inbox_item_id) DO NOTHING",
            params![execution_id,attempt,inbox_item_id,machine_id,machine_instance_id,rendered_markdown,now],
        )?;
        let delivery = transaction.query_row(
            "SELECT steer_delivery_id,execution_id,attempt,machine_id,machine_instance_id,rendered_markdown,created_at,(SELECT m.resources_json FROM agent_inbox_items i JOIN messages m ON m.message_id=i.source_id WHERE i.inbox_item_id=steer_deliveries.inbox_item_id) FROM steer_deliveries WHERE execution_id=?1 AND attempt=?2 AND inbox_item_id=?3",
            params![execution_id, attempt, inbox_item_id],
            steer_delivery_row,
        )?;
        transaction.commit()?;
        Ok(Some(delivery))
    }
    pub fn pending_steer_deliveries(
        &self,
        machine_id: &str,
        machine_instance_id: &str,
    ) -> Result<Vec<SteerDelivery>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare(
            "SELECT s.steer_delivery_id,s.execution_id,s.attempt,s.machine_id,s.machine_instance_id,s.rendered_markdown,s.created_at,(SELECT m.resources_json FROM agent_inbox_items i JOIN messages m ON m.message_id=i.source_id WHERE i.inbox_item_id=s.inbox_item_id) FROM steer_deliveries s JOIN executions e USING(execution_id) JOIN execution_attempts a ON a.execution_id=s.execution_id AND a.attempt=s.attempt WHERE s.machine_id=?1 AND s.machine_instance_id=?2 AND s.state='pending' AND e.attempt=s.attempt AND e.state IN('assigning','running','recovering') AND a.state IN('offered','accepted','running') ORDER BY s.steer_delivery_id",
        )?;
        Ok(statement
            .query_map(params![machine_id, machine_instance_id], steer_delivery_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn mark_steer_delivery_result(
        &self,
        delivery_id: i64,
        execution_id: i64,
        attempt: i64,
        machine_id: &str,
        machine_instance_id: &str,
        accepted: bool,
        now: i64,
    ) -> Result<bool> {
        let state = if accepted { "accepted" } else { "pending" };
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE steer_deliveries SET state=?6,accepted_at=CASE WHEN ?7 THEN ?8 ELSE accepted_at END,last_error_at=CASE WHEN ?7 THEN last_error_at ELSE ?8 END WHERE steer_delivery_id=?1 AND execution_id=?2 AND attempt=?3 AND machine_id=?4 AND machine_instance_id=?5 AND state='pending' AND EXISTS(SELECT 1 FROM executions e JOIN execution_attempts a ON a.execution_id=e.execution_id AND a.attempt=e.attempt WHERE e.execution_id=?2 AND e.attempt=?3 AND e.state IN('assigning','running','recovering') AND a.machine_id=?4 AND a.machine_instance_id=?5 AND a.state IN('offered','accepted','running'))",
            params![delivery_id,execution_id,attempt,machine_id,machine_instance_id,state, accepted, now],
        )? == 1;
        if changed && accepted {
            let (snapshot_id, chat_id, message_id): (i64, i64, i64) = transaction.query_row(
                "SELECT e.snapshot_id,i.chat_id,i.source_id FROM steer_deliveries s JOIN executions e USING(execution_id) JOIN agent_inbox_items i USING(inbox_item_id) WHERE s.steer_delivery_id=?1", [delivery_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            transaction.execute(
                "INSERT OR IGNORE INTO unread_snapshot_chats(snapshot_id,chat_id,read_from_message_id,read_to_message_id,unread_count,trigger_summary) SELECT snapshot_id,?2,(SELECT read_to_message_id FROM agent_chat_cursors WHERE workspace_id=s.workspace_id AND agent_id=s.agent_id AND chat_id=?2),?3,0,'human_steer' FROM unread_snapshots s WHERE snapshot_id=?1",
                params![snapshot_id,chat_id,message_id])?;
            // Extend lookup scope, but acknowledge only the input actually sent.
            transaction.execute(
                "UPDATE unread_snapshot_chats SET read_to_message_id=max(read_to_message_id,?3),unread_count=(SELECT count(*) FROM messages WHERE chat_id=?2 AND message_id>coalesce(read_from_message_id,0) AND message_id<=max(read_to_message_id,?3) AND send_status IN ('received','sent')) WHERE snapshot_id=?1 AND chat_id=?2",
                params![snapshot_id, chat_id, message_id],
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO unread_snapshot_messages(snapshot_id,chat_id,message_id,position) VALUES(?1,?2,?3,?3)",
                params![snapshot_id, chat_id, message_id],
            )?;
            record_delivered_messages(&transaction, execution_id, chat_id, &[message_id], now)?;
            transaction.execute(
                "UPDATE agent_inbox_items SET state='delivered',claimed_execution_id=?2,updated_at=?3 WHERE inbox_item_id=(SELECT inbox_item_id FROM steer_deliveries WHERE steer_delivery_id=?1) AND state='pending'",
                params![delivery_id, execution_id, now],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
    pub fn complete_execution(&self, id: i64, out: &str, now: i64) -> Result<bool> {
        Ok(matches!(
            self.commit_execution(id, out, None, now)?,
            CommitResult::Committed
        ))
    }
    pub fn commit_execution(
        &self,
        id: i64,
        out: &str,
        response_send_request_key: Option<&str>,
        now: i64,
    ) -> Result<CommitResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let current = state(&t, id)?;
        if current == ExecutionState::Succeeded {
            return Ok(CommitResult::AlreadyCommitted);
        }
        if current.terminal() {
            return Ok(CommitResult::NotReady);
        }
        let delivery_ready: i64 = t.query_row(
            "SELECT count(*) FROM execution_deliveries WHERE execution_id=?1 AND state='delivered'",
            [id],
            |r| r.get(0),
        )?;
        if delivery_ready == 0 {
            return Ok(CommitResult::NotReady);
        }
        if let Some(key) = response_send_request_key {
            let sent:i64=t.query_row("SELECT count(*) FROM message_send_attempts a JOIN messages m USING(message_id) WHERE a.request_key=?1 AND a.execution_id=?2 AND a.state='sent' AND m.send_status='sent'",params![key,id],|r|r.get(0))?;
            if sent == 0 {
                return Ok(CommitResult::NotReady);
            }
        }
        commit_execution_transaction(&t, id, 0, out, now)?;
        t.commit()?;
        Ok(CommitResult::Committed)
    }
    pub fn fail_execution(&self, id: i64, e: &str, now: i64) -> Result<bool> {
        self.unsuccessful(id, ExecutionState::Failed, Some(e), now)
    }
    pub fn fail_execution_retry_after(
        &self,
        id: i64,
        error: &str,
        detail: &str,
        retry_at: i64,
        now: i64,
    ) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        if state(&transaction, id)?.terminal() {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE agent_inbox_items SET available_at=max(available_at,?2) WHERE claimed_execution_id=?1 AND state!='consumed'",
            params![id, retry_at],
        )?;
        abandon_execution(&transaction, id, ExecutionState::Failed, Some(error), now)?;
        transaction.execute(
            "UPDATE executions SET error_message=?2 WHERE execution_id=?1",
            params![id, detail],
        )?;
        transaction.commit()?;
        Ok(true)
    }
    pub fn cancel_execution(&self, id: i64, now: i64) -> Result<bool> {
        self.unsuccessful(id, ExecutionState::Cancelled, None, now)
    }
    fn unsuccessful(
        &self,
        id: i64,
        end: ExecutionState,
        err: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if state(&t, id)?.terminal() {
            return Ok(false);
        }
        abandon_execution(&t, id, end, err, now)?;
        t.commit()?;
        Ok(true)
    }
    pub fn execution(&self, id: i64) -> Result<ExecutionView> {
        let(s,st,o,e,detail):(i64,String,Option<String>,Option<String>,Option<String>)=self.read_conn()?.query_row("SELECT snapshot_id,state,final_output,error_code,error_message FROM executions WHERE execution_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        Ok(ExecutionView {
            execution_id: id,
            snapshot_id: s,
            state: ExecutionState::parse(&st)?,
            final_output: o,
            error_code: e,
            error_message: detail,
        })
    }
    pub fn has_active_execution(&self, workspace_id: &WorkspaceId) -> Result<bool> {
        Ok(self.read_conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions WHERE workspace_id=?1 AND state IN('queued','assigning','running','recovering'))",
            [workspace_id.as_str()],
            |row| row.get(0),
        )?)
    }
    pub fn active_execution_ids_for_agent(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<Vec<i64>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare(
            "SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2 AND state IN('queued','assigning','running','recovering') ORDER BY execution_id",
        )?;
        Ok(statement
            .query_map(params![workspace_id.as_str(), agent_id.as_str()], |row| {
                row.get(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn recoverable_executions(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<RecoverableExecutionView>> {
        let c = self.read_conn()?;
        let mut sql="SELECT e.execution_id,e.snapshot_id,e.state,e.final_output,e.error_code,d.delivery_id,d.state,e.error_message FROM executions e JOIN execution_deliveries d USING(execution_id) WHERE e.state IN('queued','assigning','running','recovering')".to_owned();
        if workspace_id.is_some() {
            sql.push_str(" AND e.workspace_id=?1");
        }
        sql.push_str(" ORDER BY e.execution_id");
        let mut statement = c.prepare(&sql)?;
        let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RecoverableExecutionRow> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        };
        let rows = if let Some(workspace_id) = workspace_id {
            statement
                .query_map([workspace_id.as_str()], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            statement
                .query_map([], map_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut views = Vec::new();
        for (
            execution_id,
            snapshot_id,
            state_name,
            final_output,
            error_code,
            delivery_id,
            delivery_state,
            error_message,
        ) in rows
        {
            views.push(RecoverableExecutionView {
                execution: ExecutionView {
                    execution_id,
                    snapshot_id,
                    state: ExecutionState::parse(&state_name)?,
                    final_output,
                    error_code,
                    error_message,
                },
                snapshot: load_snapshot(&c, snapshot_id)?,
                delivery_id,
                delivery_state,
            });
        }
        Ok(views)
    }
}

/// What Runtime needs to decide whether the next run resumes the native session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionActivityView {
    /// When the Agent's latest finished execution ended.
    pub last_completed_at: i64,
    /// Whole prompt size of the latest reported model call, cache included.
    pub last_context_tokens: Option<u64>,
}

impl Storage {
    pub fn session_activity(
        &self,
        workspace: &WorkspaceId,
        agent: &AgentId,
    ) -> Result<Option<SessionActivityView>> {
        let connection = self.read_conn()?;
        let row: Option<(i64, Option<i64>)> = connection
            .query_row(
                "SELECT max(e.completed_at),
                   (SELECT json_extract(CAST(ev.payload AS TEXT),'$.input_tokens') FROM execution_events ev
                      JOIN executions x ON x.execution_id=ev.execution_id
                      WHERE x.workspace_id=?1 AND x.agent_id=?2 AND ev.kind='usage'
                      ORDER BY ev.occurred_at DESC,ev.execution_id DESC,ev.sequence DESC LIMIT 1)
                 FROM executions e WHERE e.workspace_id=?1 AND e.agent_id=?2 AND e.completed_at IS NOT NULL",
                params![workspace.as_str(), agent.as_str()],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
            .and_then(|(completed, tokens)| completed.map(|completed| (completed, tokens)));
        Ok(row.map(|(last_completed_at, tokens)| SessionActivityView {
            last_completed_at,
            last_context_tokens: tokens.and_then(|tokens| u64::try_from(tokens).ok()),
        }))
    }
}

// The message a trigger points at: the reacted-to message, the message itself, or for a
// timer or a move the latest message in its conversation.
fn trigger_message_id(
    t: &rusqlite::Transaction<'_>,
    chat_id: i64,
    source_id: i64,
    source_kind: &str,
) -> Result<i64> {
    Ok(match source_kind {
        "reaction" => t.query_row(
            "SELECT target_message_id FROM reactions WHERE reaction_id=?1",
            [-source_id],
            |r| r.get(0),
        )?,
        "timer" | "move" => t.query_row(
            "SELECT last_message_id FROM chats WHERE chat_id=?1",
            [chat_id],
            |r| r.get(0),
        )?,
        _ => source_id,
    })
}
