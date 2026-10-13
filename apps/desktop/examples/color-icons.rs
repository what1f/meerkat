//! Visual acceptance for every bundled brand icon through the public UI component.
//! Run: cargo run --offline --locked -p lankide-desktop --example color-icons
//! Optional: --capture /absolute/path.png --scale-factor 2
//! Scenario: inspect all brand icons at 14/16/18/20/22/30 px on light/dark surfaces.
//! Prerequisites: macOS window session; no accounts, credentials or daemons.
//! The runner opens the complete gallery without user input; use Computer Use to
//! retain the real-window capture and inspect small-size edge quality.

#[path = "../src/assets.rs"]
mod assets;

use desktop_ui::{DesktopColorIcon, DesktopTheme};
use gpui::*;
use gpui_component::{Icon, Root};

const ICONS: &[(&str, &str, bool)] = &[
    ("飞书", "icons/feishu.svg", true),
    ("Telegram", "icons/telegram.svg", true),
    ("Codex", "icons/lobehub/codex.svg", true),
    ("Claude Code", "icons/lobehub/claude-code.svg", true),
    ("DeepSeek", "icons/lobehub/deepseek.svg", true),
    ("Claude", "icons/lobehub/claude.svg", true),
    ("Gemini", "icons/lobehub/gemini.svg", true),
    ("Qwen", "icons/lobehub/qwen.svg", true),
    ("Mistral", "icons/lobehub/mistral.svg", true),
    ("Meta", "icons/lobehub/meta.svg", true),
    ("Pi", "icons/lobehub/pi.svg", false),
    ("OpenAI", "icons/lobehub/openai.svg", false),
    ("Anthropic", "icons/lobehub/anthropic.svg", false),
    ("Moonshot", "icons/lobehub/moonshot.svg", false),
    ("Grok", "icons/lobehub/grok.svg", false),
];

const SIZES: &[f32] = &[14., 16., 18., 20., 22., 30.];

struct Gallery;

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .children([DesktopTheme::light(), DesktopTheme::dark()].map(|theme| {
                div()
                    .flex_1()
                    .h_full()
                    .p(px(20.))
                    .bg(theme.content)
                    .text_color(theme.text)
                    .child(
                        div()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .child(div().w(px(132.)).child("品牌图标"))
                            .children(
                                SIZES
                                    .iter()
                                    .map(|edge| div().w(px(54.)).child(format!("{edge}px"))),
                            ),
                    )
                    .children(ICONS.iter().map(|&(name, path, color)| {
                        div()
                            .h(px(44.))
                            .flex()
                            .items_center()
                            .child(div().w(px(132.)).text_size(px(13.)).child(name))
                            .children(SIZES.iter().map(|&edge| {
                                let icon = if color {
                                    DesktopColorIcon::new(path)
                                        .size(px(edge))
                                        .into_any_element()
                                } else {
                                    Icon::default()
                                        .path(path)
                                        .size(px(edge))
                                        .text_color(theme.text)
                                        .into_any_element()
                                };
                                div().w(px(54.)).child(icon)
                            }))
                    }))
            }))
    }
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let capture = args
        .windows(2)
        .find(|pair| pair[0] == "--capture")
        .map(|pair| pair[1].clone());
    let scale_factor = args
        .windows(2)
        .find(|pair| pair[0] == "--scale-factor")
        .map(|pair| pair[1].parse::<f32>().expect("numeric display scale"));
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            cx.set_app_identity("com.lankide.color-icons", "Lankide Icons");
            gpui_component::init(cx);
            assets::register_fonts(cx).expect("fonts");
            let window = cx
                .open_window(
                    WindowOptions {
                        titlebar: Some(TitlebarOptions {
                            title: Some("Lankide Icons".into()),
                            ..Default::default()
                        }),
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(1040.), px(752.)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    |window, cx| {
                        let gallery = cx.new(|_| Gallery);
                        cx.new(|cx| Root::new(gallery, window, cx))
                    },
                )
                .expect("icon gallery window");
            if let Some(path) = capture.clone() {
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(1))
                        .await;
                    if let Some(scale) = scale_factor {
                        // Override only after native initial resize events have settled.
                        AnyWindowHandle::from(window)
                            .update(cx, |_, window, cx| {
                                window.set_scale_factor(scale);
                                window.draw(cx).clear(cx);
                            })
                            .expect("set capture display scale");
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(500))
                            .await;
                    }
                    AnyWindowHandle::from(window)
                        .update(cx, |_, window, cx| -> anyhow::Result<()> {
                            if let Some(scale) = scale_factor {
                                assert_eq!(window.scale_factor(), scale, "capture display scale");
                            }
                            window.draw(cx).clear(cx);
                            let frame = window.render_to_image()?;
                            frame.save(&path)?;
                            println!(
                                "Captured {}x{} at {}x: {path}",
                                frame.width(),
                                frame.height(),
                                window.scale_factor()
                            );
                            Ok(())
                        })
                        .expect("gallery window available")
                        .expect("capture gallery");
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
            cx.activate(true);
        });
}
