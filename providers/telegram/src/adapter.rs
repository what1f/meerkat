//! Thin Telegram Provider-host adapter.
//!
//! The Bot API client, poller, preflight guard, and normalizer are public
//! independent surfaces.  This module only binds them to one ProviderHost
//! instance per workspace and forwards acknowledged events to that host.

use crate::client::{Client as TelegramClient, Credentials, PollRequest};
use crate::http::{ReqwestTransport, Transport};
use crate::mention::{MentionTarget, extract_agent_ids, render_html_with_mentions};
use crate::normalize::{NormalizationConfig, NormalizedEvent, SenderKind};
use crate::polling::Poller;
use lankide_core::markdown_mention;
use lankide_core::{AgentId, AttentionLevel, ChannelAbility, ChannelId, ProviderId};
use lankide_provider::{
    AgentContext, AgentRuntimeContext, AutoReactionContext, ChannelDefaultAttention,
    ChannelDefinition, EditMessageContext, EventSender, EventSenderType, InitContext,
    InterfaceDefinition, InvocationHandler, JsonSchema, MessageAttachment, MessageEvent, Provider,
    ProviderError, ProviderEvent, ProviderEventKind, ProviderFuture, ProviderInfo, ProviderStorage,
    ReactionEvent, RecallMessageContext, ResourceUpload, SendMessageContext, SendMessageResult,
    ToolDefinition, WorkspaceContext,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

type SharedTransport = Arc<dyn Transport>;
type SharedClient = TelegramClient<SharedTransport>;

struct WorkspaceState {
    client: Arc<SharedClient>,
    event_task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct WorkspaceStatusHandler {
    context: InitContext,
}

impl InvocationHandler for WorkspaceStatusHandler {
    fn invoke<'a>(&'a self, _input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let state = self.context.storage().workspace_state().await?;
            let mut status = serde_json::Map::from_iter([(
                "manager_bot_ready".into(),
                Value::Bool(
                    state
                        .get("manager_bot_ready")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
            )]);
            if let Some(username) = state.get("manager_bot_username").and_then(Value::as_str) {
                status.insert(
                    "manager_bot_username".into(),
                    Value::String(username.into()),
                );
            }
            Ok(Value::Object(status))
        })
    }
}

/// Telegram Provider with a production reqwest transport by default.
pub struct TelegramProvider {
    transport: SharedTransport,
    workspaces: Arc<Mutex<BTreeMap<String, WorkspaceState>>>,
    contexts: Arc<Mutex<BTreeMap<String, InitContext>>>,
    identity_lock: Arc<AsyncMutex<()>>,
}

impl Clone for TelegramProvider {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            workspaces: self.workspaces.clone(),
            contexts: self.contexts.clone(),
            identity_lock: self.identity_lock.clone(),
        }
    }
}

impl fmt::Debug for TelegramProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramProvider")
            .field("transport", &"[configured]")
            .field("workspace_count", &self.workspace_count())
            .finish()
    }
}

impl Default for TelegramProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TelegramProvider {
    pub fn new() -> Self {
        Self {
            transport: Arc::new(ReqwestTransport::new()),
            workspaces: Arc::new(Mutex::new(BTreeMap::new())),
            contexts: Arc::new(Mutex::new(BTreeMap::new())),
            identity_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    pub fn with_transport<T: Transport + 'static>(mut self, transport: T) -> Self {
        self.transport = Arc::new(transport);
        self
    }

    pub fn with_shared_transport(mut self, transport: SharedTransport) -> Self {
        self.transport = transport;
        self
    }

    pub fn workspace_count(&self) -> usize {
        self.workspaces
            .lock()
            .map(|states| states.len())
            .unwrap_or_default()
    }

    fn state(
        &self,
        workspace_id: &str,
        agent_id: &lankide_core::AgentId,
    ) -> Result<Arc<SharedClient>, ProviderError> {
        let states = self
            .workspaces
            .lock()
            .map_err(|_| ProviderError::External {
                operation: "telegram_workspace_state",
                retryable: true,
            })?;
        states
            .get(&agent_state_key(workspace_id, agent_id.as_str()))
            .map(|state| state.client.clone())
            .ok_or(ProviderError::NotFound {
                entity: "workspace",
            })
    }

    fn replace_state(&self, workspace_id: String, state: WorkspaceState) {
        if let Ok(mut states) = self.workspaces.lock() {
            if let Some(previous) = states.insert(workspace_id, state) {
                if let Some(task) = previous.event_task {
                    task.abort();
                }
            }
        }
    }

    async fn remove_state(&self, workspace_id: &str) {
        let removed = {
            let Ok(mut states) = self.workspaces.lock() else {
                return;
            };
            let keys = states
                .keys()
                .filter(|key| {
                    key.as_str() == workspace_id
                        || key.starts_with(&format!("{workspace_id}/agent/"))
                })
                .cloned()
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| states.remove(&key))
                .collect::<Vec<_>>()
        };
        for state in removed {
            if let Some(task) = state.event_task {
                task.abort();
                let _ = task.await;
            }
        }
    }
    async fn remove_agent_state(&self, workspace_id: &str, agent_id: &AgentId) {
        let removed = self.workspaces.lock().ok().and_then(|mut states| {
            states.remove(&agent_state_key(workspace_id, agent_id.as_str()))
        });
        if let Some(task) = removed.and_then(|state| state.event_task) {
            task.abort();
            let _ = task.await;
        }
    }
}

