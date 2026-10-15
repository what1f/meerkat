use super::*;
use lankide_protocol::{
    AgentProfile, AgentRef, AgentRunContext, Capability, CapabilityInvokeRequest,
    CapabilityInvokeResult, CapabilityList, ContentKind, ContentRequest, ContentResult,
    SendMessageRequest, SendMessageResult as ProtocolSendMessageResult, ToolDefinition, ToolError,
    UpdateAgentRequest, UpdateSelfProfileRequest,
};
use lankide_provider::{MessageAttachment, ResourceUpload, SendMessageStatus};

pub(crate) fn standard_tool_definitions() -> Vec<ToolDefinition> {
    [
        (
            "message_send",
            "Send a message to a conversation in the current task.",
            r#"{"type":"object","required":["channel_id","chat_id"],"anyOf":[{"required":["content"]},{"required":["content_file"]},{"required":["attachments"]}],"properties":{"channel_id":{"type":"string"},"chat_id":{"type":"string"},"content":{"type":"string"},"content_file":{"type":"string"},"attachments":{"type":"array","minItems":1,"items":{"type":"object"}},"reply_to":{"type":"integer"},"force":{"type":"boolean","description":"强制发送。使用前必须读取当前可获取的最新消息，重新判断待发送内容。仅当内容仍然有效、此刻有必要发出，且继续等待会妨碍沟通时使用。禁止因发送被拒绝、图省事或机械重试而启用；尚未读取新消息时不得使用。"}},"additionalProperties":false}"#,
        ),
        (
            "message_recall",
            "Recall a previously sent message when supported.",
            r#"{"type":"object","required":["message_id"],"properties":{"message_id":{"type":"integer"}},"additionalProperties":false}"#,
        ),
        (
            "message_edit",
            "Edit a previously sent message when supported.",
            r#"{"type":"object","required":["message_id","content"],"properties":{"message_id":{"type":"integer"},"content":{"type":"string","minLength":1}},"additionalProperties":false}"#,
        ),
        (
            "chat_messages",
            "Read messages from a conversation at the current task boundary.",
            r#"{"type":"object","required":["channel_id","chat_id"],"properties":{"channel_id":{"type":"string"},"chat_id":{"type":"string"},"before_message_id":{"type":"integer"},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}"#,
        ),
        (
            "unread_current",
            "Read the conversations and messages frozen for the current task.",
            r#"{"type":"object","additionalProperties":false}"#,
        ),
        (
            "attention_set",
            "Choose how closely to follow one group conversation.",
            r#"{"type":"object","required":["channel_id","chat_id","level"],"properties":{"channel_id":{"type":"string"},"chat_id":{"type":"string"},"level":{"type":"string","enum":["follow","schedule","mention","mute"]},"interval_minutes":{"type":"integer","enum":[5,15,30,60,120,240,480,1440]}} ,"additionalProperties":false}"#,
        ),
    ]
    .into_iter()
    .map(|(name, description, input_schema_json)| ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema_json: input_schema_json.into(),
        output_schema_json: r#"{"type":"object"}"#.into(),
    })
    .collect()
}

impl RuntimeApplication {
    pub async fn get_self_profile(
        &self,
        context: AgentRunContext,
    ) -> Result<AgentProfile, RuntimeError> {
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(&context, now_millis()).await?;
        self.self_profile(&validated.workspace_id, &validated.agent_id)
    }

    pub async fn update_self_profile(
        &self,
        request: UpdateSelfProfileRequest,
    ) -> Result<AgentProfile, RuntimeError> {
        let context = request
            .context
            .as_ref()
            .ok_or_else(|| RuntimeError::Application("run context required".into()))?;
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(context, now_millis()).await?;
        let workspace_id = validated.workspace_id;
        let agent_id = validated.agent_id;
        self.update_agent(UpdateAgentRequest {
            request_id: String::new(),
            agent: Some(AgentRef {
                workspace_id: workspace_id.to_string(),
                agent_id: agent_id.to_string(),
            }),
            name: request.name,
            role: request.role,
            bio: request.bio,
            avatar: request.avatar,
            ..Default::default()
        })
        .await?;
        self.self_profile(&workspace_id, &agent_id)
    }

