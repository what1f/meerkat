use crate::{Result, Storage};
use meerkat_core::{AgentId, WorkspaceId};
use rusqlite::params;

impl Storage {
    /// Earliest business work for this Agent. Offline Machines keep their inbox
    /// queued, but failure policy and notices do not require a Machine link.
    pub fn next_agent_work_at(
        &self,
        workspace: &WorkspaceId,
        agent: &AgentId,
        dispatchable: bool,
    ) -> Result<Option<i64>> {
        Ok(self.read_conn()?.query_row(
            "SELECT min(due) FROM (
                SELECT 0 AS due FROM executions e
                WHERE workspace_id=?1 AND agent_id=?2 AND state IN('failed','cancelled')
                  AND retry_policy_applied=0 AND (state='cancelled' OR EXISTS(
                    SELECT 1 FROM execution_attempts a WHERE a.execution_id=e.execution_id AND a.failure_json IS NOT NULL))
                UNION ALL SELECT 0 FROM failure_notices
                WHERE workspace_id=?1 AND agent_id=?2 AND sent_at IS NULL
                UNION ALL SELECT min(available_at) FROM agent_inbox_items i JOIN agents p USING(workspace_id,agent_id)
                WHERE i.workspace_id=?1 AND i.agent_id=?2 AND i.state='pending'
                  AND ?3 AND p.pause_reason IS NULL
                  AND NOT EXISTS(SELECT 1 FROM agent_relocations r WHERE r.workspace_id=?1 AND r.agent_id=?2 AND r.state!='failed')
                  AND coalesce(i.retry_batch_id,0)=coalesce((SELECT retry_batch_id FROM agent_inbox_items
                    WHERE workspace_id=?1 AND agent_id=?2 AND state='pending' ORDER BY inbox_item_id LIMIT 1),0)
                  AND NOT EXISTS(SELECT 1 FROM executions e WHERE e.workspace_id=?1 AND e.agent_id=?2
                    AND e.state IN('queued','assigning','running','recovering'))
            )",
            params![workspace.as_str(), agent.as_str(), dispatchable],
            |row| row.get(0),
        )?)
    }

    pub fn next_attempt_lease_at(&self) -> Result<Option<i64>> {
        Ok(self.read_conn()?.query_row(
            "SELECT min(CASE WHEN p.machine_id!=e.placement_machine_id
                OR p.placement_generation!=e.placement_generation OR c.revoked_at IS NOT NULL
                THEN 0 ELSE a.lease_expires_at END)
             FROM execution_attempts a JOIN executions e USING(execution_id)
             JOIN agents p ON p.workspace_id=e.workspace_id AND p.agent_id=e.agent_id
             JOIN machines c ON c.machine_id=a.machine_id
             WHERE e.attempt=a.attempt AND e.state IN('assigning','running','recovering')
               AND a.state IN('offered','accepted','running')",
            [],
            |row| row.get(0),
        )?)
    }
}
