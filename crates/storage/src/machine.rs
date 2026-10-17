use crate::{Result, Storage, StorageError};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineRecord {
    pub machine_id: String,
    pub is_local: bool,
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

    /// Consumes a code and registers its device atomically.
    /// Local re-pairing rotates credentials while preserving the sole local identity.
    pub fn pair_machine(
        &self,
        machine: &MachineRecord,
        code_hash: Option<&str>,
        validity_ms: i64,
    ) -> Result<String> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        if machine.is_local {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT machine_id FROM machines WHERE is_local=1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(machine_id) = existing {
                transaction.execute(
                    "UPDATE machines SET credential_hash=?2 WHERE machine_id=?1",
                    params![machine_id, machine.credential_hash],
                )?;
                transaction.commit()?;
                return Ok(machine_id);
            }
        }
        if let Some(hash) = code_hash {
            let changed = transaction.execute("UPDATE pairing_codes SET used_at=?2 WHERE code_hash=?1 AND used_at IS NULL AND issued_at<=?2 AND issued_at>?2-?3", params![hash, machine.paired_at, validity_ms])?;
            if changed != 1 {
                return Err(StorageError::Invalid("pairing_code_invalid".into()));
            }
        }
        transaction.execute("INSERT INTO machines(machine_id,display_name,credential_hash,paired_at,os,version,inventory_json,is_local) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![machine.machine_id,machine.display_name,machine.credential_hash,machine.paired_at,machine.os,machine.version,machine.inventory_json,machine.is_local])?;
        transaction.commit()?;
        Ok(machine.machine_id.clone())
    }

    pub fn registered_machines(&self) -> Result<Vec<MachineRecord>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare("SELECT machine_id,display_name,credential_hash,paired_at,revoked_at,last_seen_at,os,version,inventory_json,is_local FROM machines ORDER BY paired_at,machine_id")?;
        Ok(statement
            .query_map([], |r| {
                Ok(MachineRecord {
                    machine_id: r.get(0)?,
                    is_local: r.get(9)?,
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

    pub fn revoke_machine(&self, machine_id: &str, now: i64) -> Result<()> {
        let connection = self.conn()?;
        let exists = connection
            .query_row(
                "SELECT is_local FROM machines WHERE machine_id=?1",
                [machine_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StorageError::Invalid("machine_unknown".into()));
        }
        if exists == Some(true) {
            return Err(StorageError::Invalid(
                "local_machine_cannot_be_removed".into(),
            ));
        }
        let assigned: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE machine_id=?1)",
            [machine_id],
            |r| r.get(0),
        )?;
        if assigned {
            return Err(StorageError::Invalid("machine_has_assigned_agents".into()));
        }
        connection.execute(
            "UPDATE machines SET revoked_at=COALESCE(revoked_at,?2) WHERE machine_id=?1",
            params![machine_id, now],
        )?;
        Ok(())
    }

    pub fn rename_machine(&self, machine_id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(StorageError::Invalid("machine_name_invalid".into()));
        }
        let changed = self.conn()?.execute(
            "UPDATE machines SET display_name=?2 WHERE machine_id=?1 AND revoked_at IS NULL",
            params![machine_id, name],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid("machine_unknown".into()));
        }
        Ok(())
    }

    pub fn observe_machine(
        &self,
        machine_id: &str,
        now: i64,
        inventory_json: Option<&str>,
        os: Option<&str>,
        version: Option<&str>,
    ) -> Result<()> {
        self.conn()?.execute("UPDATE machines SET last_seen_at=?2,inventory_json=COALESCE(?3,inventory_json),os=COALESCE(?4,os),version=COALESCE(?5,version) WHERE machine_id=?1 AND revoked_at IS NULL", params![machine_id,now,inventory_json,os,version])?;
        Ok(())
    }
}
