use super::*;

impl LankideApp {
    pub(super) fn selected_workspace(&self) -> Option<&Workspace> {
        let id = self.selected_workspace_id.as_deref()?;
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    pub(super) fn provider_supports_capability(&self, provider_id: &str, capability: &str) -> bool {
        self.provider_capabilities
            .get(provider_id)
            .is_some_and(|capabilities| capabilities.iter().any(|item| item == capability))
    }

    pub(super) fn select_workspace_provider(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        self.workspace_provider = provider_id.to_owned();
        if !self.provider_supports_capability(provider_id, PROVIDER_CAPABILITY_TASK_SYSTEM) {
            self.workspace_task_enabled = false;
        }
        self.notify_dialog_changed(cx);
    }

    pub(super) fn reload_workspaces(&mut self, cx: &mut Context<Self>) {
        // A reconnect retry keeps the current error state instead of flashing a loader.
        if self.error.is_none() {
            self.loading = true;
        }
        let workspace_client = self.client.clone();
        let capability_client = self.client.clone();
        self.workspace_request += 1;
        let request = self.workspace_request;
        cx.spawn(async move |this, cx| {
            let (result, provider_capability_catalog) = tokio::join!(
                workspace_client.list_workspaces(),
                capability_client.get_provider_capability_catalog()
            );
            let _ = this.update(cx, |this, cx| {
                if this.workspace_request != request {
                    return;
                }
                if let Ok(catalog) = provider_capability_catalog {
                    this.provider_capabilities = catalog
                        .into_iter()
                        .map(|entry| (entry.provider_id, entry.capabilities))
                        .collect();
                }
                match result {
                    Ok(workspaces) => {
                        this.loading = false;
                        this.error = None;
                        this.workspace_loaded = true;
                        this.workspaces = workspaces;
                        this.workspace_startup_retries = 0;
                        let selected = this
                            .selected_workspace_id
                            .clone()
                            .filter(|id| {
                                this.workspaces.iter().any(|workspace| &workspace.id == id)
                            })
                            .or_else(|| {
                                this.workspaces
                                    .first()
                                    .map(|workspace| workspace.id.clone())
                            });
                        if this.selected_workspace_id != selected {
                            this.selection_epoch += 1;
                            this.agents.clear();
                            this.images.clear();
                            this.reaction_options.clear();
                        }
                        this.selected_workspace_id = selected;
                        if this.selected_workspace_id.is_some() {
                            this.reload_agents(cx);
                        } else {
                            this.finish_boot(cx);
                        }
                    }
                    // Background services report their own outcome; until then a
                    // failed request only means the Runtime is still starting.
                    Err(_) if this.startup_pending || this.workspace_startup_retries < 50 => {
                        this.workspace_startup_retries =
                            this.workspace_startup_retries.saturating_add(1);
                        this.schedule_workspace_reload(cx);
                    }
                    Err(error) => {
                        let reconnect = is_connection_error(&error);
                        this.loading = false;
                        this.error = Some(UiMessage::client(error));
                        this.finish_boot(cx);
                        // launchd restarts the Runtime; keep trying until it answers.
                        if reconnect {
                            this.workspace_startup_retries =
                                this.workspace_startup_retries.saturating_add(1);
                            this.schedule_workspace_reload(cx);
                        }
                    }
                };
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn schedule_workspace_reload(&mut self, cx: &mut Context<Self>) {
        let request = self.workspace_request;
        // 300 ms while starting, then back off to 5 s.
        let delay = Duration::from_millis(300)
            .saturating_mul(1 << self.workspace_startup_retries.saturating_sub(50).min(4))
            .min(Duration::from_secs(5));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace_request == request {
                    this.reload_workspaces(cx);
                }
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn select_workspace(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.selected_workspace_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.selection_epoch += 1;
        self.selected_workspace_id = Some(id);
        self.agents.clear();
        self.images.clear();
        self.reaction_options.clear();
        self.reaction_loading = false;
        self.reaction_error = None;
        self.modal = None;
        self.error = None;
        self.reload_agents(cx);
        cx.notify();
    }

    fn reset_workspace_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.workspace_id_input, "", window, cx);
        set_input(&self.workspace_name_input, "", window, cx);
        set_input(&self.workspace_manager_token_input, "", window, cx);
        self.workspace_provider = "feishu".into();
        self.workspace_task_enabled = false;
        self.workspace_id_error = None;
        self.workspace_name_error = None;
        self.workspace_token_error = None;
        self.error = None;
    }

    pub(super) fn open_create_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.reset_workspace_form(window, cx);
        self.modal = Some(Modal::CreateWorkspace { step: 0 });
        cx.notify();
    }

    pub(super) fn open_workspace_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        set_input(&self.settings_name_input, &workspace.name, window, cx);
        set_textarea(&self.settings_rules_input, &workspace.rules, window, cx);
        set_input(
            &self.settings_task_name_input,
            workspace
                .provider_config
                .get("task_list_name")
                .and_then(Value::as_str)
                .unwrap_or(self.language.text("团队任务", "Team tasks")),
            window,
            cx,
        );
        self.settings_task_enabled = workspace.task_system_enabled;
        self.page = Page::Workspace;
        self.error = None;
        cx.notify();
    }

    pub(super) fn submit_workspace(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let id = input_value(&self.workspace_id_input, cx);
        let name = input_value(&self.workspace_name_input, cx);
        self.workspace_id_error = if id.trim().is_empty() {
            Some(UiMessage::localized(
                "请填写 Workspace ID。",
                "Enter a Workspace ID.",
            ))
        } else if !valid_id(&id) {
            Some(UiMessage::localized(
                "只能使用小写字母、数字和下划线。",
                "Use lowercase letters, digits and underscores only.",
            ))
        } else {
            None
        };
        self.workspace_name_error = name
            .trim()
            .is_empty()
            .then(|| UiMessage::localized("请填写工作区名称。", "Enter a workspace name."));
        if self.workspace_id_error.is_some() || self.workspace_name_error.is_some() {
            self.error = None;
            cx.notify();
            return;
        }
        let provider_id = self.workspace_provider.clone();
        let manager_bot_token = input_value(&self.workspace_manager_token_input, cx);
        if provider_id == "telegram" && manager_bot_token.trim().is_empty() {
            self.workspace_token_error = Some(UiMessage::localized(
                "请填写 Manager Bot Token。",
                "Enter a Manager Bot Token.",
            ));
            cx.notify();
            return;
        }
        self.workspace_token_error = None;
        let task_enabled = self.workspace_task_enabled
            && self.provider_supports_capability(&provider_id, PROVIDER_CAPABILITY_TASK_SYSTEM);
        let provider_secrets = if provider_id == "telegram" {
            json!({"manager_bot_token": manager_bot_token})
        } else {
            json!({})
        };
        let draft = WorkspaceDraft {
            id,
            name: name.trim().into(),
            rules: String::new(),
            provider_id,
            provider_config: json!({"task_system_enabled": task_enabled}),
            provider_secrets,
        };
        let client = self.client.clone();
        self.busy = Some(BusyOperation::CreateWorkspace);
        self.error = None;
        self.notify_dialog_changed(cx);
        cx.spawn(async move |this, cx| {
            let created = client.create_workspace(draft).await;
            let notification = this.update(cx, |this, cx| {
                this.busy = None;
                let notification = match created {
                    Ok(workspace) => {
                        this.error = None;
                        this.selection_epoch += 1;
                        this.selected_workspace_id = Some(workspace.id.clone());
                        this.reaction_options.clear();
                        this.telegram_manager_ready = false;
                        let feishu =
                            (workspace.provider.id == "feishu").then(|| workspace.id.clone());
                        this.workspaces.push(workspace);
                        this.agents.clear();
                        this.images.clear();
                        this.modal = None;
                        if let Some(workspace_id) = feishu {
                            this.begin_feishu_sign_in(workspace_id, cx);
                        }
                        Some((
                            NotificationType::Success,
                            UiMessage::localized("工作区已创建。", "Workspace created."),
                        ))
                    }
                    Err(ClientError::DuplicateWorkspaceId(_)) => {
                        this.workspace_id_error = Some(UiMessage::localized(
                            "这个 Workspace ID 已经被使用。",
                            "This Workspace ID is already in use.",
                        ));
                        this.error = None;
                        None
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        None
                    }
                };
                cx.notify();
                notification.map(|(notification_type, message)| {
                    (notification_type, message.text(this.language))
                })
            });
            if let Ok(Some((notification_type, message))) = notification {
                push_notification(cx, window_handle, notification_type, message);
            }
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn save_workspace_settings(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let Some(mut workspace) = self.selected_workspace().cloned() else {
            return;
        };
        let name = input_value(&self.settings_name_input, cx);
        if name.trim().is_empty() {
            self.error = Some(UiMessage::localized(
                "请填写工作区名称。",
                "Enter a workspace name.",
            ));
            cx.notify();
            return;
        }
        workspace.name = name.trim().into();
        workspace.rules = textarea_value(&self.settings_rules_input, cx);
        let task_system_enabled = self.settings_task_enabled
            && self.provider_supports_capability(
                &workspace.provider.id,
                PROVIDER_CAPABILITY_TASK_SYSTEM,
            );
        workspace.task_system_enabled = task_system_enabled;
        workspace.provider_config["task_system_enabled"] = json!(task_system_enabled);
        workspace.provider_config["task_list_name"] =
            json!(input_value(&self.settings_task_name_input, cx).trim());
        let client = self.client.clone();
        self.busy = Some(BusyOperation::SaveWorkspace);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.update_workspace(workspace).await;
            let notification = this.update(cx, |this, cx| {
                this.busy = None;
                let notification = match result {
                    Ok(updated) => {
                        if let Some(current) =
                            this.workspaces.iter_mut().find(|w| w.id == updated.id)
                        {
                            *current = updated;
                        }
                        Some((
                            NotificationType::Success,
                            UiMessage::localized("工作区设置已保存。", "Workspace settings saved."),
                        ))
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        None
                    }
                };
                cx.notify();
                notification.map(|(notification_type, message)| {
                    (notification_type, message.text(this.language))
                })
            });
            if let Ok(Some((notification_type, message))) = notification {
                push_notification(cx, window_handle, notification_type, message);
            }
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn toggle_runtime_state(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        let next = if workspace.status == WorkspaceStatus::Active {
            WorkspaceStatus::Paused
        } else {
            WorkspaceStatus::Active
        };
        let id = workspace.id;
        let client = self.client.clone();
        self.busy = Some(BusyOperation::ToggleRuntime);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.set_workspace_state(&id, next).await;
            let notification = this.update(cx, |this, cx| {
                this.busy = None;
                let notification = match result {
                    Ok(updated) => {
                        let observed = updated.observed_state;
                        if let Some(current) =
                            this.workspaces.iter_mut().find(|w| w.id == updated.id)
                        {
                            *current = updated;
                        }
                        use lankide_client::WorkspaceObservedState;
                        this.error = None;
                        match observed {
                            WorkspaceObservedState::Running if next == WorkspaceStatus::Active => {
                                Some((
                                    NotificationType::Success,
                                    UiMessage::localized("已启用。", "Enabled."),
                                ))
                            }
                            WorkspaceObservedState::Paused => Some((
                                NotificationType::Success,
                                UiMessage::localized("已暂停。", "Paused."),
                            )),
                            WorkspaceObservedState::Error => {
                                this.error = Some(UiMessage::localized(
                                    "需要处理，请检查设置后重试。",
                                    "Needs attention. Check the settings and retry.",
                                ));
                                None
                            }
                            _ => Some((
                                NotificationType::Info,
                                UiMessage::localized(
                                    "正在更新运行状态，请稍候。",
                                    "Updating runtime status. Please wait.",
                                ),
                            )),
                        }
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        None
                    }
                };
                cx.notify();
                notification.map(|(notification_type, message)| {
                    (notification_type, message.text(this.language))
                })
            });
            if let Ok(Some((notification_type, message))) = notification {
                push_notification(cx, window_handle, notification_type, message);
            }
            cx.refresh();
        })
        .detach();
    }
}

pub(super) fn is_connection_error(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Unavailable(_) | ClientError::Transport(_) | ClientError::StreamEnded
    )
}
