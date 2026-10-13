use gpui::prelude::{FluentBuilder as _, StatefulInteractiveElement as _};
use gpui::*;
use gpui_component::{Root, h_resizable, resizable_panel};

use crate::{DesktopTheme, FONT_UI};

pub const MACOS_TITLEBAR_SAFE_HEIGHT: f32 = 44.;

#[derive(IntoElement)]
pub struct DesktopShell {
    id: ElementId,
    theme: DesktopTheme,
    sidebar: AnyElement,
    pane: Option<AnyElement>,
    content: AnyElement,
    sidebar_width: Pixels,
    sidebar_min_width: Pixels,
    sidebar_max_width: Pixels,
    pane_width: Pixels,
    pane_min_width: Pixels,
    pane_max_width: Pixels,
    render_dialog_layer: bool,
}

impl DesktopShell {
    pub fn two_pane(
        theme: DesktopTheme,
        sidebar: impl IntoElement,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: "desktop-shell-two-pane".into(),
            theme,
            sidebar: sidebar.into_any_element(),
            pane: None,
            content: content.into_any_element(),
            sidebar_width: px(240.),
            sidebar_min_width: px(180.),
            sidebar_max_width: px(420.),
            pane_width: px(280.),
            pane_min_width: px(220.),
            pane_max_width: px(520.),
            render_dialog_layer: true,
        }
    }

    pub fn three_pane(
        theme: DesktopTheme,
        sidebar: impl IntoElement,
        pane: impl IntoElement,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: "desktop-shell-three-pane".into(),
            theme,
            sidebar: sidebar.into_any_element(),
            pane: Some(pane.into_any_element()),
            content: content.into_any_element(),
            sidebar_width: px(240.),
            sidebar_min_width: px(180.),
            sidebar_max_width: px(420.),
            pane_width: px(280.),
            pane_min_width: px(220.),
            pane_max_width: px(520.),
            render_dialog_layer: true,
        }
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn sidebar_widths(mut self, initial: Pixels, min: Pixels, max: Pixels) -> Self {
        self.sidebar_width = initial;
        self.sidebar_min_width = min;
        self.sidebar_max_width = max;
        self
    }

    pub fn pane_widths(mut self, initial: Pixels, min: Pixels, max: Pixels) -> Self {
        self.pane_width = initial;
        self.pane_min_width = min;
        self.pane_max_width = max;
        self
    }

    pub fn render_dialog_layer(mut self, render: bool) -> Self {
        self.render_dialog_layer = render;
        self
    }
}

impl RenderOnce for DesktopShell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let dialog_layer = self
            .render_dialog_layer
            .then(|| Root::render_dialog_layer(window, cx))
            .flatten();
        let mut layout = h_resizable(self.id).child(
            resizable_panel()
                .size(self.sidebar_width)
                .size_range(self.sidebar_min_width..self.sidebar_max_width)
                .flex_none()
                .child(self.sidebar),
        );
        if let Some(pane) = self.pane {
            layout = layout.child(
                resizable_panel()
                    .size(self.pane_width)
                    .size_range(self.pane_min_width..self.pane_max_width)
                    .flex_none()
                    .child(pane),
            );
        }
        layout = layout.child(
            resizable_panel()
                .size_range(px(360.)..Pixels::MAX)
                .child(self.content),
        );

        div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .font_family(FONT_UI)
            .text_size(px(14.))
            .text_color(self.theme.text)
            .bg(self.theme.window)
            .child(layout)
            .children(dialog_layer)
    }
}

#[derive(IntoElement)]
pub struct Sidebar {
    theme: DesktopTheme,
    header: Option<AnyElement>,
    children: Vec<AnyElement>,
    footer: Option<AnyElement>,
}

impl Sidebar {
    pub fn new(theme: DesktopTheme) -> Self {
        Self {
            theme,
            header: None,
            children: Vec::new(),
            footer: None,
        }
    }

    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }
}

impl RenderOnce for Sidebar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .h_full()
            .min_h_0()
            .flex_none()
            .flex()
            .flex_col()
            .bg(self.theme.sidebar)
            .when(cfg!(target_os = "macos"), |sidebar| {
                sidebar.child(
                    div()
                        .id("desktop-sidebar-titlebar-safe-area")
                        .w_full()
                        .h(px(MACOS_TITLEBAR_SAFE_HEIGHT))
                        .flex_none(),
                )
            })
            .when_some(self.header, |sidebar, header| sidebar.child(header))
            .child(
                div()
                    .id("desktop-sidebar-scroll")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(self.children),
            )
            .when_some(self.footer, |sidebar, footer| sidebar.child(footer))
    }
}

