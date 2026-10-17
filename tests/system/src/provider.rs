use lankide_core::{AgentId, AttentionLevel, ChannelAbility, ChannelId, ProviderId};
use lankide_provider::*;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundKind {
    Send,
    Reply,
    Reaction,
    Recall,
    Edit,
}

#[derive(Clone, Debug)]
pub struct Outbound {
    pub order: u64,
    pub workspace: String,
    pub account: String,
    pub chat: String,
    pub channel_type: String,
    pub kind: OutboundKind,
    pub content: String,
    pub message_id: String,
    pub reply_to: Option<String>,
    pub idempotency_key: Option<String>,
    pub resources: Vec<lankide_provider::MessageAttachment>,
}

#[derive(Clone, Debug)]
pub struct Incoming {
    pub order: u64,
    pub workspace: String,
    pub account: Option<String>,
    pub chat: String,
    pub person: String,
    pub content: String,
}

#[derive(Default)]
pub(crate) struct ImState {
    pub order: AtomicU64,
    pub preparation_gate: Mutex<Option<std::path::PathBuf>>,
    pub resource_contexts: Mutex<BTreeMap<String, InitContext>>,
    pub sinks: Mutex<BTreeMap<String, EventSink>>,
    pub accounts: Mutex<BTreeMap<(String, String), String>>,
    pub outbound: Mutex<Vec<Outbound>>,
    pub incoming: Mutex<Vec<Incoming>>,
}

pub(crate) struct ScriptedProvider {
    pub state: Arc<ImState>,
}

impl ScriptedProvider {
    fn account(&self, workspace: &str, agent: &AgentId) -> String {
        self.state
            .accounts
            .lock()
            .expect("accounts")
            .get(&(workspace.into(), agent.to_string()))
            .expect("registered bot account")
            .clone()
    }
}

