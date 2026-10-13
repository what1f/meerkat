use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement,
    RenderOnce, SharedString, Stateful, Styled, Text, Window, div, px, relative, rgb, rgba,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    select::{Select, SelectDelegate, SelectItem, SelectState},
    spinner::Spinner,
};

use crate::{DesktopTheme, FONT_MONO, RADIUS_CARD, RADIUS_CONTROL};

pub const CONTROL_HEIGHT: f32 = 40.;
pub const BUTTON_HEIGHT: f32 = 32.;
const BUTTON_LABEL_LINE_HEIGHT: f32 = 1.25;

/// The only constructors business UI should use for ordinary buttons.
pub struct DesktopButton;

impl DesktopButton {
    pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        let id = id.into();
        Self::sized(
            identified_button(id.clone())
                .primary()
                .child(button_label(&id, label.into())),
        )
    }

    pub fn primary_content(id: impl Into<ElementId>, child: impl IntoElement) -> Button {
        Self::sized(identified_button(id).primary().child(child))
    }

    pub fn primary_loading(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        loading: bool,
    ) -> Button {
        let id = id.into();
        Self::sized(
            identified_button(id.clone())
                .primary()
                .child(loading_button_label(&id, label.into(), loading)),
        )
        .loading(loading)
    }

    pub fn outline(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        let id = id.into();
        Self::sized(
            identified_button(id.clone())
                .outline()
                .child(button_label(&id, label.into())),
        )
    }

    pub fn outline_content(id: impl Into<ElementId>, child: impl IntoElement) -> Button {
        Self::sized(identified_button(id).outline().child(child))
    }

    pub fn outline_loading(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        loading: bool,
    ) -> Button {
        let id = id.into();
        Self::sized(
            identified_button(id.clone())
                .outline()
                .child(loading_button_label(&id, label.into(), loading)),
        )
        .loading(loading)
    }

    pub fn ghost(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        let id = id.into();
        Self::sized(
            identified_button(id.clone())
                .ghost()
                .child(button_label(&id, label.into())),
        )
    }

    pub fn ghost_content(id: impl Into<ElementId>, child: impl IntoElement) -> Button {
        Self::sized(identified_button(id).ghost().child(child))
    }

    pub fn icon(id: impl Into<ElementId>) -> Button {
        identified_button(id)
            .ghost()
            .size(px(BUTTON_HEIGHT))
            .rounded(px(RADIUS_CONTROL))
            .compact()
    }

    pub fn sized(button: Button) -> Button {
        button
            .h(px(BUTTON_HEIGHT))
            .min_w(px(56.))
            .px(px(14.))
            .rounded(px(RADIUS_CONTROL))
    }
}

fn identified_button(id: impl Into<ElementId>) -> Button {
    let id = id.into();
    let selector = match &id {
        ElementId::Name(name) => name.to_string(),
        other => format!("{other:?}"),
    };
    Button::new(id).debug_selector(move || selector)
}

fn button_label(id: &ElementId, label: SharedString) -> impl IntoElement {
    let label_id = ElementId::Name(format!("desktop-button-label-{id:?}").into());
    div()
        .line_height(relative(BUTTON_LABEL_LINE_HEIGHT))
        .child(Text::new(label_id, label))
}

fn loading_button_label(id: &ElementId, label: SharedString, loading: bool) -> impl IntoElement {
    let selector = match id {
        ElementId::Name(name) => name.to_string(),
        other => format!("{other:?}"),
    };
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .when(loading, |content| {
            content.child(
                div()
                    .debug_selector(move || format!("desktop-button-loading-{selector}"))
                    .flex()
                    .items_center()
                    .child(Spinner::new().with_size(px(14.))),
            )
        })
        .child(button_label(id, label))
}

pub struct DesktopInput;

impl DesktopInput {
    pub fn new(state: &Entity<InputState>) -> Input {
        Input::new(state).h(px(CONTROL_HEIGHT))
    }

    pub fn large(state: &Entity<InputState>) -> Input {
        Input::new(state).large()
    }

    pub fn mono_large(state: &Entity<InputState>) -> Input {
        Self::large(state).font_family(FONT_MONO)
    }
}

pub struct DesktopTextarea;

impl DesktopTextarea {
    pub fn new(state: &Entity<TextareaState>) -> Textarea {
        Textarea::new(state)
    }

    pub fn mono(state: &Entity<TextareaState>) -> Textarea {
        Self::new(state).font_family(FONT_MONO)
    }
}

pub struct DesktopSelect;

