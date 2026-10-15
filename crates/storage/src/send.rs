use crate::agent::aid;
use crate::execution::expand_execution_chat_boundary;
use crate::message::cid;
use crate::workspace::{pid, wid};
use crate::{Result, Storage, StorageError};
use lankide_core::{AgentId, ChannelId, ProviderId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

type RecoverableSendRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    String,
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorView {
    pub chat_id: i64,
    pub read_to_message_id: i64,
}
#[derive(Clone, Debug)]
pub struct ToolInvocationRequest {
    pub execution_id: i64,
    pub attempt: i64,
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub now: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInvocationResult {
    pub created: bool,
    pub state: String,
    pub result: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct SendRequest {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub request_key: String,
    pub sender_agent_id: AgentId,
    pub content: String,
    pub now: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendAttemptView {
    pub created: bool,
    pub message_id: i64,
    pub request_key: String,
    pub state: String,
    pub provider_message_id: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GuardedSendResult {
    Ready(SendAttemptView),
    NewMessages {
        snapshot_message_id: i64,
        latest_message_id: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SendOutcome {
    Sent { provider_message_id: String },
    Unknown,
    Failed { code: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishSendResult {
    pub changed: bool,
    pub attempt: SendAttemptView,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableSendView {
    pub attempt: SendAttemptView,
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub sender_agent_id: AgentId,
    pub content: String,
}
fn load_send(c: &Connection, k: &str) -> Result<Option<SendAttemptView>> {
    Ok(c.query_row("SELECT message_id,request_key,state,provider_message_id,error_code FROM message_send_attempts WHERE request_key=?1",[k],|r|Ok(SendAttemptView{created:false,message_id:r.get(0)?,request_key:r.get(1)?,state:r.get(2)?,provider_message_id:r.get(3)?,error_code:r.get(4)?})).optional()?)
}
pub(super) fn load_recoverable_sends(
    c: &Connection,
    workspace_id: Option<&WorkspaceId>,
) -> Result<Vec<RecoverableSendView>> {
    let mut sql="SELECT a.message_id,a.request_key,a.state,a.provider_message_id,a.error_code,c.workspace_id,w.provider_id,c.channel_id,c.provider_chat_id,m.sender_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats c USING(chat_id) JOIN workspaces w USING(workspace_id) WHERE a.state IN('sending','unknown')".to_owned();
    if workspace_id.is_some() {
        sql.push_str(" AND c.workspace_id=?1");
    }
    sql.push_str(" ORDER BY a.send_attempt_id");
    let mut s = c.prepare(&sql)?;
    let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<RecoverableSendRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
            r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        ))
    };
    let rows = if let Some(w) = workspace_id {
        s.query_map([w.as_str()], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        s.query_map([], mapper)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut out = Vec::new();
    for (
        message_id,
        request_key,
        state,
        provider_message_id,
        error_code,
        workspace,
        provider,
        channel,
        provider_chat_id,
        sender,
        content,
    ) in rows
    {
        out.push(RecoverableSendView {
            attempt: SendAttemptView {
                created: false,
                message_id,
                request_key,
                state,
                provider_message_id,
                error_code,
            },
            workspace_id: wid(workspace)?,
            provider_id: pid(provider)?,
            channel_id: cid(channel)?,
            provider_chat_id,
            sender_agent_id: aid(sender)?,
            content,
        });
    }
    Ok(out)
}

impl Storage {
    pub fn cursors(&self, w: &WorkspaceId, a: &AgentId) -> Result<Vec<CursorView>> {
        let c = self.read_conn()?;
        let mut s=c.prepare("SELECT chat_id,read_to_message_id FROM agent_chat_cursors WHERE workspace_id=?1 AND agent_id=?2 ORDER BY chat_id")?;
        Ok(s.query_map(params![w.as_str(), a.as_str()], |r| {
            Ok(CursorView {
                chat_id: r.get(0)?,
                read_to_message_id: r.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn cursor(&self, w: &WorkspaceId, a: &AgentId, ch: i64) -> Result<Option<i64>> {
        Ok(self.read_conn()?.query_row("SELECT read_to_message_id FROM agent_chat_cursors WHERE workspace_id=?1 AND agent_id=?2 AND chat_id=?3",params![w.as_str(),a.as_str(),ch],|r|r.get(0)).optional()?)
    }
    pub fn begin_tool_invocation(&self, r: ToolInvocationRequest) -> Result<ToolInvocationResult> {
        let c = self.conn()?;
        if let Some((tool_name, arguments, state, result)) = c
            .query_row(
                "SELECT tool_name,arguments_json,state,result_json FROM tool_invocations WHERE execution_id=?1 AND attempt=?2 AND tool_call_id=?3",
                params![r.execution_id, r.attempt, r.tool_call_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        {
            if tool_name != r.tool_name
                || serde_json::from_str::<Value>(&arguments)? != r.arguments
            {
                return Err(StorageError::Conflict(
                    "tool invocation identity reused with different request".into(),
                ));
            }
            return Ok(ToolInvocationResult {
                created: false,
                state,
                result: result
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            });
        }
        c.execute(
            "INSERT INTO tool_invocations VALUES(?1,?2,?3,?4,?5,'running',NULL,?6,NULL)",
            params![
                r.execution_id,
                r.attempt,
                r.tool_call_id,
                r.tool_name,
                serde_json::to_string(&r.arguments)?,
                r.now
            ],
        )?;
        Ok(ToolInvocationResult {
            created: true,
            state: "running".into(),
            result: None,
        })
    }
    pub fn finish_tool_invocation(
        &self,
        r: &ToolInvocationRequest,
        v: Value,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute("UPDATE tool_invocations SET state='completed',result_json=?4,finished_at=?5 WHERE execution_id=?1 AND attempt=?2 AND tool_call_id=?3 AND state='running'",params![r.execution_id,r.attempt,r.tool_call_id,serde_json::to_string(&v)?,now])?==1)
    }
    pub fn fail_tool_invocation(
        &self,
        request: &ToolInvocationRequest,
        code: &str,
        now: i64,
    ) -> Result<bool> {
        if code.trim().is_empty() {
            return Err(StorageError::Invalid("empty tool failure code".into()));
        }
        let result = serde_json::json!({"error": code});
        Ok(self.conn()?.execute(
            "UPDATE tool_invocations SET state='failed',result_json=?4,finished_at=?5 WHERE execution_id=?1 AND attempt=?2 AND tool_call_id=?3 AND state='running'",
            params![request.execution_id, request.attempt, request.tool_call_id, serde_json::to_string(&result)?, now],
        )? == 1)
    }
    pub fn begin_send(&self, r: SendRequest) -> Result<SendAttemptView> {
        self.begin_send_for_execution(None, r)
    }
    pub fn begin_send_guarded(
        &self,
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        force: bool,
        r: SendRequest,
    ) -> Result<GuardedSendResult> {
        if &r.channel_id != channel_id || r.provider_chat_id != provider_chat_id {
            return Err(StorageError::Invalid("guarded send target mismatch".into()));
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let execution_id: i64 = t.query_row(
            "SELECT execution_id FROM executions WHERE snapshot_id=?1 AND workspace_id=?2 AND agent_id=?3 ORDER BY execution_id DESC LIMIT 1",
            params![snapshot_id, r.workspace_id.as_str(), r.sender_agent_id.as_str()],
            |row| row.get(0),
        )?;
        if let Some(existing) = load_send(&t, &r.request_key)? {
            let owner: (String, String, String, Option<String>, Option<i64>, String) = t.query_row(
                "SELECT ch.workspace_id,ch.channel_id,ch.provider_chat_id,m.sender_id,a.execution_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats ch USING(chat_id) WHERE a.request_key=?1",
                [r.request_key.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )?;
            if owner.0 != r.workspace_id.as_str()
                || owner.1 != channel_id.as_str()
                || owner.2 != provider_chat_id
                || owner.3.as_deref() != Some(r.sender_agent_id.as_str())
                || owner.4 != Some(execution_id)
            {
                return Err(StorageError::Conflict(
                    "guarded send idempotency owner mismatch".into(),
                ));
            }
            if owner.5 != r.content {
                return Err(StorageError::Conflict(
                    "guarded send idempotency key reused with different content".into(),
                ));
            }
            return Ok(GuardedSendResult::Ready(existing));
        }
        let guarded: Option<(i64, i64, String, String)> = t
            .query_row(
                "SELECT sc.chat_id,sc.read_to_message_id,s.workspace_id,s.agent_id FROM unread_snapshot_chats sc JOIN unread_snapshots s USING(snapshot_id) JOIN chats ch ON ch.chat_id=sc.chat_id WHERE sc.snapshot_id=?1 AND ch.channel_id=?2 AND ch.provider_chat_id=?3",
                params![snapshot_id, channel_id.as_str(), provider_chat_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((chat_id, snapshot_message_id, workspace_id, agent_id)) = guarded else {
            return Err(StorageError::Invalid(
                "guarded send snapshot target not found".into(),
            ));
        };
        if workspace_id != r.workspace_id.as_str() || agent_id != r.sender_agent_id.as_str() {
            return Err(StorageError::Invalid(
                "guarded send snapshot owner mismatch".into(),
            ));
        }
        let latest_message_id: i64 = t.query_row(
            "SELECT coalesce(max(message_id),0) FROM messages WHERE chat_id=?1 AND send_status IN ('received','sent') AND NOT(sender_kind='agent' AND coalesce(sender_id,'')=?2)",
            params![chat_id, r.sender_agent_id.as_str()],
            |row| row.get(0),
        )?;
        if !force && latest_message_id > snapshot_message_id {
            let latest_boundary_id: i64 = t.query_row(
                "SELECT coalesce(max(message_id),0) FROM messages WHERE chat_id=?1 AND send_status IN ('received','sent')",
                [chat_id],
                |row| row.get(0),
            )?;
            expand_execution_chat_boundary(
                &t,
                execution_id,
                snapshot_id,
                chat_id,
                latest_boundary_id,
                r.now,
            )?;
            t.commit()?;
            return Ok(GuardedSendResult::NewMessages {
                snapshot_message_id,
                latest_message_id,
            });
        }
        t.execute("INSERT INTO messages(chat_id,direction,sender_kind,sender_id,sender_name,content_markdown,send_status,occurred_at,created_at,updated_at)VALUES(?1,'outbound','agent',?2,(SELECT name FROM agents WHERE workspace_id=?5 AND agent_id=?2),?3,'sending',?4,?4,?4)",params![chat_id,r.sender_agent_id.as_str(),r.content,r.now,r.workspace_id.as_str()])?;
        let message_id = t.last_insert_rowid();
        t.execute("INSERT INTO message_send_attempts(message_id,execution_id,request_key,state,started_at)VALUES(?1,?2,?3,'sending',?4)",params![message_id,execution_id,r.request_key,r.now])?;
        t.commit()?;
        Ok(GuardedSendResult::Ready(SendAttemptView {
            created: true,
            message_id,
            request_key: r.request_key,
            state: "sending".into(),
            provider_message_id: None,
            error_code: None,
        }))
    }
    pub fn begin_execution_send(
        &self,
        execution_id: i64,
        r: SendRequest,
    ) -> Result<SendAttemptView> {
        self.begin_send_for_execution(Some(execution_id), r)
    }
    fn begin_send_for_execution(
        &self,
        execution_id: Option<i64>,
        r: SendRequest,
    ) -> Result<SendAttemptView> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        if let Some(x) = load_send(&t, &r.request_key)? {
            let owner: (Option<i64>, String, String, String, Option<String>, String) = t.query_row(
                "SELECT a.execution_id,ch.workspace_id,ch.channel_id,ch.provider_chat_id,m.sender_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats ch USING(chat_id) WHERE a.request_key=?1",
                [r.request_key.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )?;
            if owner.0 != execution_id
                || owner.1 != r.workspace_id.as_str()
                || owner.2 != r.channel_id.as_str()
                || owner.3 != r.provider_chat_id
                || owner.4.as_deref() != Some(r.sender_agent_id.as_str())
                || owner.5 != r.content
            {
                return Err(StorageError::Conflict(
                    "send idempotency key reused with different request".into(),
                ));
            }
            return Ok(x);
        }
        let ch:i64=t.query_row("SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3 AND (receiving_agent_id='' OR receiving_agent_id=?4) ORDER BY receiving_agent_id DESC LIMIT 1",params![r.workspace_id.as_str(),r.channel_id.as_str(),r.provider_chat_id,r.sender_agent_id.as_str()],|x|x.get(0))?;
        if let Some(execution_id) = execution_id {
            let matches_workspace: i64 = t.query_row(
                "SELECT count(*) FROM executions WHERE execution_id=?1 AND workspace_id=?2",
                params![execution_id, r.workspace_id.as_str()],
                |row| row.get(0),
            )?;
            if matches_workspace == 0 {
                return Err(StorageError::Invalid(
                    "response send execution mismatch".into(),
                ));
            }
        }
        t.execute("INSERT INTO messages(chat_id,direction,sender_kind,sender_id,sender_name,content_markdown,send_status,occurred_at,created_at,updated_at)VALUES(?1,'outbound','agent',?2,(SELECT name FROM agents WHERE workspace_id=?5 AND agent_id=?2),?3,'sending',?4,?4,?4)",params![ch,r.sender_agent_id.as_str(),r.content,r.now,r.workspace_id.as_str()])?;
        let mid = t.last_insert_rowid();
        t.execute("INSERT INTO message_send_attempts(message_id,execution_id,request_key,state,started_at)VALUES(?1,?2,?3,'sending',?4)",params![mid,execution_id,r.request_key,r.now])?;
        t.commit()?;
        Ok(SendAttemptView {
            created: true,
            message_id: mid,
            request_key: r.request_key,
            state: "sending".into(),
            provider_message_id: None,
            error_code: None,
        })
    }
    pub fn finish_send(
        &self,
        request_key: &str,
        outcome: SendOutcome,
        now: i64,
    ) -> Result<FinishSendResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let current = load_send(&t, request_key)?
            .ok_or_else(|| StorageError::Invalid("send not found".into()))?;
        let (next, provider_message_id, error_code) = match &outcome {
            SendOutcome::Sent {
                provider_message_id,
            } if provider_message_id.trim().is_empty() => {
                return Err(StorageError::Invalid("empty provider message id".into()));
            }
            SendOutcome::Sent {
                provider_message_id,
            } => ("sent", Some(provider_message_id.as_str()), None),
            SendOutcome::Unknown => ("unknown", None, None),
            SendOutcome::Failed { code } if code.trim().is_empty() => {
                return Err(StorageError::Invalid("empty send failure code".into()));
            }
            SendOutcome::Failed { code } => ("failed", None, Some(code.as_str())),
        };
        let can_reconcile_unknown = current.state == "unknown" && next != "unknown";
        if current.state != "sending" && !can_reconcile_unknown {
            let same = current.state == next
                && current.provider_message_id.as_deref() == provider_message_id
                && current.error_code.as_deref() == error_code;
            if !same {
                return Err(StorageError::Conflict(format!(
                    "send outcome for {request_key}"
                )));
            }
            return Ok(FinishSendResult {
                changed: false,
                attempt: current,
            });
        }
        if let Some(provider_message_id) = provider_message_id {
            let conflict:Option<i64>=t.query_row("SELECT message_id FROM messages WHERE chat_id=(SELECT chat_id FROM messages WHERE message_id=?1) AND provider_message_id=?2 AND message_id<>?1",params![current.message_id,provider_message_id],|r|r.get(0)).optional()?;
            if conflict.is_some() {
                return Err(StorageError::Conflict(format!(
                    "provider message id {provider_message_id}"
                )));
            }
        }
        t.execute("UPDATE message_send_attempts SET state=?2,provider_message_id=?3,error_code=?4,finished_at=?5 WHERE request_key=?1 AND state IN ('sending','unknown')",params![request_key,next,provider_message_id,error_code,now])?;
        t.execute("UPDATE messages SET send_status=?2,provider_message_id=?3,updated_at=?4 WHERE message_id=?1 AND send_status IN ('sending','unknown')",params![current.message_id,next,provider_message_id,now])?;
        if next == "sent" {
            t.execute(
                "UPDATE chats SET first_message_id=coalesce(first_message_id,?2),last_message_id=CASE WHEN last_message_id IS NULL OR last_message_id<?2 THEN ?2 ELSE last_message_id END,updated_at=?3 WHERE chat_id=(SELECT chat_id FROM messages WHERE message_id=?2)",
                params![current.message_id, current.message_id, now],
            )?;
        }
        let attempt = load_send(&t, request_key)?
            .ok_or_else(|| StorageError::Invalid("send disappeared".into()))?;
        t.commit()?;
        Ok(FinishSendResult {
            changed: true,
            attempt,
        })
    }
    pub fn send_attempt(&self, k: &str) -> Result<SendAttemptView> {
        let connection = self.read_conn()?;
        load_send(&connection, k)?.ok_or_else(|| StorageError::Invalid("send not found".into()))
    }
    pub fn recoverable_sends(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<RecoverableSendView>> {
        let c = self.read_conn()?;
        let mut sql="SELECT a.message_id,a.request_key,a.state,a.provider_message_id,a.error_code,c.workspace_id,w.provider_id,c.channel_id,c.provider_chat_id,m.sender_id,m.content_markdown FROM message_send_attempts a JOIN messages m USING(message_id) JOIN chats c USING(chat_id) JOIN workspaces w USING(workspace_id) WHERE a.state IN('sending','unknown')".to_owned();
        if workspace_id.is_some() {
            sql.push_str(" AND c.workspace_id=?1");
        }
        sql.push_str(" ORDER BY a.send_attempt_id");
        let mut s = c.prepare(&sql)?;
        let mapper = |r: &rusqlite::Row<'_>| -> rusqlite::Result<_> {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, Option<String>>(10)?.unwrap_or_default(),
            ))
        };
        let rows = if let Some(workspace_id) = workspace_id {
            s.query_map([workspace_id.as_str()], mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            s.query_map([], mapper)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut out = Vec::new();
        for (
            message_id,
            request_key,
            state,
            provider_message_id,
            error_code,
            workspace,
            provider,
            channel,
            provider_chat_id,
            sender,
            content,
        ) in rows
        {
            out.push(RecoverableSendView {
                attempt: SendAttemptView {
                    created: false,
                    message_id,
                    request_key,
                    state,
                    provider_message_id,
                    error_code,
                },
                workspace_id: wid(workspace)?,
                provider_id: pid(provider)?,
                channel_id: cid(channel)?,
                provider_chat_id,
                sender_agent_id: aid(sender)?,
                content,
            });
        }
        Ok(out)
    }
}
