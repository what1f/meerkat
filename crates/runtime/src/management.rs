use super::*;
use lankide_protocol::{
    Agent as ProtocolAgent, AgentAttention, AgentOnboardingState, AgentRef, AttentionMode,
    CatalogRequest, ChannelAttention, ComputerCatalog, ComputerCatalogItem, CreateAgentRequest,
    CreateWorkspaceRequest, DeleteAgentRequest, DeleteAgentResponse, DesiredState,
    EnvironmentVariable, GetAgentAvatarRequest, GetAgentAvatarResponse, GetAgentOnboardingRequest,
    GetAgentRequest, GetProviderOnboardingRequest, GetWorkspaceRequest, HarnessCatalog,
    HarnessCatalogItem, ListAgentsRequest, ListAgentsResponse, ListWorkspacesRequest,
    ListWorkspacesResponse, ManagementEventKind, ObservedState, OnboardingStatus, ProviderCatalog,
    ProviderCatalogItem, ProviderOnboardingState, ReactionCatalog, SetWorkspaceStateRequest,
    UpdateAgentRequest, UpdateWorkspaceRequest, Workspace as ProtocolWorkspace, WorkspaceRef,
};
use lankide_provider::{
    AgentSpec as ProviderAgentSpec, ChannelDefaultAttention,
    EnvironmentVariable as ProviderEnvironmentVariable, SecretValue as ProviderSecret,
    WorkspaceSpec as ProviderWorkspaceSpec,
};
use lankide_storage::{AgentRecord, AttentionRecord, WorkspaceRecord, WorkspaceSetupStatus};
use std::collections::BTreeSet;

mod lifecycle;
use lifecycle::{
    AgentLifecycle, CreateWorkspace, UpdateWorkspace, WorkspaceDesiredState, WorkspaceLifecycle,
};

impl RuntimeApplication {
    pub async fn cancel_attempt(
        &self,
        request: lankide_protocol::CancelAttemptRequest,
    ) -> Result<lankide_protocol::CancelAttemptResponse, RuntimeError> {
        // Validate ownership/existence through storage before forwarding control.
        self.runtime()?.storage().execution(request.execution_id)?;
        self.computer_broker()
            .request_cancellation(request.execution_id)
            .await?;
        Ok(lankide_protocol::CancelAttemptResponse {})
    }

