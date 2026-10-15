#![recursion_limit = "256"]
#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/ui/mod.rs"]
mod ui;

use desktop_ui::{DesktopTheme, apply_component_theme};
use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};
use gpui_component::Root;
use lankide_client::ManagementClient;
use lankide_computer_daemon::{ComputerDaemonPaths, pair};
use lankide_system_e2e::{RigConfig, SystemRig};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use ui::{LankideApp, LankideFrame, observe_dialog_root};

struct OwnedComputer(std::process::Child);
impl Drop for OwnedComputer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
    });
}
fn wait(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        draw(cx);
        if ready(cx) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Desktop observable state timed out"
        );
        std::thread::yield_now();
    }
}
fn click(cx: &mut VisualTestContext, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    wait(cx, |cx| cx.debug_bounds(selector).is_some());
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
}
fn fill(cx: &mut VisualTestContext, selector: &str, value: &str) {
    click(cx, selector);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(value);
    draw(cx);
}

#[test]
fn local_online_offline_selection_and_pairing_dialog() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _entered = runtime.enter();
    let rig = runtime
        .block_on(SystemRig::start(RigConfig {
            local_discovery: true,
            pairing_validity: Duration::from_secs(2),
            harness_executable: "/usr/bin/false".into(),
            ..Default::default()
        }))
        .unwrap();
    let client = Arc::new(rig.management().clone());
    let local = runtime.block_on(client.get_computer_catalog(None)).unwrap()[0]
        .id
        .clone();
    assert!(runtime.block_on(client.get_computer_catalog(None)).unwrap()[0].available);
    // The rig uses separate homes. Copy the real local identity into the
    // Desktop home to reproduce the shared-home product composition.
    std::fs::copy(
        rig.computer_home().join("computer-credentials.json"),
        rig.runtime_home().join("computer-credentials.json"),
    )
    .unwrap();
    let remote_home = tempfile::tempdir_in("/tmp").unwrap();
    let code = runtime.block_on(client.create_pairing_code()).unwrap();
    let (url, _) = rig.local_pairing_endpoint().unwrap();
    let remote = runtime
        .block_on(pair(
            &ComputerDaemonPaths::for_home(remote_home.path()).unwrap(),
            &url,
            &code.code,
        ))
        .unwrap();
    let executable = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lankide-computer-daemon");
    let mut remote_process = OwnedComputer(
        std::process::Command::new(executable)
            .arg("--home")
            .arg(remote_home.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    runtime
        .block_on(tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if client
                    .get_computer_catalog(None)
                    .await
                    .unwrap()
                    .iter()
                    .any(|c| c.id == remote.computer_id && c.available)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }))
        .unwrap();
    remote_process.0.kill().unwrap();
    remote_process.0.wait().unwrap();
    runtime
        .block_on(tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if client
                    .get_computer_catalog(None)
                    .await
                    .unwrap()
                    .iter()
                    .any(|c| c.id == remote.computer_id && !c.available)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }))
        .unwrap();
    runtime.block_on(rig.workspace("team")).unwrap();
    let revoked_home = tempfile::tempdir_in("/tmp").unwrap();
    let code = runtime.block_on(client.create_pairing_code()).unwrap();
    let revoked = runtime
        .block_on(pair(
            &ComputerDaemonPaths::for_home(revoked_home.path()).unwrap(),
            &url,
            &code.code,
        ))
        .unwrap();
    runtime
        .block_on(client.revoke_computer(&revoked.computer_id))
        .unwrap();

    let local_selector: &'static str = Box::leak(format!("computer-{local}").into_boxed_str());
    let remote_selector: &'static str =
        Box::leak(format!("computer-{}", remote.computer_id).into_boxed_str());
    let revoked_selector: &'static str =
        Box::leak(format!("computer-{}", revoked.computer_id).into_boxed_str());
    let offline_selector: &'static str =
        Box::leak(format!("computer-offline-{}", remote.computer_id).into_boxed_str());
    let online_selector: &'static str =
        Box::leak(format!("computer-online-{local}").into_boxed_str());
    let scenario = std::panic::AssertUnwindSafe(|dispatcher: gpui::TestDispatcher| {
        let mut cx = TestAppContext::build(dispatcher.clone(), Some("computer_pairing"));
        let _refs = cx.app.borrow().ref_counts_drop_handle();
        cx.update(|cx| cx.background_executor().allow_parking());
        cx.update(|cx| {
            gpui_component::init(cx);
            apply_component_theme(DesktopTheme::dark(), cx);
        });
        let mounted = Arc::new(std::sync::Mutex::new(None));
        let capture = mounted.clone();
        let view_client = client.clone();
        let local_home = rig.runtime_home().to_path_buf();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(Ok(())).unwrap();
        let (root, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                LankideApp::new_with_startup(view_client, rx, local_home, true, window, cx)
            });
            let frame = cx.new(|cx| LankideFrame::new(view, window, cx));
            *capture.lock().unwrap() = Some(frame.clone());
            Root::new(frame, window, cx).bordered(false)
        });
        cx.update(|window, cx| {
            observe_dialog_root(&mounted.lock().unwrap().take().unwrap(), &root, window, cx)
        });
        wait(cx, |cx| cx.debug_bounds("empty-add-agent").is_some());
        click(cx, "nav-settings");
        wait(cx, |cx| cx.debug_bounds(local_selector).is_some());
        assert!(
            cx.debug_bounds("pairing-code").is_none(),
            "first launch showed a code"
        );
        assert!(cx.debug_bounds(remote_selector).is_some());
        assert!(cx.debug_bounds(online_selector).is_some());
        assert!(cx.debug_bounds(offline_selector).is_some());
        assert!(cx.debug_bounds(revoked_selector).is_none());
        // Status and registration changes reach the visible Settings card.
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lankide-computer-daemon");
        let mut reconnected = OwnedComputer(
            std::process::Command::new(executable)
                .arg("--home")
                .arg(remote_home.path())
                .env("HOME", remote_home.path())
                .env("SHELL", "/bin/sh")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let remote_online: &'static str =
            Box::leak(format!("computer-online-{}", remote.computer_id).into_boxed_str());
        wait(cx, |cx| cx.debug_bounds(remote_online).is_some());
        reconnected.0.kill().unwrap();
        reconnected.0.wait().unwrap();
        wait(cx, |cx| cx.debug_bounds(offline_selector).is_some());
        let added_home = tempfile::tempdir_in("/tmp").unwrap();
        let code = runtime.block_on(client.create_pairing_code()).unwrap();
        let added = runtime
            .block_on(pair(
                &ComputerDaemonPaths::for_home(added_home.path()).unwrap(),
                &url,
                &code.code,
            ))
            .unwrap();
        let added_selector: &'static str =
            Box::leak(format!("computer-{}", added.computer_id).into_boxed_str());
        wait(cx, |cx| cx.debug_bounds(added_selector).is_some());
        runtime
            .block_on(client.revoke_computer(&added.computer_id))
            .unwrap();
        wait(cx, |cx| cx.debug_bounds(added_selector).is_none());

        click(cx, "pair-computer");
        wait(cx, |cx| cx.debug_bounds("pairing-address-input").is_some());
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap().to_string();
        drop(socket);
        fill(cx, "pairing-address-input", &address);
        click(cx, "enable-computer-access");
        wait(cx, |cx| cx.debug_bounds("pairing-command").is_some());
        assert!(cx.debug_bounds("pairing-code").is_none());
        assert!(cx.debug_bounds("pairing-waiting-hint").is_none());
        cx.executor().advance_clock(Duration::from_secs(30));
        wait(cx, |cx| cx.debug_bounds("pairing-waiting-hint").is_some());
        click(cx, "copy-pairing-command");
        let copied = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
        std::thread::sleep(Duration::from_millis(2100));
        let original = copied;
        let deadline = Instant::now() + Duration::from_secs(10);
        let copied = loop {
            click(cx, "copy-pairing-command");
            let refreshed = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
            if refreshed != original {
                break refreshed;
            }
            assert!(Instant::now() < deadline, "expired command was not renewed");
            std::thread::yield_now();
        };
        let args = copied.split_whitespace().collect::<Vec<_>>();
        assert_eq!(args[0], "lankide-computer-daemon");
        assert_eq!(args[1], "pair");
        assert_eq!(args[2], format!("wss://{address}"));
        assert_eq!(args[4], "--cert-sha256");
        let paired_home = tempfile::tempdir_in("/tmp").unwrap();
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lankide-computer-daemon");
        let _paired = OwnedComputer(
            std::process::Command::new(executable)
                .args(&args[1..])
                .arg("--home")
                .arg(paired_home.path())
                .env("HOME", paired_home.path())
                .env("SHELL", "/bin/sh")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        wait(cx, |cx| cx.debug_bounds("pairing-connected").is_some());
        wait(cx, |cx| cx.debug_bounds("pairing-connected").is_none());
        click(cx, "language-select");
        cx.simulate_keystrokes("down enter");
        draw(cx);
        click(cx, "nav-agents");
        click(cx, "empty-add-agent");
        fill(cx, "agent-id-input", "builder");
        fill(cx, "agent-name-input", "Builder");
        fill(cx, "agent-model-input", "scripted");
        click(cx, "agent-computer-select");
        cx.simulate_keystrokes("enter");
        click(cx, "save-agent");
        wait(cx, |cx| cx.debug_bounds("agent-card-builder").is_some());
        assert_eq!(
            runtime
                .block_on(client.get_agent("team", "builder"))
                .unwrap()
                .computer_id,
            local
        );
        // Edit through the same public form, choose the offline Computer.
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-0");
        click(cx, "agent-computer-select");
        click(cx, &format!("computer-choice-{}", remote.computer_id));
        click(cx, "save-agent");
        wait(cx, |cx| cx.debug_bounds("agent-computer-select").is_none());
        assert_eq!(
            runtime
                .block_on(client.get_agent("team", "builder"))
                .unwrap()
                .computer_id,
            remote.computer_id
        );
        click(cx, "add-agent");
        fill(cx, "agent-id-input", "remote_builder");
        fill(cx, "agent-name-input", "Remote Builder");
        fill(cx, "agent-model-input", "scripted");
        click(cx, "agent-computer-select");
        click(cx, &format!("computer-choice-{}", remote.computer_id));
        click(cx, "save-agent");
        wait(cx, |cx| {
            cx.debug_bounds("agent-card-remote_builder").is_some()
        });
        assert_eq!(
            runtime
                .block_on(client.get_agent("team", "remote_builder"))
                .unwrap()
                .computer_id,
            remote.computer_id
        );
        cx.update(|_, cx| cx.quit());
    });
    gpui::run_test(1, &[], 0, &mut |dispatcher, _| scenario(dispatcher), None);
    runtime.block_on(rig.close()).unwrap();
}

