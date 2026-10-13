//! Lankide-specific presentation blocks assembled from `desktop-ui` primitives.
//! Generic controls and layout remain owned by `desktop-ui`.

use super::*;

pub(super) fn qr_code_image(value: &str) -> Option<(Arc<RenderImage>, f32)> {
    use qrcode::{QrCode, types::Color};

    let code = QrCode::new(value.as_bytes()).ok()?;
    let module_count = code.width();
    let quiet_zone = 4usize;
    let total_modules = module_count + quiet_zone * 2;
    let module_size = (188usize / total_modules).max(2);
    let edge = total_modules * module_size;
    let mut pixels =
        image::RgbaImage::from_pixel(edge as u32, edge as u32, image::Rgba([255, 255, 255, 255]));
    for y in 0..module_count {
        for x in 0..module_count {
            if code[(x, y)] != Color::Dark {
                continue;
            }
            let left = (x + quiet_zone) * module_size;
            let top = (y + quiet_zone) * module_size;
            for py in top..top + module_size {
                for px in left..left + module_size {
                    pixels.put_pixel(px as u32, py as u32, image::Rgba([26, 26, 34, 255]));
                }
            }
        }
    }
    Some((
        Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
        edge as f32,
    ))
}

pub(super) fn success_panel(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    theme: DesktopTheme,
) -> impl IntoElement {
    div()
        .w_full()
        .p(px(14.))
        .rounded(px(15.))
        .border_1()
        .border_color(rgb(0x86c89a))
        .bg(rgba(0x22a4470d))
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .size(px(20.))
                .rounded_full()
                .bg(rgb(0x238636))
                .text_color(rgb(0xffffff))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .child("✓"),
        )
        .child(
            div()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title.into()))
                .child(
                    div()
                        .mt(px(3.))
                        .text_size(px(12.))
                        .text_color(theme.text_secondary)
                        .child(description.into()),
                ),
        )
}

pub(super) fn color_icon(path: &'static str, size: f32) -> impl IntoElement {
    DesktopColorIcon::new(path).size(px(size))
}

pub(super) fn themed_icon(path: &'static str, size: f32, theme: DesktopTheme) -> impl IntoElement {
    Icon::default()
        .path(path)
        .size(px(size))
        .text_color(theme.text)
}

pub(super) fn muted_themed_icon(
    path: &'static str,
    size: f32,
    theme: DesktopTheme,
) -> impl IntoElement {
    Icon::default()
        .path(path)
        .size(px(size))
        .text_color(theme.text_secondary)
}

pub(super) fn primary_button_icon(
    path: &'static str,
    size: f32,
    theme: DesktopTheme,
) -> impl IntoElement {
    Icon::default()
        .path(path)
        .size(px(size))
        .text_color(theme.content)
}

pub(super) fn provider_icon(provider: &str) -> &'static str {
    if provider == "telegram" {
        "icons/telegram.svg"
    } else {
        "icons/feishu.svg"
    }
}

pub(super) fn provider_tile(
    id: &'static str,
    icon: impl IntoElement,
    name: &'static str,
    description: &'static str,
    selected: bool,
    theme: DesktopTheme,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_1()
        .min_h(px(82.))
        .p(px(14.))
        .rounded(px(14.))
        .border_1()
        .border_color(if selected { theme.focus } else { theme.border })
        .bg(if selected {
            theme.selected
        } else {
            theme.content
        })
        .cursor_pointer()
        .hover(move |tile| tile.bg(theme.hover))
        .flex()
        .items_center()
        .gap(px(12.))
        .child(icon)
        .child(
            div()
                .child(div().font_weight(FontWeight::MEDIUM).child(name))
                .child(
                    div()
                        .mt(px(4.))
                        .text_size(px(12.))
                        .text_color(theme.text_secondary)
                        .child(description),
                ),
        )
}