async fn persist_manager_bot(
    transport: SharedTransport,
    state_lock: Arc<AsyncMutex<()>>,
    context: WorkspaceContext,
) -> Result<(), ProviderError> {
    let token = context
        .provider_secrets()
        .read("manager_bot_token")
        .await?
        .expose_secret()
        .trim()
        .to_owned();
    let account_id = token
        .split_once(':')
        .map(|(id, _)| id)
        .filter(|id| !id.trim().is_empty())
        .ok_or(ProviderError::InvalidInput {
            field: "manager_bot_token",
        })?
        .to_owned();
    let client = TelegramClient::new(transport, Credentials::new(account_id, token))
        .map_err(|_| ProviderError::InvalidInput {
            field: "manager_bot_token",
        })?
        .with_domain(api_endpoint(&context.new_provider_config)?);
    let manager = client
        .verify_credentials()
        .await
        .map_err(|_| ProviderError::InvalidInput {
            field: "manager_bot_token",
        })?;
    if !manager.is_bot || !manager.can_manage_bots {
        return Err(ProviderError::InvalidInput {
            field: "manager_bot_token",
        });
    }
    let _state_guard = state_lock.lock().await;
    let mut state = context.storage().workspace_state().await?;
    let object = state.as_object_mut().ok_or(ProviderError::InvalidInput {
        field: "workspace_provider_state",
    })?;
    object.insert("manager_bot_id".into(), serde_json::json!(manager.id));
    object.insert(
        "manager_bot_username".into(),
        serde_json::json!(manager.username),
    );
    object.insert("manager_bot_ready".into(), serde_json::json!(true));
    context.storage().put_workspace_state(state).await
}

