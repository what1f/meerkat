use super::*;

impl Runtime {
    /// Waits for one normalized event from a running Workspace instance.
    /// Paused Workspaces never touch the Provider receiver.
    pub async fn next_provider_event(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<EventDelivery>, RuntimeError> {
        self.require_active(workspace_id)?;
        let host = self.host(workspace_id).await?;
        let event = host.next_event().await;
        if !self.is_workspace_active(workspace_id)? {
            return Err(RuntimeError::WorkspacePaused(workspace_id.clone()));
        }
        Ok(event)
    }

    /// Accept a Provider delivery only after its canonical event has been committed.
    pub async fn accept_provider_event(
        &self,
        workspace_id: &WorkspaceId,
        delivery: EventDelivery,
        now: i64,
    ) -> Result<CoordinatedEvent, RuntimeError> {
        let (event, acknowledgement) = delivery.into_parts();
        let result = self.coordinate_event(workspace_id, event, now).await;
        if result.is_ok() {
            acknowledgement.accept();
        } else {
            acknowledgement.reject();
        }
        result
    }

    /// Persists one normalized Provider event and computes its Agent dispatches.
    /// The returned snapshot boundary is still created later by `create_execution`.
    pub async fn coordinate_event(
        &self,
        workspace_id: &WorkspaceId,
        event: ProviderEvent,
        now: i64,
    ) -> Result<CoordinatedEvent, RuntimeError> {
        self.require_active(workspace_id)?;
        let provider = self.provider(workspace_id)?;
        // Agent messages are dispatched by send_message. Platform callbacks can
        // arrive before any native part ID is known, so identify echoes by sender.
        if matches!(event.kind, ProviderEventKind::Message(_))
            && provider.agents.read().await.values().any(|agent| {
                event.sender.id == agent.agent_id.as_str()
                    || event.sender.id == agent.provider_sender_id
            })
        {
            return Ok(CoordinatedEvent::Dropped);
        }
        self.coordinate_event_inner(workspace_id, event, now).await
    }

    pub(super) async fn coordinate_event_inner(
        &self,
        workspace_id: &WorkspaceId,
        mut event: ProviderEvent,
        now: i64,
    ) -> Result<CoordinatedEvent, RuntimeError> {
        self.require_active(workspace_id)?;
        let provider = self.provider(workspace_id)?;
        let channels = provider.channels.read().await;
        let channel = channels
            .iter()
            .find(|channel| channel.id == event.channel_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownChannel {
                workspace_id: workspace_id.clone(),
                channel_id: event.channel_id.clone(),
            })?;
        drop(channels);
        let mut agents = provider.agents.read().await.clone();
        // Providers only know an Agent's account; the Runtime owns its name.
        let name_agent = |sender: &mut lankide_provider::EventSender| {
            if sender.kind != EventSenderType::Agent {
                return;
            }
            if let Some(agent) = agents.values().find(|agent| {
                sender.id == agent.agent_id.as_str() || sender.id == agent.provider_sender_id
            }) {
                sender.name = agent.agent_profile.name.clone();
            }
        };
        name_agent(&mut event.sender);
        if let ProviderEventKind::Reaction(reaction) = &mut event.kind {
            name_agent(&mut reaction.sender);
        }
        let receiving_agent_id = if channel.default_attention
            == ChannelDefaultAttention::DirectMessage
        {
            let Some(receiver) = event
                .receiving_agent_id
                .as_ref()
                .filter(|id| agents.contains_key(*id))
            else {
                log::error!(
                    "[runtime] warning: workspace={workspace_id} direct message dropped: missing or unknown receiving Agent"
                );
                return Ok(CoordinatedEvent::Dropped);
            };
            agents.retain(|id, _| id == receiver);
            Some(receiver.clone())
        } else {
            None
        };
        let provider_id = self.storage.workspace(workspace_id).await?.provider_id;
        let occurred_at = event.occurred_at_millis().unwrap_or(now);
        let chat_name = event
            .provider_data
            .get("chat_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&event.provider_chat_id)
            .to_owned();
        let provider_event_id = event.event_id().map(str::to_owned);

        match event.kind {
            ProviderEventKind::Message(message) => {
                let mut chat_attention = BTreeMap::new();
                for agent_id in agents.keys() {
                    if let Some(attention) = self
                        .storage
                        .chat_attention(
                            workspace_id,
                            agent_id,
                            &event.channel_id,
                            &event.provider_chat_id,
                        )
                        .await?
                    {
                        chat_attention.insert(agent_id.clone(), attention_policy(&attention)?);
                    }
                }
                let team_command = self
                    .apply_team_command(workspace_id, &event.sender, &message.content, now)
                    .await?;
                let dispatch = if team_command {
                    Vec::new()
                } else {
                    message_dispatches(
                        &channel,
                        &agents,
                        &chat_attention,
                        &event.sender,
                        &message,
                        now,
                    )?
                };
                let resources = message
                    .attachments
                    .iter()
                    .map(resource_view)
                    .collect::<Result<Vec<_>, _>>()?;
                let result = self
                    .storage
                    .ingest_event(IngestEvent {
                        receiving_agent_id: receiving_agent_id.clone(),
                        workspace_id: workspace_id.clone(),
                        provider_id,
                        channel_id: event.channel_id.clone(),
                        channel_name: channel.name.clone(),
                        provider_chat_id: event.provider_chat_id.clone(),
                        chat_name,
                        provider_event_id,
                        provider_message_id: message.provider_message_id,
                        message: MessageInput {
                            sender_kind: sender_kind(event.sender.kind),
                            sender_id: event.sender.id,
                            sender_name: event.sender.name,
                            content: message.content,
                            mentions: message.mentioned_agent_ids,
                            reply_to_provider_message_id: message.reply_to_provider_message_id,
                            resources,
                            occurred_at,
                        },
                        dispatch: dispatch.clone(),
                        now,
                    })
                    .await?;
                if receiving_agent_id.is_none()
                    && let Some(receiver) = event
                        .receiving_agent_id
                        .as_ref()
                        .filter(|id| agents.contains_key(*id))
                {
                    self.storage
                        .record_chat_receiver(
                            workspace_id,
                            receiver,
                            &event.channel_id,
                            &event.provider_chat_id,
                        )
                        .await?;
                }
                // Keep new messages pending for the next execution. Injecting only
                // their text into a running execution would not expand that run's
                // readable messages, reply scope, or cursor boundary.
                let steer_deliveries = Vec::new();
                let auto_reaction_errors = Vec::new();
                if !result.new_inbox_items.is_empty()
                    && channel.abilities.contains(&ChannelAbility::Reaction)
                {
                    let message_id = MessageId::new(result.message_id)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                    let host = self.host(workspace_id).await?;
                    for dispatch in dispatch.into_iter().filter(|dispatch| {
                        result
                            .new_inbox_items
                            .iter()
                            .any(|(agent_id, _)| agent_id == &dispatch.agent_id)
                    }) {
                        let Some(agent) = agents.get(&dispatch.agent_id) else {
                            continue;
                        };
                        let Some(auto_reaction) = agent.auto_reaction.clone() else {
                            continue;
                        };
                        let host = host.clone();
                        let workspace_id = workspace_id.clone();
                        let agent_id = agent.agent_id.clone();
                        let spec = AutoReactionSpec {
                            auto_reaction,
                            channel_id: event.channel_id.clone(),
                            chat_id: event.provider_chat_id.clone(),
                            trigger_messages: vec![message_id],
                            provider_agent_config: agent.provider_agent_config.clone(),
                        };
                        let mut tasks = self
                            .background_tasks
                            .lock()
                            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
                        while tasks.try_join_next().is_some() {}
                        tasks.spawn(async move {
                            if let Err(error) = host.auto_reaction(agent_id.clone(), spec).await {
                                log::error!(
                                    "[runtime] workspace={workspace_id} agent={agent_id} auto reaction failed: {error:?}"
                                );
                            }
                        });
                    }
                }
                Ok(CoordinatedEvent::Message {
                    result,
                    auto_reaction_errors,
                    steer_deliveries,
                })
            }
            ProviderEventKind::Reaction(reaction) => {
                let own_auto_reaction = agents.values().any(|agent| {
                    agent.provider_sender_id == reaction.sender.id
                        && agent.auto_reaction.as_deref() == Some(reaction.reaction.as_str())
                });
                let dispatch = if own_auto_reaction
                    || reaction.action == ReactionAction::Remove
                    || !channel.abilities.contains(&ChannelAbility::Reaction)
                {
                    Vec::new()
                } else {
                    reaction_dispatches(
                        &agents,
                        &reaction.sender,
                        &reaction.reaction,
                        if reaction.sender.kind == EventSenderType::Agent {
                            self.storage
                                .reaction_target_author(
                                    workspace_id,
                                    &event.channel_id,
                                    &event.provider_chat_id,
                                    &reaction.target_provider_message_id,
                                    receiving_agent_id.as_ref(),
                                )
                                .await?
                        } else {
                            None
                        },
                        now,
                    )
                };
                let event_id = provider_event_id.unwrap_or_else(|| {
                    format!(
                        "reaction:{}:{}:{}:{:?}",
                        reaction.target_provider_message_id,
                        reaction.sender.id,
                        reaction.reaction,
                        reaction.action
                    )
                });
                let result = self
                    .storage
                    .record_reaction(IncomingReaction {
                        receiving_agent_id,
                        workspace_id: workspace_id.clone(),
                        provider_id,
                        channel_id: event.channel_id,
                        provider_chat_id: event.provider_chat_id,
                        target_provider_message_id: reaction.target_provider_message_id,
                        sender_kind: sender_kind(reaction.sender.kind),
                        sender_id: reaction.sender.id,
                        sender_name: reaction.sender.name,
                        reaction: reaction.reaction,
                        action: reaction.action,
                        origin: if own_auto_reaction {
                            ReactionOrigin::AutoReaction
                        } else {
                            ReactionOrigin::Provider
                        },
                        provider_event_id: event_id,
                        dispatch,
                        occurred_at,
                        now,
                    })
                    .await?;
                Ok(CoordinatedEvent::Reaction(result))
            }
            ProviderEventKind::Custom(custom) => {
                let dispatch = custom
                    .target_agent_ids
                    .iter()
                    .filter(|id| {
                        agents.get(*id).is_some_and(|agent| !agent.muted())
                            && !(event.sender.kind == EventSenderType::Agent
                                && (event.sender.id == id.as_str()
                                    || agents.get(*id).is_some_and(|agent| {
                                        event.sender.id == agent.provider_sender_id
                                    })))
                    })
                    .map(|id| InboxDispatch {
                        agent_id: id.clone(),
                        available_at: now,
                        trigger_reason: if custom.mentioned_agent_ids.contains(id) {
                            DispatchReason::Mention
                        } else {
                            DispatchReason::Custom {
                                name: custom.reason.clone(),
                                object_id: custom.subject_id.clone(),
                            }
                        },
                    })
                    .collect();
                let synthetic_message_id = provider_event_id
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| format!("custom-event:{value}"))
                    .unwrap_or_else(|| format!("custom-object:{}", custom.subject_id));
                let result = self
                    .storage
                    .ingest_event(IngestEvent {
                        receiving_agent_id: receiving_agent_id.clone(),
                        workspace_id: workspace_id.clone(),
                        provider_id,
                        channel_id: event.channel_id,
                        channel_name: channel.name.clone(),
                        provider_chat_id: event.provider_chat_id,
                        chat_name,
                        provider_event_id,
                        provider_message_id: synthetic_message_id,
                        message: MessageInput {
                            sender_kind: sender_kind(event.sender.kind),
                            sender_id: event.sender.id,
                            sender_name: event.sender.name,
                            content: custom.subject_name,
                            mentions: custom.target_agent_ids,
                            reply_to_provider_message_id: None,
                            resources: Vec::new(),
                            occurred_at,
                        },
                        dispatch,
                        now,
                    })
                    .await?;
                Ok(CoordinatedEvent::Custom(result))
            }
        }
    }

    async fn apply_team_command(
        &self,
        workspace_id: &WorkspaceId,
        sender: &lankide_provider::EventSender,
        content: &str,
        now: i64,
    ) -> Result<bool, RuntimeError> {
        if sender.kind != EventSenderType::Human {
            return Ok(false);
        }
        let Some(command) = content.strip_prefix("/team") else {
            return Ok(false);
        };
        let mut parts = command.trim_start().splitn(2, char::is_whitespace);
        let role = match parts.next() {
            Some("member") => TeamRole::Member,
            Some("lead") => TeamRole::Lead,
            _ => return Ok(false),
        };
        let Some(target) = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(false);
        };
        let Some(target) = lankide_core::markdown_mentions(target)
            .into_iter()
            .find(|mention| mention.range == (0..target.len()))
            .and_then(|mention| PersonId::new(mention.id.to_owned()).ok())
        else {
            return Ok(false);
        };
        let Ok(actor) = PersonId::new(sender.id.clone()) else {
            return Ok(false);
        };
        match self
            .storage
            .update_team_member(workspace_id, &actor, &target, role, now)
            .await
        {
            Ok(_) => Ok(true),
            Err(StorageError::Conflict(_)) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn set_chat_attention(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        provider_chat_id: &str,
        attention: lankide_storage::AttentionRecord,
        now: i64,
    ) -> Result<(), RuntimeError> {
        self.require_active(workspace_id)?;
        let provider = self.provider(workspace_id)?;
        let channels = provider.channels.read().await;
        let channel = channels
            .iter()
            .find(|channel| &channel.id == channel_id)
            .ok_or_else(|| RuntimeError::UnknownChannel {
                workspace_id: workspace_id.clone(),
                channel_id: channel_id.clone(),
            })?;
        let supported = match &attention {
            lankide_storage::AttentionRecord::Mention => {
                channel.attention_levels.contains(&AttentionLevel::Mention)
            }
            lankide_storage::AttentionRecord::Always => {
                channel.attention_levels.contains(&AttentionLevel::Follow)
            }
            lankide_storage::AttentionRecord::Schedule { .. } => {
                channel.attention_levels.contains(&AttentionLevel::Schedule)
            }
            lankide_storage::AttentionRecord::Mute => !channel.attention_levels.is_empty(),
        };
        if !supported {
            return Err(RuntimeError::Application(
                "This conversation does not support that attention level.".into(),
            ));
        }
        self.storage
            .set_chat_attention(
                workspace_id,
                agent_id,
                channel_id,
                provider_chat_id,
                &attention,
                now,
            )
            .await?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttentionPolicy {
    Mention,
    Schedule(ScheduleInterval),
    Always,
    Mute,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoordinatedEvent {
    Dropped,
    Message {
        result: IngestResult,
        auto_reaction_errors: Vec<String>,
        steer_deliveries: Vec<lankide_storage::SteerDelivery>,
    },
    Reaction(ReactionResult),
    Custom(IngestResult),
}

fn sender_kind(value: EventSenderType) -> MessageAuthorType {
    match value {
        EventSenderType::Human => MessageAuthorType::Human,
        EventSenderType::Agent => MessageAuthorType::Agent,
        EventSenderType::Bot => MessageAuthorType::Bot,
        EventSenderType::System => MessageAuthorType::System,
    }
}

fn resource_view(value: &lankide_core::ResourceRef) -> Result<ResourceView, RuntimeError> {
    Ok(ResourceView {
        resource_id: ResourceId::new(value.resource_id)
            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
        file_name: value.file_name.clone(),
        media_type: value.media_type.clone(),
        size_bytes: value.size_bytes,
        sha256: value.sha256.clone(),
    })
}

fn message_dispatches(
    channel: &ChannelDefinition,
    agents: &BTreeMap<AgentId, AgentRuntimeConfig>,
    chat_attention: &BTreeMap<AgentId, AttentionPolicy>,
    sender: &lankide_provider::EventSender,
    message: &lankide_provider::MessageEvent,
    now: i64,
) -> Result<Vec<InboxDispatch>, RuntimeError> {
    let mut dispatches = Vec::new();
    for agent in agents.values() {
        if sender.kind == EventSenderType::Agent
            && (sender.id == agent.provider_sender_id || sender.id == agent.agent_id.as_str())
        {
            continue;
        }
        let default_policy = match channel.default_attention {
            ChannelDefaultAttention::DirectMessage | ChannelDefaultAttention::Follow => {
                AttentionPolicy::Always
            }
            ChannelDefaultAttention::Mention => AttentionPolicy::Mention,
            ChannelDefaultAttention::Mute => AttentionPolicy::Mute,
        };
        let policy = chat_attention
            .get(&agent.agent_id)
            .or_else(|| agent.attention.get(&channel.id))
            .unwrap_or(&default_policy);
        if policy == &AttentionPolicy::Mute {
            continue;
        }
        let mentioned = message.mentioned_agent_ids.contains(&agent.agent_id)
            && (channel.attention_levels.contains(&AttentionLevel::Mention)
                || channel.default_attention == ChannelDefaultAttention::Mention);
        let (available_at, reason) =
            if channel.default_attention == ChannelDefaultAttention::DirectMessage {
                if policy != &AttentionPolicy::Always {
                    continue;
                }
                (now, DispatchReason::DirectMessage)
            } else if mentioned {
                (now, DispatchReason::Mention)
            } else if matches!(sender.kind, EventSenderType::Agent | EventSenderType::Bot) {
                continue;
            } else {
                match policy {
                    AttentionPolicy::Mute => unreachable!("mute policy handled above"),
                    AttentionPolicy::Mention => continue,
                    AttentionPolicy::Always
                        if channel.attention_levels.contains(&AttentionLevel::Follow)
                            || channel.default_attention == ChannelDefaultAttention::Follow =>
                    {
                        (now, DispatchReason::Follow)
                    }
                    AttentionPolicy::Always => continue,
                    AttentionPolicy::Schedule(interval)
                        if channel.attention_levels.contains(&AttentionLevel::Schedule) =>
                    {
                        let boundary =
                            interval
                                .next_boundary(UnixMillis::new(now).map_err(|error| {
                                    RuntimeError::InvalidCore(error.to_string())
                                })?)
                                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                        (boundary.get(), DispatchReason::Schedule)
                    }
                    AttentionPolicy::Schedule(_) => continue,
                }
            };
        dispatches.push(InboxDispatch {
            agent_id: agent.agent_id.clone(),
            available_at,
            trigger_reason: reason,
        });
    }
    Ok(dispatches)
}

fn attention_policy(
    record: &lankide_storage::AttentionRecord,
) -> Result<AttentionPolicy, RuntimeError> {
    Ok(match record {
        lankide_storage::AttentionRecord::Mention => AttentionPolicy::Mention,
        lankide_storage::AttentionRecord::Always => AttentionPolicy::Always,
        lankide_storage::AttentionRecord::Mute => AttentionPolicy::Mute,
        lankide_storage::AttentionRecord::Schedule { interval_minutes } => {
            AttentionPolicy::Schedule(
                ScheduleInterval::try_from(*interval_minutes)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
            )
        }
    })
}

fn reaction_dispatches(
    agents: &BTreeMap<AgentId, AgentRuntimeConfig>,
    sender: &lankide_provider::EventSender,
    reaction: &str,
    target_author: Option<(MessageAuthorType, String)>,
    now: i64,
) -> Vec<InboxDispatch> {
    agents
        .values()
        .filter(|agent| {
            !agent.muted()
                && (sender.kind != EventSenderType::Agent
                    || (sender.id != agent.provider_sender_id
                        && target_author.as_ref().is_some_and(|(kind, id)| {
                            *kind == MessageAuthorType::Agent
                                && (id == agent.agent_id.as_str()
                                    || id == &agent.provider_sender_id)
                        })))
        })
        .map(|agent| InboxDispatch {
            agent_id: agent.agent_id.clone(),
            available_at: now,
            trigger_reason: DispatchReason::Reaction {
                reaction: reaction.to_owned(),
                sender_name: sender.name.clone(),
            },
        })
        .collect()
}
