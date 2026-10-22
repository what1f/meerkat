mod backend;
mod background_services;
#[cfg(target_os = "macos")]
mod traffic_lights;

use backend::DesktopBackend;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{
    Manager, Url, WebviewUrl, WebviewWindowBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
};
use tauri_plugin_opener::OpenerExt;

#[derive(Default)]
struct ShellState {
    backend: Mutex<Option<DesktopBackend>>,
    runtime_url: Mutex<Option<Url>>,
    authorized_runtime_origin: Mutex<Option<String>>,
    starting: AtomicBool,
}

fn boot_url() -> Url {
    #[cfg(target_os = "windows")]
    let base = "http://tauri.localhost/boot.html";
    #[cfg(not(target_os = "windows"))]
    let base = "tauri://localhost/boot.html";
    Url::parse(base).expect("embedded boot URL")
}

fn sync_window_fullscreen(window: &tauri::WebviewWindow) {
    match window.is_fullscreen() {
        Ok(fullscreen) => {
            if let Err(error) = window.eval(format!(
                "document.documentElement.dataset.windowFullscreen = '{fullscreen}';"
            )) {
                eprintln!("lankide-desktop: could not sync window layout: {error}");
            }
        }
        Err(error) => eprintln!("lankide-desktop: could not read window state: {error}"),
    }
}

fn is_boot(url: &Url) -> bool {
    let boot = boot_url();
    url.scheme() == boot.scheme()
        && url.host_str() == boot.host_str()
        && url.port() == boot.port()
        && url.path() == boot.path()
        && url.query().is_none()
}

fn open_external(app: &tauri::AppHandle, url: &Url) {
    if let Err(error) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("lankide-desktop: could not open URL: {error}");
    }
}

fn start_services(app: &tauri::AppHandle) {
    let state = app.state::<ShellState>();
    if state.starting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(previous) = state.backend.lock().unwrap().take() {
        previous.stop_local_services();
    }
    *state.runtime_url.lock().unwrap() = None;
    let mut backend = DesktopBackend::launch();
    let receiver = backend.take_startup_result().expect("startup receiver");
    *state.backend.lock().unwrap() = Some(backend);
    let app = app.clone();
    std::thread::spawn(move || {
        let result = receiver.blocking_recv().unwrap_or_else(|_| {
            Err(lankide_client::ClientError::Unavailable(
                "bootstrap stopped".into(),
            ))
        });
        let state = app.state::<ShellState>();
        let result = result.and_then(|_| {
            state
                .backend
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .navigation_url()
        });
        let target = match result {
            Ok(url) => {
                let origin = url.origin().ascii_serialization();
                let mut authorized = state.authorized_runtime_origin.lock().unwrap();
                if authorized
                    .as_ref()
                    .is_some_and(|previous| previous != &origin)
                {
                    // Dynamic capabilities accumulate and cannot revoke the old origin.
                    // A fresh process must own the new origin's authorization.
                    drop(authorized);
                    if let Some(backend) = state.backend.lock().unwrap().as_ref() {
                        backend.stop_local_services();
                    }
                    app.request_restart();
                    return;
                }
                // Only the discovered local Runtime gets the two window operations.
                if authorized.is_none() {
                    let capability = serde_json::json!({
                        "identifier": "runtime-window",
                        "windows": ["main"],
                        "local": false,
                        "remote": { "urls": [format!("{origin}/*")] },
                        "permissions": ["core:window:allow-start-dragging", "core:window:allow-set-theme"]
                    });
                    app.add_capability(capability.to_string())
                        .expect("Runtime window capability");
                    *authorized = Some(origin);
                }
                drop(authorized);
                *state.runtime_url.lock().unwrap() = Some(url.clone());
                for id in ["open-browser", "settings"] {
                    if let Some(item) = app
                        .menu()
                        .and_then(|menu| menu.get("lankide"))
                        .and_then(|item| item.as_submenu().and_then(|menu| menu.get(id)))
                        .and_then(|item| item.as_menuitem().cloned())
                    {
                        let _ = item.set_enabled(true);
                    }
                }
                url
            }
            Err(error) => {
                eprintln!("lankide-desktop: {error}");
                let mut url = boot_url();
                url.set_fragment(Some(backend::startup_error_code(&error)));
                url
            }
        };
        state.starting.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window("main") {
            if let Err(error) = window.navigate(target) {
                eprintln!("lankide-desktop: could not navigate: {error}");
            }
        }
    });
}

fn allow_navigation(app: &tauri::AppHandle, url: &Url) -> bool {
    if is_boot(url) {
        return true;
    }
    if url.scheme() == "lankide-boot" {
        match url.host_str() {
            Some("retry") => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.navigate(boot_url());
                }
                start_services(app);
            }
            Some("settings") => open_external(
                app,
                &Url::parse("x-apple.systempreferences:com.apple.LoginItems-Settings.extension")
                    .unwrap(),
            ),
            _ => {}
        }
        return false;
    }
    let state = app.state::<ShellState>();
    if state
        .runtime_url
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|runtime| runtime.origin() == url.origin())
    {
        return true;
    }
    open_external(app, url);
    false
}

