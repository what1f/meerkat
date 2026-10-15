use super::*;

impl Runtime {
    pub async fn send_message(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        spec: SendMessageSpec,
    ) -> Result<SendMessageResult, RuntimeError> {
        self.host(workspace_id)
            .await?
            .send_message(agent_id, spec)
            .await
            .map_err(Into::into)
    }

    pub async fn invoke_provider_tool(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: AgentId,
        runtime_spec: AgentRuntimeSpec,
        request: ToolInvocationRequest,
    ) -> Result<ToolProgress, RuntimeError> {
        let begun = self.storage.begin_tool_invocation(request.clone())?;
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
        let guard = ToolInvocationGuard::new(self.storage.as_ref(), request.clone());
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
                guard.complete(result.clone(), request.now)?;
                Ok(ToolProgress::Completed { result })
            }
            Err(error) => {
                guard.fail("provider_tool_failed", now_millis())?;
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
        self.host(workspace_id)
            .await?
            .invoke_interface(interface_id, input)
            .await
            .map_err(Into::into)
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
            .execution_auth_context(execution.execution_id, i64::from(execution.attempt))?
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
        let snapshot_id = self.storage.execution(execution.execution_id)?.snapshot_id;
        let snapshot = self.storage.snapshot(snapshot_id)?;
        let authorization = self
            .storage
            .attempt_authorization(execution.execution_id, i64::from(execution.attempt))?
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
        computer_id: text("computer_id")?,
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
