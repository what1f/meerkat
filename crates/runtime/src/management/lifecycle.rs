use super::*;
use std::collections::BTreeMap;

pub(super) struct CreateWorkspace {
    pub id: WorkspaceId,
    pub name: String,
    pub rules: String,
    pub provider_id: ProviderId,
    pub provider_config: serde_json::Value,
    pub provider_secrets: BTreeMap<String, ProviderSecret>,
}

pub(super) struct UpdateWorkspace {
    pub id: WorkspaceId,
    pub name: Option<String>,
    pub rules: Option<String>,
    pub provider_config: Option<serde_json::Value>,
    pub provider_secrets: BTreeMap<String, ProviderSecret>,
    pub cover_resource_id: Option<i64>,
}

pub(super) enum WorkspaceDesiredState {
    Running,
    Paused,
}

pub(super) struct WorkspaceLifecycle<'a> {
    application: &'a RuntimeApplication,
}

impl<'a> WorkspaceLifecycle<'a> {
    pub fn new(application: &'a RuntimeApplication) -> Self {
        Self { application }
    }

    pub async fn create(&self, command: CreateWorkspace) -> Result<WorkspaceRecord, RuntimeError> {
        let CreateWorkspace {
            id,
            name,
            rules,
            provider_id,
            provider_config,
            provider_secrets,
        } = command;
        self.application.factories().factory(&provider_id)?;
        let now = now_millis();
        let runtime = self.application.runtime()?;
        if runtime
            .database()
            .list_workspace_records()
            .await?
            .iter()
            .any(|record| record.id == id)
        {
            return Err(RuntimeError::DuplicateWorkspace(id));
        }
        let candidate = WorkspaceRecord {
            id: id.clone(),
            name,
            provider_id,
            provider_config,
            provider_state: serde_json::json!({}),
            rules,
            cover_resource_id: None,
            setup_status: WorkspaceSetupStatus::Pending,
            setup_error_code: None,
            secret_ref: None,
            active: true,
            created_at: now,
            updated_at: now,
        };
        let record = runtime
            .database()
            .insert_new_workspace_record(candidate, &secrets_json(&provider_secrets))
            .await?;
        let factory = self.application.factories().factory(&record.provider_id)?;
        let services = workspace_services_with_secrets(
            runtime.database(),
            self.application.resource_store()?,
            &record,
            provider_secrets,
        );
        let provider = factory.create(&record, services)?;
        if let Err(error) = provider
            .create_workspace(provider_workspace_spec(&record))
            .await
        {
            if let Err(rollback_error) = runtime.database().rollback_new_workspace(&id).await {
                log::error!("[runtime] workspace={id} setup rollback failed: {rollback_error}");
                return Err(rollback_error.into());
            }
            return Err(error.into());
        }
        let record = runtime.database().workspace_record(&id).await?;
        self.application
            .attach_workspace(&runtime, record.clone())
            .await?;
        if self.application.readiness().providers_started {
            if runtime.resume_workspace(&id, now).await.is_err() {
                return self.fail_setup(&runtime, &id, now).await;
            }
            self.application.start_workspace_loop(id.clone())?;
        }
        runtime
            .database()
            .set_workspace_setup(&id, WorkspaceSetupStatus::Ready, None, now)
            .await?;
        self.application
            .emit_management(ManagementEventKind::WorkspaceCreated, &id, None, now);
        runtime
            .database()
            .workspace_record(&id)
            .await
            .map_err(Into::into)
    }

    pub async fn update(&self, command: UpdateWorkspace) -> Result<WorkspaceRecord, RuntimeError> {
        let runtime = self.application.runtime()?;
        let old_record = runtime.database().workspace_record(&command.id).await?;
        let mut record = old_record.clone();
        if let Some(name) = command.name {
            record.name = name;
        }
        if let Some(config) = command.provider_config {
            record.provider_config = config;
        }
        if let Some(rules) = command.rules {
            record.rules = rules;
        }
        if let Some(cover) = command.cover_resource_id {
            record.cover_resource_id =
                Some(workspace_image_resource(runtime.database(), &command.id, cover).await?);
        }
        record.updated_at = now_millis();
        if record.provider_config == old_record.provider_config
            && command.provider_secrets.is_empty()
        {
            let record = runtime
                .database()
                .update_workspace_record_with_secrets(
                    record,
                    &secrets_json(&command.provider_secrets),
                )
                .await?;
            self.application.emit_management(
                ManagementEventKind::WorkspaceUpdated,
                &command.id,
                None,
                record.updated_at,
            );
            return Ok(record);
        }
        let was_active = record.active;
        let factory = self.application.factories().factory(&record.provider_id)?;
        let provider = factory.create(
            &record,
            workspace_services_with_secrets(
                runtime.database(),
                self.application.resource_store()?,
                &record,
                command.provider_secrets.clone(),
            ),
        )?;
        provider
            .update_workspace(old_record.provider_config, provider_workspace_spec(&record))
            .await?;
        record.provider_state = runtime
            .database()
            .workspace_provider_state(&command.id)
            .await?;
        let record = runtime
            .database()
            .update_workspace_record_with_secrets(record, &secrets_json(&command.provider_secrets))
            .await?;
        self.application.stop_workspace_loop(&command.id).await?;
        runtime.remove_provider_instance(&command.id).await?;
        if self
            .application
            .attach_workspace(&runtime, record.clone())
            .await
            .is_err()
        {
            return self
                .fail_setup(&runtime, &command.id, record.updated_at)
                .await;
        }
        if was_active && self.application.readiness().providers_started {
            if runtime
                .resume_workspace(&command.id, record.updated_at)
                .await
                .is_err()
            {
                return self
                    .fail_setup(&runtime, &command.id, record.updated_at)
                    .await;
            }
            self.application.start_workspace_loop(command.id.clone())?;
            runtime
                .database()
                .set_workspace_setup(
                    &command.id,
                    WorkspaceSetupStatus::Ready,
                    None,
                    record.updated_at,
                )
                .await?;
        }
        self.application.emit_management(
            ManagementEventKind::WorkspaceUpdated,
            &command.id,
            None,
            record.updated_at,
        );
        runtime
            .database()
            .workspace_record(&command.id)
            .await
            .map_err(Into::into)
    }

