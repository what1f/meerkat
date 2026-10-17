//! Thin Feishu Provider-host adapter.
//!
//! The wire client and normalizer remain usable without this module.  This
//! layer only translates the current Provider context, owns one client/task
//! per workspace instance, and forwards normalized events to that instance's
//! event sink.

use crate::client::{Auth, Credentials, Domain, ReplyMessage, SendMessage};
use crate::events::{
    DispatchError, Dispatcher, EventSink as NormalizedEventSink, EventSource, SourceError,
};
use crate::http::{ReqwestTransport, Transport};
use crate::mention::{
    BARE_MENTION, MentionTarget, extract_agent_ids, has_bare_mention, render_card_markdown,
};
use crate::normalize::{NormalizationConfig, NormalizedEvent, SenderKind};
use lankide_core::{
    AgentId, AttentionLevel, ChannelAbility, ChannelId, ProviderId, ResourceRef, markdown_mention,
};
use lankide_provider::{
    AgentContext, AgentRuntimeContext, AutoReactionContext, ChannelDefaultAttention,
    ChannelDefinition, EventSender, EventSenderType, InitContext, InterfaceDefinition,
    InvocationHandler, JsonSchema, MessageAttachment, MessageEvent,
    PROVIDER_CAPABILITY_TASK_SYSTEM, Provider, ProviderError, ProviderEvent, ProviderEventKind,
    ProviderFuture, ProviderInfo, ProviderStorage, ReactionEvent, RecallMessageContext,
    ResourceUpload, SendMessageContext, SendMessageResult, ToolDefinition,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

const FEISHU_EVENT_TYPES: [&str; 5] = [
    "im.message.receive_v1",
    "im.chat.updated_v1",
    "im.message.reaction.created_v1",
    "im.message.reaction.deleted_v1",
    "task.*",
];

/// A host-provided Feishu long-connection source.  The source owns SDK
/// websocket framing; the Provider owns source retry, registration,
/// normalization, lifecycle, and sink delivery.
pub type EventSourceFactory = dyn for<'a> Fn(
        &'a lankide_core::WorkspaceId,
        &'a Credentials,
        &'a Domain,
        &'a dyn Transport,
    ) -> crate::events::EventFuture<'a, Result<Box<dyn EventSource>, SourceError>>
    + Send
    + Sync;

type SharedTransport = Arc<dyn Transport>;
type SharedClient = crate::client::Client<SharedTransport>;

struct EventTask {
    task: JoinHandle<()>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}
impl EventTask {
    async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = self.task.await;
    }
}

struct WorkspaceState {
    client: Arc<SharedClient>,
    auth: Auth,
    event_task: Option<EventTask>,
}

const INBOUND_RESOURCE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const INBOUND_RESOURCE_CACHE_LIMIT: usize = 1024;

#[derive(Default)]
struct InboundResourceCache {
    entries: BTreeMap<String, (ResourceRef, Instant)>,
}

impl InboundResourceCache {
    fn get(&mut self, key: &str) -> Option<ResourceRef> {
        self.prune();
        self.entries.get(key).map(|(resource, _)| resource.clone())
    }

    fn insert(&mut self, key: String, resource: ResourceRef) {
        self.prune();
        if self.entries.len() >= INBOUND_RESOURCE_CACHE_LIMIT
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, observed_at))| *observed_at)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, (resource, Instant::now()));
    }

    fn prune(&mut self) {
        self.entries
            .retain(|_, (_, observed_at)| observed_at.elapsed() < INBOUND_RESOURCE_CACHE_TTL);
    }
}

/// Signs the user in to the Feishu developer console with a QR code, so new
/// Agent Apps are created without another confirmation.
pub const CONSOLE_LOGIN_INTERFACE: &str = "feishu.workspace.console_login";

struct ConsoleLoginHandler {
    switch_lock: AsyncMutex<()>,
    context: InitContext,
    transport: SharedTransport,
}

impl InvocationHandler for ConsoleLoginHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let action = input
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("status");
            if action.starts_with("switch_") {
                return crate::workspace_owner::switch_account(
                    &self.context,
                    self.transport.clone(),
                    action,
                    &self.switch_lock,
                )
                .await;
            }
            let login = crate::console::login(
                self.transport.clone(),
                self.context.secret_reader(),
                self.context.secret_writer(),
                action,
            )
            .await?;
            Ok(match login {
                crate::console::Login::Ready(_) => serde_json::json!({"state": "ready"}),
                crate::console::Login::Required => serde_json::json!({"state": "required"}),
                crate::console::Login::Waiting { qr_code, scanned } => serde_json::json!({
                    "state": if scanned { "scanned" } else { "waiting" },
                    "qr_code": qr_code,
                }),
            })
        })
    }
}

/// Feishu Provider with a production reqwest transport by default.
///
/// `with_transport` and `with_event_source_factory` are the two seams used by
/// module E2E and by hosts that already own a configured SDK websocket client.
pub struct FeishuProvider {
    transport: SharedTransport,
    event_source_factory: Option<Arc<EventSourceFactory>>,
    workspaces: Arc<Mutex<BTreeMap<String, WorkspaceState>>>,
    contexts: Arc<Mutex<BTreeMap<String, InitContext>>>,
    identity_lock: Arc<AsyncMutex<()>>,
    inbound_resource_cache: Arc<AsyncMutex<InboundResourceCache>>,
}

impl Clone for FeishuProvider {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            event_source_factory: self.event_source_factory.clone(),
            workspaces: self.workspaces.clone(),
            contexts: self.contexts.clone(),
            identity_lock: self.identity_lock.clone(),
            inbound_resource_cache: self.inbound_resource_cache.clone(),
        }
    }
}

impl fmt::Debug for FeishuProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeishuProvider")
            .field("transport", &"[configured]")
            .field("event_source_factory", &self.event_source_factory.is_some())
            .field("workspace_count", &self.workspace_count())
            .finish()
    }
}

impl Default for FeishuProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuProvider {
    pub fn new() -> Self {
        Self {
            transport: Arc::new(ReqwestTransport::new()),
            event_source_factory: Some(Arc::new(crate::ws::native_event_source_factory)),
            workspaces: Arc::new(Mutex::new(BTreeMap::new())),
            contexts: Arc::new(Mutex::new(BTreeMap::new())),
            identity_lock: Arc::new(AsyncMutex::new(())),
            inbound_resource_cache: Arc::new(AsyncMutex::new(InboundResourceCache::default())),
        }
    }

