use crate::execution::{abandon_execution, load_recoverable_executions};
use crate::send::load_recoverable_sends;
use crate::{
    DatabaseConfig, RecoverableExecutionView, RecoverableSendView, Result, Storage, StorageError,
};
use rusqlite::Connection;
use std::ops::Deref;
use std::path::Path;
use std::sync::{Condvar, Mutex, MutexGuard};

// Writes retain one connection for the whole transaction. Read connections are
// checked out independently and cannot accidentally start a write transaction.
pub(crate) struct ConnectionPool {
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
    available: Condvar,
}

pub(super) struct ReadConnection<'a> {
    connection: Option<Connection>,
    pool: &'a ConnectionPool,
}

impl Deref for ReadConnection<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection.as_ref().expect("checked out connection")
    }
}

impl Drop for ReadConnection<'_> {
    fn drop(&mut self) {
        if let Ok(mut readers) = self.pool.readers.lock() {
            readers.push(self.connection.take().expect("checked out connection"));
            self.pool.available.notify_one();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryState {
    pub executions: Vec<RecoverableExecutionView>,
    pub sends: Vec<RecoverableSendView>,
    pub transitioned_executions: usize,
    pub transitioned_sends: usize,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteSettings {
    pub journal_mode: String,
    pub foreign_keys: bool,
}

fn configure(c: &Connection) -> Result<()> {
    c.busy_timeout(std::time::Duration::from_secs(2))?;
    c.execute_batch("PRAGMA foreign_keys=ON;PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL;")?;
    Ok(())
}

fn initialize_schema(c: &mut Connection) -> Result<()> {
    let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    match version {
        0 => {
            let transaction = c.transaction()?;
            transaction.execute_batch(BASELINE_SCHEMA)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction.commit()?;
        }
        1 => {}
        version => return Err(StorageError::UnsupportedSchemaVersion(version)),
    }
    Ok(())
}

const BASELINE_SCHEMA: &str = r#"
CREATE TABLE workspaces(workspace_id TEXT PRIMARY KEY,name TEXT NOT NULL,provider_id TEXT NOT NULL,provider_config_json TEXT NOT NULL,desired_state TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,rules TEXT NOT NULL DEFAULT '',cover_resource_id INTEGER,setup_status TEXT NOT NULL DEFAULT 'pending',setup_error_code TEXT,provider_state_json TEXT NOT NULL DEFAULT '{}');
CREATE TABLE workspace_named_secrets(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,name));
CREATE TABLE agent_named_secrets(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,name),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
CREATE TABLE provider_kv(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,namespace TEXT NOT NULL,key TEXT NOT NULL,value BLOB NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,namespace,key));
CREATE TABLE resource_metadata(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,resource_id INTEGER NOT NULL,file_name TEXT NOT NULL,media_type TEXT NOT NULL,size_bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,storage_ref TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,resource_id));
CREATE TABLE workspace_resource_counters(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,next_id INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE figure_packs(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),id TEXT NOT NULL,metadata_json TEXT NOT NULL,PRIMARY KEY(workspace_id,id));
CREATE TABLE agents(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),agent_id TEXT NOT NULL,name TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,role TEXT NOT NULL DEFAULT '',bio TEXT NOT NULL DEFAULT '',avatar TEXT,figure TEXT NOT NULL DEFAULT 'default',launch_arguments_json TEXT NOT NULL DEFAULT '[]',soul TEXT NOT NULL DEFAULT '',machine_id TEXT NOT NULL REFERENCES machines(machine_id) CHECK(length(machine_id)>0),harness_id TEXT NOT NULL DEFAULT '',model TEXT NOT NULL DEFAULT '',thinking TEXT NOT NULL DEFAULT '',auto_reaction TEXT,cli_path TEXT,env_json TEXT NOT NULL DEFAULT '{}',provider_agent_config_json TEXT NOT NULL DEFAULT '{}',session_config_json TEXT NOT NULL DEFAULT '{}',attention_json TEXT NOT NULL DEFAULT '{}',provider_state_json TEXT NOT NULL DEFAULT '{}', pause_reason TEXT, failure_fingerprint TEXT, failure_streak INTEGER NOT NULL DEFAULT 0, placement_generation INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(workspace_id,agent_id));
CREATE TABLE agent_file_deletions(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,machine_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id));
CREATE TABLE people(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,person_id TEXT NOT NULL,team_role TEXT NOT NULL CHECK(team_role IN('external','member','lead')),provider_instance_id TEXT NOT NULL,external_id TEXT NOT NULL,name TEXT,avatar_json TEXT,position TEXT NOT NULL DEFAULT '',tenant_id TEXT,provider_identity_state_json TEXT NOT NULL DEFAULT '{}',provider_state_revision INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,person_id),UNIQUE(workspace_id,provider_instance_id,external_id));
CREATE UNIQUE INDEX people_one_lead_per_workspace ON people(workspace_id) WHERE team_role='lead';
CREATE TABLE channels(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),channel_id TEXT NOT NULL,name TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,channel_id));
CREATE TABLE messages(message_id INTEGER PRIMARY KEY AUTOINCREMENT,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),provider_message_id TEXT,provider_event_id TEXT,direction TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT,sender_name TEXT,content_markdown TEXT,mentions_json TEXT NOT NULL DEFAULT '[]',reply_to_provider_message_id TEXT,resources_json TEXT NOT NULL DEFAULT '[]',send_status TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE message_provider_parts(chat_id INTEGER NOT NULL REFERENCES chats(chat_id),provider_message_id TEXT NOT NULL,message_id INTEGER NOT NULL REFERENCES messages(message_id) ON DELETE CASCADE,PRIMARY KEY(chat_id,provider_message_id));
CREATE UNIQUE INDEX message_provider_key ON messages(chat_id,provider_message_id)WHERE provider_message_id IS NOT NULL;
CREATE TABLE reactions(reaction_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),target_message_id INTEGER NOT NULL REFERENCES messages(message_id),provider_event_id TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT NOT NULL,reaction TEXT NOT NULL,action TEXT NOT NULL,origin TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,UNIQUE(workspace_id,provider_event_id));
CREATE TABLE agent_inbox_items(inbox_item_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),source_id INTEGER NOT NULL,source_kind TEXT NOT NULL DEFAULT 'message',trigger_reason_json TEXT NOT NULL DEFAULT '{"kind":"follow"}',available_at INTEGER NOT NULL,state TEXT NOT NULL,claimed_execution_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL, retry_batch_id INTEGER, failure_count INTEGER NOT NULL DEFAULT 0,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id),UNIQUE(workspace_id,agent_id,source_id));
CREATE TABLE agent_chat_cursors(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE agent_chat_attention(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),attention_json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE unread_snapshots(snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,completed_at INTEGER,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE unread_snapshot_chats(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_from_message_id INTEGER,read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),unread_count INTEGER NOT NULL,trigger_summary TEXT NOT NULL,PRIMARY KEY(snapshot_id,chat_id));
CREATE TABLE unread_snapshot_messages(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,chat_id,message_id));
CREATE TABLE unread_snapshot_triggers(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),reason_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,position));
CREATE TABLE executions(execution_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),state TEXT NOT NULL,trigger_summary TEXT NOT NULL,final_output TEXT,error_code TEXT,error_message TEXT,created_at INTEGER NOT NULL,started_at INTEGER,completed_at INTEGER,updated_at INTEGER NOT NULL,attempt INTEGER NOT NULL DEFAULT 1,token_hash TEXT,token_expires_at INTEGER, retry_policy_applied INTEGER NOT NULL DEFAULT 0, placement_generation INTEGER NOT NULL, placement_machine_id TEXT NOT NULL,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE UNIQUE INDEX one_active_execution ON executions(workspace_id,agent_id)WHERE state IN('queued','assigning','running','recovering');
CREATE TABLE execution_attempts(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,machine_id TEXT NOT NULL,machine_instance_id TEXT NOT NULL,state TEXT NOT NULL,last_event_sequence INTEGER NOT NULL DEFAULT 0,native_session_id TEXT,error_code TEXT,error_message TEXT,lease_expires_at INTEGER NOT NULL,assigned_at INTEGER NOT NULL,accepted_at INTEGER,finished_at INTEGER, failure_json TEXT,PRIMARY KEY(execution_id,attempt));
CREATE INDEX active_attempt_lease ON execution_attempts(machine_id,lease_expires_at) WHERE state IN('offered','accepted','running');
CREATE TABLE execution_events(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,sequence INTEGER NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL,occurred_at INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,sequence),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt));
CREATE TABLE steer_deliveries(steer_delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),machine_id TEXT NOT NULL,machine_instance_id TEXT NOT NULL,rendered_markdown TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,accepted_at INTEGER,last_error_at INTEGER,FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt),UNIQUE(execution_id,attempt,inbox_item_id));
CREATE TABLE execution_attempt_authorizations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,context_json TEXT NOT NULL,context_fingerprint TEXT NOT NULL,PRIMARY KEY(execution_id,attempt));
CREATE TABLE execution_attempt_capabilities(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,name TEXT NOT NULL,display_name TEXT NOT NULL,description TEXT NOT NULL,kind INTEGER NOT NULL,input_schema_json TEXT NOT NULL,output_schema_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,name),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempt_authorizations(execution_id,attempt));
CREATE TABLE execution_deliveries(delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL REFERENCES executions(execution_id),state TEXT NOT NULL,created_at INTEGER NOT NULL,delivered_at INTEGER);
CREATE TABLE execution_delivery_items(delivery_id INTEGER NOT NULL REFERENCES execution_deliveries(delivery_id),inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),PRIMARY KEY(delivery_id,inbox_item_id));
CREATE TABLE execution_delivery_chats(delivery_id INTEGER NOT NULL REFERENCES execution_deliveries(delivery_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),PRIMARY KEY(delivery_id,chat_id));
CREATE TABLE message_send_attempts(send_attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,message_id INTEGER NOT NULL REFERENCES messages(message_id),execution_id INTEGER REFERENCES executions(execution_id),request_key TEXT NOT NULL UNIQUE,state TEXT NOT NULL,provider_message_id TEXT,error_code TEXT,started_at INTEGER NOT NULL,finished_at INTEGER);
CREATE TABLE tool_invocations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,tool_call_id TEXT NOT NULL,tool_name TEXT NOT NULL,arguments_json TEXT NOT NULL,state TEXT NOT NULL,result_json TEXT,started_at INTEGER NOT NULL,finished_at INTEGER,PRIMARY KEY(execution_id,attempt,tool_call_id));
CREATE TABLE "chats"(chat_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,channel_id TEXT NOT NULL,provider_chat_id TEXT NOT NULL,recipient_person_id TEXT,receiving_agent_id TEXT NOT NULL DEFAULT '',name TEXT,first_message_id INTEGER,last_message_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,channel_id)REFERENCES channels(workspace_id,channel_id),UNIQUE(workspace_id,channel_id,provider_chat_id,receiving_agent_id));
CREATE TABLE chat_receivers(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE failure_notices(notice_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL,fingerprint TEXT NOT NULL,content TEXT NOT NULL,created_at INTEGER NOT NULL,sent_at INTEGER);
CREATE INDEX failure_notice_dedupe ON failure_notices(workspace_id,agent_id,chat_id,fingerprint,created_at);
CREATE TABLE machines(machine_id TEXT PRIMARY KEY,is_local INTEGER NOT NULL DEFAULT 0,display_name TEXT NOT NULL,credential_hash TEXT NOT NULL UNIQUE,paired_at INTEGER NOT NULL,revoked_at INTEGER,last_seen_at INTEGER,os TEXT NOT NULL,version TEXT NOT NULL,inventory_json TEXT NOT NULL DEFAULT '[]');
CREATE UNIQUE INDEX machines_one_local ON machines(is_local) WHERE is_local=1;
CREATE TABLE pairing_codes(code_hash TEXT PRIMARY KEY,issued_at INTEGER NOT NULL,used_at INTEGER);
CREATE TRIGGER agent_placement_changed AFTER UPDATE OF machine_id,harness_id ON agents
WHEN OLD.machine_id != NEW.machine_id OR OLD.harness_id != NEW.harness_id
BEGIN
    UPDATE agents SET placement_generation=OLD.placement_generation+1
    WHERE workspace_id=NEW.workspace_id AND agent_id=NEW.agent_id;
END;
"#;

impl Storage {
    pub fn open(config: DatabaseConfig) -> Result<Self> {
        if let Some(p) = config.path.parent() {
            std::fs::create_dir_all(p).map_err(|e| StorageError::Invalid(e.to_string()))?;
        }
        let mut c = Connection::open(&config.path)?;
        configure(&c)?;
        initialize_schema(&mut c)?;
        let mut readers = Vec::new();
        for _ in 0..4 {
            let reader = Connection::open_with_flags(
                &config.path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            reader.busy_timeout(std::time::Duration::from_secs(2))?;
            reader.execute_batch("PRAGMA foreign_keys=ON;PRAGMA query_only=ON;")?;
            readers.push(reader);
        }
        Ok(Self {
            config,
            pool: ConnectionPool {
                writer: Mutex::new(c),
                readers: Mutex::new(readers),
                available: Condvar::new(),
            },
        })
    }
    pub(super) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.pool
            .writer
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
    pub(super) fn read_conn(&self) -> Result<ReadConnection<'_>> {
        let mut readers = self
            .pool
            .readers
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        loop {
            if let Some(connection) = readers.pop() {
                return Ok(ReadConnection {
                    connection: Some(connection),
                    pool: &self.pool,
                });
            }
            readers = self
                .pool
                .available
                .wait(readers)
                .map_err(|_| StorageError::LockPoisoned)?;
        }
    }
    pub fn database_path(&self) -> &Path {
        &self.config.path
    }
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .read_conn()?
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
    pub fn sqlite_settings(&self) -> Result<SqliteSettings> {
        let c = self.read_conn()?;
        let j: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        let f: i64 = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        Ok(SqliteSettings {
            journal_mode: j.to_ascii_lowercase(),
            foreign_keys: f == 1,
        })
    }
    pub fn recover_startup(&self, now: i64) -> Result<RecoveryState> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        t.execute(
            "UPDATE execution_deliveries SET state='abandoned' WHERE state='pending' AND execution_id IN (SELECT execution_id FROM executions WHERE state IN('succeeded','failed','cancelled'))",
            [],
        )?;
        let transitioned_sends=t.execute("UPDATE message_send_attempts SET state='unknown',finished_at=coalesce(finished_at,?1) WHERE state='sending'",[now])?;
        t.execute(
            "UPDATE messages SET send_status='unknown',updated_at=?1 WHERE send_status='sending'",
            [now],
        )?;
        t.execute("UPDATE tool_invocations SET state='unknown',finished_at=coalesce(finished_at,?1) WHERE state='running'",[now])?;
        let transitioned_executions=t.execute("UPDATE executions SET state='recovering',updated_at=?1 WHERE state IN('queued','assigning','running')",[now])?;
        // Preparation has no Machine owner to reconcile. Release its frozen
        // inbox batch so startup can prepare it again.
        let mut query = t.prepare("SELECT execution_id FROM executions e WHERE state='recovering' AND NOT EXISTS(SELECT 1 FROM execution_attempts a WHERE a.execution_id=e.execution_id AND a.attempt=e.attempt)")?;
        let unassigned = query
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(query);
        for id in unassigned {
            abandon_execution(
                &t,
                id,
                crate::ExecutionState::Failed,
                Some("input_preparation_interrupted"),
                now,
            )?;
        }
        t.commit()?;
        let executions = load_recoverable_executions(&c, None)?;
        let sends = load_recoverable_sends(&c, None)?;
        Ok(RecoveryState {
            executions,
            sends,
            transitioned_executions,
            transitioned_sends,
        })
    }
    pub fn close(self) -> Result<()> {
        drop(
            self.pool
                .readers
                .into_inner()
                .map_err(|_| StorageError::LockPoisoned)?,
        );
        let c = self
            .pool
            .writer
            .into_inner()
            .map_err(|_| StorageError::LockPoisoned)?;
        let (busy, _, _): (i64, i64, i64) =
            c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        if busy != 0 {
            return Err(StorageError::Busy(
                "wal checkpoint has active readers".into(),
            ));
        }
        c.close()
            .map_err(|(_, error)| StorageError::Sqlite(error))?;
        Ok(())
    }
}
