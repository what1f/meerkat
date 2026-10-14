use crate::execution::load_recoverable_executions;
use crate::send::load_recoverable_sends;
use crate::{
    DatabaseConfig, RecoverableExecutionView, RecoverableSendView, Result, Storage, StorageError,
};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

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

fn migrate(c: &mut Connection) -> Result<()> {
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v > 16 {
        return Err(StorageError::Invalid(format!("unsupported schema {v}")));
    }
    if v == 0 {
        let t = c.transaction()?;
        t.execute_batch(SCHEMA)?;
        t.pragma_update(None, "user_version", 15)?;
        t.commit()?;
    } else if v == 1 {
        let t = c.transaction()?;
        t.execute_batch(MIGRATION_V2)?;
        t.pragma_update(None, "user_version", 2)?;
        t.commit()?;
    }
    if v == 1 || v == 2 {
        let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current == 2 {
            let t = c.transaction()?;
            t.execute_batch(MIGRATION_V3)?;
            t.pragma_update(None, "user_version", 3)?;
            t.commit()?;
        }
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 3 {
        let t = c.transaction()?;
        t.execute_batch(MIGRATION_V4)?;
        t.pragma_update(None, "user_version", 4)?;
        t.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 4 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V5)?;
        transaction.pragma_update(None, "user_version", 5)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 5 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V6)?;
        transaction.pragma_update(None, "user_version", 6)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 6 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V7)?;
        transaction.pragma_update(None, "user_version", 7)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 7 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V8)?;
        transaction.pragma_update(None, "user_version", 8)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 8 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V9)?;
        transaction.pragma_update(None, "user_version", 9)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 9 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V10)?;
        transaction.pragma_update(None, "user_version", 10)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 10 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V11)?;
        transaction.pragma_update(None, "user_version", 11)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 11 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V12)?;
        transaction.pragma_update(None, "user_version", 12)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 12 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V13)?;
        transaction.pragma_update(None, "user_version", 13)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 13 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V14)?;
        transaction.pragma_update(None, "user_version", 14)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 14 {
        let transaction = c.transaction()?;
        transaction.execute_batch(MIGRATION_V15)?;
        transaction.pragma_update(None, "user_version", 15)?;
        transaction.commit()?;
    }
    let current: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current == 15 {
        // Rebuild only the chat identity table. IDs and all referencing rows stay intact.
        c.pragma_update(None, "foreign_keys", false)?;
        let result = (|| -> Result<()> {
            let transaction = c.transaction()?;
            transaction.execute_batch(MIGRATION_V16)?;
            transaction.pragma_update(None, "user_version", 16)?;
            transaction.commit()?;
            Ok(())
        })();
        c.pragma_update(None, "foreign_keys", true)?;
        result?;
    }
    Ok(())
}
const MIGRATION_V16: &str = r#"
CREATE TABLE chats_v16(chat_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,channel_id TEXT NOT NULL,provider_chat_id TEXT NOT NULL,receiving_agent_id TEXT NOT NULL DEFAULT '',name TEXT,first_message_id INTEGER,last_message_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,channel_id)REFERENCES channels(workspace_id,channel_id),UNIQUE(workspace_id,channel_id,provider_chat_id,receiving_agent_id));
INSERT INTO chats_v16(chat_id,workspace_id,channel_id,provider_chat_id,name,first_message_id,last_message_id,created_at,updated_at)
SELECT chat_id,workspace_id,channel_id,provider_chat_id,name,first_message_id,last_message_id,created_at,updated_at FROM chats;
-- The stored channels schema has no attention policy; private_chat identifies DMs.
-- Only the receiving bot can successfully send to a DM. Failed sends and inbox
-- dispatches may include every Agent because of the old routing bug.
UPDATE chats_v16 SET receiving_agent_id=(
    SELECT min(m.sender_id) FROM messages m
    JOIN message_send_attempts a USING(message_id)
    WHERE m.chat_id=chats_v16.chat_id AND m.sender_kind='agent'
      AND m.direction='outbound' AND a.state='sent'
) WHERE channel_id='private_chat' AND (
    SELECT count(DISTINCT m.sender_id) FROM messages m
    JOIN message_send_attempts a USING(message_id)
    WHERE m.chat_id=chats_v16.chat_id AND m.sender_kind='agent'
      AND m.direction='outbound' AND a.state='sent'
)=1;
-- Inbox ownership is evidence only when no Agent successfully sent.
UPDATE chats_v16 SET receiving_agent_id=(
    SELECT min(agent_id) FROM agent_inbox_items WHERE chat_id=chats_v16.chat_id
) WHERE channel_id='private_chat' AND receiving_agent_id='' AND NOT EXISTS (
    SELECT 1 FROM messages m JOIN message_send_attempts a USING(message_id)
    WHERE m.chat_id=chats_v16.chat_id AND m.sender_kind='agent'
      AND m.direction='outbound' AND a.state='sent'
) AND (
    SELECT count(DISTINCT agent_id) FROM agent_inbox_items WHERE chat_id=chats_v16.chat_id
)=1;
-- Archive ambiguous histories and stop queued work for every non-receiving Agent.
UPDATE agent_inbox_items SET state='consumed'
WHERE state='pending' AND EXISTS (
    SELECT 1 FROM chats_v16 c WHERE c.chat_id=agent_inbox_items.chat_id
      AND c.channel_id='private_chat' AND c.receiving_agent_id<>agent_inbox_items.agent_id
);
DROP TABLE chats;
ALTER TABLE chats_v16 RENAME TO chats;
"#;
const SCHEMA: &str = r#"
CREATE TABLE workspaces(workspace_id TEXT PRIMARY KEY,name TEXT NOT NULL,provider_id TEXT NOT NULL,provider_config_json TEXT NOT NULL,desired_state TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,rules TEXT NOT NULL DEFAULT '',cover_resource_id INTEGER,setup_status TEXT NOT NULL DEFAULT 'pending',setup_error_code TEXT,provider_state_json TEXT NOT NULL DEFAULT '{}');
CREATE TABLE workspace_provider_secrets(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,secret_json TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE workspace_named_secrets(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,name));
CREATE TABLE agent_named_secrets(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,name),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
CREATE TABLE provider_kv(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,namespace TEXT NOT NULL,key TEXT NOT NULL,value BLOB NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,namespace,key));
CREATE TABLE resource_metadata(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,resource_id INTEGER NOT NULL,file_name TEXT NOT NULL,media_type TEXT NOT NULL,size_bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,storage_ref TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,resource_id));
CREATE TABLE workspace_resource_counters(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,next_id INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE agents(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),agent_id TEXT NOT NULL,name TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,role TEXT NOT NULL DEFAULT '',bio TEXT NOT NULL DEFAULT '',avatar TEXT,soul TEXT NOT NULL DEFAULT '',computer_id TEXT NOT NULL DEFAULT '',harness_id TEXT NOT NULL DEFAULT '',model TEXT NOT NULL DEFAULT '',thinking TEXT NOT NULL DEFAULT '',auto_reaction TEXT,cli_path TEXT,env_json TEXT NOT NULL DEFAULT '{}',provider_agent_config_json TEXT NOT NULL DEFAULT '{}',session_config_json TEXT NOT NULL DEFAULT '{}',attention_json TEXT NOT NULL DEFAULT '{}',provider_state_json TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(workspace_id,agent_id));
CREATE TABLE agent_file_deletions(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,computer_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id));
CREATE TABLE people(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,person_id TEXT NOT NULL,team_role TEXT NOT NULL CHECK(team_role IN('external','member','lead')),provider_instance_id TEXT NOT NULL,external_id TEXT NOT NULL,provider_identity_state_json TEXT NOT NULL DEFAULT '{}',provider_state_revision INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,person_id),UNIQUE(workspace_id,provider_instance_id,external_id));
CREATE UNIQUE INDEX people_one_lead_per_workspace ON people(workspace_id) WHERE team_role='lead';
CREATE TABLE channels(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),channel_id TEXT NOT NULL,name TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,channel_id));
CREATE TABLE chats(chat_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,channel_id TEXT NOT NULL,provider_chat_id TEXT NOT NULL,name TEXT,first_message_id INTEGER,last_message_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,channel_id)REFERENCES channels(workspace_id,channel_id),UNIQUE(workspace_id,channel_id,provider_chat_id));
CREATE TABLE messages(message_id INTEGER PRIMARY KEY AUTOINCREMENT,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),provider_message_id TEXT,provider_event_id TEXT,direction TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT,sender_name TEXT,content_markdown TEXT,mentions_json TEXT NOT NULL DEFAULT '[]',reply_to_provider_message_id TEXT,resources_json TEXT NOT NULL DEFAULT '[]',send_status TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);CREATE UNIQUE INDEX message_provider_key ON messages(chat_id,provider_message_id)WHERE provider_message_id IS NOT NULL;
CREATE TABLE reactions(reaction_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),target_message_id INTEGER NOT NULL REFERENCES messages(message_id),provider_event_id TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT NOT NULL,reaction TEXT NOT NULL,action TEXT NOT NULL,origin TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,UNIQUE(workspace_id,provider_event_id));
CREATE TABLE agent_inbox_items(inbox_item_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),source_id INTEGER NOT NULL,source_kind TEXT NOT NULL DEFAULT 'message',trigger_reason_json TEXT NOT NULL DEFAULT '{"kind":"follow"}',available_at INTEGER NOT NULL,state TEXT NOT NULL,claimed_execution_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id),UNIQUE(workspace_id,agent_id,source_id));
CREATE TABLE agent_chat_cursors(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE agent_chat_attention(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),attention_json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE unread_snapshots(snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,completed_at INTEGER,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
CREATE TABLE unread_snapshot_chats(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_from_message_id INTEGER,read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),unread_count INTEGER NOT NULL,trigger_summary TEXT NOT NULL,PRIMARY KEY(snapshot_id,chat_id));
CREATE TABLE unread_snapshot_messages(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,chat_id,message_id));
CREATE TABLE unread_snapshot_triggers(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),reason_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,position));
CREATE TABLE executions(execution_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),state TEXT NOT NULL,trigger_summary TEXT NOT NULL,final_output TEXT,error_code TEXT,created_at INTEGER NOT NULL,started_at INTEGER,completed_at INTEGER,updated_at INTEGER NOT NULL,attempt INTEGER NOT NULL DEFAULT 1,token_hash TEXT,token_expires_at INTEGER,FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));CREATE UNIQUE INDEX one_active_execution ON executions(workspace_id,agent_id)WHERE state IN('queued','assigning','running','recovering');
CREATE TABLE execution_attempts(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,state TEXT NOT NULL,last_event_sequence INTEGER NOT NULL DEFAULT 0,native_session_id TEXT,error_code TEXT,error_message TEXT,lease_expires_at INTEGER NOT NULL,assigned_at INTEGER NOT NULL,accepted_at INTEGER,finished_at INTEGER,PRIMARY KEY(execution_id,attempt));
CREATE INDEX active_attempt_lease ON execution_attempts(computer_id,lease_expires_at) WHERE state IN('offered','accepted','running');
CREATE TABLE execution_events(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,sequence INTEGER NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL,occurred_at INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,sequence),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt));
CREATE TABLE steer_deliveries(steer_delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,rendered_markdown TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,accepted_at INTEGER,last_error_at INTEGER,FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt),UNIQUE(execution_id,attempt,inbox_item_id));
CREATE TABLE execution_attempt_authorizations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,context_json TEXT NOT NULL,context_fingerprint TEXT NOT NULL,PRIMARY KEY(execution_id,attempt));
CREATE TABLE execution_attempt_capabilities(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,name TEXT NOT NULL,display_name TEXT NOT NULL,description TEXT NOT NULL,kind INTEGER NOT NULL,input_schema_json TEXT NOT NULL,output_schema_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,name),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempt_authorizations(execution_id,attempt));
CREATE TABLE execution_deliveries(delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL REFERENCES executions(execution_id),state TEXT NOT NULL,created_at INTEGER NOT NULL,delivered_at INTEGER);CREATE TABLE execution_delivery_items(delivery_id INTEGER NOT NULL REFERENCES execution_deliveries(delivery_id),inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),PRIMARY KEY(delivery_id,inbox_item_id));CREATE TABLE execution_delivery_chats(delivery_id INTEGER NOT NULL REFERENCES execution_deliveries(delivery_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),read_to_message_id INTEGER NOT NULL REFERENCES messages(message_id),PRIMARY KEY(delivery_id,chat_id));
CREATE TABLE message_send_attempts(send_attempt_id INTEGER PRIMARY KEY AUTOINCREMENT,message_id INTEGER NOT NULL REFERENCES messages(message_id),execution_id INTEGER REFERENCES executions(execution_id),request_key TEXT NOT NULL UNIQUE,state TEXT NOT NULL,provider_message_id TEXT,error_code TEXT,started_at INTEGER NOT NULL,finished_at INTEGER);CREATE TABLE tool_invocations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,tool_call_id TEXT NOT NULL,tool_name TEXT NOT NULL,arguments_json TEXT NOT NULL,state TEXT NOT NULL,result_json TEXT,started_at INTEGER NOT NULL,finished_at INTEGER,PRIMARY KEY(execution_id,attempt,tool_call_id));
"#;

