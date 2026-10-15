#![allow(dead_code)]
use lankide_client::*;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn local_harnesses() -> Vec<HarnessOption> {
    vec![
        HarnessOption {
            id: "pi".into(),
            name: "Pi".into(),
            executable_path: None,
        },
        HarnessOption {
            id: "codex".into(),
            name: "Codex".into(),
            executable_path: None,
        },
        HarnessOption {
            id: "claude_code".into(),
            name: "Claude Code".into(),
            executable_path: Some("/usr/local/bin/dcc".into()),
        },
    ]
}

fn provider_name(id: &str) -> String {
    match id {
        "feishu" => "飞书".into(),
        "telegram" => "Telegram".into(),
        other => other.into(),
    }
}

fn validate_agent_choice(harness_id: &str, cli_path: &str) -> Result<(), ClientError> {
    if cli_path.ends_with("/dcc") && harness_id != "claude_code" {
        return Err(ClientError::Validation(
            "dcc is only a Claude Code executable choice".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockState {
    Loading,
    Empty,
    Success,
    Error,
    Retrying,
}
#[derive(Clone, Debug)]
pub struct MockManagementClient {
    data: Arc<tokio::sync::Mutex<MockData>>,
}
#[derive(Clone, Debug)]
struct MockData {
    state: MockState,
    fault: Option<ClientError>,
    onboarding_fault: Option<ClientError>,
    delayed_agents: Vec<(String, Arc<tokio::sync::Notify>)>,
    delayed_onboarding: Vec<(String, Arc<tokio::sync::Notify>)>,
    delayed_agent_updates: Vec<(String, Arc<tokio::sync::Notify>)>,
    delayed_workspace_states: Vec<(String, Arc<tokio::sync::Notify>)>,
    agent_onboarding: std::collections::BTreeMap<(String, String), AgentOnboardingState>,
    workspaces: Vec<Workspace>,
    agents: Vec<Agent>,
    sequence: u64,
    images_requested: Vec<(String, String)>,
}
impl Default for MockManagementClient {
    fn default() -> Self {
        Self::sample()
    }
}
impl MockManagementClient {
    pub fn sample() -> Self {
        let ws = |id: &str, name: &str, p: &str| Workspace {
            id: id.into(),
            name: name.into(),
            rules: String::new(),
            provider: ProviderInfo {
                id: p.into(),
                name: provider_name(p),
                icon: p.into(),
            },
            status: WorkspaceStatus::Active,
            observed_state: WorkspaceObservedState::Running,
            status_message: String::new(),
            cover_resource_id: None,
            task_system_enabled: p == "feishu",
            provider_config: json!({"task_system_enabled": p == "feishu"}),
        };
        let agent = |w: &str, id: &str, name: &str, role: &str| Agent {
            pause_reason: None,
            workspace_id: w.into(),
            id: id.into(),
            name: name.into(),
            role: role.into(),
            bio: match id {
                "clark" => "兴趣是第一生产力，品味是最深的护城河。",
                "evan" => "I build, I ship, I keep the bundles small.",
                "rocky" => "该稳时稳如磐石，该浪时浪迹天涯",
                "bruce" => "I’m Rich",
                _ => "",
            }
            .into(),
            avatar: Avatar::Custom(name.into()),
            soul: String::new(),
            computer_id: "fixture-computer".into(),
            harness_id: "pi".into(),
            model: "deepseek/deepseek-v4-flash".into(),
            thinking_level: ThinkingLevel::High,
            auto_reaction: None,
            cli_path: String::new(),
            environment_variables: vec![],
            provider_handle: format!("{w}-{id}"),
            provider_agent_config: json!({"provider_handle": format!("{w}-{id}")}),
        };
        Self {
            data: Arc::new(tokio::sync::Mutex::new(MockData {
                state: MockState::Success,
                fault: None,
                onboarding_fault: None,
                delayed_agents: Vec::new(),
                delayed_onboarding: Vec::new(),
                delayed_agent_updates: Vec::new(),
                delayed_workspace_states: Vec::new(),
                agent_onboarding: std::collections::BTreeMap::new(),
                workspaces: vec![
                    ws("example-team", "示例团队", "feishu"),
                    ws("tg-team", "TG小队", "telegram"),
                ],
                agents: vec![
                    agent("example-team", "clark", "Clark", "PM"),
                    agent("example-team", "evan", "Evan", "RD"),
                    agent("example-team", "rocky", "Rocky", "QA"),
                    agent("tg-team", "bruce", "Bruce", "RD"),
                ],
                sequence: 0,
                images_requested: Vec::new(),
            })),
        }
    }
    pub fn empty() -> Self {
        Self {
            data: Arc::new(tokio::sync::Mutex::new(MockData {
                state: MockState::Empty,
                fault: None,
                onboarding_fault: None,
                delayed_agents: Vec::new(),
                delayed_onboarding: Vec::new(),
                delayed_agent_updates: Vec::new(),
                delayed_workspace_states: Vec::new(),
                agent_onboarding: std::collections::BTreeMap::new(),
                workspaces: vec![],
                agents: vec![],
                sequence: 0,
                images_requested: Vec::new(),
            })),
        }
    }
    pub async fn set_state(&self, s: MockState) {
        self.data.lock().await.state = s
    }
    pub async fn fail_next(&self, e: ClientError) {
        self.data.lock().await.fault = Some(e)
    }
    pub async fn fail_next_onboarding(&self, error: ClientError) {
        self.data.lock().await.onboarding_fault = Some(error);
    }
    pub async fn set_agent_onboarding(&self, state: AgentOnboardingState) {
        self.data
            .lock()
            .await
            .agent_onboarding
            .insert((state.workspace_id.clone(), state.agent_id.clone()), state);
    }
    pub async fn delay_next_agents(&self, workspace: &str) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.data
            .lock()
            .await
            .delayed_agents
            .push((workspace.into(), gate.clone()));
        gate
    }
    pub async fn delay_next_onboarding(&self, agent: &str) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.data
            .lock()
            .await
            .delayed_onboarding
            .push((agent.into(), gate.clone()));
        gate
    }
    pub async fn delay_next_agent_update(&self, agent: &str) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.data
            .lock()
            .await
            .delayed_agent_updates
            .push((agent.into(), gate.clone()));
        gate
    }
    pub async fn delay_next_workspace_state(&self, workspace: &str) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        self.data
            .lock()
            .await
            .delayed_workspace_states
            .push((workspace.into(), gate.clone()));
        gate
    }
    pub async fn image_requests(&self) -> Vec<(String, String)> {
        self.data.lock().await.images_requested.clone()
    }
    async fn lock(&self) -> Result<tokio::sync::MutexGuard<'_, MockData>, ClientError> {
        let mut d = self.data.lock().await;
        if let Some(e) = d.fault.take() {
            return Err(e);
        }
        match d.state {
            MockState::Loading => Err(ClientError::Unavailable("loading".into())),
            MockState::Error => Err(ClientError::Provider("mock provider error".into())),
            MockState::Retrying => Err(ClientError::Unavailable("retrying".into())),
            _ => Ok(d),
        }
    }
    fn valid(id: &str) -> bool {
        !id.is_empty()
            && id.as_bytes()[0].is_ascii_lowercase()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }
}