    /// Disable the native websocket for a host that only wants REST calls or
    /// supplies its own event receiver later.
    pub fn without_event_source(mut self) -> Self {
        self.event_source_factory = None;
        self
    }

    pub fn with_transport<T: Transport + 'static>(mut self, transport: T) -> Self {
        self.transport = Arc::new(transport);
        self
    }

    pub fn with_shared_transport(mut self, transport: SharedTransport) -> Self {
        self.transport = transport;
        self
    }

    pub fn with_event_source_factory<F>(mut self, factory: F) -> Self
    where
        F: for<'a> Fn(
                &'a lankide_core::WorkspaceId,
                &'a Credentials,
                &'a Domain,
                &'a dyn Transport,
            )
                -> crate::events::EventFuture<'a, Result<Box<dyn EventSource>, SourceError>>
            + Send
            + Sync
            + 'static,
    {
        self.event_source_factory = Some(Arc::new(factory));
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
        agent_id: &AgentId,
    ) -> Result<WorkspaceStateRef, ProviderError> {
        let states = self
            .workspaces
            .lock()
            .map_err(|_| ProviderError::External {
                operation: "feishu_workspace_state",
                retryable: true,
            })?;
        let state = states
            .get(&agent_state_key(workspace_id, agent_id.as_str()))
            .ok_or(ProviderError::NotFound {
                entity: "agent_connection",
            })?;
        Ok(WorkspaceStateRef {
            client: state.client.clone(),
            auth: state.auth,
        })
    }

    fn replace_state(&self, workspace_id: String, state: WorkspaceState) {
        if let Ok(mut states) = self.workspaces.lock() {
            if let Some(previous) = states.insert(workspace_id, state) {
                if let Some(task) = previous.event_task {
                    let _ = task.shutdown.send(());
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
                task.stop().await;
            }
        }
    }

    async fn remove_agent_state(&self, workspace_id: &str, agent_id: &AgentId) {
        let removed = self.workspaces.lock().ok().and_then(|mut states| {
            states.remove(&agent_state_key(workspace_id, agent_id.as_str()))
        });
        if let Some(task) = removed.and_then(|state| state.event_task) {
            task.stop().await;
        }
    }
}

#[derive(Clone)]
struct WorkspaceStateRef {
    client: Arc<SharedClient>,
    auth: Auth,
}