pub(super) fn workspace_provider_choice(
    provider: &'static str,
    label: &'static str,
    selected: bool,
    disabled: bool,
    theme: DesktopTheme,
) -> impl IntoElement {
    div()
        .min_w(px(150.))
        .h(px(44.))
        .px(px(14.))
        .rounded(px(12.))
        .border_1()
        .border_color(if selected {
            theme.border_strong
        } else {
            theme.border
        })
        .bg(if selected {
            theme.selected
        } else {
            theme.content
        })
        .opacity(if disabled && !selected { 0.52 } else { 1. })
        .flex()
        .items_center()
        .gap(px(9.))
        .child(
            div()
                .size(px(14.))
                .rounded_full()
                .border_1()
                .border_color(if selected {
                    theme.text_secondary
                } else {
                    theme.border_strong
                })
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |radio| {
                    radio.child(div().size(px(6.)).rounded_full().bg(theme.text_secondary))
                }),
        )
        .child(color_icon(provider_icon(provider), 20.))
        .child(div().text_size(px(13.)).child(label))
}

pub(super) fn harness_icon_path(id: &str) -> &'static str {
    match id {
        "codex" => "icons/lobehub/codex.svg",
        "claude_code" => "icons/lobehub/claude-code.svg",
        _ => "icons/lobehub/pi.svg",
    }
}

pub(super) fn model_icon(model: &str, size: f32, theme: DesktopTheme) -> AnyElement {
    let value = model.to_ascii_lowercase();
    let (path, mono) = if value.contains("deepseek") {
        ("icons/lobehub/deepseek.svg", false)
    } else if value.contains("claude") {
        ("icons/lobehub/claude.svg", false)
    } else if value.contains("anthropic") {
        ("icons/lobehub/anthropic.svg", true)
    } else if value.contains("codex") {
        ("icons/lobehub/codex.svg", false)
    } else if value.contains("gpt")
        || value.contains("openai")
        || ["o1", "o3", "o4"].iter().any(|family| {
            value == *family
                || value.starts_with(&format!("{family}-"))
                || value.contains(&format!("/{family}"))
        })
    {
        ("icons/lobehub/openai.svg", true)
    } else if value.contains("gemini") || value.contains("gemma") {
        ("icons/lobehub/gemini.svg", false)
    } else if value.contains("qwen") || value.contains("tongyi") || value.contains("dashscope") {
        ("icons/lobehub/qwen.svg", false)
    } else if value.contains("mistral") || value.contains("mixtral") || value.contains("codestral")
    {
        ("icons/lobehub/mistral.svg", false)
    } else if value.contains("moonshot") || value.contains("kimi") {
        ("icons/lobehub/moonshot.svg", true)
    } else if value.contains("llama") || value.contains("meta") {
        ("icons/lobehub/meta.svg", false)
    } else if value.contains("grok") || value.contains("xai") {
        ("icons/lobehub/grok.svg", true)
    } else {
        ("icons/lucide/sparkles.svg", true)
    };
    if mono {
        themed_icon(path, size, theme).into_any_element()
    } else {
        color_icon(path, size).into_any_element()
    }
}