#[async_trait::async_trait]
impl ManagementClient for MockManagementClient {
    async fn get_workspace_image(
        &self,
        workspace: &str,
        resource: i64,
    ) -> Result<WorkspaceImage, ClientError> {
        self.data
            .lock()
            .await
            .images_requested
            .push((workspace.into(), resource.to_string()));
        Ok(WorkspaceImage {
            media_type: "image/png".into(),
            bytes: include_bytes!("../../../runtime/tests/assets/profile.png").to_vec(),
        })
    }

    async fn get_agent_avatar(
        &self,
        workspace: &str,
        agent: &str,
    ) -> Result<AgentAvatarImage, ClientError> {
        self.data
            .lock()
            .await
            .images_requested
            .push((workspace.into(), agent.into()));
        Ok(AgentAvatarImage {
            path: format!("resources/{workspace}/{agent}_avatar.png"),
            media_type: "image/png".into(),
            bytes: include_bytes!("../../../runtime/tests/assets/profile.png").to_vec(),
        })
    }

    async fn list_workspaces(&self) -> Result<Vec<Workspace>, ClientError> {
        let data = self.lock().await?;
        Ok(if data.state == MockState::Empty {
            Vec::new()
        } else {
            data.workspaces.clone()
        })
    }
    async fn get_workspace(&self, id: &str) -> Result<Workspace, ClientError> {
        self.lock()
            .await?
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .cloned()
            .ok_or_else(|| ClientError::WorkspaceNotFound(id.into()))
    }
    async fn create_workspace(&self, d: WorkspaceDraft) -> Result<Workspace, ClientError> {
        if !Self::valid(&d.id) {
            return Err(ClientError::Validation("invalid workspace id".into()));
        }
        let mut data = self.lock().await?;
        if data.workspaces.iter().any(|workspace| workspace.id == d.id) {
            return Err(ClientError::DuplicateWorkspaceId(d.id));
        }
        let w = Workspace {
            id: d.id,
            name: d.name,
            rules: d.rules,
            provider: ProviderInfo {
                id: d.provider_id.clone(),
                name: provider_name(&d.provider_id),
                icon: d.provider_id,
            },
            status: WorkspaceStatus::Active,
            observed_state: WorkspaceObservedState::Running,
            status_message: String::new(),
            cover_resource_id: None,
            task_system_enabled: d
                .provider_config
                .get("task_system_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            provider_config: d.provider_config,
        };
        data.workspaces.push(w.clone());
        Ok(w)
    }
    async fn update_workspace(&self, w: Workspace) -> Result<Workspace, ClientError> {
        let mut d = self.lock().await?;
        let x = d
            .workspaces
            .iter_mut()
            .find(|x| x.id == w.id)
            .ok_or_else(|| ClientError::WorkspaceNotFound(w.id.clone()))?;
        *x = w.clone();
        Ok(w)
    }
    async fn set_workspace_state(
        &self,
        id: &str,
        s: WorkspaceStatus,
    ) -> Result<Workspace, ClientError> {
        let gate = {
            let mut data = self.data.lock().await;
            data.delayed_workspace_states
                .iter()
                .position(|(workspace, _)| workspace == id)
                .map(|index| data.delayed_workspace_states.remove(index).1)
        };
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let mut d = self.lock().await?;
        let w = d
            .workspaces
            .iter_mut()
            .find(|x| x.id == id)
            .ok_or_else(|| ClientError::WorkspaceNotFound(id.into()))?;
        w.status = s;
        Ok(w.clone())
    }
    async fn list_agents(&self, w: &str) -> Result<Vec<Agent>, ClientError> {
        let mut d = self.lock().await?;
        if d.state == MockState::Empty {
            return Ok(Vec::new());
        }
        if !d.workspaces.iter().any(|x| x.id == w) {
            return Err(ClientError::WorkspaceNotFound(w.into()));
        }
        let agents = d
            .agents
            .iter()
            .filter(|a| a.workspace_id == w)
            .cloned()
            .collect();
        let gate = d
            .delayed_agents
            .iter()
            .position(|(id, _)| id == w)
            .map(|index| d.delayed_agents.remove(index).1);
        drop(d);
        if let Some(gate) = gate {
            gate.notified().await;
        }
        Ok(agents)
    }
    async fn get_agent(&self, w: &str, a: &str) -> Result<Agent, ClientError> {
        self.lock()
            .await?
            .agents
            .iter()
            .find(|x| x.workspace_id == w && x.id == a)
            .cloned()
            .ok_or_else(|| ClientError::AgentNotFound(a.into()))
    }
    async fn create_agent(&self, w: &str, d: AgentDraft) -> Result<Agent, ClientError> {
        let mut data = self.lock().await?;
        if !data.workspaces.iter().any(|x| x.id == w) {
            return Err(ClientError::WorkspaceNotFound(w.into()));
        }
        if !Self::valid(&d.id) {
            return Err(ClientError::InvalidAgentId);
        }
        validate_agent_choice(&d.harness_id, &d.cli_path)?;
        if data
            .agents
            .iter()
            .any(|a| a.workspace_id == w && a.id == d.id)
        {
            return Err(ClientError::DuplicateAgentId(d.id));
        }
        let a = Agent {
            pause_reason: None,
            workspace_id: w.into(),
            id: d.id,
            name: d.name,
            role: d.role,
            bio: d.bio,
            avatar: d.avatar,
            soul: d.soul,
            computer_id: d.computer_id,
            harness_id: d.harness_id,
            model: d.model,
            thinking_level: d.thinking_level,
            auto_reaction: d.auto_reaction,
            cli_path: d.cli_path,
            environment_variables: d.environment_variables,
            provider_handle: d.provider_handle,
            provider_agent_config: d.provider_agent_config,
        };
        data.agents.push(a.clone());
        Ok(a)
    }
    async fn update_agent(
        &self,
        mut a: Agent,
        avatar: Option<lankide_client::AgentAvatarUpload>,
    ) -> Result<Agent, ClientError> {
        validate_agent_choice(&a.harness_id, &a.cli_path)?;
        if let Some(avatar) = avatar {
            let extension = match avatar.media_type.as_str() {
                "image/jpeg" => "jpg",
                "image/webp" => "webp",
                "image/gif" => "gif",
                _ => "png",
            };
            a.avatar = Avatar::Path(format!(
                "resources/{}/{}_avatar.{extension}",
                a.workspace_id, a.id
            ));
        }
        let gate = {
            let mut data = self.data.lock().await;
            data.delayed_agent_updates
                .iter()
                .position(|(id, _)| id == &a.id)
                .map(|index| data.delayed_agent_updates.remove(index).1)
        };
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let mut d = self.lock().await?;
        let x = d
            .agents
            .iter_mut()
            .find(|x| x.workspace_id == a.workspace_id && x.id == a.id)
            .ok_or_else(|| ClientError::AgentNotFound(a.id.clone()))?;
        *x = a.clone();
        Ok(a)
    }
    async fn resume_agent(&self, w: &str, a: &str) -> Result<Agent, ClientError> {
        let mut agent = self.get_agent(w, a).await?;
        agent.pause_reason = None;
        self.update_agent(agent, None).await
    }
    async fn delete_agent(&self, workspace_id: &str, agent_id: &str) -> Result<(), ClientError> {
        let mut data = self.lock().await?;
        let before = data.agents.len();
        data.agents
            .retain(|agent| agent.workspace_id != workspace_id || agent.id != agent_id);
        if data.agents.len() == before {
            return Err(ClientError::AgentNotFound(agent_id.into()));
        }
        Ok(())
    }
    async fn get_provider_onboarding(
        &self,
        w: &str,
    ) -> Result<ProviderOnboardingState, ClientError> {
        let ws = ManagementClient::get_workspace(self, w).await?;
        Ok(ProviderOnboardingState {
            workspace_id: w.into(),
            provider_id: ws.provider.id,
            step: "credentials".into(),
            status: OnboardingStatus::ActionRequired,
            verification_uri: None,
            poll_interval_seconds: None,
            payload: json!({}),
            error: None,
        })
    }
    async fn advance_provider_onboarding(
        &self,
        w: &str,
        a: &str,
        p: Value,
    ) -> Result<ProviderOnboardingState, ClientError> {
        let mut s = self.get_provider_onboarding(w).await?;
        s.step = a.into();
        s.status = OnboardingStatus::Ready;
        s.payload = p;
        Ok(s)
    }
    async fn get_agent_onboarding(
        &self,
        w: &str,
        a: &str,
    ) -> Result<AgentOnboardingState, ClientError> {
        if let Some(error) = self.data.lock().await.onboarding_fault.take() {
            return Err(error);
        }
        let workspace = ManagementClient::get_workspace(self, w).await?;
        let _ = ManagementClient::get_agent(self, w, a).await?;
        if let Some(state) = self
            .data
            .lock()
            .await
            .agent_onboarding
            .get(&(w.to_owned(), a.to_owned()))
            .cloned()
        {
            return Ok(state);
        }
        Ok(AgentOnboardingState {
            workspace_id: w.into(),
            agent_id: a.into(),
            provider_id: workspace.provider.id,
            step: "create_bot".into(),
            status: OnboardingStatus::ActionRequired,
            verification_uri: None,
            poll_interval_seconds: None,
            payload: json!({}),
            error: None,
        })
    }
    async fn advance_agent_onboarding(
        &self,
        w: &str,
        a: &str,
        action: &str,
        payload: Value,
    ) -> Result<AgentOnboardingState, ClientError> {
        let mut data = self.data.lock().await;
        if let Some(error) = data.onboarding_fault.take() {
            return Err(error);
        }
        let gate = data
            .delayed_onboarding
            .iter()
            .position(|(agent, _)| agent == a)
            .map(|index| data.delayed_onboarding.remove(index).1);
        drop(data);
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let mut state = self.get_agent_onboarding(w, a).await?;
        state.step = action.into();
        state.status = OnboardingStatus::Ready;
        state.verification_uri = None;
        state.poll_interval_seconds = None;
        state.payload = payload;
        self.data
            .lock()
            .await
            .agent_onboarding
            .insert((w.to_owned(), a.to_owned()), state.clone());
        Ok(state)
    }
    async fn get_provider_catalog(&self) -> Result<Vec<ProviderInfo>, ClientError> {
        drop(self.lock().await?);
        Ok(vec![
            ProviderInfo {
                id: "feishu".into(),
                name: "飞书".into(),
                icon: "feishu".into(),
            },
            ProviderInfo {
                id: "telegram".into(),
                name: "Telegram".into(),
                icon: "telegram".into(),
            },
        ])
    }
    async fn get_computer_catalog(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<ComputerOption>, ClientError> {
        drop(self.lock().await?);
        Ok(vec![ComputerOption {
            registered: true,
            id: "fixture-computer".into(),
            name: "Local Computer".into(),
            available: true,
            ..ComputerOption::default()
        }])
    }
    async fn get_harness_catalog(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<HarnessOption>, ClientError> {
        drop(self.lock().await?);
        Ok(local_harnesses())
    }
    async fn get_model_catalog(
        &self,
        _: Option<&str>,
        _: &str,
        h: &str,
    ) -> Result<Vec<ModelOption>, ClientError> {
        drop(self.lock().await?);
        Ok(vec![ModelOption {
            thinking_levels: vec!["low".into(), "medium".into(), "high".into()],
            id: if h == "codex" {
                "gpt-5.6-luna"
            } else {
                "deepseek-v4-flash"
            }
            .into(),
            name: "Default".into(),
            available: true,
        }])
    }
    async fn get_reaction_catalog(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<ReactionOption>, ClientError> {
        drop(self.lock().await?);
        Ok((0..48)
            .map(|index| ReactionOption {
                id: format!("test-{index}"),
                name: format!("Reaction {index}"),
                emoji: "●".into(),
                available: true,
            })
            .collect())
    }
    async fn invoke_provider_interface(
        &self,
        workspace_id: &str,
        interface_id: &str,
        input: Value,
    ) -> Result<Value, ClientError> {
        let data = self.lock().await?;
        if interface_id == "telegram.workspace.status" {
            let workspace = data
                .workspaces
                .iter()
                .find(|workspace| workspace.id == workspace_id)
                .ok_or_else(|| ClientError::WorkspaceNotFound(workspace_id.into()))?;
            return Ok(json!({
                "manager_bot_ready": workspace.provider.id == "telegram",
                "manager_bot_username": if workspace.provider.id == "telegram" {
                    Some("lankide_mgr_260822_bot")
                } else {
                    None
                }
            }));
        }
        Ok(input)
    }
    async fn list_provider_interfaces(
        &self,
        _: &str,
    ) -> Result<Vec<ProviderInterface>, ClientError> {
        drop(self.lock().await?);
        Ok(vec![ProviderInterface {
            id: "fixture.echo".into(),
            description: None,
            input_schema: json!({"type":"object"}),
            output_schema: json!({"type":"object"}),
        }])
    }
    async fn subscribe_management_events(
        &self,
        w: Option<&str>,
        after: u64,
    ) -> Result<ManagementSubscription, ClientError> {
        let mut d = self.lock().await?;
        d.sequence = d.sequence.max(after) + 1;
        let e = ManagementEvent {
            sequence: d.sequence,
            workspace_id: w.unwrap_or("example-team").into(),
            agent_id: None,
            kind: ManagementEventKind::CatalogChanged,
        };
        Ok(ManagementSubscription::from_stream(tokio_stream::iter(
            vec![Ok(e)],
        )))
    }
}
