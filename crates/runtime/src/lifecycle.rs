use super::*;

impl Runtime {
    pub(crate) async fn close(
        self,
        shutdown_timeout: std::time::Duration,
    ) -> Result<(), RuntimeError> {
        let providers = self
            .providers
            .into_inner()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        drop(providers);
        // Provider shutdown signals background receivers, including native
        // blocking tasks. Their contexts can outlive the host's shutdown call.
        // ProviderHost exposes cancellation, but no join for those receivers.
        // Close SQLite only after those contexts have released HostServices.
        self.storage.close(shutdown_timeout).await?;
        Ok(())
    }

    /// Registers a restartable Provider instance for one Workspace.
    ///
    /// The factory is intentionally retained: a stopped [`ProviderHost`] owns a
    /// closed event sink, so resume must construct a fresh host rather than
    /// pretending that the stopped instance can consume events again.
    pub async fn register_provider_factory<P, F>(
        &self,
        workspace_id: WorkspaceId,
        factory: F,
    ) -> Result<(), RuntimeError>
    where
        P: Provider,
        F: Fn() -> Result<ProviderHost<P>, ProviderError> + Send + Sync + 'static,
    {
        let factory =
            Arc::new(move || factory().map(|host| Arc::new(host) as Arc<dyn ProviderHostInstance>));
        self.register_provider_instance_factory(workspace_id, factory)
            .await
    }

    pub async fn register_provider_instance_factory(
        &self,
        workspace_id: WorkspaceId,
        factory: ProviderFactory,
    ) -> Result<(), RuntimeError> {
        let active = self.storage.workspace(&workspace_id).await?.active;
        let mut providers = self
            .providers
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        if providers.contains_key(&workspace_id) {
            return Err(RuntimeError::DuplicateWorkspace(workspace_id));
        }
        providers.insert(
            workspace_id,
            Arc::new(ProviderInstance {
                active: AtomicBool::new(active),
                factory,
                host: Mutex::new(None),
                channels: RwLock::new(Vec::new()),
                agents: RwLock::new(BTreeMap::new()),
            }),
        );
        Ok(())
    }

    pub fn provider_count(&self) -> usize {
        self.providers
            .read()
            .map(|values| values.len())
            .unwrap_or(0)
    }

    pub fn has_provider_instance(&self, workspace_id: &WorkspaceId) -> bool {
        self.providers
            .read()
            .map(|values| values.contains_key(workspace_id))
            .unwrap_or(false)
    }

    pub async fn remove_provider_instance(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<bool, RuntimeError> {
        let provider = self
            .providers
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .remove(workspace_id);
        let Some(provider) = provider else {
            return Ok(false);
        };
        provider.active.store(false, Ordering::Release);
        if let Some(host) = provider.host.lock().await.take() {
            host.shutdown().await?;
        }
        Ok(true)
    }

    pub fn is_workspace_active(&self, workspace_id: &WorkspaceId) -> Result<bool, RuntimeError> {
        Ok(self.provider(workspace_id)?.active.load(Ordering::Acquire))
    }

    /// Starts every registered Workspace whose configured state is active.
    pub async fn start(&self) -> Result<(), RuntimeError> {
        let providers = self.provider_entries()?;
        for (workspace_id, provider) in providers {
            if provider.active.load(Ordering::Acquire) {
                self.start_instance(&workspace_id, &provider).await?;
            }
        }
        Ok(())
    }

    /// Stops consumption before asking the Provider to shut down.
    pub async fn pause_workspace(
        &self,
        workspace_id: &WorkspaceId,
        now: i64,
    ) -> Result<(), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        provider.active.store(false, Ordering::Release);
        if let Err(error) = self
            .storage
            .set_workspace_active(workspace_id, false, now)
            .await
        {
            provider.active.store(true, Ordering::Release);
            return Err(error.into());
        }
        provider.channels.write().await.clear();
        if !self.storage.has_active_execution(workspace_id).await? {
            if let Some(host) = provider.host.lock().await.take() {
                host.shutdown().await?;
            }
        }
        Ok(())
    }