    pub async fn get_execution_details(
        &self,
        request: lankide_protocol::GetExecutionDetailsRequest,
    ) -> Result<lankide_protocol::ExecutionDetails, RuntimeError> {
        let runtime = self.runtime()?;
        let execution = runtime.storage().execution(request.execution_id)?;
        let attempts = runtime
            .storage()
            .execution_attempts(request.execution_id)?
            .into_iter()
            .map(|attempt| {
                Ok(lankide_protocol::ExecutionAttemptDetails {
                    attempt: attempt.attempt as u32,
                    computer_id: attempt.computer_id,
                    state: attempt.state.as_str().into(),
                    failure: attempt
                        .failure
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|error| RuntimeError::Application(error.to_string()))?,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        Ok(lankide_protocol::ExecutionDetails {
            execution_id: execution.execution_id,
            state: execution.state.as_str().into(),
            attempts,
        })
    }

    pub async fn list_workspaces(
        &self,
        request: ListWorkspacesRequest,
    ) -> Result<ListWorkspacesResponse, RuntimeError> {
        let runtime = self.runtime()?;
        let workspaces = runtime
            .storage()
            .list_workspace_records()?
            .into_iter()
            .map(|record| workspace_dto(runtime.storage(), record))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListWorkspacesResponse {
            request_id: request.request_id,
            workspaces,
        })
    }

    pub async fn get_workspace(
        &self,
        request: GetWorkspaceRequest,
    ) -> Result<ProtocolWorkspace, RuntimeError> {
        let id = workspace_id(request.workspace)?;
        let runtime = self.runtime()?;
        workspace_dto(runtime.storage(), runtime.storage().workspace_record(&id)?)
    }

    pub async fn create_workspace(
        &self,
        request: CreateWorkspaceRequest,
    ) -> Result<ProtocolWorkspace, RuntimeError> {
        let command = CreateWorkspace {
            id: WorkspaceId::new(request.workspace_id)
                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
            name: request.name,
            rules: request.rules,
            provider_id: ProviderId::new(request.provider_id)
                .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
            provider_config: parse_json(&request.provider_config_json)?,
            provider_secrets: parse_secrets(&request.provider_secrets_json)?,
        };
        let record = WorkspaceLifecycle::new(self).create(command).await?;
        workspace_dto(self.runtime()?.storage(), record)
    }

    pub async fn update_workspace(
        &self,
        request: UpdateWorkspaceRequest,
    ) -> Result<ProtocolWorkspace, RuntimeError> {
        let command = UpdateWorkspace {
            id: workspace_id(request.workspace)?,
            name: request.name,
            rules: request.rules,
            provider_config: request
                .provider_config_json
                .as_deref()
                .map(parse_json)
                .transpose()?,
            provider_secrets: request
                .provider_secrets_json
                .as_deref()
                .map(parse_secrets)
                .transpose()?
                .unwrap_or_default(),
            cover_resource_id: request.cover_resource_id,
        };
        let record = WorkspaceLifecycle::new(self).update(command).await?;
        workspace_dto(self.runtime()?.storage(), record)
    }

    pub async fn set_workspace_state(
        &self,
        request: SetWorkspaceStateRequest,
    ) -> Result<ProtocolWorkspace, RuntimeError> {
        let desired = match DesiredState::try_from(request.desired_state)
            .unwrap_or(DesiredState::Unspecified)
        {
            DesiredState::Running => WorkspaceDesiredState::Running,
            DesiredState::Paused => WorkspaceDesiredState::Paused,
            DesiredState::Unspecified => {
                return Err(RuntimeError::Application("invalid desired state".into()));
            }
        };
        let record = WorkspaceLifecycle::new(self)
            .set_state(workspace_id(request.workspace)?, desired)
            .await?;
        workspace_dto(self.runtime()?.storage(), record)
    }

    pub async fn list_agents(
        &self,
        request: ListAgentsRequest,
    ) -> Result<ListAgentsResponse, RuntimeError> {
        let workspace = workspace_id(request.workspace)?;
        let runtime = self.runtime()?;
        let agents = runtime
            .storage()
            .list_agent_records(&workspace)?
            .into_iter()
            .map(|record| agent_dto(record, runtime.storage()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ListAgentsResponse {
            request_id: request.request_id,
            agents,
        })
    }

    pub async fn get_agent(&self, request: GetAgentRequest) -> Result<ProtocolAgent, RuntimeError> {
        let (workspace, agent) = agent_id(request.agent)?;
        let runtime = self.runtime()?;
        agent_dto(
            runtime.storage().agent_record(&workspace, &agent)?,
            runtime.storage(),
        )
    }

    pub async fn get_agent_avatar(
        &self,
        request: GetAgentAvatarRequest,
    ) -> Result<GetAgentAvatarResponse, RuntimeError> {
        let (workspace_id, agent_id) = agent_id(request.agent)?;
        let runtime = self.runtime()?;
        let record = runtime.storage().agent_record(&workspace_id, &agent_id)?;
        let avatar = self
            .agent_profiles()
            .read_avatar(&workspace_id, &agent_id, record.avatar.as_deref())?
            .ok_or_else(|| RuntimeError::Application("Agent avatar is not set".into()))?;
        Ok(GetAgentAvatarResponse {
            request_id: request.request_id,
            agent: Some(AgentRef {
                workspace_id: workspace_id.to_string(),
                agent_id: agent_id.to_string(),
            }),
            avatar: Some(avatar_file(avatar)),
        })
    }

    fn validate_placement(&self, agent: &AgentRecord) -> Result<(), RuntimeError> {
        let computer = self
            .runtime()?
            .storage()
            .registered_computers()?
            .into_iter()
            .find(|c| c.computer_id == agent.computer_id)
            .ok_or(RuntimeError::ComputerNotRegistered)?;
        if computer.revoked_at.is_some() {
            return Err(RuntimeError::ComputerRevoked);
        }
        let inventory: Vec<lankide_protocol::link::HarnessInfo> =
            serde_json::from_str(&computer.inventory_json)
                .map_err(|e| RuntimeError::Application(e.to_string()))?;
        if !inventory.iter().any(|h| h.harness_id == agent.harness_id) {
            return Err(RuntimeError::HarnessUnavailableOnComputer);
        }
        Ok(())
    }

    pub async fn create_agent(
        &self,
        request: CreateAgentRequest,
    ) -> Result<ProtocolAgent, RuntimeError> {
        let (workspace_id, agent_id) = agent_id(request.agent)?;
        let now = now_millis();
        let runtime = self.runtime()?;
        let workspace = runtime.storage().workspace_record(&workspace_id)?;
        let provider_agent_secrets = parse_secrets(&request.provider_agent_secrets_json)?;
        let channels = runtime.channels(&workspace_id).await?;
        let attention = attention_record(&channels, request.attention)?;
        let candidate = AgentRecord {
            workspace_id: workspace_id.clone(),
            agent_id: agent_id.clone(),
            name: request.name,
            role: request.role,
            bio: request.bio,
            avatar: None,
            soul: request.soul,
            computer_id: request.computer_id,
            placement_generation: 1,
            harness_id: request.harness_id,
            model: request.model,
            thinking: request.thinking_level,
            auto_reaction: nonempty(request.auto_reaction),
            cli_path: nonempty(request.cli_path),
            env: request
                .environment_variables
                .into_iter()
                .map(|value| (value.name, value.value))
                .collect(),
            provider_agent_config: parse_json(&request.provider_agent_config_json)?,
            provider_state: serde_json::json!({}),
            session_config: serde_json::Value::Object(Default::default()),
            attention,
            created_at: now,
            updated_at: now,
        };
        self.validate_placement(&candidate)?;
        let record = AgentLifecycle::new(self)
            .create(workspace, candidate, provider_agent_secrets)
            .await?;
        agent_dto(record, runtime.storage())
    }

    pub async fn resume_agent(
        &self,
        request: GetAgentRequest,
    ) -> Result<ProtocolAgent, RuntimeError> {
        let (w, a) = agent_id(request.agent.clone())?;
        let runtime = self.runtime()?;
        runtime.storage().resume_agent(&w, &a, now_millis())?;
        self.emit_management(
            ManagementEventKind::AgentUpdated,
            &w,
            Some(&a),
            now_millis(),
        );
        self.get_agent(request).await
    }

    pub async fn update_agent(
        &self,
        request: UpdateAgentRequest,
    ) -> Result<ProtocolAgent, RuntimeError> {
        let (workspace_id, agent_id) = agent_id(request.agent)?;
        let runtime = self.runtime()?;
        let workspace = runtime.storage().workspace_record(&workspace_id)?;
        let old_record = runtime.storage().agent_record(&workspace_id, &agent_id)?;
        let mut record = old_record.clone();
        if let Some(value) = request.name {
            record.name = value;
        }
        if let Some(value) = request.role {
            record.role = value;
        }
        if let Some(value) = request.bio {
            record.bio = value;
        }
        if let Some(value) = request.harness_id {
            record.harness_id = value;
        }
        if let Some(value) = request.model {
            record.model = value;
        }
        if let Some(value) = request.thinking_level {
            record.thinking = value;
        }
        if let Some(value) = request.auto_reaction {
            record.auto_reaction = nonempty(value);
        }
        if let Some(value) = request.soul {
            record.soul = value;
        }
        if let Some(value) = request.cli_path {
            record.cli_path = nonempty(value);
        }
        if let Some(value) = request.computer_id {
            record.computer_id = value;
        }
        if let Some(value) = request.provider_agent_config_json {
            record.provider_agent_config = parse_json(&value)?;
        }
        self.validate_placement(&record)?;
        let avatar_update = request
            .avatar
            .as_ref()
            .map(|upload| {
                self.agent_profiles().replace_avatar(
                    &workspace_id,
                    &agent_id,
                    old_record.avatar.as_deref(),
                    upload,
                )
            })
            .transpose()?;
        if let Some(avatar) = avatar_update.as_ref() {
            record.avatar = Some(avatar.avatar.path.clone());
        }
        if let Some(environment) = request.environment_variables {
            record.env = environment
                .entries
                .into_iter()
                .map(|v| (v.name, v.value))
                .collect();
        }
        if let Some(attention) = request.attention {
            let channels = runtime.channels(&workspace_id).await?;
            record.attention = attention_record(&channels, Some(attention))?;
        }
        record.updated_at = now_millis();
        let provider_agent_secrets = request
            .provider_agent_secrets_json
            .as_deref()
            .map(parse_secrets)
            .transpose()?
            .unwrap_or_default();
        let resume = old_record.harness_id != record.harness_id
            || old_record.model != record.model
            || old_record.computer_id != record.computer_id
            || old_record.thinking != record.thinking
            || old_record.cli_path != record.cli_path
            || old_record.env != record.env
            || old_record.session_config != record.session_config;
        let result = AgentLifecycle::new(self)
            .update(
                workspace,
                old_record,
                record,
                provider_agent_secrets,
                avatar_update
                    .as_ref()
                    .filter(|avatar| avatar.changed)
                    .map(|avatar| &avatar.avatar),
            )
            .await;
        if result.is_ok() && resume {
            runtime
                .storage()
                .resume_agent(&workspace_id, &agent_id, now_millis())?;
            self.emit_management(
                ManagementEventKind::AgentUpdated,
                &workspace_id,
                Some(&agent_id),
                now_millis(),
            );
        }
        match (result, avatar_update) {
            (Ok(record), Some(avatar)) => {
                avatar.finish();
                agent_dto(record, runtime.storage())
            }
            (Ok(record), None) => agent_dto(record, runtime.storage()),
            (Err(error), Some(avatar)) => {
                avatar.rollback()?;
                Err(error)
            }
            (Err(error), None) => Err(error),
        }
    }

    pub async fn delete_agent(
        &self,
        request: DeleteAgentRequest,
    ) -> Result<DeleteAgentResponse, RuntimeError> {
        let (workspace_id, agent_id) = agent_id(request.agent)?;
        AgentLifecycle::new(self)
            .delete(workspace_id.clone(), agent_id.clone())
            .await?;
        Ok(DeleteAgentResponse {
            request_id: request.request_id,
            agent: Some(AgentRef {
                workspace_id: workspace_id.to_string(),
                agent_id: agent_id.to_string(),
            }),
        })
    }

    pub async fn get_provider_onboarding(
        &self,
        request: GetProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingState, RuntimeError> {
        let workspace_id = workspace_id(request.workspace)?;
        self.provider_onboarding(workspace_id, "get".into(), serde_json::Value::Null)
            .await
    }

    pub async fn advance_provider_onboarding(
        &self,
        request: lankide_protocol::AdvanceProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingState, RuntimeError> {
        let workspace_id = workspace_id(request.workspace)?;
        self.provider_onboarding(
            workspace_id,
            request.action,
            parse_json(&request.field_values_json)?,
        )
        .await
    }

    async fn provider_onboarding(
        &self,
        workspace_id: WorkspaceId,
        action: String,
        payload: serde_json::Value,
    ) -> Result<ProviderOnboardingState, RuntimeError> {
        let runtime = self.runtime()?;
        let workspace = runtime.storage().workspace_record(&workspace_id)?;
        let provider = self.factories().factory(&workspace.provider_id)?;
        let state = provider
            .workspace_onboarding(
                action,
                payload,
                workspace_services(runtime.storage_arc(), self.resource_store()?, &workspace),
            )
            .await?;
        Ok(onboarding_dto(&workspace, state))
    }

    pub async fn get_agent_onboarding(
        &self,
        request: GetAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, RuntimeError> {
        let (workspace, agent) = agent_id(request.agent)?;
        self.agent_onboarding(workspace, agent, "get".into(), serde_json::Value::Null)
            .await
    }

    pub async fn advance_agent_onboarding(
        &self,
        request: lankide_protocol::AdvanceAgentOnboardingRequest,
    ) -> Result<AgentOnboardingState, RuntimeError> {
        let (workspace, agent) = agent_id(request.agent)?;
        self.agent_onboarding(
            workspace,
            agent,
            request.action,
            parse_json(&request.field_values_json)?,
        )
        .await
    }

    async fn agent_onboarding(
        &self,
        workspace_id: WorkspaceId,
        agent_id: AgentId,
        action: String,
        payload: serde_json::Value,
    ) -> Result<AgentOnboardingState, RuntimeError> {
        let runtime = self.runtime()?;
        let workspace = runtime.storage().workspace_record(&workspace_id)?;
        let mut agent = runtime.storage().agent_record(&workspace_id, &agent_id)?;
        let host = runtime.host(&workspace_id).await?;
        let state = host
            .agent_onboarding(
                agent.agent_id.clone(),
                agent.name.clone(),
                agent.provider_agent_config.clone(),
                action,
                payload,
            )
            .await?;
        if state.status == crate::ProviderOnboardingStatus::Ready {
            agent.provider_state = runtime
                .storage()
                .agent_provider_state(&workspace_id, &agent_id)?;
            if let (Some(target), Some(updates)) = (
                agent.provider_agent_config.as_object_mut(),
                state.agent_config_updates.as_object(),
            ) {
                for (key, value) in updates {
                    target.insert(key.clone(), value.clone());
                }
            }
            agent.updated_at = now_millis();
            let agent = runtime
                .storage()
                .update_agent_record_with_secrets(agent, &serde_json::json!({}))?;
            if let Some(person_id) = agent
                .provider_agent_config
                .get("creator_person_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| lankide_core::PersonId::new(value.to_owned()).ok())
            {
                match runtime.storage().initialize_team_lead(
                    &workspace_id,
                    &person_id,
                    now_millis(),
                ) {
                    Ok(_) | Err(StorageError::Conflict(_)) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            let host = runtime.host(&workspace_id).await?;
            host.create_agent(agent_id.clone(), provider_agent_spec(&agent))
                .await?;
            runtime
                .configure_agent(&workspace_id, agent_runtime_config(&workspace, agent)?)
                .await?;
        }
        Ok(agent_onboarding_dto(&workspace, &agent_id, state))
    }

    pub fn provider_catalog(&self, _: CatalogRequest) -> Result<ProviderCatalog, RuntimeError> {
        let factories = self.factories();
        let providers = factories
            .catalog()?
            .into_iter()
            .map(|info| {
                let capabilities = factories.factory(&info.id)?.capabilities();
                Ok(ProviderCatalogItem {
                    provider_id: info.id.to_string(),
                    display_name: info.name,
                    description: info.description,
                    available: true,
                    capabilities,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        Ok(ProviderCatalog { providers })
    }

    pub fn computer_catalog(&self, _: CatalogRequest) -> Result<ComputerCatalog, RuntimeError> {
        let broker = self.computer_broker();
        let online = broker.catalog()?;
        let mut computers: Vec<_> = self
            .runtime()?
            .storage()
            .registered_computers()?
            .into_iter()
            .map(|record| {
                let live = online
                    .iter()
                    .find(|value| value.computer_id == record.computer_id);
                let inventory: Vec<lankide_protocol::link::HarnessInfo> =
                    serde_json::from_str(&record.inventory_json)
                        .map_err(|e| RuntimeError::Application(e.to_string()))?;
                Ok(ComputerCatalogItem {
                    computer_id: record.computer_id,
                    display_name: record.display_name,
                    observed_state: if record.revoked_at.is_some() {
                        ObservedState::Stopped
                    } else if live.is_some() {
                        ObservedState::Running
                    } else {
                        ObservedState::Stopped
                    } as i32,
                    harnesses: inventory.into_iter().map(Into::into).collect(),
                    status_message: String::new(),
                    registered: true,
                    revoked_at: record.revoked_at,
                    last_seen_at: record.last_seen_at,
                    paired_at: record.paired_at,
                    os: record.os,
                    version: record.version,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        for (computer_id, reason) in broker.rejected_links()? {
            if let Some(computer) = computers
                .iter_mut()
                .find(|value| value.computer_id == computer_id)
            {
                if computer.revoked_at.is_none() {
                    computer.observed_state = ObservedState::Error as i32;
                    computer.status_message = reason.to_string();
                }
                continue;
            }
            computers.push(ComputerCatalogItem {
                display_name: computer_id.clone(),
                computer_id,
                observed_state: ObservedState::Error as i32,
                harnesses: Vec::new(),
                status_message: reason.to_string(),
                ..Default::default()
            });
        }
        Ok(ComputerCatalog { computers })
    }

    pub fn harness_catalog(&self, _: CatalogRequest) -> Result<HarnessCatalog, RuntimeError> {
        let harnesses = self
            .computer_broker()
            .catalog()?
            .into_iter()
            .flat_map(|computer| computer.harnesses)
            .map(|harness| HarnessCatalogItem {
                harness_id: harness.harness_id,
                display_name: harness.display_name,
                description: harness.reason,
                availability: harness.availability,
                supported_thinking_levels: Vec::new(),
            })
            .collect();
        Ok(HarnessCatalog { harnesses })
    }

    pub async fn reaction_catalog(
        &self,
        request: CatalogRequest,
    ) -> Result<ReactionCatalog, RuntimeError> {
        let workspace = workspace_id(request.workspace)?;
        let runtime = self.runtime()?;
        let (host, _) = runtime.lifecycle_host(&workspace).await?;
        Ok(ReactionCatalog {
            reactions: host
                .reaction_catalog()
                .into_iter()
                .map(|reaction| lankide_protocol::ReactionCatalogItem {
                    reaction_id: reaction.id,
                    display_name: reaction.name,
                    emoji: reaction.emoji,
                    available: true,
                })
                .collect(),
        })
    }
}

fn workspace_dto(
    storage: &Storage,
    record: WorkspaceRecord,
) -> Result<ProtocolWorkspace, RuntimeError> {
    let cover = record
        .cover_resource_id
        .map(|id| resource_ref(storage, &record.id, id))
        .transpose()?;
    Ok(ProtocolWorkspace {
        workspace: Some(WorkspaceRef {
            workspace_id: record.id.to_string(),
        }),
        name: record.name,
        cover,
        rules: record.rules,
        provider_id: record.provider_id.to_string(),
        provider_config_json: record.provider_config.to_string(),
        desired_state: if record.active {
            DesiredState::Running as i32
        } else {
            DesiredState::Paused as i32
        },
        observed_state: if record.setup_status == WorkspaceSetupStatus::Error {
            ObservedState::Error as i32
        } else if record.setup_status == WorkspaceSetupStatus::Pending {
            ObservedState::Starting as i32
        } else if record.active {
            ObservedState::Running as i32
        } else {
            ObservedState::Paused as i32
        },
        status_message: if record.setup_status == WorkspaceSetupStatus::Error {
            "Setup needs attention.".into()
        } else {
            String::new()
        },
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn agent_onboarding_dto(
    workspace: &WorkspaceRecord,
    agent_id: &AgentId,
    state: ProviderOnboarding,
) -> AgentOnboardingState {
    AgentOnboardingState {
        agent: Some(AgentRef {
            workspace_id: workspace.id.to_string(),
            agent_id: agent_id.to_string(),
        }),
        provider_id: workspace.provider_id.to_string(),
        step: state.step,
        status: onboarding_status(state.status) as i32,
        payload_json: state.opaque_payload.to_string(),
        verification_uri: state.verification_uri.unwrap_or_default(),
        poll_interval_seconds: state.poll_interval_seconds.unwrap_or_default(),
        error_code: state.error_code.unwrap_or_default(),
        error_message: state.error_message.unwrap_or_default(),
    }
}

fn default_attention(
    channels: &[lankide_provider::ChannelDefinition],
) -> BTreeMap<ChannelId, AttentionRecord> {
    channels
        .iter()
        .map(|channel| {
            let policy = match channel.default_attention {
                ChannelDefaultAttention::DirectMessage | ChannelDefaultAttention::Follow => {
                    AttentionRecord::Always
                }
                ChannelDefaultAttention::Mention => AttentionRecord::Mention,
                ChannelDefaultAttention::Mute => AttentionRecord::Mute,
            };
            (channel.id.clone(), policy)
        })
        .collect()
}

fn attention_record(
    channels: &[lankide_provider::ChannelDefinition],
    attention: Option<AgentAttention>,
) -> Result<BTreeMap<ChannelId, AttentionRecord>, RuntimeError> {
    let mut result = default_attention(channels);
    let mut seen = BTreeSet::new();
    for value in attention.unwrap_or_default().channels {
        let channel = ChannelId::new(value.channel_id)
            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
        if !seen.insert(channel.clone()) {
            return Err(RuntimeError::Application(
                "Choose each channel only once.".into(),
            ));
        }
        let definition = channels
            .iter()
            .find(|definition| definition.id == channel)
            .ok_or_else(|| {
                RuntimeError::Application("This attention setting is unavailable.".into())
            })?;
        let mode = AttentionMode::try_from(value.mode).unwrap_or(AttentionMode::Unspecified);
        let record = match mode {
            AttentionMode::Mention | AttentionMode::Always | AttentionMode::Mute
                if value.interval_minutes != 0 =>
            {
                return Err(RuntimeError::Application(
                    "Choose a supported schedule interval.".into(),
                ));
            }
            AttentionMode::Mention => AttentionRecord::Mention,
            AttentionMode::Always => AttentionRecord::Always,
            AttentionMode::Mute => AttentionRecord::Mute,
            AttentionMode::Schedule => AttentionRecord::Schedule {
                interval_minutes: value.interval_minutes,
            },
            AttentionMode::Unspecified => {
                return Err(RuntimeError::Application(
                    "Choose how the agent should respond in each channel.".into(),
                ));
            }
        };
        record.validate()?;
        let configurable = match record {
            AttentionRecord::Mention => definition
                .attention_levels
                .contains(&AttentionLevel::Mention),
            AttentionRecord::Always => definition
                .attention_levels
                .contains(&AttentionLevel::Follow),
            AttentionRecord::Schedule { .. } => definition
                .attention_levels
                .contains(&AttentionLevel::Schedule),
            AttentionRecord::Mute => !definition.attention_levels.is_empty(),
        };
        if !configurable {
            return Err(RuntimeError::Application(
                "This attention setting cannot be changed.".into(),
            ));
        }
        result.insert(channel, record);
    }
    Ok(result)
}

fn workspace_image_resource(
    storage: &Storage,
    workspace: &WorkspaceId,
    value: i64,
) -> Result<ResourceId, RuntimeError> {
    let id =
        ResourceId::new(value).map_err(|error| RuntimeError::InvalidCore(error.to_string()))?;
    let metadata = storage.resource_metadata(workspace, id)?.ok_or_else(|| {
        RuntimeError::Application("Profile image not found in this workspace".into())
    })?;
    if metadata.size_bytes > 3 * 1024 * 1024
        || !matches!(
            metadata.media_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        )
    {
        return Err(RuntimeError::Application(
            "Choose a PNG, JPEG, WebP or GIF image up to 3 MiB".into(),
        ));
    }
    Ok(id)
}

fn resource_ref(
    storage: &Storage,
    workspace_id: &WorkspaceId,
    resource_id: ResourceId,
) -> Result<lankide_protocol::ResourceRef, RuntimeError> {
    let value = storage
        .resource_metadata(workspace_id, resource_id)?
        .ok_or_else(|| RuntimeError::Application("resource metadata not found".into()))?;
    Ok(lankide_protocol::ResourceRef {
        resource_id: value.resource_id.get(),
        file_name: value.file_name,
        media_type: value.media_type,
        size_bytes: value.size_bytes,
        sha256: value.sha256,
    })
}

fn agent_dto(record: AgentRecord, storage: &Storage) -> Result<ProtocolAgent, RuntimeError> {
    let pause_reason = storage.agent_pause_reason(&record.workspace_id, &record.agent_id)?;
    Ok(ProtocolAgent {
        pause_reason,
        agent: Some(AgentRef {
            workspace_id: record.workspace_id.to_string(),
            agent_id: record.agent_id.to_string(),
        }),
        name: record.name,
        role: record.role,
        bio: record.bio,
        avatar_path: record.avatar.unwrap_or_default(),
        harness_id: record.harness_id,
        model: record.model,
        thinking_level: record.thinking,
        auto_reaction: record.auto_reaction.unwrap_or_default(),
        soul: record.soul,
        cli_path: record.cli_path.unwrap_or_default(),
        environment_variables: record
            .env
            .into_iter()
            .map(|(name, value)| EnvironmentVariable { name, value })
            .collect(),
        computer_id: record.computer_id,
        created_at: record.created_at,
        updated_at: record.updated_at,
        attention: Some(AgentAttention {
            channels: record
                .attention
                .into_iter()
                .map(|(channel_id, value)| {
                    let (mode, interval_minutes) = match value {
                        AttentionRecord::Mention => (AttentionMode::Mention, 0),
                        AttentionRecord::Always => (AttentionMode::Always, 0),
                        AttentionRecord::Mute => (AttentionMode::Mute, 0),
                        AttentionRecord::Schedule { interval_minutes } => {
                            (AttentionMode::Schedule, interval_minutes)
                        }
                    };
                    ChannelAttention {
                        channel_id: channel_id.to_string(),
                        mode: mode as i32,
                        interval_minutes,
                    }
                })
                .collect(),
        }),
        provider_agent_config_json: record.provider_agent_config.to_string(),
    })
}

fn parse_secrets(value: &str) -> Result<BTreeMap<String, ProviderSecret>, RuntimeError> {
    let value = parse_json(value)?;
    let object = value.as_object().ok_or_else(|| {
        RuntimeError::Application("Provider credentials must be a JSON object.".into())
    })?;
    object
        .iter()
        .map(|(name, value)| {
            let value = value.as_str().ok_or_else(|| {
                RuntimeError::Application("Provider credential values must be strings.".into())
            })?;
            if name.trim().is_empty() {
                return Err(RuntimeError::Application(
                    "Provider credential names cannot be empty.".into(),
                ));
            }
            Ok((name.clone(), ProviderSecret::new(value)))
        })
        .collect()
}

fn secrets_json(secrets: &BTreeMap<String, ProviderSecret>) -> serde_json::Value {
    serde_json::Value::Object(
        secrets
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    serde_json::Value::String(value.expose_secret().to_owned()),
                )
            })
            .collect(),
    )
}

fn provider_workspace_spec(record: &WorkspaceRecord) -> ProviderWorkspaceSpec {
    ProviderWorkspaceSpec {
        name: record.name.clone(),
        cover: record.cover_resource_id.map(|id| id.get().to_string()),
        rules: record.rules.clone(),
        provider_config: record.provider_config.clone(),
    }
}

pub(crate) fn provider_agent_spec(record: &AgentRecord) -> ProviderAgentSpec {
    ProviderAgentSpec {
        name: record.name.clone(),
        role: record.role.clone(),
        bio: record.bio.clone(),
        avatar: record.avatar.clone(),
        soul: record.soul.clone(),
        computer_id: record.computer_id.clone(),
        harness_id: record.harness_id.clone(),
        model: record.model.clone(),
        thinking_level: record.thinking.clone(),
        auto_reaction: record.auto_reaction.clone(),
        cli_path: record.cli_path.clone(),
        environment_variables: record
            .env
            .iter()
            .map(|(name, value)| ProviderEnvironmentVariable {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
        provider_agent_config: record.provider_agent_config.clone(),
    }
}

fn onboarding_dto(
    workspace: &WorkspaceRecord,
    state: ProviderOnboarding,
) -> ProviderOnboardingState {
    ProviderOnboardingState {
        workspace: Some(WorkspaceRef {
            workspace_id: workspace.id.to_string(),
        }),
        provider_id: workspace.provider_id.to_string(),
        step: state.step,
        status: onboarding_status(state.status) as i32,
        payload_json: state.opaque_payload.to_string(),
        verification_uri: state.verification_uri.unwrap_or_default(),
        poll_interval_seconds: state.poll_interval_seconds.unwrap_or_default(),
        error_code: state.error_code.unwrap_or_default(),
        error_message: state.error_message.unwrap_or_default(),
    }
}

fn onboarding_status(status: ProviderOnboardingStatus) -> OnboardingStatus {
    match status {
        ProviderOnboardingStatus::ActionRequired => OnboardingStatus::ActionRequired,
        ProviderOnboardingStatus::Verifying => OnboardingStatus::Verifying,
        ProviderOnboardingStatus::Ready => OnboardingStatus::Ready,
        ProviderOnboardingStatus::Error => OnboardingStatus::Error,
    }
}

fn workspace_id(value: Option<WorkspaceRef>) -> Result<WorkspaceId, RuntimeError> {
    WorkspaceId::new(
        value
            .ok_or_else(|| RuntimeError::Application("workspace ref required".into()))?
            .workspace_id,
    )
    .map_err(|error| RuntimeError::InvalidCore(error.to_string()))
}
fn agent_id(value: Option<AgentRef>) -> Result<(WorkspaceId, AgentId), RuntimeError> {
    let value = value.ok_or_else(|| RuntimeError::Application("agent ref required".into()))?;
    Ok((
        WorkspaceId::new(value.workspace_id)
            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
        AgentId::new(value.agent_id)
            .map_err(|error| RuntimeError::InvalidCore(error.to_string()))?,
    ))
}
fn parse_json(value: &str) -> Result<serde_json::Value, RuntimeError> {
    if value.trim().is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    serde_json::from_str(value)
        .map_err(|error| RuntimeError::Application(format!("invalid json: {error}")))
}
fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