impl DesktopSelect {
    pub fn new<D>(state: &Entity<SelectState<D>>) -> Select<D>
    where
        D: SelectDelegate + 'static,
        <D::Item as SelectItem>::Value: PartialEq + Clone,
    {
        Select::new(state).h(px(CONTROL_HEIGHT))
    }

    pub fn large<D>(state: &Entity<SelectState<D>>) -> Select<D>
    where
        D: SelectDelegate + 'static,
        <D::Item as SelectItem>::Value: PartialEq + Clone,
    {
        Select::new(state).large()
    }

    pub fn mono_large<D>(state: &Entity<SelectState<D>>) -> Select<D>
    where
        D: SelectDelegate + 'static,
        <D::Item as SelectItem>::Value: PartialEq + Clone,
    {
        Self::large(state).font_family(FONT_MONO)
    }
}

pub struct DesktopCheckbox;

impl DesktopCheckbox {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Checkbox {
        Checkbox::new(id).label(label.into())
    }
}

#[derive(IntoElement)]
pub struct DesktopFormField {
    theme: DesktopTheme,
    label: SharedString,
    hint: Option<SharedString>,
    error: Option<SharedString>,
    child: AnyElement,
}

impl DesktopFormField {
    pub fn new(
        theme: DesktopTheme,
        label: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            theme,
            label: label.into(),
            hint: None,
            error: None,
            child: child.into_any_element(),
        }
    }

    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn error(mut self, error: Option<impl Into<SharedString>>) -> Self {
        self.error = error.map(Into::into);
        self
    }
}

impl RenderOnce for DesktopFormField {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let theme = self.theme;
        let has_error = self.error.is_some();
        let error = self.error;
        let hint = self.hint;
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme.text_secondary)
                    .child(self.label),
            )
            .child(
                div()
                    .w_full()
                    .rounded(px(RADIUS_CONTROL + 1.))
                    .when(has_error, |field| {
                        field.border_1().border_color(rgb(0xdc2626))
                    })
                    .child(self.child),
            )
            .when_some(error, |field, error| {
                field.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(0xdc2626))
                        .child(error),
                )
            })
            .when(!has_error, |field| {
                field.when_some(hint, |field, hint| {
                    field.child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.text_tertiary)
                            .child(hint),
                    )
                })
            })
    }
}

#[derive(Clone, Copy)]
pub enum DesktopBannerTone {
    Neutral,
    Error,
}

#[derive(IntoElement)]
pub struct DesktopBanner {
    theme: DesktopTheme,
    message: SharedString,
    tone: DesktopBannerTone,
}

impl DesktopBanner {
    pub fn neutral(theme: DesktopTheme, message: impl Into<SharedString>) -> Self {
        Self {
            theme,
            message: message.into(),
            tone: DesktopBannerTone::Neutral,
        }
    }

    pub fn error(theme: DesktopTheme, message: impl Into<SharedString>) -> Self {
        Self {
            theme,
            message: message.into(),
            tone: DesktopBannerTone::Error,
        }
    }
}

impl RenderOnce for DesktopBanner {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let theme = self.theme;
        let error = matches!(self.tone, DesktopBannerTone::Error);
        div()
            .p(px(12.))
            .rounded(px(12.))
            .border_1()
            .border_color(if error {
                gpui::rgb(0xdc2626).into()
            } else {
                theme.border
            })
            .bg(if error {
                gpui::rgba(0xff00000a).into()
            } else {
                theme.raised
            })
            .text_color(if error {
                gpui::rgb(0xb91c1c).into()
            } else {
                theme.text_secondary
            })
            .child(self.message)
    }
}

pub fn desktop_card(theme: DesktopTheme) -> Div {
    div()
        .rounded(px(RADIUS_CARD))
        .border_1()
        .border_color(theme.border)
        .bg(theme.elevated)
        .shadow_lg()
}

/// A settings group is a quiet bordered surface. Unlike content cards it never
/// adds elevation or a drop shadow.
pub fn desktop_settings_group(theme: DesktopTheme) -> Div {
    div()
        .w_full()
        .rounded(px(RADIUS_CARD))
        .border_1()
        .border_color(theme.border)
        .bg(theme.content)
        .overflow_hidden()
        .flex()
        .flex_col()
}

