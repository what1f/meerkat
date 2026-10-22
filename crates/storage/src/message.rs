use crate::agent::aid;
use crate::error::conv;
use crate::workspace::{ensure_workspace_provider, pid, wid};
use crate::{AttentionRecord, Result, Storage, StorageError};
use lankide_core::{AgentId, ChannelId, MessageAuthorType, ProviderId, ResourceId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

type StoredMessageRow = (
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
);
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResourceMetadata {
    pub workspace_id: WorkspaceId,
    pub resource_id: ResourceId,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub storage_ref: String,
    pub created_at: i64,
}
#[derive(Clone, Debug)]
pub struct IncomingMessage {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub provider_chat_id: String,
    pub chat_name: String,
    pub provider_message_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub occurred_at: i64,
    pub now: i64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomingMessageResult {
    pub chat_id: i64,
    pub message_id: i64,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResourceView {
    pub resource_id: ResourceId,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DispatchReason {
    DirectMessage,
    Mention,
    Follow,
    Schedule,
    Reaction {
        reaction: String,
        sender_name: String,
    },
    Custom {
        name: String,
        object_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct InboxDispatch {
    pub agent_id: AgentId,
    pub available_at: i64,
    pub trigger_reason: DispatchReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageInput {
    pub sender_kind: MessageAuthorType,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub mentions: Vec<AgentId>,
    pub reply_to_provider_message_id: Option<String>,
    pub resources: Vec<ResourceView>,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestEvent {
    pub receiving_agent_id: Option<AgentId>,
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub provider_chat_id: String,
    pub chat_name: String,
    pub provider_event_id: Option<String>,
    pub provider_message_id: String,
    pub message: MessageInput,
    pub dispatch: Vec<InboxDispatch>,
    pub now: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IngestResult {
    pub chat_id: i64,
    pub message_id: i64,
    pub duplicate: bool,
    pub inbox_item_ids: Vec<i64>,
    pub new_inbox_items: Vec<(AgentId, i64)>,
}

pub use lankide_core::ReactionAction;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReactionOrigin {
    Provider,
    AutoReaction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomingReaction {
    pub receiving_agent_id: Option<AgentId>,
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub target_provider_message_id: String,
    pub sender_kind: MessageAuthorType,
    pub sender_id: String,
    pub sender_name: String,
    pub reaction: String,
    pub action: ReactionAction,
    pub origin: ReactionOrigin,
    pub provider_event_id: String,
    pub dispatch: Vec<InboxDispatch>,
    pub occurred_at: i64,
    pub now: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionResult {
    pub reaction_id: Option<i64>,
    pub target_message_id: Option<i64>,
    pub duplicate: bool,
    pub ignored_unknown_target: bool,
    pub inbox_item_ids: Vec<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatRefView {
    pub chat_id: i64,
    pub workspace_id: WorkspaceId,
    pub provider_id: ProviderId,
    pub channel_id: ChannelId,
    pub channel_name: String,
    pub provider_chat_id: String,
    pub chat_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageView {
    pub message_id: i64,
    pub provider_message_id: Option<String>,
    pub sender_kind: MessageAuthorType,
    pub sender_id: String,
    pub sender_name: String,
    pub content: String,
    pub mentions: Vec<AgentId>,
    pub reply_to_provider_message_id: Option<String>,
    /// One-level quote, resolved only within the same stored conversation.
    pub reply_to: Option<Box<MessageView>>,
    pub resources: Vec<ResourceView>,
    pub occurred_at: i64,
}

/// The transport-facing identity of one Runtime-owned Message.
///
/// Provider implementations need this small projection to translate an
/// internal reply target into the external platform's native message ID. It
/// intentionally excludes message content and storage implementation details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderMessageView {
    pub message_id: i64,
    pub provider_message_id: Option<String>,
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecallTarget {
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub message_id: i64,
    pub provider_message_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenSnapshotChatView {
    pub chat: ChatRefView,
    pub read_from_message_id: Option<i64>,
    pub read_to_message_id: i64,
    pub unread_count: usize,
    pub recent_messages: Vec<MessageView>,
    pub mentioned_message: Option<MessageView>,
    pub trigger_messages: Vec<MessageView>,
    pub trigger_reasons: Vec<DispatchReason>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotView {
    pub snapshot_id: i64,
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub created_at: i64,
    pub chats: Vec<FrozenSnapshotChatView>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InboxState {
    Pending,
    Claimed,
    Delivered,
    Consumed,
    Failed,
}
impl InboxState {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "delivered" => Ok(Self::Delivered),
            "consumed" => Ok(Self::Consumed),
            "failed" => Ok(Self::Failed),
            _ => Err(StorageError::Invalid(format!("inbox state {s}"))),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxItemView {
    pub inbox_item_id: i64,
    pub chat_id: i64,
    pub source_id: i64,
    pub state: InboxState,
    pub claimed_execution_id: Option<i64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotChatView {
    pub chat_id: i64,
    pub read_to_message_id: i64,
    pub unread_count: usize,
}
fn receive(t: &Transaction<'_>, m: &IncomingMessage) -> Result<IncomingMessageResult> {
    t.execute("INSERT INTO channels VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,channel_id)DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![m.workspace_id.as_str(),m.channel_id.as_str(),m.channel_name,m.now])?;
    t.execute("INSERT INTO chats(workspace_id,channel_id,provider_chat_id,name,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?5)ON CONFLICT(workspace_id,channel_id,provider_chat_id,receiving_agent_id)DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![m.workspace_id.as_str(),m.channel_id.as_str(),m.provider_chat_id,m.chat_name,m.now])?;
    let ch: i64 = t.query_row(
        "SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3 AND receiving_agent_id=''",
        params![
            m.workspace_id.as_str(),
            m.channel_id.as_str(),
            m.provider_chat_id
        ],
        |r| r.get(0),
    )?;
    if let Some(id) = t
        .query_row(
            "SELECT message_id FROM messages WHERE chat_id=?1 AND provider_message_id=?2",
            params![ch, m.provider_message_id],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(IncomingMessageResult {
            chat_id: ch,
            message_id: id,
            duplicate: true,
        });
    }
    t.execute("INSERT INTO messages(chat_id,provider_message_id,direction,sender_kind,sender_id,sender_name,content_markdown,send_status,occurred_at,created_at,updated_at)VALUES(?1,?2,'inbound','human',?3,?4,?5,'received',?6,?7,?7)",params![ch,m.provider_message_id,m.sender_id,m.sender_name,m.content,m.occurred_at,m.now])?;
    let id = t.last_insert_rowid();
    t.execute("UPDATE chats SET first_message_id=coalesce(first_message_id,?2),last_message_id=?2,updated_at=?3 WHERE chat_id=?1",params![ch,id,m.now])?;
    t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,available_at,state,created_at,updated_at)SELECT workspace_id,agent_id,?2,?3,?4,'pending',?4,?4 FROM agents WHERE workspace_id=?1",params![m.workspace_id.as_str(),ch,id,m.now])?;
    Ok(IncomingMessageResult {
        chat_id: ch,
        message_id: id,
        duplicate: false,
    })
}
fn load_resource_metadata(
    c: &Connection,
    w: &WorkspaceId,
    id: ResourceId,
) -> Result<Option<ResourceMetadata>> {
    let row:Option<(String,String,i64,String,String,i64)>=c.query_row("SELECT file_name,media_type,size_bytes,sha256,storage_ref,created_at FROM resource_metadata WHERE workspace_id=?1 AND resource_id=?2",params![w.as_str(),id.get()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    Ok(row.map(
        |(file_name, media_type, size_bytes, sha256, storage_ref, created_at)| ResourceMetadata {
            workspace_id: w.clone(),
            resource_id: id,
            file_name,
            media_type,
            size_bytes: size_bytes as u64,
            sha256,
            storage_ref,
            created_at,
        },
    ))
}
fn validate_dispatch(dispatch: &[InboxDispatch]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for item in dispatch {
        if !seen.insert(item.agent_id.as_str()) {
            return Err(StorageError::Invalid(format!(
                "duplicate dispatch agent {}",
                item.agent_id
            )));
        }
    }
    Ok(())
}
fn sender_kind(kind: MessageAuthorType) -> &'static str {
    match kind {
        MessageAuthorType::Human => "human",
        MessageAuthorType::Agent => "agent",
        MessageAuthorType::Bot => "bot",
        MessageAuthorType::System => "system",
    }
}
fn parse_sender_kind(value: &str) -> Result<MessageAuthorType> {
    match value {
        "human" => Ok(MessageAuthorType::Human),
        "agent" => Ok(MessageAuthorType::Agent),
        "bot" => Ok(MessageAuthorType::Bot),
        "system" => Ok(MessageAuthorType::System),
        _ => Err(StorageError::Invalid(format!("sender kind {value}"))),
    }
}
fn reaction_action(value: ReactionAction) -> &'static str {
    match value {
        ReactionAction::Add => "add",
        ReactionAction::Remove => "remove",
    }
}
fn reaction_origin(value: ReactionOrigin) -> &'static str {
    match value {
        ReactionOrigin::Provider => "provider",
        ReactionOrigin::AutoReaction => "auto_reaction",
    }
}
fn load_message(c: &Connection, message_id: i64) -> Result<MessageView> {
    load_message_with_reply(c, message_id, true)
}

fn load_message_with_reply(
    c: &Connection,
    message_id: i64,
    include_reply: bool,
) -> Result<MessageView> {
    let tuple:StoredMessageRow=c.query_row("SELECT provider_message_id,sender_kind,sender_id,sender_name,content_markdown,mentions_json,reply_to_provider_message_id,resources_json,occurred_at FROM messages WHERE message_id=?1",[message_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))?;
    let reply_id: Option<i64> = if include_reply && tuple.6.is_some() {
        c.query_row(
            "SELECT coalesce((SELECT p.message_id FROM messages p WHERE p.chat_id=m.chat_id AND p.provider_message_id=m.reply_to_provider_message_id AND p.message_id<m.message_id AND p.send_status IN ('received','sent')), (SELECT p.message_id FROM message_provider_parts parts JOIN messages p ON p.message_id=parts.message_id WHERE parts.chat_id=m.chat_id AND parts.provider_message_id=m.reply_to_provider_message_id AND p.chat_id=m.chat_id AND p.message_id<m.message_id AND p.send_status IN ('received','sent'))) FROM messages m WHERE m.message_id=?1",
            [message_id], |row| row.get(0),
        )?
    } else {
        None
    };
    let reply_to = reply_id
        .map(|id| load_message_with_reply(c, id, false).map(Box::new))
        .transpose()?;
    Ok(MessageView {
        message_id,
        provider_message_id: tuple.0,
        sender_kind: parse_sender_kind(&tuple.1)?,
        sender_id: tuple.2.unwrap_or_default(),
        sender_name: tuple.3.unwrap_or_default(),
        content: tuple.4.unwrap_or_default(),
        mentions: serde_json::from_str(&tuple.5)?,
        reply_to_provider_message_id: tuple.6,
        reply_to,
        resources: serde_json::from_str(&tuple.7)?,
        occurred_at: tuple.8,
    })
}
fn load_chat_ref(c: &Connection, chat_id: i64) -> Result<ChatRefView> {
    let (workspace,provider,channel,channel_name,provider_chat_id,chat_name): (String,String,String,String,String,String) = c.query_row("SELECT c.workspace_id,w.provider_id,c.channel_id,ch.name,c.provider_chat_id,coalesce(c.name,'') FROM chats c JOIN workspaces w USING(workspace_id) JOIN channels ch ON ch.workspace_id=c.workspace_id AND ch.channel_id=c.channel_id WHERE c.chat_id=?1",[chat_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    Ok(ChatRefView {
        chat_id,
        workspace_id: wid(workspace)?,
        provider_id: pid(provider)?,
        channel_id: cid(channel)?,
        channel_name,
        provider_chat_id,
        chat_name,
    })
}

pub(super) fn load_snapshot(c: &Connection, snapshot_id: i64) -> Result<SnapshotView> {
    let (workspace, agent, created_at): (String, String, i64) = c.query_row(
        "SELECT workspace_id,agent_id,created_at FROM unread_snapshots WHERE snapshot_id=?1",
        [snapshot_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let workspace_id = wid(workspace)?;
    let provider_id: ProviderId = pid(c.query_row(
        "SELECT provider_id FROM workspaces WHERE workspace_id=?1",
        [workspace_id.as_str()],
        |r| r.get(0),
    )?)?;
    let mut statement=c.prepare("SELECT sc.chat_id,ch.channel_id,cn.name,ch.provider_chat_id,ch.name,sc.read_from_message_id,sc.read_to_message_id,sc.unread_count FROM unread_snapshot_chats sc JOIN chats ch USING(chat_id) JOIN channels cn ON cn.workspace_id=ch.workspace_id AND cn.channel_id=ch.channel_id WHERE sc.snapshot_id=?1 ORDER BY sc.chat_id")?;
    let rows = statement
        .query_map([snapshot_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    let mut chats = Vec::new();
    for (
        chat_id,
        channel,
        channel_name,
        provider_chat_id,
        chat_name,
        read_from_message_id,
        read_to_message_id,
        unread_count,
    ) in rows
    {
        let mut recent_stmt=c.prepare("SELECT message_id FROM unread_snapshot_messages WHERE snapshot_id=?1 AND chat_id=?2 ORDER BY position")?;
        let recent_ids = recent_stmt
            .query_map(params![snapshot_id, chat_id], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(recent_stmt);
        let mut recent_messages = Vec::new();
        for id in recent_ids {
            recent_messages.push(load_message(c, id)?);
        }
        let mut trigger_stmt=c.prepare("SELECT message_id,reason_json FROM unread_snapshot_triggers WHERE snapshot_id=?1 AND chat_id=?2 ORDER BY position")?;
        let trigger_rows = trigger_stmt
            .query_map(params![snapshot_id, chat_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(trigger_stmt);
        let mut trigger_messages = Vec::new();
        let mut trigger_reasons = Vec::new();
        let mut mentioned_message = None;
        for (id, reason) in trigger_rows {
            let message = load_message(c, id)?;
            let reason = serde_json::from_str(&reason)?;
            if reason == DispatchReason::Mention {
                mentioned_message = Some(message.clone());
            }
            trigger_messages.push(message);
            trigger_reasons.push(reason);
        }
        chats.push(FrozenSnapshotChatView {
            chat: ChatRefView {
                chat_id,
                workspace_id: workspace_id.clone(),
                provider_id: provider_id.clone(),
                channel_id: cid(channel)?,
                channel_name,
                provider_chat_id,
                chat_name,
            },
            read_from_message_id,
            read_to_message_id,
            unread_count: unread_count as usize,
            recent_messages,
            mentioned_message,
            trigger_messages,
            trigger_reasons,
        });
    }
    Ok(SnapshotView {
        snapshot_id,
        workspace_id,
        agent_id: aid(agent)?,
        created_at,
        chats,
    })
}
pub(super) fn cid(s: String) -> rusqlite::Result<ChannelId> {
    ChannelId::new(s).map_err(conv)
}

impl Storage {
    pub fn allocate_resource_id(&self, workspace_id: &WorkspaceId, now: i64) -> Result<ResourceId> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO workspace_resource_counters(workspace_id,next_id,updated_at) VALUES(?1,1,?2)",
            params![workspace_id.as_str(), now],
        )?;
        let next_id: i64 = transaction.query_row(
            "SELECT next_id FROM workspace_resource_counters WHERE workspace_id=?1",
            [workspace_id.as_str()],
            |row| row.get(0),
        )?;
        transaction.execute(
            "UPDATE workspace_resource_counters SET next_id=?2,updated_at=?3 WHERE workspace_id=?1 AND next_id=?4",
            params![workspace_id.as_str(), next_id + 1, now, next_id],
        )?;
        transaction.commit()?;
        ResourceId::new(next_id).map_err(|error| StorageError::Invalid(error.to_string()))
    }
    pub fn upsert_resource_metadata(&self, metadata: ResourceMetadata) -> Result<ResourceMetadata> {
        self.conn()?.execute("INSERT INTO resource_metadata(workspace_id,resource_id,file_name,media_type,size_bytes,sha256,storage_ref,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(workspace_id,resource_id) DO UPDATE SET file_name=excluded.file_name,media_type=excluded.media_type,size_bytes=excluded.size_bytes,sha256=excluded.sha256,storage_ref=excluded.storage_ref",params![metadata.workspace_id.as_str(),metadata.resource_id.get(),metadata.file_name,metadata.media_type,metadata.size_bytes as i64,metadata.sha256,metadata.storage_ref,metadata.created_at])?;
        let c = self.conn()?;
        load_resource_metadata(&c, &metadata.workspace_id, metadata.resource_id)?
            .ok_or_else(|| StorageError::Invalid("resource metadata disappeared".into()))
    }
    pub fn resource_metadata(
        &self,
        w: &WorkspaceId,
        id: ResourceId,
    ) -> Result<Option<ResourceMetadata>> {
        let c = self.read_conn()?;
        load_resource_metadata(&c, w, id)
    }
    pub fn receive_message(&self, m: IncomingMessage) -> Result<IncomingMessageResult> {
        let mut c = self.conn()?;
        let t = c.transaction()?;
        let o = receive(&t, &m)?;
        t.commit()?;
        Ok(o)
    }
    pub fn ingest_event(&self, event: IngestEvent) -> Result<IngestResult> {
        if let Some(receiver) = &event.receiving_agent_id
            && event
                .dispatch
                .iter()
                .any(|dispatch| &dispatch.agent_id != receiver)
        {
            return Err(StorageError::Invalid(
                "direct-message dispatch receiver mismatch".into(),
            ));
        }
        let mut c = self.conn()?;
        let t = c.transaction()?;
        ensure_workspace_provider(&t, &event.workspace_id, &event.provider_id)?;
        t.execute("INSERT INTO channels VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,channel_id) DO UPDATE SET name=excluded.name,updated_at=excluded.updated_at",params![event.workspace_id.as_str(),event.channel_id.as_str(),event.channel_name,event.now])?;
        t.execute("INSERT INTO chats(workspace_id,channel_id,provider_chat_id,name,created_at,updated_at,receiving_agent_id)VALUES(?1,?2,?3,?4,?5,?5,?6) ON CONFLICT(workspace_id,channel_id,provider_chat_id,receiving_agent_id) DO UPDATE SET name=CASE WHEN excluded.name=excluded.provider_chat_id THEN chats.name ELSE excluded.name END,updated_at=excluded.updated_at",params![event.workspace_id.as_str(),event.channel_id.as_str(),event.provider_chat_id,event.chat_name,event.now,event.receiving_agent_id.as_ref().map(AgentId::as_str).unwrap_or("")])?;
        let chat_id: i64 = t.query_row("SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3 AND receiving_agent_id=?4",params![event.workspace_id.as_str(),event.channel_id.as_str(),event.provider_chat_id,event.receiving_agent_id.as_ref().map(AgentId::as_str).unwrap_or("")],|r|r.get(0))?;
        if event.receiving_agent_id.is_some()
            && event.message.sender_kind == MessageAuthorType::Human
        {
            t.execute(
                "UPDATE chats SET recipient_person_id=?2 WHERE chat_id=?1",
                params![chat_id, event.message.sender_id],
            )?;
        }
        if let Some((message_id, direction, sender_kind, sender_id)) = t
            .query_row(
                "SELECT message_id,direction,sender_kind,sender_id FROM messages WHERE chat_id=?1 AND provider_message_id=?2",
                params![chat_id, event.provider_message_id],
                |row| Ok((row.get(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
            )
            .optional()?
        {
            validate_dispatch(&event.dispatch)?;
            let existing_mentions: String = t.query_row(
                "SELECT mentions_json FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get(0),
            )?;
            let mut merged_mentions =
                serde_json::from_str::<Vec<AgentId>>(&existing_mentions).unwrap_or_default();
            for mention in &event.message.mentions {
                if !merged_mentions.contains(mention) {
                    merged_mentions.push(mention.clone());
                }
            }
            merged_mentions.sort_by(|left, right| left.as_str().cmp(right.as_str()));
            t.execute(
                "UPDATE messages SET mentions_json=?2,provider_event_id=coalesce(provider_event_id,?3),updated_at=?4 WHERE message_id=?1",
                params![
                    message_id,
                    serde_json::to_string(&merged_mentions)?,
                    event.provider_event_id,
                    event.now
                ],
            )?;
            let mut inbox_item_ids = Vec::new();
            let mut new_inbox_items = Vec::new();
            for dispatch in &event.dispatch {
                if direction == "outbound"
                    && sender_kind == "agent"
                    && sender_id.as_deref() == Some(dispatch.agent_id.as_str())
                {
                    continue;
                }
                let changed = t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'message',?5,?6,'pending',?7,?7)",params![event.workspace_id.as_str(),dispatch.agent_id.as_str(),chat_id,message_id,serde_json::to_string(&dispatch.trigger_reason)?,dispatch.available_at,event.now])?;
                if changed == 1 {
                    let inbox_item_id = t.last_insert_rowid();
                    inbox_item_ids.push(inbox_item_id);
                    new_inbox_items.push((dispatch.agent_id.clone(), inbox_item_id));
                }
            }
            t.commit()?;
            return Ok(IngestResult {
                chat_id,
                message_id,
                duplicate: true,
                inbox_item_ids,
                new_inbox_items,
            });
        }
        validate_dispatch(&event.dispatch)?;
        t.execute("INSERT INTO messages(chat_id,provider_message_id,provider_event_id,direction,sender_kind,sender_id,sender_name,content_markdown,mentions_json,reply_to_provider_message_id,resources_json,send_status,occurred_at,created_at,updated_at) VALUES(?1,?2,?3,'inbound',?4,?5,?6,?7,?8,?9,?10,'received',?11,?12,?12)",params![chat_id,event.provider_message_id,event.provider_event_id,sender_kind(event.message.sender_kind),event.message.sender_id,event.message.sender_name,event.message.content,serde_json::to_string(&event.message.mentions)?,event.message.reply_to_provider_message_id,serde_json::to_string(&event.message.resources)?,event.message.occurred_at,event.now])?;
        let message_id = t.last_insert_rowid();
        t.execute("UPDATE chats SET first_message_id=coalesce(first_message_id,?2),last_message_id=?2,updated_at=?3 WHERE chat_id=?1",params![chat_id,message_id,event.now])?;
        let mut inbox_item_ids = Vec::new();
        let mut new_inbox_items = Vec::new();
        for dispatch in &event.dispatch {
            let changed = t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'message',?5,?6,'pending',?7,?7)",params![event.workspace_id.as_str(),dispatch.agent_id.as_str(),chat_id,message_id,serde_json::to_string(&dispatch.trigger_reason)?,dispatch.available_at,event.now])?;
            if changed == 1 {
                let inbox_item_id = t.last_insert_rowid();
                inbox_item_ids.push(inbox_item_id);
                new_inbox_items.push((dispatch.agent_id.clone(), inbox_item_id));
            }
        }
        t.commit()?;
        Ok(IngestResult {
            chat_id,
            message_id,
            duplicate: false,
            inbox_item_ids,
            new_inbox_items,
        })
    }

    pub fn record_reaction(&self, reaction: IncomingReaction) -> Result<ReactionResult> {
        if let Some(receiver) = &reaction.receiving_agent_id
            && reaction
                .dispatch
                .iter()
                .any(|dispatch| &dispatch.agent_id != receiver)
        {
            return Err(StorageError::Invalid(
                "direct-reaction dispatch receiver mismatch".into(),
            ));
        }
        let provider_event_id = if let Some(receiver) = &reaction.receiving_agent_id {
            serde_json::json!([receiver.as_str(), reaction.provider_event_id]).to_string()
        } else {
            reaction.provider_event_id.clone()
        };
        let mut c = self.conn()?;
        let t = c.transaction()?;
        ensure_workspace_provider(&t, &reaction.workspace_id, &reaction.provider_id)?;
        if let Some((reaction_id, target_message_id)) = t.query_row("SELECT reaction_id,target_message_id FROM reactions WHERE workspace_id=?1 AND provider_event_id=?2",params![reaction.workspace_id.as_str(),provider_event_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()? {
            return Ok(ReactionResult { reaction_id: Some(reaction_id), target_message_id: Some(target_message_id), duplicate: true, ignored_unknown_target: false, inbox_item_ids: Vec::new() });
        }
        let target: Option<(i64, i64)>=t.query_row("SELECT c.chat_id,m.message_id FROM chats c JOIN messages m ON m.chat_id=c.chat_id WHERE c.workspace_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND (m.provider_message_id=?4 OR EXISTS(SELECT 1 FROM message_provider_parts p WHERE p.message_id=m.message_id AND p.provider_message_id=?4)) AND c.receiving_agent_id=?5",params![reaction.workspace_id.as_str(),reaction.channel_id.as_str(),reaction.provider_chat_id,reaction.target_provider_message_id,reaction.receiving_agent_id.as_ref().map(AgentId::as_str).unwrap_or("")],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((chat_id, target_message_id)) = target else {
            return Ok(ReactionResult {
                reaction_id: None,
                target_message_id: None,
                duplicate: false,
                ignored_unknown_target: true,
                inbox_item_ids: Vec::new(),
            });
        };
        let dispatches: &[InboxDispatch] = if reaction.origin == ReactionOrigin::AutoReaction {
            &[]
        } else {
            validate_dispatch(&reaction.dispatch)?;
            &reaction.dispatch
        };
        t.execute("INSERT INTO reactions(workspace_id,chat_id,target_message_id,provider_event_id,sender_kind,sender_id,sender_name,reaction,action,origin,occurred_at,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![reaction.workspace_id.as_str(),chat_id,target_message_id,provider_event_id,sender_kind(reaction.sender_kind),reaction.sender_id,reaction.sender_name,reaction.reaction,reaction_action(reaction.action),reaction_origin(reaction.origin),reaction.occurred_at,reaction.now])?;
        let reaction_id = t.last_insert_rowid();
        let source_id = -reaction_id;
        let mut inbox_item_ids = Vec::new();
        for dispatch in dispatches {
            let changed=t.execute("INSERT OR IGNORE INTO agent_inbox_items(workspace_id,agent_id,chat_id,source_id,source_kind,trigger_reason_json,available_at,state,created_at,updated_at)VALUES(?1,?2,?3,?4,'reaction',?5,?6,'pending',?7,?7)",params![reaction.workspace_id.as_str(),dispatch.agent_id.as_str(),chat_id,source_id,serde_json::to_string(&dispatch.trigger_reason)?,dispatch.available_at,reaction.now])?;
            if changed == 1 {
                inbox_item_ids.push(t.last_insert_rowid());
            }
        }
        t.commit()?;
        Ok(ReactionResult {
            reaction_id: Some(reaction_id),
            target_message_id: Some(target_message_id),
            duplicate: false,
            ignored_unknown_target: false,
            inbox_item_ids,
        })
    }
    pub fn outbound_message_id(
        &self,
        workspace: &WorkspaceId,
        channel: &ChannelId,
        chat: &str,
        native_id: &str,
    ) -> Result<Option<i64>> {
        Ok(self.read_conn()?.query_row(
            "SELECT m.message_id FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND m.direction='outbound' AND (m.provider_message_id=?4 OR EXISTS(SELECT 1 FROM message_provider_parts p WHERE p.message_id=m.message_id AND p.provider_message_id=?4)) LIMIT 1",
            params![workspace.as_str(),channel.as_str(),chat,native_id],|row|row.get(0),
        ).optional()?)
    }
    pub fn find_message(
        &self,
        w: &WorkspaceId,
        ch: &ChannelId,
        pc: &str,
        pm: &str,
    ) -> Result<Option<i64>> {
        self.find_message_for_receiver(w, ch, pc, pm, None)
    }
    /// Resolve a native message identity within one receiving account's chat.
    pub fn find_message_for_receiver(
        &self,
        w: &WorkspaceId,
        ch: &ChannelId,
        pc: &str,
        pm: &str,
        receiver: Option<&AgentId>,
    ) -> Result<Option<i64>> {
        Ok(self.read_conn()?.query_row("SELECT m.message_id FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND (m.provider_message_id=?4 OR EXISTS(SELECT 1 FROM message_provider_parts p WHERE p.message_id=m.message_id AND p.provider_message_id=?4)) AND c.receiving_agent_id=?5",params![w.as_str(),ch.as_str(),pc,pm,receiver.map(AgentId::as_str).unwrap_or("")],|r|r.get(0)).optional()?)
    }
    /// Resolve the author of a reaction target within its receiving account.
    pub fn reaction_target_author(
        &self,
        workspace: &WorkspaceId,
        channel: &ChannelId,
        chat: &str,
        target: &str,
        receiver: Option<&AgentId>,
    ) -> Result<Option<(MessageAuthorType, String)>> {
        let row: Option<(String, String)> = self.read_conn()?.query_row(
            "SELECT m.sender_kind,COALESCE(m.sender_id,'') FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND (m.provider_message_id=?4 OR EXISTS(SELECT 1 FROM message_provider_parts p WHERE p.message_id=m.message_id AND p.provider_message_id=?4)) AND c.receiving_agent_id=?5",
            params![workspace.as_str(), channel.as_str(), chat, target, receiver.map(AgentId::as_str).unwrap_or("")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        row.map(|(kind, id)| Ok((parse_sender_kind(&kind)?, id)))
            .transpose()
    }
    pub fn message_count(&self, w: &WorkspaceId) -> Result<usize> {
        let n: i64 = self.read_conn()?.query_row(
            "SELECT count(*) FROM messages m JOIN chats c USING(chat_id) WHERE c.workspace_id=?1",
            [w.as_str()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
    pub fn provider_message(
        &self,
        workspace_id: &WorkspaceId,
        message_id: i64,
    ) -> Result<Option<ProviderMessageView>> {
        let row: Option<(Option<String>, String, String)> = self.read_conn()?.query_row(
            "SELECT m.provider_message_id,ch.channel_id,ch.provider_chat_id FROM messages m JOIN chats ch USING(chat_id) WHERE m.message_id=?1 AND ch.workspace_id=?2",
            params![message_id, workspace_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        row.map(|(provider_message_id, channel_id, provider_chat_id)| {
            Ok(ProviderMessageView {
                message_id,
                provider_message_id,
                channel_id: cid(channel_id)?,
                provider_chat_id,
            })
        })
        .transpose()
    }
    pub fn chat_attention(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
    ) -> Result<Option<AttentionRecord>> {
        let value: Option<String> = self
            .read_conn()?
            .query_row(
                "SELECT a.attention_json FROM agent_chat_attention a JOIN chats c USING(chat_id) WHERE a.workspace_id=?1 AND a.agent_id=?2 AND c.channel_id=?3 AND c.provider_chat_id=?4 AND (c.receiving_agent_id='' OR c.receiving_agent_id=?2)",
                params![workspace_id.as_str(), agent_id.as_str(), channel_id.as_str(), provider_chat_id],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(StorageError::from))
            .transpose()
    }
    pub fn set_chat_attention(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        attention: &AttentionRecord,
        now: i64,
    ) -> Result<()> {
        attention.validate()?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let chat_id: Option<i64> = transaction
            .query_row(
                "SELECT chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?2 AND provider_chat_id=?3 AND (receiving_agent_id='' OR receiving_agent_id=?4) ORDER BY receiving_agent_id DESC LIMIT 1",
                params![workspace_id.as_str(), channel_id.as_str(), provider_chat_id, agent_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(chat_id) = chat_id else {
            return Err(StorageError::Invalid("chat not found".into()));
        };
        transaction.execute(
            "INSERT INTO agent_chat_attention(workspace_id,agent_id,chat_id,attention_json,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,agent_id,chat_id) DO UPDATE SET attention_json=excluded.attention_json,updated_at=excluded.updated_at",
            params![workspace_id.as_str(), agent_id.as_str(), chat_id, serde_json::to_string(attention)?, now],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn recall_target(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        message_id: i64,
    ) -> Result<Option<RecallTarget>> {
        let row: Option<(String, String, Option<String>)> = self.read_conn()?.query_row(
            "SELECT ch.channel_id,ch.provider_chat_id,m.provider_message_id FROM messages m JOIN chats ch USING(chat_id) WHERE m.message_id=?1 AND ch.workspace_id=?2 AND m.direction='outbound' AND m.sender_kind='agent' AND m.sender_id=?3",
            params![message_id, workspace_id.as_str(), agent_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        row.map(|(channel_id, provider_chat_id, provider_message_id)| {
            Ok(RecallTarget {
                channel_id: cid(channel_id)?,
                provider_chat_id,
                message_id,
                provider_message_id,
            })
        })
        .transpose()
    }
    pub fn update_outbound_message_content(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        message_id: i64,
        content: &str,
        now: i64,
    ) -> Result<bool> {
        let changed = self.conn()?.execute(
            "UPDATE messages SET content_markdown=?4,updated_at=?5 WHERE message_id=?1 AND chat_id IN (SELECT chat_id FROM chats WHERE workspace_id=?2) AND direction='outbound' AND sender_kind='agent' AND sender_id=?3 AND send_status='sent'",
            params![message_id, workspace_id.as_str(), agent_id.as_str(), content, now],
        )?;
        Ok(changed == 1)
    }
    /// Record which Agent account received a group conversation, independently
    /// of its attention policy or whether the message wakes it.
    pub fn record_chat_receiver(
        &self,
        workspace: &WorkspaceId,
        agent: &AgentId,
        channel: &ChannelId,
        chat: &str,
    ) -> Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO chat_receivers(workspace_id,agent_id,chat_id) SELECT workspace_id,?2,chat_id FROM chats WHERE workspace_id=?1 AND channel_id=?3 AND provider_chat_id=?4 AND receiving_agent_id=''",
            params![workspace.as_str(), agent.as_str(), channel.as_str(), chat],
        )?;
        Ok(())
    }
    pub fn agent_can_access_chat(
        &self,
        workspace: &WorkspaceId,
        agent: &AgentId,
        channel: &ChannelId,
        chat: &str,
    ) -> Result<bool> {
        Ok(self.read_conn()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM chats c WHERE c.workspace_id=?1 AND c.channel_id=?3 AND c.provider_chat_id=?4 AND (c.receiving_agent_id=?2 OR EXISTS(SELECT 1 FROM chat_receivers r WHERE r.chat_id=c.chat_id AND r.workspace_id=?1 AND r.agent_id=?2)))",
            params![workspace.as_str(), agent.as_str(), channel.as_str(), chat],
            |row| row.get(0),
        )?)
    }
    /// Observation is separate from the consumption cursor: failed runs can retry.
    pub fn record_chat_observation(
        &self,
        workspace: &WorkspaceId,
        agent: &AgentId,
        message_id: i64,
        now: i64,
    ) -> Result<bool> {
        Ok(self.conn()?.execute("INSERT INTO agent_chat_observations(workspace_id,agent_id,chat_id,message_id,observed_at) SELECT ?1,?2,m.chat_id,m.message_id,?4 FROM messages m JOIN chats c USING(chat_id) WHERE m.message_id=?3 AND c.workspace_id=?1 ON CONFLICT(workspace_id,agent_id,chat_id) DO UPDATE SET message_id=excluded.message_id,observed_at=excluded.observed_at WHERE excluded.message_id>agent_chat_observations.message_id",params![workspace.as_str(),agent.as_str(),message_id,now])? == 1)
    }
    pub fn observed_chats(
        &self,
        workspace: &WorkspaceId,
        agent: &AgentId,
    ) -> Result<Vec<(ChatRefView, MessageView)>> {
        let connection = self.read_conn()?;
        let mut statement = connection.prepare("SELECT o.chat_id,o.message_id FROM agent_chat_observations o WHERE o.workspace_id=?1 AND o.agent_id=?2 ORDER BY o.observed_at DESC LIMIT 30")?;
        let ids = statement
            .query_map(params![workspace.as_str(), agent.as_str()], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|(chat_id, message_id)| {
                Ok((
                    load_chat_ref(&connection, chat_id)?,
                    load_message(&connection, message_id)?,
                ))
            })
            .collect()
    }
    pub fn snapshot_chat_messages(
        &self,
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        before_message_id: Option<i64>,
        limit: usize,
        now: i64,
    ) -> Result<Vec<MessageView>> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction()?;
        let boundary: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT sc.chat_id,sc.read_to_message_id FROM unread_snapshot_chats sc JOIN chats ch USING(chat_id) WHERE sc.snapshot_id=?1 AND ch.channel_id=?2 AND ch.provider_chat_id=?3",
                params![snapshot_id, channel_id.as_str(), provider_chat_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (chat_id, read_to_message_id) = match boundary {
            Some(boundary) => boundary,
            None => transaction.query_row(
                "SELECT c.chat_id,coalesce(max(m.message_id),0) FROM unread_snapshots s JOIN chats c ON c.workspace_id=s.workspace_id JOIN messages m ON m.chat_id=c.chat_id WHERE s.snapshot_id=?1 AND c.channel_id=?2 AND c.provider_chat_id=?3 AND (c.receiving_agent_id=s.agent_id OR EXISTS(SELECT 1 FROM chat_receivers r WHERE r.chat_id=c.chat_id AND r.workspace_id=s.workspace_id AND r.agent_id=s.agent_id)) AND m.send_status IN ('received','sent') GROUP BY c.chat_id ORDER BY c.receiving_agent_id DESC LIMIT 1",
                params![snapshot_id, channel_id.as_str(), provider_chat_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?.ok_or_else(|| StorageError::Invalid("conversation is not available to this Agent".into()))?,
        };
        let upper = before_message_id
            .map(|message_id| message_id.saturating_sub(1))
            .unwrap_or(read_to_message_id)
            .min(read_to_message_id);
        let limit = limit.clamp(1, 100) as i64;
        let mut statement = transaction.prepare(
            "SELECT message_id FROM messages WHERE chat_id=?1 AND message_id<=?2 AND send_status IN ('received','sent') ORDER BY message_id DESC LIMIT ?3",
        )?;
        let mut ids = statement
            .query_map(params![chat_id, upper, limit], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        ids.reverse();
        let messages = ids
            .iter()
            .map(|id| load_message(&transaction, *id))
            .collect::<Result<Vec<_>>>()?;
        if boundary.is_none()
            && let Some(latest_read) = ids.last()
        {
            transaction.execute(
                "INSERT INTO unread_snapshot_chats(snapshot_id,chat_id,read_from_message_id,read_to_message_id,unread_count,trigger_summary) SELECT snapshot_id,?2,(SELECT read_to_message_id FROM agent_chat_cursors WHERE workspace_id=s.workspace_id AND agent_id=s.agent_id AND chat_id=?2),?3,0,'' FROM unread_snapshots s WHERE snapshot_id=?1",
                params![snapshot_id, chat_id, latest_read],
            )?;
            for id in &ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO unread_snapshot_messages(snapshot_id,chat_id,message_id,position) VALUES(?1,?2,?3,?3)",
                    params![snapshot_id,chat_id,id],
                )?;
            }
        }
        let execution_id: i64 = transaction.query_row(
            "SELECT execution_id FROM executions WHERE snapshot_id=?1 ORDER BY execution_id DESC LIMIT 1",
            [snapshot_id], |row| row.get(0),
        )?;
        crate::execution::record_delivered_messages(
            &transaction,
            execution_id,
            chat_id,
            &ids,
            now,
        )?;
        transaction.commit()?;
        Ok(messages)
    }
    pub fn snapshot_message(
        &self,
        snapshot_id: i64,
        message_id: i64,
    ) -> Result<Option<MessageView>> {
        let connection = self.read_conn()?;
        let included: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM unread_snapshot_chats sc JOIN messages m ON m.chat_id=sc.chat_id WHERE sc.snapshot_id=?1 AND m.message_id=?2 AND m.message_id<=sc.read_to_message_id AND m.send_status IN ('received','sent'))",
            params![snapshot_id, message_id], |row| row.get(0),
        )?;
        included
            .then(|| load_message(&connection, message_id))
            .transpose()
    }
    pub fn snapshot_contains_message(
        &self,
        snapshot_id: i64,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        message_id: i64,
    ) -> Result<bool> {
        let count: i64 = self.read_conn()?.query_row(
            "SELECT count(*) FROM unread_snapshot_chats sc JOIN chats ch USING(chat_id) JOIN messages m ON m.chat_id=sc.chat_id WHERE sc.snapshot_id=?1 AND ch.channel_id=?2 AND ch.provider_chat_id=?3 AND m.message_id=?4 AND m.message_id<=sc.read_to_message_id AND m.send_status IN ('received','sent')",
            params![snapshot_id, channel_id.as_str(), provider_chat_id, message_id],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }
    pub fn inbox_items(&self, w: &WorkspaceId, a: &AgentId) -> Result<Vec<InboxItemView>> {
        let c = self.read_conn()?;
        let mut s=c.prepare("SELECT inbox_item_id,chat_id,source_id,state,claimed_execution_id FROM agent_inbox_items WHERE workspace_id=?1 AND agent_id=?2 ORDER BY inbox_item_id")?;
        let rows = s.query_map(params![w.as_str(), a.as_str()], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get::<_, String>(3)?,
                r.get(4)?,
            ))
        })?;
        let mut out = vec![];
        for x in rows {
            let (i, ch, src, st, ce) = x?;
            out.push(InboxItemView {
                inbox_item_id: i,
                chat_id: ch,
                source_id: src,
                state: InboxState::parse(&st)?,
                claimed_execution_id: ce,
            })
        }
        Ok(out)
    }
    pub fn snapshot(&self, snapshot_id: i64) -> Result<SnapshotView> {
        let connection = self.read_conn()?;
        load_snapshot(&connection, snapshot_id)
    }
}