    pub async fn set_state(
        &self,
        id: WorkspaceId,
        desired: WorkspaceDesiredState,
    ) -> Result<WorkspaceRecord, RuntimeError> {
        let now = now_millis();
        let runtime = self.application.runtime()?;
        match desired {
            WorkspaceDesiredState::Running => {
                runtime
                    .database()
                    .set_workspace_active(&id, true, now)
                    .await?;
                if !runtime.has_provider_instance(&id) {
                    let record = runtime.database().workspace_record(&id).await?;
                    if self
                        .application
                        .attach_workspace(&runtime, record)
                        .await
                        .is_err()
                    {
                        return self.fail_setup(&runtime, &id, now).await;
                    }
                }
                if runtime.resume_workspace(&id, now).await.is_err() {
                    return self.fail_setup(&runtime, &id, now).await;
                }
                runtime
                    .database()
                    .set_workspace_setup(&id, WorkspaceSetupStatus::Ready, None, now)
                    .await?;
                if self.application.readiness().providers_started {
                    self.application.start_workspace_loop(id.clone())?;
                }
            }
            WorkspaceDesiredState::Paused => {
                self.application.stop_workspace_loop(&id).await?;
                if runtime.has_provider_instance(&id) {
                    runtime.pause_workspace(&id, now).await?;
                } else {
                    runtime
                        .database()
                        .set_workspace_active(&id, false, now)
                        .await?;
                }
            }
        }
        self.application.emit_management(
            ManagementEventKind::WorkspaceStateChanged,
            &id,
            None,
            now,
        );
        runtime
            .database()
            .workspace_record(&id)
            .await
            .map_err(Into::into)
    }

    async fn fail_setup<T>(
        &self,
        runtime: &Runtime,
        id: &WorkspaceId,
        now: i64,
    ) -> Result<T, RuntimeError> {
        runtime
            .database()
            .set_workspace_setup(
                id,
                WorkspaceSetupStatus::Error,
                Some("provider_setup_failed"),
                now,
            )
            .await?;
        Err(RuntimeError::ActionRequired(
            "Workspace setup needs attention. Check the provider settings and try again.".into(),
        ))
    }
}

pub(super) struct AgentLifecycle<'a> {
    application: &'a RuntimeApplication,
}

impl<'a> AgentLifecycle<'a> {
    pub fn new(application: &'a RuntimeApplication) -> Self {
        Self { application }
    }

    pub async fn create(
        &self,
        workspace: WorkspaceRecord,
        candidate: AgentRecord,
        provider_secrets: BTreeMap<String, ProviderSecret>,
    ) -> Result<AgentRecord, RuntimeError> {
        let runtime = self.application.runtime()?;
        let workspace_id = candidate.workspace_id.clone();
        let agent_id = candidate.agent_id.clone();
        let occurred_at = candidate.updated_at;
        if runtime
            .database()
            .agent_record(&workspace_id, &agent_id)
            .await
            .is_ok()
        {
            return Err(RuntimeError::DuplicateAgent {
                workspace_id,
                agent_id,
            });
        }
        let record = runtime
            .database()
            .insert_new_agent_record(candidate, &secrets_json(&provider_secrets))
            .await?;
        let (provider, temporary) = runtime.lifecycle_host(&workspace_id).await?;
        if let Err(error) = provider
            .create_agent(agent_id.clone(), provider_agent_spec(&record))
            .await
        {
            if let Err(rollback_error) = runtime
                .database()
                .rollback_new_agent(&workspace_id, &agent_id)
                .await
            {
                log::error!(
                    "[runtime] workspace={workspace_id} agent={agent_id} setup rollback failed: {rollback_error}"
                );
                return Err(rollback_error.into());
            }
            if temporary {
                let _ = provider.shutdown().await;
            }
            return Err(error.into());
        }
        if temporary {
            provider.shutdown().await?;
        }
        let record = runtime
            .database()
            .agent_record(&workspace_id, &agent_id)
            .await?;
        runtime
            .configure_agent(
                &workspace_id,
                agent_runtime_config(&workspace, record.clone())?,
            )
            .await?;
        self.application.emit_management(
            ManagementEventKind::AgentCreated,
            &workspace_id,
            Some(&agent_id),
            occurred_at,
        );
        Ok(record)
    }

