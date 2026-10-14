//! Application navigation, page headers and business page composition.

use super::*;

impl LankideApp {
    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = self.theme();
        Sidebar::new(theme)
            .child(
                div()
                    .id("nav-agents")
                    .debug_selector(|| "nav-agents".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = Page::Agents;
                        cx.notify();
                    }))
                    .child(
                        SidebarItem::new("nav-agents-item", self.tr("成员", "Members"), theme)
                            .leading_element(themed_icon("icons/lucide/users.svg", 16., theme))
                            .when(self.workspace_loaded, |item| {
                                item.trailing(self.agents.len().to_string())
                            })
                            .selected(self.page == Page::Agents),
                    ),
            )
            .child(
                div()
                    .id("nav-workspace")
                    .debug_selector(|| "nav-workspace".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_workspace_settings(window, cx);
                    }))
                    .child(
                        SidebarItem::new("nav-workspace-item", self.tr("团队", "Team"), theme)
                            .leading_element(themed_icon("icons/lucide/bot.svg", 16., theme))
                            .selected(self.page == Page::Workspace),
                    ),
            )
            .footer(
                div()
                    .p(px(8.))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .id("nav-settings")
                            .debug_selector(|| "nav-settings".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = Page::Settings;
                                cx.notify();
                            }))
                            .child(
                                SidebarItem::new(
                                    "nav-settings-item",
                                    self.tr("设置", "Settings"),
                                    theme,
                                )
                                .leading_element(themed_icon(
                                    "icons/lucide/settings.svg",
                                    16.,
                                    theme,
                                ))
                                .selected(self.page == Page::Settings),
                            ),
                    ),
            )
    }

    fn workspace_switcher(&self, cx: &mut Context<Self>) -> AnyElement {
        let language = self.language;
        let active_workspace = self.selected_workspace();
        let has_active_workspace = active_workspace.is_some();
        let active_name = active_workspace
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| self.tr("选择工作区", "Choose workspace").into());
        let active_initial = active_name
            .chars()
            .next()
            .map(|character| character.to_string())
            .unwrap_or_else(|| language.text("工", "W").into());
        let active = active_workspace
            .map(|workspace| {
                workspace.observed_state == lankide_client::WorkspaceObservedState::Running
            })
            .unwrap_or(false);
        let theme = self.theme();
        let workspaces = self.workspaces.clone();
        let selected = self.selected_workspace_id.clone();
        let app = cx.entity();
        let switcher_app = app.clone();
        DesktopButton::ghost_content(
            "workspace-switcher",
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(has_active_workspace, |content| {
                    content.child(
                        div()
                            .relative()
                            .size(px(18.))
                            .rounded(px(6.))
                            .bg(theme.raised)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(active_initial)
                            .child(
                                div()
                                    .absolute()
                                    .right(px(-2.))
                                    .bottom(px(-2.))
                                    .size(px(6.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(theme.content)
                                    .bg(if active { rgb(0x22a447) } else { rgb(0x9a9a9a) }),
                            ),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_left()
                        .child(active_name),
                )
                .child(themed_icon("icons/lucide/chevron-down.svg", 13., theme)),
        )
        .h(px(28.))
        .w(px(176.))
        .px(px(8.))
        .bg(theme.hover)
        .justify_start()
        .dropdown_menu(move |mut menu, window, _| {
            menu = menu.min_w(px(176.)).max_w(px(176.));
            for workspace in &workspaces {
                let id = workspace.id.clone();
                let label = workspace.name.clone();
                let active = selected.as_deref() == Some(id.as_str());
                let target = switcher_app.clone();
                menu = menu.item(PopupMenuItem::new(label).checked(active).on_click(
                    window.listener_for(&target, move |this, _, _, cx| {
                        this.select_workspace(id.clone(), cx);
                    }),
                ));
            }
            let target = switcher_app.clone();
            menu.separator().item(
                PopupMenuItem::new(language.text("新建工作区", "New workspace")).on_click(
                    window.listener_for(&target, move |this, _, window, cx| {
                        this.open_create_workspace(window, cx);
                    }),
                ),
            )
        })
        .into_any_element()
    }

    fn page_header(&self, title: impl Into<SharedString>, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme();
        PaneHeader::new(title, theme)
            .trailing(
                div()
                    .flex()
                    .items_center()
                    .child(self.workspace_switcher(cx)),
            )
            .into_any_element()
    }

    pub(super) fn agents_content(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let language = self.language;
        let theme = self.theme();
        let app = cx.entity();
        let add_app = app.clone();
        let header = self.page_header(language.text("你的团队", "Your team"), cx);
        let toolbar = if self.selected_workspace_id.is_some() {
            Some(
                div().w_full().mb(px(24.)).flex().justify_end().child(
                    DesktopButton::primary_content(
                        "add-agent",
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(primary_button_icon("icons/lucide/plus.svg", 15., theme))
                            .child(language.text("添加成员", "Add member")),
                    )
                    .min_w(px(112.))
                    .px(px(16.))
                    .on_click(move |_, window, cx| {
                        let _ = add_app.update(cx, |this, cx| {
                            this.open_create_agent(window, cx);
                        });
                    }),
                ),
            )
        } else {
            None
        };

        let body = if !self.workspace_loaded && self.loading {
            DesktopCenteredMessage::new(
                theme,
                language.text("正在读取工作区…", "Loading workspaces…"),
            )
            .into_any_element()
        } else if !self.workspace_loaded {
            DesktopCenteredMessage::new(
                theme,
                language.text("暂时无法显示内容", "Content is temporarily unavailable"),
            )
            .into_any_element()
        } else if self.selected_workspace_id.is_none() {
            self.empty_workspace(cx).into_any_element()
        } else if self.loading && self.agents.is_empty() {
            // Switching workspaces is a local round trip; a message would only flash.
            div().into_any_element()
        } else if self.agents.is_empty() {
            let empty_app = app.clone();
            DesktopEmptyState::new(
                theme,
                language.text("还没有队友", "No teammates yet"),
                language.text(
                    "添加第一个 Agent，开始在 IM 中协作。",
                    "Add your first Agent to collaborate in chat.",
                ),
            )
            .action(
                DesktopButton::primary("empty-add-agent", language.text("添加成员", "Add member"))
                    .on_click(move |_, window, cx| {
                        let _ = empty_app.update(cx, |this, cx| {
                            this.open_create_agent(window, cx);
                        });
                    }),
            )
            .into_any_element()
        } else {
            let mut cards = div().w_full().flex().flex_wrap().items_start().gap(px(24.));
            for (index, agent) in self.agents.iter().cloned().enumerate() {
                cards = cards.child(self.agent_card(agent, index, &app));
            }
            cards.into_any_element()
        };

        ContentPane::new(
            theme,
            div()
                .w_full()
                .max_w(px(1120.))
                .mx_auto()
                .p(px(32.))
                .children(self.feedback(theme))
                .children(toolbar)
                .child(body),
        )
        .header(header)
    }

    fn empty_workspace(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let language = self.language;
        let app = cx.entity();
        DesktopEmptyState::new(
            self.theme(),
            language.text("创建第一个工作区", "Create your first workspace"),
            language.text(
                "选择飞书或 Telegram，随后添加一起工作的 Agent。",
                "Choose Feishu or Telegram, then add Agents to work with.",
            ),
        )
        .action(
            DesktopButton::primary(
                "first-workspace",
                language.text("新建工作区", "New workspace"),
            )
            .on_click(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.open_create_workspace(window, cx);
                });
            }),
        )
    }

    fn agent_card(
        &self,
        agent: Agent,
        index: usize,
        app: &Entity<Self>,
    ) -> impl IntoElement + use<> {
        let theme = self.theme();
        let edit_app = app.clone();
        let delete_app = app.clone();
        let hover_app = app.clone();
        let resume_app = app.clone();
        let resume_id = agent.id.clone();
        let paused_label = self.tr("已暂停", "Paused");
        let resume_label = self.tr("继续", "Resume");
        let edit_id = agent.id.clone();
        let delete_id = agent.id.clone();
        let hover_id = agent.id.clone();
        let hovered = self.hovered_agent_id.as_deref() == Some(agent.id.as_str());
        desktop_settings_group(theme)
            .id(ElementId::Name(format!("agent-card-{}", agent.id).into()))
            .debug_selector(|| format!("agent-card-{}", agent.id))
            .w(px(296.))
            .min_h(px(360.))
            .p(px(20.))
            .rounded(px(28.))
            .flex()
            .flex_col()
            .items_center()
            .relative()
            .on_hover(move |entered, _, cx| {
                let id = hover_id.clone();
                let _ = hover_app.update(cx, |this, cx| {
                    if *entered {
                        this.hovered_agent_id = Some(id);
                    } else if this.hovered_agent_id.as_deref() == Some(id.as_str()) {
                        this.hovered_agent_id = None;
                    }
                    cx.notify();
                });
            })
            .when(hovered, |card| {
                card.child(
                    div()
                        .absolute()
                        .top(px(16.))
                        .right(px(16.))
                        .flex()
                        .gap(px(6.))
                        .child(
                            DesktopButton::icon(ElementId::Name(
                                format!("edit-agent-{index}").into(),
                            ))
                            .bg(theme.raised)
                            .rounded_full()
                            .child(muted_themed_icon("icons/lucide/pencil.svg", 15., theme))
                            .on_click(move |_, window, cx| {
                                let id = edit_id.clone();
                                let _ = edit_app.update(cx, |this, cx| {
                                    this.open_edit_agent(id, window, cx);
                                });
                            }),
                        )
                        .child(
                            DesktopButton::icon(ElementId::Name(
                                format!("delete-agent-{index}").into(),
                            ))
                            .bg(theme.raised)
                            .rounded_full()
                            .child(muted_themed_icon("icons/lucide/trash.svg", 15., theme))
                            .on_click(move |_, _, cx| {
                                let id = delete_id.clone();
                                let _ = delete_app.update(cx, |this, cx| {
                                    this.open_delete_agent(id, cx);
                                });
                            }),
                        ),
                )
            })
            .child(self.agent_avatar(&agent, 112.))
            .child(
                div()
                    .mt(px(14.))
                    .max_w_full()
                    .truncate()
                    .text_size(px(21.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(agent.name.clone()),
            )
            .child(
                div()
                    .mt(px(5.))
                    .font_family(desktop_ui::FONT_MONO)
                    .text_size(px(12.))
                    .text_color(theme.text_secondary)
                    .child(if agent.role.is_empty() {
                        "—".into()
                    } else {
                        agent.role.clone()
                    }),
            )
            .when_some(agent.pause_reason.clone(), |card, reason| {
                card.child(
                    div()
                        .mt(px(12.))
                        .text_size(px(12.))
                        .debug_selector(|| "agent-paused".to_string())
                        .child(paused_label),
                )
                .child(
                    div()
                        .mt(px(5.))
                        .max_w_full()
                        .text_size(px(12.))
                        .text_color(theme.text_secondary)
                        .child(reason),
                )
                .child(
                    DesktopButton::outline(
                        ElementId::Name(format!("resume-agent-{}", agent.id).into()),
                        resume_label,
                    )
                    .mt(px(8.))
                    .on_click(move |_, _, cx| {
                        let id = resume_id.clone();
                        let _ = resume_app.update(cx, |this, cx| this.resume_agent(id, cx));
                    }),
                )
            })
            .when(!agent.bio.is_empty(), |card| {
                card.child(
                    div()
                        .mt(px(14.))
                        .h(px(40.))
                        .text_center()
                        .line_height(relative(1.45))
                        .text_color(theme.text_secondary)
                        .child(agent.bio.clone()),
                )
            })
            .child(
                div()
                    .mt(px(12.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded_full()
                    .bg(theme.raised)
                    .font_family(desktop_ui::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(theme.text_secondary)
                    .child(format!("@{}", agent.id)),
            )
            .child(
                div()
                    .mt_auto()
                    .pt(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(DesktopChip::new(
                        theme,
                        harness_icon(&agent.harness_id, 16., theme),
                        harness_label(&agent.harness_id),
                    ))
                    .child(DesktopChip::new(
                        theme,
                        model_icon(&agent.model, 16., theme),
                        model_label(&agent.model),
                    )),
            )
    }

    pub(super) fn workspace_content(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let language = self.language;
        let theme = self.theme();
        let Some(workspace) = self.selected_workspace().cloned() else {
            return ContentPane::new(theme, self.empty_workspace(cx)).into_any_element();
        };
        let app = cx.entity();
        let save_app = app.clone();
        let any_busy = self.busy.is_some();
        let save_busy = self.busy == Some(BusyOperation::SaveWorkspace);
        let task_system_supported = self
            .provider_supports_capability(&workspace.provider.id, PROVIDER_CAPABILITY_TASK_SYSTEM);
        let has_agents = !self.agents.is_empty();
        let content = div()
            .w_full()
            .max_w(px(1040.))
            .mx_auto()
            .p(px(32.))
            .children(self.feedback(theme))
            .child(
                desktop_settings_group(theme)
                    .p(px(12.))
                    .rounded(px(26.))
                    .child(
                        div()
                            .p(px(22.))
                            .flex()
                            .flex_col()
                            .gap(px(18.))
                            .child(DesktopFormField::new(
                                theme,
                                language.text("团队名称", "Team name"),
                                DesktopInput::large(&self.settings_name_input),
                            ))
                            .child(DesktopFormField::new(
                                theme,
                                language.text("团队规约", "Team guidelines"),
                                DesktopTextarea::mono(&self.settings_rules_input).h(px(170.)),
                            ))
                            .child(
                                div()
                                    .p(px(16.))
                                    .rounded(px(16.))
                                    .bg(theme.raised)
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(theme.text_secondary)
                                            .child("IM"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(10.))
                                            .child(workspace_provider_choice(
                                                "feishu", language.text("飞书", "Feishu"), workspace.provider.id == "feishu", true, theme,
                                            ))
                                            .child(workspace_provider_choice(
                                                "telegram", "Telegram", workspace.provider.id == "telegram", true, theme,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(theme.text_secondary)
                                            .child(if has_agents {
                                                language.text("已有 Agent 后不能更换 IM。", "You cannot change the chat service after adding Agents.")
                                            } else {
                                                language.text("IM 在创建工作区时确定。", "The chat service is chosen when creating the workspace.")
                                            }),
                                    )
                                    .when(task_system_supported, |panel| {
                                        panel
                                            .child(div().h(px(1.)).bg(theme.border))
                                            .child(
                                                DesktopCheckbox::new(
                                                    "settings-task-system",
                                                    language.text("启用任务系统", "Enable tasks"),
                                                )
                                                .checked(self.settings_task_enabled)
                                                .on_click(cx.listener(
                                                    |this, checked: &bool, _, cx| {
                                                        this.settings_task_enabled = *checked;
                                                        cx.notify();
                                                    },
                                                )),
                                            )
                                            .child(
                                                div()
                                                    .ml(px(28.))
                                                    .text_size(px(12.))
                                                    .text_color(theme.text_secondary)
                                                    .child(language.text(
                                                        "集中查看团队任务，并明确负责人和协作者。",
                                                        "View team tasks with their assignees and collaborators.",
                                                    )),
                                            )
                                            .when(self.settings_task_enabled, |panel| {
                                                panel.child(DesktopFormField::new(
                                                    theme,
                                                    language.text("清单名称", "Task list name"),
                                                    DesktopInput::large(
                                                        &self.settings_task_name_input,
                                                    ),
                                                ))
                                            })
                                    }),
                            )
                            .child(
                                div().flex().justify_end().child(
                                    DesktopButton::primary_loading(
                                        "save-workspace",
                                        language.text("保存", "Save"),
                                        save_busy,
                                    )
                                    .disabled(any_busy && !save_busy)
                                        .on_click(move |_, window, cx| {
                                            let window_handle = window.window_handle();
                                            let _ = save_app.update(cx, |this, cx| {
                                                this.save_workspace_settings(window_handle, cx);
                                            });
                                        }),
                                ),
                            ),
                    ),
            );
        ContentPane::new(theme, content)
            .header(self.page_header(language.text("团队", "Team"), cx))
            .into_any_element()
    }

    fn runtime_status_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let language = self.language;
        let theme = self.theme();
        let runtime = self.selected_workspace();
        let runtime_active =
            runtime.is_some_and(|workspace| workspace.status == WorkspaceStatus::Active);
        use lankide_client::WorkspaceObservedState;
        let observed_state = runtime
            .map(|workspace| workspace.observed_state)
            .unwrap_or(WorkspaceObservedState::Unknown);
        let status_color = match observed_state {
            WorkspaceObservedState::Running if runtime_active => rgb(0x238636).into(),
            WorkspaceObservedState::Error => rgb(0xdc2626).into(),
            WorkspaceObservedState::Starting | WorkspaceObservedState::Running => {
                rgb(0xb7791f).into()
            }
            _ => theme.text_secondary,
        };
        let status_title = match observed_state {
            WorkspaceObservedState::Starting => language.text("启动中", "Starting"),
            WorkspaceObservedState::Running if !runtime_active => {
                language.text("暂停中", "Pausing")
            }
            WorkspaceObservedState::Running => language.text("启用", "Enabled"),
            WorkspaceObservedState::Paused => language.text("已暂停", "Paused"),
            WorkspaceObservedState::Error => language.text("需要处理", "Needs attention"),
            WorkspaceObservedState::Stopped => language.text("已停止", "Stopped"),
            WorkspaceObservedState::Unknown => language.text("状态待确认", "Status unknown"),
        };
        let status_description = if let Some(workspace) = runtime
            && !workspace.status_message.is_empty()
            && workspace.status_message != "Setup needs attention."
        {
            workspace.status_message.clone()
        } else {
            match observed_state {
                WorkspaceObservedState::Running => language.text(
                    "已启用。团队成员可以继续接收协作消息。",
                    "Enabled. Team members can continue receiving collaboration messages.",
                ),
                WorkspaceObservedState::Starting => {
                    language.text("正在启动，请稍候。", "Starting. Please wait.")
                }
                WorkspaceObservedState::Error => language.text(
                    "请检查配置后重试启用。",
                    "Check the settings, then try enabling again.",
                ),
                WorkspaceObservedState::Unknown if runtime.is_none() => language.text(
                    "创建团队后即可启用运行状态。",
                    "Create a team to enable the runtime.",
                ),
                WorkspaceObservedState::Unknown => language.text(
                    "请刷新后确认运行状态。",
                    "Refresh to check the runtime status.",
                ),
                _ => language.text(
                    "已暂停。团队成员不会接收新的协作消息。",
                    "Paused. Team members won't receive new collaboration messages.",
                ),
            }
            .to_owned()
        };
        let status_label = if runtime_active {
            language.text("暂停", "Pause")
        } else {
            language.text("启用", "Enable")
        };
        let status_icon = if runtime_active {
            "icons/lucide/pause.svg"
        } else {
            "icons/lucide/play.svg"
        };
        let status_action_color = if runtime_active {
            rgb(0xdc2626)
        } else {
            rgb(0x16803a)
        };
        let status_action_background = if runtime_active {
            rgba(0xdc262611)
        } else {
            rgba(0x16803a11)
        };
        let app = cx.entity();
        let state_app = app.clone();
        let any_busy = self.busy.is_some();
        let toggle_busy = self.busy == Some(BusyOperation::ToggleRuntime);
        desktop_settings_group(theme)
            .child(
                div()
                    .w_full()
                    .min_h(px(76.))
                    .px(px(18.))
                    .py(px(14.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(11.))
                            .child(div().size(px(8.)).rounded_full().bg(status_color))
                            .child(
                                div()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(status_color)
                                            .child(status_title),
                                    )
                                    .child(
                                        div()
                                            .mt(px(3.))
                                            .text_size(px(12.))
                                            .text_color(theme.text_secondary)
                                            .child(status_description),
                                    ),
                            ),
                    )
                    .child(
                        DesktopButton::outline_content(
                            "toggle-runtime",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .child(if toggle_busy {
                                    div()
                                        .debug_selector(|| "toggle-runtime-loading".into())
                                        .flex()
                                        .items_center()
                                        .child(
                                            Spinner::new()
                                                .with_size(px(14.))
                                                .color(status_action_color.into()),
                                        )
                                        .into_any_element()
                                } else {
                                    Icon::default()
                                        .path(status_icon)
                                        .size(px(14.))
                                        .text_color(status_action_color)
                                        .into_any_element()
                                })
                                .child(status_label),
                        )
                        .min_w(px(82.))
                        .border_color(status_action_color)
                        .text_color(status_action_color)
                        .bg(status_action_background)
                        .loading(toggle_busy)
                        .disabled((any_busy && !toggle_busy) || runtime.is_none())
                        .on_click(move |_, window, cx| {
                            let window_handle = window.window_handle();
                            let _ = state_app.update(cx, |this, cx| {
                                this.toggle_runtime_state(window_handle, cx);
                            });
                        }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn settings_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme();
        let app = cx.entity();
        let system_app = app.clone();
        let light_app = app.clone();
        let dark_app = app.clone();
        let theme_controls = div()
            .flex()
            .gap(px(4.))
            .child(
                desktop_choice(
                    "theme-system",
                    self.tr("跟随系统", "System"),
                    self.theme_preference == ThemePreference::System,
                    theme,
                )
                .on_click(move |_, window, cx| {
                    system_app.update(cx, |this, cx| {
                        this.set_theme(ThemePreference::System, window, cx)
                    });
                }),
            )
            .child(
                desktop_choice(
                    "theme-light",
                    self.tr("浅色", "Light"),
                    self.theme_preference == ThemePreference::Light,
                    theme,
                )
                .on_click(move |_, window, cx| {
                    light_app.update(cx, |this, cx| {
                        this.set_theme(ThemePreference::Light, window, cx)
                    });
                }),
            )
            .child(
                desktop_choice(
                    "theme-dark",
                    self.tr("深色", "Dark"),
                    self.theme_preference == ThemePreference::Dark,
                    theme,
                )
                .on_click(move |_, window, cx| {
                    dark_app.update(cx, |this, cx| {
                        this.set_theme(ThemePreference::Dark, window, cx)
                    });
                }),
            );
        let body = div()
            .w_full()
            .max_w(px(760.))
            .p(px(32.))
            .children(self.feedback(theme))
            .child(
                div()
                    .text_size(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.tr("偏好设置", "Preferences")),
            )
            .child(desktop_settings_section_title(
                self.tr("运行状态", "Runtime status"),
                true,
            ))
            .child(self.runtime_status_card(cx))
            .child(desktop_settings_section_title(
                self.tr("语言", "Language"),
                false,
            ))
            .child(
                desktop_settings_group(theme).child(desktop_settings_row(
                    self.tr("语言", "Language"),
                    self.tr("选择桌面端的显示语言。", "Choose the desktop language."),
                    div()
                        .w(px(180.))
                        .debug_selector(|| "language-select".into())
                        .flex_none()
                        .child(DesktopSelect::new(&self.language_select).w_full()),
                    theme,
                )),
            )
            .child(desktop_settings_section_title(
                self.tr("外观", "Appearance"),
                false,
            ))
            .child(desktop_settings_group(theme).child(desktop_settings_row(
                self.tr("主题", "Theme"),
                self.tr(
                    "跟随系统，或固定使用浅色与深色外观。",
                    "Follow the system, or use light or dark appearance.",
                ),
                theme_controls,
                theme,
            )));
        ContentPane::new(theme, body)
            .header(self.settings_page_header(self.tr("设置", "Settings")))
            .into_any_element()
    }

    fn settings_page_header(&self, title: impl Into<SharedString>) -> AnyElement {
        PaneHeader::new(title, self.theme()).into_any_element()
    }

    fn feedback(&self, _theme: DesktopTheme) -> Vec<AnyElement> {
        // Modal forms render their own feedback beside the active controls. Keeping
        // the page-level banner hidden avoids showing the same error behind them.
        if self.modal.is_some() {
            return Vec::new();
        }
        let mut rows = Vec::new();
        if let Some(error) = &self.error {
            rows.push(
                div()
                    .debug_selector(|| "application-error".to_owned())
                    .mb(px(16.))
                    .child(DesktopBanner::error(
                        self.theme(),
                        error.text(self.language),
                    ))
                    .into_any_element(),
            );
        }
        rows
    }
}
