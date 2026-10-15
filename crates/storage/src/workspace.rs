use crate::error::conv;
use crate::{Result, Storage, StorageError};
use lankide_core::{AgentId, ProviderId, ResourceId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Debug)]
pub struct WorkspaceSpec {
    pub id: WorkspaceId,
    pub name: String,
    pub provider_id: ProviderId,
    pub provider_config: Value,
    pub provider_secret: Value,
    pub active: bool,
    pub now: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WorkspaceView {
    pub id: WorkspaceId,
    pub name: String,
    pub provider_id: ProviderId,
    pub provider_config: Value,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WorkspaceSecretRef {
    pub workspace_id: WorkspaceId,
    pub revision: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub name: String,
    pub provider_id: ProviderId,
    pub provider_config: Value,
    /// Opaque, Provider-owned durable state. Management updates never replace it.
    pub provider_state: Value,
    pub rules: String,
    pub cover_resource_id: Option<ResourceId>,
    pub setup_status: WorkspaceSetupStatus,
    pub setup_error_code: Option<String>,
    pub secret_ref: Option<WorkspaceSecretRef>,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSetupStatus {
    Pending,
    Ready,
    Error,
}
impl WorkspaceSetupStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "error" => Ok(Self::Error),
            _ => Err(StorageError::Invalid(format!(
                "workspace setup status {value}"
            ))),
        }
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct SecretValue(String);
impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

fn load_workspace_record(c: &Connection, id: &WorkspaceId) -> Result<WorkspaceRecord> {
    let view = c.query_row(
        "SELECT * FROM workspaces WHERE workspace_id=?1",
        [id.as_str()],
        map_workspace,
    )?;
    let named: Option<i64> = c.query_row(
        "SELECT max(revision) FROM workspace_named_secrets WHERE workspace_id=?1",
        [id.as_str()],
        |r| r.get(0),
    )?;
    let revision = named;
    let (rules, cover_resource_id, setup_status, setup_error_code, provider_state_json): (
        String,
        Option<i64>,
        String,
        Option<String>,
        String,
    ) = c.query_row(
        "SELECT rules,cover_resource_id,setup_status,setup_error_code,provider_state_json FROM workspaces WHERE workspace_id=?1",
        [id.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    Ok(WorkspaceRecord {
        id: view.id.clone(),
        name: view.name,
        provider_id: view.provider_id,
        provider_config: view.provider_config,
        provider_state: serde_json::from_str(&provider_state_json)?,
        rules,
        cover_resource_id: cover_resource_id
            .map(ResourceId::new)
            .transpose()
            .map_err(|error| StorageError::Invalid(error.to_string()))?,
        setup_status: WorkspaceSetupStatus::parse(&setup_status)?,
        setup_error_code,
        secret_ref: revision.map(|revision| WorkspaceSecretRef {
            workspace_id: view.id,
            revision,
        }),
        active: view.active,
        created_at: view.created_at,
        updated_at: view.updated_at,
    })
}
pub(super) fn ensure_workspace_provider(
    c: &Connection,
    w: &WorkspaceId,
    p: &ProviderId,
) -> Result<()> {
    let actual: String = c.query_row(
        "SELECT provider_id FROM workspaces WHERE workspace_id=?1",
        [w.as_str()],
        |r| r.get(0),
    )?;
    if actual != p.as_str() {
        return Err(StorageError::Invalid(format!(
            "provider mismatch for workspace {w}"
        )));
    }
    Ok(())
}
fn map_workspace(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceView> {
    let c: String = r.get(3)?;
    Ok(WorkspaceView {
        id: wid(r.get(0)?)?,
        name: r.get(1)?,
        provider_id: pid(r.get(2)?)?,
        provider_config: serde_json::from_str(&c).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?,
        active: r.get::<_, String>(4)? == "running",
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}
pub(super) fn wid(s: String) -> rusqlite::Result<WorkspaceId> {
    WorkspaceId::new(s).map_err(conv)
}
pub(super) fn pid(s: String) -> rusqlite::Result<ProviderId> {
    ProviderId::new(s).map_err(conv)
}

impl Storage {
    pub fn create_workspace(&self, s: WorkspaceSpec) -> Result<()> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        t.execute(
            "INSERT INTO workspaces(workspace_id,name,provider_id,provider_config_json,desired_state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",
            params![
                s.id.as_str(),
                s.name,
                s.provider_id.as_str(),
                serde_json::to_string(&s.provider_config)?,
                if s.active { "running" } else { "paused" },
                s.now
            ],
        )?;
        let secrets = s
            .provider_secret
            .as_object()
            .ok_or_else(|| StorageError::Invalid("workspace secrets must be an object".into()))?;
        for (index, (name, value)) in secrets.iter().enumerate() {
            let value = value
                .as_str()
                .ok_or_else(|| StorageError::Invalid("workspace secret must be text".into()))?;
            t.execute("INSERT INTO workspace_named_secrets(workspace_id,name,value,revision,updated_at) VALUES(?1,?2,?3,?4,?5)", params![s.id.as_str(),name,value,index as i64+1,s.now])?;
        }
        t.commit()?;
        Ok(())
    }
    pub fn workspace(&self, id: &WorkspaceId) -> Result<WorkspaceView> {
        Ok(self.read_conn()?.query_row(
            "SELECT * FROM workspaces WHERE workspace_id=?1",
            [id.as_str()],
            map_workspace,
        )?)
    }
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceView>> {
        let c = self.read_conn()?;
        let mut s = c.prepare("SELECT * FROM workspaces ORDER BY workspace_id")?;
        Ok(s.query_map([], map_workspace)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Resolve one stable Provider identity inside a Workspace. New people are
    /// assigned a compact UUID-derived ID which is also checked against Agent
    /// IDs so the value remains unambiguous in Agent-facing text.
    pub fn workspace_record(&self, id: &WorkspaceId) -> Result<WorkspaceRecord> {
        let c = self.read_conn()?;
        load_workspace_record(&c, id)
    }
    pub fn list_workspace_records(&self) -> Result<Vec<WorkspaceRecord>> {
        let c = self.read_conn()?;
        let mut statement =
            c.prepare("SELECT workspace_id FROM workspaces ORDER BY workspace_id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let mut records = Vec::new();
        for id in ids {
            records.push(load_workspace_record(&c, &wid(id)?)?);
        }
        Ok(records)
    }
    pub fn upsert_workspace_record(&self, record: WorkspaceRecord) -> Result<WorkspaceRecord> {
        self.commit_workspace_record(record, &Value::Object(Default::default()))
    }
    pub fn insert_new_workspace_record(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> Result<WorkspaceRecord> {
        self.write_workspace_record(record, provider_secrets, true)
    }
    pub fn update_workspace_record_with_secrets(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> Result<WorkspaceRecord> {
        self.write_workspace_record(record, provider_secrets, false)
    }
    /// Atomically commits the public Workspace record and an opaque set of
    /// named Provider secrets. Secret values must be strings and are never
    /// returned from this operation.
    pub fn commit_workspace_record(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
    ) -> Result<WorkspaceRecord> {
        match self.workspace_record(&record.id) {
            Ok(_) => self.update_workspace_record_with_secrets(record, provider_secrets),
            Err(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                self.insert_new_workspace_record(record, provider_secrets)
            }
            Err(error) => Err(error),
        }
    }
    fn write_workspace_record(
        &self,
        record: WorkspaceRecord,
        provider_secrets: &Value,
        insert: bool,
    ) -> Result<WorkspaceRecord> {
        if record.name.trim().is_empty() {
            return Err(StorageError::Invalid("empty workspace name".into()));
        }
        if record.setup_status != WorkspaceSetupStatus::Error && record.setup_error_code.is_some() {
            return Err(StorageError::Invalid(
                "workspace setup error requires error status".into(),
            ));
        }
        if !record.provider_state.is_object() {
            return Err(StorageError::Invalid(
                "workspace provider state must be a JSON object".into(),
            ));
        }
        let secrets = provider_secrets.as_object().ok_or_else(|| {
            StorageError::Invalid("provider secrets must be a JSON object".into())
        })?;
        for (name, value) in secrets {
            if name.trim().is_empty() || !value.is_string() {
                return Err(StorageError::Invalid(
                    "provider secret names and values must be strings".into(),
                ));
            }
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = if insert {
            t.execute("INSERT INTO workspaces(workspace_id,name,provider_id,provider_config_json,desired_state,created_at,updated_at,rules,cover_resource_id,setup_status,setup_error_code,provider_state_json)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![record.id.as_str(),record.name,record.provider_id.as_str(),serde_json::to_string(&record.provider_config)?,if record.active{"running"}else{"paused"},record.created_at,record.updated_at,record.rules,record.cover_resource_id.map(ResourceId::get),record.setup_status.as_str(),record.setup_error_code,serde_json::to_string(&record.provider_state)?])?
        } else {
            t.execute("UPDATE workspaces SET name=?2,provider_id=?3,provider_config_json=?4,desired_state=?5,updated_at=?6,rules=?7,cover_resource_id=?8,setup_status=?9,setup_error_code=?10,provider_state_json=?11 WHERE workspace_id=?1",params![record.id.as_str(),record.name,record.provider_id.as_str(),serde_json::to_string(&record.provider_config)?,if record.active{"running"}else{"paused"},record.updated_at,record.rules,record.cover_resource_id.map(ResourceId::get),record.setup_status.as_str(),record.setup_error_code,serde_json::to_string(&record.provider_state)?])?
        };
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "workspace {} not found",
                record.id
            )));
        }
        let mut revision: i64 = t.query_row(
            "SELECT coalesce(max(revision),0) FROM workspace_named_secrets WHERE workspace_id=?1",
            [record.id.as_str()],
            |row| row.get(0),
        )?;
        for (name, value) in secrets {
            revision += 1;
            t.execute(
                "INSERT INTO workspace_named_secrets(workspace_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",
                params![record.id.as_str(), name, value.as_str().unwrap_or_default(), revision, record.updated_at],
            )?;
        }
        t.commit()?;
        load_workspace_record(&c, &record.id)
    }
    pub fn update_workspace_record(&self, record: WorkspaceRecord) -> Result<WorkspaceRecord> {
        self.update_workspace_record_with_secrets(record, &Value::Object(Default::default()))
    }
    pub fn delete_workspace(&self, id: &WorkspaceId) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM workspaces WHERE workspace_id=?1",
            [id.as_str()],
        )? == 1)
    }
    /// Roll back a Workspace that failed during its initial Provider setup.
    /// This is intentionally not exposed as a product deletion capability.
    pub fn rollback_new_workspace(&self, id: &WorkspaceId) -> Result<()> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let agents: i64 = transaction.query_row(
            "SELECT count(*) FROM agents WHERE workspace_id=?1",
            [id.as_str()],
            |row| row.get(0),
        )?;
        if agents != 0 {
            return Err(StorageError::Conflict(
                "workspace rollback after Agent creation".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM resource_metadata WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspace_resource_counters WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM provider_kv WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspace_named_secrets WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM workspaces WHERE workspace_id=?1",
            [id.as_str()],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn set_workspace_active(
        &self,
        workspace_id: &WorkspaceId,
        active: bool,
        now: i64,
    ) -> Result<bool> {
        let desired = if active { "running" } else { "paused" };
        Ok(self.conn()?.execute(
            "UPDATE workspaces SET desired_state=?2,updated_at=?3 WHERE workspace_id=?1 AND desired_state<>?2",
            params![workspace_id.as_str(), desired, now],
        )? == 1)
    }
    pub fn set_workspace_setup(
        &self,
        workspace_id: &WorkspaceId,
        status: WorkspaceSetupStatus,
        error_code: Option<&str>,
        now: i64,
    ) -> Result<WorkspaceRecord> {
        if status != WorkspaceSetupStatus::Error && error_code.is_some() {
            return Err(StorageError::Invalid(
                "workspace setup error requires error status".into(),
            ));
        }
        let changed = self.conn()?.execute(
            "UPDATE workspaces SET setup_status=?2,setup_error_code=?3,updated_at=?4 WHERE workspace_id=?1 AND (setup_status<>?2 OR setup_error_code IS NOT ?3)",
            params![workspace_id.as_str(), status.as_str(), error_code, now],
        )?;
        if changed > 1 {
            return Err(StorageError::Conflict("workspace setup update".into()));
        }
        self.workspace_record(workspace_id)
    }
    pub fn put_secret(
        &self,
        workspace_id: &WorkspaceId,
        name: &str,
        value: SecretValue,
        now: i64,
    ) -> Result<WorkspaceSecretRef> {
        if name.trim().is_empty() {
            return Err(StorageError::Invalid("empty secret name".into()));
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let next: i64 = t.query_row(
            "SELECT coalesce(max(revision),0)+1 FROM workspace_named_secrets WHERE workspace_id=?1",
            [workspace_id.as_str()],
            |r| r.get(0),
        )?;
        t.execute("INSERT INTO workspace_named_secrets(workspace_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",params![workspace_id.as_str(),name,value.expose_secret(),next,now])?;
        t.commit()?;
        Ok(WorkspaceSecretRef {
            workspace_id: workspace_id.clone(),
            revision: next,
        })
    }
    pub fn put_agent_secret(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        name: &str,
        value: SecretValue,
        now: i64,
    ) -> Result<()> {
        if name.trim().is_empty() || name != name.trim() {
            return Err(StorageError::Invalid("empty secret name".into()));
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let next: i64 = transaction.query_row(
            "SELECT coalesce(max(revision),0)+1 FROM agent_named_secrets WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO agent_named_secrets(workspace_id,agent_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,agent_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",
            params![workspace_id.as_str(), agent_id.as_str(), name, value.expose_secret(), next, now],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn read_secret(
        &self,
        workspace_id: &WorkspaceId,
        name: &str,
    ) -> Result<Option<SecretValue>> {
        if let Some(scoped) = name.strip_prefix("agent:")
            && let Some((agent_id, secret_name)) = scoped.split_once(':')
        {
            return Ok(self
                .read_conn()?
                .query_row(
                    "SELECT value FROM agent_named_secrets WHERE workspace_id=?1 AND agent_id=?2 AND name=?3",
                    params![workspace_id.as_str(), agent_id, secret_name],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(SecretValue::new));
        }
        Ok(self
            .read_conn()?
            .query_row(
                "SELECT value FROM workspace_named_secrets WHERE workspace_id=?1 AND name=?2",
                params![workspace_id.as_str(), name],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(SecretValue::new))
    }
    pub fn delete_secret(&self, workspace_id: &WorkspaceId, name: &str) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM workspace_named_secrets WHERE workspace_id=?1 AND name=?2",
            params![workspace_id.as_str(), name],
        )? == 1)
    }
    pub fn workspace_provider_state(&self, workspace_id: &WorkspaceId) -> Result<Value> {
        let value: String = self.read_conn()?.query_row(
            "SELECT provider_state_json FROM workspaces WHERE workspace_id=?1",
            [workspace_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&value)?)
    }
    pub fn put_workspace_provider_state(
        &self,
        workspace_id: &WorkspaceId,
        state: &Value,
    ) -> Result<()> {
        if !state.is_object() {
            return Err(StorageError::Invalid(
                "workspace provider state must be a JSON object".into(),
            ));
        }
        let changed = self.conn()?.execute(
            "UPDATE workspaces SET provider_state_json=?2 WHERE workspace_id=?1",
            params![workspace_id.as_str(), serde_json::to_string(state)?],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "workspace {workspace_id} not found"
            )));
        }
        Ok(())
    }
    pub fn agent_provider_state(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<Value> {
        let value: String = self.read_conn()?.query_row(
            "SELECT provider_state_json FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&value)?)
    }
    pub fn put_agent_provider_state(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        state: &Value,
    ) -> Result<()> {
        if !state.is_object() {
            return Err(StorageError::Invalid(
                "agent provider state must be a JSON object".into(),
            ));
        }
        let changed = self.conn()?.execute(
            "UPDATE agents SET provider_state_json=?3 WHERE workspace_id=?1 AND agent_id=?2",
            params![
                workspace_id.as_str(),
                agent_id.as_str(),
                serde_json::to_string(state)?
            ],
        )?;
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "agent {workspace_id}/{agent_id} not found"
            )));
        }
        Ok(())
    }
    pub fn provider_kv_get(
        &self,
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .read_conn()?
            .query_row(
                "SELECT value FROM provider_kv WHERE workspace_id=?1 AND namespace=?2 AND key=?3",
                params![workspace_id.as_str(), namespace, key],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn provider_kv_put(
        &self,
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
        value: &[u8],
        now: i64,
    ) -> Result<()> {
        if namespace.trim().is_empty() || key.trim().is_empty() {
            return Err(StorageError::Invalid(
                "empty provider namespace or key".into(),
            ));
        }
        self.conn()?.execute("INSERT INTO provider_kv(workspace_id,namespace,key,value,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,namespace,key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![workspace_id.as_str(),namespace,key,value,now])?;
        Ok(())
    }
    pub fn provider_kv_delete(
        &self,
        workspace_id: &WorkspaceId,
        namespace: &str,
        key: &str,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM provider_kv WHERE workspace_id=?1 AND namespace=?2 AND key=?3",
            params![workspace_id.as_str(), namespace, key],
        )? == 1)
    }
}
