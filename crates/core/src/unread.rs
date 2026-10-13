use crate::collaboration::{ensure_chat, ensure_workspace};
use crate::{
    Agent, AgentChatCursor, AgentId, Channel, Chat, ChatId, DomainError, ExecutionId, Message,
    MessageId, MessageTimeline, SnapshotId, UnixMillis, WorkspaceId,
};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadChatInput {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    chat: Chat,
    channel_name: String,
    cursor: Option<MessageId>,
    messages: Vec<Message>,
    mentioned_message_id: Option<MessageId>,
}

impl UnreadChatInput {
    pub fn new(
        agent: &Agent,
        channel: &Channel,
        chat: &Chat,
        cursor: &AgentChatCursor,
        timeline: &MessageTimeline,
        mentioned_message_id: Option<MessageId>,
    ) -> Result<Self, DomainError> {
        ensure_workspace("agent", chat.workspace_id(), agent.workspace_id())?;
        ensure_workspace("channel", chat.workspace_id(), channel.workspace_id())?;
        ensure_chat(chat.id(), cursor.chat_id())?;
        ensure_chat(chat.id(), timeline.chat_id())?;
        if cursor.agent_id() != agent.id() {
            return Err(DomainError::AgentMismatch {
                expected_agent_id: agent.id().to_string(),
                actual_agent_id: cursor.agent_id().to_string(),
            });
        }
        Ok(Self {
            workspace_id: agent.workspace_id().clone(),
            agent_id: agent.id().clone(),
            chat: chat.clone(),
            channel_name: channel.name().to_owned(),
            cursor: cursor.read_to_message_id(),
            messages: timeline.messages().to_vec(),
            mentioned_message_id,
        })
    }