pub fn desktop_settings_section_title(
    title: impl Into<SharedString>,
    first: bool,
) -> impl IntoElement {
    div()
        .mt(px(if first { 18. } else { 32. }))
        .mb(px(10.))
        .text_size(px(14.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(title.into())
}

pub fn desktop_settings_row(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
    theme: DesktopTheme,
) -> impl IntoElement {
    div()
        .w_full()
        .min_h(px(76.))
        .px(px(18.))
        .py(px(14.))
        .flex()
        .items_center()
        .gap(px(24.))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(title.into()),
                )
                .child(
                    div()
                        .mt(px(3.))
                        .text_size(px(12.))
                        .text_color(theme.text_secondary)
                        .child(description.into()),
                ),
        )
        .child(div().flex_none().child(control))
}

pub fn desktop_choice(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    theme: DesktopTheme,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px(px(11.))
        .rounded(px(RADIUS_CONTROL))
        .bg(if selected {
            theme.selected
        } else {
            rgba(0x00000000).into()
        })
        .text_color(if selected {
            theme.text
        } else {
            theme.text_secondary
        })
        .hover(move |button| button.bg(theme.hover))
        .cursor_pointer()
        .flex()
        .items_center()
        .child(label.into())
}

#[derive(IntoElement)]
pub struct DesktopChip {
    theme: DesktopTheme,
    leading: AnyElement,
    label: SharedString,
}

impl DesktopChip {
    pub fn new(
        theme: DesktopTheme,
        leading: impl IntoElement,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            theme,
            leading: leading.into_any_element(),
            label: label.into(),
        }
    }
}

impl RenderOnce for DesktopChip {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let theme = self.theme;
        div()
            .max_w(px(170.))
            .px(px(9.))
            .py(px(5.))
            .rounded_full()
            .bg(theme.raised)
            .flex()
            .items_center()
            .gap(px(6.))
            .child(self.leading)
            .child(div().truncate().text_size(px(12.)).child(self.label))
    }
}

#[derive(IntoElement)]
pub struct DesktopCenteredMessage {
    theme: DesktopTheme,
    message: SharedString,
}

#[derive(IntoElement)]
pub struct DesktopSteps {
    theme: DesktopTheme,
    labels: Vec<SharedString>,
    current: usize,
}

impl DesktopSteps {
    pub fn new(theme: DesktopTheme, labels: &[&str], current: usize) -> Self {
        Self {
            theme,
            labels: labels
                .iter()
                .map(|label| SharedString::from(*label))
                .collect(),
            current,
        }
    }
}

impl RenderOnce for DesktopSteps {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let theme = self.theme;
        let mut row = div().mt(px(12.)).flex().items_center().gap(px(8.));
        for (index, label) in self.labels.into_iter().enumerate() {
            if index > 0 {
                row = row.child(div().w(px(20.)).h(px(1.)).bg(if index <= self.current {
                    gpui::rgb(0x86c89a).into()
                } else {
                    theme.border
                }));
            }
            let complete = index < self.current;
            row = row.child(
                div()
                    .px(px(9.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if complete {
                        gpui::rgb(0x86c89a).into()
                    } else if index == self.current {
                        theme.border_strong
                    } else {
                        gpui::rgba(0x00000000).into()
                    })
                    .bg(if complete {
                        gpui::rgba(0x22a44714).into()
                    } else if index == self.current {
                        theme.selected
                    } else {
                        theme.raised
                    })
                    .text_size(px(12.))
                    .text_color(if complete {
                        gpui::rgb(0x238636).into()
                    } else if index == self.current {
                        theme.text
                    } else {
                        theme.text_tertiary
                    })
                    .child(if complete {
                        format!("✓ {label}").into()
                    } else {
                        label
                    }),
            );
        }
        row
    }
}

impl DesktopCenteredMessage {
    pub fn new(theme: DesktopTheme, message: impl Into<SharedString>) -> Self {
        Self {
            theme,
            message: message.into(),
        }
    }
}

impl RenderOnce for DesktopCenteredMessage {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        div()
            .min_h(px(420.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(self.theme.text_secondary)
            .child(self.message)
    }
}

#[derive(IntoElement)]
pub struct DesktopEmptyState {
    theme: DesktopTheme,
    title: SharedString,
    description: SharedString,
    action: Option<AnyElement>,
}

impl DesktopEmptyState {
    pub fn new(
        theme: DesktopTheme,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
    ) -> Self {
        Self {
            theme,
            title: title.into(),
            description: description.into(),
            action: None,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for DesktopEmptyState {
    fn render(self, _: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        let theme = self.theme;
        div()
            .min_h(px(420.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.title),
            )
            .child(
                div()
                    .max_w(px(440.))
                    .text_center()
                    .text_color(theme.text_secondary)
                    .child(self.description),
            )
            .when_some(self.action, |state, action| state.child(action))
    }
}
