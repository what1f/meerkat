use super::*;

impl LankideApp {
    pub(super) fn reload_provider_status(&mut self, cx: &mut Context<Self>) {
        self.telegram_manager_ready = false;
        self.provider_request += 1;
        let request = self.provider_request;
        let epoch = self.selection_epoch;
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        if workspace.provider.id != "telegram" {
            return;
        }
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client
                .invoke_provider_interface(&workspace.id, "telegram.workspace.status", json!({}))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.provider_request != request {
                    return;
                }
                match result {
                    Ok(status) => {
                        this.telegram_manager_ready = status
                            .get("manager_bot_ready")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn record_agent_onboarding_state(&mut self, state: &AgentOnboardingState) {
        let key = (state.workspace_id.clone(), state.agent_id.clone());
        if state.status == OnboardingStatus::Ready {
            self.onboarding_retries.remove(&key);
            self.onboarding_steps.remove(&key);
        } else {
            self.onboarding_retries.insert(key.clone());
            self.onboarding_steps.insert(key, state.step.clone());
        }
    }

    pub(super) fn finish_agent_onboarding(
        &mut self,
        workspace_id: &str,
        agent_id: &str,
        cx: &mut Context<Self>,
    ) {
        let key = (workspace_id.to_owned(), agent_id.to_owned());
        let repaired_permissions = self
            .onboarding_steps
            .get(&key)
            .is_some_and(|step| step == "authorization_update");
        self.onboarding_retries.remove(&key);
        self.onboarding_steps.remove(&key);

        if repaired_permissions && self.pending_agent_save_retry.as_deref() == Some(agent_id) {
            self.pending_agent_save_retry = None;
            self.onboarding_return_to_agent = None;
            self.modal = Some(Modal::EditAgent(agent_id.to_owned()));
            self.submit_agent(Some(agent_id.to_owned()), cx);
        } else if repaired_permissions
            && self.onboarding_return_to_agent.as_deref() == Some(agent_id)
        {
            self.onboarding_return_to_agent = None;
            self.error = None;
            self.modal = Some(Modal::EditAgent(agent_id.to_owned()));
            self.notify_dialog_changed(cx);
        } else {
            self.onboarding_return_to_agent = None;
            self.modal = None;
            self.reload_agents(cx);
        }
    }

    pub(super) fn advance_agent_onboarding(
        &mut self,
        state: AgentOnboardingState,
        cx: &mut Context<Self>,
    ) {
        if self.onboarding_checking {
            return;
        }
        let client = self.client.clone();
        self.onboarding_checking = true;
        self.error = None;
        self.notify_dialog_changed(cx);
        self.onboarding_poll_epoch = self.onboarding_poll_epoch.wrapping_add(1);
        let request_epoch = self.onboarding_poll_epoch;
        let retry_state = state.clone();
        let (action, payload) = next_agent_onboarding_request(&state);
        let workspace_id = state.workspace_id;
        let agent_id = state.agent_id;
        cx.spawn(async move |this, cx| {
            let result = client
                .advance_agent_onboarding(&workspace_id, &agent_id, action, payload)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.onboarding_poll_epoch != request_epoch {
                    return;
                }
                this.onboarding_checking = false;
                match result {
                    Ok(next) if next.status == OnboardingStatus::Ready => {
                        this.finish_agent_onboarding(&workspace_id, &agent_id, cx);
                    }
                    Ok(next) => {
                        this.record_agent_onboarding_state(&next);
                        this.modal = Some(Modal::AgentOnboarding(next.clone()));
                        this.schedule_agent_onboarding_poll(next, cx);
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        this.schedule_agent_onboarding_poll(retry_state, cx);
                    }
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn resume_agent_onboarding(&mut self, agent_id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace_id) = self.selected_workspace_id.clone() else {
            return;
        };
        self.onboarding_return_to_agent = Some(agent_id.clone());
        let client = self.client.clone();
        self.busy = Some(BusyOperation::ResumeAgentOnboarding);
        self.error = None;
        self.notify_dialog_changed(cx);
        cx.spawn(async move |this, cx| {
            let current = client.get_agent_onboarding(&workspace_id, &agent_id).await;
            let result = match current {
                Ok(state)
                    if state.verification_uri.as_deref().is_none()
                        && state.status != OnboardingStatus::Ready =>
                {
                    client
                        .advance_agent_onboarding(&workspace_id, &agent_id, "start", json!({}))
                        .await
                }
                other => other,
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(state) if state.status == OnboardingStatus::Ready => {
                        this.finish_agent_onboarding(&workspace_id, &agent_id, cx);
                    }
                    Ok(state) => {
                        this.record_agent_onboarding_state(&state);
                        this.modal = Some(Modal::AgentOnboarding(state.clone()));
                        this.schedule_agent_onboarding_poll(state, cx);
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn schedule_agent_onboarding_poll(
        &mut self,
        state: AgentOnboardingState,
        cx: &mut Context<Self>,
    ) {
        let Some(interval) = state.poll_interval_seconds else {
            return;
        };
        if matches!(
            state.status,
            OnboardingStatus::Ready | OnboardingStatus::Error
        ) {
            return;
        }
        let interval = u64::from(interval);
        let step = state.step.clone();
        let verification_uri = state.verification_uri.clone();
        let workspace_id = state.workspace_id.clone();
        let agent_id = state.agent_id.clone();
        let payload = state.payload.clone();
        let client = self.client.clone();
        self.onboarding_poll_epoch = self.onboarding_poll_epoch.wrapping_add(1);
        let poll_epoch = self.onboarding_poll_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(interval))
                .await;
            let should_poll = this
                .update(cx, |this, _| {
                    this.onboarding_poll_epoch == poll_epoch
                        && this.busy.is_none()
                        && !this.onboarding_checking
                        && matches!(
                            &this.modal,
                            Some(Modal::AgentOnboarding(current))
                                if current.workspace_id == workspace_id
                                    && current.agent_id == agent_id
                                    && current.step == step
                                    && current.verification_uri == verification_uri
                        )
                })
                .unwrap_or(false);
            if !should_poll {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.onboarding_checking = true;
                this.error = None;
                cx.notify();
            });
            let result = client
                .advance_agent_onboarding(&workspace_id, &agent_id, "poll", payload)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.onboarding_poll_epoch != poll_epoch {
                    return;
                }
                this.onboarding_checking = false;
                match result {
                    Ok(next) if next.status == OnboardingStatus::Ready => {
                        this.finish_agent_onboarding(&workspace_id, &agent_id, cx);
                    }
                    Ok(next) => {
                        this.record_agent_onboarding_state(&next);
                        this.modal = Some(Modal::AgentOnboarding(next.clone()));
                        this.schedule_agent_onboarding_poll(next, cx);
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        this.schedule_agent_onboarding_poll(state, cx);
                    }
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }
}

pub(crate) fn next_agent_onboarding_request(state: &AgentOnboardingState) -> (&'static str, Value) {
    let can_poll = state.poll_interval_seconds.is_some();
    if can_poll {
        ("poll", state.payload.clone())
    } else {
        ("start", json!({}))
    }
}

pub(super) fn telegram_bot_username(workspace_id: &str, agent_id: &str) -> String {
    fn normalize(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_owned()
    }
    let workspace = normalize(workspace_id);
    let workspace = workspace
        .strip_prefix("telegram_")
        .map(|value| format!("tg_{value}"))
        .unwrap_or(workspace);
    let agent = normalize(agent_id);
    let agent = &agent[..agent.len().min(14)];
    let workspace_limit = 27usize.saturating_sub(agent.len()).max(1);
    let workspace = &workspace[..workspace.len().min(workspace_limit)];
    format!("{workspace}_{agent}_bot")
}

pub(crate) fn localized_agent_onboarding_copy(
    state: &AgentOnboardingState,
    language: DesktopLanguage,
) -> (String, String) {
    let is_permission_update =
        state.provider_id == "feishu" && state.step == "authorization_update";
    if is_permission_update {
        return match state.status {
            OnboardingStatus::Ready => (
                language
                    .text("飞书权限已更新", "Feishu permissions updated")
                    .into(),
                language
                    .text("现在可以继续。", "You can continue now.")
                    .into(),
            ),
            OnboardingStatus::Error => (
                language
                    .text("未能更新飞书权限", "Could not update Feishu permissions")
                    .into(),
                language.text("请稍后重试。", "Try again shortly.").into(),
            ),
            OnboardingStatus::ActionRequired if state.verification_uri.as_deref().is_some() => (
                language
                    .text("在飞书中确认权限更新", "Confirm the update in Feishu")
                    .into(),
                language
                    .text(
                        "打开下方链接完成确认。",
                        "Open the link below to confirm the update.",
                    )
                    .into(),
            ),
            OnboardingStatus::ActionRequired => (
                language
                    .text("更新飞书权限", "Update Feishu permissions")
                    .into(),
                language
                    .text(
                        "更新后即可继续使用完整功能。",
                        "Update permissions to continue using all features.",
                    )
                    .into(),
            ),
            OnboardingStatus::Verifying => (
                language
                    .text("正在更新飞书权限", "Updating Feishu permissions")
                    .into(),
                language
                    .text("完成后将自动继续。", "You can continue when it finishes.")
                    .into(),
            ),
        };
    }
    let is_waiting_for_confirmation = state.poll_interval_seconds.is_some()
        && !matches!(
            state.status,
            OnboardingStatus::Ready | OnboardingStatus::Error
        );
    if is_waiting_for_confirmation {
        if state.provider_id == "feishu" && state.step == "console_login" {
            return (
                language.text("登录飞书", "Sign in to Feishu").to_owned(),
                language
                    .text(
                        "登录后自动创建 Agent 应用。",
                        "The Agent app is created after you sign in.",
                    )
                    .to_owned(),
            );
        }
        if state.provider_id == "feishu"
            && state.step == "permissions"
            && state.verification_uri.is_none()
        {
            return (
                language.text("正在创建应用", "Creating the app").to_owned(),
                language
                    .text("完成后自动启用。", "The Agent is enabled when it finishes.")
                    .to_owned(),
            );
        }
        if state.provider_id == "feishu" && state.step == "permissions" {
            return (
                language
                    .text(
                        "应用已创建，继续授权",
                        "App created — continue authorization",
                    )
                    .to_owned(),
                language
                    .text(
                        "还需完成 Agent 能力授权，完成后将自动启用。",
                        "Authorize Agent capabilities to finish setup.",
                    )
                    .to_owned(),
            );
        }
        if state.provider_id == "telegram" {
            return (
                language
                    .text("在 Telegram 中确认", "Confirm in Telegram")
                    .to_owned(),
                language
                    .text(
                        "打开下方链接完成 Bot 创建。",
                        "Open the link below to create the Bot.",
                    )
                    .to_owned(),
            );
        }
        return (
            language
                .text("等待确认", "Waiting for confirmation")
                .to_owned(),
            language
                .text(
                    "打开下方链接完成设置",
                    "Open the link below to finish setup",
                )
                .to_owned(),
        );
    }
    let has_link = state.verification_uri.as_deref().is_some();
    let (title_zh, title_en, instruction_zh, instruction_en) = match state.status {
        OnboardingStatus::Ready => (
            "Agent 已启用",
            "Agent enabled",
            "连接已完成。",
            "Connection completed.",
        ),
        OnboardingStatus::Error => (
            "未能完成设置",
            "Setup incomplete",
            "请重新发起设置。",
            "Start setup again.",
        ),
        OnboardingStatus::Verifying => (
            "等待确认",
            "Waiting for confirmation",
            "完成确认后，点击检查状态。",
            "After confirming, check the status.",
        ),
        OnboardingStatus::ActionRequired if has_link => (
            "请完成确认",
            "Confirm setup",
            "打开下方链接完成设置，然后返回检查状态。",
            "Open the link below to finish setup, then return to check the status.",
        ),
        OnboardingStatus::ActionRequired => (
            "连接 Agent",
            "Connect Agent",
            "继续后，按照提示完成连接。",
            "Continue and follow the prompts to connect.",
        ),
    };
    (
        language.text(title_zh, title_en).to_owned(),
        language.text(instruction_zh, instruction_en).to_owned(),
    )
}