    pub async fn update(
        &self,
        workspace: WorkspaceRecord,
        old_record: AgentRecord,
        mut record: AgentRecord,
        provider_secrets: BTreeMap<String, ProviderSecret>,
        avatar: Option<&StoredAgentAvatar>,
    ) -> Result<AgentRecord, RuntimeError> {
        if record.name.trim().is_empty() {
            return Err(RuntimeError::Application("Agent name is required".into()));
        }
        let runtime = self.application.runtime()?;
        let workspace_id = record.workspace_id.clone();
        let agent_id = record.agent_id.clone();
        let _ = agent_runtime_config(&workspace, record.clone())?;
        let avatar_changed = avatar.is_some() || old_record.avatar != record.avatar;
        let provider_profile_changed = old_record.name != record.name
            || old_record.bio != record.bio
            || avatar_changed
            || old_record.provider_agent_config != record.provider_agent_config;
        if provider_profile_changed {
            let (provider, temporary) = runtime.lifecycle_host(&workspace_id).await?;
            if temporary {
                provider.init().await?;
            }
            let mut provider_spec = provider_agent_spec(&record);
            // Provider profile updates are field-scoped.  Passing the full
            // Agent profile here makes a change to the bio look like a name
            // change to providers, which is especially harmful for Telegram:
            // setMyName is rate-limited much more aggressively than ordinary
            // profile updates.  Empty values mean "leave this field alone"
            // for the Provider update contract.
            if old_record.name == record.name {
                provider_spec.name.clear();
            }
            if old_record.bio == record.bio {
                provider_spec.bio.clear();
            }
            if !avatar_changed {
                provider_spec.avatar = None;
            }
            if avatar_changed && let Some(avatar) = avatar {
                let config = provider_spec
                    .provider_agent_config
                    .as_object_mut()
                    .ok_or_else(|| RuntimeError::Application("Agent profile is invalid".into()))?;
                config.insert(
                    "avatar_file_name".into(),
                    serde_json::json!(avatar.file_name),
                );
                config.insert("avatar_bytes".into(), serde_json::json!(avatar.bytes));
            }
            let update_result = provider
                .update_agent(
                    agent_id.clone(),
                    old_record.provider_agent_config,
                    provider_spec,
                )
                .await;
            if temporary {
                let _ = provider.shutdown().await;
            }
            update_result?;
            record.provider_state = runtime
                .database()
                .agent_record(&workspace_id, &agent_id)
                .await?
                .provider_state;
        }
        let record = runtime
            .database()
            .update_agent_record_with_secrets(record, &secrets_json(&provider_secrets))
            .await?;
        runtime
            .configure_agent(
                &workspace_id,
                agent_runtime_config(&workspace, record.clone())?,
            )
            .await?;
        self.application.emit_management(
            ManagementEventKind::AgentUpdated,
            &workspace_id,
            Some(&agent_id),
            record.updated_at,
        );
        Ok(record)
    }

    pub async fn delete(
        &self,
        workspace_id: WorkspaceId,
        agent_id: AgentId,
    ) -> Result<(), RuntimeError> {
        let runtime = self.application.runtime()?;
        let record = runtime
            .database()
            .agent_record(&workspace_id, &agent_id)
            .await?;
        self.application
            .stop_agent_execution(&workspace_id, &agent_id)
            .await?;
        let (provider, temporary) = runtime.lifecycle_host(&workspace_id).await?;
        if temporary {
            provider.init().await?;
        }
        let result = provider
            .delete_agent(agent_id.clone(), provider_agent_spec(&record))
            .await;
        if temporary {
            let _ = provider.shutdown().await;
        }
        result?;
        if !runtime
            .database()
            .delete_agent(&workspace_id, &agent_id)
            .await?
        {
            return Err(RuntimeError::UnknownAgent {
                workspace_id,
                agent_id,
            });
        }
        let _ = self.application.agent_profiles().remove_avatar(
            &workspace_id,
            &agent_id,
            record.avatar.as_deref(),
        );
        runtime.remove_agent(&workspace_id, &agent_id).await;
        self.application.emit_management(
            ManagementEventKind::AgentDeleted,
            &workspace_id,
            Some(&agent_id),
            now_millis(),
        );
        if !record.machine_id.is_empty() {
            let broker = self.application.machine_broker();
            let machine_id = record.machine_id;
            let cleanup_workspace_id = workspace_id.clone();
            let cleanup_agent_id = agent_id.clone();
            self.application.spawn_background(async move {
                if let Err(error) = broker
                    .delete_agent_files(&machine_id, &cleanup_workspace_id, &cleanup_agent_id)
                    .await
                {
                    log::error!(
                        "[runtime] workspace={cleanup_workspace_id} agent={cleanup_agent_id} deferred file cleanup failed: {error}"
                    );
                }
            })?;
        }
        Ok(())
    }
}
