use gpui::{App, Hsla, Rgba, px, rgb, rgba};
use gpui_component::Theme as ComponentTheme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

pub const FONT_UI: &str = ".SystemUIFont";
pub const FONT_MONO: &str = "JetBrains Mono";
pub const RADIUS_CONTROL: f32 = 10.;
pub const RADIUS_ROW: f32 = 10.;
pub const RADIUS_CARD: f32 = 20.;
pub const RADIUS_MODAL: f32 = 25.;

#[derive(Clone, Copy, Debug)]
pub struct DesktopTheme {
    pub window: Hsla,
    pub sidebar: Hsla,
    pub pane: Hsla,
    pub content: Hsla,
    pub elevated: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub focus: Hsla,
    pub error: Hsla,
    pub success: Hsla,
}

impl DesktopTheme {
    pub fn light() -> Self {
        Self {
            window: rgb(0xf5f5f5).into(),
            sidebar: Rgba::from(rgba(0xffffffb3)).into(),
            pane: rgb(0xffffff).into(),
            content: rgb(0xffffff).into(),
            // Menus and dialogs must fully occlude the content below them.
            // A translucent elevated token made nearby icons bleed through
            // dropdown rows.
            elevated: rgb(0xffffff).into(),
            raised: Rgba::from(rgba(0x0d0d0d0d)).into(),
            hover: Rgba::from(rgba(0x0d0d0d0e)).into(),
            selected: Rgba::from(rgba(0x0d0d0d0e)).into(),
            border: Rgba::from(rgba(0x0d0d0d14)).into(),
            border_strong: Rgba::from(rgba(0x0d0d0d1f)).into(),
            text: rgb(0x0d0d0d).into(),
            text_secondary: Rgba::from(rgba(0x0d0d0db3)).into(),
            text_tertiary: Rgba::from(rgba(0x0d0d0d80)).into(),
            focus: rgb(0x0169cc).into(),
            error: rgb(0xb91c1c).into(),
            success: rgb(0x16a34a).into(),
        }
    }

    pub fn dark() -> Self {
        Self {
            window: rgb(0x141414).into(),
            sidebar: rgb(0x181818).into(),
            pane: rgb(0x181818).into(),
            content: rgb(0x181818).into(),
            elevated: rgb(0x363636).into(),
            raised: Rgba::from(rgba(0xffffff0d)).into(),
            hover: Rgba::from(rgba(0xffffff0f)).into(),
            selected: Rgba::from(rgba(0xffffff0f)).into(),
            border: Rgba::from(rgba(0xffffff14)).into(),
            border_strong: Rgba::from(rgba(0xffffff29)).into(),
            text: rgb(0xdfdfdf).into(),
            text_secondary: Rgba::from(rgba(0xffffffb5)).into(),
            text_tertiary: Rgba::from(rgba(0xffffff80)).into(),
            focus: rgb(0x339cff).into(),
            error: rgb(0xfca5a5).into(),
            success: rgb(0x4ade80).into(),
        }
    }
}

impl Default for DesktopTheme {
    fn default() -> Self {
        Self::light()
    }
}

/// Project the desktop design tokens into GPUI Component's global theme.
/// Call this after component initialization and after every light/dark mode change.
pub fn apply_component_theme(theme: DesktopTheme, cx: &mut App) {
    {
        let component = ComponentTheme::global_mut(cx);
        component.font_family = FONT_UI.into();
        component.font_size = px(14.);
        // Normal fields use a restrained rounded rectangle. Search inputs opt
        // into a full pill locally instead of inheriting it globally.
        component.radius = px(RADIUS_CONTROL);
        component.radius_lg = px(RADIUS_MODAL);
        // A focused field changes its 1 px border color; the additional outer
        // ring made Select/Input look like a persistent 3 px outline.
        component.focus_ring = false;
        component.shadow = true;

        let colors = &mut component.colors;
        colors.background = theme.content;
        colors.foreground = theme.text;
        colors.border = theme.border;
        colors.input = theme.border_strong;
        colors.ring = theme.focus;
        colors.muted = theme.raised;
        colors.muted_foreground = theme.text_secondary;
        colors.accent = theme.hover;
        colors.accent_foreground = theme.text;
        colors.popover = theme.elevated;
        colors.popover_foreground = theme.text;
        colors.list = theme.content;
        colors.list_hover = theme.hover;
        colors.list_active = theme.selected;
        colors.list_active_border = rgba(0x00000000).into();
        colors.primary = theme.text;
        colors.primary_active = theme.text;
        colors.primary_hover = theme.text;
        colors.primary_foreground = theme.content;
        colors.button = rgba(0x00000000).into();
        colors.button_foreground = theme.text;
        colors.button_hover = theme.hover;
        colors.button_active = theme.selected;
        colors.button_primary = theme.text;
        colors.button_primary_active = theme.text;
        colors.button_primary_hover = theme.text;
        colors.button_primary_foreground = theme.content;
        colors.secondary = theme.raised;
        colors.secondary_active = theme.selected;
        colors.secondary_hover = theme.hover;
        colors.secondary_foreground = theme.text;
    }
    ComponentTheme::sync_base(cx);
}
