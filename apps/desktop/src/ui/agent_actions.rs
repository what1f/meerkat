use super::*;

impl LankideApp {
    pub(super) fn reload_agents(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected_workspace_id.clone() else {
            self.agents.clear();
            return;
        };
        self.loading = true;
        let client = self.client.clone();
        self.agent_request += 1;
        let request = self.agent_request;
        let epoch = self.selection_epoch;
        cx.spawn(async move |this, cx| {
            let result = client.list_agents(&workspace_id).await;
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.agent_request != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(agents) => {
                        this.agents = agents;
                        this.reload_images(cx);
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                this.finish_boot(cx);
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn reload_images(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace() else {
            return;
        };
        let workspace_id = workspace.id.clone();
        let avatars = self
            .agents
            .iter()
            .filter_map(|agent| {
                if let Avatar::Path(path) = &agent.avatar {
                    Some((path.clone(), agent.id.clone()))
                } else {
                    None
                }
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        self.images.retain(|path, _| avatars.contains_key(path));
        let missing = avatars
            .into_iter()
            .filter(|(path, _)| !self.images.contains_key(path))
            .collect::<Vec<_>>();
        self.image_request += 1;
        let request = self.image_request;
        let epoch = self.selection_epoch;
        if missing.is_empty() {
            return;
        }
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let mut loaded = Vec::new();
            let mut failed = false;
            for (path, agent_id) in missing {
                match client.get_agent_avatar(&workspace_id, &agent_id).await {
                    Ok(image) => {
                        if image.path != path {
                            failed = true;
                            continue;
                        }
                        let decoded = (|| {
                            let mut reader =
                                image::ImageReader::new(std::io::Cursor::new(image.bytes))
                                    .with_guessed_format()
                                    .ok()?;
                            let mut limits = image::Limits::default();
                            limits.max_image_width = Some(4096);
                            limits.max_image_height = Some(4096);
                            limits.max_alloc = Some(64 * 1024 * 1024);
                            reader.limits(limits);
                            let pixels = convert_rgba_to_bgra(
                                reader.decode().ok()?.thumbnail(1200, 1200).into_rgba8(),
                            );
                            Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
                        })();
                        if let Some(image) = decoded {
                            loaded.push((path, image));
                        } else {
                            failed = true;
                        }
                    }
                    Err(_) => failed = true,
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.image_request != request {
                    return;
                }
                this.images.extend(loaded);
                if failed && this.error.is_none() {
                    this.error = Some(UiMessage::localized(
                        "部分图片未能加载，请刷新重试。",
                        "Some images could not load. Refresh to try again.",
                    ));
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn agent_avatar(&self, agent: &Agent, size: f32) -> DesktopAvatar {
        let image = if let Avatar::Path(path) = &agent.avatar {
            self.images.get(path).cloned()
        } else {
            None
        };
        DesktopAvatar::new(agent.name.clone(), self.theme())
            .size(px(size))
            .when_some(image, |avatar, image| avatar.src(image))
    }

    pub(super) fn update_thinking_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let levels: Vec<String> = match selected_harness_id(&self.agent_harness, cx).as_str() {
            "pi" => vec!["off", "minimal", "low", "medium", "high", "xhigh"],
            "claude_code" => vec!["off", "low", "medium", "high", "xhigh", "max"],
            "codex" => vec!["none", "minimal", "low", "medium", "high", "xhigh"],
            _ => vec![],
        }
        .into_iter()
        .map(str::to_owned)
        .collect();
        let current = selected_thinking_level(&self.agent_thinking, cx);
        let selected = if levels.contains(&current) {
            Some(current)
        } else {
            levels.first().cloned()
        };
        self.agent_thinking.update(cx, |select, cx| {
            select.set_items(
                levels
                    .iter()
                    .map(|level| ThinkingChoice::new(level))
                    .collect(),
                window,
                cx,
            );
            if let Some(selected) = selected {
                select.set_selected_value(&selected, window, cx);
            }
        });
    }

    pub(super) fn load_reaction_catalog(&mut self, cx: &mut Context<Self>) {
        self.reaction_request += 1;
        let request = self.reaction_request;
        let epoch = self.selection_epoch;
        let workspace_id = self.selected_workspace_id.clone();
        let client = self.client.clone();
        self.reaction_loading = true;
        self.reaction_error = None;
        cx.spawn(async move |this, cx| {
            let result = client.get_reaction_catalog(workspace_id.as_deref()).await;
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.reaction_request != request {
                    return;
                }
                this.reaction_loading = false;
                match result {
                    Ok(options) => {
                        this.reaction_options = options;
                        this.reaction_error = None;
                    }
                    Err(error) => {
                        this.reaction_options.clear();
                        this.reaction_error = Some(UiMessage::client(error));
                    }
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn reset_agent_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.agent_id_input, "", window, cx);
        set_input(&self.agent_name_input, "", window, cx);
        set_input(&self.agent_role_input, "", window, cx);
        set_input(&self.agent_bio_input, "", window, cx);
        set_textarea(&self.agent_soul_input, "", window, cx);
        self.agent_environment_rows = vec![new_environment_row("", "", window, cx)];
        set_input(&self.agent_model_input, "", window, cx);
        set_input(&self.agent_cli_input, "", window, cx);
        set_input(&self.agent_provider_handle_input, "", window, cx);
        set_input(&self.agent_secret_input, "", window, cx);
        self.agent_harness.update(cx, |select, cx| {
            select.set_selected_value(&"pi".to_string(), window, cx)
        });
        self.agent_thinking.update(cx, |select, cx| {
            select.set_selected_value(&"high".to_string(), window, cx)
        });
        self.agent_reaction = None;
        self.advanced_open = false;
        self.agent_mode_new = true;
        self.agent_provider_handle_edited = false;
        self.agent_id_error = None;
        self.agent_name_error = None;
        self.agent_model_error = None;
        self.agent_provider_handle_error = None;
        self.agent_secret_error = None;
        self.error = None;
    }

    pub(super) fn open_create_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.reset_agent_form(window, cx);
        self.reload_provider_status(cx);
        self.modal = Some(Modal::CreateAgent);
        self.update_thinking_choices(window, cx);
        cx.notify();
    }

    pub(super) fn add_environment_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_environment_rows
            .push(new_environment_row("", "", window, cx));
        cx.notify();
    }

    pub(super) fn remove_environment_variable(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.agent_environment_rows.len() {
            self.agent_environment_rows.remove(index);
            cx.notify();
        }
    }

    pub(super) fn open_edit_agent(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let Some(agent) = self.agents.iter().find(|agent| agent.id == id).cloned() else {
            return;
        };
        set_input(&self.agent_id_input, &agent.id, window, cx);
        set_input(&self.agent_name_input, &agent.name, window, cx);
        set_input(&self.agent_role_input, &agent.role, window, cx);
        set_input(&self.agent_bio_input, &agent.bio, window, cx);
        set_textarea(&self.agent_soul_input, &agent.soul, window, cx);
        self.agent_environment_rows = if agent.environment_variables.is_empty() {
            vec![new_environment_row("", "", window, cx)]
        } else {
            agent
                .environment_variables
                .iter()
                .map(|(key, value)| new_environment_row(key, value, window, cx))
                .collect()
        };
        set_input(&self.agent_model_input, &agent.model, window, cx);
        set_input(&self.agent_cli_input, &agent.cli_path, window, cx);
        set_input(
            &self.agent_provider_handle_input,
            &agent.provider_handle,
            window,
            cx,
        );
        self.agent_harness.update(cx, |select, cx| {
            select.set_selected_value(&agent.harness_id, window, cx)
        });
        self.agent_thinking.update(cx, |select, cx| {
            select.set_selected_value(&agent.thinking_level.label().to_string(), window, cx)
        });
        self.agent_reaction = agent.auto_reaction.clone();
        self.pending_agent_avatar = None;
        self.agent_provider_handle_edited = true;
        self.agent_id_error = None;
        self.agent_name_error = None;
        self.agent_model_error = None;
        self.agent_provider_handle_error = None;
        self.agent_secret_error = None;
        self.load_reaction_catalog(cx);
        self.advanced_open = false;
        self.error = None;
        self.modal = Some(Modal::EditAgent(id));
        let client = self.client.clone();
        let workspace_id = agent.workspace_id.clone();
        let agent_id = agent.id.clone();
        cx.spawn(async move |this, cx| {
            let state = client.get_agent_onboarding(&workspace_id, &agent_id).await;
            let _ = this.update(cx, |this, cx| {
                match state {
                    Ok(state) => this.record_agent_onboarding_state(&state),
                    Err(_) => {}
                }
                if matches!(&this.modal, Some(Modal::EditAgent(id)) if id == &agent_id) {
                    this.notify_dialog_changed(cx);
                }
            });
            cx.refresh();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_delete_agent(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() || !self.agents.iter().any(|agent| agent.id == id) {
            return;
        }
        self.error = None;
        self.modal = Some(Modal::DeleteAgent(id));
        cx.notify();
    }

    pub(super) fn delete_agent(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace_id) = self.selected_workspace_id.clone() else {
            return;
        };
        self.busy = Some(BusyOperation::DeleteAgent);
        self.error = None;
        self.notify_dialog_changed(cx);
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client.delete_agent(&workspace_id, &id).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => {
                        // Invalidate any list response that was started before the
                        // deletion completed. Otherwise a stale response can restore
                        // the deleted Agent in the cards after we remove it locally.
                        this.agent_request += 1;
                        this.agents.retain(|agent| agent.id != id);
                        this.modal = None;
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn choose_agent_avatar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.language.text("选择", "Choose").into()),
        });
        let app = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let Some(path) = selection
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next())
            else {
                return;
            };
            let result = (|| {
                let bytes = std::fs::read(&path).map_err(|_| ())?;
                if bytes.is_empty() || bytes.len() > MAX_AGENT_AVATAR_BYTES {
                    return Err(());
                }
                let format = image::guess_format(&bytes).map_err(|_| ())?;
                let media_type = match format {
                    image::ImageFormat::Png => "image/png",
                    image::ImageFormat::Jpeg => "image/jpeg",
                    image::ImageFormat::WebP => "image/webp",
                    image::ImageFormat::Gif => "image/gif",
                    _ => return Err(()),
                };
                let pixels = convert_rgba_to_bgra(
                    image::load_from_memory_with_format(&bytes, format)
                        .map_err(|_| ())?
                        .to_rgba8(),
                );
                Ok(PendingAgentAvatar {
                    media_type: media_type.into(),
                    bytes,
                    image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
                })
            })();
            let _ = window.update(|_, cx| {
                let _ = app.update(cx, |this, cx| {
                    match result {
                        Ok(avatar) => {
                            this.pending_agent_avatar = Some(avatar);
                            this.error = None;
                        }
                        Err(()) => {
                            this.error = Some(UiMessage::localized(
                                "请选择不超过 3 MB 的 PNG、JPEG、WebP 或 GIF 图片。",
                                "Choose a PNG, JPEG, WebP, or GIF image up to 3 MB.",
                            ));
                        }
                    }
                    this.notify_dialog_changed(cx);
                });
            });
        })
        .detach();
    }

    pub(super) fn submit_agent(&mut self, editing: Option<String>, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        let workspace_id = workspace.id.clone();
        let id = input_value(&self.agent_id_input, cx);
        let name = input_value(&self.agent_name_input, cx);
        let model = input_value(&self.agent_model_input, cx);
        let harness_id = selected_harness_id(&self.agent_harness, cx);
        let provider_handle = input_value(&self.agent_provider_handle_input, cx);
        let provider_secret = input_value(&self.agent_secret_input, cx);
        self.agent_id_error = if id.trim().is_empty() {
            Some(UiMessage::localized(
                "请填写 Agent ID。",
                "Enter an Agent ID.",
            ))
        } else if !valid_id(&id) {
            Some(UiMessage::localized(
                "只能使用小写字母、数字和下划线。",
                "Use lowercase letters, digits and underscores only.",
            ))
        } else {
            None
        };
        self.agent_name_error = name
            .trim()
            .is_empty()
            .then(|| UiMessage::localized("请填写 Agent 名称。", "Enter an Agent name."));
        self.agent_model_error = if model.trim().is_empty() {
            Some(UiMessage::localized(
                "请填写模型名称。",
                "Enter a model name.",
            ))
        } else if harness_id == "pi"
            && model.trim().starts_with("gpt-")
            && !model.trim().contains('/')
        {
            Some(UiMessage::localized(
                format!("请填写完整模型 ID，例如 openai-codex/{}。", model.trim()),
                format!(
                    "Enter the complete model ID, for example openai-codex/{}.",
                    model.trim()
                ),
            ))
        } else {
            None
        };
        self.agent_provider_handle_error = None;
        self.agent_secret_error = None;
        if editing.is_none() && !self.agent_mode_new {
            if workspace.provider.id == "feishu" && provider_handle.trim().is_empty() {
                self.agent_provider_handle_error =
                    Some(UiMessage::localized("请填写 App ID。", "Enter an App ID."));
            }
            if provider_secret.trim().is_empty() {
                self.agent_secret_error = Some(if workspace.provider.id == "telegram" {
                    UiMessage::localized("请填写 Bot Token。", "Enter a Bot Token.")
                } else {
                    UiMessage::localized("请填写 App Secret。", "Enter an App Secret.")
                });
            }
        }
        if self.agent_id_error.is_some()
            || self.agent_name_error.is_some()
            || self.agent_model_error.is_some()
            || self.agent_provider_handle_error.is_some()
            || self.agent_secret_error.is_some()
        {
            self.error = None;
            cx.notify();
            return;
        }
        let thinking_level =
            match ThinkingLevel::parse(&selected_thinking_level(&self.agent_thinking, cx)) {
                Ok(level) => level,
                Err(error) => {
                    self.error = Some(UiMessage::client(error));
                    cx.notify();
                    return;
                }
            };
        let auto_reaction = self.agent_reaction.clone();
        let role = input_value(&self.agent_role_input, cx);
        let bio = input_value(&self.agent_bio_input, cx);
        let soul = textarea_value(&self.agent_soul_input, cx);
        let cli_path = input_value(&self.agent_cli_input, cx);
        let environment_variables =
            match collect_environment_variables(&self.agent_environment_rows, cx) {
                Ok(values) => values,
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            };
        let client = self.client.clone();
        self.busy = Some(BusyOperation::SaveAgent);
        self.error = None;
        self.notify_dialog_changed(cx);

        if let Some(editing_id) = editing {
            let Some(mut agent) = self
                .agents
                .iter()
                .find(|agent| agent.id == editing_id)
                .cloned()
            else {
                return;
            };
            agent.name = name.trim().into();
            agent.role = role;
            agent.bio = bio;
            agent.soul = soul;
            agent.harness_id = harness_id;
            agent.model = model;
            agent.thinking_level = thinking_level;
            agent.auto_reaction = auto_reaction;
            agent.cli_path = cli_path;
            agent.environment_variables = environment_variables;
            agent.provider_handle = provider_handle;
            let pending_avatar = self.pending_agent_avatar.clone();
            let workspace_id = agent.workspace_id.clone();
            let retry_agent_id = editing_id.clone();
            let check_feishu_permissions = workspace.provider.id == "feishu";
            cx.spawn(async move |this, cx| {
                let result = async {
                    // Provider identity is established by onboarding and is not an
                    // editable profile field. Refresh it before saving so a stale
                    // dialog cannot erase a connection while changing the model.
                    let current = client.get_agent(&workspace_id, &editing_id).await?;
                    agent.provider_handle = current.provider_handle;
                    agent.provider_agent_config = current.provider_agent_config;
                    let (upload, uploaded) = if let Some(avatar) = pending_avatar {
                        (
                            Some(AgentAvatarUpload {
                                media_type: avatar.media_type,
                                bytes: avatar.bytes,
                            }),
                            Some(avatar.image),
                        )
                    } else {
                        (None, None)
                    };
                    client
                        .update_agent(agent, upload)
                        .await
                        .map(|agent| (agent, uploaded))
                }
                .await;
                let onboarding = if check_feishu_permissions
                    && matches!(&result, Err(ClientError::Provider(_)))
                {
                    client
                        .get_agent_onboarding(&workspace_id, &retry_agent_id)
                        .await
                        .ok()
                } else {
                    None
                };
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok((updated, uploaded)) => {
                            let updated_id = updated.id.clone();
                            if let Some(image) = uploaded
                                && let Avatar::Path(path) = &updated.avatar
                            {
                                this.images.insert(path.clone(), image);
                            }
                            if let Some(current) =
                                this.agents.iter_mut().find(|a| a.id == updated.id)
                            {
                                *current = updated;
                            }
                            this.pending_agent_avatar = None;
                            if this.pending_agent_save_retry.as_deref() == Some(updated_id.as_str()) {
                                this.pending_agent_save_retry = None;
                            }
                            this.modal = None;
                        }
                        Err(error) => {
                            if let Some(state) = onboarding.filter(|state| {
                                state.step == "authorization_update"
                                    && state.status != OnboardingStatus::Ready
                            }) {
                                this.record_agent_onboarding_state(&state);
                                this.pending_agent_save_retry = Some(retry_agent_id);
                                this.error = Some(UiMessage::localized(
                                    "需要先更新飞书权限，完成后将自动重试保存。",
                                    "Update Feishu permissions first. Saving will retry automatically when it finishes.",
                                ));
                            } else {
                                this.error = Some(UiMessage::client(error));
                            }
                        }
                    }
                    cx.notify();
                });
                cx.refresh();
            })
            .detach();
        } else {
            let avatar = Avatar::Custom(name.clone());
            let provider_id = workspace.provider.id.as_str();
            let mut provider_handle = provider_handle;
            if self.agent_mode_new && provider_id == "telegram" && provider_handle.trim().is_empty()
            {
                provider_handle = telegram_bot_username(&workspace.id, &id);
            }
            let mut provider_agent_config = json!({
                "mode": if self.agent_mode_new { "new" } else { "existing" },
            });
            provider_agent_config["provider_handle"] = json!(provider_handle);
            let provider_agent_secrets = if self.agent_mode_new {
                json!({})
            } else if provider_id == "telegram" {
                json!({"bot_token": provider_secret})
            } else {
                json!({"app_secret": provider_secret})
            };
            let draft = AgentDraft {
                id,
                name: name.trim().into(),
                role,
                bio,
                avatar,
                soul,
                computer_id: "local".into(),
                harness_id,
                model,
                thinking_level,
                auto_reaction,
                cli_path,
                environment_variables,
                provider_handle,
                provider_agent_config,
                provider_agent_secrets,
            };
            let mode_new = self.agent_mode_new;
            cx.spawn(async move |this, cx| {
                let result = client.create_agent(&workspace_id, draft).await;
                let onboarding = match &result {
                    Ok(agent) if mode_new => Some(client
                        .advance_agent_onboarding(
                            &agent.workspace_id,
                            &agent.id,
                            "start",
                            json!({}),
                        )
                        .await),
                    Ok(agent) => Some(client
                        .get_agent_onboarding(&agent.workspace_id, &agent.id)
                        .await),
                    Err(_) => None,
                };
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok(created) => {
                            let retry_key = (created.workspace_id.clone(), created.id.clone());
                            this.agents.push(created);
                            match onboarding {
                                Some(Ok(state)) => {
                                    this.record_agent_onboarding_state(&state);
                                    if state.status != OnboardingStatus::Ready {
                                        this.modal = Some(Modal::AgentOnboarding(state.clone()));
                                        this.schedule_agent_onboarding_poll(state, cx);
                                    } else {
                                        this.modal = None;
                                    }
                                }
                                Some(Err(error)) => {
                                    this.onboarding_retries.insert(retry_key);
                                    this.modal = None;
                                    this.error = Some(UiMessage::localized(
                                        format!(
                                            "Agent 已创建，但尚未连接。请在 Agent 编辑页完成连接。 {}",
                                            friendly_error(&error, DesktopLanguage::SimplifiedChinese)
                                        ),
                                        format!(
                                            "Agent created, but it is not connected yet. Open the Agent editor to connect it. {}",
                                            friendly_error(&error, DesktopLanguage::English)
                                        ),
                                    ));
                                }
                                None => unreachable!("successful creation always checks onboarding"),
                            }
                        }
                        Err(ClientError::DuplicateAgentId(_)) => {
                            this.agent_id_error = Some(UiMessage::localized(
                                "这个 Agent ID 已经被使用。",
                                "This Agent ID is already in use.",
                            ));
                            this.error = None;
                        }
                        Err(ClientError::InvalidAgentId) => {
                            this.agent_id_error = Some(UiMessage::localized(
                                "只能使用小写字母、数字和下划线。",
                                "Use lowercase letters, digits and underscores only.",
                            ));
                            this.error = None;
                        }
                        Err(error) => this.error = Some(UiMessage::client(error)),
                    }
                    cx.notify();
                });
                cx.refresh();
            })
            .detach();
        }
    }
}