const MIGRATION_V2: &str = r#"
ALTER TABLE messages ADD COLUMN provider_event_id TEXT;
ALTER TABLE messages ADD COLUMN mentions_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN reply_to_provider_message_id TEXT;
ALTER TABLE messages ADD COLUMN resources_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agent_inbox_items ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'message';
ALTER TABLE agent_inbox_items ADD COLUMN trigger_reason_json TEXT NOT NULL DEFAULT '{"kind":"follow"}';
ALTER TABLE message_send_attempts ADD COLUMN error_code TEXT;
ALTER TABLE message_send_attempts ADD COLUMN finished_at INTEGER;
CREATE TABLE reactions(reaction_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),target_message_id INTEGER NOT NULL REFERENCES messages(message_id),provider_event_id TEXT NOT NULL,sender_kind TEXT NOT NULL,sender_id TEXT NOT NULL,sender_name TEXT NOT NULL,reaction TEXT NOT NULL,action TEXT NOT NULL,origin TEXT NOT NULL,occurred_at INTEGER NOT NULL,created_at INTEGER NOT NULL,UNIQUE(workspace_id,provider_event_id));
CREATE TABLE unread_snapshot_triggers(snapshot_id INTEGER NOT NULL REFERENCES unread_snapshots(snapshot_id),chat_id INTEGER NOT NULL REFERENCES chats(chat_id),message_id INTEGER NOT NULL REFERENCES messages(message_id),reason_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(snapshot_id,position));
"#;
const MIGRATION_V3: &str = r#"
ALTER TABLE message_send_attempts ADD COLUMN execution_id INTEGER REFERENCES executions(execution_id);
"#;
const MIGRATION_V4: &str = r#"
ALTER TABLE agents ADD COLUMN role TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN bio TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN avatar TEXT;
ALTER TABLE agents ADD COLUMN soul TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN computer_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN harness_id TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN model TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN thinking TEXT NOT NULL DEFAULT '';
ALTER TABLE agents ADD COLUMN auto_reaction TEXT;
ALTER TABLE agents ADD COLUMN cli_path TEXT;
ALTER TABLE agents ADD COLUMN env_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN provider_agent_config_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN session_config_json TEXT NOT NULL DEFAULT '{}';
CREATE TABLE workspace_named_secrets(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,name));
CREATE TABLE provider_kv(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,namespace TEXT NOT NULL,key TEXT NOT NULL,value BLOB NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,namespace,key));
CREATE TABLE resource_metadata(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,resource_id INTEGER NOT NULL,file_name TEXT NOT NULL,media_type TEXT NOT NULL,size_bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,storage_ref TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,resource_id));
"#;
const MIGRATION_V5: &str = r#"
ALTER TABLE workspaces ADD COLUMN rules TEXT NOT NULL DEFAULT '';
ALTER TABLE workspaces ADD COLUMN cover_resource_id INTEGER;
ALTER TABLE workspaces ADD COLUMN setup_status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE workspaces ADD COLUMN setup_error_code TEXT;
ALTER TABLE agents ADD COLUMN attention_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE executions ADD COLUMN attempt INTEGER NOT NULL DEFAULT 1;
ALTER TABLE executions ADD COLUMN token_hash TEXT;
ALTER TABLE executions ADD COLUMN token_expires_at INTEGER;
CREATE TABLE workspace_resource_counters(workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,next_id INTEGER NOT NULL,updated_at INTEGER NOT NULL);
"#;
const MIGRATION_V6: &str = r#"
CREATE TABLE execution_attempts(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,state TEXT NOT NULL,last_event_sequence INTEGER NOT NULL DEFAULT 0,native_session_id TEXT,error_code TEXT,error_message TEXT,lease_expires_at INTEGER NOT NULL,assigned_at INTEGER NOT NULL,accepted_at INTEGER,finished_at INTEGER,PRIMARY KEY(execution_id,attempt));
CREATE INDEX active_attempt_lease ON execution_attempts(computer_id,lease_expires_at) WHERE state IN('offered','accepted','running');
CREATE TABLE execution_events(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,sequence INTEGER NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL,occurred_at INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,sequence),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt));
"#;
const MIGRATION_V7: &str = r#"
CREATE TABLE execution_attempt_authorizations(execution_id INTEGER NOT NULL REFERENCES executions(execution_id),attempt INTEGER NOT NULL,context_json TEXT NOT NULL,context_fingerprint TEXT NOT NULL,PRIMARY KEY(execution_id,attempt));
CREATE TABLE execution_attempt_capabilities(execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,name TEXT NOT NULL,display_name TEXT NOT NULL,description TEXT NOT NULL,kind INTEGER NOT NULL,input_schema_json TEXT NOT NULL,output_schema_json TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(execution_id,attempt,name),FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempt_authorizations(execution_id,attempt));
"#;
const MIGRATION_V8: &str = r#"
CREATE TABLE steer_deliveries(steer_delivery_id INTEGER PRIMARY KEY AUTOINCREMENT,execution_id INTEGER NOT NULL,attempt INTEGER NOT NULL,inbox_item_id INTEGER NOT NULL REFERENCES agent_inbox_items(inbox_item_id),computer_id TEXT NOT NULL,computer_instance_id TEXT NOT NULL,rendered_markdown TEXT NOT NULL,state TEXT NOT NULL,created_at INTEGER NOT NULL,accepted_at INTEGER,last_error_at INTEGER,FOREIGN KEY(execution_id,attempt) REFERENCES execution_attempts(execution_id,attempt),UNIQUE(execution_id,attempt,inbox_item_id));
"#;
const MIGRATION_V9: &str = r#"
CREATE TABLE agent_named_secrets(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,name TEXT NOT NULL,value TEXT NOT NULL,revision INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,name),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
"#;
const MIGRATION_V10: &str = r#"
ALTER TABLE workspaces ADD COLUMN provider_state_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agents ADD COLUMN provider_state_json TEXT NOT NULL DEFAULT '{}';
"#;
const MIGRATION_V11: &str = r#"
CREATE TABLE agent_chat_attention(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,chat_id INTEGER NOT NULL REFERENCES chats(chat_id),attention_json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id,chat_id),FOREIGN KEY(workspace_id,agent_id)REFERENCES agents(workspace_id,agent_id));
"#;
const MIGRATION_V12: &str = r#"
ALTER TABLE agents ADD COLUMN deleted_at INTEGER;
"#;
const MIGRATION_V13: &str = r#"
CREATE TABLE people(workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,person_id TEXT NOT NULL,team_role TEXT NOT NULL CHECK(team_role IN('external','member','lead')),provider_instance_id TEXT NOT NULL,external_id TEXT NOT NULL,provider_identity_state_json TEXT NOT NULL DEFAULT '{}',provider_state_revision INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,person_id),UNIQUE(workspace_id,provider_instance_id,external_id));
CREATE UNIQUE INDEX people_one_lead_per_workspace ON people(workspace_id) WHERE team_role='lead';
"#;
const MIGRATION_V14: &str = r#"
CREATE TABLE agent_file_deletions(workspace_id TEXT NOT NULL,agent_id TEXT NOT NULL,computer_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(workspace_id,agent_id),FOREIGN KEY(workspace_id,agent_id) REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT);
"#;
const MIGRATION_V15: &str = r#"
CREATE TEMP TABLE deleted_agents_v15 AS
SELECT workspace_id,agent_id FROM agents WHERE deleted_at IS NOT NULL;

