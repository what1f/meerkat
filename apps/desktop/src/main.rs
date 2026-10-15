#![recursion_limit = "256"]

mod assets;
mod backend;
mod background_services;
mod ui;

use assets::Assets;
use backend::DesktopBackend;
use desktop_ui::{
    DesktopTheme, apply_component_theme, configure_window_material, observe_window_frame,
};
use gpui::*;
use gpui_component::{Root, Theme as ComponentTheme, ThemeMode as ComponentThemeMode};
use ui::{LankideApp, LankideFrame, observe_dialog_root};

actions!(lankide, [Quit]);

fn main() {
    if std::env::args().any(|argument| argument == "--local-bootstrap-smoke") {
        std::process::exit(backend::run_local_bootstrap_smoke());
    }
    let mut backend = DesktopBackend::launch();
    let startup_result = backend
        .take_startup_result()
        .expect("desktop startup result receiver");
    let _tokio_guard = backend.enter();
    let client = backend.client.clone();
    let local_home = backend.local_home.clone();
    let quit_cleanup = backend.quit_cleanup();
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            cx.set_app_identity("com.lankide.desktop", "Lankide");
            gpui_component::init(cx);
            cx.on_app_quit(quit_cleanup).detach();
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                #[cfg(target_os = "macos")]
                KeyBinding::new("cmd-q", Quit, None),
                #[cfg(not(target_os = "macos"))]
                KeyBinding::new("alt-f4", Quit, None),
            ]);
            cx.set_menus([Menu::new("Lankide").items([MenuItem::action("Quit Lankide", Quit)])]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            assets::register_fonts(cx).expect("failed to register bundled fonts");
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Lankide".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: cfg!(target_os = "macos")
                            .then(|| point(px(16.), px(17.))),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1240.), px(820.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(960.), px(660.))),
                    is_movable: true,
                    app_owns_titlebar_drag: cfg!(target_os = "macos"),
                    window_background: if cfg!(target_os = "macos") {
                        WindowBackgroundAppearance::Blurred
                    } else {
                        WindowBackgroundAppearance::Opaque
                    },
                    ..Default::default()
                },
                move |window, cx| {
                    let client = client.clone();
                    let initial_dark = matches!(
                        window.appearance(),
                        WindowAppearance::Dark | WindowAppearance::VibrantDark
                    );
                    ComponentTheme::change(
                        if initial_dark {
                            ComponentThemeMode::Dark
                        } else {
                            ComponentThemeMode::Light
                        },
                        Some(window),
                        cx,
                    );
                    apply_component_theme(
                        if initial_dark {
                            DesktopTheme::dark()
                        } else {
                            DesktopTheme::light()
                        },
                        cx,
                    );
                    let view = cx.new(|cx| {
                        observe_window_frame(window, cx);
                        LankideApp::new_with_startup(
                            client,
                            startup_result,
                            local_home,
                            initial_dark,
                            window,
                            cx,
                        )
                    });
                    configure_window_material(window, initial_dark);
                    let frame = cx.new(|cx| LankideFrame::new(view, window, cx));
                    let root = cx.new(|cx| Root::new(frame.clone(), window, cx).bordered(false));
                    observe_dialog_root(&frame, &root, window, cx);
                    root
                },
            )
            .expect("failed to open Lankide window");
            cx.activate(true);
        });
}