pub(super) fn harness_icon(id: &str, size: f32, theme: DesktopTheme) -> AnyElement {
    match id {
        "claude_code" | "codex" => color_icon(harness_icon_path(id), size).into_any_element(),
        _ => themed_icon(harness_icon_path(id), size, theme).into_any_element(),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct HarnessChoice {
    id: String,
    label: SharedString,
}

impl HarnessChoice {
    pub(super) fn new(id: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }

    fn content(&self) -> AnyElement {
        let icon = match self.id.as_str() {
            "claude_code" | "codex" => {
                color_icon(harness_icon_path(&self.id), 18.).into_any_element()
            }
            _ => Icon::default()
                .path(harness_icon_path(&self.id))
                .size(px(18.))
                .into_any_element(),
        };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(9.))
            .font_family(desktop_ui::FONT_MONO)
            .child(icon)
            .child(self.label.clone())
            .into_any_element()
    }
}

impl SelectItem for HarnessChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(self.content())
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.content()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct ThinkingChoice {
    value: String,
}

impl ThinkingChoice {
    pub(super) fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    fn content(&self) -> AnyElement {
        div()
            .w_full()
            .font_family(desktop_ui::FONT_MONO)
            .child(self.value.clone())
            .into_any_element()
    }
}

impl SelectItem for ThinkingChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.value.clone().into()
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(self.content())
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.content()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub(super) fn model_field_input(
    state: &Entity<InputState>,
    model: &str,
    theme: DesktopTheme,
) -> impl IntoElement {
    DesktopInput::mono_large(state)
        .prefix(model_icon(model, 18., theme))
        .text_color(theme.text)
}

pub(super) fn agent_edit_preview(
    id: String,
    name: String,
    role: String,
    bio: String,
    harness: String,
    model: String,
    avatar: Option<Arc<RenderImage>>,
    theme: DesktopTheme,
    language: DesktopLanguage,
    on_edit_avatar: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .w(px(260.))
        .border_l_1()
        .border_dashed()
        .border_color(theme.border)
        .px(px(18.))
        .py(px(24.))
        .flex()
        .flex_col()
        .items_center()
        .justify_start()
        .child(
            div()
                .w_full()
                .mb(px(38.))
                .text_size(px(13.))
                .text_color(theme.text_secondary)
                .child(language.text("预览", "Preview")),
        )
        .child(
            div()
                .relative()
                .child(
                    DesktopAvatar::new(name.clone(), theme)
                        .size(px(104.))
                        .when_some(avatar, |avatar, image| avatar.src(image)),
                )
                .child(
                    DesktopButton::icon("edit-agent-avatar")
                        .debug_selector(|| "edit-agent-avatar".into())
                        .absolute()
                        .right(px(-4.))
                        .bottom(px(-4.))
                        .size(px(36.))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.elevated)
                        .on_click(on_edit_avatar)
                        .child(themed_icon("icons/lucide/pencil.svg", 15., theme)),
                ),
        )
        .child(
            div()
                .mt(px(14.))
                .text_size(px(21.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if name.is_empty() {
                    "Agent".into()
                } else {
                    name
                }),
        )
        .child(
            div()
                .mt(px(5.))
                .font_family(desktop_ui::FONT_MONO)
                .text_size(px(12.))
                .text_color(theme.text_secondary)
                .child(if role.is_empty() { "—".into() } else { role }),
        )
        .when(!bio.is_empty(), |preview| {
            preview.child(
                div()
                    .mt(px(18.))
                    .text_center()
                    .line_height(relative(1.45))
                    .text_color(theme.text_secondary)
                    .child(bio),
            )
        })
        .child(
            div()
                .mt(px(18.))
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(theme.raised)
                .font_family(desktop_ui::FONT_MONO)
                .text_size(px(12.))
                .child(format!("@{id}")),
        )
        .child(
            div()
                .mt(px(18.))
                .flex()
                .flex_wrap()
                .justify_center()
                .gap(px(8.))
                .child(DesktopChip::new(
                    theme,
                    harness_icon(&harness, 16., theme),
                    harness_label(&harness),
                ))
                .child(DesktopChip::new(
                    theme,
                    model_icon(&model, 16., theme),
                    model_label(&model),
                )),
        )
}

/// Full-window startup state: the mascot peeks out of its burrow while the
/// Runtime connects. Renders an empty canvas until `visible` so warm starts
/// never flash it.
pub(super) fn boot_loader(
    theme: DesktopTheme,
    label: impl Into<SharedString>,
    visible: bool,
) -> AnyElement {
    let canvas = div().size_full().bg(theme.content);
    if !visible {
        return canvas.into_any_element();
    }
    let burrow = div()
        .relative()
        .w(px(196.))
        .h(px(180.))
        .child(
            div()
                .absolute()
                .left(px(10.))
                .top(px(148.))
                .w(px(176.))
                .h(px(32.))
                .rounded_full()
                .bg(rgb(0x4e3038)),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .w(px(196.))
                .h(px(164.))
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .w(px(MEERKAT_WIDTH))
                        .h(px(MEERKAT_HEIGHT))
                        .child(
                            img("illustrations/meerkat-peek.png")
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        )
                        .with_animation(
                            "boot-loader-peek",
                            Animation::new(Duration::from_millis(6400)).repeat(),
                            |meerkat, t| {
                                let pose = peek_pose(t);
                                meerkat
                                    .left(px((196. - MEERKAT_WIDTH) / 2.))
                                    .top(px(pose.y))
                                    .child(meerkat_eye(49.5, 46.7, pose))
                                    .child(meerkat_eye(100.2, 40.6, pose))
                            },
                        ),
                ),
        );
    let dots = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .children((0..3usize).map(|index| {
            let phase = index as f32 / 6.;
            div()
                .size(px(6.))
                .rounded_full()
                .bg(theme.text_tertiary)
                .with_animation(
                    ("boot-loader-dot", index),
                    Animation::new(Duration::from_millis(1200))
                        .repeat()
                        .with_easing(move |delta| (delta + 1. - phase) % 1.),
                    |dot, delta| dot.opacity(0.25 + 0.75 * bounce(ease_in_out)(delta)),
                )
        }));
    canvas
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(14.))
        .child(burrow)
        .child(dots)
        .child(
            div()
                .text_size(px(13.))
                .text_color(theme.text_tertiary)
                .child(label.into()),
        )
        .into_any_element()
}

