use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::avatar::Avatar;

use crate::DesktopTheme;

#[derive(Clone)]
pub struct DesktopAvatar {
    name: SharedString,
    src: Option<ImageSource>,
    size: Pixels,
    framed: bool,
}

impl DesktopAvatar {
    pub fn new(name: impl Into<SharedString>, _theme: DesktopTheme) -> Self {
        Self {
            name: name.into(),
            src: None,
            size: px(32.),
            framed: false,
        }
    }

    pub fn src(mut self, src: impl Into<ImageSource>) -> Self {
        self.src = Some(src.into());
        self
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// Do not draw the component frame when the image asset already contains one.
    pub fn unframed(mut self) -> Self {
        self.framed = false;
        self
    }

    /// Draw a frame only in contexts that explicitly require one.
    pub fn framed(mut self) -> Self {
        self.framed = true;
        self
    }
}

impl IntoElement for DesktopAvatar {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        Avatar::new()
            .name(self.name)
            .when_some(self.src, |avatar, src| avatar.src(src))
            .size(self.size)
            .when(!self.framed, |avatar| avatar.border_0())
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct DesktopAvatarGroup {
    theme: DesktopTheme,
    avatars: Vec<DesktopAvatar>,
    size: Pixels,
    overlap: Pixels,
    limit: usize,
}

impl DesktopAvatarGroup {
    pub fn new(theme: DesktopTheme) -> Self {
        Self {
            theme,
            avatars: Vec::new(),
            size: px(32.),
            overlap: px(6.),
            limit: 4,
        }
    }

    pub fn child(mut self, avatar: DesktopAvatar) -> Self {
        self.avatars.push(avatar);
        self
    }

    pub fn children(mut self, avatars: impl IntoIterator<Item = DesktopAvatar>) -> Self {
        self.avatars.extend(avatars);
        self
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn overlap(mut self, overlap: Pixels) -> Self {
        self.overlap = overlap;
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl RenderOnce for DesktopAvatarGroup {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let hidden = self.avatars.len().saturating_sub(self.limit);
        let size = self.size;
        let overlap = self.overlap;
        let theme = self.theme;
        let mut group = div().flex().items_center();

        for (index, avatar) in self.avatars.into_iter().take(self.limit).enumerate() {
            group = group.child(
                div()
                    .ml(if index == 0 { px(0.) } else { -overlap })
                    .child(avatar.size(size)),
            );
        }

        group.when(hidden > 0, |group| {
            group.child(
                div()
                    .ml(-overlap)
                    .size(size)
                    .rounded_full()
                    .border_1()
                    .border_color(theme.content)
                    .bg(theme.raised)
                    .text_color(theme.text_secondary)
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(format!("+{hidden}")),
            )
        })
    }
}
