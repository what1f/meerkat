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
    /// The move stopped; the Agent keeps working where it is.
    Failed,
}

impl RelocationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Staged => "staged",
            Self::Failed => "failed",
        }
    }
    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "waiting" => Ok(Self::Waiting),
            "staged" => Ok(Self::Staged),
            "failed" => Ok(Self::Failed),
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
}

const COLUMNS: &str = "workspace_id,agent_id,source_machine_id,target_machine_id,state,without_files,archive_size,archive_sha256,error_code";

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
            "INSERT INTO agent_relocations(workspace_id,agent_id,source_machine_id,target_machine_id,state,without_files,created_at,updated_at)
             VALUES(?1,?2,?3,?4,'waiting',?5,?6,?6)
             ON CONFLICT(workspace_id,agent_id) DO UPDATE SET
               target_machine_id=excluded.target_machine_id,
               without_files=excluded.without_files,
               state=CASE WHEN state='staged' AND NOT excluded.without_files THEN 'staged' ELSE 'waiting' END,
               error_code=NULL,
               updated_at=excluded.updated_at",
            params![w.as_str(), a.as_str(), source, target_machine_id, without_files, now],
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
             WHERE workspace_id=?1 AND agent_id=?2 AND state!='failed'",
            params![w.as_str(), a.as_str(), error_code, now],
        )? == 1)
    }

    /// Places the Agent on its target. The native session is left behind (its
    /// machine stays set to the source until the first session on the target
    /// is saved, which is how the first run learns about the move), the
    /// source is queued for cleanup, and any cleanup still pending on the
    /// target is dropped so it cannot remove the files that just arrived.
    pub fn commit_agent_relocation(
        &self,
        w: &WorkspaceId,
        a: &AgentId,
        now: i64,
    ) -> Result<Option<AgentRecord>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let Some(record) = transaction
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2"
                ),
                params![w.as_str(), a.as_str()],
                row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        if record.state == RelocationState::Failed {
            return Ok(None);
        }
        transaction.execute(
            "UPDATE agents SET machine_id=?3,cli_path=NULL,updated_at=?4,
               session_config_json=json_set(json_remove(session_config_json,'$.native_session_id','$.native_session_harness','$.native_session_fingerprint'),'$.native_session_machine',?5)
             WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str(), record.target_machine_id, now, record.source_machine_id],
        )?;
        transaction.execute(
            "DELETE FROM agent_file_deletions WHERE workspace_id=?1 AND agent_id=?2 AND machine_id=?3",
            params![w.as_str(), a.as_str(), record.target_machine_id],
        )?;
        transaction.execute(
            "INSERT INTO agent_file_deletions(workspace_id,agent_id,machine_id,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT DO UPDATE SET created_at=excluded.created_at",
            params![w.as_str(), a.as_str(), record.source_machine_id, now],
        )?;
        transaction.execute(
            "DELETE FROM agent_relocations WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
        )?;
        transaction.commit()?;
        drop(connection);
        self.agent_record(w, a).map(Some)
    }
}
