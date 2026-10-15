use crate::{Result, Storage};
use lankide_core::{AgentId, WorkspaceId};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Debug)]
pub struct FailureHistory {
    pub batch_failures: i64,
    pub fingerprint: Option<String>,
    pub streak: i64,
}
#[derive(Clone, Debug)]
pub struct FailureNotice {
    pub id: i64,
    pub channel_id: String,
    pub provider_chat_id: String,
    pub content: String,
}
impl Storage {
    pub fn unapplied_failures(&self, w: &WorkspaceId, a: &AgentId) -> Result<Vec<(i64, i64)>> {
        let c = self.read_conn()?;
        let mut q=c.prepare("SELECT execution_id,coalesce(completed_at,updated_at) FROM executions WHERE workspace_id=?1 AND agent_id=?2 AND state IN ('failed','cancelled') AND retry_policy_applied=0 AND (state='cancelled' OR EXISTS(SELECT 1 FROM execution_attempts a WHERE a.execution_id=executions.execution_id AND a.failure_json IS NOT NULL)) ORDER BY execution_id")?;
        Ok(q.query_map(params![w.as_str(), a.as_str()], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn agent_pause_reason(&self, w: &WorkspaceId, a: &AgentId) -> Result<Option<String>> {
        Ok(self.read_conn()?.query_row(
            "SELECT pause_reason FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
            |r| r.get(0),
        )?)
    }
    pub fn resume_agent(&self, w: &WorkspaceId, a: &AgentId, now: i64) -> Result<()> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if t.execute("UPDATE agents SET pause_reason=NULL,failure_fingerprint=NULL,failure_streak=0 WHERE workspace_id=?1 AND agent_id=?2 AND pause_reason IS NOT NULL", params![w.as_str(),a.as_str()])? == 0 { return Ok(()); }
        t.execute("UPDATE agent_inbox_items SET available_at=?3,failure_count=0,retry_batch_id=NULL WHERE workspace_id=?1 AND agent_id=?2 AND state='pending'", params![w.as_str(),a.as_str(),now])?;
        t.commit()?;
        Ok(())
    }
    pub fn failure_history(&self, id: i64) -> Result<FailureHistory> {
        Ok(self.read_conn()?.query_row("SELECT coalesce((SELECT max(i.failure_count) FROM agent_inbox_items i JOIN execution_delivery_items d USING(inbox_item_id) JOIN execution_deliveries ed USING(delivery_id) WHERE ed.execution_id=e.execution_id),0),a.failure_fingerprint,a.failure_streak FROM executions e JOIN agents a USING(workspace_id,agent_id) WHERE execution_id=?1", [id], |r| Ok(FailureHistory {batch_failures:r.get(0)?,fingerprint:r.get(1)?,streak:r.get(2)?}))?)
    }
    /// Requeue an ownership conflict without changing Harness failure counters.
    pub fn retry_execution_conflict(&self, id: i64, retry_at: i64) -> Result<()> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if t.execute("UPDATE executions SET retry_policy_applied=1 WHERE execution_id=?1 AND state='failed' AND retry_policy_applied=0", [id])? == 0 {
            return Ok(());
        }
        t.execute("UPDATE agent_inbox_items SET retry_batch_id=coalesce(retry_batch_id,?1),available_at=?2,state='pending',claimed_execution_id=NULL WHERE inbox_item_id IN (SELECT inbox_item_id FROM execution_delivery_items JOIN execution_deliveries USING(delivery_id) WHERE execution_id=?1) AND state!='consumed'", params![id,retry_at])?;
        t.commit()?;
        Ok(())
    }
    /// Persists a Runtime policy decision and its notice in one transaction.
    pub fn apply_failure_policy(
        &self,
        id: i64,
        fingerprint: &str,
        pause_reason: Option<&str>,
        retry_at: Option<i64>,
        notice: bool,
        dedupe_ms: i64,
        now: i64,
    ) -> Result<()> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if t.execute("UPDATE executions SET retry_policy_applied=1 WHERE execution_id=?1 AND retry_policy_applied=0", [id])? == 0 { return Ok(()); }
        let (w, a): (String, String) = t.query_row(
            "SELECT workspace_id,agent_id FROM executions WHERE execution_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        t.execute("UPDATE agents SET pause_reason=?3,failure_streak=CASE WHEN failure_fingerprint=?4 THEN failure_streak+1 ELSE 1 END,failure_fingerprint=?4 WHERE workspace_id=?1 AND agent_id=?2",params![w,a,pause_reason,fingerprint])?;
        // Terminal Computer events have already released the claims. Delivery
        // membership identifies exactly the frozen batch, including after restart.
        t.execute("UPDATE agent_inbox_items SET failure_count=failure_count+1,retry_batch_id=coalesce(retry_batch_id,?1),available_at=coalesce(?2,available_at),state=CASE WHEN ?2 IS NULL THEN 'failed' ELSE 'pending' END,claimed_execution_id=NULL WHERE inbox_item_id IN (SELECT inbox_item_id FROM execution_delivery_items JOIN execution_deliveries USING(delivery_id) WHERE execution_id=?1) AND state!='consumed'",params![id,retry_at])?;
        if notice {
            let mut q=t.prepare("SELECT DISTINCT c.chat_id,m.content_markdown FROM unread_snapshot_triggers tr JOIN executions e USING(snapshot_id) JOIN chats c USING(chat_id) JOIN messages m ON m.message_id=tr.message_id WHERE e.execution_id=?1 ORDER BY tr.position")?;
            let chats = q
                .query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(q);
            for (chat, text) in chats {
                let recent: Option<i64> = t.query_row("SELECT notice_id FROM failure_notices WHERE workspace_id=?1 AND agent_id=?2 AND chat_id=?3 AND fingerprint=?4 AND (sent_at IS NULL OR sent_at>?5 OR created_at>?5) LIMIT 1",params![w,a,chat,fingerprint,now.saturating_sub(dedupe_ms)],|r|r.get(0)).optional()?;
                if recent.is_none() {
                    t.execute("INSERT INTO failure_notices(workspace_id,agent_id,chat_id,fingerprint,content,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![w,a,chat,fingerprint,lankide_core::agent_failure_notice(&text),now])?;
                }
            }
        }
        t.commit()?;
        Ok(())
    }
    pub fn pending_failure_notices(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
    ) -> Result<Vec<FailureNotice>> {
        let c = self.read_conn()?;
        let mut q=c.prepare("SELECT n.notice_id,c.channel_id,c.provider_chat_id,n.content FROM failure_notices n JOIN chats c USING(chat_id) WHERE n.workspace_id=?1 AND n.agent_id=?2 AND n.sent_at IS NULL ORDER BY n.notice_id")?;
        Ok(q.query_map(params![w.as_str(), a.as_str()], |r| {
            Ok(FailureNotice {
                id: r.get(0)?,
                channel_id: r.get(1)?,
                provider_chat_id: r.get(2)?,
                content: r.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn finish_failure_notice(&self, id: i64, now: i64) -> Result<()> {
        self.conn()?.execute(
            "UPDATE failure_notices SET sent_at=?2 WHERE notice_id=?1",
            params![id, now],
        )?;
        Ok(())
    }
}
