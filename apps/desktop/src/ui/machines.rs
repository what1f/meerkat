use super::*;
use gpui_component::text::{TextView, TextViewStyle};
use std::path::Path;

/// A device glyph with a presence badge in its corner. Offline Machines dim
/// the glyph and gray the badge. The badge ring matches the surface behind it.
fn machine_glyph(theme: DesktopTheme, available: bool, size: f32, surface: Hsla) -> Div {
    let badge = div()
        .absolute()
        .right(px(-3.))
        .bottom(px(-3.))
        .size(px((size * 0.34).round()))
        .rounded_full()
        .border_2()
        .border_color(surface);
    let badge = if available {
        badge.bg(theme.success)
    } else {
        badge.bg(theme.text_tertiary)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px((size * 0.28).round()))
        .bg(theme.raised)
        .child(
            Icon::default()
                .path("icons/lucide/monitor.svg")
                .size(px((size * 0.5).round()))
                .text_color(if available {
                    theme.text_secondary
                } else {
                    theme.text_tertiary
                }),
        )
        .child(badge)
}

pub(crate) fn local_machine_id(home: &Path) -> Option<String> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(home.join("machine-credentials.json")).ok()?).ok()?;
    value.get("machine_id")?.as_str().map(str::to_owned)
}

#[derive(Clone)]
pub(super) struct MachineChoice {
    id: String,
    name: SharedString,
    available: bool,
    theme: DesktopTheme,
}
impl MachineChoice {
    fn label(&self, surface: Hsla) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(machine_glyph(self.theme, self.available, 22., surface))
            .child(
                div()
                    .debug_selector({
                        let name = self.name.clone();
                        move || format!("machine-choice-name-{name}")
                    })
                    .child(self.name.clone()),
            )
    }
}
impl SelectItem for MachineChoice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.name.clone()
    }
    fn display_title(&self) -> Option<AnyElement> {
        Some(self.label(self.theme.content).into_any_element())
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        self.label(self.theme.elevated)
            .debug_selector(move || format!("machine-choice-{id}"))
    }
    fn value(&self) -> &String {
        &self.id
    }
}

