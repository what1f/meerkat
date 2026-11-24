use super::*;

impl Runtime {
    pub(crate) async fn acknowledge_delivery(&self, execution_id: i64) -> Result<(), RuntimeError> {
        let execution = self.storage.execution(execution_id).await?;
        let snapshot = self.storage.snapshot(execution.snapshot_id).await?;
        for chat in &snapshot.chats {
            if let Some(message) = chat.recent_messages.last() {
                self.observe_message(
                    &snapshot.workspace_id,
                    &snapshot.agent_id,
                    &chat.chat.channel_id,
                    &chat.chat.provider_chat_id,
                    message.message_id,
                )
                .await?;
            }
        }
        Ok(())
    }
    pub(crate) async fn observe_message(
        &self,
        workspace: &WorkspaceId,
        agent_id: &AgentId,
        channel_id: &ChannelId,
        chat_id: &str,
        message_id: i64,
    ) -> Result<(), RuntimeError> {
        if !self
            .storage
            .record_chat_observation(workspace, agent_id, message_id, now_millis())
            .await?
        {
            return Ok(());
        }
        let provider = self.provider(workspace)?;
        let agent = provider.agents.read().await.get(agent_id).cloned();
        if !provider.channels.read().await.iter().any(|channel| {
            &channel.id == channel_id && channel.abilities.contains(&ChannelAbility::Reaction)
        }) {
            return Ok(());
        }
        if let Some(agent) = agent
            && let Some(auto_reaction) = agent.auto_reaction
        {
            let host = self.host(workspace).await?;
            let spec = AutoReactionSpec {
                auto_reaction,
                channel_id: channel_id.clone(),
                chat_id: chat_id.to_owned(),
                trigger_messages: vec![
                    MessageId::new(message_id)
                        .map_err(|e| RuntimeError::InvalidCore(e.to_string()))?,
                ],
                provider_agent_config: agent.provider_agent_config,
            };
            let agent_id = agent_id.clone();
            let mut tasks = self
                .background_tasks
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            while tasks.try_join_next().is_some() {}
            tasks.spawn(async move {
                let started = std::time::Instant::now();
                let result = host.auto_reaction(agent_id.clone(), spec).await;
                log::info!(
                    "agent={agent_id} message={message_id} read_reaction_request_ms={} success={}",
                    started.elapsed().as_millis(),
                    result.is_ok()
                );
                if let Err(error) = result {
                    log::error!("[runtime] agent={agent_id} read acknowledgement failed: {error}");
                }
            });
        }
        Ok(())
    }

    async fn related_context(&self, snapshot: &SnapshotView) -> Result<String, RuntimeError> {
        let mut counts = BTreeMap::<String, usize>::new();
        let mut output = String::new();
        for (chat, message) in self
            .storage
            .observed_chats(&snapshot.workspace_id, &snapshot.agent_id)
            .await?
        {
            if snapshot
                .chats
                .iter()
                .any(|current| current.chat.chat_id == chat.chat_id)
            {
                continue;
            }
            let count = counts.entry(chat.channel_id.to_string()).or_default();
            if *count >= 3 {
                continue;
            }
            *count += 1;
            output.push_str(&format!(
                "\n- {}（channel_id={}，chat_id={}，最近查看 message_id={}）：{}",
                chat.chat_name,
                chat.channel_id,
                chat.provider_chat_id,
                message.message_id,
                message.content.chars().take(80).collect::<String>()
            ));
        }
        if output.is_empty() {
            Ok(output)
        } else {
            Ok(format!("\n\n关联上下文索引（需要时主动读取）：{output}"))
        }
    }

    /// Resume the native session unless it has gone cold and large; then the
    /// Machine starts a new one seeded with the recovery context.
    async fn session_directive(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        agent: &crate::AgentRuntimeConfig,
        now: i64,
    ) -> Result<meerkat_protocol::SessionDirective, RuntimeError> {
        // The configured Agent is cached at startup; the native session it
        // last bound is in storage.
        let workspace = self.storage.workspace_record(workspace_id).await?;
        let record = self.storage.agent_record(workspace_id, agent_id).await?;
        let mut session = crate::application::agent_runtime_config(&workspace, record)?.session;
        if session.action != meerkat_protocol::SessionAction::ResumeOrStart as i32 {
            return Ok(session);
        }
        if let Some(activity) = self
            .storage
            .session_activity(workspace_id, agent_id)
            .await?
            && crate::session_recovery::should_start_fresh(
                &agent.harness,
                activity.last_completed_at,
                activity.last_context_tokens,
                now,
            )
        {
            log::info!(
                "workspace={workspace_id} agent={agent_id} fresh_session idle_ms={} context_tokens={}",
                now.saturating_sub(activity.last_completed_at),
                activity.last_context_tokens.unwrap_or_default()
            );
            session.action = meerkat_protocol::SessionAction::StartNew as i32;
            session.native_session_id = None;
        }
        Ok(session)
    }

