use crate::{Result, Storage, StorageError};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputerRecord {
    pub computer_id: String,
    pub display_name: String,
    pub credential_hash: String,
    pub paired_at: i64,
    pub revoked_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub os: String,
    pub version: String,
    pub inventory_json: String,
}

impl Storage {
    pub fn issue_pairing_code(&self, code_hash: &str, issued_at: i64) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO pairing_codes(code_hash,issued_at) VALUES(?1,?2)",
            params![code_hash, issued_at],
        )?;
        Ok(())
    }

    /// Consumes a code and registers its device atomically. The first local
    /// pairing also moves legacy Agent placement and deferred file cleanup.
    pub fn pair_computer(
        &self,
        computer: &ComputerRecord,
        code_hash: Option<&str>,
        validity_ms: i64,
    ) -> Result<()> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        if let Some(hash) = code_hash {
            let changed = transaction.execute("UPDATE pairing_codes SET used_at=?2 WHERE code_hash=?1 AND used_at IS NULL AND issued_at<=?2 AND issued_at>?2-?3", params![hash, computer.paired_at, validity_ms])?;
            if changed != 1 {
                return Err(StorageError::Invalid("pairing_code_invalid".into()));
            }
        }
        let first_local = code_hash.is_none()
            && !transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM computers WHERE local_pair=1)",
                [],
                |r| r.get::<_, bool>(0),
            )?;
        transaction.execute("INSERT INTO computers(computer_id,display_name,credential_hash,paired_at,os,version,inventory_json,local_pair) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![computer.computer_id,computer.display_name,computer.credential_hash,computer.paired_at,computer.os,computer.version,computer.inventory_json,code_hash.is_none()])?;
        if first_local {
            transaction.execute(
                "UPDATE agents SET computer_id=?1 WHERE computer_id='local'",
                [&computer.computer_id],
            )?;
            transaction.execute(
                "UPDATE agent_file_deletions SET computer_id=?1 WHERE computer_id='local'",
                [&computer.computer_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn registered_computers(&self) -> Result<Vec<ComputerRecord>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare("SELECT computer_id,display_name,credential_hash,paired_at,revoked_at,last_seen_at,os,version,inventory_json FROM computers ORDER BY paired_at,computer_id")?;
        Ok(statement
            .query_map([], |r| {
                Ok(ComputerRecord {
                    computer_id: r.get(0)?,
                    display_name: r.get(1)?,
                    credential_hash: r.get(2)?,
                    paired_at: r.get(3)?,
                    revoked_at: r.get(4)?,
                    last_seen_at: r.get(5)?,
                    os: r.get(6)?,
                    version: r.get(7)?,
                    inventory_json: r.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn revoke_computer(&self, computer_id: &str, now: i64) -> Result<()> {
        let connection = self.conn()?;
        let exists = connection
            .query_row(
                "SELECT computer_id FROM computers WHERE computer_id=?1",
                [computer_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StorageError::Invalid("computer_unknown".into()));
        }
        connection.execute(
            "UPDATE computers SET revoked_at=COALESCE(revoked_at,?2) WHERE computer_id=?1",
            params![computer_id, now],
        )?;
        Ok(())
    }

    pub fn observe_computer(
        &self,
        computer_id: &str,
        now: i64,
        inventory_json: Option<&str>,
        os: Option<&str>,
        version: Option<&str>,
    ) -> Result<()> {
        self.conn()?.execute("UPDATE computers SET last_seen_at=?2,inventory_json=COALESCE(?3,inventory_json),os=COALESCE(?4,os),version=COALESCE(?5,version) WHERE computer_id=?1 AND revoked_at IS NULL", params![computer_id,now,inventory_json,os,version])?;
        Ok(())
    }
}
