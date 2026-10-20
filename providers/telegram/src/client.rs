//! Telegram Bot API client and Manager Bot operations.

use crate::dto::{ApiEnvelope, Chat, DownloadedFile, File, Message, Update, User};
use crate::http::{ClientError, Method, Request, Response, Transport, encode_path_segment};
use crate::mention::render_html_with_mentions;
use serde_json::{Value, json};
use std::fmt;

pub const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
pub const TELEGRAM_FILE_BASE_URL: &str = "https://api.telegram.org";
pub const MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone)]
pub struct Credentials {
    pub account_id: String,
    token: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("account_id", &self.account_id)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Credentials {
    pub fn new(account_id: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            token: token.into(),
        }
    }
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.account_id.trim().is_empty()
            || self.token.trim().is_empty()
            || !self
                .token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
        {
            return Err(ClientError::InvalidInput(
                "Telegram Bot account ID and token are required".to_owned(),
            ));
        }
        Ok(())
    }
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentMessage {
    pub message_id: i64,
    pub chat_id: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollRequest {
    pub offset: i64,
    pub timeout_seconds: u8,
    pub allowed_updates: Vec<String>,
}

impl Default for PollRequest {
    fn default() -> Self {
        Self {
            offset: 0,
            timeout_seconds: 8,
            allowed_updates: vec![
                "message".to_owned(),
                "edited_message".to_owned(),
                "message_reaction".to_owned(),
            ],
        }
    }
}

impl PollRequest {
    /// Poll only the Manager Bot update stream used by Bot Management Mode.
    /// The regular message poller deliberately does not consume this stream.
    pub fn managed_bot() -> Self {
        Self {
            offset: 0,
            timeout_seconds: 8,
            allowed_updates: vec!["managed_bot".to_owned()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedBotRegistration {
    pub username: String,
    pub display_name: String,
    pub manager_username: String,
}

impl ManagedBotRegistration {
    pub fn verification_uri(&self) -> String {
        format!(
            "https://t.me/newbot/{}/{}?name={}",
            self.manager_username,
            self.username,
            percent_encode(&self.display_name)
        )
    }

    pub fn matches(&self, bot: &User) -> bool {
        let username_matches = !bot.username.trim().is_empty()
            && bot.username.eq_ignore_ascii_case(self.username.trim());
        let name_matches = !bot.display_name().trim().is_empty()
            && bot
                .display_name()
                .eq_ignore_ascii_case(self.display_name.trim());
        username_matches || name_matches
    }
}

/// Credential returned by the Manager Bot after a `managed_bot` update.
///
/// The token is intentionally private and omitted from `Debug`; the host can
/// pass it directly to `Credentials::new` through [`ManagedBotCredential::token`] without
/// placing it in logs or persisted registration metadata.
pub struct ManagedBotCredential {
    pub account_id: String,
    pub creator_user_id: String,
    pub creator: crate::dto::User,
    pub username: String,
    pub display_name: String,
    token: String,
}

impl fmt::Debug for ManagedBotCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedBotCredential")
            .field("account_id", &self.account_id)
            .field("creator_user_id", &self.creator_user_id)
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl ManagedBotCredential {
    pub fn token(&self) -> &str {
        &self.token
    }
}

#[derive(Clone)]
pub struct Client<T> {
    transport: T,
    domain: String,
    credentials: Credentials,
}

impl<T> fmt::Debug for Client<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramClient")
            .field("domain", &self.domain)
            .field("credentials", &self.credentials)
            .finish()
    }
}

impl<T: Transport> Client<T> {
    pub fn new(transport: T, credentials: Credentials) -> Result<Self, ClientError> {
        credentials.validate()?;
        Ok(Self {
            transport,
            domain: TELEGRAM_API_BASE_URL.to_owned(),
            credentials,
        })
    }
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into().trim_end_matches('/').to_owned();
        self
    }
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }
    pub async fn get_me(&self) -> Result<User, ClientError> {
        self.call("getMe", None::<&Value>, "Telegram getMe").await
    }
    /// Verify the configured credential against Telegram and the persisted
    /// account ID before a provider installation is considered ready.
    pub async fn verify_credentials(&self) -> Result<User, ClientError> {
        let bot = self.get_me().await?;
        if !bot.is_bot || bot.id == 0 {
            return Err(ClientError::InvalidInput(
                "Telegram credential does not belong to a Bot".to_owned(),
            ));
        }
        if self.credentials.account_id.trim() != bot.id.to_string() {
            return Err(ClientError::InvalidInput(
                "Telegram Bot token does not match the configured account ID".to_owned(),
            ));
        }
        Ok(bot)
    }
    pub async fn get_updates(&self, request: &PollRequest) -> Result<Vec<Update>, ClientError> {
        let raw_updates = self.get_update_values(request).await?;
        raw_updates
            .into_iter()
            .map(|value| {
                let raw = serde_json::to_vec(&value)
                    .map_err(|error| ClientError::Encode(error.to_string()))?;
                Update::from_json(&raw).map_err(|error| ClientError::Decode(error.to_string()))
            })
            .collect()
    }
    pub(crate) async fn get_update_values(
        &self,
        request: &PollRequest,
    ) -> Result<Vec<Value>, ClientError> {
        let body = json!({ "offset": request.offset, "timeout": request.timeout_seconds.min(50), "allowed_updates": request.allowed_updates });
        self.call("getUpdates", Some(&body), "Telegram getUpdates")
            .await
    }
    pub async fn send_message(
        &self,
        chat_id: i64,
        html: &str,
        reply_to: Option<i64>,
    ) -> Result<SentMessage, ClientError> {
        let mut body = json!({ "chat_id": chat_id, "text": html, "parse_mode": "HTML" });
        if let Some(message_id) = reply_to.filter(|value| *value != 0) {
            body["reply_parameters"] = json!({ "message_id": message_id });
        }
        let message: Message = self
            .call("sendMessage", Some(&body), "Telegram sendMessage")
            .await?;
        message_ref(message, "Telegram sendMessage")
    }
    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        html: &str,
    ) -> Result<(), ClientError> {
        if html.trim().is_empty() {
            return Err(ClientError::InvalidInput(
                "Telegram edited message cannot be empty".to_owned(),
            ));
        }
        let _: Message = self
            .call(
                "editMessageText",
                Some(&json!({
                    "chat_id": chat_id,
                    "message_id": message_id,
                    "text": html,
                    "parse_mode": "HTML",
                })),
                "Telegram editMessageText",
            )
            .await?;
        Ok(())
    }
    pub async fn send_markdown(
        &self,
        chat_id: i64,
        markdown: &str,
        reply_to: Option<i64>,
        mentions: &[crate::mention::MentionTarget],
    ) -> Result<SentMessage, ClientError> {
        self.send_message(
            chat_id,
            &render_html_with_mentions(markdown, mentions),
            reply_to,
        )
        .await
    }
    pub async fn send_photo(
        &self,
        chat_id: i64,
        data: &[u8],
        file_name: &str,
        caption_html: &str,
        reply_to: Option<i64>,
    ) -> Result<SentMessage, ClientError> {
        if data.is_empty() {
            return Err(ClientError::InvalidInput(
                "Telegram photo cannot be empty".to_owned(),
            ));
        }
        let mut fields = vec![("chat_id", chat_id.to_string().into_bytes())];
        if !caption_html.trim().is_empty() {
            fields.push(("caption", caption_html.as_bytes().to_vec()));
            fields.push(("parse_mode", b"HTML".to_vec()));
        }
        if let Some(message_id) = reply_to.filter(|value| *value != 0) {
            fields.push((
                "reply_parameters",
                serde_json::to_vec(&json!({ "message_id": message_id }))
                    .map_err(|error| ClientError::Encode(error.to_string()))?,
            ));
        }
        let (body, content_type) = multipart(
            &fields,
            "photo",
            file_name,
            data,
            "application/octet-stream",
        )?;
        let message: Message = self
            .multipart_call("sendPhoto", body, content_type, "Telegram sendPhoto")
            .await?;
        message_ref(message, "Telegram sendPhoto")
    }
    pub async fn send_document(
        &self,
        chat_id: i64,
        data: &[u8],
        file_name: &str,
        caption_html: &str,
        reply_to: Option<i64>,
    ) -> Result<SentMessage, ClientError> {
        if data.is_empty() {
            return Err(ClientError::InvalidInput(
                "Telegram document cannot be empty".to_owned(),
            ));
        }
        let mut fields = vec![("chat_id", chat_id.to_string().into_bytes())];
        if !caption_html.trim().is_empty() {
            fields.push(("caption", caption_html.as_bytes().to_vec()));
            fields.push(("parse_mode", b"HTML".to_vec()));
        }
        if let Some(message_id) = reply_to.filter(|value| *value != 0) {
            fields.push((
                "reply_parameters",
                serde_json::to_vec(&json!({ "message_id": message_id }))
                    .map_err(|error| ClientError::Encode(error.to_string()))?,
            ));
        }
        let (body, content_type) = multipart(
            &fields,
            "document",
            file_name,
            data,
            "application/octet-stream",
        )?;
        let message: Message = self
            .multipart_call("sendDocument", body, content_type, "Telegram sendDocument")
            .await?;
        message_ref(message, "Telegram sendDocument")
    }
    pub async fn delete_message(&self, chat_id: i64, message_id: i64) -> Result<(), ClientError> {
        let body = json!({ "chat_id": chat_id, "message_id": message_id });
        let _: bool = self
            .call("deleteMessage", Some(&body), "Telegram deleteMessage")
            .await?;
        Ok(())
    }
    pub async fn set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: Option<&str>,
    ) -> Result<(), ClientError> {
        let reaction = emoji
            .filter(|value| !value.trim().is_empty())
            .map(|value| vec![json!({ "type": "emoji", "emoji": value.trim() })])
            .unwrap_or_default();
        let body = json!({ "chat_id": chat_id, "message_id": message_id, "reaction": reaction });
        let _: bool = self
            .call(
                "setMessageReaction",
                Some(&body),
                "Telegram setMessageReaction",
            )
            .await?;
        Ok(())
    }
    pub async fn get_file(&self, file_id: &str) -> Result<File, ClientError> {
        self.call(
            "getFile",
            Some(&json!({ "file_id": file_id })),
            "Telegram getFile",
        )
        .await
    }
    pub async fn user_profile_photo(
        &self,
        user_id: i64,
    ) -> Result<Option<DownloadedFile>, ClientError> {
        let photos: Value = self
            .call(
                "getUserProfilePhotos",
                Some(&json!({"user_id":user_id,"limit":1})),
                "Telegram getUserProfilePhotos",
            )
            .await?;
        let file = photos
            .get("photos")
            .and_then(Value::as_array)
            .and_then(|p| p.first())
            .and_then(Value::as_array)
            .and_then(|p| p.last())
            .and_then(|p| p.get("file_id"))
            .and_then(Value::as_str);
        match file {
            Some(id) => Ok(Some(self.download_file(id).await?)),
            None => Ok(None),
        }
    }
    pub async fn get_chat(&self, chat_id: i64) -> Result<Chat, ClientError> {
        self.call(
            "getChat",
            Some(&json!({ "chat_id": chat_id })),
            "Telegram getChat",
        )
        .await
    }
    pub async fn download_file(&self, file_id: &str) -> Result<DownloadedFile, ClientError> {
        let file = self.get_file(file_id).await?;
        let path = file.file_path.trim();
        if path.is_empty() || path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(ClientError::InvalidInput(
                "Telegram getFile returned an invalid file path".to_owned(),
            ));
        }
        let request = Request::new(
            Method::Get,
            format!(
                "{}/file/bot{}/{}",
                self.domain,
                self.token(),
                path.split('/')
                    .map(encode_path_segment)
                    .collect::<Vec<_>>()
                    .join("/")
            ),
        );
        let response = self.transport.execute(request).await?;
        if !(200..300).contains(&response.status) {
            return Err(ClientError::HttpStatus {
                status: response.status,
                operation: "Telegram file download".to_owned(),
            });
        }
        if response.body.len() > MAX_DOWNLOAD_BYTES {
            return Err(ClientError::InvalidInput(format!(
                "Telegram file exceeds {} MB",
                MAX_DOWNLOAD_BYTES / (1024 * 1024)
            )));
        }
        let content_type = response
            .header("Content-Type")
            .unwrap_or("application/octet-stream")
            .to_owned();
        Ok(DownloadedFile {
            data: response.body,
            content_type,
        })
    }
    pub async fn get_managed_bot_token(&self, user_id: i64) -> Result<String, ClientError> {
        let token: String = self
            .call(
                "getManagedBotToken",
                Some(&json!({ "user_id": user_id })),
                "Telegram getManagedBotToken",
            )
            .await?;
        if token.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "Telegram getManagedBotToken".to_owned(),
                field: "result".to_owned(),
            });
        }
        Ok(token)
    }

    /// Resolve a `managed_bot` update into the newly-created Bot credential.
    ///
    /// The Manager Bot update contains the creator and Bot identity, while the
    /// token is returned by a separate `getManagedBotToken` call.  Keeping the
    /// two-step operation here prevents the thin adapter from accidentally
    /// treating an update as complete before the credential is verified.
    pub async fn managed_bot_credential(
        &self,
        update: &Update,
    ) -> Result<ManagedBotCredential, ClientError> {
        let managed = update.managed_bot.as_ref().ok_or_else(|| {
            ClientError::InvalidInput("Telegram update is not a managed_bot update".to_owned())
        })?;
        let bot = &managed.bot;
        if !bot.is_bot || bot.id == 0 {
            return Err(ClientError::InvalidInput(
                "Telegram managed_bot update has an invalid Bot identity".to_owned(),
            ));
        }
        if managed.user.id == 0 {
            return Err(ClientError::InvalidInput(
                "Telegram managed_bot update has no creator identity".to_owned(),
            ));
        }
        let token = self.get_managed_bot_token(bot.id).await?.trim().to_owned();
        if token.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "Telegram getManagedBotToken".to_owned(),
                field: "result".to_owned(),
            });
        }
        Ok(ManagedBotCredential {
            account_id: bot.id.to_string(),
            creator_user_id: managed.user.id.to_string(),
            creator: managed.user.clone(),
            username: bot.username.trim().to_owned(),
            display_name: bot.display_name(),
            token,
        })
    }
    pub async fn set_my_name(&self, name: &str) -> Result<(), ClientError> {
        self.boolean_call(
            "setMyName",
            Some(&json!({ "name": name })),
            "Telegram setMyName",
        )
        .await
    }
    pub async fn set_my_description(&self, description: &str) -> Result<(), ClientError> {
        self.boolean_call(
            "setMyDescription",
            Some(&json!({ "description": description })),
            "Telegram setMyDescription",
        )
        .await
    }
    pub async fn set_my_short_description(&self, description: &str) -> Result<(), ClientError> {
        self.boolean_call(
            "setMyShortDescription",
            Some(&json!({ "short_description": description })),
            "Telegram setMyShortDescription",
        )
        .await
    }
    pub async fn set_my_profile_photo(
        &self,
        data: &[u8],
        file_name: &str,
    ) -> Result<(), ClientError> {
        if data.is_empty() {
            return Err(ClientError::InvalidInput(
                "Telegram profile photo cannot be empty".to_owned(),
            ));
        }
        let fields = vec![(
            "photo",
            br#"{"type":"static","photo":"attach://avatar"}"#.to_vec(),
        )];
        let (body, content_type) = multipart(&fields, "avatar", file_name, data, "image/jpeg")?;
        let _: bool = self
            .multipart_call(
                "setMyProfilePhoto",
                body,
                content_type,
                "Telegram setMyProfilePhoto",
            )
            .await?;
        Ok(())
    }
    pub fn managed_bot_registration(
        &self,
        manager: &User,
        username: &str,
        display_name: &str,
    ) -> Result<ManagedBotRegistration, ClientError> {
        if !manager.is_bot || manager.username.trim().is_empty() || !manager.can_manage_bots {
            return Err(ClientError::InvalidInput(
                "Telegram Manager Bot is not configured for Bot Management Mode".to_owned(),
            ));
        }
        self.managed_bot_registration_for_manager(&manager.username, username, display_name)
    }

    pub fn managed_bot_registration_for_manager(
        &self,
        manager_username: &str,
        username: &str,
        display_name: &str,
    ) -> Result<ManagedBotRegistration, ClientError> {
        if manager_username.trim().is_empty() {
            return Err(ClientError::InvalidInput(
                "Telegram Manager Bot username is required".to_owned(),
            ));
        }
        let username = username.trim();
        if username.len() < 5
            || username.len() > 32
            || !username
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || !username.to_ascii_lowercase().ends_with("bot")
        {
            return Err(ClientError::InvalidInput(
                "Telegram Bot username must be 5-32 letters, numbers or underscores and end in bot"
                    .to_owned(),
            ));
        }
        if display_name.trim().is_empty() || display_name.chars().count() > 64 {
            return Err(ClientError::InvalidInput(
                "Telegram Bot name must contain 1-64 characters".to_owned(),
            ));
        }
        Ok(ManagedBotRegistration {
            username: username.to_owned(),
            display_name: display_name.trim().to_owned(),
            manager_username: manager_username.trim().to_owned(),
        })
    }
    fn token(&self) -> &str {
        self.credentials.token.trim()
    }
    fn endpoint(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.domain, self.token(), method)
    }
    async fn call<R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        body: Option<&Value>,
        operation: &str,
    ) -> Result<R, ClientError> {
        let mut request = Request::new(Method::Post, self.endpoint(method));
        if let Some(body) = body {
            request = request.json(body)?;
        }
        let response = self.transport.execute(request).await?;
        decode_envelope(&response, operation)
    }
    async fn boolean_call(
        &self,
        method: &str,
        body: Option<&Value>,
        operation: &str,
    ) -> Result<(), ClientError> {
        let _: bool = self.call(method, body, operation).await?;
        Ok(())
    }
    async fn multipart_call<R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        body: Vec<u8>,
        content_type: String,
        operation: &str,
    ) -> Result<R, ClientError> {
        let request = Request::new(Method::Post, self.endpoint(method)).body(body, content_type);
        let response = self.transport.execute(request).await?;
        decode_envelope(&response, operation)
    }
}

