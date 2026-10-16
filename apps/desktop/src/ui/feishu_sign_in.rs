//! Feishu sign-in by QR code, so Agents can be created without another scan.

use super::*;

const CONSOLE_LOGIN_INTERFACE: &str = "feishu.workspace.console_login";

#[derive(Clone)]
pub(super) struct FeishuSignIn {
    pub(super) workspace_id: String,
    pub(super) qr_code: String,
    pub(super) scanned: bool,
}

impl LankideApp {
    pub(super) fn begin_feishu_sign_in(&mut self, workspace_id: String, cx: &mut Context<Self>) {
        self.error = None;
        self.request_feishu_sign_in(workspace_id, "start", cx);
    }

    fn request_feishu_sign_in(
        &mut self,
        workspace_id: String,
        action: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.onboarding_poll_epoch = self.onboarding_poll_epoch.wrapping_add(1);
        let epoch = self.onboarding_poll_epoch;
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            if action == "poll" {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let waiting = this
                    .update(cx, |this, _| {
                        this.onboarding_poll_epoch == epoch
                            && matches!(&this.modal, Some(Modal::FeishuSignIn(_)))
                    })
                    .unwrap_or(false);
                if !waiting {
                    return;
                }
            }
            let result = client
                .invoke_provider_interface(
                    &workspace_id,
                    CONSOLE_LOGIN_INTERFACE,
                    json!({"action": action}),
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.onboarding_poll_epoch != epoch {
                    return;
                }
                let showing = matches!(&this.modal, Some(Modal::FeishuSignIn(_)));
                if action == "poll" && !showing {
                    return;
                }
                let state = result
                    .as_ref()
                    .ok()
                    .and_then(|value| value.get("state").and_then(Value::as_str));
                let qr_code = result
                    .as_ref()
                    .ok()
                    .and_then(|value| value.get("qr_code").and_then(Value::as_str))
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned);
                match (state, qr_code) {
                    (Some("waiting" | "scanned"), Some(qr_code)) => {
                        this.error = None;
                        this.modal = Some(Modal::FeishuSignIn(FeishuSignIn {
                            workspace_id: workspace_id.clone(),
                            qr_code,
                            scanned: state == Some("scanned"),
                        }));
                        this.request_feishu_sign_in(workspace_id, "poll", cx);
                    }
                    (Some("ready"), _) => {
                        if showing {
                            this.modal = None;
                        }
                    }
                    _ if showing => {
                        // Polling stops here; the user can ask for a new code.
                        this.error = Some(match result {
                            Err(error) => UiMessage::client(error),
                            Ok(_) => UiMessage::localized(
                                "二维码已失效，请刷新。",
                                "The QR code expired. Refresh it.",
                            ),
                        });
                    }
                    // Without a sign-in, Agent creation asks for it instead.
                    _ => {}
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    pub(super) fn feishu_sign_in_dialog(
        &self,
        dialog: gpui_component::dialog::Dialog,
        app: Entity<LankideApp>,
        state: FeishuSignIn,
        window: &mut Window,
    ) -> gpui_component::dialog::Dialog {
        let language = self.language;
        let theme = self.theme();
        let error = self.error.as_ref().map(|message| message.text(language));
        let cancel_app = app.clone();
        let refresh_app = app.clone();
        let qr_code = qr_code_image(&state.qr_code);
        let dialog_height = px(if error.is_some() { 560. } else { 500. })
            .min((window.viewport_size().height - px(48.)).max(px(390.)));
        let centered_top = ((window.viewport_size().height - dialog_height) * 0.5).max(px(64.));
        DesktopDialog::from(dialog)
            .w(px(440.))
            .h(dialog_height)
            .margin_top(px(0.))
            .mt(centered_top)
            .overlay(true)
            .overlay_closable(false)
            .on_cancel(move |_, _, cx| {
                cancel_app.update(cx, |this, cx| {
                    this.onboarding_poll_epoch = this.onboarding_poll_epoch.wrapping_add(1);
                    this.modal = None;
                    this.error = None;
                    cx.notify();
                });
                true
            })
            .content(move |content, window, _| {
                let workspace_id = state.workspace_id.clone();
                let mut body = div()
                    .flex_1()
                    .min_h_0()
                    .px(px(22.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.));
                if let Some(error) = &error {
                    body = body.child(DesktopBanner::error(theme, error));
                }
                body = body
                    .when_some(qr_code.clone(), |panel, (qr_code, edge)| {
                        panel.child(
                            img(qr_code)
                                .size(px(edge))
                                .flex_none()
                                .object_fit(ObjectFit::Contain),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .text_color(theme.text_secondary)
                            .child(if state.scanned {
                                language.text("在飞书中确认登录", "Confirm sign-in in Feishu")
                            } else {
                                language.text("使用飞书扫码登录", "Scan with Feishu to sign in")
                            })
                            .when(error.is_none(), |row| {
                                row.child(
                                    Spinner::new()
                                        .with_size(px(14.))
                                        .color(theme.text_secondary),
                                )
                            }),
                    );
                let mut footer = DesktopDialogFooter::new().child(desktop_dialog_cancel(
                    "close-feishu-sign-in",
                    language.text("稍后登录", "Sign in later"),
                ));
                if error.is_some() {
                    footer = footer.child(desktop_dialog_button(
                        "refresh-feishu-sign-in",
                        DesktopButton::primary(
                            "refresh-feishu-sign-in",
                            language.text("刷新", "Refresh"),
                        )
                        .on_click(window.listener_for(
                            &refresh_app,
                            move |this, _, _, cx| {
                                this.begin_feishu_sign_in(workspace_id.clone(), cx);
                            },
                        )),
                    ));
                }
                content
                    .min_h_0()
                    .overflow_hidden()
                    .child(DesktopDialogHeader::new(
                        language.text("登录飞书", "Sign in to Feishu"),
                    ))
                    .child(body)
                    .child(footer)
            })
    }
}