impl Provider for TelegramProvider {
    fn reaction_catalog(&self) -> Vec<lankide_provider::ReactionDefinition> {
        // Standard emoji reactions from https://core.telegram.org/bots/api#reactiontypeemoji.
        // Keep existing IDs and ordering so saved selections remain valid.
        [
            "👍",
            "👎",
            "❤️",
            "🔥",
            "🎉",
            "😁",
            "🤔",
            "😢",
            "🥰",
            "👏",
            "🤯",
            "😱",
            "🤬",
            "🤩",
            "🤮",
            "💩",
            "🙏",
            "👌",
            "🕊",
            "🤡",
            "🥱",
            "🥴",
            "😍",
            "🐳",
            "❤‍🔥",
            "🌚",
            "🌭",
            "💯",
            "🤣",
            "⚡",
            "🍌",
            "🏆",
            "💔",
            "🤨",
            "😐",
            "🍓",
            "🍾",
            "💋",
            "🖕",
            "😈",
            "😴",
            "😭",
            "🤓",
            "👻",
            "👨‍💻",
            "👀",
            "🎃",
            "🙈",
            "😇",
            "😨",
            "🤝",
            "✍",
            "🤗",
            "🫡",
            "🎅",
            "🎄",
            "☃",
            "💅",
            "🤪",
            "🗿",
            "🆒",
            "💘",
            "🙉",
            "🦄",
            "😘",
            "💊",
            "🙊",
            "😎",
            "👾",
            "🤷‍♂",
            "🤷",
            "🤷‍♀",
            "😡",
        ]
        .into_iter()
        .map(|emoji| lankide_provider::ReactionDefinition {
            id: emoji.into(),
            name: emoji.into(),
            emoji: emoji.into(),
        })
        .collect()
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ProviderId::new("telegram").expect("static provider id is valid"),
            name: "Telegram".into(),
            description: "Telegram Bot API messaging, polling, reactions, and attachments".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            icon: "telegram".into(),
        }
    }

    fn channels<'a>(&'a self, _context: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async {
            Ok(vec![
                ChannelDefinition::new(
                    channel("group_chat"),
                    "Telegram group chat",
                    ChannelDefaultAttention::Mention,
                    [
                        AttentionLevel::Follow,
                        AttentionLevel::Schedule,
                        AttentionLevel::Mention,
                    ],
                    [
                        ChannelAbility::Reply,
                        ChannelAbility::Attachment,
                        ChannelAbility::Reaction,
                        ChannelAbility::Edit,
                        ChannelAbility::Recall,
                    ],
                )?,
                ChannelDefinition::new(
                    channel("private_chat"),
                    "Telegram private chat",
                    ChannelDefaultAttention::DirectMessage,
                    [],
                    [
                        ChannelAbility::Reply,
                        ChannelAbility::Attachment,
                        ChannelAbility::Reaction,
                        ChannelAbility::Edit,
                        ChannelAbility::Recall,
                    ],
                )?,
            ])
        })
    }

    fn create_workspace<'a>(&'a self, context: WorkspaceContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            persist_manager_bot(self.transport.clone(), self.identity_lock.clone(), context).await
        })
    }

    fn update_workspace<'a>(&'a self, context: WorkspaceContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            persist_manager_bot(self.transport.clone(), self.identity_lock.clone(), context).await
        })
    }

    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            context
                .register_interface(InterfaceDefinition {
                    id: "telegram.workspace.status".into(),
                    input_schema: JsonSchema::new(json!({"type": "object"}))?,
                    output_schema: JsonSchema::new(json!({
                        "type": "object",
                        "required": ["manager_bot_ready"],
                        "properties": {
                            "manager_bot_ready": {"type": "boolean"},
                            "manager_bot_username": {"type": "string"}
                        }
                    }))?,
                    handler: Arc::new(WorkspaceStatusHandler {
                        context: context.clone(),
                    }),
                })
                .await?;
            context
                .register_interface(InterfaceDefinition {
                    id: "telegram.workspace.owner".into(),
                    input_schema: JsonSchema::new(json!({"type":"object","required":["action"]}))?,
                    output_schema: JsonSchema::new(json!({"type":"object","required":["state"]}))?,
                    handler: Arc::new(crate::workspace_owner::OwnerHandler {
                        context: context.clone(),
                        transport: self.transport.clone(),
                    }),
                })
                .await?;
            let workspace_key = context.workspace_id().to_string();
            if let Ok(mut contexts) = self.contexts.lock() {
                contexts.insert(workspace_key.clone(), context.clone());
            }
            Ok(())
        })
    }

    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult> {
        Box::pin(async move {
            let client = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let chat_id = context
                .chat_id
                .parse::<i64>()
                .map_err(|_| ProviderError::InvalidInput { field: "chat_id" })?;
            let attachments = resolved_attachments(context.storage(), &context.attachments).await?;
            let targets =
                telegram_mention_targets(context.storage(), &context.mentions, &context.content)
                    .await?;
            let html = render_html_with_mentions(&context.content, &targets);
            let reply_to = resolve_reply_id(
                context.storage(),
                &context.channel_id,
                &context.chat_id,
                context.reply_to,
            )
            .await?;
            if html.trim().is_empty() && attachments.is_empty() {
                return Err(ProviderError::InvalidInput { field: "content" });
            }
            let primary_caption = html.clone();
            let sent = match (async move {
                let mut primary = None;
                if !html.trim().is_empty() && attachments.is_empty() {
                    primary = Some(client.send_message(chat_id, &html, reply_to).await?);
                } else if !html.trim().is_empty() && !attachments.is_empty() {
                    // Keep the internal message identity attached to the text
                    // receipt; subsequent files are part of the same send
                    // operation and failures are reported as Unknown.
                    primary = Some(client.send_message(chat_id, &html, reply_to).await?);
                }
                for (index, attachment) in attachments.into_iter().enumerate() {
                    let caption = if index == 0 && primary.is_none() {
                        primary_caption.as_str()
                    } else {
                        ""
                    };
                    let sent_attachment = if attachment.kind == "image" {
                        client
                            .send_photo(
                                chat_id,
                                &attachment.bytes,
                                &attachment.file_name,
                                caption,
                                reply_to,
                            )
                            .await?
                    } else {
                        client
                            .send_document(
                                chat_id,
                                &attachment.bytes,
                                &attachment.file_name,
                                caption,
                                reply_to,
                            )
                            .await?
                    };
                    primary.get_or_insert(sent_attachment);
                }
                primary.ok_or(crate::http::ClientError::InvalidInput(
                    "Telegram message payload is empty".to_owned(),
                ))
            })
            .await
            {
                Ok(sent) => sent,
                Err(error) if is_unknown_send_error(&error) => {
                    return Ok(SendMessageResult::unknown());
                }
                Err(_) => {
                    return Err(ProviderError::External {
                        operation: "telegram_send_message",
                        retryable: false,
                    });
                }
            };
            SendMessageResult::sent(sent.message_id.to_string())
        })
    }

    fn recall_message<'a>(&'a self, context: RecallMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let client = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let chat_id = context
                .chat_id
                .parse::<i64>()
                .map_err(|_| ProviderError::InvalidInput { field: "chat_id" })?;
            let message_id = context.provider_message_id.parse::<i64>().map_err(|_| {
                ProviderError::InvalidInput {
                    field: "provider_message_id",
                }
            })?;
            client
                .delete_message(chat_id, message_id)
                .await
                .map_err(|_| ProviderError::External {
                    operation: "telegram_recall_message",
                    retryable: false,
                })
        })
    }

    fn edit_message<'a>(&'a self, context: EditMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let client = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let chat_id = context
                .chat_id
                .parse::<i64>()
                .map_err(|_| ProviderError::InvalidInput { field: "chat_id" })?;
            let message_id = context.provider_message_id.parse::<i64>().map_err(|_| {
                ProviderError::InvalidInput {
                    field: "provider_message_id",
                }
            })?;
            let targets =
                telegram_mention_targets(context.storage(), &[], &context.content).await?;
            let html = render_html_with_mentions(&context.content, &targets);
            if html.trim().is_empty() {
                return Err(ProviderError::InvalidInput { field: "content" });
            }
            client
                .edit_message_text(chat_id, message_id, &html)
                .await
                .map_err(|_| ProviderError::External {
                    operation: "telegram_edit_message",
                    retryable: false,
                })
        })
    }

    fn auto_reaction<'a>(&'a self, context: AutoReactionContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let client = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let chat_id = context
                .chat_id
                .parse::<i64>()
                .map_err(|_| ProviderError::InvalidInput { field: "chat_id" })?;
            let trigger_message =
                context
                    .trigger_messages
                    .last()
                    .copied()
                    .ok_or(ProviderError::InvalidInput {
                        field: "trigger_messages",
                    })?;
            let target = context.storage().message(trigger_message).await?;
            if target.channel_id != context.channel_id || target.chat_id != context.chat_id {
                return Err(ProviderError::InvalidInput {
                    field: "trigger_messages",
                });
            }
            let target = target
                .provider_message_id
                .as_deref()
                .and_then(transport_message_id)
                .ok_or(ProviderError::InvalidInput {
                    field: "trigger_messages",
                })?;
            if context.auto_reaction.trim().is_empty() {
                return Err(ProviderError::InvalidInput {
                    field: "auto_reaction",
                });
            }
            let emoji = context.auto_reaction.clone();
            client
                .set_message_reaction(chat_id, target, Some(&emoji))
                .await
                .map_err(|_| ProviderError::External {
                    operation: "telegram_auto_reaction",
                    retryable: false,
                })
        })
    }

    fn create_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let token = match context.provider_secrets().read("bot_token").await {
                Ok(value) => value.expose_secret().trim().to_owned(),
                Err(ProviderError::SecretAccessDenied)
                    if config_text(&context.new_provider_agent_config, "mode").as_deref()
                        == Some("new") =>
                {
                    let mut provider_state =
                        context.storage().agent_state(&context.agent_id).await?;
                    let object =
                        provider_state
                            .as_object_mut()
                            .ok_or(ProviderError::InvalidInput {
                                field: "agent_provider_state",
                            })?;
                    object.entry("onboarding").or_insert_with(|| {
                        serde_json::json!({
                            "step": "create_bot",
                            "status": "action_required",
                            "title": "创建 Telegram Bot",
                            "instruction": "继续完成 Telegram Bot 创建。",
                            "payload": {},
                        })
                    });
                    context
                        .storage()
                        .put_agent_state(&context.agent_id, provider_state)
                        .await?;
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let account_id = token
                .split_once(':')
                .map(|(id, _)| id)
                .filter(|id| !id.trim().is_empty())
                .ok_or(ProviderError::InvalidInput { field: "bot_token" })?
                .to_owned();
            let endpoint = api_endpoint(&context.workspace_config)?;
            let credentials = Credentials::new(account_id, token);
            let client = Arc::new(
                SharedClient::new(self.transport.clone(), credentials.clone())
                    .map_err(|_| ProviderError::InvalidInput { field: "bot_token" })?
                    .with_domain(endpoint.clone()),
            );
            let verify_client = client.clone();
            let bot = verify_client
                .verify_credentials()
                .await
                .map_err(|_| ProviderError::InvalidInput { field: "bot_token" })?;
            let mut provider_state = context.storage().agent_state(&context.agent_id).await?;
            let already_bound_to_this_bot = provider_state
                .get("account_id")
                .and_then(Value::as_i64)
                .is_some_and(|account_id| account_id == bot.id)
                || provider_state
                    .get("account_id")
                    .and_then(Value::as_str)
                    .is_some_and(|account_id| account_id == bot.id.to_string());
            let provider_state_object =
                provider_state
                    .as_object_mut()
                    .ok_or(ProviderError::InvalidInput {
                        field: "agent_provider_state",
                    })?;
            provider_state_object.insert("account_id".into(), serde_json::json!(bot.id));
            provider_state_object.insert(
                "bot_username".into(),
                serde_json::json!(bot.username.clone()),
            );
            context
                .storage()
                .put_agent_state(&context.agent_id, provider_state)
                .await?;
            {
                let _identity_guard = self.identity_lock.lock().await;
                let (workspace_state, mut directory) =
                    crate::identity::load(context.storage()).await?;
                directory.register_agent(
                    &context.agent_id,
                    bot.id,
                    &bot.username,
                    if bot.first_name.trim().is_empty() {
                        &context.name
                    } else {
                        &bot.first_name
                    },
                );
                crate::identity::save(context.storage(), workspace_state, &directory).await?;
            }
            if let Some(expected) = config_text(&context.new_provider_agent_config, "bot_username")
                && !expected.trim().is_empty()
                && !bot.username.eq_ignore_ascii_case(expected.trim())
            {
                return Err(ProviderError::InvalidInput {
                    field: "bot_username",
                });
            }
            let name = context.name.clone();
            let description = context.bio;
            // A restarted Runtime calls create_agent again for every persisted
            // Agent.  Once the credential is already bound to this Bot, the
            // profile has already had its initial sync opportunity; repeating
            // setMyName on every restart can hit Telegram's long rate limit and
            // incorrectly prevent the Agent from becoming available.
            if !already_bound_to_this_bot
                && !config_bool(
                    &context.new_provider_agent_config,
                    "skip_profile_sync",
                    false,
                )
            {
                let profile_client = client.clone();
                (async move {
                    if !name.trim().is_empty() {
                        profile_client.set_my_name(&name).await?;
                    }
                    if !description.trim().is_empty() {
                        profile_client.set_my_description(&description).await?;
                    }
                    Ok::<_, crate::http::ClientError>(())
                })
                .await
                .map_err(|_| ProviderError::External {
                    operation: "telegram_create_agent",
                    retryable: false,
                })?;
            }
            let init_context = self
                .contexts
                .lock()
                .ok()
                .and_then(|contexts| contexts.get(&context.workspace_id.to_string()).cloned())
                .ok_or(ProviderError::NotFound {
                    entity: "workspace",
                })?;
            let offset_key = format!(
                "telegram/{}/agent/{}/poll_offset",
                context.workspace_id, context.agent_id
            );
            let offset = init_context
                .storage()
                .get(&offset_key)
                .await?
                .and_then(|value| String::from_utf8(value).ok())
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_default();
            let poll_client = SharedClient::new(self.transport.clone(), credentials)
                .map_err(|_| ProviderError::InvalidInput { field: "bot_token" })?
                .with_domain(endpoint);
            let mut request = PollRequest::default();
            request.offset = offset;
            if let Some(timeout) =
                config_u64(init_context.workspace_config(), "poll_timeout_seconds")
            {
                request.timeout_seconds = timeout.min(50) as u8;
            }
            let normalization = normalization_config(&init_context, bot).await?;
            let event_task =
                config_bool(init_context.workspace_config(), "polling", true).then(|| {
                    spawn_poller(
                        init_context,
                        poll_client,
                        request,
                        normalization,
                        offset_key,
                    )
                });
            self.replace_state(
                agent_state_key(&context.workspace_id.to_string(), context.agent_id.as_str()),
                WorkspaceState { client, event_task },
            );
            Ok(())
        })
    }

    fn tools<'a>(
        &'a self,
        context: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<ToolDefinition>> {
        Box::pin(async move {
            let client = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            Ok(vec![crate::person_profile::tool(&context, client)?])
        })
    }

    fn delete_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.remove_agent_state(&context.workspace_id.to_string(), &context.agent_id)
                .await;
            let _identity_guard = self.identity_lock.lock().await;
            let (workspace_state, mut directory) = crate::identity::load(context.storage()).await?;
            directory.remove_agent(&context.agent_id);
            crate::identity::save(context.storage(), workspace_state, &directory).await
        })
    }

    fn agent_onboarding<'a>(
        &'a self,
        context: lankide_provider::AgentOnboardingContext,
        action: String,
        opaque_payload: Value,
    ) -> ProviderFuture<'a, lankide_provider::ProviderOnboarding> {
        Box::pin(crate::onboarding::agent_onboarding_with_transport(
            context,
            action,
            opaque_payload,
            self.transport.clone(),
        ))
    }

    fn update_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let client = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let name = context.name;
            let description = context.bio;
            let profile = context.new_provider_agent_config;
            let avatar_bytes = parse_byte_array(&profile, "avatar_bytes");
            let avatar_file_name = config_text(&profile, "avatar_file_name")
                .unwrap_or_else(|| "avatar.jpg".to_owned());
            if name.trim().is_empty() && description.trim().is_empty() && avatar_bytes.is_none() {
                return Ok(());
            }
            (async move {
                if !name.trim().is_empty() {
                    client.set_my_name(&name).await?;
                }
                if !description.trim().is_empty() {
                    client.set_my_description(&description).await?;
                }
                if let Some(bytes) = avatar_bytes.as_deref() {
                    client
                        .set_my_profile_photo(bytes, &avatar_file_name)
                        .await?;
                }
                Ok::<(), crate::http::ClientError>(())
            })
            .await
            .map_err(|_| ProviderError::External {
                operation: "telegram_update_profile",
                retryable: false,
            })
        })
    }

    fn shutdown<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let workspace_id = context.workspace_id().to_string();
            self.remove_state(&workspace_id).await;
            if let Ok(mut contexts) = self.contexts.lock() {
                contexts.remove(&workspace_id);
            }
            Ok(())
        })
    }
}