impl LankideApp {
    pub(super) fn subscribe_machines(&mut self, cx: &mut Context<Self>) {
        if self.machine_events_task.is_some() {
            return;
        }
        let client = self.client.clone();
        self.machine_events_task = Some(cx.spawn(async move |this, cx| {
            loop {
                // Subscribe before reading: changes during the snapshot stay queued.
                if let Ok(mut events) = client.subscribe_management_events(None, 0).await
                    && let Ok(catalog) = client.get_machine_catalog(None).await
                {
                    if this
                        .update(cx, |this, cx| this.apply_machine_catalog(catalog, cx))
                        .is_err()
                    {
                        return;
                    }
                    while let Some(Ok(event)) = events.next().await {
                        if event.kind != lankide_client::ManagementEventKind::MachineCatalogChanged
                        {
                            continue;
                        }
                        match client.get_machine_catalog(None).await {
                            Ok(catalog) => {
                                if this
                                    .update(cx, |this, cx| this.apply_machine_catalog(catalog, cx))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                // Retry only a disconnected subscription. Each connection rediscovers
                // the Runtime and reads a fresh snapshot, including after restart.
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        }));
    }

    fn apply_machine_catalog(
        &mut self,
        catalog: Vec<lankide_client::MachineOption>,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.modal, Some(Modal::PairMachine))
            && !self.pairing_loading
            && self.pairing_connected.is_none()
            && let Some(machine) = catalog.iter().find(|machine| {
                machine.registered
                    && machine.revoked_at.is_none()
                    && machine.available
                    && !self.pairing_known.contains(&machine.id)
            })
        {
            self.pairing_connected = Some(machine.name.clone());
            self.pairing_code_task = None;
            let request = self.pairing_request;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(1200))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.pairing_active(request) {
                        this.modal = None;
                        this.notify_dialog_changed(cx);
                    }
                });
            })
            .detach();
        }
        self.machine_choices_changed = true;
        self.machines = catalog
            .into_iter()
            .filter(|machine| machine.registered && machine.revoked_at.is_none())
            .collect();
        self.notify_dialog_changed(cx);
    }

    fn renew_pairing_code_at_expiry(&mut self, cx: &mut Context<Self>) {
        let Some(code) = self.pairing_code.as_ref() else {
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let delay = Duration::from_millis(code.expires_at.saturating_sub(now).max(0) as u64);
        let request = self.pairing_request;
        let client = self.client.clone();
        self.pairing_code_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            if !this
                .update(cx, |this, _| {
                    this.pairing_active(request)
                        && !this.pairing_editing
                        && this.pairing_connected.is_none()
                })
                .unwrap_or(false)
            {
                return;
            }
            let result = client.create_pairing_code().await;
            let _ = this.update(cx, |this, cx| {
                if !this.pairing_active(request) || this.pairing_connected.is_some() {
                    return;
                }
                match result {
                    Ok(code) => {
                        this.pairing_code = Some(code);
                        this.pairing_copied = false;
                        this.renew_pairing_code_at_expiry(cx);
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                this.notify_dialog_changed(cx);
            });
        }));
    }

    fn start_pairing_hint(&mut self, cx: &mut Context<Self>) {
        self.pairing_waiting_hint = false;
        let request = self.pairing_request;
        self.pairing_hint_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(30))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.pairing_active(request)
                    && !this.pairing_editing
                    && this.pairing_connected.is_none()
                {
                    this.pairing_waiting_hint = true;
                    this.notify_dialog_changed(cx);
                }
            });
        }));
    }

    pub(super) fn read_local_machine(&mut self, home: &Path) {
        self.local_machine_id = local_machine_id(home);
    }

    pub(super) fn update_machine_choices(
        &mut self,
        preferred: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = preferred;
        let selected = selected
            .filter(|id| self.machines.iter().any(|c| &c.id == id))
            .or_else(|| {
                self.local_machine_id
                    .clone()
                    .filter(|id| self.machines.iter().any(|c| &c.id == id))
            })
            .or_else(|| self.machines.first().map(|c| c.id.clone()));
        let choices = self
            .machines
            .iter()
            .map(|machine| MachineChoice {
                id: machine.id.clone(),
                name: machine.name.clone().into(),
                available: machine.available,
                theme: self.theme(),
            })
            .collect();
        self.agent_machine.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_index(None, window, cx);
            if let Some(id) = selected {
                select.set_selected_value(&id, window, cx);
            }
            cx.notify();
        });
        window.refresh();
    }

    pub(super) fn reload_machines(
        &mut self,
        preferred: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.subscribe_machines(cx);
        self.machine_request += 1;
        let request = self.machine_request;
        let client = self.client.clone();
        // Clear stale placement while the new form's catalog is loading.
        self.agent_machine.update(cx, |select, cx| {
            select.set_items(Vec::new(), window, cx);
            select.set_selected_index(None, window, cx);
        });
        cx.spawn_in(window, async move |this, window| {
            let result = client.get_machine_catalog(None).await;
            let _ = this.update_in(window, |this, window, cx| {
                if this.machine_request != request {
                    return;
                }
                match result {
                    Ok(machines) => {
                        this.machines = machines
                            .into_iter()
                            .filter(|c| c.registered && c.revoked_at.is_none())
                            .collect();
                        this.update_machine_choices(preferred, window, cx);
                    }
                    Err(error) => {
                        this.machines.clear();
                        this.error = Some(UiMessage::client(error));
                    }
                }
                this.notify_dialog_changed(cx);
            });
        })
        .detach();
    }

    fn open_pair_machine(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(Modal::PairMachine);
        self.pairing_code = None;
        self.pairing_waiting_hint = false;
        self.pairing_hint_task = None;
        self.pairing_code_task = None;
        self.pairing_known.clear();
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
                let access = client.enable_remote_access().await?;
                let catalog = client.get_machine_catalog(None).await?;
                let code = if access.address.is_some() {
                    Some(client.create_pairing_code().await?)
                } else {
                    None
                };
                Ok::<_, ClientError>((access, catalog, code))
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                if !this.pairing_active(request) {
                    return;
                }
                this.pairing_loading = false;
                match result {
                    Ok((access, catalog, code)) => {
                        this.pairing_known = catalog.iter().map(|c| c.id.clone()).collect();
                        this.machines = catalog
                            .into_iter()
                            .filter(|c| c.registered && c.revoked_at.is_none())
                            .collect();
                        this.pairing_address = access.address;
                        this.pairing_pin = access.cert_sha256;
                        this.pairing_editing = this.pairing_address.is_none();
                        this.pairing_code = code;
                        if this.pairing_code.is_some() {
                            this.start_pairing_hint(cx);
                            this.renew_pairing_code_at_expiry(cx);
                        }
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                this.notify_dialog_changed(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn mark_pairing_copied(&mut self, cx: &mut Context<Self>) {
        self.pairing_copied = true;
        let request = self.pairing_request;
        self.notify_dialog_changed(cx);
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
    }

    fn edit_pairing_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let address = self.pairing_address.clone().unwrap_or_default();
        self.pairing_address_input
            .update(cx, |input, cx| input.set_value(address, window, cx));
        self.pairing_editing = true;
        self.pairing_waiting_hint = false;
        self.pairing_hint_task = None;
        self.error = None;
        self.notify_dialog_changed(cx);
    }

    fn pairing_active(&self, request: u64) -> bool {
        self.pairing_request == request && matches!(self.modal, Some(Modal::PairMachine))
    }

    fn enable_remote_machines(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                        this.start_pairing_hint(cx);
                        this.renew_pairing_code_at_expiry(cx);
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

    fn rename_machine(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(machine) = self.machines.iter().find(|c| c.id == id) else {
            return;
        };
        let name = machine.name.clone();
        self.machine_name_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        self.machine_renaming = Some(id);
        self.error = None;
        cx.notify();
    }

    pub(super) fn save_machine_name(&mut self, cx: &mut Context<Self>) {
        if self.machine_rename_saving {
            return;
        }
        let Some(id) = self.machine_renaming.clone() else {
            return;
        };
        let name = self.machine_name_input.read(cx).value().trim().to_owned();
        self.machine_rename_saving = true;
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client.rename_machine(&id, &name).await;
            let _ = this.update(cx, |this, cx| {
                this.machine_rename_saving = false;
                match result {
                    Ok(()) => {
                        this.machine_renaming = None;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_machine(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(BusyOperation::RemoveMachine);
        self.error = None;
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client.revoke_machine(&id).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => this.modal = None,
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                this.notify_dialog_changed(cx);
            });
        })
        .detach();
        self.notify_dialog_changed(cx);
    }

    pub(super) fn remove_machine_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<Self>,
        id: String,
        _window: &mut Window,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let busy = self.busy == Some(BusyOperation::RemoveMachine);
        let theme = self.theme();
        let error = self.error.as_ref().map(|message| message.text(language));
        let name = self
            .machines
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let title = language
            .text("移除 {name}？", "Remove {name}?")
            .replace("{name}", &name);
        let cancel_app = app.clone();
        DesktopDialog::from(dialog)
            .w(px(440.))
            .h(px(if error.is_some() { 270. } else { 200. }))
            .overlay(true)
            .overlay_closable(!busy)
            .on_cancel(move |_, _, cx| {
                cancel_app.update(cx, |this, cx| {
                    if !busy {
                        this.modal = None;
                        this.error = None;
                        cx.notify();
                    }
                });
                !busy
            })
            .content(move |content, window, _| {
                let id = id.clone();
                content
                    .child(DesktopDialogHeader::new(title.clone()).closable(!busy))
                    .child(
                        div().px(px(22.)).text_color(theme.text_secondary).child(
                            language.text("再次连接需要重新配对。", "Pair again to reconnect."),
                        ),
                    )
                    .when_some(error.clone(), |content, error| {
                        content.child(
                            div()
                                .debug_selector(|| "machine-remove-error".into())
                                .px(px(22.))
                                .child(DesktopBanner::error(theme, error)),
                        )
                    })
                    .child(
                        DesktopDialogFooter::new()
                            .child(desktop_dialog_cancel(
                                "cancel-remove-machine",
                                language.text("取消", "Cancel"),
                            ))
                            .child(desktop_dialog_button(
                                "confirm-remove-machine",
                                DesktopButton::primary_loading(
                                    "confirm-remove-machine",
                                    language.text("移除", "Remove"),
                                    busy,
                                )
                                .on_click(
                                    window.listener_for(&app, move |this, _, _, cx| {
                                        this.remove_machine(id.clone(), cx)
                                    }),
                                ),
                            )),
                    )
            })
    }

    pub(super) fn machine_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme();
        let local = self.local_machine_id.as_deref();
        div()
            .child(
                div()
                    .mt(px(32.))
                    .mb(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(14.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(self.tr("机器", "Machines"))
                    .child(
                        DesktopButton::icon("pair-machine")
                            .debug_selector(|| "pair-machine".into())
                            .tooltip(self.tr("添加机器", "Add Machine"))
                            .child(muted_themed_icon("icons/lucide/plus.svg", 16., theme))
                            .on_click(cx.listener(|this, _, _, cx| this.open_pair_machine(cx))),
                    ),
            )
            .when(!self.machines.is_empty(), |section| {
                section.child(desktop_settings_group(theme).children(
                    self.machines.iter().enumerate().map(|(index, machine)| {
                        let id = machine.id.clone();
                        let group: SharedString = format!("machine-row-{id}").into();
                        let rename_id = id.clone();
                        let remove_id = id.clone();
                        let app = cx.entity();
                        let language = self.language;
                        let removable = local != Some(id.as_str());
                        let editing = self.machine_renaming.as_deref() == Some(id.as_str());
                        let state = if machine.available {
                            "online"
                        } else {
                            "offline"
                        };
                        div()
                            .id(SharedString::from(format!("machine-row-{id}")))
                            .group(group.clone())
                            .debug_selector({
                                let id = id.clone();
                                move || format!("machine-{id}")
                            })
                            .px(px(18.))
                            .py(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .when(index > 0, |row| row.border_t_1().border_color(theme.border))
                            .child(
                                machine_glyph(theme, machine.available, 32., theme.content)
                                    .debug_selector(move || format!("machine-{state}-{id}")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .when(editing, |name| {
                                        name.child(
                                            div()
                                                .debug_selector(|| "machine-name-input".into())
                                                .on_key_down(cx.listener(
                                                    |this, event: &KeyDownEvent, _, cx| {
                                                        if event.keystroke.key == "escape"
                                                            && !this.machine_rename_saving
                                                        {
                                                            this.machine_renaming = None;
                                                            this.error = None;
                                                            cx.stop_propagation();
                                                            cx.notify();
                                                        }
                                                    },
                                                ))
                                                .child(
                                                    DesktopInput::new(&self.machine_name_input)
                                                        .disabled(self.machine_rename_saving),
                                                ),
                                        )
                                    })
                                    .when(!editing, |name| {
                                        name.child(
                                            div()
                                                .truncate()
                                                .when(!machine.available, |name| {
                                                    name.text_color(theme.text_secondary)
                                                })
                                                .debug_selector({
                                                    let name = machine.name.clone();
                                                    move || format!("machine-name-{name}")
                                                })
                                                .child(machine.name.clone()),
                                        )
                                    }),
                            )
                            .when(local == Some(machine.id.as_str()), |row| {
                                row.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.text_tertiary)
                                        .child(self.tr("本机", "This machine")),
                                )
                            })
                            .when(!editing, |row| {
                                row.child(
                                    div()
                                        .invisible()
                                        .group_hover(group, |button| button.visible())
                                        .child(
                                            DesktopButton::icon(SharedString::from(format!(
                                                "machine-menu-{}",
                                                machine.id
                                            )))
                                            .debug_selector({
                                                let id = machine.id.clone();
                                                move || format!("machine-menu-{id}")
                                            })
                                            .tooltip(language.text("更多", "More"))
                                            .child(muted_themed_icon(
                                                "icons/lucide/ellipsis.svg",
                                                16.,
                                                theme,
                                            ))
                                            .dropdown_menu(move |menu, window, _| {
                                                let target = app.clone();
                                                let rename_id = rename_id.clone();
                                                let menu = menu.item(
                                                    PopupMenuItem::new(
                                                        language.text("重命名", "Rename"),
                                                    )
                                                    .on_click(window.listener_for(
                                                        &target,
                                                        move |this, _, window, cx| {
                                                            this.rename_machine(
                                                                rename_id.clone(),
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                                );
                                                if !removable {
                                                    return menu;
                                                }
                                                let target = app.clone();
                                                let remove_id = remove_id.clone();
                                                menu.item(
                                                    PopupMenuItem::new(
                                                        language.text("移除", "Remove"),
                                                    )
                                                    .on_click(window.listener_for(
                                                        &target,
                                                        move |this, _, _, cx| {
                                                            this.error = None;
                                                            this.modal =
                                                                Some(Modal::RemoveMachine(
                                                                    remove_id.clone(),
                                                                ));
                                                            cx.notify();
                                                        },
                                                    )),
                                                )
                                            }),
                                        ),
                                )
                            })
                    }),
                ))
            })
            .into_any_element()
    }

    pub(super) fn pair_machine_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<Self>,
        _window: &mut Window,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        // Shown one argument per line so long values never wrap mid-token;
        // copied as a single line.
        let command_lines = self.pairing_code.as_ref().and_then(|code| {
            Some([
                "lankide-machine pair".to_owned(),
                format!("wss://{}", self.pairing_address.as_ref()?),
                code.code.clone(),
                format!("--cert-sha256 {}", self.pairing_pin.as_ref()?),
            ])
        });
        let editing = self.pairing_editing;
        let loading = self.pairing_loading;
        let copied = self.pairing_copied;
        let connected = self.pairing_connected.clone();
        let waiting_hint = self.pairing_waiting_hint;
        let address_input = self.pairing_address_input.clone();
        let theme = self.theme();
        let error = self.error.as_ref().map(|e| e.text(language));
        let enable_app = app.clone();
        let edit_app = app.clone();
        let copy_app = app.clone();
        DesktopDialog::from(dialog)
            .width(px(660.))
            .overlay(true)
            .on_cancel(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    this.modal = None;
                    this.error = None;
                    this.reload_machines(None, window, cx);
                    cx.notify();
                });
                true
            })
            .content(move |content, _, _| {
                let enable_app = enable_app.clone();
                let edit_app = edit_app.clone();
                let copy_app = copy_app.clone();
                content
                    .child(DesktopDialogHeader::new(
                        language.text("添加机器", "Add Machine"),
                    ))
                    .child(
                        div()
                            .px(px(22.))
                            .pb(px(22.))
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
                            .when(!editing, |body| {
                                body.when_some(command_lines.clone(), |body, lines| {
                                    body.child(div().text_color(theme.text_secondary).child(
                                        language.text(
                                            "在要添加的机器上运行",
                                            "Run this on the machine you want to add",
                                        ),
                                    ))
                                    .child(pairing_command_block(
                                        lines,
                                        copied,
                                        theme,
                                        language,
                                        copy_app.clone(),
                                    ))
                                    .child(
                                        pairing_status_row(
                                            connected.clone(),
                                            waiting_hint,
                                            theme,
                                            language,
                                            edit_app.clone(),
                                        ),
                                    )
                                })
                            })
                            .when(editing, |body| {
                                body.child(
                                    div().flex().justify_end().child(desktop_dialog_button(
                                        "enable-machine-access",
                                        DesktopButton::primary_loading(
                                            "enable-machine-access",
                                            language.text("保存", "Save"),
                                            loading,
                                        )
                                        .disabled(loading)
                                        .on_click(
                                            move |_, window, cx| {
                                                enable_app.update(cx, |this, cx| {
                                                    this.enable_remote_machines(window, cx)
                                                })
                                            },
                                        ),
                                    )),
                                )
                            })
                            .when_some(error.clone(), |body, error| {
                                body.child(DesktopBanner::error(theme, error))
                            }),
                    )
            })
    }
}

fn pairing_command_block(
    lines: [String; 4],
    copied: bool,
    theme: DesktopTheme,
    language: DesktopLanguage,
    app: Entity<LankideApp>,
) -> Div {
    let html = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let indent = if index == 0 { "" } else { "&nbsp;&nbsp;" };
            let tail = if index + 1 == lines.len() { "" } else { " \\" };
            format!("<p>{indent}{line}{tail}</p>")
        })
        .collect::<String>();
    let command = lines.join(" ");
    let icon = if copied {
        "icons/lucide/check.svg"
    } else {
        "icons/lucide/copy.svg"
    };
    div()
        .debug_selector(|| "pairing-command".into())
        .relative()
        .w_full()
        .p(px(14.))
        .rounded(px(desktop_ui::RADIUS_CONTROL))
        .bg(theme.raised)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.text)
        .text_size(px(12.))
        .line_height(px(20.))
        .font_family(desktop_ui::FONT_MONO)
        .child(
            TextView::html("pairing-command-text", html)
                .style(TextViewStyle::default().paragraph_gap(gpui::rems(0.)))
                .selectable(true),
        )
        .child(
            div().absolute().top(px(6.)).right(px(6.)).child(
                DesktopButton::icon("copy-pairing-command")
                    .debug_selector(|| "copy-pairing-command".into())
                    .tooltip(language.text("复制", "Copy"))
                    .child(muted_themed_icon(icon, 14., theme))
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
                        app.update(cx, |this, cx| this.mark_pairing_copied(cx));
                    }),
            ),
        )
}

