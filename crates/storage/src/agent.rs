use crate::error::conv;
use crate::workspace::wid;
use crate::{Result, Storage, StorageError};
use lankide_core::{AgentId, ChannelId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

type StoredAgentRow = (
    String,
    String,
    i64,
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    i64,
);
#[derive(Clone, Debug)]
pub struct AgentSpec {
    pub computer_id: String,
    pub workspace_id: WorkspaceId,
    pub id: AgentId,
    pub name: String,
    pub now: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct AgentView {
    pub workspace_id: WorkspaceId,
    pub id: AgentId,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct AgentRecord {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub name: String,
    pub role: String,
    pub bio: String,
    pub avatar: Option<String>,
    pub soul: String,
    pub computer_id: String,
    /// Storage-owned revision of the Computer/Harness placement.
    pub placement_generation: i64,
    pub harness_id: String,
    pub model: String,
    pub thinking: String,
    pub auto_reaction: Option<String>,
    pub cli_path: Option<String>,
    pub env: BTreeMap<String, String>,
    pub provider_agent_config: Value,
    /// Opaque, Provider-owned durable state. Management updates never replace it.
    pub provider_state: Value,
    pub session_config: Value,
    pub attention: BTreeMap<ChannelId, AttentionRecord>,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttentionRecord {
    Mention,
    Always,
    Mute,
    Schedule { interval_minutes: u32 },
}
impl AttentionRecord {
    pub fn validate(&self) -> Result<()> {
        if let Self::Schedule { interval_minutes } = self
            && !matches!(*interval_minutes, 5 | 15 | 30 | 60 | 120 | 240 | 480 | 1440)
        {
            return Err(StorageError::Invalid(format!(
                "unsupported schedule interval {interval_minutes}"
            )));
        }
        Ok(())
    }
}

fn delete_agent_state(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    agent_id: &AgentId,
) -> Result<()> {
    let agent = params![workspace_id.as_str(), agent_id.as_str()];
    transaction.execute(
        "DELETE FROM failure_notices WHERE workspace_id=?1 AND agent_id=?2",
        agent,
    )?;
    transaction.execute(
        "DELETE FROM execution_delivery_items
         WHERE delivery_id IN (
             SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
                 SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
             )
         ) OR inbox_item_id IN (
             SELECT inbox_item_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2
         )",
        agent,
    )?;
    transaction.execute(
        "DELETE FROM execution_delivery_chats WHERE delivery_id IN (
             SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
                 SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
             )
         )",
        params![workspace_id.as_str(), agent_id.as_str()],
    )?;
    transaction.execute(
        "DELETE FROM steer_deliveries
         WHERE execution_id IN (
             SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
         ) OR inbox_item_id IN (
             SELECT inbox_item_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2
         )",
        params![workspace_id.as_str(), agent_id.as_str()],
    )?;
    for table in [
        "execution_attempt_capabilities",
        "execution_attempt_authorizations",
        "execution_events",
        "execution_attempts",
        "message_send_attempts",
        "tool_invocations",
        "execution_deliveries",
    ] {
        transaction.execute(
            &format!(
                "DELETE FROM {table} WHERE execution_id IN (
                    SELECT execution_id FROM executions WHERE workspace_id=?1 AND agent_id=?2
                )"
            ),
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
    }
    transaction.execute(
        "DELETE FROM executions WHERE workspace_id=?1 AND agent_id=?2",
        params![workspace_id.as_str(), agent_id.as_str()],
    )?;
    for table in [
        "unread_snapshot_chats",
        "unread_snapshot_messages",
        "unread_snapshot_triggers",
    ] {
        transaction.execute(
            &format!(
                "DELETE FROM {table} WHERE snapshot_id IN (
                    SELECT snapshot_id FROM unread_snapshots WHERE workspace_id=?1 AND agent_id=?2
                )"
            ),
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
    }
    for table in [
        "unread_snapshots",
        "agent_inbox_items",
        "agent_chat_cursors",
        "agent_chat_attention",
        "agent_named_secrets",
    ] {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE workspace_id=?1 AND agent_id=?2"),
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
    }
    Ok(())
}

fn load_agent_record(c: &Connection, w: &WorkspaceId, a: &AgentId) -> Result<AgentRecord> {
    let row:StoredAgentRow=c.query_row("SELECT name,agent_id,created_at,updated_at,role,bio,avatar,soul,computer_id,harness_id,model,thinking,auto_reaction,cli_path,env_json,provider_agent_config_json,session_config_json,attention_json,provider_state_json,placement_generation FROM agents WHERE workspace_id=?1 AND agent_id=?2",params![w.as_str(),a.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?,r.get(13)?,r.get(14)?,r.get(15)?,r.get(16)?,r.get(17)?,r.get(18)?,r.get(19)?)))?;
    Ok(AgentRecord {
        workspace_id: w.clone(),
        agent_id: aid(row.1)?,
        name: row.0,
        created_at: row.2,
        updated_at: row.3,
        role: row.4,
        bio: row.5,
        avatar: row.6,
        soul: row.7,
        computer_id: row.8,
        placement_generation: row.19,
        harness_id: row.9,
        model: row.10,
        thinking: row.11,
        auto_reaction: row.12,
        cli_path: row.13,
        env: serde_json::from_str(&row.14)?,
        provider_agent_config: serde_json::from_str(&row.15)?,
        provider_state: serde_json::from_str(&row.18)?,
        session_config: serde_json::from_str(&row.16)?,
        attention: serde_json::from_str(&row.17)?,
    })
}
pub(super) fn aid(s: String) -> rusqlite::Result<AgentId> {
    AgentId::new(s).map_err(conv)
}