    fn self_profile(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<AgentProfile, RuntimeError> {
        let runtime = self.runtime()?;
        let record = runtime.storage().agent_record(workspace_id, agent_id)?;
        let avatar = self
            .agent_profiles()
            .read_avatar(workspace_id, agent_id, record.avatar.as_deref())
            .ok()
            .flatten()
            .map(avatar_file);
        Ok(AgentProfile {
            agent: Some(AgentRef {
                workspace_id: workspace_id.to_string(),
                agent_id: agent_id.to_string(),
            }),
            name: record.name,
            role: record.role,
            bio: record.bio,
            avatar,
        })
    }

    pub async fn save_resource(
        &self,
        context: &AgentRunContext,
        upload: ResourceUpload,
    ) -> Result<ResourceRef, RuntimeError> {
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(context, now_millis()).await?;
        self.resource_store()?
            .save(&validated.workspace_id, upload)
            .await
    }

    pub async fn read_resource(
        &self,
        context: &AgentRunContext,
        resource_id: ResourceId,
    ) -> Result<ResourceBytes, RuntimeError> {
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(context, now_millis()).await?;
        if runtime
            .storage()
            .resource_metadata(&validated.workspace_id, resource_id)?
            .is_none()
        {
            return Err(RuntimeError::InvalidExecutionToken);
        }
        self.resource_store()?
            .read(&validated.workspace_id, resource_id)
            .await
    }

    pub async fn list_capabilities(
        &self,
        context: AgentRunContext,
    ) -> Result<CapabilityList, RuntimeError> {
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(&context, now_millis()).await?;
        let capabilities = validated
            .capabilities
            .into_iter()
            .map(|capability| Capability {
                name: capability.name,
                display_name: capability.display_name,
                description: capability.description,
                kind: capability.kind,
                input_schema_json: capability.input_schema_json,
                output_schema_json: capability.output_schema_json,
            })
            .collect();
        Ok(CapabilityList { capabilities })
    }

    pub async fn invoke_capability(
        &self,
        request: CapabilityInvokeRequest,
    ) -> Result<CapabilityInvokeResult, RuntimeError> {
        let context = request
            .context
            .as_ref()
            .ok_or_else(|| RuntimeError::Application("run context required".into()))?;
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(context, now_millis()).await?;
        let authorized = validated
            .capabilities
            .iter()
            .find(|capability| capability.name == request.capability_name);
        let Some(authorized) = authorized else {
            return Ok(capability_error(
                request.invocation_id,
                "not_authorized",
                false,
            ));
        };
        let arguments = parse_arguments(&request.arguments_json)?;
        if let Some(result) = invoke_standard_tool(self, &request, &validated, &arguments).await? {
            return Ok(result);
        }
        let result = if authorized.kind == lankide_protocol::CapabilityKind::Tool as i32 {
            let name = request.capability_name.as_str();
            match runtime
                .invoke_provider_tool(
                    &validated.workspace_id,
                    validated.agent_id.clone(),
                    validated.runtime_spec.clone(),
                    ToolInvocationRequest {
                        execution_id: validated.execution_id,
                        attempt: i64::from(validated.attempt),
                        tool_call_id: request.invocation_id.clone(),
                        tool_name: name.to_owned(),
                        arguments,
                        now: now_millis(),
                    },
                )
                .await?
            {
                ToolProgress::Completed { result } => result,
                ToolProgress::Failed { code } => {
                    return Ok(capability_error(request.invocation_id, &code, false));
                }
                ToolProgress::InProgress => {
                    return Ok(CapabilityInvokeResult {
                        invocation_id: request.invocation_id,
                        success: false,
                        content_markdown: String::new(),
                        structured_content_json: "{}".into(),
                        resources: Vec::new(),
                        error: Some(ToolError {
                            code: "in_progress".into(),
                            message: "Invocation is still running.".into(),
                            retryable: true,
                        }),
                    });
                }
            }
        } else if let Some(name) = request.capability_name.strip_prefix("cli: ") {
            let values = arguments
                .as_array()
                .ok_or_else(|| RuntimeError::Application("CLI arguments must be an array".into()))?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        RuntimeError::Application("CLI argument must be text".into())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            runtime
                .invoke_provider_cli(
                    &validated.workspace_id,
                    validated.agent_id.clone(),
                    validated.runtime_spec.clone(),
                    name.to_owned(),
                    values,
                )
                .await?
        } else {
            return Err(RuntimeError::Application("unknown capability".into()));
        };
        Ok(CapabilityInvokeResult {
            invocation_id: request.invocation_id,
            success: true,
            content_markdown: result.as_str().unwrap_or_default().to_owned(),
            structured_content_json: result.to_string(),
            resources: Vec::new(),
            error: None,
        })
    }

    pub async fn send_message_gateway(
        &self,
        request: SendMessageRequest,
    ) -> Result<ProtocolSendMessageResult, RuntimeError> {
        let context = request
            .context
            .as_ref()
            .ok_or_else(|| RuntimeError::Application("run context required".into()))?;
        if request.idempotency_key.trim().is_empty() {
            return Err(RuntimeError::Application("idempotency key required".into()));
        }
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(context, now_millis()).await?;
        let channel_id = ChannelId::new(request.channel_id)
            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
        let snapshot_chat = validated.snapshot.chats.iter().find(|chat| {
            chat.chat.channel_id == channel_id && chat.chat.provider_chat_id == request.chat_id
        });
        if snapshot_chat.is_none() {
            return Err(RuntimeError::Application(
                "Choose a conversation included in the current task.".into(),
            ));
        }
        if let Some(message_id) = request.reply_to_message_id
            && !runtime.storage().snapshot_contains_message(
                validated.snapshot.snapshot_id,
                &channel_id,
                &request.chat_id,
                message_id,
            )?
        {
            return Err(RuntimeError::Application(
                "Choose a message included in the current conversation.".into(),
            ));
        }
        let mut attachments = Vec::with_capacity(request.resources.len());
        for resource in request.resources {
            let id = ResourceId::new(resource.resource_id)
                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
            let stored = runtime
                .storage()
                .resource_metadata(&validated.workspace_id, id)?
                .ok_or_else(|| RuntimeError::Application("resource not found".into()))?;
            if stored.sha256 != resource.sha256 || stored.size_bytes != resource.size_bytes {
                return Err(RuntimeError::Application(
                    "resource metadata mismatch".into(),
                ));
            }
            attachments.push(MessageAttachment {
                resource: lankide_core::ResourceRef {
                    resource_id: stored.resource_id.get(),
                    file_name: stored.file_name,
                    media_type: stored.media_type,
                    size_bytes: stored.size_bytes,
                    sha256: stored.sha256,
                },
                caption: None,
            });
        }
        let now = now_millis();
        let provider_idempotency_key = provider_send_idempotency_key(
            &format!("execution:{}", validated.execution_id),
            &request.idempotency_key,
        );
        let begun = match runtime.storage().begin_send_guarded(
            validated.snapshot.snapshot_id,
            &channel_id,
            &request.chat_id,
            request.force,
            SendRequest {
                workspace_id: validated.workspace_id.clone(),
                channel_id: channel_id.clone(),
                provider_chat_id: request.chat_id.clone(),
                request_key: provider_idempotency_key.clone(),
                sender_agent_id: validated.agent_id.clone(),
                content: request.content_markdown.clone(),
                now,
            },
        )? {
            GuardedSendResult::Ready(attempt) => attempt,
            GuardedSendResult::NewMessages { .. } => {
                return Ok(ProtocolSendMessageResult {
                    accepted: false,
                    message_id: 0,
                    provider_message_id: String::new(),
                    error: Some(ToolError {
                        code: "new_messages".into(),
                        message: "有新消息，请先读取，再决定是否发送以及发送什么内容。".into(),
                        retryable: true,
                    }),
                });
            }
        };
        if !begun.created {
            return Ok(send_attempt_result(begun));
        }
        let provider_result = runtime
            .send_message(
                &validated.workspace_id,
                validated.agent_id,
                SendMessageSpec {
                    channel_id,
                    chat_id: request.chat_id,
                    message_id: MessageId::new(begun.message_id)
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                    idempotency_key: provider_idempotency_key.clone(),
                    content: request.content_markdown,
                    reply_to: request
                        .reply_to_message_id
                        .map(MessageId::new)
                        .transpose()
                        .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
                    mentions: Vec::new(),
                    attachments,
                    force: request.force,
                    provider_agent_config: validated.runtime_spec.provider_agent_config,
                },
            )
            .await;
        match provider_result {
            Ok(result) if result.status == SendMessageStatus::Sent => {
                let provider_message_id = result.provider_message_id.ok_or_else(|| {
                    RuntimeError::InvalidCore("sent Provider result missing message id".into())
                })?;
                runtime.storage().finish_send(
                    &provider_idempotency_key,
                    SendOutcome::Sent {
                        provider_message_id: provider_message_id.clone(),
                    },
                    now,
                )?;
                Ok(ProtocolSendMessageResult {
                    accepted: true,
                    message_id: begun.message_id,
                    provider_message_id,
                    error: None,
                })
            }
            Ok(_) => {
                runtime.storage().finish_send(
                    &provider_idempotency_key,
                    SendOutcome::Unknown,
                    now,
                )?;
                Ok(ProtocolSendMessageResult {
                    accepted: false,
                    message_id: begun.message_id,
                    provider_message_id: String::new(),
                    error: Some(tool_error("send_unknown", false)),
                })
            }
            Err(error) => {
                let provider_error = match &error {
                    RuntimeError::ProviderCall(source) | RuntimeError::Provider { source, .. } => {
                        Some(source)
                    }
                    _ => None,
                };
                let unknown = provider_error.is_some_and(|source| {
                    matches!(
                        source,
                        lankide_provider::ProviderError::Timeout { .. }
                            | lankide_provider::ProviderError::External {
                                retryable: true,
                                ..
                            }
                    )
                });
                let outcome = if unknown {
                    SendOutcome::Unknown
                } else {
                    SendOutcome::Failed {
                        code: provider_error
                            .map(|source| source.code())
                            .unwrap_or("provider_send_failed")
                            .to_owned(),
                    }
                };
                runtime
                    .storage()
                    .finish_send(&provider_idempotency_key, outcome, now)?;
                Ok(ProtocolSendMessageResult {
                    accepted: false,
                    message_id: begun.message_id,
                    provider_message_id: String::new(),
                    error: Some(ToolError {
                        code: if unknown { "send_unknown" } else { "send_failed" }.into(),
                        message: if unknown {
                            "The send result is unknown. Check the conversation before sending again."
                        } else {
                            "The message could not be sent."
                        }
                        .into(),
                        retryable: false,
                    }),
                })
            }
        }
    }

    pub async fn get_content_gateway(
        &self,
        request: ContentRequest,
    ) -> Result<ContentResult, RuntimeError> {
        let context = request
            .context
            .as_ref()
            .ok_or_else(|| RuntimeError::Application("run context required".into()))?;
        let runtime = self.runtime()?;
        let validated = runtime.validate_run_context(context, now_millis()).await?;
        if ContentKind::try_from(request.kind).unwrap_or(ContentKind::Unspecified)
            != ContentKind::Message
        {
            return Err(RuntimeError::Application("unsupported content kind".into()));
        }
        let message_id = request
            .content_id
            .parse::<i64>()
            .map_err(|_| RuntimeError::Application("invalid message content id".into()))?;
        let message = runtime
            .storage()
            .snapshot_message(validated.snapshot.snapshot_id, message_id)?
            .ok_or_else(|| {
                RuntimeError::Application(
                    "This content is not available for the current task.".into(),
                )
            })?;
        Ok(ContentResult {
            content_id: request.content_id,
            content_markdown: message.content.clone(),
            structured_content_json: serde_json::json!({
                "message_id": message.message_id,
                "sender_id": message.sender_id,
                "sender_name": message.sender_name,
                "occurred_at": message.occurred_at,
            })
            .to_string(),
            resources: message
                .resources
                .iter()
                .map(|resource| ResourceRef {
                    resource_id: resource.resource_id.get(),
                    file_name: resource.file_name.clone(),
                    media_type: resource.media_type.clone(),
                    size_bytes: resource.size_bytes,
                    sha256: resource.sha256.clone(),
                })
                .collect(),
        })
    }
}

fn send_attempt_result(attempt: lankide_storage::SendAttemptView) -> ProtocolSendMessageResult {
    ProtocolSendMessageResult {
        accepted: attempt.state == "sent",
        message_id: attempt.message_id,
        provider_message_id: attempt.provider_message_id.unwrap_or_default(),
        error: (attempt.state != "sent").then(|| {
            tool_error(
                attempt
                    .error_code
                    .as_deref()
                    .unwrap_or(attempt.state.as_str()),
                attempt.state == "sending" || attempt.state == "unknown",
            )
        }),
    }
}

fn tool_error(code: &str, retryable: bool) -> ToolError {
    ToolError {
        code: code.to_owned(),
        message: code.to_owned(),
        retryable,
    }
}

async fn invoke_standard_tool(
    application: &RuntimeApplication,
    request: &CapabilityInvokeRequest,
    validated: &ValidatedRunContext,
    arguments: &serde_json::Value,
) -> Result<Option<CapabilityInvokeResult>, RuntimeError> {
    let result = match request.capability_name.as_str() {
        "message_recall" => {
            return Ok(Some(
                invoke_message_recall(application, request, validated, arguments).await?,
            ));
        }
        "message_edit" => {
            return Ok(Some(
                invoke_message_edit(application, request, validated, arguments).await?,
            ));
        }
        "unread_current" => validated.runtime_spec.unread_snapshot.clone(),
        "attention_set" => {
            let object = argument_object(arguments)?;
            let level = required_text(object, "level")?;
            let interval_minutes = object
                .get("interval_minutes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default() as u32;
            let attention = match level {
                "follow" if interval_minutes == 0 => lankide_storage::AttentionRecord::Always,
                "mention" if interval_minutes == 0 => lankide_storage::AttentionRecord::Mention,
                "mute" if interval_minutes == 0 => lankide_storage::AttentionRecord::Mute,
                "schedule"
                    if matches!(interval_minutes, 5 | 15 | 30 | 60 | 120 | 240 | 480 | 1440) =>
                {
                    lankide_storage::AttentionRecord::Schedule { interval_minutes }
                }
                _ => {
                    return Ok(Some(capability_error(
                        request.invocation_id.clone(),
                        "attention_level_invalid",
                        false,
                    )));
                }
            };
            let channel_id = ChannelId::new(required_text(object, "channel_id")?)
                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
            let chat_id = required_text(object, "chat_id")?;
            application
                .runtime()?
                .set_chat_attention(
                    &validated.workspace_id,
                    &validated.agent_id,
                    &channel_id,
                    chat_id,
                    attention,
                    now_millis(),
                )
                .await?;
            serde_json::json!({
                "channel_id": channel_id.as_str(),
                "chat_id": chat_id,
                "level": level,
                "interval_minutes": interval_minutes,
            })
        }
        "chat_messages" => {
            let object = argument_object(arguments)?;
            let channel_id = required_text(object, "channel_id")?;
            let chat_id = required_text(object, "chat_id")?;
            let before_message_id = object
                .get("before_message_id")
                .and_then(serde_json::Value::as_i64);
            let limit = object
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(50) as usize;
            let channel = ChannelId::new(channel_id)
                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
            let messages = application.runtime()?.storage().snapshot_chat_messages(
                validated.snapshot.snapshot_id,
                &channel,
                chat_id,
                before_message_id,
                limit,
            )?;
            serde_json::json!({
                "channel_id": channel.as_str(),
                "chat_id": chat_id,
                "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
                "boundary": "current",
            })
        }
        "message_send" => {
            let object = argument_object(arguments)?;
            if object.contains_key("content_file") {
                return Ok(Some(capability_error(
                    request.invocation_id.clone(),
                    "content_file_unresolved",
                    false,
                )));
            }
            let resources = object
                .get("attachments")
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(resource_argument)
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            let content = object
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if content.is_empty() && resources.is_empty() {
                return Ok(Some(capability_error(
                    request.invocation_id.clone(),
                    "message_content_required",
                    false,
                )));
            }
            let response = application
                .send_message_gateway(SendMessageRequest {
                    context: request.context.clone(),
                    channel_id: required_text(object, "channel_id")?.to_owned(),
                    chat_id: required_text(object, "chat_id")?.to_owned(),
                    content_markdown: content.to_owned(),
                    reply_to_message_id: object.get("reply_to").and_then(serde_json::Value::as_i64),
                    resources,
                    idempotency_key: request.invocation_id.clone(),
                    force: object
                        .get("force")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
                .await?;
            if let Some(error) = response.error {
                return Ok(Some(CapabilityInvokeResult {
                    invocation_id: request.invocation_id.clone(),
                    success: false,
                    content_markdown: String::new(),
                    structured_content_json: serde_json::json!({"message_id": response.message_id})
                        .to_string(),
                    resources: Vec::new(),
                    error: Some(error),
                }));
            }
            serde_json::json!({
                "message_id": response.message_id,
                "provider_message_id": response.provider_message_id,
            })
        }
        _ => return Ok(None),
    };
    Ok(Some(CapabilityInvokeResult {
        invocation_id: request.invocation_id.clone(),
        success: true,
        content_markdown: String::new(),
        structured_content_json: result.to_string(),
        resources: Vec::new(),
        error: None,
    }))
}

async fn invoke_message_recall(
    application: &RuntimeApplication,
    request: &CapabilityInvokeRequest,
    validated: &ValidatedRunContext,
    arguments: &serde_json::Value,
) -> Result<CapabilityInvokeResult, RuntimeError> {
    let runtime = application.runtime()?;
    let object = argument_object(arguments)?;
    let message_id = object
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| RuntimeError::Application("message_id required".into()))?;
    let invocation = ToolInvocationRequest {
        execution_id: validated.execution_id,
        attempt: i64::from(validated.attempt),
        tool_call_id: request.invocation_id.clone(),
        tool_name: "message_recall".into(),
        arguments: arguments.clone(),
        now: now_millis(),
    };
    let begun = runtime
        .storage()
        .begin_tool_invocation(invocation.clone())?;
    if !begun.created {
        return Ok(match begun.result {
            Some(value) => recall_capability_result(request.invocation_id.clone(), &value),
            None => capability_error(request.invocation_id.clone(), "in_progress", true),
        });
    }
    let guard = ToolInvocationGuard::new(runtime.storage(), invocation.clone());
    let result = match runtime.storage().recall_target(
        &validated.workspace_id,
        &validated.agent_id,
        message_id,
    )? {
        None => serde_json::json!({"success":false,"error":"message_not_found"}),
        Some(target) => match target.provider_message_id {
            None => serde_json::json!({"success":false,"error":"provider_message_id_missing"}),
            Some(provider_message_id) => {
                let message_id = MessageId::new(target.message_id)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                match runtime
                    .recall_message(
                        &validated.workspace_id,
                        validated.agent_id.clone(),
                        lankide_provider::RecallMessageSpec {
                            channel_id: target.channel_id,
                            chat_id: target.provider_chat_id,
                            message_id,
                            provider_message_id,
                            provider_agent_config: validated
                                .runtime_spec
                                .provider_agent_config
                                .clone(),
                        },
                    )
                    .await
                {
                    Ok(()) => serde_json::json!({"success":true,"message_id":target.message_id}),
                    Err(RuntimeError::ProviderCall(
                        lankide_provider::ProviderError::Unsupported { .. },
                    )) => {
                        serde_json::json!({"success":false,"error":"recall_unsupported"})
                    }
                    Err(_) => serde_json::json!({"success":false,"error":"recall_failed"}),
                }
            }
        },
    };
    guard.complete(result.clone(), now_millis())?;
    Ok(recall_capability_result(
        request.invocation_id.clone(),
        &result,
    ))
}

async fn invoke_message_edit(
    application: &RuntimeApplication,
    request: &CapabilityInvokeRequest,
    validated: &ValidatedRunContext,
    arguments: &serde_json::Value,
) -> Result<CapabilityInvokeResult, RuntimeError> {
    let runtime = application.runtime()?;
    let object = argument_object(arguments)?;
    let message_id = object
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| RuntimeError::Application("message_id required".into()))?;
    let content = required_text(object, "content")?.to_owned();
    let invocation = ToolInvocationRequest {
        execution_id: validated.execution_id,
        attempt: i64::from(validated.attempt),
        tool_call_id: request.invocation_id.clone(),
        tool_name: "message_edit".into(),
        arguments: arguments.clone(),
        now: now_millis(),
    };
    let begun = runtime
        .storage()
        .begin_tool_invocation(invocation.clone())?;
    if !begun.created {
        return Ok(match begun.result {
            Some(value) => edit_capability_result(request.invocation_id.clone(), &value),
            None => capability_error(request.invocation_id.clone(), "in_progress", true),
        });
    }
    let guard = ToolInvocationGuard::new(runtime.storage(), invocation.clone());
    let result = match runtime.storage().recall_target(
        &validated.workspace_id,
        &validated.agent_id,
        message_id,
    )? {
        None => serde_json::json!({"success":false,"error":"message_not_found"}),
        Some(target) => match target.provider_message_id {
            None => serde_json::json!({"success":false,"error":"provider_message_id_missing"}),
            Some(provider_message_id) => {
                let internal_message_id = MessageId::new(target.message_id)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                match runtime
                    .edit_message(
                        &validated.workspace_id,
                        validated.agent_id.clone(),
                        lankide_provider::EditMessageSpec {
                            channel_id: target.channel_id,
                            chat_id: target.provider_chat_id,
                            message_id: internal_message_id,
                            provider_message_id,
                            content: content.clone(),
                            provider_agent_config: validated
                                .runtime_spec
                                .provider_agent_config
                                .clone(),
                        },
                    )
                    .await
                {
                    Ok(()) => {
                        let updated = runtime.storage().update_outbound_message_content(
                            &validated.workspace_id,
                            &validated.agent_id,
                            target.message_id,
                            &content,
                            now_millis(),
                        )?;
                        if updated {
                            serde_json::json!({"success":true,"message_id":target.message_id})
                        } else {
                            serde_json::json!({"success":false,"error":"message_update_failed"})
                        }
                    }
                    Err(RuntimeError::ProviderCall(
                        lankide_provider::ProviderError::Unsupported { .. },
                    )) => serde_json::json!({"success":false,"error":"edit_unsupported"}),
                    Err(_) => serde_json::json!({"success":false,"error":"edit_failed"}),
                }
            }
        },
    };
    guard.complete(result.clone(), now_millis())?;
    Ok(edit_capability_result(
        request.invocation_id.clone(),
        &result,
    ))
}

fn edit_capability_result(
    invocation_id: String,
    value: &serde_json::Value,
) -> CapabilityInvokeResult {
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
        CapabilityInvokeResult {
            invocation_id,
            success: true,
            content_markdown: String::new(),
            structured_content_json: value.to_string(),
            resources: Vec::new(),
            error: None,
        }
    } else {
        capability_error(
            invocation_id,
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("edit_failed"),
            false,
        )
    }
}

fn recall_capability_result(
    invocation_id: String,
    value: &serde_json::Value,
) -> CapabilityInvokeResult {
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
        CapabilityInvokeResult {
            invocation_id,
            success: true,
            content_markdown: String::new(),
            structured_content_json: value.to_string(),
            resources: Vec::new(),
            error: None,
        }
    } else {
        capability_error(
            invocation_id,
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("recall_failed"),
            false,
        )
    }
}