fn pairing_status_row(
    connected: Option<String>,
    waiting_hint: bool,
    theme: DesktopTheme,
    language: DesktopLanguage,
    app: Entity<LankideApp>,
) -> Div {
    let waiting = connected.is_none();
    let status = connected.unwrap_or_else(|| {
        if waiting_hint {
            language
                .text(
                    "连不上？检查网络和防火墙",
                    "Not connecting? Check network and firewall",
                )
                .into()
        } else {
            language.text("等待连接", "Waiting for connection").into()
        }
    });
    let dot = div()
        .size(px(6.))
        .rounded_full()
        .bg(if waiting {
            theme.text_tertiary
        } else {
            theme.success
        })
        .with_animation(
            "pairing-waiting-dot",
            Animation::new(Duration::from_secs(2)).repeat(),
            move |dot, t| {
                let pulse = 0.4 + 0.6 * (std::f32::consts::PI * t).sin().powi(2);
                dot.opacity(if waiting { pulse } else { 1. })
            },
        );
    let change_address = div()
        .id("change-pairing-address")
        .role(gpui::Role::Button)
        .debug_selector(|| "change-pairing-address".into())
        .cursor_pointer()
        .text_color(theme.text_secondary)
        .hover(|button| button.text_color(theme.text))
        .child(language.text("更改地址", "Change address"))
        .on_click(move |_, window, cx| {
            app.update(cx, |this, cx| this.edit_pairing_address(window, cx))
        });
    div()
        .flex()
        .items_center()
        .justify_between()
        .text_size(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_color(theme.text_tertiary)
                .child(dot)
                .child(
                    div()
                        .when(!waiting, |status| {
                            status.debug_selector(|| "pairing-connected".into())
                        })
                        .when(waiting && waiting_hint, |status| {
                            status.debug_selector(|| "pairing-waiting-hint".into())
                        })
                        .child(status),
                ),
        )
        .child(change_address)
}
