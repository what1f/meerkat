//! Telegram Bot API wire DTOs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct User {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub can_manage_bots: bool,
}

impl User {
    pub fn display_name(&self) -> String {
        let value = format!("{} {}", self.first_name.trim(), self.last_name.trim())
            .trim()
            .to_owned();
        if value.is_empty() {
            self.username.trim().to_owned()
        } else {
            value
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Chat {
    #[serde(default)]
    pub id: i64,
    #[serde(rename = "type", default)]
    pub chat_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub photo: Option<ChatPhoto>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ChatPhoto {
    #[serde(default)]
    pub small_file_id: String,
    #[serde(default)]
    pub big_file_id: String,
}

impl Chat {
    pub fn display_name(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.trim().to_owned();
        }
        let value = format!("{} {}", self.first_name.trim(), self.last_name.trim())
            .trim()
            .to_owned();
        if value.is_empty() {
            self.username.trim().to_owned()
        } else {
            value
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageEntity {
    #[serde(rename = "type", default)]
    pub entity_type: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub length: usize,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct PhotoSize {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_size: i64,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct File {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_size: i64,
    #[serde(default)]
    pub file_path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Document {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub file_size: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Audio {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub duration: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Video {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub duration: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Voice {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub duration: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Sticker {
    #[serde(default)]
    pub file_id: String,
    #[serde(default)]
    pub file_unique_id: String,
    #[serde(default)]
    pub is_animated: bool,
    #[serde(default)]
    pub is_video: bool,
    #[serde(default)]
    pub emoji: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Message {
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub message_thread_id: i64,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub sender_chat: Option<Chat>,
    #[serde(default)]
    pub chat: Chat,
    #[serde(default)]
    pub date: i64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub photo: Vec<PhotoSize>,
    #[serde(default)]
    pub document: Option<Document>,
    #[serde(default)]
    pub audio: Option<Audio>,
    #[serde(default)]
    pub video: Option<Video>,
    #[serde(default)]
    pub voice: Option<Voice>,
    #[serde(default)]
    pub sticker: Option<Sticker>,
    #[serde(default)]
    pub entities: Vec<MessageEntity>,
    #[serde(default)]
    pub caption_entities: Vec<MessageEntity>,
    #[serde(default)]
    pub reply_to_message: Option<Box<Message>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ReactionType {
    #[serde(rename = "type", default)]
    pub reaction_type: String,
    #[serde(default)]
    pub emoji: String,
    #[serde(default)]
    pub custom_emoji_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct MessageReactionUpdated {
    #[serde(default)]
    pub chat: Chat,
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub date: i64,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub actor_chat: Option<Chat>,
    #[serde(default)]
    pub old_reaction: Vec<ReactionType>,
    #[serde(default)]
    pub new_reaction: Vec<ReactionType>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ManagedBot {
    #[serde(default)]
    pub user: User,
    #[serde(default)]
    pub bot: User,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct Update {
    #[serde(default)]
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub edited_message: Option<Message>,
    #[serde(default)]
    pub message_reaction: Option<MessageReactionUpdated>,
    #[serde(default)]
    pub managed_bot: Option<ManagedBot>,
    #[serde(skip)]
    pub raw_payload: Vec<u8>,
}

impl Update {
    pub fn from_json(raw: &[u8]) -> Result<Self, serde_json::Error> {
        let mut update: Self = serde_json::from_slice(raw)?;
        update.raw_payload = raw.to_vec();
        Ok(update)
    }
}

#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct ApiEnvelope<T> {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub result: Option<T>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub error_code: i64,
    #[serde(default)]
    pub parameters: Option<ApiParameters>,
}

impl<T> fmt::Debug for ApiEnvelope<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiEnvelope")
            .field("ok", &self.ok)
            .field("result", &"<redacted>")
            .field("description", &self.description)
            .field("error_code", &self.error_code)
            .field("parameters", &self.parameters)
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct ApiParameters {
    #[serde(default)]
    pub retry_after: u64,
    #[serde(default)]
    pub migrate_to_chat_id: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct DownloadedFile {
    pub data: Vec<u8>,
    pub content_type: String,
}

#[derive(Clone, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct RawResult {
    #[serde(flatten)]
    pub values: std::collections::BTreeMap<String, Value>,
}

impl fmt::Debug for RawResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawResult")
            .field("fields", &"<redacted>")
            .finish()
    }
}