impl Provider for ScriptedProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ProviderId::new("system_im").expect("provider id"),
            name: "System IM".into(),
            description: "Scripted IM boundary".into(),
            version: "1".into(),
            icon: String::new(),
        }
    }
    fn tools<'a>(&'a self, _: AgentRuntimeContext) -> ProviderFuture<'a, Vec<ToolDefinition>> {
        Box::pin(async {
            Ok(vec![ToolDefinition {
                id: "failing_reaction".into(),
                description: "IM reaction".into(),
                input_schema: JsonSchema::new(serde_json::json!({"type":"object"}))?,
                output_schema: JsonSchema::new(serde_json::json!({"type":"object"}))?,
                handler: Arc::new(FailedImCall),
            }])
        })
    }
    fn reaction_catalog(&self) -> Vec<ReactionDefinition> {
        vec![ReactionDefinition {
            id: "thumbs_up".into(),
            name: "Thumbs up".into(),
            emoji: "👍".into(),
        }]
    }
    fn channels<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async {
            Ok(vec![
                ChannelDefinition::new(
                    ChannelId::new("group_chat").expect("channel"),
                    "Group",
                    ChannelDefaultAttention::Mention,
                    [AttentionLevel::Mention, AttentionLevel::Follow],
                    [ChannelAbility::Reply, ChannelAbility::Reaction],
                )?,
                ChannelDefinition::new(
                    ChannelId::new("private_chat").expect("channel"),
                    "Direct",
                    ChannelDefaultAttention::DirectMessage,
                    [AttentionLevel::Follow],
                    [ChannelAbility::Reply, ChannelAbility::Reaction],
                )?,
            ])
        })
    }
    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state
                .resource_contexts
                .lock()
                .expect("resource contexts")
                .insert(context.workspace_id().to_string(), context.clone());
            self.state.sinks.lock().expect("sinks").insert(
                context.workspace_id().to_string(),
                context.event_sink().clone(),
            );
            Ok(())
        })
    }
    fn shutdown<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state
                .resource_contexts
                .lock()
                .expect("resource contexts")
                .remove(context.workspace_id().as_str());
            Ok(())
        })
    }
    fn create_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state.accounts.lock().expect("accounts").insert(
                (
                    context.workspace_id.to_string(),
                    context.agent_id.to_string(),
                ),
                context.new_provider_agent_config["account"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("bot-{}", context.agent_id)),
            );
            Ok(())
        })
    }
    fn system_prompt<'a>(&'a self, _: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        Box::pin(async move {
            let gate = self
                .state
                .preparation_gate
                .lock()
                .expect("preparation gate")
                .clone();
            if let Some(gate) = gate {
                while !gate.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
            Ok(String::new())
        })
    }
    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult> {
        Box::pin(async move {
            let account = self.account(context.workspace_id.as_str(), &context.agent_id);
            let reply_to = match context.reply_to {
                Some(message) => {
                    context
                        .storage()
                        .message(message)
                        .await?
                        .provider_message_id
                }
                None => None,
            };
            let mut sends = self.state.outbound.lock().expect("outbound");
            // Platform message IDs are unique across bots in the same group.
            let id = format!("sent-{}", sends.len() + 1);
            sends.push(Outbound {
                order: self.state.order.fetch_add(1, Ordering::SeqCst),
                workspace: context.workspace_id.to_string(),
                account,
                channel_type: context.channel_id.to_string(),
                chat: context.chat_id,
                kind: if context.reply_to.is_some() {
                    OutboundKind::Reply
                } else {
                    OutboundKind::Send
                },
                content: context.content,
                message_id: id.clone(),
                reply_to,
                resources: context.attachments,
                idempotency_key: Some(context.idempotency_key),
            });
            SendMessageResult::sent(id)
        })
    }
    fn auto_reaction<'a>(&'a self, context: AutoReactionContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let account = self.account(context.workspace_id.as_str(), &context.agent_id);
            for message in context.trigger_messages {
                self.state
                    .outbound
                    .lock()
                    .expect("outbound")
                    .push(Outbound {
                        order: self.state.order.fetch_add(1, Ordering::SeqCst),
                        workspace: context.workspace_id.to_string(),
                        account: account.clone(),
                        channel_type: context.channel_id.to_string(),
                        chat: context.chat_id.clone(),
                        kind: OutboundKind::Reaction,
                        content: context.auto_reaction.clone(),
                        message_id: message.to_string(),
                        reply_to: None,
                        resources: Vec::new(),
                        idempotency_key: None,
                    });
            }
            Ok(())
        })
    }
    fn recall_message<'a>(&'a self, context: RecallMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state
                .outbound
                .lock()
                .expect("outbound")
                .push(Outbound {
                    order: self.state.order.fetch_add(1, Ordering::SeqCst),
                    workspace: context.workspace_id.to_string(),
                    account: self.account(context.workspace_id.as_str(), &context.agent_id),
                    channel_type: context.channel_id.to_string(),
                    chat: context.chat_id,
                    kind: OutboundKind::Recall,
                    content: String::new(),
                    message_id: context.provider_message_id,
                    reply_to: None,
                    resources: Vec::new(),
                    idempotency_key: None,
                });
            Ok(())
        })
    }
    fn edit_message<'a>(&'a self, context: EditMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state
                .outbound
                .lock()
                .expect("outbound")
                .push(Outbound {
                    order: self.state.order.fetch_add(1, Ordering::SeqCst),
                    workspace: context.workspace_id.to_string(),
                    account: self.account(context.workspace_id.as_str(), &context.agent_id),
                    channel_type: context.channel_id.to_string(),
                    chat: context.chat_id,
                    kind: OutboundKind::Edit,
                    content: context.content,
                    message_id: context.provider_message_id,
                    reply_to: None,
                    resources: Vec::new(),
                    idempotency_key: None,
                });
            Ok(())
        })
    }
}

struct FailedImCall;
impl InvocationHandler for FailedImCall {
    fn invoke<'a>(&'a self, _: serde_json::Value) -> ProviderFuture<'a, serde_json::Value> {
        Box::pin(async {
            Err(ProviderError::External {
                operation: "reaction_add",
                retryable: false,
            })
        })
    }
}