    /// Recent conversations for a fresh native session; Machine drops it when
    /// the session resumes.
    async fn recovery_context(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        snapshot: &SnapshotView,
        now: i64,
    ) -> Result<String, RuntimeError> {
        use crate::session_recovery::*;
        let conversations = self
            .storage
            .recent_conversations(
                workspace_id,
                agent_id,
                now.saturating_sub(RECENT_WINDOW_MS),
                RECENT_CHAT_LIMIT,
                RECENT_MESSAGE_LIMIT,
            )
            .await?;
        Ok(render_recovery(&conversations, snapshot, now))
    }

    /// Builds Provider-owned prompt and capabilities from the same runtime
    /// context that will be sent to Machine.
    pub async fn dynamic_execution_input(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> Result<DynamicExecutionInput, RuntimeError> {
        self.require_active(workspace_id)?;
        let host = self.host(workspace_id).await?;
        let preparation_started = std::time::Instant::now();
        let provider_prompt = host.system_prompt(agent_id.clone(), spec.clone()).await?;
        let workspace = self.storage.workspace_record(workspace_id).await?;
        let humans = self
            .storage
            .list_human_team_member_views(workspace_id)
            .await?;
        let agents = self.storage.list_agent_records(workspace_id).await?;
        let system_prompt = prompt::system_prompt(
            &workspace,
            &agent_id,
            &spec.agent_config,
            &humans,
            &agents,
            &provider_prompt,
        );
        let context_index = host.context_index(agent_id.clone(), spec.clone()).await?;
        let tools_started = std::time::Instant::now();
        let tools = host.tools(agent_id.clone(), spec.clone()).await?;
        let tools_ms = tools_started.elapsed().as_millis();
        let cli_commands = host.cli_commands(agent_id.clone(), spec).await?;
        log::info!(
            "workspace={workspace_id} agent={agent_id} input_preparation_ms={} tools_ms={tools_ms}",
            preparation_started.elapsed().as_millis()
        );
        Ok(DynamicExecutionInput {
            system_prompt,
            context_index,
            tools: tools
                .into_iter()
                .map(protocol_tool)
                .collect::<Result<Vec<_>, _>>()?,
            cli_commands: cli_commands.into_iter().map(protocol_cli).collect(),
        })
    }

    /// Claims all currently available unread Chats for one Agent and executes
    /// exactly that frozen multi-Chat snapshot.
    pub async fn execute_ready<C: MachineExecutor>(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        machine: &C,
        now: i64,
    ) -> Result<ExecutionProgress, RuntimeError> {
        self.require_active(workspace_id)?;
        let agent = self
            .provider(workspace_id)?
            .agents
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownAgent {
                workspace_id: workspace_id.clone(),
                agent_id: agent_id.clone(),
            })?;
        self.apply_pending_failure_policies(workspace_id, agent_id)
            .await?;
        // Recovery after Runtime restart has no in-memory waiter. Storage still
        // owns the Agent until the attempt is completed or declared lost.
        if !self
            .storage
            .active_execution_ids_for_agent(workspace_id, agent_id)
            .await?
            .is_empty()
        {
            return Ok(ExecutionProgress::Idle);
        }
        let execution_token = generate_execution_token()?;
        let provider_send_namespace = token_hash(&execution_token);
        let Some(created) = self
            .storage
            .create_execution_with_auth(
                workspace_id,
                agent_id,
                "unread",
                now,
                ExecutionAuthSpec {
                    attempt: 1,
                    token_hash: token_hash(&execution_token),
                    expires_at: now
                        .saturating_add(self.execution_token_lifetime_ms.load(Ordering::Relaxed)),
                },
            )
            .await?
        else {
            return Ok(ExecutionProgress::Idle);
        };
        let snapshot_json = snapshot_json(&created.snapshot);
        let runtime_spec = AgentRuntimeSpec {
            agent_config: agent.agent_config.clone(),
            provider_agent_config: agent.provider_agent_config.clone(),
            machine_id: agent.machine_id.clone(),
            harness_id: agent.harness.harness_id.clone(),
            model: agent.harness.model.clone(),
            thinking_level: agent.harness.thinking_level.clone(),
            trigger_reason: trigger_summary(&created.snapshot),
            chats: runtime_chats(&created.snapshot),
            unread_snapshot: snapshot_json.clone(),
        };
        let mut dynamic = match self
            .dynamic_execution_input(workspace_id, agent_id.clone(), runtime_spec.clone())
            .await
        {
            Ok(input) => input,
            Err(error) => {
                // Provider errors contain typed codes and static operation names,
                // never raw HTTP responses, configuration, or credentials.
                let detail = match &error {
                    RuntimeError::ProviderCall(source) | RuntimeError::Provider { source, .. } => {
                        match source {
                            ProviderError::External { operation, .. }
                            | ProviderError::Timeout { operation }
                            | ProviderError::Unsupported { operation } => {
                                format!("{error} ({operation})")
                            }
                            ProviderError::InvalidInput { field } => format!("{error} ({field})"),
                            ProviderError::InvalidSchema { location }
                            | ProviderError::SchemaMismatch { location } => {
                                format!("{error} ({location})")
                            }
                            _ => format!("runtime_provider_call: {}", source.code()),
                        }
                    }
                    // Dynamic messages may include Provider-supplied values.
                    RuntimeError::InvalidCore(_) => "runtime_invalid_core".into(),
                    RuntimeError::InvalidProviderSchema(_) => {
                        "runtime_invalid_provider_schema".into()
                    }
                    RuntimeError::Application(_) | RuntimeError::ActionRequired(_) => {
                        "runtime_application".into()
                    }
                    RuntimeError::Storage(source) => match source {
                        StorageError::Sqlite(error) => {
                            format!("runtime_storage: sqlite {:?}", error.sqlite_error_code())
                        }
                        StorageError::Json(_) => "runtime_storage: invalid JSON".into(),
                        StorageError::Invalid(_)
                        | StorageError::Conflict(_)
                        | StorageError::Busy(_) => "runtime_storage: unavailable data".into(),
                        _ => source.to_string(),
                    },
                    _ => error.to_string(),
                };
                log::error!(
                    "workspace={workspace_id} agent={agent_id} execution={} input_preparation_failed: {detail}",
                    created.execution_id
                );
                self.storage
                    .fail_execution_retry_after(
                        created.execution_id,
                        "input_preparation_failed",
                        &detail,
                        now.saturating_add(60_000),
                        now,
                    )
                    .await?;
                return Err(error);
            }
        };
        let mut tools = capability::standard_tool_definitions();
        tools.append(&mut dynamic.tools);
        let context_json = frozen_runtime_spec_json(&runtime_spec).to_string();
        let context_fingerprint = lowercase_sha256(context_json.as_bytes());
        let mut capabilities = tools
            .iter()
            .map(|tool| AttemptCapability {
                name: tool.name.clone(),
                display_name: tool.name.clone(),
                description: tool.description.clone(),
                kind: meerkat_protocol::CapabilityKind::Tool as i32,
                input_schema_json: tool.input_schema_json.clone(),
                output_schema_json: tool.output_schema_json.clone(),
            })
            .collect::<Vec<_>>();
        capabilities.extend(
            dynamic
                .cli_commands
                .iter()
                .map(|command| AttemptCapability {
                    name: cli_capability_name(command),
                    display_name: command.name.clone(),
                    description: command.description.clone(),
                    kind: meerkat_protocol::CapabilityKind::CliCommand as i32,
                    input_schema_json: r#"{"type":"array","items":{"type":"string"}}"#.into(),
                    output_schema_json: "{}".into(),
                }),
        );
        if let Err(error) = self
            .storage
            .freeze_attempt_authorization(AttemptAuthorization {
                execution_id: created.execution_id,
                attempt: 1,
                context_json,
                context_fingerprint,
                capabilities,
            })
            .await
        {
            self.storage
                .fail_execution(
                    created.execution_id,
                    "authorization_preparation_failed",
                    now,
                )
                .await?;
            return Err(error.into());
        }
        let execution_ref = ExecutionRef {
            execution_id: created.execution_id,
            attempt: 1,
        };
        let session = self
            .session_directive(workspace_id, agent_id, &agent, now)
            .await?;
        let recovery_markdown = self
            .recovery_context(workspace_id, agent_id, &created.snapshot, now)
            .await?;
        let assignment = AssignExecution {
            execution: Some(execution_ref),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            agent: Some(agent.agent_profile.clone()),
            harness: Some(agent.harness.clone()),
            session: Some(session),
            initial_delivery: Some(Delivery {
                delivery_id: created.delivery_id,
                kind: DeliveryKind::Initial as i32,
                rendered_markdown: format!(
                    "{}{}{}",
                    render_snapshot(&created.snapshot),
                    self.related_context(&created.snapshot).await?,
                    dynamic.context_index
                ),
                snapshot_id: created.snapshot_id,
                trigger_reasons: trigger_reasons(&created.snapshot),
                resources: snapshot_resources(&created.snapshot),
                created_at: now,
            }),
            system_prompt: dynamic.system_prompt,
            tools,
            cli_commands: dynamic.cli_commands,
            resources: snapshot_resources(&created.snapshot),
            execution_token,
            created_at: now,
            recovery_markdown,
        };
        self.storage
            .mark_execution_running(created.execution_id, now)
            .await?;
        let completion = match machine.execute(assignment).await {
            Ok(completion) => completion,
            Err(error) => {
                log::error!(
                    "execution={} machine failed: {}",
                    created.execution_id,
                    error.code
                );
                if error.code == "machine_disconnected" {
                    self.storage
                        .mark_execution_recovering(created.execution_id, now)
                        .await?;
                } else {
                    self.storage
                        .fail_execution(created.execution_id, "machine_unavailable", now)
                        .await?;
                }
                return Ok(ExecutionProgress::Failed {
                    execution_id: created.execution_id,
                    code: error.code,
                });
            }
        };
        if !completion.delivery_accepted
            && matches!(completion.outcome, MachineOutcome::Succeeded { .. })
        {
            self.storage
                .fail_execution(created.execution_id, "delivery_rejected", now)
                .await?;
            return Ok(ExecutionProgress::Failed {
                execution_id: created.execution_id,
                code: "delivery_rejected".into(),
            });
        }
        if completion.delivery_accepted {
            self.storage
                .mark_delivery_delivered(created.delivery_id, now)
                .await?;
            self.acknowledge_delivery(created.execution_id).await?;
        }
        match completion.outcome {
            MachineOutcome::Failed { code } => {
                log::error!("execution={} failed: {code}", created.execution_id);
                self.storage
                    .fail_execution(created.execution_id, &code, now_millis())
                    .await?;
                self.apply_pending_failure_policies(workspace_id, agent_id)
                    .await?;
                Ok(ExecutionProgress::Failed {
                    execution_id: created.execution_id,
                    code,
                })
            }
            MachineOutcome::Cancelled => {
                self.storage
                    .cancel_execution(created.execution_id, now)
                    .await?;
                self.apply_pending_failure_policies(workspace_id, agent_id)
                    .await?;
                Ok(ExecutionProgress::Cancelled {
                    execution_id: created.execution_id,
                })
            }
            MachineOutcome::Succeeded { output, reply } => {
                let Some(reply) = reply else {
                    let committed = self
                        .storage
                        .commit_execution(created.execution_id, &output, None, now)
                        .await?;
                    return Ok(ExecutionProgress::Committed {
                        execution_id: created.execution_id,
                        committed: matches!(
                            committed,
                            meerkat_storage::CommitResult::Committed
                                | meerkat_storage::CommitResult::AlreadyCommitted
                        ),
                    });
                };
                let request_key = format!("execution:{}:reply", created.execution_id);
                let send = match self
                    .storage
                    .begin_send_guarded(
                        created.snapshot_id,
                        &reply.channel_id,
                        &reply.provider_chat_id,
                        false,
                        SendRequest {
                            workspace_id: workspace_id.clone(),
                            channel_id: reply.channel_id.clone(),
                            provider_chat_id: reply.provider_chat_id.clone(),
                            request_key: request_key.clone(),
                            sender_agent_id: agent_id.clone(),
                            content: reply.content.clone(),
                            now,
                        },
                    )
                    .await?
                {
                    GuardedSendResult::Ready(send) => send,
                    GuardedSendResult::NewMessages { .. } => {
                        let committed = self
                            .storage
                            .commit_execution(created.execution_id, &output, None, now)
                            .await?;
                        return Ok(ExecutionProgress::Committed {
                            execution_id: created.execution_id,
                            committed: matches!(
                                committed,
                                meerkat_storage::CommitResult::Committed
                                    | meerkat_storage::CommitResult::AlreadyCommitted
                            ),
                        });
                    }
                };
                let message_id = MessageId::new(send.message_id)
                    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
                let provider_result = self
                    .send_message(
                        workspace_id,
                        agent_id.clone(),
                        SendMessageSpec {
                            channel_id: reply.channel_id,
                            chat_id: reply.provider_chat_id,
                            message_id,
                            idempotency_key: provider_send_idempotency_key(
                                &provider_send_namespace,
                                &request_key,
                            ),
                            content: reply.content,
                            reply_to: None,
                            mentions: Vec::new(),
                            attachments: Vec::new(),
                            force: false,
                            provider_agent_config: agent.provider_agent_config,
                        },
                    )
                    .await;
                match provider_result {
                    Ok(result) if result.status == meerkat_provider::SendMessageStatus::Sent => {
                        let provider_message_id = result.provider_message_id.ok_or_else(|| {
                            RuntimeError::InvalidCore(
                                "sent Provider result missing message id".into(),
                            )
                        })?;
                        self.storage
                            .finish_send(
                                &request_key,
                                SendOutcome::Sent {
                                    provider_message_id,
                                },
                                now,
                            )
                            .await?;
                        let committed = self
                            .storage
                            .commit_execution(
                                created.execution_id,
                                &output,
                                Some(&request_key),
                                now,
                            )
                            .await?;
                        Ok(ExecutionProgress::Committed {
                            execution_id: created.execution_id,
                            committed: matches!(
                                committed,
                                meerkat_storage::CommitResult::Committed
                                    | meerkat_storage::CommitResult::AlreadyCommitted
                            ),
                        })
                    }
                    Ok(_) => {
                        self.storage
                            .finish_send(&request_key, SendOutcome::Unknown, now)
                            .await?;
                        Ok(ExecutionProgress::PendingSend {
                            execution_id: created.execution_id,
                            request_key,
                        })
                    }
                    Err(error) => {
                        log::error!(
                            "execution={} provider send failed: {error}",
                            created.execution_id
                        );
                        self.storage
                            .finish_send(
                                &request_key,
                                SendOutcome::Failed {
                                    code: "provider_send_failed".into(),
                                },
                                now,
                            )
                            .await?;
                        self.storage
                            .fail_execution(created.execution_id, "provider_send_failed", now)
                            .await?;
                        Ok(ExecutionProgress::Failed {
                            execution_id: created.execution_id,
                            code: "provider_send_failed".into(),
                        })
                    }
                }
            }
        }
    }