CREATE TABLE agent_file_deletions_v15(
    workspace_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    computer_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(workspace_id,agent_id)
);
INSERT INTO agent_file_deletions_v15(workspace_id,agent_id,computer_id,created_at)
SELECT workspace_id,agent_id,computer_id,created_at FROM agent_file_deletions;
DROP TABLE agent_file_deletions;
ALTER TABLE agent_file_deletions_v15 RENAME TO agent_file_deletions;

DELETE FROM execution_delivery_items
WHERE delivery_id IN (
    SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
        SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
    )
) OR inbox_item_id IN (
    SELECT inbox_item_id FROM agent_inbox_items JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_delivery_chats WHERE delivery_id IN (
    SELECT delivery_id FROM execution_deliveries WHERE execution_id IN (
        SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
    )
);
DELETE FROM steer_deliveries
WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
) OR inbox_item_id IN (
    SELECT inbox_item_id FROM agent_inbox_items JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_attempt_capabilities WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_attempt_authorizations WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_events WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_attempts WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM message_send_attempts WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM tool_invocations WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM execution_deliveries WHERE execution_id IN (
    SELECT execution_id FROM executions JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM executions WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=executions.workspace_id
      AND deleted_agents_v15.agent_id=executions.agent_id
);
DELETE FROM unread_snapshot_chats WHERE snapshot_id IN (
    SELECT snapshot_id FROM unread_snapshots JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM unread_snapshot_messages WHERE snapshot_id IN (
    SELECT snapshot_id FROM unread_snapshots JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM unread_snapshot_triggers WHERE snapshot_id IN (
    SELECT snapshot_id FROM unread_snapshots JOIN deleted_agents_v15 USING(workspace_id,agent_id)
);
DELETE FROM unread_snapshots WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=unread_snapshots.workspace_id
      AND deleted_agents_v15.agent_id=unread_snapshots.agent_id
);
DELETE FROM agent_inbox_items WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_inbox_items.workspace_id
      AND deleted_agents_v15.agent_id=agent_inbox_items.agent_id
);
DELETE FROM agent_chat_cursors WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_chat_cursors.workspace_id
      AND deleted_agents_v15.agent_id=agent_chat_cursors.agent_id
);
DELETE FROM agent_chat_attention WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_chat_attention.workspace_id
      AND deleted_agents_v15.agent_id=agent_chat_attention.agent_id
);
DELETE FROM agent_named_secrets WHERE EXISTS (
    SELECT 1 FROM deleted_agents_v15
    WHERE deleted_agents_v15.workspace_id=agent_named_secrets.workspace_id
      AND deleted_agents_v15.agent_id=agent_named_secrets.agent_id
);
DELETE FROM agents WHERE deleted_at IS NOT NULL;
DROP TABLE deleted_agents_v15;
ALTER TABLE agents DROP COLUMN deleted_at;
"#;

impl Storage {
    pub fn open(config: DatabaseConfig) -> Result<Self> {
        if let Some(p) = config.path.parent() {
            std::fs::create_dir_all(p).map_err(|e| StorageError::Invalid(e.to_string()))?;
        }
        let mut c = Connection::open(&config.path)?;
        configure(&c)?;
        migrate(&mut c)?;
        Ok(Self {
            config,
            connection: Mutex::new(c),
        })
    }
    pub(super) fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
    pub fn database_path(&self) -> &Path {
        &self.config.path
    }
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn()?
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
    pub fn sqlite_settings(&self) -> Result<SqliteSettings> {
        let c = self.conn()?;
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
        let c = self
            .connection
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