    pub fn chat_id(&self) -> &ChatId {
        self.chat.id()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadSnapshotInput {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    created_at: UnixMillis,
    chats: Vec<UnreadChatInput>,
}

impl UnreadSnapshotInput {
    pub fn new(agent: &Agent, created_at: UnixMillis) -> Self {
        Self {
            workspace_id: agent.workspace_id().clone(),
            agent_id: agent.id().clone(),
            created_at,
            chats: Vec::new(),
        }
    }

    pub fn add_chat(&mut self, input: UnreadChatInput) -> Result<(), DomainError> {
        ensure_workspace("unread_chat", &self.workspace_id, &input.workspace_id)?;
        if input.agent_id != self.agent_id {
            return Err(DomainError::AgentMismatch {
                expected_agent_id: self.agent_id.to_string(),
                actual_agent_id: input.agent_id.to_string(),
            });
        }
        if self
            .chats
            .iter()
            .any(|existing| existing.chat.id() == input.chat.id())
        {
            return Err(DomainError::DuplicateChat {
                chat_id: input.chat.id().to_string(),
            });
        }
        self.chats.push(input);
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadChatSnapshot {
    chat_id: ChatId,
    channel_id: crate::ChannelId,
    channel_name: String,
    read_to_message_id: MessageId,
    unread_count: usize,
    mentioned_message_id: Option<MessageId>,
    recent_messages: Vec<Message>,
}

impl UnreadChatSnapshot {
    pub fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }
    pub fn channel_id(&self) -> &crate::ChannelId {
        &self.channel_id
    }
    pub fn channel_name(&self) -> &str {
        &self.channel_name
    }
    pub const fn read_to_message_id(&self) -> MessageId {
        self.read_to_message_id
    }
    pub const fn unread_count(&self) -> usize {
        self.unread_count
    }
    pub const fn mentioned_message_id(&self) -> Option<MessageId> {
        self.mentioned_message_id
    }
    pub fn recent_messages(&self) -> &[Message] {
        &self.recent_messages
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnreadSnapshot {
    id: SnapshotId,
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    created_at: UnixMillis,
    chats: Vec<UnreadChatSnapshot>,
}

impl UnreadSnapshot {
    pub fn build(id: SnapshotId, mut input: UnreadSnapshotInput) -> Result<Self, DomainError> {
        input
            .chats
            .sort_by(|left, right| left.chat.id().cmp(right.chat.id()));
        let mut chats = Vec::new();
        for chat_input in input.chats {
            let unread: Vec<_> = chat_input
                .messages
                .into_iter()
                .filter(|message| chat_input.cursor.is_none_or(|cursor| message.id() > cursor))
                .collect();
            let Some(read_to_message_id) = unread.last().map(Message::id) else {
                continue;
            };

            let mentioned_message_id = match chat_input.mentioned_message_id {
                Some(message_id) => {
                    match unread.iter().find(|message| message.id() == message_id) {
                        Some(message) if message.mentions_agent_id(&chat_input.agent_id) => {
                            Some(message_id)
                        }
                        _ => {
                            return Err(DomainError::MentionedMessageNotUnread {
                                message_id: message_id.get(),
                            });
                        }
                    }
                }
                None => None,
            };
            let recent_messages = select_recent_messages(&unread, mentioned_message_id);
            chats.push(UnreadChatSnapshot {
                chat_id: chat_input.chat.id().clone(),
                channel_id: chat_input.chat.channel_id().clone(),
                channel_name: chat_input.channel_name,
                read_to_message_id,
                unread_count: unread.len(),
                mentioned_message_id,
                recent_messages,
            });
        }
        if chats.is_empty() {
            return Err(DomainError::EmptyUnreadSnapshot);
        }
        Ok(Self {
            id,
            workspace_id: input.workspace_id,
            agent_id: input.agent_id,
            created_at: input.created_at,
            chats,
        })
    }

    pub const fn id(&self) -> SnapshotId {
        self.id
    }
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }
    pub fn chats(&self) -> &[UnreadChatSnapshot] {
        &self.chats
    }
}

fn select_recent_messages(unread: &[Message], mentioned: Option<MessageId>) -> Vec<Message> {
    let selected_ids: BTreeSet<_> = match mentioned {
        Some(mentioned_id) => std::iter::once(mentioned_id)
            .chain(
                unread
                    .iter()
                    .rev()
                    .filter(|message| message.id() != mentioned_id)
                    .take(2)
                    .map(Message::id),
            )
            .collect(),
        None => unread.iter().rev().take(3).map(Message::id).collect(),
    };
    unread
        .iter()
        .filter(|message| selected_ids.contains(&message.id()))
        .cloned()
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TriggerReason {
    DirectMessage {
        chat_id: ChatId,
        message_id: MessageId,
    },
    Mention {
        chat_id: ChatId,
        message_id: MessageId,
    },
    Follow {
        chat_id: ChatId,
    },
    Schedule,
    Custom {
        name: String,
        object_id: String,
    },
}

impl TriggerReason {
    pub fn custom(
        name: impl Into<String>,
        object_id: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        let object_id = object_id.into();
        if name.trim().is_empty() {
            return Err(DomainError::EmptyField {
                field: "trigger_name",
            });
        }
        if object_id.trim().is_empty() {
            return Err(DomainError::EmptyField {
                field: "trigger_object_id",
            });
        }
        Ok(Self::Custom { name, object_id })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionInput {
    execution_id: ExecutionId,
    snapshot: UnreadSnapshot,
    trigger_reasons: Vec<TriggerReason>,
}

impl ExecutionInput {
    pub fn new(
        execution_id: ExecutionId,
        snapshot: &UnreadSnapshot,
        trigger_reasons: Vec<TriggerReason>,
    ) -> Result<Self, DomainError> {
        if trigger_reasons.is_empty() {
            return Err(DomainError::EmptyTriggerReasons);
        }
        for reason in &trigger_reasons {
            match reason {
                TriggerReason::DirectMessage {
                    chat_id,
                    message_id,
                }
                | TriggerReason::Mention {
                    chat_id,
                    message_id,
                } => validate_trigger(snapshot, chat_id, Some(*message_id))?,
                TriggerReason::Follow { chat_id } => validate_trigger(snapshot, chat_id, None)?,
                TriggerReason::Schedule | TriggerReason::Custom { .. } => {}
            }
        }
        Ok(Self {
            execution_id,
            snapshot: snapshot.clone(),
            trigger_reasons,
        })
    }

    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    pub fn snapshot(&self) -> &UnreadSnapshot {
        &self.snapshot
    }
    pub fn trigger_reasons(&self) -> &[TriggerReason] {
        &self.trigger_reasons
    }
}

fn validate_trigger(
    snapshot: &UnreadSnapshot,
    chat_id: &ChatId,
    message_id: Option<MessageId>,
) -> Result<(), DomainError> {
    let chat = snapshot.chats.iter().find(|chat| chat.chat_id() == chat_id);
    let valid = match (chat, message_id) {
        (Some(_), None) => true,
        (Some(chat), Some(message_id)) => message_id <= chat.read_to_message_id(),
        (None, _) => false,
    };
    if valid {
        Ok(())
    } else {
        Err(DomainError::TriggerOutsideSnapshot {
            chat_id: chat_id.to_string(),
            message_id: message_id.map(MessageId::get),
        })
    }
}
