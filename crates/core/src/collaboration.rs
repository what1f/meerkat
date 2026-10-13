use crate::{
    AgentId, ChannelId, ChatId, DomainError, MessageId, ProviderId, ResourceId, UnixMillis,
    WorkspaceId,
};
use std::collections::BTreeSet;

fn require_text(field: &'static str, value: impl Into<String>) -> Result<String, DomainError> {
    let value = value.into();
    if value.trim().is_empty() {
        Err(DomainError::EmptyField { field })
    } else {
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceStatus {
    Active,
    Paused,
}

impl WorkspaceStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    id: WorkspaceId,
    name: String,
    provider_id: ProviderId,
    status: WorkspaceStatus,
}

impl Workspace {
    pub fn new(
        id: WorkspaceId,
        name: impl Into<String>,
        provider_id: ProviderId,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            id,
            name: require_text("workspace_name", name)?,
            provider_id,
            status: WorkspaceStatus::Active,
        })
    }

    pub fn id(&self) -> &WorkspaceId {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }
    pub const fn status(&self) -> WorkspaceStatus {
        self.status
    }
    pub fn set_status(&mut self, status: WorkspaceStatus) {
        self.status = status;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Agent {
    workspace_id: WorkspaceId,
    id: AgentId,
    name: String,
}

impl Agent {
    pub fn new(
        workspace: &Workspace,
        id: AgentId,
        name: impl Into<String>,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            workspace_id: workspace.id.clone(),
            id,
            name: require_text("agent_name", name)?,
        })
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn id(&self) -> &AgentId {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AttentionLevel {
    Follow,
    Schedule,
    Mention,
}

impl AttentionLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Schedule => "schedule",
            Self::Mention => "mention",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChannelAbility {
    Reply,
    Attachment,
    Reaction,
    Edit,
    Recall,
}

impl ChannelAbility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Attachment => "attachment",
            Self::Reaction => "reaction",
            Self::Edit => "edit",
            Self::Recall => "recall",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Channel {
    workspace_id: WorkspaceId,
    provider_id: ProviderId,
    id: ChannelId,
    name: String,
    attention_levels: BTreeSet<AttentionLevel>,
    abilities: BTreeSet<ChannelAbility>,
}

impl Channel {
    pub fn new(
        workspace: &Workspace,
        id: ChannelId,
        name: impl Into<String>,
        attention_levels: impl IntoIterator<Item = AttentionLevel>,
        abilities: impl IntoIterator<Item = ChannelAbility>,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            workspace_id: workspace.id.clone(),
            provider_id: workspace.provider_id.clone(),
            id,
            name: require_text("channel_name", name)?,
            attention_levels: attention_levels.into_iter().collect(),
            abilities: abilities.into_iter().collect(),
        })
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }
    pub fn id(&self) -> &ChannelId {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn supports_attention(&self, level: AttentionLevel) -> bool {
        self.attention_levels.contains(&level)
    }
    pub fn attention_levels(&self) -> impl ExactSizeIterator<Item = AttentionLevel> + '_ {
        self.attention_levels.iter().copied()
    }
    pub fn supports_ability(&self, ability: ChannelAbility) -> bool {
        self.abilities.contains(&ability)
    }
    pub fn abilities(&self) -> impl ExactSizeIterator<Item = ChannelAbility> + '_ {
        self.abilities.iter().copied()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chat {
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    id: ChatId,
    provider_chat_id: String,
    name: String,
}

impl Chat {
    pub fn new(
        workspace: &Workspace,
        channel: &Channel,
        table_id: i64,
        provider_chat_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, DomainError> {
        ensure_workspace("channel", workspace.id(), channel.workspace_id())?;
        ensure_provider(workspace.provider_id(), channel.provider_id())?;
        Ok(Self {
            workspace_id: workspace.id.clone(),
            channel_id: channel.id.clone(),
            id: ChatId::new(workspace.provider_id.clone(), channel.id.clone(), table_id)?,
            provider_chat_id: require_text("provider_chat_id", provider_chat_id)?,
            name: require_text("chat_name", name)?,
        })
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }
    pub fn id(&self) -> &ChatId {
        &self.id
    }
    pub fn provider_chat_id(&self) -> &str {
        &self.provider_chat_id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageStatus {
    Sending,
    Sent,
    Failed,
    Unknown,
}

impl MessageStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageAuthorType {
    Human,
    Agent,
    Bot,
    System,
}

impl MessageAuthorType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Bot => "bot",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageAuthor {
    kind: MessageAuthorType,
    id: String,
    name: String,
}

impl MessageAuthor {
    pub fn new(
        kind: MessageAuthorType,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            kind,
            id: require_text("author_id", id)?,
            name: require_text("author_name", name)?,
        })
    }
    pub const fn kind(&self) -> MessageAuthorType {
        self.kind
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mention {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
}

impl Mention {
    pub fn for_agent(chat: &Chat, agent: &Agent) -> Result<Self, DomainError> {
        ensure_workspace("mentioned_agent", chat.workspace_id(), agent.workspace_id())?;
        Ok(Self {
            workspace_id: agent.workspace_id.clone(),
            agent_id: agent.id.clone(),
        })
    }
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    chat_id: ChatId,
    message_id: MessageId,
}

impl Reply {
    fn to_message(chat: &Chat, message: &Message) -> Result<Self, DomainError> {
        ensure_chat(chat.id(), message.chat_id())?;
        Ok(Self {
            chat_id: message.chat_id.clone(),
            message_id: message.id,
        })
    }

    pub fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }

    pub const fn message_id(&self) -> MessageId {
        self.message_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attachment {
    resource_id: ResourceId,
    file_name: String,
    media_type: String,
    size_bytes: u64,
    sha256: String,
}

impl Attachment {
    pub fn new(
        resource_id: ResourceId,
        file_name: impl Into<String>,
        media_type: impl Into<String>,
        size_bytes: u64,
        sha256: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let sha256 = sha256.into();
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::InvalidAttachmentHash);
        }
        Ok(Self {
            resource_id,
            file_name: require_text("attachment_file_name", file_name)?,
            media_type: require_text("attachment_media_type", media_type)?,
            size_bytes,
            sha256: sha256.to_ascii_lowercase(),
        })
    }
    pub const fn resource_id(&self) -> ResourceId {
        self.resource_id
    }
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
    pub fn media_type(&self) -> &str {
        &self.media_type
    }
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    id: MessageId,
    workspace_id: WorkspaceId,
    chat_id: ChatId,
    author: MessageAuthor,
    content: String,
    mentions: Vec<Mention>,
    reply: Option<Reply>,
    attachments: Vec<Attachment>,
    status: MessageStatus,
    created_at: UnixMillis,
}

impl Message {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chat: &Chat,
        id: MessageId,
        author: MessageAuthor,
        content: impl Into<String>,
        mentioned_agents: impl IntoIterator<Item = Agent>,
        reply_to: Option<&Message>,
        attachments: Vec<Attachment>,
        status: MessageStatus,
        created_at: UnixMillis,
    ) -> Result<Self, DomainError> {
        let mut mentions = Vec::new();
        let mut seen = BTreeSet::new();
        for agent in mentioned_agents {
            let mention = Mention::for_agent(chat, &agent)?;
            if seen.insert(mention.agent_id.clone()) {
                mentions.push(mention);
            }
        }
        let reply = match reply_to {
            Some(message) => {
                let reply = Reply::to_message(chat, message)?;
                if message.id >= id {
                    return Err(DomainError::MessageOrderRegression {
                        previous_message_id: message.id.get(),
                        proposed_message_id: id.get(),
                    });
                }
                Some(reply)
            }
            None => None,
        };
        Ok(Self {
            id,
            workspace_id: chat.workspace_id.clone(),
            chat_id: chat.id.clone(),
            author,
            content: require_text("message_content", content)?,
            mentions,
            reply,
            attachments,
            status,
            created_at,
        })
    }

    pub const fn id(&self) -> MessageId {
        self.id
    }
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }
    pub fn author(&self) -> &MessageAuthor {
        &self.author
    }
    pub fn content(&self) -> &str {
        &self.content
    }
    pub fn mentions(&self) -> &[Mention] {
        &self.mentions
    }
    pub fn reply(&self) -> Option<&Reply> {
        self.reply.as_ref()
    }
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }
    pub const fn status(&self) -> MessageStatus {
        self.status
    }
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }

    pub fn mentions_agent(&self, agent: &Agent) -> bool {
        self.workspace_id == *agent.workspace_id()
            && self
                .mentions
                .iter()
                .any(|mention| mention.agent_id() == agent.id())
    }

    pub(crate) fn mentions_agent_id(&self, agent_id: &AgentId) -> bool {
        self.mentions
            .iter()
            .any(|mention| mention.agent_id() == agent_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageTimeline {
    chat_id: ChatId,
    messages: Vec<Message>,
}

impl MessageTimeline {
    pub fn new(chat: &Chat) -> Self {
        Self {
            chat_id: chat.id.clone(),
            messages: Vec::new(),
        }
    }

    pub fn append(&mut self, message: Message) -> Result<(), DomainError> {
        ensure_chat(&self.chat_id, message.chat_id())?;
        if let Some(previous) = self.messages.last() {
            if message.id <= previous.id {
                return Err(DomainError::MessageOrderRegression {
                    previous_message_id: previous.id.get(),
                    proposed_message_id: message.id.get(),
                });
            }
        }
        self.messages.push(message);
        Ok(())
    }

    pub fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleInterval {
    Minutes5,
    Minutes15,
    Minutes30,
    Hour1,
    Hours2,
    Hours4,
    Hours8,
    Hours24,
}

impl ScheduleInterval {
    pub const fn minutes(self) -> u32 {
        match self {
            Self::Minutes5 => 5,
            Self::Minutes15 => 15,
            Self::Minutes30 => 30,
            Self::Hour1 => 60,
            Self::Hours2 => 120,
            Self::Hours4 => 240,
            Self::Hours8 => 480,
            Self::Hours24 => 1_440,
        }
    }

    pub fn next_boundary(self, now: UnixMillis) -> Result<UnixMillis, DomainError> {
        let interval = i64::from(self.minutes()) * 60_000;
        let periods = now
            .get()
            .checked_div(interval)
            .and_then(|value| value.checked_add(1))
            .ok_or(DomainError::TimestampOverflow)?;
        let boundary = periods
            .checked_mul(interval)
            .ok_or(DomainError::TimestampOverflow)?;
        UnixMillis::new(boundary)
    }
}

impl TryFrom<u32> for ScheduleInterval {
    type Error = DomainError;

    fn try_from(minutes: u32) -> Result<Self, Self::Error> {
        match minutes {
            5 => Ok(Self::Minutes5),
            15 => Ok(Self::Minutes15),
            30 => Ok(Self::Minutes30),
            60 => Ok(Self::Hour1),
            120 => Ok(Self::Hours2),
            240 => Ok(Self::Hours4),
            480 => Ok(Self::Hours8),
            1_440 => Ok(Self::Hours24),
            _ => Err(DomainError::InvalidScheduleInterval { minutes }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionSetting {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    chat_id: ChatId,
    level: AttentionLevel,
    schedule_interval: Option<ScheduleInterval>,
    next_scheduled_at: Option<UnixMillis>,
}

impl AttentionSetting {
    pub fn new(
        agent: &Agent,
        channel: &Channel,
        chat: &Chat,
        level: AttentionLevel,
        schedule_interval_minutes: Option<u32>,
        now: UnixMillis,
    ) -> Result<Self, DomainError> {
        ensure_workspace("agent", chat.workspace_id(), agent.workspace_id())?;
        ensure_workspace("channel", chat.workspace_id(), channel.workspace_id())?;
        ensure_channel(chat.channel_id(), channel.id())?;
        if !channel.supports_attention(level) {
            return Err(DomainError::UnsupportedAttention {
                channel_id: channel.id().to_string(),
                level: level.as_str(),
            });
        }
        let schedule_interval = match (level, schedule_interval_minutes) {
            (AttentionLevel::Schedule, Some(minutes)) => Some(ScheduleInterval::try_from(minutes)?),
            (AttentionLevel::Schedule, None) => return Err(DomainError::MissingScheduleInterval),
            (_, Some(_)) => {
                return Err(DomainError::UnexpectedScheduleInterval {
                    level: level.as_str(),
                });
            }
            (_, None) => None,
        };
        let next_scheduled_at = schedule_interval
            .map(|interval| interval.next_boundary(now))
            .transpose()?;
        Ok(Self {
            workspace_id: agent.workspace_id.clone(),
            agent_id: agent.id.clone(),
            chat_id: chat.id.clone(),
            level,
            schedule_interval,
            next_scheduled_at,
        })
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
    pub fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }
    pub const fn level(&self) -> AttentionLevel {
        self.level
    }
    pub const fn schedule_interval(&self) -> Option<ScheduleInterval> {
        self.schedule_interval
    }
    pub const fn next_scheduled_at(&self) -> Option<UnixMillis> {
        self.next_scheduled_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatCursor {
    workspace_id: WorkspaceId,
    agent_id: AgentId,
    chat_id: ChatId,
    read_to_message_id: Option<MessageId>,
}

impl AgentChatCursor {
    pub fn new(agent: &Agent, chat: &Chat) -> Result<Self, DomainError> {
        ensure_workspace("agent", chat.workspace_id(), agent.workspace_id())?;
        Ok(Self {
            workspace_id: agent.workspace_id.clone(),
            agent_id: agent.id.clone(),
            chat_id: chat.id.clone(),
            read_to_message_id: None,
        })
    }

    pub fn advance_to(&mut self, message: &Message) -> Result<(), DomainError> {
        ensure_workspace("message", &self.workspace_id, message.workspace_id())?;
        ensure_chat(&self.chat_id, message.chat_id())?;
        if let Some(current) = self.read_to_message_id {
            if message.id < current {
                return Err(DomainError::CursorRegression {
                    current_message_id: current.get(),
                    proposed_message_id: message.id.get(),
                });
            }
        }
        self.read_to_message_id = Some(message.id);
        Ok(())
    }

    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
    pub fn chat_id(&self) -> &ChatId {
        &self.chat_id
    }
    pub const fn read_to_message_id(&self) -> Option<MessageId> {
        self.read_to_message_id
    }
}

pub(crate) fn ensure_workspace(
    entity: &'static str,
    expected: &WorkspaceId,
    actual: &WorkspaceId,
) -> Result<(), DomainError> {
    if expected == actual {
        Ok(())
    } else {
        Err(DomainError::CrossWorkspaceReference {
            entity,
            expected_workspace_id: expected.to_string(),
            actual_workspace_id: actual.to_string(),
        })
    }
}

fn ensure_provider(expected: &ProviderId, actual: &ProviderId) -> Result<(), DomainError> {
    if expected == actual {
        Ok(())
    } else {
        Err(DomainError::ProviderMismatch {
            expected_provider_id: expected.to_string(),
            actual_provider_id: actual.to_string(),
        })
    }
}

pub(crate) fn ensure_channel(expected: &ChannelId, actual: &ChannelId) -> Result<(), DomainError> {
    if expected == actual {
        Ok(())
    } else {
        Err(DomainError::ChannelMismatch {
            expected_channel_id: expected.to_string(),
            actual_channel_id: actual.to_string(),
        })
    }
}

pub(crate) fn ensure_chat(expected: &ChatId, actual: &ChatId) -> Result<(), DomainError> {
    if expected == actual {
        Ok(())
    } else {
        Err(DomainError::ChatMismatch {
            expected_chat_id: expected.to_string(),
            actual_chat_id: actual.to_string(),
        })
    }
}
