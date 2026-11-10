//! Moving an Agent's home to another Machine.
use crate::agent::aid;
use crate::workspace::wid;
use crate::{AgentRecord, Result, Storage, StorageError};
use meerkat_core::{AgentId, WorkspaceId};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelocationState {
    /// Waiting for the current run to end, then for the source to hand over files.
    Waiting,
    /// The archive is held by the Runtime and waits for the target.
    Staged,
    /// The move stopped before the Agent was placed; it keeps working where it is.
    Failed,
    /// Placed on the target and woken there; done once its first native session starts.
    Arriving,
    /// Placed on the target, but its first run there failed or never started.
    Stalled,
}

impl RelocationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Staged => "staged",
            Self::Failed => "failed",
            Self::Arriving => "arriving",
            Self::Stalled => "stalled",
        }
    }
    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "waiting" => Ok(Self::Waiting),
            "staged" => Ok(Self::Staged),
            "failed" => Ok(Self::Failed),
            "arriving" => Ok(Self::Arriving),
            "stalled" => Ok(Self::Stalled),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelocationRecord {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub source_machine_id: String,
    pub target_machine_id: String,
    pub state: RelocationState,
    pub without_files: bool,
    pub archive_size: Option<u64>,
    pub archive_sha256: Option<String>,
    pub error_code: Option<String>,
    /// AI settings picked for the target; applied when the Agent is placed there.
    pub brain: Option<RelocationBrain>,
    /// When the Agent was placed on the target and woken.
    pub arriving_at: Option<i64>,
}

/// The AI an Agent runs with on its new Machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelocationBrain {
    pub harness_id: String,
    pub model: String,
    pub thinking: String,
}

const COLUMNS: &str = "workspace_id,agent_id,source_machine_id,target_machine_id,state,without_files,archive_size,archive_sha256,error_code,harness_id,model,thinking,arriving_at";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RelocationRecord> {
    Ok(RelocationRecord {
        workspace_id: wid(row.get(0)?)?,
        agent_id: aid(row.get(1)?)?,
        source_machine_id: row.get(2)?,
        target_machine_id: row.get(3)?,
        state: RelocationState::parse(&row.get::<_, String>(4)?)?,
        without_files: row.get(5)?,
        archive_size: row.get::<_, Option<i64>>(6)?.map(|size| size as u64),
        archive_sha256: row.get(7)?,
        error_code: row.get(8)?,
        brain: match (row.get(9)?, row.get(10)?, row.get(11)?) {
            (Some(harness_id), Some(model), Some(thinking)) => Some(RelocationBrain {
                harness_id,
                model,
                thinking,
            }),
            _ => None,
        },
        arriving_at: row.get(12)?,
    })
}

