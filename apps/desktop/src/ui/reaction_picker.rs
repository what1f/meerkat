use std::rc::Rc;

use crate::assets::reaction_asset_path;
use desktop_ui::{DesktopButton, DesktopTheme};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, App, InteractiveElement as _, IntoElement, ObjectFit, ParentElement, RenderOnce,
    Styled, StyledImage as _, Window, div, img, px, rgba,
};
use gpui_component::Icon;
use gpui_component::popover::Popover;
use gpui_component::scroll::ScrollableElement as _;
use lankide_client::ReactionOption;

type SelectHandler = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct ReactionPicker {
    options: Vec<ReactionOption>,
    selected: Option<String>,
    loading: bool,
    on_select: SelectHandler,
    theme: DesktopTheme,
}

impl ReactionPicker {
    pub fn new(
        options: Vec<ReactionOption>,
        selected: Option<String>,
        loading: bool,
        theme: DesktopTheme,
        on_select: impl Fn(Option<String>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            options,
            selected,
            loading,
            on_select: Rc::new(on_select),
            theme,
        }
    }
}

impl RenderOnce for ReactionPicker {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let selected_option = self
            .selected
            .as_deref()
            .and_then(|selected| self.options.iter().find(|option| option.id == selected))
            .cloned();
        let trigger = DesktopButton::icon("reaction-picker-trigger-button")
            .outline()
            .debug_selector(|| "reaction-picker-trigger".into())
            .size(px(36.))
            .rounded_full()
            .child(reaction_trigger_visual(
                selected_option.as_ref(),
                self.loading,
                32.,
                self.theme,
            ));
        let options = self.options;
        let selected = self.selected;
        let on_select = self.on_select;
        let theme = self.theme;

        Popover::new("reaction-picker-popover")
            .anchor(Anchor::BottomRight)
            .appearance(false)
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity();
                let disable_handler = on_select.clone();
                let disable_popover = popover.clone();
                let mut grid = div()
                    .w_full()
                    .p(px(12.))
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(reaction_choice(
                        "reaction-none",
                        None,
                        selected.is_none(),
                        theme,
                        move |_, window, cx| {
                            disable_popover.update(cx, |state, cx| {
                                state.dismiss(window, cx);
                            });
                            (disable_handler)(None, window, cx);
                        },
                    ));

                for option in options.iter().cloned() {
                    if !option.available {
                        continue;
                    }
                    let choice_handler = on_select.clone();
                    let choice_popover = popover.clone();
                    let choice_id = option.id.clone();
                    grid = grid.child(reaction_choice(
                        format!("reaction-{}", option.id.to_ascii_lowercase()),
                        Some(option.clone()),
                        selected.as_deref() == Some(option.id.as_str()),
                        theme,
                        move |_, window, cx| {
                            choice_popover.update(cx, |state, cx| {
                                state.dismiss(window, cx);
                            });
                            (choice_handler)(Some(choice_id.clone()), window, cx);
                        },
                    ));
                }
                div()
                    .debug_selector(|| "reaction-picker-content".into())
                    .w(px(280.))
                    .h(px(232.))
                    .rounded(px(15.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.elevated)
                    .shadow_lg()
                    .overflow_hidden()
                    .occlude()
                    .child(
                        div()
                            .debug_selector(|| "reaction-picker-scroll".into())
                            .size_full()
                            .overflow_y_scrollbar()
                            .child(grid),
                    )
            })
    }
}

fn reaction_trigger_visual(
    option: Option<&ReactionOption>,
    loading: bool,
    size: f32,
    theme: DesktopTheme,
) -> impl IntoElement {
    if option.is_none() && !loading {
        return div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::default()
                    .path("icons/lucide/smile.svg")
                    .size(px(19.))
                    .text_color(theme.text),
            )
            .into_any_element();
    }
    reaction_visual(option, loading, size).into_any_element()
}

fn reaction_choice(
    id: impl Into<String>,
    option: Option<ReactionOption>,
    selected: bool,
    theme: DesktopTheme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let debug_id = id.clone();
    let disabled_choice = option.is_none();
    let tooltip = option.as_ref().map(|option| option.name.clone());
    div()
        .debug_selector(move || debug_id.clone().into())
        .size(px(36.))
        .flex_none()
        .child(
            DesktopButton::icon(id)
                .size(px(36.))
                .rounded(px(11.))
                .border_1()
                .border_color(if selected {
                    theme.focus
                } else {
                    rgba(0x00000000).into()
                })
                .bg(if selected {
                    theme.selected
                } else {
                    rgba(0x00000000).into()
                })
                .when(disabled_choice, |button| {
                    button
                        .outline()
                        .border_dashed()
                        .border_color(theme.border_strong)
                })
                .when_some(tooltip, |button, tooltip| button.tooltip(tooltip))
                .on_click(on_click)
                .child(reaction_visual(option.as_ref(), false, 26.)),
        )
}

fn reaction_visual(option: Option<&ReactionOption>, loading: bool, size: f32) -> impl IntoElement {
    if loading {
        return div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .child("…")
            .into_any_element();
    }
    let Some(option) = option else {
        return div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(18.))
            .child("−")
            .into_any_element();
    };
    if let Some(path) = reaction_asset_path(&option.id) {
        img(path)
            .size(px(size))
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    } else {
        div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(18.))
            .child(if option.emoji.is_empty() {
                option.name.clone()
            } else {
                option.emoji.clone()
            })
            .into_any_element()
    }
}
