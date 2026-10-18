use super::*;

impl Runtime {
    pub async fn send_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: SendMessageSpec,
    ) -> Result<SendMessageResult, RuntimeError> {
        let result = async {
            let resources = spec
                .attachments
                .iter()
                .map(|attachment| {
                    let resource = &attachment.resource;
                    Ok::<_, RuntimeError>(lankide_storage::ResourceView {
                        resource_id: lankide_core::ResourceId::new(resource.resource_id)
                            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                        file_name: resource.file_name.clone(),
                        media_type: resource.media_type.clone(),
                        size_bytes: resource.size_bytes,
                        sha256: resource.sha256.clone(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.storage
                .record_outbound_resources(spec.message_id.get(), resources.clone())
                .await?;
            let result = self
                .host(workspace_id)
                .await?
                .send_message(agent_id.clone(), spec.clone())
                .await?;
            if let Some(chat) = &result.provider_chat_id {
                self.storage
                    .bind_sent_chat(spec.message_id.get(), chat)
                    .await?;
            }
            let now = now_millis();
            let outcome = if result.status == lankide_provider::SendMessageStatus::Sent {
                SendOutcome::Sent {
                    provider_message_id: result.provider_message_id.clone().ok_or_else(|| {
                        RuntimeError::InvalidCore("sent Provider result missing message id".into())
                    })?,
                }
            } else {
                SendOutcome::Unknown
            };
            self.storage
                .finish_message_send(
                    spec.message_id.get(),
                    outcome,
                    result.provider_message_ids.clone(),
                    resources,
                    now,
                )
                .await?;
            if let Some(native_id) = &result.provider_message_id {
                let mut mentioned_agent_ids = spec.mentions.clone();
                for mention in lankide_core::markdown_mentions(&spec.content) {
                    if let Ok(id) = AgentId::new(mention.id) {
                        if !mentioned_agent_ids.contains(&id) {
                            mentioned_agent_ids.push(id);
                        }
                    }
                }
                self.coordinate_event_inner(
                    workspace_id,
                    ProviderEvent {
                        receiving_agent_id: Some(agent_id.clone()),
                        channel_id: spec.channel_id.clone(),
                        provider_chat_id: result
                            .provider_chat_id
                            .clone()
                            .unwrap_or_else(|| spec.chat_id.clone()),
                        sender: lankide_provider::EventSender {
                            id: agent_id.to_string(),
                            kind: EventSenderType::Agent,
                            name: String::new(),
                        },
                        kind: ProviderEventKind::Message(lankide_provider::MessageEvent {
                            provider_message_id: native_id.clone(),
                            content: spec.content.clone(),
                            reply_to_provider_message_id: None,
                            mentioned_agent_ids,
                            attachments: spec
                                .attachments
                                .iter()
                                .map(|item| item.resource.clone())
                                .collect(),
                        }),
                        provider_data: serde_json::json!({}),
                    },
                    now,
                )
                .await?;
            }
            Ok::<_, RuntimeError>(result)
        }
        .await;
        if let Err(error) = &result {
            log::error!(
                "workspace={workspace_id} agent={agent_id} message={} send failed: {error:?}",
                spec.message_id
            );
        }
        result
    }

    pub async fn invoke_provider_tool(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        runtime_spec: AgentRuntimeSpec,
        request: ToolInvocationRequest,
    ) -> Result<ToolProgress, RuntimeError> {
        let begun = self.storage.begin_tool_invocation(request.clone()).await?;
        if !begun.created {
            return Ok(match (begun.state.as_str(), begun.result) {
                ("completed", Some(result)) => ToolProgress::Completed { result },
                ("failed", result) => ToolProgress::Failed {
                    code: result
                        .as_ref()
                        .and_then(|value| value.get("error"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("provider_tool_failed")
                        .to_owned(),
                },
                _ => ToolProgress::InProgress,
            });
        }
        let guard = ToolInvocationGuard::new(self.storage.clone(), request.clone());
        match self
            .host(workspace_id)
            .await?
            .invoke_tool(
                agent_id,
                runtime_spec,
                request.tool_name.clone(),
                request.arguments.clone(),
            )
            .await
        {
            Ok(result) => {
                guard.complete(result.clone(), request.now).await?;
                Ok(ToolProgress::Completed { result })
            }
            Err(error) => {
                guard.fail("provider_tool_failed", now_millis()).await?;
                log::error!("provider Tool {} failed: {error}", request.tool_name);
                Ok(ToolProgress::Failed {
                    code: "provider_tool_failed".into(),
                })
            }
        }
    }

    pub async fn invoke_provider_cli(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        runtime_spec: AgentRuntimeSpec,
        command: String,
        arguments: Vec<String>,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.host(workspace_id)
            .await?
            .invoke_cli(agent_id, runtime_spec, command, arguments)
            .await
            .map_err(Into::into)
    }

    pub async fn invoke_provider_interface(
        &self,
        workspace_id: &WorkspaceId,
        interface_id: String,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.require_active(workspace_id)?;
        let result = self
            .host(workspace_id)
            .await?
            .invoke_interface(interface_id, input)
            .await;
        if let Err(error) = &result {
            // Never log interface input or upstream error text: either can contain secrets.
            log::warn!(
                "provider interface failed workspace={workspace_id} category={}",
                error.code()
            );
        }
        result.map_err(Into::into)
    }

    pub async fn recall_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: RecallMessageSpec,
    ) -> Result<(), RuntimeError> {
        self.host(workspace_id)
            .await?
            .recall_message(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn edit_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: EditMessageSpec,
    ) -> Result<(), RuntimeError> {
        self.host(workspace_id)
            .await?
            .edit_message(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn list_provider_interfaces(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<InterfaceCatalogEntry>, RuntimeError> {
        self.require_active(workspace_id)?;
        self.host(workspace_id)
            .await?
            .list_interfaces()
            .await
            .map_err(Into::into)
    }

    pub async fn validate_run_context(
        &self,
        context: &lankide_protocol::AgentRunContext,
        now: i64,
    ) -> Result<ValidatedRunContext, RuntimeError> {
        let execution = context
            .execution
            .as_ref()
            .ok_or_else(|| RuntimeError::Application("execution ref required".into()))?;
        let auth = self
            .storage
            .execution_auth_context(execution.execution_id, i64::from(execution.attempt))
            .await?
            .ok_or(RuntimeError::InvalidExecutionToken)?;
        if auth.expires_at < now
            || !constant_time_eq(&auth.token_hash, &token_hash(&context.execution_token))
            || matches!(
                auth.state,
                lankide_storage::ExecutionState::Succeeded
                    | lankide_storage::ExecutionState::Failed
                    | lankide_storage::ExecutionState::Cancelled
            )
        {
            return Err(RuntimeError::InvalidExecutionToken);
        }
        let snapshot_id = self
            .storage
            .execution(execution.execution_id)
            .await?
            .snapshot_id;
        let snapshot = self.storage.snapshot(snapshot_id).await?;
        let authorization = self
            .storage
            .attempt_authorization(execution.execution_id, i64::from(execution.attempt))
            .await?
            .ok_or(RuntimeError::InvalidExecutionToken)?;
        if !constant_time_eq(
            &authorization.context_fingerprint,
            &lowercase_sha256(authorization.context_json.as_bytes()),
        ) {
            return Err(RuntimeError::InvalidExecutionToken);
        }
        let runtime_spec = frozen_runtime_spec(&authorization.context_json)?;
        let agent = self
            .provider(&auth.workspace_id)?
            .agents
            .read()
            .await
            .get(&auth.agent_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownAgent {
                workspace_id: auth.workspace_id.clone(),
                agent_id: auth.agent_id.clone(),
            })?;
        Ok(ValidatedRunContext {
            workspace_id: auth.workspace_id,
            agent_id: auth.agent_id,
            execution_id: execution.execution_id,
            attempt: execution.attempt,
            snapshot,
            agent,
            runtime_spec,
            capabilities: authorization.capabilities,
        })
    }

    pub async fn auto_reaction(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: AutoReactionSpec,
    ) -> Result<(), RuntimeError> {
        self.require_active(workspace_id)?;
        self.host(workspace_id)
            .await?
            .auto_reaction(agent_id, spec)
            .await
            .map_err(Into::into)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedRunContext {
    pub workspace_id: WorkspaceId,
    pub agent_id: AgentId,
    pub execution_id: i64,
    pub attempt: u32,
    pub snapshot: SnapshotView,
    pub agent: AgentRuntimeConfig,
    pub runtime_spec: AgentRuntimeSpec,
    pub capabilities: Vec<AttemptCapability>,
}
fn frozen_runtime_spec(value: &str) -> Result<AgentRuntimeSpec, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| RuntimeError::Application(format!("invalid frozen context: {error}")))?;
    let text = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| RuntimeError::Application(format!("frozen context missing {name}")))
    };
    let chats = value
        .get("chats")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| RuntimeError::Application("frozen context missing chats".into()))?
        .iter()
        .map(|chat| {
            let field = |name: &str| {
                chat.get(name)
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| RuntimeError::Application(format!("frozen chat missing {name}")))
            };
            Ok::<lankide_provider::AgentRuntimeChat, RuntimeError>(
                lankide_provider::AgentRuntimeChat {
                    provider_id: ProviderId::new(field("provider_id")?)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                    channel_id: ChannelId::new(field("channel_id")?)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                    channel_name: field("channel_name")?.to_owned(),
                    chat_id: field("chat_id")?.to_owned(),
                    chat_name: field("chat_name")?.to_owned(),
                },
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AgentRuntimeSpec {
        agent_config: value.get("agent_config").cloned().unwrap_or_default(),
        provider_agent_config: value
            .get("provider_agent_config")
            .cloned()
            .unwrap_or_default(),
        machine_id: text("machine_id")?,
        harness_id: text("harness_id")?,
        model: text("model")?,
        thinking_level: text("thinking_level")?,
        trigger_reason: text("trigger_reason")?,
        chats,
        unread_snapshot: value.get("unread_snapshot").cloned().unwrap_or_default(),
    })
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