const MEERKAT_WIDTH: f32 = 140.;
const MEERKAT_HEIGHT: f32 = 148.;

#[derive(Clone, Copy)]
struct PeekPose {
    y: f32,
    gaze: f32,
    openness: f32,
}

/// Pupils are drawn over the illustration so the eyes can move and blink on
/// their own; that is what makes the mascot feel alive rather than slid around.
fn meerkat_eye(center_x: f32, center_y: f32, pose: PeekPose) -> impl IntoElement {
    const WIDTH: f32 = 13.2;
    const HEIGHT: f32 = 15.;
    let height = (HEIGHT * pose.openness).max(2.);
    let x = center_x + pose.gaze * 4.5;
    div()
        .absolute()
        .left(px(x - WIDTH / 2.))
        .top(px(center_y - height / 2.))
        .w(px(WIDTH))
        .h(px(height))
        .rounded_full()
        .bg(rgb(0x3e2328))
        .when(pose.openness > 0.6, |pupil| {
            pupil.child(
                div()
                    .absolute()
                    .left(px(WIDTH / 2. + 1.3 - 2.5))
                    .top(px(height / 2. - 3.8 - 2.5))
                    .size(px(5.))
                    .rounded_full()
                    .bg(rgb(0xfccc73)),
            )
        })
}

/// One peekaboo cycle, paced like a wary meerkat: only the eyes come up
/// first for a quick look around, then the whole body climbs out and looks to
/// each side, and finally sinks back to rest. Only the eyes move sideways.
fn peek_pose(t: f32) -> PeekPose {
    use std::f32::consts::PI;
    const HIDDEN: f32 = 168.;
    const PEEK: f32 = 94.;
    const UP: f32 = 18.;
    let span = |from: f32, to: f32| ((t - from) / (to - from)).clamp(0., 1.);
    let decelerate = |p: f32| 1. - (1. - p).powi(3);
    let ease = |p: f32| (1. - (PI * p).cos()) / 2.;
    let blink = |at: f32| {
        let p = span(at, at + 0.032);
        1. - (PI * p).sin().powi(2) * 0.95
    };
    // Eyes snap to a side quickly and then hold.
    let dart = |at: f32| decelerate(span(at, at + 0.04));

    let peek_gaze = -dart(0.16) + 2. * dart(0.225) - dart(0.29);
    let gaze = -dart(0.53) + 2. * dart(0.64) - dart(0.75);
    let peek = decelerate(span(0.03, 0.13));
    let climb = ease(span(0.34, 0.45));
    let sink = ease(span(0.82, 0.95));
    PeekPose {
        y: HIDDEN + (PEEK - HIDDEN) * peek + (UP - PEEK) * climb + (HIDDEN - UP) * sink,
        gaze: peek_gaze + gaze,
        openness: blink(0.315) * blink(0.48) * blink(0.69),
    }
}