impl Provider for FeishuProvider {
    fn capabilities() -> &'static [&'static str] {
        &[PROVIDER_CAPABILITY_TASK_SYSTEM]
    }

    fn reaction_catalog(&self) -> Vec<lankide_provider::ReactionDefinition> {
        crate::reactions::management_catalog()
    }
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ProviderId::new("feishu").expect("static provider id is valid"),
            name: "Feishu".into(),
            description: "Feishu messaging, events, reactions, attachments, and Tasks".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            icon: "feishu".into(),
        }
    }

    fn channels<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async move {
            let mut channels = vec![
                ChannelDefinition::new(
                    channel("group_chat"),
                    "Feishu group chat",
                    ChannelDefaultAttention::Follow,
                    [
                        AttentionLevel::Follow,
                        AttentionLevel::Schedule,
                        AttentionLevel::Mention,
                    ],
                    [
                        ChannelAbility::Reply,
                        ChannelAbility::Attachment,
                        ChannelAbility::Reaction,
                        ChannelAbility::Recall,
                    ],
                )?,
                ChannelDefinition::new(
                    channel("private_chat"),
                    "Feishu private chat",
                    ChannelDefaultAttention::DirectMessage,
                    [],
                    [
                        ChannelAbility::Reply,
                        ChannelAbility::Attachment,
                        ChannelAbility::Reaction,
                        ChannelAbility::Recall,
                    ],
                )?,
            ];
            if crate::agent_tasks::task_system_enabled(context.workspace_config()) {
                channels.push(ChannelDefinition::new(
                    channel("task"),
                    "Feishu task",
                    ChannelDefaultAttention::Follow,
                    [],
                    [],
                )?);
            }
            Ok(channels)
        })
    }

    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            context
                .register_interface(InterfaceDefinition {
                    id: CONSOLE_LOGIN_INTERFACE.into(),
                    input_schema: JsonSchema::new(serde_json::json!({
                        "type": "object",
                        "properties": {"action": {"enum": ["status", "start", "poll", "switch_start", "switch_poll", "switch_cancel"]}}
                    }))?,
                    output_schema: JsonSchema::new(serde_json::json!({
                        "type": "object",
                        "required": ["state"],
                        "properties": {
                            "state": {"enum": ["ready", "required", "waiting", "scanned"]},
                            "qr_code": {"type": "string"},
                            "poll_interval_seconds": {"type": "integer", "minimum": 1}
                        }
                    }))?,
                    handler: Arc::new(ConsoleLoginHandler {
                        switch_lock: AsyncMutex::new(()),
                        context: context.clone(),
                        transport: self.transport.clone(),
                    }),
                })
                .await?;
            for (id, handler) in [
                (
                    "feishu.workspace.owner",
                    Arc::new(crate::workspace_owner::OwnerHandler {
                        context: context.clone(),
                        transport: self.transport.clone(),
                    }) as Arc<dyn InvocationHandler>,
                ),
                (
                    "feishu.workspace.tasks",
                    Arc::new(crate::workspace_tasks::TasksHandler {
                        context: context.clone(),
                        transport: self.transport.clone(),
                    }) as Arc<dyn InvocationHandler>,
                ),
            ] {
                context
                    .register_interface(InterfaceDefinition {
                        id: id.into(),
                        input_schema: JsonSchema::new(
                            serde_json::json!({"type":"object","required":["action"]}),
                        )?,
                        output_schema: JsonSchema::new(serde_json::json!({"type":"object"}))?,
                        handler,
                    })
                    .await?;
            }
            let workspace_key = context.workspace_id().to_string();
            if let Ok(mut contexts) = self.contexts.lock() {
                contexts.insert(workspace_key, context);
            }
            Ok(())
        })
    }

    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult> {
        Box::pin(async move {
            let state = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            if context.idempotency_key.trim().is_empty() {
                return Err(ProviderError::InvalidInput {
                    field: "idempotency_key",
                });
            }
            let idempotency_key = context.idempotency_key.clone();
            let mut chat_id = context.chat_id.clone();
            let direct_recipient = if let Some(id) = chat_id.strip_prefix("person:") {
                if context.channel_id.as_str() != "private_chat" || context.reply_to.is_some() {
                    return Err(ProviderError::InvalidInput {
                        field: "recipient_id: 请选择成员并发送新私信",
                    });
                }
                let id =
                    lankide_core::PersonId::new(id).map_err(|_| ProviderError::InvalidInput {
                        field: "recipient_id",
                    })?;
                let person = context.storage().person(&id).await?;
                if person.team_role == lankide_provider::TeamRole::External {
                    return Err(ProviderError::InvalidInput {
                        field: "recipient_id: 请选择团队成员",
                    });
                }
                let open_id = crate::identity::person_open_id(
                    context.storage(),
                    person,
                    &crate::identity::load(context.storage())
                        .await?
                        .1
                        .agent_app_id(&context.agent_id)
                        .ok_or(ProviderError::InvalidInput {
                            field: "sender_app_id",
                        })?
                        .to_owned(),
                    state.client.clone(),
                    state.auth,
                )
                .await?;
                chat_id = open_id;
                true
            } else {
                false
            };
            let content = context.content.clone();
            let mentions = context.mentions.clone();
            let reply_to = context.reply_to;
            let attachments = resolved_attachments(context.storage(), &context.attachments).await?;
            if content.trim().is_empty() && attachments.is_empty() {
                return Err(ProviderError::InvalidInput { field: "content" });
            }
            let targets = feishu_mention_targets(
                context.storage(),
                &context.agent_id,
                &mentions,
                &content,
                state.client.clone(),
                state.auth,
            )
            .await?;
            let (rendered, _) = render_card_markdown(&content, &targets);
            let provider_reply_id =
                resolve_reply_id(context.storage(), &context.channel_id, &chat_id, reply_to)
                    .await?;
            let receive_id_type = if direct_recipient {
                "open_id".to_owned()
            } else {
                receive_id_type(&context.provider_config, &chat_id)
            };
            let auth = state.auth;
            let client = state.client;
            let mut part_ids = Vec::new();
            let mut sent_chat = None;
            let sent = (async {
                let mut primary = None;
                if !rendered.trim().is_empty() {
                    let result = if let Some(reply_id) = provider_reply_id.as_deref() {
                        let mut reply = ReplyMessage::markdown(reply_id, rendered);
                        reply.idempotency_key =
                            Some(feishu_send_part_key(&idempotency_key, "content"));
                        client.reply_message(&reply, auth).await
                    } else {
                        let mut message = SendMessage::markdown(
                            chat_id.clone(),
                            receive_id_type.clone(),
                            rendered,
                        );
                        message.idempotency_key =
                            Some(feishu_send_part_key(&idempotency_key, "content"));
                        client.send_message(&message, auth).await
                    }?;
                    sent_chat.get_or_insert_with(|| result.chat_id.clone());
                    part_ids.push(result.message_id.clone());
                    primary = Some(result);
                }
                for (index, attachment) in attachments.into_iter().enumerate() {
                    let uploaded = if attachment.kind == "image" {
                        client
                            .upload_image(&attachment.bytes, &attachment.file_name, "message", auth)
                            .await?
                    } else {
                        client
                            .upload_file(
                                &attachment.bytes,
                                &attachment.file_name,
                                feishu_file_type(&attachment.file_name, &attachment.kind),
                                None,
                                auth,
                            )
                            .await?
                    };
                    let result = if let Some(reply_id) = provider_reply_id.as_deref() {
                        let mut reply = if attachment.kind == "image" {
                            ReplyMessage::image(reply_id, uploaded.key)
                        } else {
                            ReplyMessage::file(reply_id, uploaded.key)
                        };
                        reply.idempotency_key = Some(feishu_send_part_key(
                            &idempotency_key,
                            &format!("attachment:{index}"),
                        ));
                        client.reply_message(&reply, auth).await?
                    } else {
                        let mut message = if attachment.kind == "image" {
                            SendMessage::image(
                                chat_id.clone(),
                                receive_id_type.clone(),
                                uploaded.key,
                            )
                        } else {
                            SendMessage::file(
                                chat_id.clone(),
                                receive_id_type.clone(),
                                uploaded.key,
                            )
                        };
                        message.idempotency_key = Some(feishu_send_part_key(
                            &idempotency_key,
                            &format!("attachment:{index}"),
                        ));
                        client.send_message(&message, auth).await?
                    };
                    sent_chat.get_or_insert_with(|| result.chat_id.clone());
                    part_ids.push(result.message_id.clone());
                    primary.get_or_insert(result);
                }
                primary.ok_or(crate::http::ClientError::InvalidInput(
                    "Feishu message payload is empty".to_owned(),
                ))
            })
            .await;
            match sent {
                Ok(primary) => {
                    let mut result = SendMessageResult::sent_parts(part_ids)?;
                    if direct_recipient {
                        result.provider_chat_id = Some(primary.chat_id);
                    }
                    Ok(result)
                }
                Err(error) => {
                    if let Some(init) =
                        self.contexts.lock().ok().and_then(|contexts| {
                            contexts.get(context.workspace_id.as_str()).cloned()
                        })
                    {
                        init.logger().log(
                            lankide_provider::LogLevel::Error,
                            &format!("Feishu send failed: {error:?}"),
                        );
                    }
                    if !part_ids.is_empty() || is_unknown_send_error(&error) {
                        let mut result = SendMessageResult::unknown();
                        result.provider_message_ids = part_ids;
                        if direct_recipient {
                            result.provider_chat_id = sent_chat;
                        }
                        Ok(result)
                    } else {
                        Err(ProviderError::External {
                            operation: "feishu_send_message",
                            retryable: false,
                        })
                    }
                }
            }
        })
    }

    fn recall_message<'a>(&'a self, context: RecallMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let state = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let message_id = context.provider_message_id;
            state
                .client
                .recall_message(&message_id, state.auth)
                .await
                .map_err(|_| ProviderError::External {
                    operation: "feishu_recall_message",
                    retryable: false,
                })
        })
    }

    fn auto_reaction<'a>(&'a self, context: AutoReactionContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let state = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let emoji = context.auto_reaction.clone();
            if emoji.trim().is_empty() || context.trigger_messages.is_empty() {
                return Err(ProviderError::InvalidInput {
                    field: "auto_reaction",
                });
            }
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
                .ok_or(ProviderError::InvalidInput {
                    field: "trigger_messages",
                })?;
            state
                .client
                .add_reaction(&target, &emoji, state.auth)
                .await
                .map(|_| ())
                .map_err(|_| ProviderError::External {
                    operation: "feishu_auto_reaction",
                    retryable: false,
                })
        })
    }

    fn system_prompt<'a>(&'a self, context: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        Box::pin(async move { Ok(crate::agent_tasks::system_prompt(&context)) })
    }

    fn tools<'a>(
        &'a self,
        context: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<ToolDefinition>> {
        Box::pin(async move {
            let state = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let mut tools =
                crate::reactions::tools(&context, state.client.clone(), state.auth).await?;
            tools.push(crate::person_profile::tool(
                &context,
                state.client.clone(),
                state.auth,
            )?);
            if crate::agent_tasks::task_system_enabled(&context.workspace_config) {
                let _identity_guard = self.identity_lock.lock().await;
                tools.extend(
                    crate::agent_tasks::tools(
                        &context,
                        state.client,
                        state.auth,
                        self.identity_lock.clone(),
                    )
                    .await?,
                );
            }
            Ok(tools)
        })
    }

    fn create_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let config = &context.new_provider_agent_config;
            let app_secret = match context.provider_secrets().read("app_secret").await {
                Ok(value) => value.expose_secret().trim().to_owned(),
                Err(ProviderError::SecretAccessDenied)
                    if config_text(config, "mode").as_deref() == Some("new") =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            let saved_state = context.storage().agent_state(&context.agent_id).await?;
            let app_id = config_text(config, "app_id")
                .or_else(|| config_text(config, "provider_handle"))
                .ok_or(ProviderError::InvalidInput { field: "app_id" })?;
            if app_secret.is_empty() {
                return Err(ProviderError::InvalidInput {
                    field: "app_secret",
                });
            }
            let domain = configured_domain(config, &context.workspace_config)?;
            let credentials = Credentials::app(app_id.clone(), app_secret);
            let client = Arc::new(
                SharedClient::new(self.transport.clone(), credentials.clone())
                    .map_err(|_| ProviderError::InvalidInput {
                        field: "feishu_credentials",
                    })?
                    .with_domain(domain.clone()),
            );
            let verify_client = client.clone();
            let bot = (async move {
                verify_client.verify_credentials().await?;
                Ok::<_, crate::http::ClientError>(verify_client.bot_info(Auth::Tenant).await.ok())
            })
            .await
            .map_err(|_| ProviderError::InvalidInput {
                field: "feishu_credentials",
            })?;
            let mut provider_state = saved_state;
            let provider_state_object =
                provider_state
                    .as_object_mut()
                    .ok_or(ProviderError::InvalidInput {
                        field: "agent_provider_state",
                    })?;
            provider_state_object.insert("app_id".into(), serde_json::json!(app_id.clone()));
            if let Some(bot) = bot.as_ref() {
                provider_state_object
                    .insert("open_id".into(), serde_json::json!(bot.open_id.clone()));
            }
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
                    &app_id,
                    bot.as_ref().map(|value| value.open_id.as_str()),
                    &context.name,
                );
                crate::identity::save(context.storage(), workspace_state, &directory).await?;
            }
            let init_context = self
                .contexts
                .lock()
                .ok()
                .and_then(|contexts| contexts.get(&context.workspace_id.to_string()).cloned())
                .ok_or(ProviderError::NotFound {
                    entity: "workspace",
                })?;
            let mut event_task = None;
            if let Some(factory) = self.event_source_factory.clone() {
                let source_workspace_id = context.workspace_id.clone();
                let source_credentials = credentials.clone();
                let source_domain = domain.clone();
                let source = factory(
                    &source_workspace_id,
                    &source_credentials,
                    &source_domain,
                    self.transport.as_ref(),
                )
                .await
                .map_err(|_| ProviderError::External {
                    operation: "feishu_event_source",
                    retryable: true,
                })?;
                let normalization = normalization_config(&init_context, &app_id).await?;
                let mut dispatcher = Dispatcher::new(source, normalization.clone());
                for event_type in FEISHU_EVENT_TYPES.into_iter().filter(|event_type| {
                    *event_type != "task.*"
                        || crate::agent_tasks::task_system_enabled(init_context.workspace_config())
                }) {
                    dispatcher
                        .register(event_type)
                        .map_err(|_| ProviderError::External {
                            operation: "feishu_event_registration",
                            retryable: false,
                        })?;
                }
                event_task = Some(spawn_dispatcher(
                    init_context,
                    client.clone(),
                    Auth::Tenant,
                    dispatcher,
                    factory,
                    self.transport.clone(),
                    source_workspace_id,
                    source_credentials,
                    source_domain,
                    normalization,
                    self.identity_lock.clone(),
                    self.inbound_resource_cache.clone(),
                ));
            }
            self.replace_state(
                agent_state_key(&context.workspace_id.to_string(), context.agent_id.as_str()),
                WorkspaceState {
                    client,
                    auth: Auth::Tenant,
                    event_task,
                },
            );
            Ok(())
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

    fn update_agent<'a>(
        &'a self,
        context: lankide_provider::AgentContext,
    ) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let state = self.state(&context.workspace_id.to_string(), &context.agent_id)?;
            let profile = context.new_provider_agent_config.clone();
            let display_name = context.name.trim().to_owned();
            let description = context.bio.trim().to_owned();
            let language = config_text(&profile, "language")
                .or_else(|| config_text(&context.workspace_config, "language"))
                .unwrap_or_else(|| match state.client.domain() {
                    Domain::Lark => "en_us".to_owned(),
                    Domain::Feishu | Domain::Custom(_) => "zh_cn".to_owned(),
                });
            let avatar_url = config_text(&profile, "avatar_url").or_else(|| {
                context
                    .avatar
                    .as_deref()
                    .filter(|value| value.starts_with("https://") || value.starts_with("http://"))
                    .map(str::to_owned)
            });
            let avatar_bytes = parse_byte_array(&profile, "avatar_bytes");
            if display_name.is_empty()
                && description.is_empty()
                && avatar_url.is_none()
                && avatar_bytes.is_none()
            {
                return Ok(());
            }
            let avatar_file_name = config_text(&profile, "avatar_file_name")
                .unwrap_or_else(|| "avatar.jpg".to_owned());
            let client = state.client;
            let auth = state.auth;
            (async move {
                let uploaded_avatar = if let Some(bytes) = avatar_bytes.as_deref() {
                    Some(
                        client
                            .upload_app_avatar(bytes, &avatar_file_name, auth)
                            .await?,
                    )
                } else {
                    None
                };
                let avatar = uploaded_avatar.as_deref().or(avatar_url.as_deref());
                client
                    .update_application_profile(
                        &display_name,
                        &description,
                        avatar,
                        &language,
                        auth,
                    )
                    .await?;
                client
                    .publish_application(
                        "Lankide Agent profile update",
                        "Update Agent name or avatar",
                        auth,
                    )
                    .await?;
                Ok::<(), crate::http::ClientError>(())
            })
            .await
            .map_err(|_| ProviderError::External {
                operation: "feishu_update_profile",
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
    ChannelId::new(value).expect("static Feishu channel ID is valid")
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

async fn normalization_config(
    context: &InitContext,
    app_id: &str,
) -> Result<NormalizationConfig, ProviderError> {
    let (_, directory) = crate::identity::load(context.storage()).await?;
    Ok(directory.normalization_config(app_id, ""))
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
fn spawn_dispatcher(
    context: InitContext,
    client: Arc<SharedClient>,
    auth: Auth,
    mut dispatcher: Dispatcher<Box<dyn EventSource>>,
    factory: Arc<EventSourceFactory>,
    transport: SharedTransport,
    workspace_id: lankide_core::WorkspaceId,
    credentials: Credentials,
    domain: Domain,
    normalization: NormalizationConfig,
    identity_lock: Arc<AsyncMutex<()>>,
    inbound_resource_cache: Arc<AsyncMutex<InboundResourceCache>>,
) -> EventTask {
    let (shutdown, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let retry = EventRetry::configured(&context);
        let mut failures = 0;
        let mut sink = FeishuEventBridge {
            context: context.clone(),
            client,
            auth,
            identity_lock,
            inbound_resource_cache,
            human_names: BTreeMap::new(),
        };
        loop {
            let result = tokio::select! {
                _ = &mut stopped => break,
                _ = context.shutdown_signal().cancelled() => break,
                result = dispatcher.poll_once(&mut sink) => result,
            };
            match result {
                Ok(_) => {
                    failures = 0;
                }
                Err(DispatchError::Sink(error)) => {
                    failures += 1;
                    if !error.retryable || retry.exhausted(failures) {
                        context.logger().log(
                            lankide_provider::LogLevel::Warn,
                            &format!("Feishu event skipped: {error}"),
                        );
                        // Mark the skip before sending its receipt. An ack retry must not
                        // process a rejected event again, and one event must not hold
                        // back the ones after it.
                        dispatcher.discard();
                        failures = 0;
                    } else {
                        let delay = retry.delay(failures);
                        context.logger().log(
                            lankide_provider::LogLevel::Warn,
                            &format!(
                                "Feishu event acceptance failed; retry in {}ms: {error}",
                                delay.as_millis()
                            ),
                        );
                        tokio::select! { _ = &mut stopped => break, _ = context.shutdown_signal().cancelled() => break, _ = tokio::time::sleep(delay) => {} }
                    }
                }
                Err(DispatchError::Normalize(error)) => {
                    context.logger().log(
                        lankide_provider::LogLevel::Warn,
                        &format!("Feishu event skipped: {error}"),
                    );
                    dispatcher.discard();
                }
                Err(DispatchError::Source(SourceError::Malformed(error))) => {
                    context.logger().log(
                        lankide_provider::LogLevel::Warn,
                        &format!("Feishu malformed event skipped: {error}"),
                    );
                    dispatcher.discard();
                }
                Err(error) => {
                    context.logger().log(
                        lankide_provider::LogLevel::Warn,
                        &format!("Feishu event receiver disconnected: {error}"),
                    );
                    dispatcher.stop().await;
                    let mut reconnect_failures = 1;
                    let source = loop {
                        let delay = retry.delay(reconnect_failures);
                        tokio::select! { _ = &mut stopped => return, _ = context.shutdown_signal().cancelled() => return, _ = tokio::time::sleep(delay) => {} }
                        let result = tokio::select! {
                            _ = &mut stopped => return,
                            _ = context.shutdown_signal().cancelled() => return,
                            result = factory(&workspace_id, &credentials, &domain, transport.as_ref()) => result,
                        };
                        match result {
                            Ok(source) => break source,
                            Err(error) => {
                                context.logger().log(
                                    lankide_provider::LogLevel::Warn,
                                    &format!("Feishu event reconnect failed: {error}"),
                                );
                                reconnect_failures += 1;
                            }
                        }
                    };
                    dispatcher = Dispatcher::new(source, normalization.clone());
                    for event_type in FEISHU_EVENT_TYPES.into_iter().filter(|event_type| {
                        *event_type != "task.*"
                            || crate::agent_tasks::task_system_enabled(context.workspace_config())
                    }) {
                        if dispatcher.register(event_type).is_err() {
                            return;
                        }
                    }
                    failures = 0;
                }
            }
        }
        dispatcher.stop().await;
    });
    EventTask { task, shutdown }
}

struct FeishuEventBridge {
    context: InitContext,
    client: Arc<SharedClient>,
    auth: Auth,
    identity_lock: Arc<AsyncMutex<()>>,
    inbound_resource_cache: Arc<AsyncMutex<InboundResourceCache>>,
    human_names: BTreeMap<String, (String, Instant)>,
}

impl NormalizedEventSink for FeishuEventBridge {
    fn emit(
        &mut self,
        event: NormalizedEvent,
    ) -> crate::events::EventFuture<'_, Result<(), crate::events::SinkError>> {
        Box::pin(async move {
            if let NormalizedEvent::Task(task) = &event
                && task.event_types.is_empty()
            {
                return Ok(());
            }
            let event = if let NormalizedEvent::Message(mut message) = event {
                if message.sender.kind == SenderKind::Bot {
                    let record = self
                        .client
                        .get_message(&message.provider_message_id, self.auth)
                        .await
                        .map_err(crate::events::SinkError::from)?;
                    if let Some(sender) = record.sender
                        && sender.sender_type == "app"
                        && sender.id_type == "app_id"
                        && !sender.id.trim().is_empty()
                    {
                        message
                            .sender
                            .aliases
                            .push(("app_id".to_owned(), sender.id));
                    }
                }
                let app_id = message.app_id.clone();
                let tenant_key = message.tenant_key.clone();
                let raw = message.raw.clone();
                let _identity_guard = self.identity_lock.lock().await;
                let (workspace_state, mut directory) =
                    crate::identity::load(self.context.storage())
                        .await
                        .map_err(crate::events::SinkError::from)?;
                directory.learn_message(&message);
                crate::identity::save(self.context.storage(), workspace_state, &directory)
                    .await
                    .map_err(crate::events::SinkError::from)?;
                let normalization = directory.normalization_config(&app_id, &tenant_key);
                crate::normalize::normalize_event(&raw, &normalization)
                    .map_err(crate::events::SinkError::from)?
            } else {
                event
            };
            let _task_guard = if matches!(&event, NormalizedEvent::Task(_)) {
                Some(self.identity_lock.lock().await)
            } else {
                None
            };
            let Some(provider_event) = normalized_event_to_provider(
                &self.context,
                &self.client,
                self.auth,
                &self.inbound_resource_cache,
                &mut self.human_names,
                event,
            )
            .await?
            else {
                return Ok(());
            };
            self.context
                .event_sink()
                .emit(provider_event)
                .await
                .map_err(|error| crate::events::SinkError::from(error))
        })
    }
}

async fn receiving_agent(
    context: &InitContext,
    app_id: &str,
) -> Result<Option<AgentId>, crate::events::SinkError> {
    let (_, directory) = crate::identity::load(context.storage())
        .await
        .map_err(crate::events::SinkError::from)?;
    let mut receivers = directory
        .agents
        .iter()
        .filter(|(_, identity)| identity.app_id == app_id)
        .filter_map(|(id, _)| AgentId::new(id.clone()).ok());
    let receiver = receivers.next();
    // An ambiguous account binding must not leak a DM to either Agent.
    Ok(if receivers.next().is_none() {
        receiver
    } else {
        None
    })
}

async fn normalized_event_to_provider(
    context: &InitContext,
    client: &Arc<SharedClient>,
    auth: Auth,
    inbound_resource_cache: &Arc<AsyncMutex<InboundResourceCache>>,
    human_names: &mut BTreeMap<String, (String, Instant)>,
    event: NormalizedEvent,
) -> Result<Option<ProviderEvent>, crate::events::SinkError> {
    match event {
        NormalizedEvent::Message(mut message) => {
            let sender = resolved_sender(
                context,
                client,
                auth,
                human_names,
                &message.sender.kind,
                &message.sender.sender_id,
                &message.sender.display_name,
                &message.sender.aliases,
                &message.app_id,
                &message.tenant_key,
            )
            .await?;
            for mention in &message.mentions {
                if mention.kind != SenderKind::Human {
                    continue;
                }
                let resolved = resolved_sender(
                    context,
                    client,
                    auth,
                    human_names,
                    &mention.kind,
                    &mention.external_id,
                    &mention.display_name,
                    &mention.aliases,
                    &message.app_id,
                    &message.tenant_key,
                )
                .await?;
                if let (Some(external_marker), Some(person_marker)) = (
                    markdown_mention(&mention.display_name, &mention.external_id),
                    markdown_mention(&resolved.name, &resolved.id),
                ) {
                    message.content = message.content.replace(&external_marker, &person_marker);
                }
            }
            let mut attachments = Vec::with_capacity(message.attachments.len());
            for attachment in &message.attachments {
                let cache_key = format!(
                    "{}:{}:{}",
                    message.provider_message_id, attachment.kind, attachment.key
                );
                let mut cache = inbound_resource_cache.lock().await;
                if let Some(resource) = cache.get(&cache_key) {
                    attachments.push(resource);
                    continue;
                }
                let downloaded = match client
                    .download_message_resource(
                        &message.provider_message_id,
                        &attachment.key,
                        if attachment.kind == "image" {
                            "image"
                        } else {
                            "file"
                        },
                        auth,
                    )
                    .await
                {
                    Ok(downloaded) => downloaded,
                    Err(_) => {
                        context.logger().log(
                            lankide_provider::LogLevel::Warn,
                            "A Feishu attachment could not be downloaded; the message was accepted without that attachment",
                        );
                        continue;
                    }
                };
                let (file_name, media_type) =
                    inbound_media_metadata(&downloaded, &attachment.name, &attachment.content_type);
                let resource = match context
                    .save_resource(ResourceUpload {
                        file_name,
                        media_type,
                        bytes: downloaded.data,
                    })
                    .await
                {
                    Ok(resource) => resource,
                    Err(_) => {
                        context.logger().log(
                            lankide_provider::LogLevel::Warn,
                            "A Feishu attachment could not be saved; the message was accepted without that attachment",
                        );
                        continue;
                    }
                };
                cache.insert(cache_key, resource.clone());
                attachments.push(resource);
            }
            let mentioned_agent_ids = message
                .mentioned_agent_ids
                .iter()
                .filter_map(|value| AgentId::new(value.clone()).ok())
                .collect();
            let mut provider_data =
                serde_json::to_value(&message).map_err(crate::events::SinkError::from)?;
            if message.channel_id == "group_chat" {
                let chat = client
                    .get_chat(&message.chat_id, auth)
                    .await
                    .map_err(crate::events::SinkError::from)?;
                if !chat.name.trim().is_empty() {
                    provider_data["chat_name"] = serde_json::json!(chat.name);
                }
            }
            let receiving_agent_id = receiving_agent(context, &message.app_id).await?;
            Ok(Some(ProviderEvent {
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
            }))
        }
        NormalizedEvent::Reaction(reaction) => {
            let sender = resolved_sender(
                context,
                client,
                auth,
                human_names,
                &reaction.sender.kind,
                &reaction.sender.sender_id,
                &reaction.sender.display_name,
                &reaction.sender.aliases,
                &reaction.app_id,
                &reaction.tenant_key,
            )
            .await?;
            let provider_data =
                serde_json::to_value(&reaction).map_err(crate::events::SinkError::from)?;
            let action = reaction.action;
            // Feishu reaction callbacks omit chat identity, and the message
            // lookup omits the chat kind. Resolve both through the same App
            // before choosing a channel or recipient.
            let target = client
                .get_message(&reaction.target_provider_message_id, auth)
                .await
                .map_err(crate::events::SinkError::from)?;
            let chat = client
                .get_chat(&target.chat_id, auth)
                .await
                .map_err(crate::events::SinkError::from)?;
            let direct = chat.chat_mode == "p2p";
            let receiving_agent_id = if direct {
                receiving_agent(context, &reaction.app_id).await?
            } else {
                None
            };
            let provider_chat_id = target.chat_id;
            Ok(Some(ProviderEvent {
                receiving_agent_id,
                channel_id: channel(if direct { "private_chat" } else { "group_chat" }),
                provider_chat_id,
                sender: sender.clone(),
                kind: ProviderEventKind::Reaction(ReactionEvent {
                    target_provider_message_id: reaction.target_provider_message_id,
                    sender,
                    reaction: reaction.reaction,
                    action,
                }),
                provider_data,
            }))
        }
        NormalizedEvent::ChatUpdated(update) => Ok(Some(ProviderEvent {
            receiving_agent_id: None,
            channel_id: channel("group_chat"),
            provider_chat_id: update.chat_id.clone(),
            sender: EventSender {
                id: "feishu:system".to_owned(),
                kind: EventSenderType::System,
                name: "Feishu".to_owned(),
            },
            kind: ProviderEventKind::Custom(lankide_provider::CustomEvent {
                mentioned_agent_ids: Vec::new(),
                reason: "chat_updated".to_owned(),
                target_agent_ids: Vec::new(),
                subject_id: update.chat_id.clone(),
                subject_name: update.name.clone(),
                data: serde_json::to_value(&update).map_err(crate::events::SinkError::from)?,
            }),
            provider_data: serde_json::to_value(&update).map_err(crate::events::SinkError::from)?,
        })),
        NormalizedEvent::Task(task) => {
            if crate::agent_tasks::task_system_enabled(context.workspace_config()) {
                context
                    .storage()
                    .management_changed()
                    .await
                    .map_err(crate::events::SinkError::from)?;
            }
            let Some((sender, custom)) =
                crate::agent_tasks::task_event(context, client.clone(), auth, &task)
                    .await
                    .map_err(crate::events::SinkError::from)?
            else {
                return Ok(None);
            };
            Ok(Some(ProviderEvent {
                receiving_agent_id: None,
                channel_id: channel("task"),
                provider_chat_id: "tasks".into(),
                sender,
                kind: ProviderEventKind::Custom(custom),
                provider_data: serde_json::to_value(&task)
                    .map_err(crate::events::SinkError::from)?,
            }))
        }
    }
}

fn sender(kind: &SenderKind, id: &str, name: &str) -> EventSender {
    EventSender {
        id: if id.trim().is_empty() {
            "feishu:unknown".to_owned()
        } else {
            id.to_owned()
        },
        kind: match kind {
            SenderKind::Human => EventSenderType::Human,
            SenderKind::Agent => EventSenderType::Agent,
            SenderKind::Bot => EventSenderType::Bot,
            SenderKind::System | SenderKind::Unknown => EventSenderType::System,
        },
        name: name
            .trim()
            .is_empty()
            .then(|| match kind {
                SenderKind::Human => "飞书成员",
                SenderKind::Agent | SenderKind::Bot => "Agent",
                SenderKind::System | SenderKind::Unknown => "飞书",
            })
            .unwrap_or(name)
            .to_owned(),
    }
}

async fn resolved_sender(
    context: &InitContext,
    client: &SharedClient,
    auth: Auth,
    human_names: &mut BTreeMap<String, (String, Instant)>,
    kind: &SenderKind,
    id: &str,
    name: &str,
    aliases: &[(String, String)],
    app_id: &str,
    tenant_key: &str,
) -> Result<EventSender, crate::events::SinkError> {
    if *kind != SenderKind::Human {
        return Ok(sender(kind, id, name));
    }
    let alias = |kind: &str| {
        aliases
            .iter()
            .find_map(|(candidate, value)| (candidate == kind).then_some(value.trim()))
            .filter(|value| !value.is_empty())
    };
    let external_id = alias("union_id")
        .or_else(|| alias("user_id"))
        .or_else(|| alias("open_id"))
        .or_else(|| (!id.trim().is_empty()).then_some(id.trim()))
        .ok_or_else(|| crate::events::SinkError::permanent("missing Feishu human identity"))?;
    let external_id_type = if alias("union_id") == Some(external_id) {
        "union_id"
    } else if alias("user_id") == Some(external_id) {
        "user_id"
    } else {
        "open_id"
    };
    let mut person = context
        .storage()
        .get_or_create_person(external_id)
        .await
        .map_err(crate::events::SinkError::from)?;

    for _ in 0..3 {
        let mut state = person
            .provider_state
            .as_object()
            .cloned()
            .unwrap_or_default();
        if !tenant_key.trim().is_empty() {
            state.insert(
                "tenant_key".to_owned(),
                serde_json::json!(tenant_key.trim()),
            );
        }
        state.insert(
            "external_id_type".to_owned(),
            serde_json::json!(external_id_type),
        );
        if let Some(value) = alias("user_id") {
            state.insert("user_id".to_owned(), serde_json::json!(value));
        }
        let open_id = alias("open_id");
        let mut open_ids = state
            .remove("open_ids_by_app")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        if let Some(open_id) = open_id
            && !app_id.trim().is_empty()
        {
            open_ids.insert(app_id.trim().to_owned(), serde_json::json!(open_id));
        }
        state.insert(
            "open_ids_by_app".to_owned(),
            serde_json::Value::Object(open_ids),
        );
        let next_state = serde_json::Value::Object(state);
        if next_state == person.provider_state {
            break;
        }
        match context
            .storage()
            .compare_and_set_person_state(&person.person_id, person.state_revision, next_state)
            .await
        {
            Ok(updated) => {
                person = updated;
                break;
            }
            Err(_) => {
                person = context
                    .storage()
                    .person(&person.person_id)
                    .await
                    .map_err(crate::events::SinkError::from)?;
            }
        }
    }
    let current_open_id = alias("open_id").or_else(|| {
        person
            .provider_state
            .get("open_ids_by_app")
            .and_then(serde_json::Value::as_object)
            .and_then(|values| values.get(app_id))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    });
    let cache_key = current_open_id
        .map(|open_id| format!("{app_id}:open_id:{open_id}"))
        .unwrap_or_else(|| format!("{app_id}:{external_id_type}:{external_id}"));
    let display_name = if let Some((cached, observed_at)) = human_names.get(&cache_key)
        && observed_at.elapsed() < Duration::from_secs(2 * 60 * 60)
    {
        cached.clone()
    } else {
        let queried = match current_open_id {
            Some(open_id) => client.get_user(open_id, auth).await,
            None => {
                client
                    .get_user_by_id(&person.external_id, external_id_type, auth)
                    .await
            }
        }
        .ok()
        .map(|user| {
            if user.nickname.trim().is_empty() {
                user.name
            } else {
                user.nickname
            }
        })
        .filter(|value| !value.trim().is_empty());
        if let Some(queried) = queried {
            human_names.insert(cache_key, (queried.clone(), Instant::now()));
            queried
        } else if !name.trim().is_empty() {
            name.trim().to_owned()
        } else {
            "飞书成员".to_owned()
        }
    };
    Ok(sender(kind, person.person_id.as_str(), &display_name))
}

async fn feishu_mention_targets(
    storage: &dyn ProviderStorage,
    sender_agent_id: &AgentId,
    mentions: &[AgentId],
    content: &str,
    client: Arc<SharedClient>,
    auth: Auth,
) -> Result<Vec<MentionTarget>, ProviderError> {
    let (_, directory) = crate::identity::load(storage).await?;
    if has_bare_mention(
        content,
        directory
            .agents
            .values()
            .map(|agent| agent.display_name.as_str()),
    ) {
        return Err(ProviderError::InvalidInput {
            field: BARE_MENTION,
        });
    }
    let sender_app_id =
        directory
            .agent_app_id(sender_agent_id)
            .ok_or(ProviderError::InvalidInput {
                field: "agent_provider_state.app_id",
            })?;
    let mut ids = mentions.iter().map(ToString::to_string).collect::<Vec<_>>();
    for id in extract_agent_ids(content) {
        if !ids.iter().any(|value| value == &id) {
            ids.push(id);
        }
    }
    let mut targets = Vec::new();
    for id in ids {
        if let Ok(agent_id) = AgentId::new(id.clone())
            && let Some((external_id, name)) = directory.mention_target(sender_app_id, &agent_id)
        {
            targets.push(MentionTarget::new(id, external_id, name));
            continue;
        }
        if directory.agents.contains_key(&id) {
            return Err(ProviderError::InvalidInput {
                field: "mention: 无法提及该成员，请先确认飞书身份",
            });
        }
        let person_id =
            lankide_core::PersonId::new(id.clone()).map_err(|_| ProviderError::InvalidInput {
                field: "mention: 请使用 @名称[成员ID]",
            })?;
        let person = storage.person(&person_id).await?;
        let open_id =
            crate::identity::person_open_id(storage, person, sender_app_id, client.clone(), auth)
                .await?;
        targets.push(MentionTarget::new(id, open_id, ""));
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

/// Feishu's file upload API accepts a small set of semantic file types.  The
/// Provider attachment kind is intentionally broader (`file`, `audio`, etc.)
/// and is used to choose the outgoing message type; it is not a valid
/// `file_type` value for the upload endpoint.  Derive the API value from the
/// filename, matching the local Lark SDK behavior.
fn feishu_file_type(file_name: &str, kind: &str) -> &'static str {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("opus") | Some("ogg") => "opus",
        Some("mp4") => "mp4",
        Some("pdf") => "pdf",
        Some("doc") | Some("docx") => "doc",
        Some("xls") | Some("xlsx") => "xls",
        Some("ppt") | Some("pptx") => "ppt",
        _ => match kind.trim().to_ascii_lowercase().as_str() {
            "opus" => "opus",
            "mp4" => "mp4",
            "pdf" => "pdf",
            "doc" | "docx" => "doc",
            "xls" | "xlsx" => "xls",
            "ppt" | "pptx" => "ppt",
            _ => "stream",
        },
    }
}

async fn resolve_reply_id(
    storage: &dyn ProviderStorage,
    channel_id: &ChannelId,
    chat_id: &str,
    reply_to: Option<lankide_core::MessageId>,
) -> Result<Option<String>, ProviderError> {
    let Some(reply_to) = reply_to else {
        return Ok(None);
    };
    let target = storage.message(reply_to).await?;
    if &target.channel_id != channel_id || target.chat_id != chat_id {
        return Err(ProviderError::InvalidInput { field: "reply_to" });
    }
    target
        .provider_message_id
        .ok_or(ProviderError::InvalidInput { field: "reply_to" })
        .map(Some)
}

fn receive_id_type(config: &Value, chat_id: &str) -> String {
    config
        .get("receive_id_type")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "chat_id" | "open_id" | "union_id" | "user_id"))
        .unwrap_or_else(|| {
            if chat_id.starts_with("ou_") || chat_id.starts_with("on_") {
                "open_id"
            } else {
                "chat_id"
            }
        })
        .to_owned()
}

