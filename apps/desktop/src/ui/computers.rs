use super::*;
use gpui_component::scroll::ScrollableElement as _;
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
        // Discovery advertises the Computer listener. Loopback and wildcard
        // addresses cannot be copied as destinations on another machine.
        self.pairing_address = std::fs::read_to_string(home.join("run/computer-link.url"))
            .ok()
            .and_then(|address| {
                let uri = address.trim().parse::<tonic::codegen::http::Uri>().ok()?;
                let host = uri.host()?;
                let ip = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>();
                if host.eq_ignore_ascii_case("localhost")
                    || host.ends_with(".localhost")
                    || ip.is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
                {
                    return None;
                }
                Some(address.trim().to_owned())
            });
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
        self.pairing_loading = true;
        self.pairing_request += 1;
        let request = self.pairing_request;
        self.error = None;
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client.create_pairing_code().await;
            let _ = this.update(cx, |this, cx| {
                if this.pairing_request != request
                    || !matches!(this.modal, Some(Modal::PairComputer))
                {
                    return;
                }
                this.pairing_loading = false;
                match result {
                    Ok(code) => this.pairing_code = Some(code),
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                this.notify_dialog_changed(cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn computer_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(desktop_settings_section_title(
                self.tr("电脑", "Computers"),
                false,
            ))
            .children(self.computers.iter().map(|computer| {
                div()
                    .debug_selector({
                        let id = computer.id.clone();
                        move || format!("computer-{id}")
                    })
                    .flex()
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
                DesktopButton::outline("pair-computer", self.tr("添加电脑", "Add Computer"))
                    .debug_selector(|| "pair-computer".into())
                    .on_click(cx.listener(|this, _, _, cx| this.open_pair_computer(cx))),
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
        let code = self.pairing_code.as_ref().map(|code| code.code.clone());
        let command = code.as_ref().and_then(|code| {
            self.pairing_address
                .as_ref()
                .map(|url| format!("lankide-computer-daemon pair {url} {code}"))
        });
        let theme = self.theme();
        let error = self.error.as_ref().map(|e| e.text(language));
        let loading = self.pairing_loading;
        DesktopDialog::from(dialog)
            .width(px(560.))
            .h(px(if command.is_some() { 310. } else { 250. }))
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
                content
                    .child(DesktopDialogHeader::new(
                        language.text("添加电脑", "Add Computer"),
                    ))
                    .child(
                        div()
                            .px(px(22.))
                            .flex()
                            .flex_col()
                            .gap(px(14.))
                            .child(language.text(
                                "在另一台电脑上输入配对码。",
                                "Enter this code on the other Computer.",
                            ))
                            .when(loading, |body| body.child(Spinner::new()))
                            .when_some(code.clone(), |body, code| {
                                body.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .debug_selector(|| "pairing-code".into())
                                                .font_family(desktop_ui::FONT_MONO)
                                                .text_size(px(14.))
                                                .flex()
                                                .flex_col()
                                                .children(
                                                    code.chars()
                                                        .collect::<Vec<_>>()
                                                        .chunks(32)
                                                        .map(|line| {
                                                            div().child(
                                                                line.iter().collect::<String>(),
                                                            )
                                                        }),
                                                ),
                                        )
                                        .child(
                                            DesktopButton::outline(
                                                "copy-pairing-code",
                                                language.text("复制", "Copy"),
                                            )
                                            .on_click(
                                                move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(code.clone()),
                                                    )
                                                },
                                            ),
                                        ),
                                )
                            })
                            .when_some(command.clone(), |body, command| {
                                body.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .w_full()
                                                .overflow_x_scrollbar()
                                                .font_family(desktop_ui::FONT_MONO)
                                                .text_size(px(12.))
                                                .child(command.clone()),
                                        )
                                        .child(
                                            DesktopButton::outline(
                                                "copy-pairing-command",
                                                language.text("复制命令", "Copy command"),
                                            )
                                            .on_click(
                                                move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(command.clone()),
                                                    )
                                                },
                                            ),
                                        ),
                                )
                            })
                            .when_some(error.clone(), |body, error| {
                                body.child(DesktopBanner::error(theme, error))
                            }),
                    )
                    .child(DesktopDialogFooter::new().child(desktop_dialog_cancel(
                        "close-pairing",
                        language.text("关闭", "Close"),
                    )))
            })
    }
}