    /// Creates a new ProviderHost and resumes event consumption for a paused Workspace.
    pub async fn resume_workspace(
        &self,
        workspace_id: &WorkspaceId,
        now: i64,
    ) -> Result<(), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        if provider.active.load(Ordering::Acquire) && provider.host.lock().await.is_some() {
            return Ok(());
        }
        let has_existing_host = provider.host.lock().await.is_some();
        if has_existing_host && self.storage.has_active_execution(workspace_id).await? {
            return Err(RuntimeError::Application(
                "workspace is still completing an active execution".into(),
            ));
        }
        if let Some(host) = provider.host.lock().await.take() {
            host.shutdown().await?;
        }
        if let Err(error) = self.start_instance(workspace_id, &provider).await {
            provider.active.store(false, Ordering::Release);
            return Err(error);
        }
        self.storage
            .set_workspace_active(workspace_id, true, now)
            .await?;
        provider.active.store(true, Ordering::Release);
        Ok(())
    }

    pub async fn finish_paused_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<(), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        if provider.active.load(Ordering::Acquire)
            || self.storage.has_active_execution(workspace_id).await?
        {
            return Ok(());
        }
        if let Some(host) = provider.host.lock().await.take() {
            host.shutdown().await?;
        }
        Ok(())
    }

    pub async fn configure_agent(
        &self,
        workspace_id: &WorkspaceId,
        config: AgentRuntimeConfig,
    ) -> Result<(), RuntimeError> {
        if !self
            .storage
            .list_agents(workspace_id)
            .await?
            .iter()
            .any(|agent| agent.id == config.agent_id)
        {
            return Err(RuntimeError::UnknownAgent {
                workspace_id: workspace_id.clone(),
                agent_id: config.agent_id,
            });
        }
        self.provider(workspace_id)?
            .agents
            .write()
            .await
            .insert(config.agent_id.clone(), config);
        Ok(())
    }

    pub async fn remove_agent(&self, workspace_id: &WorkspaceId, agent_id: &AgentId) {
        if let Ok(provider) = self.provider(workspace_id) {
            provider.agents.write().await.remove(agent_id);
        }
    }

    pub async fn channels(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ChannelDefinition>, RuntimeError> {
        self.require_active(workspace_id)?;
        Ok(self.provider(workspace_id)?.channels.read().await.clone())
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let mut tasks = {
            let mut tasks = self
                .background_tasks
                .lock()
                .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
            std::mem::take(&mut *tasks)
        };
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        let providers = self.provider_entries()?;
        for (workspace_id, provider) in providers {
            provider.active.store(false, Ordering::Release);
            provider.channels.write().await.clear();
            if let Some(host) = provider.host.lock().await.take() {
                host.shutdown()
                    .await
                    .map_err(|source| RuntimeError::Provider {
                        workspace_id: workspace_id.clone(),
                        source,
                    })?;
            }
        }
        Ok(())
    }

    async fn start_instance(
        &self,
        workspace_id: &WorkspaceId,
        provider: &ProviderInstance,
    ) -> Result<(), RuntimeError> {
        let mut current = provider.host.lock().await;
        if current.is_some() {
            return Ok(());
        }
        let host = (provider.factory)().map_err(|source| RuntimeError::Provider {
            workspace_id: workspace_id.clone(),
            source,
        })?;
        let expected_provider_id = self.storage.workspace(workspace_id).await?.provider_id;
        let actual_provider_id = host.provider_id();
        if actual_provider_id != expected_provider_id {
            return Err(RuntimeError::ProviderMismatch {
                workspace_id: workspace_id.clone(),
                expected: expected_provider_id,
                actual: actual_provider_id,
            });
        }
        // Retain the host before the first await. If startup is cancelled,
        // shutdown must still signal any receiver already started by init.
        *current = Some(host.clone());
        let started = async {
            host.init().await.map_err(|source| RuntimeError::Provider {
                workspace_id: workspace_id.clone(),
                source,
            })?;
            for agent in self.storage.list_agent_records(workspace_id).await? {
                host.create_agent(
                    agent.agent_id.clone(),
                    management::provider_agent_spec(&agent),
                )
                .await
                .map_err(|source| RuntimeError::Provider {
                    workspace_id: workspace_id.clone(),
                    source,
                })?;
            }
            let channels = host
                .channels()
                .await
                .map_err(|source| RuntimeError::Provider {
                    workspace_id: workspace_id.clone(),
                    source,
                })?;
            *provider.channels.write().await = channels;
            Ok::<_, RuntimeError>(())
        }
        .await;
        if let Err(error) = started {
            *current = None;
            let _ = host.shutdown().await;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn provider(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Arc<ProviderInstance>, RuntimeError> {
        self.providers
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(workspace_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownWorkspace(workspace_id.clone()))
    }

    fn provider_entries(&self) -> Result<Vec<(WorkspaceId, Arc<ProviderInstance>)>, RuntimeError> {
        Ok(self
            .providers
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .iter()
            .map(|(id, provider)| (id.clone(), provider.clone()))
            .collect())
    }

    pub(super) fn require_active(&self, workspace_id: &WorkspaceId) -> Result<(), RuntimeError> {
        if self.provider(workspace_id)?.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(RuntimeError::WorkspacePaused(workspace_id.clone()))
        }
    }

    pub(super) async fn host(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Arc<dyn ProviderHostInstance>, RuntimeError> {
        self.provider(workspace_id)?
            .host
            .lock()
            .await
            .clone()
            .ok_or_else(|| RuntimeError::ProviderNotStarted(workspace_id.clone()))
    }

    pub(crate) async fn lifecycle_host(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<(Arc<dyn ProviderHostInstance>, bool), RuntimeError> {
        let provider = self.provider(workspace_id)?;
        if let Some(host) = provider.host.lock().await.clone() {
            return Ok((host, false));
        }
        let host = (provider.factory)().map_err(|source| RuntimeError::Provider {
            workspace_id: workspace_id.clone(),
            source,
        })?;
        Ok((host, true))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentRuntimeConfig {
    pub agent_id: AgentId,
    /// Provider-native sender identity used to suppress self-triggering echoes.
    pub provider_sender_id: String,
    pub attention: BTreeMap<ChannelId, AttentionPolicy>,
    pub auto_reaction: Option<String>,
    pub agent_config: serde_json::Value,
    pub provider_agent_config: serde_json::Value,
    pub agent_profile: AgentRuntimeProfile,
    pub computer_id: String,
    pub harness: HarnessConfig,
    pub session: SessionDirective,
    pub execution_token: String,
}

impl AgentRuntimeConfig {
    pub(super) fn muted(&self) -> bool {
        !self
            .attention
            .values()
            .any(|policy| policy != &AttentionPolicy::Mute)
    }
}

pub(super) struct ProviderInstance {
    pub(super) active: AtomicBool,
    pub(super) factory: ProviderFactory,
    pub(super) host: Mutex<Option<Arc<dyn ProviderHostInstance>>>,
    pub(super) channels: RwLock<Vec<ChannelDefinition>>,
    pub(super) agents: RwLock<BTreeMap<AgentId, AgentRuntimeConfig>>,
}

pub type ProviderFactory =
    Arc<dyn Fn() -> Result<Arc<dyn ProviderHostInstance>, ProviderError> + Send + Sync>;