fn feishu_send_part_key(send_key: &str, part: &str) -> String {
    let digest = Sha256::digest(format!("{send_key}\0{part}").as_bytes());
    let mut key = String::with_capacity(50);
    key.push_str("lk");
    for byte in digest.iter().take(24) {
        key.push_str(&format!("{byte:02x}"));
    }
    key
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

/// Select the public service or a private deployment, including its WS gateway.
pub(crate) fn configured_domain(
    config: &Value,
    workspace: &Value,
) -> Result<Domain, ProviderError> {
    if let Some(value) = config_text(workspace, "api_endpoint") {
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
        return Ok(Domain::Custom(value.trim_end_matches('/').to_owned()));
    }
    match config_text(config, "domain")
        .or_else(|| config_text(workspace, "domain"))
        .as_deref()
    {
        Some("lark") => Ok(Domain::Lark),
        Some("feishu") | None => Ok(Domain::Feishu),
        Some(value) if value.starts_with("http://") || value.starts_with("https://") => {
            Ok(Domain::Custom(value.to_owned()))
        }
        Some(_) => return Err(ProviderError::InvalidInput { field: "domain" }),
    }
}

fn inbound_media_metadata(
    downloaded: &crate::dto::DownloadedResource,
    name: &str,
    media_type: &str,
) -> (String, String) {
    let format = image::guess_format(&downloaded.data).ok();
    let media_type = format
        .map(|format| format.to_mime_type().to_owned())
        .unwrap_or_else(|| {
            if downloaded.content_type.trim().is_empty()
                || downloaded.content_type == "application/octet-stream"
            {
                media_type.to_owned()
            } else {
                downloaded
                    .content_type
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            }
        });
    let mut file_name = if downloaded.file_name.trim().is_empty() {
        name.to_owned()
    } else {
        downloaded.file_name.clone()
    };
    if file_name.trim().is_empty() {
        file_name = "attachment".into();
    }
    if std::path::Path::new(&file_name).extension().is_none() {
        let extension = format
            .and_then(|format| format.extensions_str().first().copied())
            .or_else(|| match media_type.as_str() {
                "image/png" => Some("png"),
                "image/jpeg" => Some("jpg"),
                "image/gif" => Some("gif"),
                "image/webp" => Some("webp"),
                _ => None,
            });
        if let Some(extension) = extension {
            file_name.push('.');
            file_name.push_str(extension);
        }
    }
    (file_name, media_type)
}