fn message_ref(message: Message, operation: &str) -> Result<SentMessage, ClientError> {
    if message.message_id == 0 || message.chat.id == 0 {
        return Err(ClientError::MissingField {
            operation: operation.to_owned(),
            field: "message_id or chat.id".to_owned(),
        });
    }
    Ok(SentMessage {
        message_id: message.message_id,
        chat_id: message.chat.id,
    })
}

fn decode_envelope<R: serde::de::DeserializeOwned>(
    response: &Response,
    operation: &str,
) -> Result<R, ClientError> {
    // Telegram uses a JSON envelope for rate limits and other failures even
    // when the HTTP status is 429/5xx.  Decode that envelope before reducing a
    // non-2xx response to `HttpStatus`, so callers can honor retry_after while
    // still treating an HTML/proxy response as a transport-level failure.
    let envelope: ApiEnvelope<R> = match response.json() {
        Ok(value) => value,
        Err(_error) if !(200..300).contains(&response.status) => {
            return Err(ClientError::HttpStatus {
                status: response.status,
                operation: operation.to_owned(),
            });
        }
        Err(error) => return Err(error),
    };
    if !(200..300).contains(&response.status) && (envelope.ok || envelope.error_code == 0) {
        return Err(ClientError::HttpStatus {
            status: response.status,
            operation: operation.to_owned(),
        });
    }
    if !envelope.ok {
        return Err(ClientError::Api {
            code: envelope.error_code,
            message: if envelope.description.trim().is_empty() {
                "Telegram API rejected the request".to_owned()
            } else {
                envelope.description
            },
            operation: operation.to_owned(),
            retry_after: envelope
                .parameters
                .and_then(|value| (value.retry_after > 0).then_some(value.retry_after)),
        });
    }
    envelope.result.ok_or_else(|| ClientError::MissingField {
        operation: operation.to_owned(),
        field: "result".to_owned(),
    })
}

fn multipart(
    fields: &[(&str, Vec<u8>)],
    file_field: &str,
    file_name: &str,
    file: &[u8],
    content_type: &str,
) -> Result<(Vec<u8>, String), ClientError> {
    if file_field.trim().is_empty() || file_name.trim().is_empty() {
        return Err(ClientError::InvalidInput(
            "Telegram multipart file field and name are required".to_owned(),
        ));
    }
    let boundary = "lankide-telegram-boundary-4a3b";
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n",
                name
            )
            .as_bytes(),
        );
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n", sanitize_file_name(file_name)).as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok((body, format!("multipart/form-data; boundary={boundary}")))
}

fn sanitize_file_name(value: &str) -> String {
    let value = value.trim();
    let value = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .collect::<String>();
    if value.is_empty() {
        "attachment".to_owned()
    } else {
        value
    }
}
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{:02X}", byte)
            }
        })
        .collect()
}
