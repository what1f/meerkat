use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, px,
};
use gpui_component::{
    IconName,
    button::{Button, ButtonVariants as _},
    dialog::{Dialog, DialogClose},
};

use crate::{DesktopButton, RADIUS_MODAL};

/// Applies the shared desktop surface treatment to a gpui-component dialog.
/// Business UIs should compose the header and footer below instead of styling
/// the dialog surface themselves.
pub trait DesktopDialogExt {
    fn desktop(self) -> Self;
}

pub struct DesktopDialog;

impl DesktopDialog {
    pub fn new(cx: &mut App) -> Dialog {
        Dialog::new(cx).desktop()
    }

    pub fn from(dialog: Dialog) -> Dialog {
        dialog.desktop()
    }
}

impl DesktopDialogExt for Dialog {
    fn desktop(self) -> Self {
        self.close_button(false).p_0().rounded(px(RADIUS_MODAL))
    }
}

#[derive(IntoElement)]
pub struct DesktopDialogHeader {
    title: SharedString,
    closable: bool,
    children: Vec<AnyElement>,
}

impl DesktopDialogHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            closable: true,
            children: Vec::new(),
        }
    }

    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

impl ParentElement for DesktopDialogHeader {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for DesktopDialogHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(
                div()
                    .h(px(68.))
                    .px(px(22.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(self.title),
                    )
                    .child(if self.closable {
                        div()
                            .debug_selector(|| "desktop-dialog-close".into())
                            .size(px(32.))
                            .child(
                                DialogClose::new().child(
                                    Button::new("desktop-dialog-close")
                                        .ghost()
                                        .size(px(32.))
                                        .rounded(px(8.))
                                        .icon(IconName::Close),
                                ),
                            )
                            .into_any_element()
                    } else {
                        div().size(px(32.)).into_any_element()
                    }),
            )
            .when(!self.children.is_empty(), |header| {
                header.child(div().px(px(22.)).pb(px(14.)).children(self.children))
            })
    }
}

#[derive(IntoElement)]
pub struct DesktopDialogFooter {
    children: Vec<AnyElement>,
    compact: bool,
}

impl DesktopDialogFooter {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            compact: false,
        }
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
}

impl Default for DesktopDialogFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for DesktopDialogFooter {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for DesktopDialogFooter {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .mt_auto()
            .h(px(if self.compact { 64. } else { 84. }))
            .px(px(22.))
            .pt(px(if self.compact { 16. } else { 8. }))
            .pb(px(if self.compact { 16. } else { 24. }))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .children(self.children)
    }
}

pub fn desktop_dialog_button(selector: &'static str, button: Button) -> impl IntoElement {
    div()
        .debug_selector(move || selector.into())
        .child(DesktopButton::sized(button).min_w(px(64.)).px(px(16.)))
}

pub fn desktop_dialog_cancel(id: &'static str, label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .debug_selector(move || id.into())
        .h(px(32.))
        .min_w(px(56.))
        .child(DialogClose::new().child(DesktopButton::ghost(id, label).px(px(12.))))
}