impl Storage {
    pub fn agent_relocation(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
    ) -> Result<Option<RelocationRecord>> {
        Ok(self
            .read_conn()?
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2"
                ),
                params![w.as_str(), a.as_str()],
                row,
            )
            .optional()?)
    }

    pub fn agent_relocations(&self) -> Result<Vec<RelocationRecord>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM agent_relocations ORDER BY created_at"
        ))?;
        Ok(statement
            .query_map([], row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Starts, retargets or restarts a move. A staged archive stays valid for
    /// any target, so retargeting keeps it.
    pub fn request_agent_relocation(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        target_machine_id: &str,
        without_files: bool,
        brain: Option<&RelocationBrain>,
        now: i64,
    ) -> Result<RelocationRecord> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let source: String = transaction.query_row(
            "SELECT machine_id FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
            |row| row.get(0),
        )?;
        if source == target_machine_id {
            return Err(StorageError::Invalid(
                "relocation target is the source".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO agent_relocations(workspace_id,agent_id,source_machine_id,target_machine_id,state,without_files,harness_id,model,thinking,created_at,updated_at)
             VALUES(?1,?2,?3,?4,'waiting',?5,?6,?7,?8,?9,?9)
             ON CONFLICT(workspace_id,agent_id) DO UPDATE SET
               target_machine_id=excluded.target_machine_id,
               without_files=excluded.without_files,
               harness_id=coalesce(excluded.harness_id,harness_id),
               model=coalesce(excluded.model,model),
               thinking=coalesce(excluded.thinking,thinking),
               state=CASE WHEN state='staged' AND NOT excluded.without_files THEN 'staged' ELSE 'waiting' END,
               error_code=NULL,
               updated_at=excluded.updated_at",
            params![
                w.as_str(),
                a.as_str(),
                source,
                target_machine_id,
                without_files,
                brain.map(|b| &b.harness_id),
                brain.map(|b| &b.model),
                brain.map(|b| &b.thinking),
                now
            ],
        )?;
        let record = transaction.query_row(
            &format!(
                "SELECT {COLUMNS} FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2"
            ),
            params![w.as_str(), a.as_str()],
            row,
        )?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn cancel_agent_relocation(&self, w: &WorkspaceId, a: &AgentId) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
        )? == 1)
    }

    /// Records the archive the Runtime now holds. False when the move was
    /// cancelled or changed meanwhile.
    pub fn stage_agent_relocation(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        size: u64,
        sha256: &str,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "UPDATE agent_relocations SET state='staged',archive_size=?3,archive_sha256=?4,updated_at=?5
             WHERE workspace_id=?1 AND agent_id=?2 AND state='waiting' AND NOT without_files",
            params![w.as_str(), a.as_str(), size as i64, sha256, now],
        )? == 1)
    }

    pub fn fail_agent_relocation(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        error_code: &str,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "UPDATE agent_relocations SET state='failed',error_code=?3,updated_at=?4
             WHERE workspace_id=?1 AND agent_id=?2 AND state IN('waiting','staged')",
            params![w.as_str(), a.as_str(), error_code, now],
        )? == 1)
    }

    /// Places the Agent on its target with the AI picked for it and wakes it
    /// there. The Agent's previous placement is kept so the move can be
    /// undone, and the source keeps its files until the Agent has started on
    /// the target. Any cleanup still pending on the target is dropped so it
    /// cannot remove the files that just arrived. Returns the placed Agent and
    /// whether it is still arriving; an Agent with no conversation to wake in
    /// has arrived at once.
    pub fn commit_agent_relocation(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        now: i64,
    ) -> Result<Option<(AgentRecord, bool)>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let Some(record) = relocation_in(&transaction, w, a)? else {
            return Ok(None);
        };
        if !matches!(
            record.state,
            RelocationState::Waiting | RelocationState::Staged
        ) {
            return Ok(None);
        }
        transaction.execute(
            "UPDATE agent_relocations SET previous_json=(SELECT json_object('machine_id',machine_id,'harness_id',harness_id,'model',model,'thinking',thinking,'cli_path',cli_path,'session_config',json(session_config_json)) FROM agents WHERE workspace_id=?1 AND agent_id=?2),
               state='arriving',arriving_at=?3,archive_size=NULL,archive_sha256=NULL,error_code=NULL,updated_at=?3
             WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str(), now],
        )?;
        let brain = record.brain.as_ref();
        transaction.execute(
            "UPDATE agents SET machine_id=?3,cli_path=NULL,updated_at=?4,
               harness_id=coalesce(?5,harness_id),model=coalesce(?6,model),thinking=coalesce(?7,thinking),
               session_config_json=json_remove(session_config_json,'$.native_session_id','$.native_session_harness','$.native_session_fingerprint','$.native_session_machine')
             WHERE workspace_id=?1 AND agent_id=?2",
            params![
                w.as_str(),
                a.as_str(),
                record.target_machine_id,
                now,
                brain.map(|b| &b.harness_id),
                brain.map(|b| &b.model),
                brain.map(|b| &b.thinking)
            ],
        )?;
        transaction.execute(
            "DELETE FROM agent_file_deletions WHERE workspace_id=?1 AND agent_id=?2 AND machine_id=?3",
            params![w.as_str(), a.as_str(), record.target_machine_id],
        )?;
        let arriving = wake_moved(&transaction, &record, now)?;
        if !arriving {
            arrive(&transaction, &record, now)?;
        }
        transaction.commit()?;
        drop(connection);
        Ok(Some((self.agent_record(w, a)?, arriving)))
    }

    /// The Agent started on its target: the move is over and the source's
    /// copy of its home is queued for cleanup.
    pub fn finish_agent_arrival(&self, w: &WorkspaceId, a: &AgentId, now: i64) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let Some(record) = relocation_in(&transaction, w, a)? else {
            return Ok(false);
        };
        if !matches!(
            record.state,
            RelocationState::Arriving | RelocationState::Stalled
        ) {
            return Ok(false);
        }
        arrive(&transaction, &record, now)?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn stall_agent_arrival(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        error_code: &str,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "UPDATE agent_relocations SET state='stalled',error_code=?3,updated_at=?4
             WHERE workspace_id=?1 AND agent_id=?2 AND state='arriving'",
            params![w.as_str(), a.as_str(), error_code, now],
        )? == 1)
    }

    /// Wakes a stalled Agent on its target once more.
    pub fn retry_agent_arrival(&self, w: &WorkspaceId, a: &AgentId, now: i64) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let Some(record) = relocation_in(&transaction, w, a)? else {
            return Ok(false);
        };
        if record.state != RelocationState::Stalled {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE agent_relocations SET state='arriving',arriving_at=?3,error_code=NULL,updated_at=?3
             WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str(), now],
        )?;
        if !wake_moved(&transaction, &record, now)? {
            arrive(&transaction, &record, now)?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Puts an Agent that has not started on its target back where it was,
    /// with the AI and native session it had there. The source still has its
    /// files, so nothing is carried; the target's copy is queued for cleanup.
    pub fn return_agent_from_arrival(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        now: i64,
    ) -> Result<Option<AgentRecord>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let Some(record) = relocation_in(&transaction, w, a)? else {
            return Ok(None);
        };
        if !matches!(
            record.state,
            RelocationState::Arriving | RelocationState::Stalled
        ) {
            return Ok(None);
        }
        let changed = transaction.execute(
            "UPDATE agents SET machine_id=json_extract(r.previous_json,'$.machine_id'),
               harness_id=json_extract(r.previous_json,'$.harness_id'),
               model=json_extract(r.previous_json,'$.model'),
               thinking=json_extract(r.previous_json,'$.thinking'),
               cli_path=json_extract(r.previous_json,'$.cli_path'),
               session_config_json=json_extract(r.previous_json,'$.session_config'),
               updated_at=?3
             FROM agent_relocations r
             WHERE agents.workspace_id=?1 AND agents.agent_id=?2
               AND r.workspace_id=?1 AND r.agent_id=?2 AND r.previous_json IS NOT NULL",
            params![w.as_str(), a.as_str(), now],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid(
                "relocation has no previous placement".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 AND source_kind='move' AND state='pending'",
            params![w.as_str(), a.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM agent_file_deletions WHERE workspace_id=?1 AND agent_id=?2 AND machine_id=?3",
            params![w.as_str(), a.as_str(), record.source_machine_id],
        )?;
        transaction.execute(
            "INSERT INTO agent_file_deletions(workspace_id,agent_id,machine_id,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT DO UPDATE SET created_at=excluded.created_at",
            params![w.as_str(), a.as_str(), record.target_machine_id, now],
        )?;
        transaction.execute(
            "DELETE FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
        )?;
        transaction.commit()?;
        drop(connection);
        self.agent_record(w, a).map(Some)
    }

    /// The error of the latest run on the target since the Agent was woken
    /// there, when that run failed.
    pub fn arrival_failure(&self, w: &WorkspaceId, a: &AgentId) -> Result<Option<String>> {
        Ok(self
            .read_conn()?
            .query_row(
                "SELECT e.state,coalesce(e.error_code,'') FROM executions e JOIN agent_relocations r
                   ON r.workspace_id=e.workspace_id AND r.agent_id=e.agent_id
                 WHERE e.workspace_id=?1 AND e.agent_id=?2 AND e.placement_machine_id=r.target_machine_id
                   AND e.created_at>=r.arriving_at
                 ORDER BY e.execution_id DESC LIMIT 1",
                params![w.as_str(), a.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .filter(|(state, _)| state == "failed")
            .map(|(_, code)| code))
    }
}

fn relocation_in(
    transaction: &rusqlite::Transaction<'_>,
    w: &WorkspaceId,
    a: &AgentId,
) -> Result<Option<RelocationRecord>> {
    Ok(transaction
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2"
            ),
            params![w.as_str(), a.as_str()],
            row,
        )
        .optional()?)
}

/// Wakes the Agent in the conversation it handled last, telling it which 工区
/// it came from and which it is in now. False when it has no conversation yet.
fn wake_moved(
    transaction: &rusqlite::Transaction<'_>,
    record: &RelocationRecord,
    now: i64,
) -> Result<bool> {
    let name = |machine: &str| -> Result<String> {
        Ok(transaction
            .query_row(
                "SELECT display_name FROM machines WHERE machine_id=?1",
                [machine],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default())
    };
    let reason = serde_json::to_string(&crate::DispatchReason::Moved {
        from: name(&record.source_machine_id)?,
        to: name(&record.target_machine_id)?,
    })?;
    Ok(transaction.execute(
        "INSERT INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at)
         SELECT workspace_id,agent_id,chat_id,?3,'move',?4,?3,'pending',?3,?3 FROM agent_chat_cursors
         WHERE workspace_id=?1 AND agent_id=?2 ORDER BY updated_at DESC LIMIT 1",
        params![
            record.workspace_id.as_str(),
            record.agent_id.as_str(),
            now,
            reason
        ],
    )? == 1)
}

fn arrive(
    transaction: &rusqlite::Transaction<'_>,
    record: &RelocationRecord,
    now: i64,
) -> Result<()> {
    let (w, a) = (record.workspace_id.as_str(), record.agent_id.as_str());
    transaction.execute(
        "INSERT INTO agent_file_deletions(workspace_id,agent_id,machine_id,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT DO UPDATE SET created_at=excluded.created_at",
        params![w, a, record.source_machine_id, now],
    )?;
    transaction.execute(
        "DELETE FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2",
        params![w, a],
    )?;
    Ok(())
}