pub(super) fn new_environment_row(
    key: &str,
    value: &str,
    window: &mut Window,
    cx: &mut Context<LankideApp>,
) -> AgentEnvironmentRow {
    AgentEnvironmentRow {
        key: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("KEY")
                .default_value(key.to_string())
        }),
        value: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("VALUE")
                .default_value(value.to_string())
        }),
    }
}

pub(super) fn selected_harness_id(
    select: &Entity<SelectState<Vec<HarnessChoice>>>,
    cx: &App,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| "pi".into())
}

fn selected_thinking_level(select: &Entity<SelectState<Vec<ThinkingChoice>>>, cx: &App) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| "high".into())
}

fn collect_environment_variables(
    rows: &[AgentEnvironmentRow],
    cx: &App,
) -> Result<Vec<(String, String)>, UiMessage> {
    let mut values = Vec::new();
    for row in rows {
        let key = input_value(&row.key, cx);
        let value = input_value(&row.value, cx);
        let key = key.trim();
        if key.is_empty() && value.trim().is_empty() {
            continue;
        }
        if key.is_empty() {
            return Err(UiMessage::localized(
                "请填写环境变量名称。",
                "Enter an environment variable name.",
            ));
        }
        if key.chars().any(char::is_whitespace) || key.contains('=') {
            return Err(UiMessage::localized(
                "环境变量名称不能包含空格或等号。",
                "Environment variable names cannot contain spaces or equals signs.",
            ));
        }
        values.push((key.to_string(), value));
    }
    Ok(values)
}