#[path = "../src/backend.rs"]
mod backend;
#[path = "../src/background_services.rs"]
mod background_services;

/// Run the public Desktop bootstrap and app-quit lifecycle with real helpers.
/// A subprocess provides private environment variables without mutating this test process.
#[test]
fn owned_helpers_stop_on_app_quit_while_reused_helpers_stay_running() {
    const CHILD: &str = "LANKIDE_QUIT_E2E_CHILD";
    const NAME: &str = "owned_helpers_stop_on_app_quit_while_reused_helpers_stay_running";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::Builder::new()
            .prefix("lkq-")
            .tempdir_in("/tmp")
            .unwrap();
        let current = std::env::current_exe().unwrap();
        let binaries = current.parent().unwrap().parent().unwrap();
        let executable = directory.path().join("quit-e2e");
        std::fs::hard_link(&current, &executable).unwrap();
        for name in ["lankide-runtime-daemon", "lankide-computer-daemon"] {
            std::fs::hard_link(binaries.join(name), directory.path().join(name)).unwrap();
        }
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let output = std::process::Command::new(executable)
            .args(["--exact", NAME, "--nocapture"])
            .env(CHILD, "1")
            .env("LANKIDE_HOME", &home)
            .env("HOME", &home)
            .env_remove("LANKIDE_RUNTIME_RUN")
            .env("SHELL", "/bin/sh")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let owner = backend::DesktopBackend::bootstrap();
    let _entered = owner.enter();
    assert!(owner.startup_error.is_none());
    let pids: Vec<String> = ["runtime.lock", "computer.lock"]
        .into_iter()
        .map(|name| {
            let lock = std::fs::read_to_string(owner.local_home.join("run").join(name)).unwrap();
            lock.split_whitespace()
                .find_map(|part| part.strip_prefix("pid="))
                .unwrap()
                .to_owned()
        })
        .collect();
    let reused = backend::DesktopBackend::bootstrap();
    assert!(reused.startup_error.is_none());
    let scenario = std::panic::AssertUnwindSafe(|dispatcher: gpui::TestDispatcher| {
        let mut cx = TestAppContext::build(dispatcher.clone(), Some("reused_quit"));
        let cleanup = reused.quit_cleanup();
        cx.update(|cx| {
            cx.on_app_quit(cleanup).detach();
            cx.shutdown();
        });
        for pid in &pids {
            assert!(
                std::process::Command::new("kill")
                    .args(["-0", pid])
                    .status()
                    .unwrap()
                    .success(),
                "quit stopped a reused helper"
            );
        }
        let mut cx = TestAppContext::build(dispatcher, Some("owned_quit"));
        let cleanup = owner.quit_cleanup();
        cx.update(|cx| {
            cx.on_app_quit(cleanup).detach();
            cx.shutdown();
        });
        for pid in &pids {
            assert!(
                !std::process::Command::new("kill")
                    .args(["-0", pid])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success(),
                "owned helper survived app quit"
            );
        }
        assert!(!owner.local_home.join("run/runtime.lock").exists());
        assert!(!owner.local_home.join("run/computer.lock").exists());
    });
    gpui::run_test(1, &[], 0, &mut |dispatcher, _| scenario(dispatcher), None);
}