fn channel(value: &str) -> ChannelId {
    ChannelId::new(value).expect("static Telegram channel ID is valid")
}

fn agent_state_key(workspace_id: &str, agent_id: &str) -> String {
    format!("{workspace_id}/agent/{agent_id}")
}

fn config_text(config: &Value, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn config_bool(config: &Value, key: &str, default: bool) -> bool {
    config.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn config_u64(config: &Value, key: &str) -> Option<u64> {
    config.get(key).and_then(Value::as_u64)
}

fn parse_byte_array(config: &Value, key: &str) -> Option<Vec<u8>> {
    let values = config.get(key)?.as_array()?;
    let bytes = values
        .iter()
        .map(Value::as_u64)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .map(u8::try_from)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

fn transport_message_id(value: &str) -> Option<i64> {
    value
        .trim()
        .split_once(":edit:")
        .map(|(original, _)| original)
        .unwrap_or_else(|| value.trim())
        .parse()
        .ok()
}

async fn normalization_config(
    context: &InitContext,
    bot: crate::dto::User,
) -> Result<crate::normalize::NormalizationConfig, ProviderError> {
    let (_, directory) = crate::identity::load(context.storage()).await?;
    Ok(directory.normalization_config(bot))
}

struct EventRetry {
    initial: Duration,
    maximum: Duration,
    attempts: u64,
}
impl EventRetry {
    fn configured(context: &InitContext) -> Self {
        let config = context.workspace_config();
        Self {
            initial: Duration::from_millis(
                config
                    .get("event_retry_initial_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(250)
                    .max(1),
            ),
            maximum: Duration::from_millis(
                config
                    .get("event_retry_max_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(30_000)
                    .max(1),
            ),
            attempts: config
                .get("event_retry_max_attempts")
                .and_then(Value::as_u64)
                .unwrap_or(8)
                .max(1),
        }
    }
    fn exhausted(&self, failures: u64) -> bool {
        failures >= self.attempts
    }
    fn delay(&self, failures: u64) -> Duration {
        self.initial
            .saturating_mul(2u32.saturating_pow(failures.saturating_sub(1).min(31) as u32))
            .min(self.maximum)
    }
}

#[derive(Debug)]
struct EventError {
    reason: String,
    retryable: bool,
    retry_after: Option<Duration>,
}
impl From<crate::http::ClientError> for EventError {
    fn from(error: crate::http::ClientError) -> Self {
        Self {
            retryable: error.is_retryable(),
            retry_after: error.retry_after(),
            reason: error.to_string(),
        }
    }
}
impl From<ProviderError> for EventError {
    fn from(error: ProviderError) -> Self {
        Self {
            retryable: matches!(
                error,
                ProviderError::External {
                    retryable: true,
                    ..
                } | ProviderError::Timeout { .. }
                    | ProviderError::EventSinkClosed
            ),
            reason: format!("{error:?}"),
            retry_after: None,
        }
    }
}
fn spawn_poller(
    context: InitContext,
    client: SharedClient,
    request: PollRequest,
    normalization: NormalizationConfig,
    offset_key: String,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let retry = EventRetry::configured(&context);
        let mut poller = Poller::new(client, normalization).with_request(request);
        let mut poll_failures = 0;
        loop {
            let batch = tokio::select! {
                _ = context.shutdown_signal().cancelled() => break,
                result = poller.poll_once() => match result {
                    Ok(Some(batch)) => batch,
                    Ok(None) => break,
                    Err(error) => {
                        let crate::polling::PollError::Client(client_error) = &error else {
                            context.logger().log(lankide_provider::LogLevel::Warn, &format!("Telegram polling stopped: {error}"));
                            break;
                        };
                        if !client_error.is_retryable() {
                            context.logger().log(lankide_provider::LogLevel::Warn, &format!("Telegram polling stopped: {error}"));
                            break;
                        }
                        poll_failures += 1;
                        let delay = retry.delay(poll_failures).max(client_error.retry_after().unwrap_or_default());
                        context.logger().log(lankide_provider::LogLevel::Warn, &format!("Telegram polling failed; retry in {}ms: {error}", delay.as_millis()));
                        tokio::select! { _ = context.shutdown_signal().cancelled() => break, _ = tokio::time::sleep(delay) => {} }
                        continue;
                    }
                }
            };
            poll_failures = 0;
            for reason in &batch.skipped {
                context.logger().log(
                    lankide_provider::LogLevel::Warn,
                    &format!("Telegram event skipped: {reason}"),
                );
            }
            for event in &batch.events {
                let mut failures = 0;
                // Keep the accepted prefix of this batch; a later failure must
                // never replay earlier deliveries.
                let mut converted = None;
                loop {
                    let result: Result<(), EventError> = async {
                        if converted.is_none() {
                            converted = Some(
                                normalized_event_to_provider(
                                    &context,
                                    poller.client(),
                                    event.clone(),
                                )
                                .await?,
                            );
                        }
                        context
                            .event_sink()
                            .emit(converted.as_ref().unwrap().clone())
                            .await
                            .map_err(EventError::from)
                    }
                    .await;
                    match result {
                        Ok(()) => break,
                        Err(error) => {
                            failures += 1;
                            // One event must not hold back the ones after it.
                            if !error.retryable || retry.exhausted(failures) {
                                context.logger().log(
                                    lankide_provider::LogLevel::Warn,
                                    &format!("Telegram event skipped: {}", error.reason),
                                );
                                break;
                            }
                            let delay = retry
                                .delay(failures)
                                .max(error.retry_after.unwrap_or_default());
                            context.logger().log(
                                lankide_provider::LogLevel::Warn,
                                &format!(
                                    "Telegram event acceptance failed; retry in {}ms: {}",
                                    delay.as_millis(),
                                    error.reason
                                ),
                            );
                            tokio::select! { _ = context.shutdown_signal().cancelled() => return, _ = tokio::time::sleep(delay) => {} }
                        }
                    }
                }
            }
            if batch.next_offset == batch.offset {
                continue;
            }
            let mut failures = 0;
            loop {
                let result = tokio::select! {
                    _ = context.shutdown_signal().cancelled() => return,
                    result = context.storage().put(&offset_key, batch.next_offset.to_string().into_bytes()) => result,
                };
                match result {
                    Ok(()) => break,
                    Err(error) => {
                        failures += 1;
                        let delay = retry.delay(failures);
                        context.logger().log(
                            lankide_provider::LogLevel::Warn,
                            &format!(
                                "Telegram poll offset persistence failed; retry in {}ms: {error}",
                                delay.as_millis()
                            ),
                        );
                        tokio::select! { _ = context.shutdown_signal().cancelled() => return, _ = tokio::time::sleep(delay) => {} }
                    }
                }
            }
            if let Err(error) = poller.acknowledge(&batch) {
                context.logger().log(
                    lankide_provider::LogLevel::Warn,
                    &format!("Telegram poll offset was not acknowledged: {error}"),
                );
                break;
            }
        }
        poller.stop();
    })
}

async fn receiving_agent(
    context: &InitContext,
    account_id: &str,
) -> Result<Option<AgentId>, EventError> {
    let (_, directory) = crate::identity::load(context.storage())
        .await
        .map_err(EventError::from)?;
    let mut receivers = directory
        .agents
        .iter()
        .filter(|(_, identity)| identity.account_id.to_string() == account_id)
        .filter_map(|(id, _)| AgentId::new(id.clone()).ok());
    let receiver = receivers.next();
    Ok(if receivers.next().is_none() {
        receiver
    } else {
        None
    })
}

async fn normalized_event_to_provider(
    context: &InitContext,
    client: &SharedClient,
    event: NormalizedEvent,
) -> Result<ProviderEvent, EventError> {
    match event {
        NormalizedEvent::Message(mut message) => {
            let sender = resolved_sender(
                context,
                &message.sender.kind,
                &message.sender.sender_id,
                &message.sender.display_name,
            )
            .await?;
            for mention in &message.mentions {
                if mention.agent_id.is_some() || mention.external_user_id == 0 {
                    continue;
                }
                let person = context
                    .storage()
                    .get_or_create_person(&mention.external_user_id.to_string())
                    .await
                    .map_err(EventError::from)?;
                if let (Some(external_marker), Some(person_marker)) = (
                    markdown_mention(&mention.display_name, &mention.external_user_id.to_string()),
                    markdown_mention(&mention.display_name, person.person_id.as_str()),
                ) {
                    message.content = message.content.replace(&external_marker, &person_marker);
                }
            }
            let provider_data = serde_json::to_value(&message).unwrap_or(Value::Null);
            let mut attachments = Vec::with_capacity(message.attachments.len());
            for attachment in &message.attachments {
                let downloaded = client.download_file(&attachment.file_id).await?;
                let resource = context
                    .save_resource(ResourceUpload {
                        file_name: attachment.name.clone(),
                        media_type: if downloaded.content_type.trim().is_empty() {
                            attachment.content_type.clone()
                        } else {
                            downloaded.content_type
                        },
                        bytes: downloaded.data,
                    })
                    .await
                    .map_err(EventError::from)?;
                attachments.push(resource);
            }
            let mentioned_agent_ids = message
                .mentioned_agent_ids
                .iter()
                .filter_map(|value| AgentId::new(value.clone()).ok())
                .collect();
            let receiving_agent_id = receiving_agent(context, &message.account_id).await?;
            Ok(ProviderEvent {
                receiving_agent_id,
                channel_id: channel(&message.channel_id),
                provider_chat_id: message.chat_id,
                sender,
                kind: ProviderEventKind::Message(MessageEvent {
                    provider_message_id: message.provider_message_id,
                    content: message.content,
                    reply_to_provider_message_id: message.reply_to_provider_message_id,
                    mentioned_agent_ids,
                    attachments,
                }),
                provider_data,
            })
        }
        NormalizedEvent::Reaction(reaction) => {
            let provider_data = serde_json::to_value(&reaction).unwrap_or(Value::Null);
            let sender = resolved_sender(
                context,
                &reaction.sender.kind,
                &reaction.sender.sender_id,
                &reaction.sender.display_name,
            )
            .await?;
            let action = reaction.action;
            let receiving_agent_id = if reaction.channel_id == "private_chat" {
                receiving_agent(context, &reaction.account_id).await?
            } else {
                None
            };
            Ok(ProviderEvent {
                receiving_agent_id,
                channel_id: channel(&reaction.channel_id),
                provider_chat_id: reaction.chat_id,
                sender: sender.clone(),
                kind: ProviderEventKind::Reaction(ReactionEvent {
                    target_provider_message_id: reaction.target_provider_message_id,
                    sender,
                    reaction: reaction.reaction,
                    action,
                }),
                provider_data,
            })
        }
    }
}

fn sender(kind: &SenderKind, id: &str, name: &str) -> EventSender {
    EventSender {
        id: if id.trim().is_empty() {
            "telegram:unknown".to_owned()
        } else {
            id.to_owned()
        },
        kind: match kind {
            SenderKind::Human => EventSenderType::Human,
            SenderKind::Agent => EventSenderType::Agent,
            SenderKind::Bot => EventSenderType::Bot,
            SenderKind::System | SenderKind::Unknown => EventSenderType::System,
        },
        name: if name.trim().is_empty() {
            id.to_owned()
        } else {
            name.to_owned()
        },
    }
}

async fn resolved_sender(
    context: &InitContext,
    kind: &SenderKind,
    id: &str,
    name: &str,
) -> Result<EventSender, EventError> {
    if *kind != SenderKind::Human {
        return Ok(sender(kind, id, name));
    }
    let person = context
        .storage()
        .get_or_create_person(id.trim())
        .await
        .map_err(EventError::from)?;
    Ok(sender(kind, person.person_id.as_str(), name))
}

async fn telegram_mention_targets(
    storage: &dyn ProviderStorage,
    mentions: &[AgentId],
    content: &str,
) -> Result<Vec<MentionTarget>, ProviderError> {
    let (_, directory) = crate::identity::load(storage).await?;
    let mut ids = mentions.iter().map(ToString::to_string).collect::<Vec<_>>();
    for id in extract_agent_ids(content) {
        if !ids.iter().any(|value| value == &id) {
            ids.push(id);
        }
    }
    let mut targets = Vec::new();
    for id in ids {
        if let Ok(agent_id) = AgentId::new(id.clone())
            && let Some((external_user_id, username, display_name)) =
                directory.mention_target(&agent_id)
        {
            targets.push(MentionTarget::new(
                id,
                external_user_id,
                username,
                display_name,
            ));
            continue;
        }
        let Ok(person_id) = lankide_core::PersonId::new(id.clone()) else {
            continue;
        };
        let person = storage.person(&person_id).await?;
        let external_user_id =
            person
                .external_id
                .parse::<i64>()
                .map_err(|_| ProviderError::InvalidInput {
                    field: "person_external_id",
                })?;
        targets.push(MentionTarget::new(
            id.clone(),
            Some(external_user_id),
            None,
            id,
        ));
    }
    Ok(targets)
}

#[derive(Clone)]
struct ResolvedAttachment {
    kind: String,
    file_name: String,
    bytes: Vec<u8>,
}

async fn resolved_attachments(
    storage: &dyn ProviderStorage,
    declared: &[MessageAttachment],
) -> Result<Vec<ResolvedAttachment>, ProviderError> {
    let mut result = Vec::with_capacity(declared.len());
    for attachment in declared {
        let resolved = storage.resource(&attachment.resource).await?;
        let kind = if resolved.resource.media_type.starts_with("image/") {
            "image"
        } else {
            "file"
        };
        result.push(ResolvedAttachment {
            kind: kind.to_owned(),
            file_name: resolved.resource.file_name,
            bytes: resolved.bytes,
        });
    }
    Ok(result)
}

async fn resolve_reply_id(
    storage: &dyn ProviderStorage,
    channel_id: &ChannelId,
    chat_id: &str,
    reply_to: Option<lankide_core::MessageId>,
) -> Result<Option<i64>, ProviderError> {
    let Some(reply_to) = reply_to else {
        return Ok(None);
    };
    let target = storage.message(reply_to).await?;
    if &target.channel_id != channel_id || target.chat_id != chat_id {
        return Err(ProviderError::InvalidInput { field: "reply_to" });
    }
    target
        .provider_message_id
        .as_deref()
        .and_then(transport_message_id)
        .ok_or(())
        .map(Some)
        .map_err(|_| ProviderError::InvalidInput { field: "reply_to" })
}

fn is_unknown_send_error(error: &crate::http::ClientError) -> bool {
    matches!(
        error,
        crate::http::ClientError::Transport(_)
            | crate::http::ClientError::Decode(_)
            | crate::http::ClientError::MissingField { .. }
            | crate::http::ClientError::HttpStatus { status: 500.., .. }
    )
}

/// Base URL of a public or locally hosted Telegram Bot API server.
pub(crate) fn api_endpoint(config: &Value) -> Result<String, ProviderError> {
    let Some(value) = config_text(config, "api_endpoint") else {
        return Ok(crate::client::TELEGRAM_API_BASE_URL.to_owned());
    };
    let url = reqwest::Url::parse(&value).map_err(|_| ProviderError::InvalidInput {
        field: "api_endpoint",
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::InvalidInput {
            field: "api_endpoint",
        });
    }
    Ok(value.trim_end_matches('/').to_owned())
}