fn capability_error(invocation_id: String, code: &str, retryable: bool) -> CapabilityInvokeResult {
    CapabilityInvokeResult {
        invocation_id,
        success: false,
        content_markdown: String::new(),
        structured_content_json: "{}".into(),
        resources: Vec::new(),
        error: Some(tool_error(code, retryable)),
    }
}

fn argument_object(
    value: &serde_json::Value,
) -> Result<&serde_json::Map<String, serde_json::Value>, RuntimeError> {
    value
        .as_object()
        .ok_or_else(|| RuntimeError::Application("tool arguments must be an object".into()))
}

fn required_text<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<&'a str, RuntimeError> {
    object
        .get(name)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeError::Application(format!("{name} required")))
}

fn resource_argument(value: &serde_json::Value) -> Result<ResourceRef, RuntimeError> {
    let object = argument_object(value)?;
    Ok(ResourceRef {
        resource_id: object
            .get("resource_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| RuntimeError::Application("attachment resource_id required".into()))?,
        file_name: required_text(object, "file_name")?.to_owned(),
        media_type: required_text(object, "media_type")?.to_owned(),
        size_bytes: object
            .get("size_bytes")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| RuntimeError::Application("attachment size_bytes required".into()))?,
        sha256: required_text(object, "sha256")?.to_owned(),
    })
}

fn parse_arguments(value: &str) -> Result<serde_json::Value, RuntimeError> {
    if value.trim().is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    serde_json::from_str(value)
        .map_err(|error| RuntimeError::Application(format!("invalid arguments: {error}")))
}
