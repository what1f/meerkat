//! Workspace, Agent and provider onboarding dialogs.

use super::*;
use gpui_component::scroll::ScrollableElement as _;

impl LankideApp {
    pub(super) fn sync_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() && !self.modal_presented {
            self.modal_presented = true;
            let app = cx.entity();
            cx.defer_in(window, move |_, window, cx| {
                window.open_dialog(cx, move |dialog, window, cx| {
                    let app_for_render = app.clone();
                    app.read(cx)
                        .render_modal(dialog, app_for_render, window, cx)
                });
                // The dialog layer is composed by LankideApp rather than Root.
                // Opening the Root dialog only invalidates Root, so invalidate
                // this view once more after the deferred open has installed it.
                cx.notify();
                cx.refresh_windows();
            });
        } else if self.modal.is_none() {
            self.modal_presented = false;
            if window.has_active_dialog(cx) {
                cx.defer_in(window, move |_, window, cx| {
                    window.close_dialog(cx);
                    cx.notify();
                    cx.refresh_windows();
                });
            }
        }
    }

    fn render_modal(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<LankideApp>,
        window: &mut Window,
        cx: &App,
    ) -> gpui_component::dialog::Dialog {
        match self.modal.clone() {
            Some(Modal::CreateWorkspace { step }) => {
                self.workspace_dialog(dialog, app, step, window, cx)
            }
            Some(Modal::RemoveMachine(id)) => self.remove_machine_dialog(dialog, app, id, window),
            Some(Modal::PairMachine) => self.pair_machine_dialog(dialog, app, window),
            Some(Modal::CreateAgent) => self.agent_dialog(dialog, app, None, window, cx),
            Some(Modal::EditAgent(id)) => self.agent_dialog(dialog, app, Some(id), window, cx),
            Some(Modal::DeleteAgent(id)) => self.delete_agent_dialog(dialog, app, id, window),
            Some(Modal::AgentOnboarding(state)) => {
                self.agent_onboarding_dialog(dialog, app, state, window, cx)
            }
            Some(Modal::FeishuSignIn(state)) => {
                self.feishu_sign_in_dialog(dialog, app, state, window)
            }
            None => DesktopDialog::from(dialog)
                .w(px(0.))
                .h(px(0.))
                .overlay(false),
        }
    }

    fn delete_agent_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<LankideApp>,
        id: String,
        _window: &mut Window,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let busy = self.busy == Some(BusyOperation::DeleteAgent);
        let theme = self.theme();
        let error = self.error.as_ref().map(|message| message.text(language));
        let name = self
            .agents
            .iter()
            .find(|agent| agent.id == id)
            .map(|agent| agent.name.clone())
            .unwrap_or(id.clone());
        let title = match language {
            DesktopLanguage::SimplifiedChinese => format!("删除 {name}？"),
            DesktopLanguage::English => format!("Delete {name}?"),
        };
        let cancel_app = app.clone();
        let delete_app = app.clone();
        let id_for_delete = id.clone();
        DesktopDialog::from(dialog)
            .w(px(440.))
            .h(px(if error.is_some() { 260. } else { 200. }))
            .overlay(true)
            .overlay_closable(!busy)
            .on_cancel(move |_, _, cx| {
                let _ = cancel_app.update(cx, |this, cx| {
                    if this.busy.is_none() {
                        this.modal = None;
                        this.error = None;
                        cx.notify();
                    }
                });
                !busy
            })
            .content(move |content, window, _| {
                let delete_id = id_for_delete.clone();
                content
                    .child(DesktopDialogHeader::new(title.clone()).closable(!busy))
                    .child(
                        div()
                            .flex_none()
                            .px(px(22.))
                            .text_color(theme.text_secondary)
                            .child(language.text(
                                "删除后无法恢复，历史记录会保留。",
                                "This cannot be undone. History will be kept.",
                            )),
                    )
                    .when_some(error.clone(), |content, error| {
                        content.child(div().px(px(22.)).child(DesktopBanner::error(theme, error)))
                    })
                    .child(
                        DesktopDialogFooter::new()
                            .child(desktop_dialog_cancel(
                                "cancel-delete-agent",
                                language.text("取消", "Cancel"),
                            ))
                            .child(desktop_dialog_button(
                                "confirm-delete-agent",
                                DesktopButton::primary_loading(
                                    "confirm-delete-agent",
                                    language.text("删除", "Delete"),
                                    busy,
                                )
                                .on_click(window.listener_for(
                                    &delete_app,
                                    move |this, _, _, cx| {
                                        this.delete_agent(delete_id.clone(), cx);
                                    },
                                )),
                            )),
                    )
            })
    }

    fn workspace_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<LankideApp>,
        step: usize,
        window: &mut Window,
        _cx: &App,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let theme = self.theme();
        let estimated_height = px(if step == 0 {
            330.
        } else if self.workspace_provider == "telegram" {
            425.
        } else {
            405.
        });
        let centered_top = ((window.viewport_size().height - estimated_height) * 0.5).max(px(72.));
        let cancel_app = app.clone();
        let body_app = app.clone();
        let action_app = app.clone();
        let id_input = self.workspace_id_input.clone();
        let name_input = self.workspace_name_input.clone();
        let manager_token_input = self.workspace_manager_token_input.clone();
        let provider = self.workspace_provider.clone();
        let task_system_supported =
            self.provider_supports_capability(&provider, PROVIDER_CAPABILITY_TASK_SYSTEM);
        let task_enabled = self.workspace_task_enabled;
        let busy = self.busy == Some(BusyOperation::CreateWorkspace);
        let error = self.error.as_ref().map(|message| message.text(language));
        let id_error = self
            .workspace_id_error
            .as_ref()
            .map(|message| message.text(language));
        let name_error = self
            .workspace_name_error
            .as_ref()
            .map(|message| message.text(language));
        let token_error = self
            .workspace_token_error
            .as_ref()
            .map(|message| message.text(language));
        DesktopDialog::from(dialog)
            .w(px(680.))
            .h(estimated_height)
            .margin_top(px(0.))
            .mt(centered_top)
            .overlay(true)
            .overlay_closable(!busy)
            .on_cancel(move |_, _, cx| {
                let _ = cancel_app.update(cx, |this, cx| {
                    if this.busy.is_none() {
                        this.modal = None;
                        this.error = None;
                        cx.notify();
                    }
                });
                !busy
            })
            .content(move |content, window, _| {
                let mut body = div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px(px(22.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(18.));
                if step > 0
                    && let Some(error) = &error
                {
                    body = body.child(DesktopBanner::error(theme, error));
                }
                if step == 0 {
                    body = body.child(
                        div()
                            .flex()
                            .gap(px(14.))
                            .child(
                                div().flex_1().child(
                                    DesktopFormField::new(
                                        theme,
                                        "Workspace ID",
                                        div()
                                            .id("workspace-id-input")
                                            .debug_selector(|| "workspace-id-input".into())
                                            .child(DesktopInput::mono_large(&id_input)),
                                    )
                                    .error(id_error.clone()),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    DesktopFormField::new(
                                        theme,
                                        language.text("工作区名称", "Workspace name"),
                                        div()
                                            .id("workspace-name-input")
                                            .debug_selector(|| "workspace-name-input".into())
                                            .child(DesktopInput::large(&name_input)),
                                    )
                                    .error(name_error.clone()),
                                ),
                            ),
                    );
                } else {
                    let feishu_app = body_app.clone();
                    let telegram_app = body_app.clone();
                    body = body
                        .child(div().font_weight(FontWeight::MEDIUM).child(language.text("选择 IM", "Choose a chat service")))
                        .child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .child(
                                    provider_tile(
                                        "provider-feishu",
                                        color_icon("icons/feishu.svg", 22.),
                                        language.text("飞书", "Feishu"),
                                        language.text("群聊、私聊与任务协作", "Group chats, direct messages and tasks"),
                                        provider == "feishu",
                                        theme,
                                    )
                                    .on_click(
                                        window.listener_for(&feishu_app, |this, _, _window, cx| {
                                            this.select_workspace_provider("feishu", cx);
                                        }),
                                    ),
                                )
                                .child(
                                    provider_tile(
                                        "provider-telegram",
                                        color_icon("icons/telegram.svg", 22.),
                                        "Telegram",
                                        language.text("群聊与私聊 Bot", "Bots for group chats and direct messages"),
                                        provider == "telegram",
                                        theme,
                                    )
                                    .on_click(
                                        window.listener_for(&telegram_app, |this, _, _window, cx| {
                                            this.select_workspace_provider("telegram", cx);
                                        }),
                                    ),
                                ),
                        )
                        .when(task_system_supported, |body| {
                            body.child(
                                div().p(px(14.)).rounded(px(14.)).bg(theme.raised).child(
                                    DesktopCheckbox::new("create-task-system", language.text("启用任务系统", "Enable tasks"))
                                        .checked(task_enabled)
                                        .on_click(window.listener_for(
                                            &body_app,
                                            |this, checked: &bool, _window, cx| {
                                                this.workspace_task_enabled = *checked;
                                                this.notify_dialog_changed(cx);
                                            },
                                        )),
                                ),
                            )
                        })
                        .when(provider == "telegram", |body| {
                            body.child(
                                DesktopFormField::new(
                                    theme,
                                    "Manager Bot Token",
                                    DesktopInput::large(&manager_token_input)
                                        .content_type(InputContentType::Password)
                                        .mask_toggle(),
                                )
                                .error(token_error.clone())
                                .hint(language.text("连接已开启 Bot Management Mode 的 Manager Bot。", "Connect a Manager Bot with Bot Management Mode enabled.")),
                            )
                        });
                }
                content
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        DesktopDialogHeader::new(language.text("新建工作区", "New workspace"))
                            .closable(!busy)
                            .child(DesktopSteps::new(theme, &[language.text("基本信息", "Basics"), language.text("协作方式", "Collaboration")], step)),
                    )
                    .child(body)
                    .child(
                        DesktopDialogFooter::new()
                            .child(desktop_dialog_cancel("cancel-workspace", language.text("取消", "Cancel")))
                            .when(step > 0, |footer| {
                                let back_app = action_app.clone();
                                footer.child(desktop_dialog_button(
                                    "workspace-back",
                                    DesktopButton::outline("workspace-back", language.text("上一步", "Back")).on_click(
                                        window.listener_for(&back_app, move |this, _, _window, cx| {
                                            this.modal = Some(Modal::CreateWorkspace { step: 0 });
                                            this.error = None;
                                            this.notify_dialog_changed(cx);
                                        }),
                                    ),
                                ))
                            })
                            .child(if step == 0 {
                                desktop_dialog_button(
                                    "workspace-next",
                                    DesktopButton::primary("workspace-next", language.text("继续", "Continue")).on_click(
                                        window.listener_for(&action_app, move |this, _, _window, cx| {
                                            let id = input_value(&this.workspace_id_input, cx);
                                            let name = input_value(&this.workspace_name_input, cx);
                                            this.workspace_id_error = if id.trim().is_empty() {
                                                Some(UiMessage::localized("请填写 Workspace ID。", "Enter a Workspace ID."))
                                            } else if !valid_id(&id) {
                                                Some(UiMessage::localized("只能使用小写字母、数字和下划线。", "Use lowercase letters, digits and underscores only."))
                                            } else {
                                                None
                                            };
                                            this.workspace_name_error = if name.trim().is_empty() {
                                                Some(UiMessage::localized("请填写工作区名称。", "Enter a workspace name."))
                                            } else {
                                                None
                                            };
                                            if this.workspace_id_error.is_none()
                                                && this.workspace_name_error.is_none()
                                            {
                                                this.error = None;
                                                this.modal =
                                                    Some(Modal::CreateWorkspace { step: 1 });
                                            }
                                            this.notify_dialog_changed(cx);
                                        }),
                                    ),
                                )
                                .into_any_element()
                            } else {
                                desktop_dialog_button(
                                    "create-workspace-submit",
                                    DesktopButton::primary_loading(
                                        "create-workspace-submit",
                                        language.text("创建工作区", "Create workspace"),
                                        busy,
                                    )
                                        .on_click(
                                            window.listener_for(&action_app, move |this, _, window, cx| {
                                                this.submit_workspace(window.window_handle(), cx)
                                            }),
                                        ),
                                )
                                .into_any_element()
                            }),
                    )
            })
    }

    fn agent_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<LankideApp>,
        editing: Option<String>,
        window: &mut Window,
        cx: &App,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let theme = self.theme();
        let cancel_app = app.clone();
        let editing_for_cancel = editing.clone();
        let save_app = app.clone();
        let advanced_app = app.clone();
        let id_input = self.agent_id_input.clone();
        let name_input = self.agent_name_input.clone();
        let role_input = self.agent_role_input.clone();
        let bio_input = self.agent_bio_input.clone();
        let soul_input = self.agent_soul_input.clone();
        let environment_rows = self.agent_environment_rows.clone();
        let model_input = self.agent_model_input.clone();
        let cli_input = self.agent_cli_input.clone();
        let provider_handle_input = self.agent_provider_handle_input.clone();
        let secret_input = self.agent_secret_input.clone();
        let machine = self.agent_machine.clone();
        let harness = self.agent_harness.clone();
        let thinking = self.agent_thinking.clone();
        let reaction_options = self.reaction_options.clone();
        let selected_reaction = self.agent_reaction.clone();
        let reaction_error = self
            .reaction_error
            .as_ref()
            .map(|message| message.text(language));
        // Preserve the current choice while refreshing Provider-supported reactions.
        let reaction_loading = self.reaction_loading && self.reaction_options.is_empty();
        let is_edit = editing.is_some();
        let provider_id = self
            .selected_workspace()
            .map(|workspace| workspace.provider.id.clone())
            .unwrap_or_else(|| "feishu".into());
        let manager_ready = self.telegram_manager_ready;
        let mode_new = self.agent_mode_new;
        let advanced_open = self.advanced_open;
        let dialog_busy = self.busy.is_some();
        let save_busy = self.busy == Some(BusyOperation::SaveAgent);
        let connect_busy = self.busy == Some(BusyOperation::ResumeAgentOnboarding);
        let error = self.error.as_ref().map(|message| message.text(language));
        let id_error = self
            .agent_id_error
            .as_ref()
            .map(|message| message.text(language));
        let name_error = self
            .agent_name_error
            .as_ref()
            .map(|message| message.text(language));
        let model_error = self
            .agent_model_error
            .as_ref()
            .map(|message| message.text(language));
        let provider_handle_error = self
            .agent_provider_handle_error
            .as_ref()
            .map(|message| message.text(language));
        let secret_error = self
            .agent_secret_error
            .as_ref()
            .map(|message| message.text(language));
        let preview_agent = editing
            .as_ref()
            .and_then(|id| self.agents.iter().find(|agent| &agent.id == id));
        let preview_avatar = self
            .pending_agent_avatar
            .as_ref()
            .map(|avatar| avatar.image.clone())
            .or_else(|| {
                preview_agent.and_then(|agent| match agent.avatar {
                    Avatar::Path(ref path) => self.images.get(path).cloned(),
                    Avatar::Custom(_) => None,
                })
            });
        let connection_needs_attention = preview_agent.is_some_and(|agent| {
            self.onboarding_retries
                .contains(&(agent.workspace_id.clone(), agent.id.clone()))
        });
        let permission_update_needed = preview_agent.is_some_and(|agent| {
            provider_id == "feishu"
                && self
                    .onboarding_steps
                    .get(&(agent.workspace_id.clone(), agent.id.clone()))
                    .is_some_and(|step| step == "authorization_update")
        });
        let preview_id = preview_agent
            .map(|agent| agent.id.clone())
            .unwrap_or_default();
        let preview_name = input_value(&self.agent_name_input, cx);
        let preview_role = input_value(&self.agent_role_input, cx);
        let preview_bio = input_value(&self.agent_bio_input, cx);
        let preview_model = input_value(&self.agent_model_input, cx);
        let preview_harness = selected_harness_id(&self.agent_harness, cx);
        let desired_dialog_height = px(80.
            + if is_edit && advanced_open {
                850.
            } else if is_edit {
                760.
            } else if provider_id == "telegram" {
                720.
            } else if !mode_new {
                620.
            } else {
                540.
            });
        let viewport = window.viewport_size();
        let dialog_height = desired_dialog_height.min((viewport.height - px(48.)).max(px(480.)));
        let dialog_width =
            px(if is_edit { 980. } else { 820. }).min((viewport.width - px(48.)).max(px(720.)));
        let centered_top = ((viewport.height - dialog_height) * 0.5).max(px(24.));
        DesktopDialog::from(dialog)
            .w(dialog_width)
            .h(dialog_height)
            .margin_top(px(0.))
            .mt(centered_top)
            .overlay(true)
            .overlay_closable(!dialog_busy)
            .on_cancel(move |_, _, cx| {
                let _ = cancel_app.update(cx, |this, cx| {
                    if this.busy.is_none() {
                        if editing_for_cancel.as_deref()
                            == this.pending_agent_save_retry.as_deref()
                        {
                            this.pending_agent_save_retry = None;
                            this.onboarding_return_to_agent = None;
                        }
                        this.modal = None;
                        this.error = None;
                        cx.notify();
                    }
                });
                !dialog_busy
            })
            .content(move |content, window, cx| {
                let reaction_error = reaction_error.clone();
                let editing_for_save = editing.clone();
                let mut form = div().px(px(22.)).pb(px(24.)).flex().flex_col().gap(px(16.));
                if let Some(error) = &error {
                    form = form.child(DesktopBanner::error(theme, error));
                }
                if !is_edit {
                    let new_app = save_app.clone();
                    let existing_app = save_app.clone();
                    form = form.child(DesktopSteps::new(
                        theme,
                        &if provider_id == "telegram" {
                            vec!["Manager Bot", language.text("基本信息", "Basics"), language.text("创建 Bot", "Create Bot"), language.text("Bot 协作", "Bot collaboration"), language.text("启用", "Enable")]
                        } else {
                            vec![language.text("基本信息", "Basics"), language.text("创建应用", "Create app"), language.text("授权能力", "Permissions"), language.text("启用", "Enable")]
                        },
                        if provider_id == "telegram" { 1 } else { 0 },
                    ));
                    if provider_id == "telegram" && mode_new && manager_ready {
                        form = form.child(success_panel(
                            language.text("Manager Bot 已连接", "Manager Bot connected"),
                            language.text("工作区的 Manager Bot 将负责发起 Bot 创建确认。", "Confirm the new Bot through your workspace’s Manager Bot."),
                            theme,
                        ));
                    }
                    form = form
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(theme.text_secondary)
                                .child(language.text("接入方式", "Connection method")),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .child(
                                    provider_tile(
                                        "agent-mode-new",
                                        themed_icon("icons/lucide/sparkles.svg", 22., theme),
                                        language.text("新建机器人", "Create a Bot"),
                                        if provider_id == "telegram" {
                                            language.text("前往 Telegram 确认创建 Bot", "Confirm Bot creation in Telegram")
                                        } else {
                                            language.text("扫码后在飞书完成创建", "Complete creation in Feishu")
                                        },
                                        mode_new,
                                        theme,
                                    )
                                    .on_click(
                                        window.listener_for(&new_app, |this, _, _, cx| {
                                            this.agent_mode_new = true;
                                            this.agent_provider_handle_error = None;
                                            this.agent_secret_error = None;
                                            cx.notify();
                                        }),
                                    ),
                                )
                                .child(
                                    provider_tile(
                                        "agent-mode-existing",
                                        themed_icon("icons/lucide/key.svg", 22., theme),
                                        if provider_id == "telegram" {
                                            language.text("绑定已有 Bot", "Connect an existing Bot")
                                        } else {
                                            language.text("绑定已有应用", "Connect an existing app")
                                        },
                                        if provider_id == "telegram" {
                                            language.text("粘贴 BotFather 提供的 Token", "Paste the token from BotFather")
                                        } else {
                                            language.text("使用 App ID 与 Secret 接入", "Connect with an App ID and Secret")
                                        },
                                        !mode_new,
                                        theme,
                                    )
                                    .on_click(
                                        window.listener_for(&existing_app, |this, _, _, cx| {
                                            this.agent_mode_new = false;
                                            this.agent_provider_handle_error = None;
                                            this.agent_secret_error = None;
                                            cx.notify();
                                        }),
                                    ),
                                ),
                        );
                }
                if is_edit {
                    if connection_needs_attention {
                        let connect_app = save_app.clone();
                        let agent_id = editing.clone().unwrap_or_default();
                        let (connection_title, connection_description, connection_action) =
                            if permission_update_needed {
                                (
                                    language.text(
                                        "需要更新飞书权限",
                                        "Feishu permissions need an update",
                                    ),
                                    language.text(
                                        "更新后即可继续使用完整功能。",
                                        "Update permissions to continue using all features.",
                                    ),
                                    language.text("更新权限", "Update permissions"),
                                )
                            } else if provider_id == "telegram" {
                                (
                                    language.text(
                                        "Telegram 尚未连接",
                                        "Telegram is not connected",
                                    ),
                                    language.text(
                                        "完成连接后，这个 Agent 才能在工作区收发消息。",
                                        "Connect this Agent so it can send and receive workspace messages.",
                                    ),
                                    language.text("连接 Telegram", "Connect Telegram"),
                                )
                            } else {
                                (
                                    language.text("飞书尚未连接", "Feishu is not connected"),
                                    language.text(
                                        "完成连接后，这个 Agent 才能在工作区收发消息。",
                                        "Connect this Agent so it can send and receive workspace messages.",
                                    ),
                                    language.text("连接飞书", "Connect Feishu"),
                                )
                            };
                        form = form.child(
                            div()
                                .p(px(14.))
                                .rounded(px(15.))
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.raised)
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap(px(14.))
                                .child(
                                    div()
                                        .flex_1()
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(connection_title),
                                        )
                                        .child(
                                            div()
                                                .mt(px(3.))
                                                .text_size(px(12.))
                                                .text_color(theme.text_secondary)
                                                .child(connection_description),
                                        ),
                                )
                                .child(
                                    DesktopButton::outline_loading(
                                        if permission_update_needed {
                                            "update-feishu-permissions"
                                        } else {
                                            "connect-agent"
                                        },
                                        connection_action,
                                        connect_busy,
                                    )
                                        .disabled(dialog_busy && !connect_busy)
                                        .flex_none()
                                        .on_click(window.listener_for(
                                            &connect_app,
                                            move |this, _, _, cx| {
                                                this.resume_agent_onboarding(agent_id.clone(), cx);
                                            },
                                        )),
                                ),
                        );
                    }
                    form = form
                        .child(
                            div()
                                .flex()
                                .gap(px(14.))
                                .child(
                                    div().flex_1().child(
                                        DesktopFormField::new(
                                            theme,
                                            language.text("名字", "Name"),
                                            div()
                                                .id("agent-name-input")
                                                .debug_selector(|| "agent-name-input".into())
                                                .child(DesktopInput::large(&name_input)),
                                        )
                                        .error(name_error.clone()),
                                    ),
                                )
                                .child(div().flex_1().child(DesktopFormField::new(
                                    theme,
                                    language.text("岗位", "Role"),
                                    DesktopInput::large(&role_input),
                                ))),
                        )
                        .child(DesktopFormField::new(
                            theme,
                            language.text("签名", "Bio"),
                            DesktopInput::large(&bio_input),
                        ));
                } else {
                    form = form.child(
                        div()
                            .flex()
                            .gap(px(14.))
                            .child(
                                div().flex_1().child(
                                    DesktopFormField::new(
                                        theme,
                                        "Agent ID",
                                        div()
                                            .id("agent-id-input")
                                            .debug_selector(|| "agent-id-input".into())
                                            .child(DesktopInput::mono_large(&id_input)),
                                    )
                                    .error(id_error.clone()),
                                ),
                            )
                            .child(
                                div().flex_1().child(
                                    DesktopFormField::new(
                                        theme,
                                        language.text("显示名称", "Display name"),
                                        div()
                                            .id("agent-name-input")
                                            .debug_selector(|| "agent-name-input".into())
                                            .child(DesktopInput::large(&name_input)),
                                    )
                                    .error(name_error.clone()),
                                ),
                            ),
                    );
                }
                form = form.child(DesktopFormField::new(
                    theme, language.text("机器", "Machine"),
                    div().debug_selector(|| "agent-machine-select".into())
                        .child(DesktopSelect::large(&machine).w_full()),
                ));
                form = form.child(
                    div()
                        .flex()
                        .gap(px(14.))
                        .child(div().w(px(190.)).flex_none().child(DesktopFormField::new(
                            theme,
                            "Agent",
                            DesktopSelect::large(&harness).w_full(),
                        )))
                        .child(
                            div().flex_1().child(
                                DesktopFormField::new(
                                    theme,
                                    language.text("模型", "Model"),
                                    div()
                                        .id("agent-model-input")
                                        .debug_selector(|| "agent-model-input".into())
                                        .child(model_field_input(
                                            &model_input,
                                            &preview_model,
                                            theme,
                                        )),
                                )
                                .error(model_error.clone()),
                            ),
                        )
                        .child(div().w(px(190.)).flex_none().child(DesktopFormField::new(
                            theme,
                            language.text("思考等级", "Reasoning effort"),
                            DesktopSelect::large(&thinking).w_full(),
                        ))),
                );
                if !is_edit {
                    if provider_id == "telegram" && mode_new {
                        form = form.child(
                            DesktopFormField::new(
                                theme,
                                language.text("Bot 用户名", "Bot username"),
                                DesktopInput::mono_large(&provider_handle_input),
                            )
                            .error(provider_handle_error.clone())
                            .hint(language.text("已按“工作区_Agent ID_bot”生成；可修改。", "Generated as workspace_Agent ID_bot; you can edit it.")),
                        );
                    } else if provider_id == "telegram" {
                        form = form.child(
                            DesktopFormField::new(
                                theme,
                                "Bot Token",
                                DesktopInput::large(&secret_input)
                                    .content_type(InputContentType::Password)
                                    .mask_toggle(),
                            )
                            .error(secret_error.clone())
                            .hint(language.text("粘贴 BotFather 提供的完整 Token。", "Paste the complete token from BotFather.")),
                        );
                    } else if !mode_new {
                        form = form.child(
                            div()
                                .flex()
                                .gap(px(14.))
                                .child(
                                    div().flex_1().child(
                                        DesktopFormField::new(
                                            theme,
                                            "App ID",
                                            div()
                                                .debug_selector(|| "agent-app-id-input".into())
                                                .child(DesktopInput::large(&provider_handle_input)),
                                        )
                                        .error(provider_handle_error.clone()),
                                    ),
                                )
                                .child(
                                    div().flex_1().child(
                                        DesktopFormField::new(
                                            theme,
                                            "App Secret",
                                            div()
                                                .debug_selector(|| "agent-app-secret-input".into())
                                                .child(
                                                    DesktopInput::large(&secret_input)
                                                        .content_type(InputContentType::Password)
                                                        .mask_toggle(),
                                                ),
                                        )
                                        .error(secret_error.clone()),
                                    ),
                                ),
                        );
                    }
                }
                if is_edit {
                    let reaction_app = save_app.clone();
                    form = form
                        .child(
                            div()
                                .min_h(px(48.))
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(theme.text_secondary)
                                        .child(language.text("自动回复表情", "Automatic reaction")),
                                )
                                .child(ReactionPicker::new(
                                    reaction_options.clone(),
                                    selected_reaction.clone(),
                                    reaction_loading,
                                    theme,
                                    move |selected, _window, cx| {
                                        let _ = reaction_app.update(cx, |this, cx| {
                                            this.agent_reaction = selected;
                                            this.notify_dialog_changed(cx);
                                        });
                                    },
                                )),
                        )
                        .when_some(reaction_error, |form, error| {
                            form.child(
                                div()
                                    .mt(px(-12.))
                                    .text_size(px(12.))
                                    .text_color(theme.error)
                                    .child(error),
                            )
                        })
                        .child(DesktopFormField::new(
                            theme,
                            language.text("灵魂", "Soul"),
                            DesktopTextarea::new(&soul_input).h(px(150.)),
                        ))
                        .child({
                            let mut advanced = div()
                                .debug_selector(|| "agent-advanced-settings".into())
                                .rounded(px(20.))
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.raised)
                                .overflow_hidden()
                                .child(
                                    DesktopButton::ghost("advanced-toggle", language.text("高级设置", "Advanced settings"))
                                        .w_full()
                                        .h(px(48.))
                                        .justify_between()
                                        .px(px(16.))
                                        .child(themed_icon(
                                            "icons/lucide/chevron-down.svg",
                                            15.,
                                            theme,
                                        ))
                                        .on_click(window.listener_for(
                                            &advanced_app,
                                            |this, _, _window, cx| {
                                                this.advanced_open = !this.advanced_open;
                                                this.notify_dialog_changed(cx);
                                            },
                                        )),
                                );
                            if advanced_open {
                                let mut environment_editor =
                                    div().flex().flex_col().gap(px(10.)).child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(theme.text_secondary)
                                            .child(language.text("环境变量", "Environment variables")),
                                    );
                                for (index, row) in environment_rows.iter().cloned().enumerate() {
                                    let remove_app = save_app.clone();
                                    environment_editor = environment_editor.child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(
                                                div()
                                                    .w(px(210.))
                                                    .flex_none()
                                                    .child(DesktopInput::mono_large(&row.key)),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .child(DesktopInput::mono_large(&row.value)),
                                            )
                                            .child(
                                                DesktopButton::icon(ElementId::Name(
                                                    format!("remove-env-{index}").into(),
                                                ))
                                                .child(themed_icon(
                                                    "icons/lucide/trash.svg",
                                                    16.,
                                                    theme,
                                                ))
                                                .on_click(window.listener_for(
                                                    &remove_app,
                                                    move |this, _, _, cx| {
                                                        this.remove_environment_variable(index, cx);
                                                    },
                                                )),
                                            ),
                                    );
                                }
                                let add_app = save_app.clone();
                                environment_editor = environment_editor.child(
                                    div().flex().child(
                                        DesktopButton::ghost_content(
                                            "add-environment-variable",
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(7.))
                                                .child(themed_icon(
                                                    "icons/lucide/plus.svg",
                                                    15.,
                                                    theme,
                                                ))
                                                .child(language.text("添加环境变量", "Add environment variable")),
                                        )
                                        .bg(theme.raised)
                                        .on_click(
                                            window.listener_for(&add_app, |this, _, window, cx| {
                                                this.add_environment_variable(window, cx);
                                            }),
                                        ),
                                    ),
                                );
                                advanced = advanced.child(
                                    div()
                                        .border_t_1()
                                        .border_color(theme.border)
                                        .p(px(16.))
                                        .pt(px(18.))
                                        .flex()
                                        .flex_col()
                                        .gap(px(16.))
                                        .child(DesktopFormField::new(
                                            theme,
                                            language.text("CLI 路径", "CLI path"),
                                            DesktopInput::mono_large(&cli_input),
                                        ))
                                        .child(environment_editor),
                                );
                            }
                            advanced
                        });
                }
                let body = if is_edit {
                    let avatar_app = save_app.clone();
                    let scroll_handle = window
                        .use_keyed_state("agent-edit-scroll", cx, |_, _| ScrollHandle::new())
                        .read(cx)
                        .clone();
                    let fade_handle = scroll_handle.clone();
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .relative()
                                .child(
                                    div()
                                        .id("agent-edit-scroll-area")
                                        .debug_selector(|| "agent-edit-scroll-area".into())
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&scroll_handle)
                                        .child(form.flex_none())
                                        .vertical_scrollbar(&scroll_handle),
                                )
                                .child(
                                    canvas(
                                        |_, _, _| {},
                                        move |bounds, _, window, _| {
                                            let remaining = fade_handle.max_offset().y
                                                + fade_handle.offset().y;
                                            if remaining > px(1.) {
                                                window.paint_quad(fill(
                                                    bounds,
                                                    linear_gradient(
                                                        180.,
                                                        linear_color_stop(theme.content.opacity(0.), 0.),
                                                        linear_color_stop(theme.content, 1.),
                                                    ),
                                                ));
                                            }
                                        },
                                    )
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .w_full()
                                    .h(px(28.)),
                                ),
                        )
                        .child(agent_edit_preview(
                            preview_id.clone(),
                            preview_name.clone(),
                            preview_role.clone(),
                            preview_bio.clone(),
                            preview_harness.clone(),
                            preview_model.clone(),
                            preview_avatar.clone(),
                            theme,
                            language,
                            window.listener_for(&avatar_app, |this, _, window, cx| {
                                this.choose_agent_avatar(window, cx);
                            }),
                        ))
                        .into_any_element()
                } else {
                    let body = div().flex_1().min_h_0().child(form);
                    if provider_id == "telegram" {
                        body.overflow_y_scrollbar().into_any_element()
                    } else {
                        body.overflow_hidden().into_any_element()
                    }
                };
                content
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        DesktopDialogHeader::new(if is_edit {
                            language.text("编辑 Agent 资料", "Edit Agent profile")
                        } else {
                            language.text("添加 Agent", "Add Agent")
                        })
                        .closable(!dialog_busy),
                    )
                    .child(body)
                    .child(
                        DesktopDialogFooter::new()
                            .compact()
                            .child(desktop_dialog_cancel("cancel-agent", language.text("取消", "Cancel")))
                            .child(desktop_dialog_button(
                                "save-agent",
                                DesktopButton::primary_loading(
                                    "save-agent",
                                    if is_edit {
                                        language.text("保存更改", "Save changes")
                                    } else {
                                        language.text("创建 Agent", "Create Agent")
                                    },
                                    save_busy,
                                )
                                .disabled(dialog_busy && !save_busy)
                                .on_click(window.listener_for(&save_app, move |this, _, _, cx| {
                                    this.submit_agent(editing_for_save.clone(), cx);
                                })),
                            )),
                    )
            })
    }

    fn agent_onboarding_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<LankideApp>,
        state: AgentOnboardingState,
        window: &mut Window,
        _cx: &App,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let theme = self.theme();
        let (onboarding_title, onboarding_instruction) =
            localized_agent_onboarding_copy(&state, language);
        let auto_polling = state.poll_interval_seconds.is_some()
            && !matches!(
                state.status,
                OnboardingStatus::Ready | OnboardingStatus::Error
            );
        let cancel_app = app.clone();
        let continue_app = app.clone();
        let open_link_app = app.clone();
        let state_for_action = state.clone();
        let checking = self.onboarding_checking;
        let error = self
            .error
            .as_ref()
            .map(|message| message.text(language))
            .or_else(|| {
                state
                    .error
                    .as_ref()
                    .map(|error| friendly_error(error, language))
            });
        let verification_uri = state.verification_uri.clone();
        let qr_value = verification_uri.clone().or_else(|| {
            state
                .payload
                .get("qr_code")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        });
        let is_feishu_sign_in = state.provider_id == "feishu" && state.step == "console_login";
        let is_new_feishu_permissions =
            state.provider_id == "feishu" && state.step == "permissions";
        let steps = if state.provider_id == "telegram" {
            vec![
                "Manager Bot",
                language.text("基本信息", "Basics"),
                language.text("创建 Bot", "Create Bot"),
                language.text("Bot 协作", "Bot collaboration"),
                language.text("启用", "Enable"),
            ]
        } else {
            vec![
                language.text("基本信息", "Basics"),
                language.text("创建应用", "Create app"),
                language.text("授权能力", "Permissions"),
                language.text("启用", "Enable"),
            ]
        };
        let active_step = if state.provider_id == "telegram" {
            2
        } else if matches!(state.step.as_str(), "permissions" | "authorization_update") {
            2
        } else if state.status == OnboardingStatus::Ready {
            3
        } else {
            1
        };
        let dialog_height = px(if qr_value.is_some() { 610. } else { 390. })
            .min((window.viewport_size().height - px(48.)).max(px(390.)));
        let centered_top = ((window.viewport_size().height - dialog_height) * 0.5).max(px(64.));
        DesktopDialog::from(dialog)
            .w(px(720.))
            .h(dialog_height)
            .margin_top(px(0.))
            .mt(centered_top)
            .overlay(true)
            .overlay_closable(false)
            .on_cancel(move |_, _, cx| {
                cancel_app.update(cx, |this, cx| {
                    this.onboarding_poll_epoch = this.onboarding_poll_epoch.wrapping_add(1);
                    this.onboarding_checking = false;
                    this.modal = None;
                    this.error = None;
                    cx.notify();
                });
                true
            })
            .content(move |content, window, _| {
                let onboarding_for_action = state_for_action.clone();
                let mut body = div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px(px(22.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(16.));
                if let Some(error) = &error {
                    body = body.child(DesktopBanner::error(theme, error));
                }
                body = body
                    .child(DesktopSteps::new(theme, &steps, active_step))
                    .child(
                        div()
                            .p(px(18.))
                            .rounded(px(15.))
                            .bg(theme.raised)
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(color_icon(provider_icon(&state.provider_id), 30.))
                            .child(
                                div()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(onboarding_title.clone())
                                            .when(auto_polling, |title| {
                                                title.child(
                                                    div()
                                                        .debug_selector(|| {
                                                            "agent-onboarding-spinner".into()
                                                        })
                                                        .flex()
                                                        .items_center()
                                                        .child(
                                                            Spinner::new()
                                                                .with_size(px(16.))
                                                                .color(theme.text_secondary),
                                                        ),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .mt(px(5.))
                                            .text_color(theme.text_secondary)
                                            .child(onboarding_instruction.clone()),
                                    ),
                            ),
                    );
                if let Some(qr_value) = qr_value.clone() {
                    let link = verification_uri.clone();
                    let qr_code = qr_code_image(&qr_value);
                    body = body.child(
                        div()
                            .py(px(8.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .when_some(qr_code, |panel, (qr_code, edge)| {
                                panel.child(
                                    img(qr_code)
                                        .size(px(edge))
                                        .flex_none()
                                        .object_fit(ObjectFit::Contain),
                                )
                            })
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(if state.provider_id == "telegram" {
                                        language.text("在 Telegram 中确认", "Confirm in Telegram")
                                    } else if is_feishu_sign_in {
                                        if state.status == OnboardingStatus::Verifying {
                                            language.text(
                                                "在飞书中确认登录",
                                                "Confirm sign-in in Feishu",
                                            )
                                        } else {
                                            language.text(
                                                "使用飞书扫码登录",
                                                "Scan with Feishu to sign in",
                                            )
                                        }
                                    } else if is_new_feishu_permissions {
                                        language.text(
                                            "使用飞书扫码继续授权",
                                            "Scan with Feishu to continue authorization",
                                        )
                                    } else {
                                        language
                                            .text("使用飞书扫码确认", "Scan with Feishu to confirm")
                                    }),
                            )
                            .when_some(link, |panel, link| {
                                panel.child(
                                    DesktopButton::outline(
                                        "open-agent-onboarding-link",
                                        if state.provider_id == "telegram" {
                                            language.text("在 Telegram 中打开", "Open in Telegram")
                                        } else if is_new_feishu_permissions {
                                            language.text("继续授权", "Continue authorization")
                                        } else {
                                            language.text("在飞书中打开", "Open in Feishu")
                                        },
                                    )
                                    .on_click(
                                        window.listener_for(&open_link_app, move |_, _, _, cx| {
                                            cx.open_url(&link)
                                        }),
                                    ),
                                )
                            }),
                    );
                }
                let mut footer = DesktopDialogFooter::new().child(desktop_dialog_cancel(
                    "close-agent-onboarding",
                    language.text("稍后完成", "Finish later"),
                ));
                if !auto_polling {
                    footer = footer.child(desktop_dialog_button(
                        "continue-agent-onboarding",
                        DesktopButton::primary_loading(
                            "continue-agent-onboarding",
                            if verification_uri.is_some() {
                                language.text("检查状态", "Check status")
                            } else {
                                language.text("继续", "Continue")
                            },
                            checking,
                        )
                        .on_click(window.listener_for(
                            &continue_app,
                            move |this, _, _, cx| {
                                this.advance_agent_onboarding(onboarding_for_action.clone(), cx);
                            },
                        )),
                    ));
                }
                content
                    .min_h_0()
                    .overflow_hidden()
                    .child(DesktopDialogHeader::new(
                        language.text("启用 Agent", "Enable Agent"),
                    ))
                    .child(body)
                    .child(footer)
            })
    }
}