#[derive(IntoElement)]
pub struct SidebarItem {
    id: ElementId,
    theme: DesktopTheme,
    label: SharedString,
    leading: Option<AnyElement>,
    trailing: Option<SharedString>,
    selected: bool,
}

impl SidebarItem {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        theme: DesktopTheme,
    ) -> Self {
        Self {
            id: id.into(),
            theme,
            label: label.into(),
            leading: None,
            trailing: None,
            selected: false,
        }
    }

    pub fn leading(mut self, leading: impl Into<SharedString>) -> Self {
        self.leading = Some(div().child(leading.into()).into_any_element());
        self
    }

    pub fn leading_element(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    pub fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl RenderOnce for SidebarItem {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = self.theme;
        div()
            .id(self.id)
            .h(px(30.))
            .w_full()
            .px(px(8.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .text_size(px(14.))
            .font_weight(if self.selected {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .bg(if self.selected {
                theme.selected
            } else {
                rgba(0x00000000).into()
            })
            .hover(move |item| item.bg(theme.hover))
            .when_some(self.leading, |item, leading| {
                item.child(
                    div()
                        .w(px(18.))
                        .flex_none()
                        .text_align(TextAlign::Center)
                        .text_color(theme.text_secondary)
                        .child(leading),
                )
            })
            .child(div().min_w_0().flex_1().truncate().child(self.label))
            .when_some(self.trailing, |item, trailing| {
                item.child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(theme.text_tertiary)
                        .child(trailing),
                )
            })
    }
}

#[derive(IntoElement)]
pub struct SidebarSectionLabel {
    label: SharedString,
    theme: DesktopTheme,
}

impl SidebarSectionLabel {
    pub fn new(label: impl Into<SharedString>, theme: DesktopTheme) -> Self {
        Self {
            label: label.into(),
            theme,
        }
    }
}

impl RenderOnce for SidebarSectionLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_end()
            .pb(px(4.))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(self.theme.text_tertiary)
            .child(self.label)
    }
}

#[derive(IntoElement)]
pub struct Pane {
    theme: DesktopTheme,
    header: Option<AnyElement>,
    content: AnyElement,
    footer: Option<AnyElement>,
}

impl Pane {
    pub fn new(theme: DesktopTheme, content: impl IntoElement) -> Self {
        Self {
            theme,
            header: None,
            content: content.into_any_element(),
            footer: None,
        }
    }

    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }
}

impl RenderOnce for Pane {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .h_full()
            .min_h_0()
            .flex_none()
            .flex()
            .flex_col()
            .bg(self.theme.pane)
            .when_some(self.header, |pane, header| pane.child(header))
            .child(
                div()
                    .id("desktop-pane-scroll")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(self.content),
            )
            .when_some(self.footer, |pane, footer| pane.child(footer))
    }
}

#[derive(IntoElement)]
pub struct PaneHeader {
    theme: DesktopTheme,
    title: SharedString,
    trailing: Option<AnyElement>,
}

impl PaneHeader {
    pub fn new(title: impl Into<SharedString>, theme: DesktopTheme) -> Self {
        Self {
            theme,
            title: title.into(),
            trailing: None,
        }
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }
}

impl RenderOnce for PaneHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .h(px(52.))
            .px(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(self.theme.border)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title),
            )
            .when_some(self.trailing, |header, trailing| header.child(trailing))
    }
}

#[derive(IntoElement)]
pub struct ContentPane {
    id: ElementId,
    theme: DesktopTheme,
    header: Option<AnyElement>,
    content: AnyElement,
}

impl ContentPane {
    pub fn new(theme: DesktopTheme, content: impl IntoElement) -> Self {
        Self {
            id: "desktop-content-scroll".into(),
            theme,
            header: None,
            content: content.into_any_element(),
        }
    }

    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }
}

impl RenderOnce for ContentPane {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .min_w_0()
            .min_h_0()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(self.theme.content)
            .when_some(self.header, |pane, header| pane.child(header))
            .child(
                div()
                    .id(self.id)
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(self.content),
            )
    }
}
