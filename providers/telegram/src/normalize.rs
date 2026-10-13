//! Pure Telegram update normalization.

use crate::dto::{Chat, Message, MessageReactionUpdated, ReactionType, Update, User};
use crate::mention::{mentioned_agent_ids, utf16_slice};
use lankide_core::markdown_mention;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum SenderKind {
    Human,
    Agent,
    Bot,
    System,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedSender {
    pub external_id: String,
    pub sender_id: String,
    pub kind: SenderKind,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedMention {
    pub external_user_id: i64,
    pub agent_id: Option<String>,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedAttachment {
    pub kind: String,
    pub file_id: String,
    pub file_unique_id: String,
    pub name: String,
    pub content_type: String,
    pub file_size: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedMessage {
    pub provider_id: String,
    pub account_id: String,
    pub provider_message_id: String,
    pub chat_id: String,
    pub channel_id: String,
    pub chat_type: String,
    pub content: String,
    pub sender: NormalizedSender,
    pub reply_to_provider_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub mentions: Vec<NormalizedMention>,
    pub mentioned_agent_ids: Vec<String>,
    pub attachments: Vec<NormalizedAttachment>,
    pub edited: bool,
    pub occurred_at_s: Option<i64>,
    pub update_id: i64,
    pub event_id: String,
    pub correlation_key: String,
    pub raw: Vec<u8>,
}

pub use lankide_core::ReactionAction;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct NormalizedReaction {
    pub provider_id: String,
    pub account_id: String,
    pub action: ReactionAction,
    pub chat_id: String,
    pub channel_id: String,
    pub target_provider_message_id: String,
    pub reaction: String,
    pub sender: NormalizedSender,
    pub occurred_at_s: Option<i64>,
    pub update_id: i64,
    pub event_id: String,
    pub raw: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum NormalizedEvent {
    Message(NormalizedMessage),
    Reaction(NormalizedReaction),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizationConfig {
    pub provider_id: String,
    pub account_id: String,
    pub bot: User,
    pub agent_by_external_id: HashMap<i64, String>,
}

impl NormalizationConfig {
    pub fn new(provider_id: impl Into<String>, account_id: impl Into<String>, bot: User) -> Self {
        Self {
            provider_id: provider_id.into().trim().to_owned(),
            account_id: account_id.into().trim().to_owned(),
            bot,
            agent_by_external_id: HashMap::new(),
        }
    }
    pub fn with_agent(mut self, external_id: i64, agent_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        if external_id != 0 && !agent_id.trim().is_empty() {
            self.agent_by_external_id
                .insert(external_id, agent_id.trim().to_owned());
        }
        self
    }
}

pub fn normalize_update(
    update: &Update,
    config: &NormalizationConfig,
) -> Result<Vec<NormalizedEvent>, NormalizeError> {
    if update.update_id == 0 {
        return Err(NormalizeError::Incomplete(
            "update_id is required".to_owned(),
        ));
    }
    if let Some(message) = update.message.as_ref() {
        return normalize_message_with_raw(
            update.update_id,
            message,
            false,
            config,
            &update.raw_payload,
        )
        .map(|event| vec![NormalizedEvent::Message(event)]);
    }
    if let Some(message) = update.edited_message.as_ref() {
        return normalize_message_with_raw(
            update.update_id,
            message,
            true,
            config,
            &update.raw_payload,
        )
        .map(|event| vec![NormalizedEvent::Message(event)]);
    }
    if let Some(reaction) = update.message_reaction.as_ref() {
        return Ok(normalize_reaction_with_raw(
            update.update_id,
            reaction,
            config,
            &update.raw_payload,
        )
        .into_iter()
        .map(NormalizedEvent::Reaction)
        .collect());
    }
    Ok(Vec::new())
}

pub fn normalize_message(
    update_id: i64,
    message: &Message,
    edited: bool,
    config: &NormalizationConfig,
) -> Result<NormalizedMessage, NormalizeError> {
    normalize_message_with_raw(update_id, message, edited, config, &[])
}

fn normalize_message_with_raw(
    update_id: i64,
    message: &Message,
    edited: bool,
    config: &NormalizationConfig,
    raw: &[u8],
) -> Result<NormalizedMessage, NormalizeError> {
    if message.message_id == 0 || message.chat.id == 0 {
        return Err(NormalizeError::Incomplete(
            "message_id and chat.id are required".to_owned(),
        ));
    }
    let (text, entities) = if !message.text.is_empty() {
        (&message.text, &message.entities)
    } else {
        (&message.caption, &message.caption_entities)
    };
    let sender = normalize_sender(message.from.as_ref(), message.sender_chat.as_ref(), config);
    let mentions = normalize_mentions(text, entities, config);
    let content = human_mention_markers(text, entities, config);
    let attachments = normalize_attachments(message);
    let source_type = if message.chat.chat_type == "private" {
        "private_chat"
    } else {
        "group_chat"
    };
    let original_message_id = message.message_id.to_string();
    let message_id = if edited {
        format!("{original_message_id}:edit:{update_id}")
    } else {
        original_message_id.clone()
    };
    let event_id = format!("telegram:update:{update_id}");
    Ok(NormalizedMessage {
        provider_id: config.provider_id.clone(),
        account_id: config.account_id.clone(),
        provider_message_id: message_id.clone(),
        chat_id: message.chat.id.to_string(),
        channel_id: source_type.to_owned(),
        chat_type: message.chat.chat_type.clone(),
        content,
        sender,
        reply_to_provider_message_id: message
            .reply_to_message
            .as_ref()
            .map(|value| value.message_id.to_string()),
        thread_id: (message.message_thread_id != 0).then(|| message.message_thread_id.to_string()),
        mentions: mentions.0,
        mentioned_agent_ids: mentions.1,
        attachments,
        edited,
        occurred_at_s: (message.date != 0).then_some(message.date),
        update_id,
        event_id,
        correlation_key: if edited {
            format!(
                "telegram:{}:{}:edit:{update_id}",
                message.chat.id, message.message_id
            )
        } else {
            format!("telegram:{}:{}", message.chat.id, message.message_id)
        },
        raw: raw.to_vec(),
    })
}

pub fn normalize_reaction(
    update_id: i64,
    reaction: &MessageReactionUpdated,
    config: &NormalizationConfig,
) -> Vec<NormalizedReaction> {
    normalize_reaction_with_raw(update_id, reaction, config, &[])
}

fn normalize_reaction_with_raw(
    update_id: i64,
    reaction: &MessageReactionUpdated,
    config: &NormalizationConfig,
    raw: &[u8],
) -> Vec<NormalizedReaction> {
    let old = reaction_set(&reaction.old_reaction);
    let new = reaction_set(&reaction.new_reaction);
    let sender = normalize_sender(reaction.user.as_ref(), reaction.actor_chat.as_ref(), config);
    let channel_id = if reaction.chat.chat_type == "private" {
        "private_chat"
    } else {
        "group_chat"
    }
    .to_owned();
    let mut result = Vec::new();
    for emoji in old.difference(&new) {
        result.push(NormalizedReaction {
            provider_id: config.provider_id.clone(),
            account_id: config.account_id.clone(),
            action: ReactionAction::Remove,
            chat_id: reaction.chat.id.to_string(),
            channel_id: channel_id.clone(),
            target_provider_message_id: reaction.message_id.to_string(),
            reaction: emoji.clone(),
            sender: sender.clone(),
            occurred_at_s: (reaction.date != 0).then_some(reaction.date),
            update_id,
            event_id: format!("telegram:update:{update_id}:reaction:remove:{emoji}"),
            raw: raw.to_vec(),
        });
    }
    for emoji in new.difference(&old) {
        result.push(NormalizedReaction {
            provider_id: config.provider_id.clone(),
            account_id: config.account_id.clone(),
            action: ReactionAction::Add,
            chat_id: reaction.chat.id.to_string(),
            channel_id: channel_id.clone(),
            target_provider_message_id: reaction.message_id.to_string(),
            reaction: emoji.clone(),
            sender: sender.clone(),
            occurred_at_s: (reaction.date != 0).then_some(reaction.date),
            update_id,
            event_id: format!("telegram:update:{update_id}:reaction:add:{emoji}"),
            raw: raw.to_vec(),
        });
    }
    result
}

fn normalize_sender(
    user: Option<&User>,
    actor_chat: Option<&Chat>,
    config: &NormalizationConfig,
) -> NormalizedSender {
    if let Some(user) = user {
        let external_id = user.id.to_string();
        let agent_id = config.agent_by_external_id.get(&user.id).cloned();
        let kind = if user.is_bot {
            if agent_id.is_some() {
                SenderKind::Agent
            } else {
                SenderKind::Bot
            }
        } else {
            SenderKind::Human
        };
        let sender_id = agent_id.clone().unwrap_or_else(|| external_id.clone());
        return NormalizedSender {
            external_id,
            sender_id,
            kind,
            display_name: first_non_empty(&[
                user.display_name(),
                user.username.clone(),
                user.id.to_string(),
            ]),
        };
    }
    if let Some(chat) = actor_chat {
        let external_id = format!("chat:{}", chat.id);
        return NormalizedSender {
            external_id: external_id.clone(),
            sender_id: external_id,
            kind: SenderKind::Unknown,
            display_name: first_non_empty(&[chat.display_name(), chat.id.to_string()]),
        };
    }
    NormalizedSender {
        external_id: String::new(),
        sender_id: String::new(),
        kind: SenderKind::Unknown,
        display_name: String::new(),
    }
}

fn normalize_mentions(
    text: &str,
    entities: &[crate::dto::MessageEntity],
    config: &NormalizationConfig,
) -> (Vec<NormalizedMention>, Vec<String>) {
    let mut mentions = Vec::new();
    let mut agent_ids = Vec::new();
    for entity in entities
        .iter()
        .filter(|entity| entity.entity_type == "mention" || entity.entity_type == "text_mention")
    {
        let user = entity.user.as_ref();
        let external_id = user
            .map(|value| value.id)
            .or_else(|| {
                let username_value = utf16_slice(text, entity.offset, entity.length);
                let username = username_value.trim_start_matches('@');
                config
                    .bot
                    .username
                    .eq_ignore_ascii_case(username)
                    .then_some(config.bot.id)
            })
            .unwrap_or(0);
        if external_id == 0 {
            continue;
        }
        let agent_id = config.agent_by_external_id.get(&external_id).cloned();
        if let Some(agent_id) = &agent_id {
            if !agent_ids.contains(agent_id) {
                agent_ids.push(agent_id.clone());
            }
        }
        mentions.push(NormalizedMention {
            external_user_id: external_id,
            agent_id,
            display_name: user
                .map(User::display_name)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| utf16_slice(text, entity.offset, entity.length)),
        });
    }
    // Keep the same mention interpretation used by the outbound/inbound
    // helper. It also handles UTF-16 offsets for astral Unicode characters.
    let mentioned = mentioned_agent_ids(text, entities, &config.bot, &config.agent_by_external_id);
    for agent_id in mentioned {
        if !agent_ids.contains(&agent_id) {
            agent_ids.push(agent_id);
        }
    }
    (mentions, agent_ids)
}

fn human_mention_markers(
    text: &str,
    entities: &[crate::dto::MessageEntity],
    config: &NormalizationConfig,
) -> String {
    let mut replacements = entities
        .iter()
        .filter(|entity| entity.entity_type == "text_mention")
        .filter_map(|entity| {
            let user = entity.user.as_ref()?;
            if user.is_bot || config.agent_by_external_id.contains_key(&user.id) {
                return None;
            }
            let range = utf16_byte_range(text, entity.offset, entity.length)?;
            markdown_mention(&user.display_name(), &user.id.to_string())
                .map(|marker| (range, marker))
        })
        .collect::<Vec<_>>();
    replacements.sort_by(|left, right| right.0.start.cmp(&left.0.start));
    let mut result = text.to_owned();
    for (range, marker) in replacements {
        result.replace_range(range, &marker);
    }
    result
}

fn utf16_byte_range(value: &str, offset: usize, length: usize) -> Option<std::ops::Range<usize>> {
    let end = offset.checked_add(length)?;
    let mut utf16_position = 0;
    let mut start_byte = None;
    let mut end_byte = None;
    for (byte, character) in value.char_indices() {
        if utf16_position == offset {
            start_byte = Some(byte);
        }
        if utf16_position == end {
            end_byte = Some(byte);
            break;
        }
        utf16_position += character.len_utf16();
    }
    if utf16_position == offset && start_byte.is_none() {
        start_byte = Some(value.len());
    }
    if utf16_position == end && end_byte.is_none() {
        end_byte = Some(value.len());
    }
    Some(start_byte?..end_byte?)
}

fn normalize_attachments(message: &Message) -> Vec<NormalizedAttachment> {
    let mut result = Vec::new();
    if let Some(photo) = largest_photo(&message.photo) {
        result.push(NormalizedAttachment {
            kind: "image".to_owned(),
            file_id: photo.file_id.clone(),
            file_unique_id: photo.file_unique_id.clone(),
            name: format!(
                "photo-{}.jpg",
                safe_name(if photo.file_unique_id.is_empty() {
                    &photo.file_id
                } else {
                    &photo.file_unique_id
                })
            ),
            content_type: "image/jpeg".to_owned(),
            file_size: photo.file_size,
        });
    }
    if let Some(document) = &message.document {
        result.push(NormalizedAttachment {
            kind: "file".to_owned(),
            file_id: document.file_id.clone(),
            file_unique_id: document.file_unique_id.clone(),
            name: first_non_empty(&[
                document.file_name.clone(),
                format!("document-{}", document.file_unique_id),
            ]),
            content_type: first_non_empty(&[
                document.mime_type.clone(),
                "application/octet-stream".to_owned(),
            ]),
            file_size: document.file_size,
        });
    }
    if let Some(audio) = &message.audio {
        result.push(NormalizedAttachment {
            kind: "audio".to_owned(),
            file_id: audio.file_id.clone(),
            file_unique_id: audio.file_unique_id.clone(),
            name: first_non_empty(&[
                audio.file_name.clone(),
                format!("audio-{}", audio.file_unique_id),
            ]),
            content_type: first_non_empty(&[audio.mime_type.clone(), "audio/*".to_owned()]),
            file_size: 0,
        });
    }
    if let Some(video) = &message.video {
        result.push(NormalizedAttachment {
            kind: "video".to_owned(),
            file_id: video.file_id.clone(),
            file_unique_id: video.file_unique_id.clone(),
            name: first_non_empty(&[
                video.file_name.clone(),
                format!("video-{}", video.file_unique_id),
            ]),
            content_type: first_non_empty(&[video.mime_type.clone(), "video/*".to_owned()]),
            file_size: 0,
        });
    }
    if let Some(voice) = &message.voice {
        result.push(NormalizedAttachment {
            kind: "voice".to_owned(),
            file_id: voice.file_id.clone(),
            file_unique_id: voice.file_unique_id.clone(),
            name: format!("voice-{}.ogg", safe_name(&voice.file_unique_id)),
            content_type: first_non_empty(&[voice.mime_type.clone(), "audio/ogg".to_owned()]),
            file_size: 0,
        });
    }
    if let Some(sticker) = &message.sticker {
        result.push(NormalizedAttachment {
            kind: "sticker".to_owned(),
            file_id: sticker.file_id.clone(),
            file_unique_id: sticker.file_unique_id.clone(),
            name: format!("sticker-{}", safe_name(&sticker.file_unique_id)),
            content_type: "image/*".to_owned(),
            file_size: 0,
        });
    }
    result
}

fn largest_photo(photos: &[crate::dto::PhotoSize]) -> Option<&crate::dto::PhotoSize> {
    photos.iter().max_by_key(|photo| {
        (
            i64::from(photo.width) * i64::from(photo.height),
            photo.file_size,
        )
    })
}
fn reaction_set(values: &[ReactionType]) -> HashSet<String> {
    values
        .iter()
        .filter(|value| value.reaction_type == "emoji" && !value.emoji.trim().is_empty())
        .map(|value| value.emoji.clone())
        .collect()
}
fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .find(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_default()
}
fn safe_name(value: &str) -> String {
    let value = value.trim();
    let value = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect::<String>();
    if value.is_empty() {
        "attachment".to_owned()
    } else {
        value
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NormalizeError {
    Incomplete(String),
}
impl std::fmt::Display for NormalizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete(message) => write!(formatter, "incomplete Telegram update: {message}"),
        }
    }
}
impl std::error::Error for NormalizeError {}