fn main() {
    if std::env::args().any(|argument| argument == "--local-bootstrap-smoke") {
        std::process::exit(backend::run_local_bootstrap_smoke());
    }
    tauri::Builder::default()
        .manage(ShellState::default())
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            let chinese = objc2_foundation::NSLocale::preferredLanguages()
                .firstObject()
                .is_some_and(|language| language.to_string().starts_with("zh"));
            #[cfg(not(target_os = "macos"))]
            let chinese = std::env::var("LANG").unwrap_or_default().starts_with("zh");
            let open = MenuItem::with_id(
                app,
                "open-browser",
                if chinese {
                    "在浏览器中打开"
                } else {
                    "Open in Browser"
                },
                false,
                None::<&str>,
            )?;
            let settings = MenuItem::with_id(
                app,
                "settings",
                if chinese { "设置…" } else { "Settings…" },
                false,
                Some("CmdOrCtrl+,"),
            )?;
            let quit = MenuItem::with_id(
                app,
                "quit",
                if chinese {
                    "退出 Lankide"
                } else {
                    "Quit Lankide"
                },
                true,
                Some("CmdOrCtrl+Q"),
            )?;
            let menu = Menu::with_items(
                app,
                &[
                    &Submenu::with_id_and_items(
                        app,
                        "lankide",
                        "Lankide",
                        true,
                        &[&settings, &open, &quit],
                    )?,
                    &Submenu::with_items(
                        app,
                        if chinese { "编辑" } else { "Edit" },
                        true,
                        &[
                            &PredefinedMenuItem::undo(
                                app,
                                Some(if chinese { "撤销" } else { "Undo" }),
                            )?,
                            &PredefinedMenuItem::redo(
                                app,
                                Some(if chinese { "重做" } else { "Redo" }),
                            )?,
                            &PredefinedMenuItem::separator(app)?,
                            &PredefinedMenuItem::cut(
                                app,
                                Some(if chinese { "剪切" } else { "Cut" }),
                            )?,
                            &PredefinedMenuItem::copy(
                                app,
                                Some(if chinese { "复制" } else { "Copy" }),
                            )?,
                            &PredefinedMenuItem::paste(
                                app,
                                Some(if chinese { "粘贴" } else { "Paste" }),
                            )?,
                            &PredefinedMenuItem::select_all(
                                app,
                                Some(if chinese { "全选" } else { "Select All" }),
                            )?,
                        ],
                    )?,
                ],
            )?;
            app.set_menu(menu)?;
            app.add_capability(
                serde_json::json!({
                    "identifier": "boot-window",
                    "windows": ["main"],
                    "permissions": ["core:window:allow-start-dragging"]
                })
                .to_string(),
            )?;
            let handle = app.handle().clone();
            let navigation_app = handle.clone();
            let new_window_app = handle.clone();
            let builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("boot.html".into()))
                    .title("Lankide")
                    .inner_size(1240.0, 820.0)
                    .min_inner_size(960.0, 660.0)
                    .transparent(true)
                    .on_page_load(|window, payload| {
                        if payload.event() == tauri::webview::PageLoadEvent::Finished {
                            sync_window_fullscreen(&window);
                        }
                    })
                    .on_navigation(move |url| allow_navigation(&navigation_app, url))
                    .on_new_window(move |url, _| {
                        let internal = is_boot(&url)
                            || new_window_app
                                .state::<ShellState>()
                                .runtime_url
                                .lock()
                                .unwrap()
                                .as_ref()
                                .is_some_and(|runtime| runtime.origin() == url.origin());
                        if internal {
                            if let Some(window) = new_window_app.get_webview_window("main") {
                                let _ = window.navigate(url);
                            }
                        } else {
                            open_external(&new_window_app, &url);
                        }
                        tauri::webview::NewWindowResponse::Deny
                    });
            #[cfg(target_os = "macos")]
            let builder = {
                use objc2::MainThreadMarker;
                use objc2_foundation::NSString;
                use objc2_web_kit::WKWebViewConfiguration;
                let configuration = unsafe {
                    WKWebViewConfiguration::new(MainThreadMarker::new().expect("main thread"))
                };
                unsafe {
                    let previous = configuration
                        .applicationNameForUserAgent()
                        .map(|value| value.to_string())
                        .unwrap_or_default();
                    configuration.setApplicationNameForUserAgent(Some(&NSString::from_str(
                        &format!("{previous} LankideDesktop/{}", env!("CARGO_PKG_VERSION")),
                    )));
                }
                builder
                    .with_webview_configuration(configuration)
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .hidden_title(true)
            };
            let window = builder.build()?;
            #[cfg(target_os = "macos")]
            window_vibrancy::apply_vibrancy(
                &window,
                window_vibrancy::NSVisualEffectMaterial::Sidebar,
                None,
                None,
            )?;
            #[cfg(target_os = "macos")]
            traffic_lights::install(&window)?;
            start_services(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Focused(_)
            ) && let Some(webview) = window.app_handle().get_webview_window(window.label())
            {
                sync_window_fullscreen(&webview);
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            "settings" => {
                if app
                    .state::<ShellState>()
                    .runtime_url
                    .lock()
                    .unwrap()
                    .is_some()
                    && let Some(window) = app.get_webview_window("main")
                    && let Err(error) = window.eval("window.location.hash = '#/settings';")
                {
                    eprintln!("lankide-desktop: could not open settings: {error}");
                }
            }
            "open-browser" => {
                if let Some(url) = app
                    .state::<ShellState>()
                    .runtime_url
                    .lock()
                    .unwrap()
                    .as_ref()
                {
                    open_external(app, url);
                }
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("failed to build Lankide")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(backend) = app.state::<ShellState>().backend.lock().unwrap().as_ref() {
                    backend.stop_local_services();
                }
            }
        });
}
