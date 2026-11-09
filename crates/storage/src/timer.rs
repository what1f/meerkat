use crate::agent::aid;
use crate::message::cid;
use crate::workspace::wid;
use crate::{DispatchReason, Result, Storage, StorageError};
use meerkat_core::{AgentId, ChannelId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// How a timer repeats after it fires.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimerRepeatRecord {
    Once,
    Every { minutes: u32 },
    Cron { expression: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerRecord {
    pub timer_id: i64,
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub chat_name: String,
    pub prompt: String,
    pub repeat: TimerRepeatRecord,
    pub timezone: String,
    pub next_fire_at: i64,
    pub last_fired_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewTimer {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub prompt: String,
    pub repeat: TimerRepeatRecord,
    pub timezone: String,
    pub next_fire_at: i64,
}

/// Active timers one Agent may hold at once.
pub const TIMER_LIMIT: i64 = 30;

const SELECT: &str = "SELECT t.timer_id,t.workspace_id,t.agent_id,c.channel_id,c.provider_chat_id,coalesce(nullif(c.name,''),(SELECT p.name FROM people p WHERE p.workspace_id=c.workspace_id AND p.person_id=c.recipient_person_id),''),t.prompt,t.repeat_json,t.timezone,t.next_fire_at,t.last_fired_at,t.created_at FROM agent_timers t JOIN chats c USING(chat_id)";

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<(TimerRecord, String)> {
    let repeat: String = row.get(7)?;
    Ok((
        TimerRecord {
            timer_id: row.get(0)?,
            workspace_id: wid(row.get(1)?)?,
            agent_id: aid(row.get(2)?)?,
            channel_id: cid(row.get(3)?)?,
            provider_chat_id: row.get(4)?,
            chat_name: row.get(5)?,
            prompt: row.get(6)?,
            repeat: TimerRepeatRecord::Once,
            timezone: row.get(8)?,
            next_fire_at: row.get(9)?,
            last_fired_at: row.get(10)?,
            created_at: row.get(11)?,
        },
        repeat,
    ))
}

fn query(c: &Connection, filter: &str, values: impl rusqlite::Params) -> Result<Vec<TimerRecord>> {
    let mut statement = c.prepare(&format!("{SELECT} {filter}"))?;
    let rows = statement
        .query_map(values, read)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(mut timer, repeat)| {
            timer.repeat = serde_json::from_str(&repeat)?;
            Ok(timer)
        })
        .collect()
}

fn one(c: &Connection, timer_id: i64) -> Result<TimerRecord> {
    query(c, "WHERE t.timer_id=?1", [timer_id])?
        .pop()
        .ok_or(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

impl Storage {
    /// Bind a timer to a conversation the Agent can read; it fires into that conversation.
    pub fn create_timer(&self, timer: &NewTimer, now: i64) -> Result<TimerRecord> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let chat: Option<(i64, Option<i64>)> = t
            .query_row(
                "SELECT c.chat_id,c.last_message_id FROM chats c WHERE c.workspace_id=?1 AND c.channel_id=?3 AND c.provider_chat_id=?4 AND (c.receiving_agent_id=?2 OR EXISTS(SELECT 1 FROM chat_receivers r WHERE r.chat_id=c.chat_id AND r.workspace_id=?1 AND r.agent_id=?2)) ORDER BY c.receiving_agent_id DESC LIMIT 1",
                params![timer.workspace_id.as_str(), timer.agent_id.as_str(), timer.channel_id.as_str(), timer.provider_chat_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((chat_id, Some(_))) = chat else {
            return Err(StorageError::Invalid("timer_chat_unavailable".into()));
        };
        let active: i64 = t.query_row(
            "SELECT count(*) FROM agent_timers WHERE workspace_id=?1 AND agent_id=?2 AND state='active'",
            params![timer.workspace_id.as_str(), timer.agent_id.as_str()],
            |r| r.get(0),
        )?;
        if active >= TIMER_LIMIT {
            return Err(StorageError::Conflict("timer_limit_reached".into()));
        }
        t.execute(
            "INSERT INTO agent_timers(workspace_id,agent_id,chat_id,prompt,repeat_json,timezone,next_fire_at,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8,?8)",
            params![timer.workspace_id.as_str(), timer.agent_id.as_str(), chat_id, timer.prompt, serde_json::to_string(&timer.repeat)?, timer.timezone, timer.next_fire_at, now],
        )?;
        let record = one(&t, t.last_insert_rowid())?;
        t.commit()?;
        Ok(record)
    }

    /// Active timers in a Workspace, or only one Agent's, soonest first.
    pub fn list_timers(
        &self,
        workspace: &WorkspaceId,
        agent: Option<&AgentId>,
    ) -> Result<Vec<TimerRecord>> {
        query(
            &*self.read_conn()?,
            "WHERE t.workspace_id=?1 AND (?2 IS NULL OR t.agent_id=?2) AND t.state='active' ORDER BY t.next_fire_at,t.timer_id",
            params![workspace.as_str(), agent.map(AgentId::as_str)],
        )
    }

    /// Stop an active timer. `agent` limits it to that Agent's own timers.
    pub fn cancel_timer(
        &self,
        workspace: &WorkspaceId,
        agent: Option<&AgentId>,
        timer_id: i64,
        now: i64,
    ) -> Result<Option<TimerRecord>> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = t.execute(
            "UPDATE agent_timers SET state='cancelled',updated_at=?4 WHERE timer_id=?1 AND workspace_id=?2 AND (?3 IS NULL OR agent_id=?3) AND state='active'",
            params![timer_id, workspace.as_str(), agent.map(AgentId::as_str), now],
        )?;
        let record = if changed == 1 {
            Some(one(&t, timer_id)?)
        } else {
            None
        };
        t.commit()?;
        Ok(record)
    }

    /// A cancelled timer, as it was before cancelling.
    pub fn cancelled_timer(
        &self,
        workspace: &WorkspaceId,
        timer_id: i64,
    ) -> Result<Option<TimerRecord>> {
        Ok(query(
            &*self.read_conn()?,
            "WHERE t.timer_id=?1 AND t.workspace_id=?2 AND t.state='cancelled'",
            params![timer_id, workspace.as_str()],
        )?
        .pop())
    }

    /// Put a cancelled timer back, firing next at `next_fire_at`.
    pub fn restore_timer(
        &self,
        workspace: &WorkspaceId,
        timer_id: i64,
        next_fire_at: i64,
        now: i64,
    ) -> Result<Option<TimerRecord>> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = t.execute(
            "UPDATE agent_timers SET state='active',next_fire_at=?3,updated_at=?4 WHERE timer_id=?1 AND workspace_id=?2 AND state='cancelled'",
            params![timer_id, workspace.as_str(), next_fire_at, now],
        )?;
        let record = if changed == 1 {
            Some(one(&t, timer_id)?)
        } else {
            None
        };
        t.commit()?;
        Ok(record)
    }

    pub fn due_timers(&self, workspace: &WorkspaceId, now: i64) -> Result<Vec<TimerRecord>> {
        query(
            &*self.read_conn()?,
            "WHERE t.workspace_id=?1 AND t.state='active' AND t.next_fire_at<=?2 ORDER BY t.next_fire_at,t.timer_id",
            params![workspace.as_str(), now],
        )
    }

    pub fn next_timer_at(&self, workspace: &WorkspaceId) -> Result<Option<i64>> {
        Ok(self.read_conn()?.query_row(
            "SELECT min(next_fire_at) FROM agent_timers WHERE workspace_id=?1 AND state='active'",
            [workspace.as_str()],
            |r| r.get(0),
        )?)
    }

    /// Wake the Agent in the timer's conversation and move the timer on to `next`, or finish
    /// it. Nothing happens unless the timer is still active and due at `expected`.
    pub fn fire_timer(
        &self,
        timer_id: i64,
        expected: i64,
        next: Option<i64>,
        now: i64,
    ) -> Result<bool> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = t.execute(
            "UPDATE agent_timers SET next_fire_at=coalesce(?3,next_fire_at),last_fired_at=?4,state=CASE WHEN ?3 IS NULL THEN 'done' ELSE 'active' END,updated_at=?4 WHERE timer_id=?1 AND next_fire_at=?2 AND state='active'",
            params![timer_id, expected, next, now],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        t.execute(
            "INSERT INTO agent_timer_fires(timer_id,fired_at) VALUES(?1,?2)",
            params![timer_id, now],
        )?;
        let fire_id = t.last_insert_rowid();
        let prompt: String = t.query_row(
            "SELECT prompt FROM agent_timers WHERE timer_id=?1",
            [timer_id],
            |r| r.get(0),
        )?;
        let reason = serde_json::to_string(&DispatchReason::Timer { timer_id, prompt })?;
        t.execute(
            "INSERT INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at) SELECT workspace_id,agent_id,chat_id,?2,'timer',?3,?4,'pending',?4,?4 FROM agent_timers WHERE timer_id=?1",
            params![timer_id, fire_id, reason, now],
        )?;
        t.commit()?;
        Ok(true)
    }
}
