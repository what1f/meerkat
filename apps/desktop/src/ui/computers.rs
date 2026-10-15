use super::*;
use gpui_component::text::TextView;
use std::path::Path;

pub(crate) fn local_computer_id(home: &Path) -> Option<String> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(home.join("computer-credentials.json")).ok()?)
            .ok()?;
    value.get("computer_id")?.as_str().map(str::to_owned)
}

#[derive(Clone)]
pub(super) struct ComputerChoice {
    id: String,
    label: SharedString,
}
impl SelectItem for ComputerChoice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        div()
            .debug_selector(move || format!("computer-choice-{id}"))
            .child(self.label.clone())
    }
    fn value(&self) -> &String {
        &self.id
    }
}

impl LankideApp {
    pub(super) fn read_local_computer(&mut self, home: &Path) {
        self.local_computer_id = local_computer_id(home);
    }

    pub(super) fn update_computer_choices(
        &mut self,
        preferred: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = preferred;
        let selected = selected
            .filter(|id| self.computers.iter().any(|c| &c.id == id))
            .or_else(|| {
                self.local_computer_id
                    .clone()
                    .filter(|id| self.computers.iter().any(|c| &c.id == id))
            })
            .or_else(|| self.computers.first().map(|c| c.id.clone()));
        let choices = self
            .computers
            .iter()
            .map(|computer| ComputerChoice {
                id: computer.id.clone(),
                label: format!(
                    "{} · {}",
                    computer.name,
                    self.language.text(
                        if computer.available {
                            "在线"
                        } else {
                            "离线"
                        },
                        if computer.available {
                            "Online"
                        } else {
                            "Offline"
                        },
                    )
                )
                .into(),
            })
            .collect();
        self.agent_computer.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_index(None, window, cx);
            if let Some(id) = selected {
                select.set_selected_value(&id, window, cx);
            }
        });
    }

    pub(super) fn reload_computers(
        &mut self,
        preferred: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.computer_request += 1;
        let request = self.computer_request;
        let client = self.client.clone();
        // Clear stale placement while the new form's catalog is loading.
        self.agent_computer.update(cx, |select, cx| {
            select.set_items(Vec::new(), window, cx);
            select.set_selected_index(None, window, cx);
        });
        cx.spawn_in(window, async move |this, window| {
            let result = client.get_computer_catalog(None).await;
            let _ = this.update_in(window, |this, window, cx| {
                if this.computer_request != request {
                    return;
                }
                match result {
                    Ok(computers) => {
                        this.computers = computers
                            .into_iter()
                            .filter(|c| c.registered && c.revoked_at.is_none())
                            .collect();
                        this.update_computer_choices(preferred, window, cx);
                    }
                    Err(error) => {
                        this.computers.clear();
                        this.error = Some(UiMessage::client(error));
                    }
                }
                this.notify_dialog_changed(cx);
            });
        })
        .detach();
    }

    fn open_pair_computer(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(Modal::PairComputer);
        self.pairing_code = None;
        self.pairing_connected = None;
        self.pairing_copied = false;
        self.pairing_loading = true;
        self.pairing_editing = false;
        self.pairing_request += 1;
        let request = self.pairing_request;
        self.error = None;
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = async {
                let access = client.get_remote_access().await?;
                let catalog = client.get_computer_catalog(None).await?;
                let code = if access.address.is_some() {
                    Some(client.create_pairing_code().await?)
                } else {
                    None
                };
                Ok::<_, ClientError>((access, catalog, code))
            }
            .await;
            let mut known = std::collections::HashSet::new();
            let active = this
                .update(cx, |this, cx| {
                    if !this.pairing_active(request) {
                        return false;
                    }
                    this.pairing_loading = false;
                    match result {
                        Ok((access, catalog, code)) => {
                            known = catalog.iter().map(|c| c.id.clone()).collect();
                            this.computers = catalog
                                .into_iter()
                                .filter(|c| c.registered && c.revoked_at.is_none())
                                .collect();
                            this.pairing_address = access.address;
                            this.pairing_pin = access.cert_sha256;
                            this.pairing_editing = this.pairing_address.is_none();
                            this.pairing_code = code;
                        }
                        Err(error) => this.error = Some(UiMessage::client(error)),
                    }
                    this.notify_dialog_changed(cx);
                    true
                })
                .unwrap_or(false);
            if !active {
                return;
            }
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                let needs_code = match this.update(cx, |this, _| {
                    if !this.pairing_active(request) {
                        return None;
                    }
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    Some(
                        !this.pairing_editing
                            && this.pairing_address.is_some()
                            && this
                                .pairing_code
                                .as_ref()
                                .is_none_or(|code| now >= code.expires_at),
                    )
                }) {
                    Ok(Some(value)) => value,
                    _ => return,
                };
                let catalog = client.get_computer_catalog(None).await;
                let code = if needs_code {
                    Some(client.create_pairing_code().await)
                } else {
                    None
                };
                let connected = this
                    .update(cx, |this, cx| {
                        if !this.pairing_active(request) {
                            return false;
                        }
                        if let Ok(catalog) = catalog {
                            let added = catalog.iter().find(|c| {
                                c.registered
                                    && c.revoked_at.is_none()
                                    && c.available
                                    && !known.contains(&c.id)
                            });
                            if let Some(computer) = added {
                                this.pairing_connected = Some(computer.name.clone());
                            }
                            this.computers = catalog
                                .into_iter()
                                .filter(|c| c.registered && c.revoked_at.is_none())
                                .collect();
                        }
                        if let Some(code) = code {
                            match code {
                                Ok(code) => {
                                    this.pairing_code = Some(code);
                                    this.pairing_copied = false;
                                }
                                Err(error) => this.error = Some(UiMessage::client(error)),
                            }
                        }
                        this.notify_dialog_changed(cx);
                        this.pairing_connected.is_some()
                    })
                    .unwrap_or(false);
                if connected {
                    cx.background_executor()
                        .timer(Duration::from_millis(1600))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        if this.pairing_active(request) {
                            this.modal = None;
                            this.notify_dialog_changed(cx);
                        }
                    });
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn pairing_active(&self, request: u64) -> bool {
        self.pairing_request == request && matches!(self.modal, Some(Modal::PairComputer))
    }

    fn enable_remote_computers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self
            .pairing_address_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        self.pairing_loading = true;
        self.error = None;
        let client = self.client.clone();
        let request = self.pairing_request;
        cx.spawn_in(window, async move |this, window| {
            let result = async {
                let access = client.set_remote_access(Some(address)).await?;
                let code = client.create_pairing_code().await?;
                Ok::<_, ClientError>((access, code))
            }
            .await;
            let _ = this.update_in(window, |this, _, cx| {
                if !this.pairing_active(request) {
                    return;
                }
                this.pairing_loading = false;
                match result {
                    Ok((access, code)) => {
                        this.pairing_address = access.address;
                        this.pairing_pin = access.cert_sha256;
                        this.pairing_code = Some(code);
                        this.pairing_editing = false;
                        this.pairing_copied = false;
                    }
                    Err(_) => {
                        this.error = Some(UiMessage::localized(
                            "无法使用这个地址，请检查地址和端口。",
                            "Could not use this address. Check the address and port.",
                        ))
                    }
                }
                this.notify_dialog_changed(cx);
            });
        })
        .detach();
        self.notify_dialog_changed(cx);
    }

    pub(super) fn computer_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme();
        div()
            .child(desktop_settings_section_title(
                self.tr("电脑", "Computers"),
                false,
            ))
            .child(
                desktop_settings_group(theme)
                    .children(self.computers.iter().map(|computer| {
                        div()
                            .debug_selector({
                                let id = computer.id.clone();
                                move || format!("computer-{id}")
                            })
                            .px(px(18.))
                            .py(px(16.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(computer.name.clone())
                            .child(
                                div()
                                    .debug_selector({
                                        let id = computer.id.clone();
                                        let state = if computer.available {
                                            "online"
                                        } else {
                                            "offline"
                                        };
                                        move || format!("computer-{state}-{id}")
                                    })
                                    .text_size(px(12.))
                                    .text_color(theme.text_secondary)
                                    .child(self.language.text(
                                        if computer.available {
                                            "在线"
                                        } else {
                                            "离线"
                                        },
                                        if computer.available {
                                            "Online"
                                        } else {
                                            "Offline"
                                        },
                                    )),
                            )
                    }))
                    .child(
                        div()
                            .px(px(18.))
                            .py(px(14.))
                            .flex()
                            .justify_end()
                            .when(!self.computers.is_empty(), |row| {
                                row.border_t_1().border_color(theme.border)
                            })
                            .child(
                                DesktopButton::outline(
                                    "pair-computer",
                                    self.tr("添加电脑", "Add Computer"),
                                )
                                .debug_selector(|| "pair-computer".into())
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.open_pair_computer(cx)),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn pair_computer_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<Self>,
        _window: &mut Window,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let command = self.pairing_code.as_ref().and_then(|code| {
            Some(format!(
                "lankide-computer-daemon pair wss://{} {} --cert-sha256 {}",
                self.pairing_address.as_ref()?,
                code.code,
                self.pairing_pin.as_ref()?
            ))
        });
        let editing = self.pairing_editing;
        let loading = self.pairing_loading;
        let copied = self.pairing_copied;
        let connected = self.pairing_connected.clone();
        let address_input = self.pairing_address_input.clone();
        let theme = self.theme();
        let error = self.error.as_ref().map(|e| e.text(language));
        let enable_app = app.clone();
        let edit_app = app.clone();
        let copy_app = app.clone();
        DesktopDialog::from(dialog)
            .width(px(560.))
            .overlay(true)
            .on_cancel(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.modal = None;
                    this.error = None;
                    this.reload_computers(None, window, cx);
                    cx.notify();
                });
                true
            })
            .content(move |content, _, _| {
                let command_for_copy = command.clone();
                let enable_app = enable_app.clone();
                let edit_app = edit_app.clone();
                let copy_app = copy_app.clone();
                content
                    .child(DesktopDialogHeader::new(
                        language.text("添加电脑", "Add Computer"),
                    ))
                    .child(
                        div()
                            .px(px(22.))
                            .pb(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .when(loading, |body| body.child(Spinner::new()))
                            .when(editing, |body| {
                                body.child(
                                    div()
                                        .debug_selector(|| "pairing-address-input".into())
                                        .child(DesktopInput::mono_large(&address_input)),
                                )
                            })
                            .when(!editing && connected.is_none(), |body| {
                                body.when_some(command.clone(), |body, command| {
                                    body.child(
                                        div()
                                            .debug_selector(|| "pairing-command".into())
                                            .w_full()
                                            .p(px(16.))
                                            .rounded(px(10.))
                                            .bg(rgb(0x18212f))
                                            .text_color(rgb(0xf0f4fa))
                                            .text_size(px(12.))
                                            .line_height(px(21.))
                                            .font_family(desktop_ui::FONT_MONO)
                                            .child(
                                                TextView::html(
                                                    "pairing-command-text",
                                                    format!("<p>{command}</p>"),
                                                )
                                                .selectable(true),
                                            ),
                                    )
                                })
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.text_secondary)
                                        .child(
                                            language.text("等待连接…", "Waiting for connection…"),
                                        ),
                                )
                            })
                            .when_some(connected.clone(), |body, name| {
                                body.child(
                                    div()
                                        .debug_selector(|| "pairing-connected".into())
                                        .py(px(12.))
                                        .child(format!(
                                            "{} · {name}",
                                            language.text("已连接", "Connected")
                                        )),
                                )
                            })
                            .when_some(error.clone(), |body, error| {
                                body.child(DesktopBanner::error(theme, error))
                            }),
                    )
                    .child(
                        DesktopDialogFooter::new()
                            .compact()
                            .when(!editing && connected.is_none(), |footer| {
                                footer
                                    .child(
                                        div().flex_1().child(
                                            DesktopButton::ghost(
                                                "change-pairing-address",
                                                language.text("更改地址", "Change address"),
                                            )
                                            .on_click(
                                                move |_, window, cx| {
                                                    edit_app.update(cx, |this, cx| {
                                                        let address = this
                                                            .pairing_address
                                                            .clone()
                                                            .unwrap_or_default();
                                                        this.pairing_address_input.update(
                                                            cx,
                                                            |input, cx| {
                                                                input.set_value(address, window, cx)
                                                            },
                                                        );
                                                        this.pairing_editing = true;
                                                        this.error = None;
                                                        this.notify_dialog_changed(cx);
                                                    })
                                                },
                                            ),
                                        ),
                                    )
                                    .child(desktop_dialog_button(
                                        "copy-pairing-command",
                                        DesktopButton::primary(
                                            "copy-pairing-command",
                                            language.text(
                                                if copied { "已复制" } else { "复制命令" },
                                                if copied { "Copied" } else { "Copy command" },
                                            ),
                                        )
                                        .disabled(command_for_copy.is_none())
                                        .on_click(
                                            move |_, _, cx| {
                                                if let Some(command) = command_for_copy.clone() {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(command),
                                                    );
                                                    copy_app.update(cx, |this, cx| {
                                                        this.pairing_copied = true;
                                                        let request = this.pairing_request;
                                                        this.notify_dialog_changed(cx);
                                                        cx.spawn(async move |this, cx| {
                                                            cx.background_executor()
                                                                .timer(Duration::from_millis(1600))
                                                                .await;
                                                            let _ = this.update(cx, |this, cx| {
                                                                if this.pairing_active(request) {
                                                                    this.pairing_copied = false;
                                                                    this.notify_dialog_changed(cx);
                                                                }
                                                            });
                                                        })
                                                        .detach();
                                                    });
                                                }
                                            },
                                        ),
                                    ))
                            })
                            .when(editing, |footer| {
                                footer.child(desktop_dialog_button(
                                    "enable-computer-access",
                                    DesktopButton::primary_loading(
                                        "enable-computer-access",
                                        language.text("启用", "Enable"),
                                        loading,
                                    )
                                    .disabled(loading)
                                    .on_click(
                                        move |_, window, cx| {
                                            enable_app.update(cx, |this, cx| {
                                                this.enable_remote_computers(window, cx)
                                            })
                                        },
                                    ),
                                ))
                            }),
                    )
            })
    }
}