    pub async fn recoverable_executions(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<meerkat_storage::RecoverableExecutionView>, RuntimeError> {
        Ok(self.storage.recoverable_executions(workspace_id).await?)
    }

    pub async fn recoverable_sends(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<meerkat_storage::RecoverableSendView>, RuntimeError> {
        Ok(self.storage.recoverable_sends(workspace_id).await?)
    }

    /// Resolves a Provider send whose outcome was unknown across a restart.
    /// The execution is committed only after Storage has durably linked the
    /// returned Provider message id to the pre-existing outbound Message.
    pub async fn resolve_unknown_send(
        &self,
        workspace_id: &WorkspaceId,
        request_key: &str,
        provider_message_id: String,
        now: i64,
    ) -> Result<ExecutionProgress, RuntimeError> {
        let recoverable = self
            .storage
            .recoverable_sends(Some(workspace_id))
            .await?
            .into_iter()
            .find(|send| send.attempt.request_key == request_key)
            .ok_or_else(|| RuntimeError::UnknownSend(request_key.to_owned()))?;
        self.storage
            .finish_send(
                request_key,
                SendOutcome::Sent {
                    provider_message_id,
                },
                now,
            )
            .await?;
        let execution_id = execution_id_from_request_key(request_key)?;
        let committed = self
            .storage
            .commit_execution(execution_id, &recoverable.content, Some(request_key), now)
            .await?;
        Ok(ExecutionProgress::Committed {
            execution_id,
            committed: matches!(
                committed,
                meerkat_storage::CommitResult::Committed
                    | meerkat_storage::CommitResult::AlreadyCommitted
            ),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynamicExecutionInput {
    pub system_prompt: String,
    pub context_index: String,
    pub tools: Vec<ProtocolToolDefinition>,
    pub cli_commands: Vec<CliCommand>,
}

pub type MachineFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, MachineError>> + Send + 'a>>;

pub trait MachineExecutor: Send + Sync {
    fn execute(&self, assignment: AssignExecution) -> MachineFuture<'_, MachineCompletion>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineError {
    pub code: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MachineCompletion {
    pub delivery_accepted: bool,
    pub outcome: MachineOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MachineOutcome {
    Succeeded {
        output: String,
        reply: Option<ExecutionReply>,
    },
    Failed {
        code: String,
    },
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReply {
    pub channel_id: ChannelId,
    pub provider_chat_id: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionProgress {
    Idle,
    Committed {
        execution_id: i64,
        committed: bool,
    },
    PendingSend {
        execution_id: i64,
        request_key: String,
    },
    Failed {
        execution_id: i64,
        code: String,
    },
    Cancelled {
        execution_id: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolProgress {
    InProgress,
    Completed { result: serde_json::Value },
    Failed { code: String },
}

pub(crate) struct ToolInvocationGuard {
    storage: Database,
    request: Option<ToolInvocationRequest>,
}

impl ToolInvocationGuard {
    pub(crate) fn new(storage: Database, request: ToolInvocationRequest) -> Self {
        Self {
            storage,
            request: Some(request),
        }
    }

    pub(crate) async fn complete(
        mut self,
        result: serde_json::Value,
        now: i64,
    ) -> Result<(), RuntimeError> {
        let request = self.request.as_ref().expect("active tool invocation");
        self.storage
            .finish_tool_invocation(request, result, now)
            .await?;
        self.request = None;
        Ok(())
    }

    pub(crate) async fn fail(mut self, code: &str, now: i64) -> Result<(), RuntimeError> {
        let request = self.request.as_ref().expect("active tool invocation");
        self.storage
            .fail_tool_invocation(request, code, now)
            .await?;
        self.request = None;
        Ok(())
    }
}

impl Drop for ToolInvocationGuard {
    fn drop(&mut self) {
        let Some(request) = self.request.take() else {
            return;
        };
        let database = self.storage.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = database.as_ref().fail_tool_invocation(
                &request,
                "invocation_cancelled",
                now_millis(),
            ) {
                log::error!(
                    "[runtime] execution={} attempt={} tool_call={} finalization failed: {error}",
                    request.execution_id,
                    request.attempt,
                    request.tool_call_id
                );
            }
            database.wake();
        });
    }
}

fn protocol_tool(value: ToolDefinition) -> Result<ProtocolToolDefinition, RuntimeError> {
    Ok(ProtocolToolDefinition {
        name: value.id,
        description: value.description,
        input_schema_json: serde_json::to_string(value.input_schema.value())
            .map_err(RuntimeError::InvalidProviderSchema)?,
        output_schema_json: serde_json::to_string(value.output_schema.value())
            .map_err(RuntimeError::InvalidProviderSchema)?,
    })
}

/// Wrapped commands return credentials for `meerkat exec`, so they use their
/// own prefix and the generic `meerkat invoke` path refuses them.
pub(crate) fn cli_capability_name(command: &CliCommand) -> String {
    if command.wraps.is_empty() {
        format!("cli: {}", command.name)
    } else {
        format!("exec: {}", command.name)
    }
}

fn protocol_cli(value: CliCommandDefinition) -> CliCommand {
    CliCommand {
        name: value.id,
        description: value.description,
        argv: Vec::new(),
        wraps: value.wraps,
        private_directories: value.private_directories,
    }
}

fn trigger_summary(snapshot: &SnapshotView) -> String {
    let mut values = trigger_reasons(snapshot);
    values.dedup();
    values.join(",")
}

fn trigger_reasons(snapshot: &SnapshotView) -> Vec<String> {
    snapshot
        .chats
        .iter()
        .flat_map(|chat| chat.trigger_reasons.iter())
        .map(trigger_reason_name)
        .collect()
}

fn trigger_reason_name(reason: &DispatchReason) -> String {
    match reason {
        DispatchReason::DirectMessage => "direct_message".to_owned(),
        DispatchReason::Mention => "mention".to_owned(),
        DispatchReason::Follow => "follow".to_owned(),
        DispatchReason::Schedule => "schedule".to_owned(),
        DispatchReason::Reaction { reaction, .. } => format!("reaction:{reaction}"),
        DispatchReason::Custom { name, .. } => format!("custom:{name}"),
        DispatchReason::Timer { timer_id, .. } => format!("timer:{timer_id}"),
        DispatchReason::Moved { .. } => "moved".to_owned(),
    }
}

fn snapshot_json(snapshot: &SnapshotView) -> serde_json::Value {
    serde_json::json!({
        "snapshot_id": snapshot.snapshot_id,
        "workspace_id": snapshot.workspace_id.as_str(),
        "agent_id": snapshot.agent_id.as_str(),
        "created_at": snapshot.created_at,
        "chats": snapshot.chats.iter().map(|chat| serde_json::json!({
            "provider_id": chat.chat.provider_id.as_str(),
            "channel_id": chat.chat.channel_id.as_str(),
            "channel_name": chat.chat.channel_name,
            "chat_id": chat.chat.provider_chat_id,
            "chat_name": chat.chat.chat_name,
            "unread_count": chat.unread_count,
            "read_to_message_id": chat.read_to_message_id,
            "recent_messages": chat.recent_messages.iter().map(message_preview_json).collect::<Vec<_>>(),
            "mentioned_message": chat.mentioned_message.as_ref().map(message_preview_json),
            "trigger_messages": chat.trigger_messages.iter().map(message_preview_json).collect::<Vec<_>>(),
            "trigger_reasons": chat.trigger_reasons.iter().map(|reason| serde_json::to_value(reason).unwrap_or(serde_json::Value::Null)).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn message_preview_json(message: &meerkat_storage::MessageView) -> serde_json::Value {
    let mut value = message_json(message);
    value["content"] = serde_json::Value::String(truncate_message(&message.content));
    if let Some(reply) = &message.reply_to {
        value["reply_to"]["content"] = serde_json::Value::String(truncate_message(&reply.content));
    }
    value
}

pub(super) fn message_json(message: &meerkat_storage::MessageView) -> serde_json::Value {
    let sender_kind = match message.sender_kind {
        MessageAuthorType::Human => "human",
        MessageAuthorType::Agent => "agent",
        MessageAuthorType::Bot => "bot",
        MessageAuthorType::System => "system",
    };
    serde_json::json!({
        "message_id": message.message_id,
        "sender": {
            "kind": sender_kind,
            "id": message.sender_id,
            "name": message.sender_name,
        },
        "content": message.content,
        "reply_to_message_id": message.reply_to.as_ref().map(|reply| reply.message_id),
        "reply_to": message.reply_to.as_ref().map(|reply| message_json(reply)),
        "reply_unavailable": message.reply_to_provider_message_id.is_some() && message.reply_to.is_none(),
        "resources": message.resources.iter().map(|resource| serde_json::json!({
            "resource_id": resource.resource_id.get(),
            "file_name": resource.file_name,
            "media_type": resource.media_type,
            "size_bytes": resource.size_bytes,
            "sha256": resource.sha256,
        })).collect::<Vec<_>>(),
        "mentions": message.mentions.iter().map(AgentId::as_str).collect::<Vec<_>>(),
        "occurred_at": message.occurred_at,
    })
}

fn render_snapshot(snapshot: &SnapshotView) -> String {
    let mut markdown = String::from("未读事项：");
    for chat in &snapshot.chats {
        let kind = match chat.chat.channel_id.as_str() {
            "group_chat" => "群聊",
            "private_chat" => "私聊",
            "task" => "任务",
            _ => chat.chat.channel_name.as_str(),
        };
        let name = if chat.chat.chat_name.trim().is_empty() {
            chat.chat.provider_chat_id.as_str()
        } else {
            chat.chat.chat_name.as_str()
        };
        markdown.push_str("\n\n### ");
        markdown.push_str(kind);
        markdown.push('：');
        markdown.push_str(name);
        markdown.push_str("（channel_id=");
        markdown.push_str(chat.chat.channel_id.as_str());
        markdown.push_str("，chat_id=");
        markdown.push_str(&chat.chat.provider_chat_id);
        markdown.push('）');
        for message in &chat.recent_messages {
            markdown.push_str("\n- [message_id=");
            markdown.push_str(&message.message_id.to_string());
            markdown.push_str("] ");
            if message.sender_name.trim().is_empty() {
                markdown.push_str(&message.sender_id);
            } else {
                markdown.push_str(&message.sender_name);
            }
            markdown.push_str(": ");
            if snapshot.created_at.saturating_sub(message.occurred_at) > 60_000 {
                markdown.push_str(&format!(
                    "[原始发送时间={}；此消息并非刚刚发送] ",
                    message.occurred_at
                ));
            }
            markdown.push_str(&truncate_message(&message.content));
            render_attachment_links(&mut markdown, message);
            render_reply(&mut markdown, message);
        }
        for reason in &chat.trigger_reasons {
            match reason {
                DispatchReason::Timer { timer_id, prompt } => markdown.push_str(&format!(
                    "\n- 你设置的定时任务到点了 [timer_id={timer_id}]：{prompt}"
                )),
                DispatchReason::Moved { from, to } => {
                    markdown.push_str(&format!("\n- 你刚从「{from}」工区搬到了「{to}」工区。"))
                }
                _ => {}
            }
        }
        for (reason, message) in chat.trigger_reasons.iter().zip(&chat.trigger_messages) {
            if let DispatchReason::Reaction {
                reaction,
                sender_name,
            } = reason
            {
                markdown.push_str(&format!(
                    "\n- {sender_name} 回应了 {reaction}：[message_id={}] {}: {}",
                    message.message_id,
                    if message.sender_name.trim().is_empty() {
                        &message.sender_id
                    } else {
                        &message.sender_name
                    },
                    truncate_message(&message.content),
                ));
                render_attachment_links(&mut markdown, message);
                render_reply(&mut markdown, message);
            }
        }
        if chat.unread_count > chat.recent_messages.len() {
            markdown.push_str("\n- 还有更早的未读内容，需要时使用 chat_messages 继续查看。");
        }
        if let Some(mentioned) = chat.mentioned_message.as_ref()
            && !chat
                .recent_messages
                .iter()
                .any(|message| message.message_id == mentioned.message_id)
        {
            markdown.push_str("\n\n有人 @ 你：[message_id=");
            markdown.push_str(&mentioned.message_id.to_string());
            markdown.push_str("] ");
            markdown.push_str(&truncate_message(&mentioned.content));
            render_attachment_links(&mut markdown, mentioned);
            render_reply(&mut markdown, mentioned);
        }
    }
    markdown.push_str("\n\n<system-reminder>以上是本轮冻结的未读会话。每个会话只展示最近几条内容；需要完整上下文时，用 chat_messages 主动读取。不要假装已经看过未展示的内容。</system-reminder>");
    markdown
}

fn render_reply(markdown: &mut String, message: &meerkat_storage::MessageView) {
    if let Some(reply) = &message.reply_to {
        markdown.push_str(&format!(
            "\n  > 引用消息 [message_id={}] {}: {}",
            reply.message_id,
            if reply.sender_name.trim().is_empty() {
                &reply.sender_id
            } else {
                &reply.sender_name
            },
            truncate_message(&reply.content)
        ));
        render_attachment_links(markdown, reply);
    } else if message.reply_to_provider_message_id.is_some() {
        markdown.push_str("\n  > 引用了另一条消息，原消息暂不可用。不要猜测引用内容。");
    }
}

fn render_attachment_links(markdown: &mut String, message: &meerkat_storage::MessageView) {
    for resource in &message.resources {
        markdown.push_str(&format!(" resource://{}", resource.resource_id));
    }
}

fn runtime_chats(snapshot: &SnapshotView) -> Vec<meerkat_provider::AgentRuntimeChat> {
    snapshot
        .chats
        .iter()
        .map(|chat| meerkat_provider::AgentRuntimeChat {
            provider_id: chat.chat.provider_id.clone(),
            channel_id: chat.chat.channel_id.clone(),
            channel_name: chat.chat.channel_name.clone(),
            chat_id: chat.chat.provider_chat_id.clone(),
            chat_name: chat.chat.chat_name.clone(),
        })
        .collect()
}

fn frozen_runtime_spec_json(spec: &AgentRuntimeSpec) -> serde_json::Value {
    serde_json::json!({
        "agent_config": spec.agent_config,
        "provider_agent_config": spec.provider_agent_config,
        "machine_id": spec.machine_id,
        "harness_id": spec.harness_id,
        "model": spec.model,
        "thinking_level": spec.thinking_level,
        "trigger_reason": spec.trigger_reason,
        "chats": spec.chats.iter().map(|chat| serde_json::json!({
            "provider_id": chat.provider_id.as_str(),
            "channel_id": chat.channel_id.as_str(),
            "channel_name": chat.channel_name,
            "chat_id": chat.chat_id,
            "chat_name": chat.chat_name,
        })).collect::<Vec<_>>(),
        "unread_snapshot": spec.unread_snapshot,
    })
}

pub(super) fn lowercase_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn provider_send_idempotency_key(namespace: &str, request_key: &str) -> String {
    lowercase_sha256(format!("{namespace}\0{request_key}").as_bytes())
}

fn truncate_message(content: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut chars = content.chars();
    let truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        let mut truncated = truncated;
        truncated.pop();
        truncated.push('…');
        truncated
    } else {
        truncated
    }
}

fn snapshot_resources(snapshot: &SnapshotView) -> Vec<ResourceRef> {
    let mut resources = BTreeMap::new();
    for message in snapshot.chats.iter().flat_map(|chat| {
        chat.recent_messages
            .iter()
            .chain(&chat.trigger_messages)
            .chain(chat.mentioned_message.iter())
    }) {
        for resource in message
            .resources
            .iter()
            .chain(message.reply_to.iter().flat_map(|reply| &reply.resources))
        {
            resources
                .entry(resource.resource_id.get())
                .or_insert_with(|| ResourceRef {
                    resource_id: resource.resource_id.get(),
                    file_name: resource.file_name.clone(),
                    media_type: resource.media_type.clone(),
                    size_bytes: resource.size_bytes,
                    sha256: resource.sha256.clone(),
                });
        }
    }
    resources.into_values().collect()
}

fn execution_id_from_request_key(request_key: &str) -> Result<i64, RuntimeError> {
    let value = request_key
        .strip_prefix("execution:")
        .and_then(|value| value.strip_suffix(":reply"))
        .ok_or_else(|| RuntimeError::UnknownSend(request_key.to_owned()))?;
    value
        .parse()
        .map_err(|_| RuntimeError::UnknownSend(request_key.to_owned()))
}

fn generate_execution_token() -> Result<String, RuntimeError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| RuntimeError::Application(format!("execution token: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