impl Storage {
    pub fn create_agent(&self, s: AgentSpec) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO agents(workspace_id,agent_id,name,created_at,updated_at,computer_id) VALUES(?1,?2,?3,?4,?4,?5)",
            params![s.workspace_id.as_str(), s.id.as_str(), s.name, s.now, s.computer_id],
        )?;
        Ok(())
    }
    pub fn list_agents(&self, w: &WorkspaceId) -> Result<Vec<AgentView>> {
        let c = self.read_conn()?;
        let mut s = c.prepare(
            "SELECT workspace_id,agent_id,name FROM agents WHERE workspace_id=?1 ORDER BY agent_id",
        )?;
        Ok(s.query_map([w.as_str()], |r| {
            Ok(AgentView {
                workspace_id: wid(r.get(0)?)?,
                id: aid(r.get(1)?)?,
                name: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn agent_record(&self, w: &WorkspaceId, a: &AgentId) -> Result<AgentRecord> {
        let c = self.read_conn()?;
        load_agent_record(&c, w, a)
    }
    pub fn list_agent_records(&self, w: &WorkspaceId) -> Result<Vec<AgentRecord>> {
        let c = self.read_conn()?;
        let mut s =
            c.prepare("SELECT agent_id FROM agents WHERE workspace_id=?1 ORDER BY agent_id")?;
        let ids = s
            .query_map([w.as_str()], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(s);
        let mut out = Vec::new();
        for id in ids {
            out.push(load_agent_record(&c, w, &aid(id)?)?);
        }
        Ok(out)
    }
    pub fn upsert_agent_record(&self, record: AgentRecord) -> Result<AgentRecord> {
        self.commit_agent_record(record, &Value::Object(Default::default()))
    }
    pub fn insert_new_agent_record(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> Result<AgentRecord> {
        self.write_agent_record(record, provider_secrets, true)
    }
    pub fn update_agent_record_with_secrets(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> Result<AgentRecord> {
        self.write_agent_record(record, provider_secrets, false)
    }
    /// Atomically saves the Agent record and any newly supplied opaque Provider
    /// secrets. Existing secrets not present in `provider_secrets` are kept.
    pub fn commit_agent_record(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
    ) -> Result<AgentRecord> {
        match self.agent_record(&record.workspace_id, &record.agent_id) {
            Ok(_) => self.update_agent_record_with_secrets(record, provider_secrets),
            Err(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows)) => {
                self.insert_new_agent_record(record, provider_secrets)
            }
            Err(error) => Err(error),
        }
    }
    fn write_agent_record(
        &self,
        record: AgentRecord,
        provider_secrets: &Value,
        insert: bool,
    ) -> Result<AgentRecord> {
        if record.name.trim().is_empty() {
            return Err(StorageError::Invalid("empty agent name".into()));
        }
        if !record.provider_state.is_object() {
            return Err(StorageError::Invalid(
                "agent provider state must be a JSON object".into(),
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
        for attention in record.attention.values() {
            attention.validate()?;
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let changed = if insert {
            t.execute("INSERT INTO agents(workspace_id,agent_id,name,created_at,updated_at,role,bio,avatar,soul,computer_id,harness_id,model,thinking,auto_reaction,cli_path,env_json,provider_agent_config_json,session_config_json,attention_json,provider_state_json)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",params![record.workspace_id.as_str(),record.agent_id.as_str(),record.name,record.created_at,record.updated_at,record.role,record.bio,record.avatar,record.soul,record.computer_id,record.harness_id,record.model,record.thinking,record.auto_reaction,record.cli_path,serde_json::to_string(&record.env)?,serde_json::to_string(&record.provider_agent_config)?,serde_json::to_string(&record.session_config)?,serde_json::to_string(&record.attention)?,serde_json::to_string(&record.provider_state)?])?
        } else {
            t.execute("UPDATE agents SET name=?3,updated_at=?4,role=?5,bio=?6,avatar=?7,soul=?8,computer_id=?9,harness_id=?10,model=?11,thinking=?12,auto_reaction=?13,cli_path=?14,env_json=?15,provider_agent_config_json=?16,session_config_json=?17,attention_json=?18,provider_state_json=?19 WHERE workspace_id=?1 AND agent_id=?2",params![record.workspace_id.as_str(),record.agent_id.as_str(),record.name,record.updated_at,record.role,record.bio,record.avatar,record.soul,record.computer_id,record.harness_id,record.model,record.thinking,record.auto_reaction,record.cli_path,serde_json::to_string(&record.env)?,serde_json::to_string(&record.provider_agent_config)?,serde_json::to_string(&record.session_config)?,serde_json::to_string(&record.attention)?,serde_json::to_string(&record.provider_state)?])?
        };
        if changed != 1 {
            return Err(StorageError::Invalid(format!(
                "agent {} not found",
                record.agent_id
            )));
        }
        let mut revision: i64 = t.query_row(
            "SELECT coalesce(max(revision),0) FROM agent_named_secrets WHERE workspace_id=?1 AND agent_id=?2",
            params![record.workspace_id.as_str(), record.agent_id.as_str()],
            |row| row.get(0),
        )?;
        for (name, value) in secrets {
            revision += 1;
            t.execute(
                "INSERT INTO agent_named_secrets(workspace_id,agent_id,name,value,revision,updated_at)VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,agent_id,name) DO UPDATE SET value=excluded.value,revision=excluded.revision,updated_at=excluded.updated_at",
                params![record.workspace_id.as_str(), record.agent_id.as_str(), name, value.as_str().unwrap_or_default(), revision, record.updated_at],
            )?;
        }
        t.commit()?;
        load_agent_record(&c, &record.workspace_id, &record.agent_id)
    }

    /// Roll back an Agent that failed during its initial Provider validation.
    /// This is intentionally not exposed as a product deletion capability.
    pub fn rollback_new_agent(&self, workspace_id: &WorkspaceId, agent_id: &AgentId) -> Result<()> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        delete_agent_state(&transaction, workspace_id, agent_id)?;
        transaction.execute(
            "DELETE FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn update_agent_record(&self, record: AgentRecord) -> Result<AgentRecord> {
        self.update_agent_record_with_secrets(record, &Value::Object(Default::default()))
    }
    pub fn delete_agent(&self, w: &WorkspaceId, a: &AgentId) -> Result<bool> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| StorageError::Invalid(error.to_string()))?
            .as_millis() as i64;
        let computer_id = transaction
            .query_row(
                "SELECT computer_id FROM agents WHERE workspace_id=?1 AND agent_id=?2",
                params![w.as_str(), a.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(computer_id) = computer_id else {
            transaction.commit()?;
            return Ok(false);
        };
        if !computer_id.is_empty() {
            transaction.execute(
                "INSERT INTO agent_file_deletions(workspace_id,agent_id,computer_id,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,agent_id) DO UPDATE SET computer_id=excluded.computer_id,created_at=excluded.created_at",
                params![w.as_str(), a.as_str(), computer_id, now],
            )?;
        }
        delete_agent_state(&transaction, w, a)?;
        let changed = transaction.execute(
            "DELETE FROM agents WHERE workspace_id=?1 AND agent_id=?2",
            params![w.as_str(), a.as_str()],
        )? == 1;
        transaction.commit()?;
        Ok(changed)
    }
    pub fn pending_agent_file_deletions(
        &self,
        computer_id: &str,
    ) -> Result<Vec<(WorkspaceId, AgentId)>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare(
            "SELECT workspace_id,agent_id FROM agent_file_deletions WHERE computer_id=?1 ORDER BY created_at,workspace_id,agent_id",
        )?;
        Ok(statement
            .query_map([computer_id], |row| {
                Ok((wid(row.get(0)?)?, aid(row.get(1)?)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn complete_agent_file_deletion(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<bool> {
        Ok(self.conn()?.execute(
            "DELETE FROM agent_file_deletions WHERE workspace_id=?1 AND agent_id=?2",
            params![workspace_id.as_str(), agent_id.as_str()],
        )? == 1)
    }
}
