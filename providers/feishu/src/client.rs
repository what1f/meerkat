//! Feishu Open API client built on an injected transport.
//!
//! The public methods mirror the operations used by the old adapter, but they
//! only know Feishu paths and DTOs.  A Runtime adapter can therefore translate
//! its own `Message`, `Attachment` and `Agent` contexts without coupling this
//! crate to a particular Provider trait revision.

use crate::dto::{
    ApiResponse, BotInfoResponse, BotRecord, ChatRecord, DownloadedResource, FileResult,
    ImageResult, MessagePageResponse, MessageRecord, MessageResult, ReactionPage, ReactionResult,
    TaskCommentPageResponse, TaskCommentResult, TaskPageResponse, TaskRecord, TaskResult,
    TasklistPageResponse, TasklistRecord, TasklistResult, TokenResponse, UserRecord,
};
use crate::http::{ClientError, Method, Request, Response, Transport, encode_path_segment};
use crate::tasks::{
    COMMENTS_PATH, Comment, CreateTask, ListRequest, MemberIdentity, TASKLISTS_PATH, TASKS_PATH,
    TaskMemberInput, TaskPatch, TasklistMember,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const FEISHU_BASE_URL: &str = "https://open.feishu.cn";
pub const LARK_BASE_URL: &str = "https://open.larksuite.com";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Domain {
    Feishu,
    Lark,
    Custom(String),
}

impl Default for Domain {
    fn default() -> Self {
        Self::Feishu
    }
}

impl Domain {
    pub fn base_url(&self) -> &str {
        match self {
            Self::Feishu => FEISHU_BASE_URL,
            Self::Lark => LARK_BASE_URL,
            Self::Custom(value) => value.trim_end_matches('/'),
        }
    }
}

#[derive(Clone)]
pub struct Credentials {
    pub app_id: String,
    app_secret: String,
    user_access_token: Option<String>,
    tenant_access_token: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("app_id", &self.app_id)
            .field("app_secret", &"<redacted>")
            .field(
                "user_access_token",
                &self.user_access_token.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "tenant_access_token",
                &self.tenant_access_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Credentials {
    pub fn app(app_id: impl Into<String>, app_secret: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            app_secret: app_secret.into(),
            user_access_token: None,
            tenant_access_token: None,
        }
    }

    pub fn with_user_access_token(mut self, token: impl Into<String>) -> Self {
        self.user_access_token = Some(token.into());
        self
    }

    pub fn with_tenant_access_token(mut self, token: impl Into<String>) -> Self {
        self.tenant_access_token = Some(token.into());
        self
    }

    pub fn validate(&self) -> Result<(), ClientError> {
        if self.app_id.trim().is_empty() || self.app_secret.trim().is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu App ID and App Secret are required".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn app_secret(&self) -> &str {
        self.app_secret.trim()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Auth {
    Tenant,
    User,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentMessage {
    pub message_id: String,
    pub chat_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessage {
    pub receive_id: String,
    pub receive_id_type: String,
    pub message_type: String,
    pub content: String,
    pub idempotency_key: Option<String>,
}

impl SendMessage {
    pub fn markdown(
        receive_id: impl Into<String>,
        receive_id_type: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            receive_id: receive_id.into(),
            receive_id_type: receive_id_type.into(),
            message_type: "interactive".to_owned(),
            content: markdown_card_content(content),
            idempotency_key: None,
        }
    }

    pub fn text(
        receive_id: impl Into<String>,
        receive_id_type: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            receive_id: receive_id.into(),
            receive_id_type: receive_id_type.into(),
            message_type: "text".to_owned(),
            content: json!({ "text": content.into() }).to_string(),
            idempotency_key: None,
        }
    }

    pub fn file(
        receive_id: impl Into<String>,
        receive_id_type: impl Into<String>,
        file_key: impl Into<String>,
    ) -> Self {
        Self {
            receive_id: receive_id.into(),
            receive_id_type: receive_id_type.into(),
            message_type: "file".to_owned(),
            content: json!({ "file_key": file_key.into() }).to_string(),
            idempotency_key: None,
        }
    }

    pub fn image(
        receive_id: impl Into<String>,
        receive_id_type: impl Into<String>,
        image_key: impl Into<String>,
    ) -> Self {
        Self {
            receive_id: receive_id.into(),
            receive_id_type: receive_id_type.into(),
            message_type: "image".to_owned(),
            content: json!({ "image_key": image_key.into() }).to_string(),
            idempotency_key: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyMessage {
    pub message_id: String,
    pub message_type: String,
    pub content: String,
    pub idempotency_key: Option<String>,
    pub reply_in_thread: bool,
}

impl ReplyMessage {
    pub fn markdown(message_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            message_type: "interactive".to_owned(),
            content: markdown_card_content(content),
            idempotency_key: None,
            reply_in_thread: false,
        }
    }

    pub fn text(message_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            message_type: "text".to_owned(),
            content: json!({ "text": content.into() }).to_string(),
            idempotency_key: None,
            reply_in_thread: false,
        }
    }

    pub fn file(message_id: impl Into<String>, file_key: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            message_type: "file".to_owned(),
            content: json!({ "file_key": file_key.into() }).to_string(),
            idempotency_key: None,
            reply_in_thread: false,
        }
    }

    pub fn image(message_id: impl Into<String>, image_key: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            message_type: "image".to_owned(),
            content: json!({ "image_key": image_key.into() }).to_string(),
            idempotency_key: None,
            reply_in_thread: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadedResource {
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionItem {
    pub reaction_id: String,
    pub emoji: String,
    pub operator_id: String,
    pub operator_type: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_page_token: Option<String>,
    pub has_more: bool,
}

/// REST client.  `T` is normally a TLS HTTP implementation supplied by the
/// host; module E2E scenarios can inject a local peer without replacing this
/// client or its request/response handling.
pub struct Client<T> {
    transport: T,
    domain: Domain,
    credentials: Credentials,
    cached_tenant_token: Mutex<Option<CachedTenantToken>>,
}

impl<T> fmt::Debug for Client<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeishuClient")
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
            domain: Domain::default(),
            credentials,
            cached_tenant_token: Mutex::new(None),
        })
    }

    pub fn with_domain(mut self, domain: Domain) -> Self {
        self.domain = domain;
        self
    }

    pub fn domain(&self) -> &Domain {
        &self.domain
    }

    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    pub async fn verify_credentials(&self) -> Result<String, ClientError> {
        self.obtain_tenant_token(true).await
    }

    pub async fn tenant_token(&self) -> Result<String, ClientError> {
        self.obtain_tenant_token(false).await
    }

    pub async fn bot_info(&self, auth: Auth) -> Result<BotRecord, ClientError> {
        let operation = "get Feishu bot info";
        let request = self
            .authorized_request(Method::Get, "/open-apis/bot/v3/info", &[], auth)
            .await?;
        let response = self.execute_authorized(request, auth).await?;
        crate::http::validate_success(&response, operation)?;
        let envelope = response
            .json::<BotInfoEnvelope>()
            .map_err(|error| match error {
                ClientError::Decode(message) => {
                    ClientError::Decode(format!("{operation}: {message}"))
                }
                other => other,
            })?;
        if envelope.code != 0 {
            return Err(ClientError::Api {
                code: envelope.code,
                message: if envelope.msg.trim().is_empty() {
                    "Feishu API rejected the request".to_owned()
                } else {
                    envelope.msg
                },
                operation: operation.to_owned(),
            });
        }
        let bot = if envelope.bot.open_id.trim().is_empty() {
            envelope.data.map(|value| value.bot).unwrap_or_default()
        } else {
            envelope.bot
        };
        if bot.open_id.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: operation.to_owned(),
                field: "bot.open_id".to_owned(),
            });
        }
        Ok(bot)
    }

    pub async fn published_capabilities(
        &self,
        auth: Auth,
    ) -> Result<PublishedCapabilities, ClientError> {
        let path = format!(
            "/open-apis/application/v6/applications/{}/app_versions",
            encode_path_segment(self.credentials.app_id.trim())
        );
        let query = [
            ("lang".to_owned(), "zh_cn".to_owned()),
            ("page_size".to_owned(), "20".to_owned()),
        ];
        let response: ApiResponse<AppVersionPage> = self
            .json_request(
                Method::Get,
                &path,
                &query,
                None::<&Value>,
                auth,
                "inspect Feishu application versions",
            )
            .await?;
        let mut items = response.data.unwrap_or_default().items;
        items.retain(|item| item.status == 1 && item.publish_time.is_some());
        let Some(item) = items.into_iter().next() else {
            return Ok(PublishedCapabilities::default());
        };
        Ok(PublishedCapabilities {
            version: item.version,
            tenant_scopes: item
                .scopes
                .into_iter()
                .filter(|scope| scope.token_types.iter().any(|kind| kind == "tenant"))
                .map(|scope| scope.scope)
                .filter(|scope| !scope.trim().is_empty())
                .collect(),
            events: item
                .event_infos
                .into_iter()
                .map(|event| event.event_type)
                .filter(|event| !event.trim().is_empty())
                .collect(),
        })
    }

    pub async fn send_message(
        &self,
        message: &SendMessage,
        auth: Auth,
    ) -> Result<SentMessage, ClientError> {
        if message.receive_id.trim().is_empty()
            || message.receive_id_type.trim().is_empty()
            || message.message_type.trim().is_empty()
        {
            return Err(ClientError::InvalidInput(
                "Feishu receive identity and message type are required".to_owned(),
            ));
        }
        let mut body = json!({
            "receive_id": message.receive_id.trim(),
            "msg_type": message.message_type.trim(),
            "content": message.content,
        });
        if let Some(uuid) = message
            .idempotency_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["uuid"] = Value::String(uuid.trim().to_owned());
        }
        let query = [(
            "receive_id_type".to_owned(),
            message.receive_id_type.trim().to_owned(),
        )];
        let response: ApiResponse<MessageResult> = self
            .json_request(
                Method::Post,
                "/open-apis/im/v1/messages",
                &query,
                Some(&body),
                auth,
                "send Feishu message",
            )
            .await?;
        let result = response.data.ok_or_else(|| ClientError::MissingField {
            operation: "send Feishu message".to_owned(),
            field: "data".to_owned(),
        })?;
        if result.message_id.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "send Feishu message".to_owned(),
                field: "message_id".to_owned(),
            });
        }
        Ok(SentMessage {
            message_id: result.message_id,
            chat_id: result.chat_id,
        })
    }

    pub async fn reply_message(
        &self,
        message: &ReplyMessage,
        auth: Auth,
    ) -> Result<SentMessage, ClientError> {
        if message.message_id.trim().is_empty() || message.message_type.trim().is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu reply message ID and type are required".to_owned(),
            ));
        }
        let mut body =
            json!({ "msg_type": message.message_type.trim(), "content": message.content });
        if let Some(uuid) = message
            .idempotency_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            body["uuid"] = Value::String(uuid.trim().to_owned());
        }
        if message.reply_in_thread {
            body["reply_in_thread"] = Value::Bool(true);
        }
        let path = format!(
            "/open-apis/im/v1/messages/{}/reply",
            encode_path_segment(message.message_id.trim())
        );
        let response: ApiResponse<MessageResult> = self
            .json_request(
                Method::Post,
                &path,
                &[],
                Some(&body),
                auth,
                "reply to Feishu message",
            )
            .await?;
        let result = response.data.ok_or_else(|| ClientError::MissingField {
            operation: "reply to Feishu message".to_owned(),
            field: "data".to_owned(),
        })?;
        if result.message_id.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "reply to Feishu message".to_owned(),
                field: "message_id".to_owned(),
            });
        }
        Ok(SentMessage {
            message_id: result.message_id,
            chat_id: result.chat_id,
        })
    }

    pub async fn recall_message(&self, message_id: &str, auth: Auth) -> Result<(), ClientError> {
        let path = format!(
            "/open-apis/im/v1/messages/{}",
            encode_path_segment(message_id.trim())
        );
        self.empty_request(Method::Delete, &path, &[], auth, "recall Feishu message")
            .await
    }

    pub async fn get_message(
        &self,
        message_id: &str,
        auth: Auth,
    ) -> Result<MessageRecord, ClientError> {
        let path = format!(
            "/open-apis/im/v1/messages/{}",
            encode_path_segment(message_id.trim())
        );
        let response: ApiResponse<MessageRecord> = self
            .json_request(
                Method::Get,
                &path,
                &[],
                None::<&Value>,
                auth,
                "get Feishu message",
            )
            .await?;
        let mut message = response.data.ok_or_else(|| ClientError::MissingField {
            operation: "get Feishu message".to_owned(),
            field: "data".to_owned(),
        })?;
        if message.message_id.trim().is_empty() {
            message =
                message
                    .items
                    .into_iter()
                    .next()
                    .ok_or_else(|| ClientError::MissingField {
                        operation: "get Feishu message".to_owned(),
                        field: "message_id".to_owned(),
                    })?;
        }
        Ok(message)
    }

    pub async fn list_messages(
        &self,
        container_id: &str,
        container_id_type: &str,
        page_token: Option<&str>,
        page_size: usize,
        sort_type: &str,
        auth: Auth,
    ) -> Result<Page<MessageRecord>, ClientError> {
        if container_id.trim().is_empty() || container_id_type.trim().is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu message container identity is required".to_owned(),
            ));
        }
        let mut query = vec![
            (
                "container_id_type".to_owned(),
                container_id_type.trim().to_owned(),
            ),
            ("container_id".to_owned(), container_id.trim().to_owned()),
            (
                "page_size".to_owned(),
                bounded_page_size(page_size, 50, 50).to_string(),
            ),
            (
                "sort_type".to_owned(),
                if sort_type.trim().is_empty() {
                    "ByCreateTimeDesc".to_owned()
                } else {
                    sort_type.trim().to_owned()
                },
            ),
            (
                "card_msg_content_type".to_owned(),
                "raw_card_content".to_owned(),
            ),
        ];
        if let Some(token) = page_token.filter(|value| !value.trim().is_empty()) {
            query.push(("page_token".to_owned(), token.trim().to_owned()));
        }
        let response: ApiResponse<MessagePageResponse> = self
            .json_request(
                Method::Get,
                "/open-apis/im/v1/messages",
                &query,
                None::<&Value>,
                auth,
                "list Feishu messages",
            )
            .await?;
        let page = response.data.unwrap_or_default();
        Ok(Page {
            items: page.items,
            next_page_token: (!page.page_token.trim().is_empty()).then_some(page.page_token),
            has_more: page.has_more,
        })
    }

    pub async fn upload_image(
        &self,
        data: &[u8],
        file_name: &str,
        image_type: &str,
        auth: Auth,
    ) -> Result<UploadedResource, ClientError> {
        if data.is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu image cannot be empty".to_owned(),
            ));
        }
        let (body, content_type) = multipart(
            &[("image_type", image_type.as_bytes(), None)],
            "image",
            file_name,
            data,
            "image/*",
        )?;
        let request = self
            .authorized_request(Method::Post, "/open-apis/im/v1/images", &[], auth)
            .await?
            .body(body, content_type);
        let response = self.execute_authorized(request, auth).await?;
        let envelope: ApiResponse<ImageResult> =
            parse_api_response(&response, "upload Feishu image")?;
        let key = envelope
            .data
            .map(|value| value.image_key)
            .unwrap_or_default();
        if key.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "upload Feishu image".to_owned(),
                field: "image_key".to_owned(),
            });
        }
        Ok(UploadedResource { key })
    }

    pub async fn upload_file(
        &self,
        data: &[u8],
        file_name: &str,
        file_type: &str,
        duration_ms: Option<i64>,
        auth: Auth,
    ) -> Result<UploadedResource, ClientError> {
        if data.is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu file cannot be empty".to_owned(),
            ));
        }
        let duration = duration_ms.map(|value| value.to_string());
        let mut fields = vec![
            ("file_type", file_type.as_bytes(), None),
            ("file_name", file_name.as_bytes(), None),
        ];
        if let Some(value) = duration.as_deref() {
            fields.push(("duration", value.as_bytes(), None));
        }
        let (body, content_type) =
            multipart(&fields, "file", file_name, data, "application/octet-stream")?;
        let request = self
            .authorized_request(Method::Post, "/open-apis/im/v1/files", &[], auth)
            .await?
            .body(body, content_type);
        let response = self.execute_authorized(request, auth).await?;
        let envelope: ApiResponse<FileResult> =
            parse_api_response(&response, "upload Feishu file")?;
        let key = envelope
            .data
            .map(|value| value.file_key)
            .unwrap_or_default();
        if key.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "upload Feishu file".to_owned(),
                field: "file_key".to_owned(),
            });
        }
        Ok(UploadedResource { key })
    }

    pub async fn download_image(
        &self,
        image_key: &str,
        auth: Auth,
    ) -> Result<DownloadedResource, ClientError> {
        let path = format!(
            "/open-apis/im/v1/images/{}",
            encode_path_segment(image_key.trim())
        );
        self.download(&path, &[], auth, "download Feishu image")
            .await
    }

    pub async fn download_message_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
        auth: Auth,
    ) -> Result<DownloadedResource, ClientError> {
        let path = format!(
            "/open-apis/im/v1/messages/{}/resources/{}",
            encode_path_segment(message_id.trim()),
            encode_path_segment(file_key.trim())
        );
        let query = [("type".to_owned(), resource_type.trim().to_owned())];
        self.download(&path, &query, auth, "download Feishu message resource")
            .await
    }

    pub async fn download_public_url(&self, url: &str) -> Result<DownloadedResource, ClientError> {
        let url = url.trim();
        if !url.starts_with("https://") {
            return Err(ClientError::InvalidInput(
                "Feishu resource URL must use HTTPS".to_owned(),
            ));
        }
        let response = self
            .transport
            .execute(Request::new(Method::Get, url))
            .await?;
        if !(200..300).contains(&response.status) {
            return Err(ClientError::HttpStatus {
                status: response.status,
                operation: "download Feishu profile image".to_owned(),
            });
        }
        let content_type = response
            .header("Content-Type")
            .unwrap_or("image/jpeg")
            .to_owned();
        let file_name = response
            .header("Content-Disposition")
            .and_then(parse_file_name)
            .unwrap_or_default();
        Ok(DownloadedResource {
            data: response.body,
            content_type,
            file_name,
        })
    }

    pub async fn add_reaction(
        &self,
        message_id: &str,
        emoji: &str,
        auth: Auth,
    ) -> Result<String, ClientError> {
        let path = format!(
            "/open-apis/im/v1/messages/{}/reactions",
            encode_path_segment(message_id.trim())
        );
        let body = json!({ "reaction_type": { "emoji_type": emoji.trim() } });
        let response: ApiResponse<ReactionResult> = self
            .json_request(
                Method::Post,
                &path,
                &[],
                Some(&body),
                auth,
                "add Feishu reaction",
            )
            .await?;
        let id = response
            .data
            .map(|value| value.reaction_id)
            .unwrap_or_default();
        if id.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "add Feishu reaction".to_owned(),
                field: "reaction_id".to_owned(),
            });
        }
        Ok(id)
    }

    pub async fn remove_reaction(
        &self,
        message_id: &str,
        reaction_id: &str,
        auth: Auth,
    ) -> Result<(), ClientError> {
        let path = format!(
            "/open-apis/im/v1/messages/{}/reactions/{}",
            encode_path_segment(message_id.trim()),
            encode_path_segment(reaction_id.trim())
        );
        self.empty_request(Method::Delete, &path, &[], auth, "remove Feishu reaction")
            .await
    }

    pub async fn list_reactions(
        &self,
        message_id: &str,
        page_token: Option<&str>,
        page_size: usize,
        auth: Auth,
    ) -> Result<Page<ReactionItem>, ClientError> {
        let path = format!(
            "/open-apis/im/v1/messages/{}/reactions",
            encode_path_segment(message_id.trim())
        );
        let mut query = vec![
            ("user_id_type".to_owned(), "open_id".to_owned()),
            (
                "page_size".to_owned(),
                bounded_page_size(page_size, 50, 50).to_string(),
            ),
        ];
        if let Some(token) = page_token.filter(|value| !value.trim().is_empty()) {
            query.push(("page_token".to_owned(), token.trim().to_owned()));
        }
        let response: ApiResponse<ReactionPage> = self
            .json_request(
                Method::Get,
                &path,
                &query,
                None::<&Value>,
                auth,
                "list Feishu reactions",
            )
            .await?;
        let page = response.data.unwrap_or_default();
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(|item| ReactionItem {
                    reaction_id: item.reaction_id,
                    emoji: item.reaction_type.emoji_type,
                    operator_id: item.operator.operator_id,
                    operator_type: item.operator.operator_type,
                })
                .collect(),
            next_page_token: (!page.page_token.trim().is_empty()).then_some(page.page_token),
            has_more: page.has_more,
        })
    }

    pub async fn get_chat(&self, chat_id: &str, auth: Auth) -> Result<ChatRecord, ClientError> {
        let path = format!(
            "/open-apis/im/v1/chats/{}",
            encode_path_segment(chat_id.trim())
        );
        let response: ApiResponse<ChatRecord> = self
            .json_request(
                Method::Get,
                &path,
                &[],
                None::<&Value>,
                auth,
                "get Feishu chat",
            )
            .await?;
        response.data.ok_or_else(|| ClientError::MissingField {
            operation: "get Feishu chat".to_owned(),
            field: "data".to_owned(),
        })
    }

    /// The enterprise of this App's tenant token (not a developer-session tenant ID).
    pub(crate) async fn tenant_key(&self) -> Result<String, ClientError> {
        let response: ApiResponse<Value> = self
            .json_request(
                Method::Get,
                "/open-apis/tenant/v2/tenant/query",
                &[],
                None,
                Auth::Tenant,
                "tenant_info",
            )
            .await?;
        response
            .data
            .and_then(|data| {
                data["tenant"]["tenant_key"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
            })
            .ok_or_else(|| ClientError::MissingField {
                operation: "tenant_info".into(),
                field: "tenant_key".into(),
            })
    }

    pub(crate) async fn application_creator(&self, id_type: &str) -> Result<String, ClientError> {
        let response: ApiResponse<Value> = self
            .json_request(
                Method::Get,
                "/open-apis/application/v6/applications/me",
                &[
                    ("lang".into(), "zh_cn".into()),
                    ("user_id_type".into(), id_type.into()),
                ],
                None,
                Auth::Tenant,
                "application_creator",
            )
            .await?;
        response
            .data
            .and_then(|data| {
                data["app"]["creator_id"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
            })
            .ok_or_else(|| ClientError::MissingField {
                operation: "application_creator".into(),
                field: "creator_id".into(),
            })
    }

    /// Read only the creator's signature through the official directory API.
    /// Missing fields remain None; an explicit empty string clears the signature.
    pub(crate) async fn human_signature(
        &self,
        user_id: &str,
        id_type: &str,
    ) -> Result<Option<String>, ClientError> {
        let response: ApiResponse<Value> = self
            .json_request(
                Method::Post,
                "/open-apis/directory/v1/employees/mget",
                &[("employee_id_type".into(), id_type.into())],
                Some(&serde_json::json!({
                    "employee_ids": [user_id],
                    "required_fields": ["base_info.description"],
                })),
                Auth::Tenant,
                "human_signature",
            )
            .await?;
        Ok(response.data.and_then(|data| {
            data["employees"].as_array()?.first()?["base_info"]["description"]
                .as_str()
                .map(str::to_owned)
        }))
    }

    pub async fn get_user(&self, open_id: &str, auth: Auth) -> Result<UserRecord, ClientError> {
        self.get_user_by_id(open_id, "open_id", auth).await
    }

    pub async fn get_user_by_id(
        &self,
        user_id: &str,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<UserRecord, ClientError> {
        let path = format!(
            "/open-apis/contact/v3/users/{}",
            encode_path_segment(user_id.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let response: ApiResponse<UserRecordEnvelope> = self
            .json_request(
                Method::Get,
                &path,
                &query,
                None::<&Value>,
                auth,
                "get Feishu user",
            )
            .await?;
        response
            .data
            .map(|value| value.user)
            .ok_or_else(|| ClientError::MissingField {
                operation: "get Feishu user".to_owned(),
                field: "user".to_owned(),
            })
    }

    pub async fn subscribe_task_events(
        &self,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<(), ClientError> {
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        self.empty_request(
            Method::Post,
            "/open-apis/task/v2/task_v2/task_subscription",
            &query,
            auth,
            "subscribe Feishu task events",
        )
        .await
    }

    pub async fn list_tasks(
        &self,
        request: &ListRequest,
        auth: Auth,
    ) -> Result<Page<TaskRecord>, ClientError> {
        let query = request.query();
        let response: ApiResponse<TaskPageResponse> = self
            .json_request(
                Method::Get,
                TASKS_PATH,
                &query,
                None::<&Value>,
                auth,
                "list Feishu tasks",
            )
            .await?;
        let page = response.data.unwrap_or_default();
        Ok(Page {
            items: page.items,
            next_page_token: (!page.page_token.trim().is_empty()).then_some(page.page_token),
            has_more: page.has_more,
        })
    }

    pub async fn get_task(
        &self,
        task_guid: &str,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TaskRecord, ClientError> {
        let path = format!("{TASKS_PATH}/{}", encode_path_segment(task_guid.trim()));
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let response: ApiResponse<TaskResult> = self
            .json_request(
                Method::Get,
                &path,
                &query,
                None::<&Value>,
                auth,
                "get Feishu task",
            )
            .await?;
        response
            .data
            .and_then(|value| value.task)
            .ok_or_else(|| ClientError::MissingField {
                operation: "get Feishu task".to_owned(),
                field: "task".to_owned(),
            })
    }

    pub async fn create_task(
        &self,
        task: &CreateTask,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TaskRecord, ClientError> {
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = task.to_json();
        let response: ApiResponse<TaskResult> = self
            .json_request(
                Method::Post,
                TASKS_PATH,
                &query,
                Some(&body),
                auth,
                "create Feishu task",
            )
            .await?;
        response
            .data
            .and_then(|value| value.task)
            .ok_or_else(|| ClientError::MissingField {
                operation: "create Feishu task".to_owned(),
                field: "task".to_owned(),
            })
    }

    pub async fn update_task(
        &self,
        task_guid: &str,
        patch: &TaskPatch,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TaskRecord, ClientError> {
        if patch.is_empty() {
            return self.get_task(task_guid, user_id_type, auth).await;
        }
        let path = format!("{TASKS_PATH}/{}", encode_path_segment(task_guid.trim()));
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = patch.to_json();
        let response: ApiResponse<TaskResult> = self
            .json_request(
                Method::Patch,
                &path,
                &query,
                Some(&body),
                auth,
                "update Feishu task",
            )
            .await?;
        response
            .data
            .and_then(|value| value.task)
            .ok_or_else(|| ClientError::MissingField {
                operation: "update Feishu task".to_owned(),
                field: "task".to_owned(),
            })
    }

    pub async fn delete_task(
        &self,
        task_guid: &str,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<(), ClientError> {
        let path = format!("{TASKS_PATH}/{}", encode_path_segment(task_guid.trim()));
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        self.empty_request(Method::Delete, &path, &query, auth, "delete Feishu task")
            .await
    }

    pub async fn add_task_members(
        &self,
        task_guid: &str,
        members: &[TaskMemberInput],
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TaskRecord, ClientError> {
        let values = members
            .iter()
            .filter_map(TaskMemberInput::as_json)
            .collect::<Vec<_>>();
        if task_guid.trim().is_empty() || values.is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu task GUID and members are required".to_owned(),
            ));
        }
        let path = format!(
            "{TASKS_PATH}/{}/add_members",
            encode_path_segment(task_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = json!({ "members": values });
        let response: ApiResponse<TaskResult> = self
            .json_request(
                Method::Post,
                &path,
                &query,
                Some(&body),
                auth,
                "add Feishu task members",
            )
            .await?;
        response
            .data
            .and_then(|value| value.task)
            .ok_or_else(|| ClientError::MissingField {
                operation: "add Feishu task members".to_owned(),
                field: "task".to_owned(),
            })
    }

    pub async fn remove_task_members(
        &self,
        task_guid: &str,
        members: &[TaskMemberInput],
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TaskRecord, ClientError> {
        let values = members
            .iter()
            .filter_map(TaskMemberInput::as_json)
            .collect::<Vec<_>>();
        if task_guid.trim().is_empty() || values.is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu task GUID and members are required".to_owned(),
            ));
        }
        let path = format!(
            "{TASKS_PATH}/{}/remove_members",
            encode_path_segment(task_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = json!({ "members": values });
        let response: ApiResponse<TaskResult> = self
            .json_request(
                Method::Post,
                &path,
                &query,
                Some(&body),
                auth,
                "remove Feishu task members",
            )
            .await?;
        response
            .data
            .and_then(|value| value.task)
            .ok_or_else(|| ClientError::MissingField {
                operation: "remove Feishu task members".to_owned(),
                field: "task".to_owned(),
            })
    }

    pub async fn create_task_comment(
        &self,
        comment: &Comment,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<String, ClientError> {
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        // Task v2 exposes plain comments; native @ markup is documented for
        // the rich-comment endpoint. Both APIs use the same task GUID.
        let path = format!(
            "/open-apis/task/v1/tasks/{}/comments",
            encode_path_segment(&comment.task_guid)
        );
        let body = serde_json::json!({"rich_content":comment.content,
            "parent_id":comment.reply_to_comment_id.as_deref().unwrap_or("0")});
        let response: ApiResponse<TaskCommentResult> = self
            .json_request(
                Method::Post,
                &path,
                &query,
                Some(&body),
                auth,
                "create Feishu task comment",
            )
            .await?;
        response
            .data
            .and_then(|value| value.comment)
            .map(|value| value.comment_id)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ClientError::MissingField {
                operation: "create Feishu task comment".to_owned(),
                field: "comment.id".to_owned(),
            })
    }

    pub async fn get_task_comment(
        &self,
        comment_id: &str,
        auth: Auth,
    ) -> Result<crate::dto::TaskComment, ClientError> {
        let path = format!("{COMMENTS_PATH}/{}", encode_path_segment(comment_id));
        let response: ApiResponse<TaskCommentResult> = self
            .json_request(
                Method::Get,
                &path,
                &[("user_id_type".to_owned(), "open_id".to_owned())],
                None::<&Value>,
                auth,
                "get Feishu task comment",
            )
            .await?;
        response
            .data
            .and_then(|data| data.comment)
            .ok_or_else(|| ClientError::MissingField {
                operation: "get Feishu task comment".to_owned(),
                field: "comment".to_owned(),
            })
    }

    pub async fn list_task_comments(
        &self,
        task_guid: &str,
        page_token: Option<&str>,
        page_size: usize,
        auth: Auth,
    ) -> Result<Page<crate::dto::TaskComment>, ClientError> {
        let mut query = vec![
            ("resource_type".to_owned(), "task".to_owned()),
            ("resource_id".to_owned(), task_guid.trim().to_owned()),
            ("direction".to_owned(), "asc".to_owned()),
            ("user_id_type".to_owned(), "open_id".to_owned()),
            (
                "page_size".to_owned(),
                bounded_page_size(page_size, 50, 100).to_string(),
            ),
        ];
        if let Some(token) = page_token.filter(|value| !value.trim().is_empty()) {
            query.push(("page_token".to_owned(), token.trim().to_owned()));
        }
        let response: ApiResponse<TaskCommentPageResponse> = self
            .json_request(
                Method::Get,
                COMMENTS_PATH,
                &query,
                None::<&Value>,
                auth,
                "list Feishu task comments",
            )
            .await?;
        let page = response.data.unwrap_or_default();
        Ok(Page {
            items: page.items,
            next_page_token: (!page.page_token.trim().is_empty()).then_some(page.page_token),
            has_more: page.has_more,
        })
    }

    pub async fn list_tasklists(
        &self,
        page_token: Option<&str>,
        page_size: usize,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<Page<TasklistRecord>, ClientError> {
        let mut query = vec![
            (
                "page_size".to_owned(),
                bounded_page_size(page_size, 50, 100).to_string(),
            ),
            (
                "user_id_type".to_owned(),
                normalized_user_id_type(user_id_type),
            ),
        ];
        if let Some(token) = page_token.filter(|value| !value.trim().is_empty()) {
            query.push(("page_token".to_owned(), token.trim().to_owned()));
        }
        let response: ApiResponse<TasklistPageResponse> = self
            .json_request(
                Method::Get,
                TASKLISTS_PATH,
                &query,
                None::<&Value>,
                auth,
                "list Feishu tasklists",
            )
            .await?;
        let page = response.data.unwrap_or_default();
        Ok(Page {
            items: page.items,
            next_page_token: (!page.page_token.trim().is_empty()).then_some(page.page_token),
            has_more: page.has_more,
        })
    }

    pub async fn get_tasklist(
        &self,
        tasklist_guid: &str,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TasklistRecord, ClientError> {
        let path = format!(
            "{TASKLISTS_PATH}/{}",
            encode_path_segment(tasklist_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let response: ApiResponse<TasklistResult> = self
            .json_request(
                Method::Get,
                &path,
                &query,
                None::<&Value>,
                auth,
                "get Feishu tasklist",
            )
            .await?;
        response
            .data
            .and_then(|value| value.tasklist)
            .ok_or_else(|| ClientError::MissingField {
                operation: "get Feishu tasklist".to_owned(),
                field: "tasklist".to_owned(),
            })
    }

    pub async fn create_tasklist(
        &self,
        name: &str,
        members: &[TasklistMember],
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TasklistRecord, ClientError> {
        if name.trim().is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu tasklist name is required".to_owned(),
            ));
        }
        let mut body = json!({ "name": name.trim() });
        let values = members
            .iter()
            .filter_map(TasklistMember::as_json)
            .collect::<Vec<_>>();
        if !values.is_empty() {
            body["members"] = Value::Array(values);
        }
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let response: ApiResponse<TasklistResult> = self
            .json_request(
                Method::Post,
                TASKLISTS_PATH,
                &query,
                Some(&body),
                auth,
                "create Feishu tasklist",
            )
            .await?;
        response
            .data
            .and_then(|value| value.tasklist)
            .ok_or_else(|| ClientError::MissingField {
                operation: "create Feishu tasklist".to_owned(),
                field: "tasklist".to_owned(),
            })
    }

    pub async fn list_tasklist_tasks(
        &self,
        tasklist_guid: &str,
        request: &ListRequest,
        auth: Auth,
    ) -> Result<Page<TaskRecord>, ClientError> {
        let path = format!(
            "{TASKLISTS_PATH}/{}/tasks",
            encode_path_segment(tasklist_guid.trim())
        );
        let query = request.query();
        let response: ApiResponse<TaskPageResponse> = self
            .json_request(
                Method::Get,
                &path,
                &query,
                None::<&Value>,
                auth,
                "list Feishu tasklist tasks",
            )
            .await?;
        let page = response.data.unwrap_or_default();
        Ok(Page {
            items: page.items,
            next_page_token: (!page.page_token.trim().is_empty()).then_some(page.page_token),
            has_more: page.has_more,
        })
    }

    pub async fn patch_tasklist(
        &self,
        tasklist_guid: &str,
        name: Option<&str>,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TasklistRecord, ClientError> {
        let Some(name) = name.filter(|value| !value.trim().is_empty()) else {
            return self.get_tasklist(tasklist_guid, user_id_type, auth).await;
        };
        let path = format!(
            "{TASKLISTS_PATH}/{}",
            encode_path_segment(tasklist_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = json!({
            "tasklist": { "name": name.trim() },
            "update_fields": ["name"]
        });
        let response: ApiResponse<TasklistResult> = self
            .json_request(
                Method::Patch,
                &path,
                &query,
                Some(&body),
                auth,
                "patch Feishu tasklist",
            )
            .await?;
        response
            .data
            .and_then(|value| value.tasklist)
            .ok_or_else(|| ClientError::MissingField {
                operation: "patch Feishu tasklist".to_owned(),
                field: "tasklist".to_owned(),
            })
    }

    /// Hands the tasklist to `owner`; the previous owner stays as an editor.
    pub async fn transfer_tasklist_owner(
        &self,
        tasklist_guid: &str,
        owner: &MemberIdentity,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TasklistRecord, ClientError> {
        let path = format!(
            "{TASKLISTS_PATH}/{}",
            encode_path_segment(tasklist_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = json!({
            "tasklist": {
                "owner": { "id": owner.external_id.trim(), "type": owner.identity_type.trim() }
            },
            "update_fields": ["owner"],
            "origin_owner_to_role": "editor"
        });
        let response: ApiResponse<TasklistResult> = self
            .json_request(
                Method::Patch,
                &path,
                &query,
                Some(&body),
                auth,
                "transfer Feishu tasklist owner",
            )
            .await?;
        response
            .data
            .and_then(|value| value.tasklist)
            .ok_or_else(|| ClientError::MissingField {
                operation: "transfer Feishu tasklist owner".to_owned(),
                field: "tasklist".to_owned(),
            })
    }

    pub async fn delete_tasklist(
        &self,
        tasklist_guid: &str,
        user_id_type: &str,
        auth: Auth,
    ) -> Result<(), ClientError> {
        let path = format!(
            "{TASKLISTS_PATH}/{}",
            encode_path_segment(tasklist_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        self.empty_request(
            Method::Delete,
            &path,
            &query,
            auth,
            "delete Feishu tasklist",
        )
        .await
    }

    pub async fn add_tasklist_members(
        &self,
        tasklist_guid: &str,
        members: &[TasklistMember],
        user_id_type: &str,
        auth: Auth,
    ) -> Result<TasklistRecord, ClientError> {
        let values = members
            .iter()
            .filter_map(TasklistMember::as_json)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu tasklist members cannot be empty".to_owned(),
            ));
        }
        let path = format!(
            "{TASKLISTS_PATH}/{}/add_members",
            encode_path_segment(tasklist_guid.trim())
        );
        let query = [(
            "user_id_type".to_owned(),
            normalized_user_id_type(user_id_type),
        )];
        let body = json!({ "members": values });
        let response: ApiResponse<TasklistResult> = self
            .json_request(
                Method::Post,
                &path,
                &query,
                Some(&body),
                auth,
                "add Feishu tasklist members",
            )
            .await?;
        response
            .data
            .and_then(|value| value.tasklist)
            .ok_or_else(|| ClientError::MissingField {
                operation: "add Feishu tasklist members".to_owned(),
                field: "tasklist".to_owned(),
            })
    }

    /// Escape hatch for Task v2 operations that are provider-defined.  The
    /// request still receives normal Feishu auth/error handling; the Runtime
    /// never needs to import Feishu Task SDK types.
    pub async fn task_json(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
        auth: Auth,
        operation: &str,
    ) -> Result<Value, ClientError> {
        let response: ApiResponse<Value> = self
            .json_request(method, path, query, body, auth, operation)
            .await?;
        Ok(response.data.unwrap_or(Value::Null))
    }

    pub async fn upload_app_avatar(
        &self,
        data: &[u8],
        file_name: &str,
        auth: Auth,
    ) -> Result<String, ClientError> {
        if data.is_empty() {
            return Err(ClientError::InvalidInput(
                "Feishu app avatar cannot be empty".to_owned(),
            ));
        }
        let (body, content_type) = multipart(&[], "avatar", file_name, data, "image/*")?;
        let request = self
            .authorized_request(
                Method::Post,
                "/open-apis/application/v7/app_avatar/upload",
                &[],
                auth,
            )
            .await?
            .body(body, content_type);
        let response = self.execute_authorized(request, auth).await?;
        let envelope: ApiResponse<AvatarUploadResult> =
            parse_api_response(&response, "upload Feishu app avatar")?;
        envelope
            .data
            .map(|value| value.url)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ClientError::MissingField {
                operation: "upload Feishu app avatar".to_owned(),
                field: "url".to_owned(),
            })
    }

    pub async fn update_application_profile(
        &self,
        display_name: &str,
        description: &str,
        avatar_url: Option<&str>,
        language: &str,
        auth: Auth,
    ) -> Result<(), ClientError> {
        let mut locale = serde_json::Map::new();
        locale.insert(
            "i18n_key".to_owned(),
            Value::String(language.trim().to_owned()),
        );
        // Empty profile fields mean "leave unchanged". Feishu rejects them
        // when present, including on an avatar-only update.
        if !display_name.trim().is_empty() {
            locale.insert("name".to_owned(), Value::String(display_name.to_owned()));
        }
        if !description.trim().is_empty() {
            locale.insert(
                "description".to_owned(),
                Value::String(description.to_owned()),
            );
        }
        // The Application v7 API names this list `i18ns` (the generated Go
        // SDK uses AppI18ns).  Keeping the wire spelling here avoids a
        // profile update that is accepted at HTTP level but silently ignored.
        let mut body = json!({});
        if locale.len() > 1 {
            body["i18ns"] = json!([Value::Object(locale)]);
        }
        if let Some(url) = avatar_url.filter(|value| !value.trim().is_empty()) {
            body["avatar_url"] = Value::String(url.to_owned());
        }
        let path = format!(
            "/open-apis/application/v7/applications/{}/base",
            encode_path_segment(self.credentials.app_id.trim())
        );
        self.empty_json_request(
            Method::Patch,
            &path,
            &[],
            &body,
            auth,
            "update Feishu app profile",
        )
        .await
    }

    pub async fn publish_application(
        &self,
        remark: &str,
        changelog: &str,
        auth: Auth,
    ) -> Result<PublishedApplication, ClientError> {
        let path = format!(
            "/open-apis/application/v7/applications/{}/publish",
            encode_path_segment(self.credentials.app_id.trim())
        );
        let body = json!({ "mobile_default_ability": "bot", "pc_default_ability": "bot", "remark": remark, "changelog": changelog });
        let response: ApiResponse<PublishedApplication> = self
            .json_request(
                Method::Post,
                &path,
                &[],
                Some(&body),
                auth,
                "publish Feishu application",
            )
            .await?;
        response.data.ok_or_else(|| ClientError::MissingField {
            operation: "publish Feishu application".to_owned(),
            field: "data".to_owned(),
        })
    }

    pub async fn update_authorization(
        &self,
        scopes: &[String],
        events: &[String],
        auth: Auth,
    ) -> Result<PublishedApplication, ClientError> {
        let scopes = unique_non_empty(scopes)
            .into_iter()
            .map(|scope| json!({ "scope_name": scope, "token_type": "tenant" }))
            .collect::<Vec<_>>();
        let events = unique_non_empty(events)
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>();
        let mut body = serde_json::Map::new();
        if !scopes.is_empty() {
            body.insert("scope".to_owned(), json!({ "add_scopes": scopes }));
        }
        if !events.is_empty() {
            body.insert(
                "event".to_owned(),
                json!({ "subscription_type": "websocket", "add_events": events }),
            );
        }
        if body.is_empty() {
            return Ok(PublishedApplication::default());
        }
        let path = format!(
            "/open-apis/application/v7/applications/{}/config",
            encode_path_segment(self.credentials.app_id.trim())
        );
        self.empty_json_request(
            Method::Patch,
            &path,
            &[],
            &Value::Object(body),
            auth,
            "update Feishu app authorization",
        )
        .await?;
        self.publish_application(
            "Meerkat permission update",
            "Update Agent permissions or events",
            auth,
        )
        .await
    }

    async fn obtain_tenant_token(&self, force_refresh: bool) -> Result<String, ClientError> {
        if !force_refresh {
            if let Some(value) = self
                .credentials
                .tenant_access_token
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                return Ok(value.trim().to_owned());
            }
            if let Ok(guard) = self.cached_tenant_token.lock() {
                if let Some(value) = guard
                    .as_ref()
                    .filter(|value| value.expires_at > Instant::now())
                {
                    return Ok(value.value.clone());
                }
            }
        }
        let body = json!({ "app_id": self.credentials.app_id.trim(), "app_secret": self.credentials.app_secret.trim() });
        let request = Request::new(
            Method::Post,
            format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                self.domain.base_url()
            ),
        )
        .json(&body)?;
        let response = self.transport.execute(request).await?;
        crate::http::validate_success(&response, "obtain Feishu tenant token")?;
        let envelope: TokenEnvelope = response.json().map_err(|error| match error {
            ClientError::Decode(message) => {
                ClientError::Decode(format!("obtain Feishu tenant token: {message}"))
            }
            other => other,
        })?;
        if envelope.code != 0 {
            return Err(ClientError::Api {
                code: envelope.code,
                message: envelope.msg,
                operation: "obtain Feishu tenant token".to_owned(),
            });
        }
        let token = if !envelope.tenant_access_token.trim().is_empty() {
            envelope.tenant_access_token
        } else {
            envelope
                .data
                .as_ref()
                .map(|value| value.tenant_access_token.clone())
                .unwrap_or_default()
        };
        if token.trim().is_empty() {
            return Err(ClientError::MissingField {
                operation: "obtain Feishu tenant token".to_owned(),
                field: "tenant_access_token".to_owned(),
            });
        }
        let expire = if envelope.expire > 0 {
            envelope.expire
        } else {
            envelope
                .data
                .as_ref()
                .map(|value| value.expire)
                .unwrap_or(0)
        };
        let ttl_seconds = if expire > 60 {
            expire.saturating_sub(60) as u64
        } else if expire > 0 {
            expire as u64
        } else {
            // Feishu normally supplies `expire`; keep an upper bound when a
            // compatible deployment omits it instead of caching forever.
            3_600
        };
        if let Ok(mut guard) = self.cached_tenant_token.lock() {
            *guard = Some(CachedTenantToken {
                value: token.trim().to_owned(),
                expires_at: Instant::now() + Duration::from_secs(ttl_seconds),
            });
        }
        Ok(token.trim().to_owned())
    }

    async fn auth_token(&self, auth: Auth) -> Result<String, ClientError> {
        match auth {
            Auth::Tenant => self.tenant_token().await,
            Auth::User => self
                .credentials
                .user_access_token
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ClientError::InvalidInput(
                        "Feishu user access token is not configured".to_owned(),
                    )
                }),
        }
    }

    async fn authorized_request(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        auth: Auth,
    ) -> Result<Request, ClientError> {
        let token = self.auth_token(auth).await?;
        let mut request = Request::new(method, format!("{}{}", self.domain.base_url(), path))
            .header("Authorization", format!("Bearer {token}"));
        for (key, value) in query {
            request = request.query(key, value);
        }
        Ok(request)
    }

    async fn execute_authorized(
        &self,
        request: Request,
        auth: Auth,
    ) -> Result<crate::http::Response, ClientError> {
        let retry = request.clone();
        let response = self.transport.execute(request).await?;
        if response.status != 401 || auth != Auth::Tenant {
            return Ok(response);
        }
        let token = self.obtain_tenant_token(true).await?;
        let mut retry = retry;
        retry
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        retry
            .headers
            .push(("Authorization".into(), format!("Bearer {token}")));
        Ok(self.transport.execute(retry).await?)
    }

    async fn json_request<R: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
        auth: Auth,
        operation: &str,
    ) -> Result<ApiResponse<R>, ClientError> {
        let mut request = self.authorized_request(method, path, query, auth).await?;
        if let Some(body) = body {
            request = request.json(body)?;
        }
        let response = self.execute_authorized(request, auth).await?;
        parse_api_response(&response, operation)
    }

    async fn empty_json_request(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: &Value,
        auth: Auth,
        operation: &str,
    ) -> Result<(), ClientError> {
        let request = self
            .authorized_request(method, path, query, auth)
            .await?
            .json(body)?;
        let response = self.execute_authorized(request, auth).await?;
        if (200..300).contains(&response.status) && response.body.is_empty() {
            return Ok(());
        }
        let _: ApiResponse<Value> = parse_api_response(&response, operation)?;
        Ok(())
    }

    async fn empty_request(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        auth: Auth,
        operation: &str,
    ) -> Result<(), ClientError> {
        let request = self.authorized_request(method, path, query, auth).await?;
        let response = self.execute_authorized(request, auth).await?;
        if (200..300).contains(&response.status) && response.body.is_empty() {
            return Ok(());
        }
        let _: ApiResponse<Value> = parse_api_response(&response, operation)?;
        Ok(())
    }

    async fn download(
        &self,
        path: &str,
        query: &[(String, String)],
        auth: Auth,
        operation: &str,
    ) -> Result<DownloadedResource, ClientError> {
        let request = self
            .authorized_request(Method::Get, path, query, auth)
            .await?;
        let response = self.execute_authorized(request, auth).await?;
        if !(200..300).contains(&response.status) {
            return Err(ClientError::HttpStatus {
                status: response.status,
                operation: operation.to_owned(),
            });
        }
        let content_type = response
            .header("Content-Type")
            .unwrap_or("application/octet-stream")
            .to_owned();
        let file_name = response
            .header("Content-Disposition")
            .and_then(parse_file_name)
            .unwrap_or_default();
        Ok(DownloadedResource {
            data: response.body,
            content_type,
            file_name,
        })
    }
}

#[derive(Clone, Deserialize, Serialize, Default, Eq, PartialEq)]
struct TokenEnvelope {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    tenant_access_token: String,
    #[serde(default)]
    expire: i64,
    #[serde(default)]
    data: Option<TokenResponse>,
}

impl fmt::Debug for TokenEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TokenEnvelope")
            .field("code", &self.code)
            .field("msg", &self.msg)
            .field("tenant_access_token", &"<redacted>")
            .field("expire", &self.expire)
            .field("data", &self.data)
            .finish()
    }
}

struct CachedTenantToken {
    value: String,
    expires_at: Instant,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
struct UserRecordEnvelope {
    #[serde(default)]
    user: UserRecord,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
struct BotInfoEnvelope {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    bot: BotRecord,
    #[serde(default)]
    data: Option<BotInfoResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
struct AvatarUploadResult {
    #[serde(default)]
    url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct PublishedApplication {
    #[serde(default)]
    pub version_id: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, Eq, PartialEq)]
pub struct PublishedCapabilities {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tenant_scopes: Vec<String>,
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AppVersionPage {
    #[serde(default)]
    items: Vec<AppVersion>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AppVersion {
    #[serde(default)]
    version: String,
    #[serde(default)]
    status: i64,
    #[serde(default)]
    publish_time: Option<Value>,
    #[serde(default)]
    event_infos: Vec<AppVersionEvent>,
    #[serde(default)]
    scopes: Vec<AppVersionScope>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AppVersionEvent {
    #[serde(default)]
    event_type: String,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct AppVersionScope {
    #[serde(default)]
    scope: String,
    #[serde(default)]
    token_types: Vec<String>,
}

fn parse_api_response<R: serde::de::DeserializeOwned>(
    response: &Response,
    operation: &str,
) -> Result<ApiResponse<R>, ClientError> {
    crate::http::validate_success(response, operation)?;
    let envelope = response
        .json::<ApiResponse<R>>()
        .map_err(|error| match error {
            ClientError::Decode(message) => ClientError::Decode(format!("{operation}: {message}")),
            other => other,
        })?;
    if envelope.code != 0 {
        return Err(ClientError::Api {
            code: envelope.code,
            message: if envelope.msg.trim().is_empty() {
                "Feishu API rejected the request".to_owned()
            } else {
                envelope.msg.clone()
            },
            operation: operation.to_owned(),
        });
    }
    Ok(envelope)
}

fn parse_file_name(value: &str) -> Option<String> {
    value.split(';').map(str::trim).find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if !key.eq_ignore_ascii_case("filename") && !key.eq_ignore_ascii_case("filename*") {
            return None;
        }
        Some(value.trim_matches('"').to_owned())
    })
}

fn normalized_user_id_type(value: &str) -> String {
    if value.trim().is_empty() {
        "open_id".to_owned()
    } else {
        value.trim().to_owned()
    }
}

fn bounded_page_size(value: usize, default: usize, maximum: usize) -> usize {
    if value == 0 {
        default.min(maximum).max(1)
    } else {
        value.clamp(1, maximum)
    }
}

fn unique_non_empty(values: &[String]) -> Vec<String> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !result.iter().any(|item| item == value) {
            result.push(value.to_owned());
        }
    }
    result
}

fn markdown_card_content(content: impl Into<String>) -> String {
    json!({
        "schema": "2.0",
        "config": { "wide_screen_mode": true },
        "body": { "elements": [{ "tag": "markdown", "content": content.into() }] }
    })
    .to_string()
}

fn multipart(
    fields: &[(&str, &[u8], Option<&str>)],
    file_field: &str,
    file_name: &str,
    file: &[u8],
    file_content_type: &str,
) -> Result<(Vec<u8>, String), ClientError> {
    if file_field.trim().is_empty() || file_name.trim().is_empty() {
        return Err(ClientError::InvalidInput(
            "Feishu multipart field and file name are required".to_owned(),
        ));
    }
    let boundary = "meerkat-feishu-boundary-7f8e";
    let mut body = Vec::new();
    for (name, value, content_type) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
        if let Some(content_type) = content_type {
            let _ = content_type;
        }
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{}\"\r\nContent-Type: {file_content_type}\r\n\r\n", sanitize_file_name(file_name)).as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    Ok((body, format!("multipart/form-data; boundary={boundary}")))
}

pub fn sanitize_file_name(value: &str) -> String {
    let value = value.trim();
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "attachment".to_owned()
    } else {
        value
    }
}
