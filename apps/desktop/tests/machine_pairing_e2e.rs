use crate::{backend, ui};

use desktop_ui::{DesktopTheme, apply_component_theme};
use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};
use gpui_component::Root;
use lankide_client::ManagementClient;
use lankide_machine::{MachineDaemonPaths, pair};
use lankide_system_e2e::{RigConfig, SystemRig};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use ui::{LankideApp, LankideFrame, observe_dialog_root};

struct OwnedMachine(std::process::Child);
impl Drop for OwnedMachine {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn draw(cx: &mut VisualTestContext, clock: &std::cell::Cell<Instant>) {
    let now = Instant::now();
    let elapsed = now.duration_since(clock.replace(now));
    cx.run_until_parked();
    cx.executor().advance_clock(elapsed);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
    });
}
#[track_caller]
fn wait(
    cx: &mut VisualTestContext,
    clock: &std::cell::Cell<Instant>,
    mut ready: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        draw(cx, &clock);
        if ready(cx) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Desktop observable state timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn click(cx: &mut VisualTestContext, clock: &std::cell::Cell<Instant>, selector: &str) {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    wait(cx, &clock, |cx| cx.debug_bounds(selector).is_some());
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx, &clock);
}
fn fill(cx: &mut VisualTestContext, clock: &std::cell::Cell<Instant>, selector: &str, value: &str) {
    click(cx, &clock, selector);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(value);
    draw(cx, &clock);
}

#[test]
fn local_online_offline_selection_and_pairing_dialog() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _entered = runtime.enter();
    let rig = runtime
        .block_on(SystemRig::start(RigConfig {
            local_discovery: true,
            pairing_validity: Duration::from_secs(1),
            harness_executable: "/usr/bin/false".into(),
            ..Default::default()
        }))
        .unwrap();
    let client = Arc::new(rig.management().clone());
    let local = runtime.block_on(client.get_machine_catalog(None)).unwrap()[0]
        .id
        .clone();
    assert!(runtime.block_on(client.get_machine_catalog(None)).unwrap()[0].available);
    // The rig uses separate homes. Copy the real local identity into the
    // Desktop home to reproduce the shared-home product composition.
    std::fs::copy(
        rig.machine_home().join("machine-credentials.json"),
        rig.runtime_home().join("machine-credentials.json"),
    )
    .unwrap();
    let remote_home = tempfile::tempdir_in("/tmp").unwrap();
    let code = runtime.block_on(client.create_pairing_code()).unwrap();
    let (url, _) = rig.local_pairing_endpoint().unwrap();
    let remote = runtime
        .block_on(pair(
            &MachineDaemonPaths::for_home(remote_home.path()).unwrap(),
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
        .join("lankide-machine");
    let mut remote_process = OwnedMachine(
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
                    .get_machine_catalog(None)
                    .await
                    .unwrap()
                    .iter()
                    .any(|c| c.id == remote.machine_id && c.available)
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
                    .get_machine_catalog(None)
                    .await
                    .unwrap()
                    .iter()
                    .any(|c| c.id == remote.machine_id && !c.available)
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
            &MachineDaemonPaths::for_home(revoked_home.path()).unwrap(),
            &url,
            &code.code,
        ))
        .unwrap();
    runtime
        .block_on(client.revoke_machine(&revoked.machine_id))
        .unwrap();

    let local_selector: &'static str = Box::leak(format!("machine-{local}").into_boxed_str());
    let remote_selector: &'static str =
        Box::leak(format!("machine-{}", remote.machine_id).into_boxed_str());
    let revoked_selector: &'static str =
        Box::leak(format!("machine-{}", revoked.machine_id).into_boxed_str());
    let offline_selector: &'static str =
        Box::leak(format!("machine-offline-{}", remote.machine_id).into_boxed_str());
    let online_selector: &'static str =
        Box::leak(format!("machine-online-{local}").into_boxed_str());
    let rig = std::cell::RefCell::new(rig);
    let scenario = std::panic::AssertUnwindSafe(|dispatcher: gpui::TestDispatcher| {
        let clock = std::cell::Cell::new(Instant::now());
        let mut rig = rig.borrow_mut();
        let mut cx = TestAppContext::build(dispatcher.clone(), Some("machine_pairing"));
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
        wait(cx, &clock, |cx| {
            cx.debug_bounds("empty-add-agent").is_some()
        });
        click(cx, &clock, "nav-settings");
        wait(cx, &clock, |cx| cx.debug_bounds(local_selector).is_some());
        assert!(
            cx.debug_bounds("pairing-code").is_none(),
            "first launch showed a code"
        );
        assert!(cx.debug_bounds(remote_selector).is_some());
        assert!(cx.debug_bounds(online_selector).is_some());
        assert!(cx.debug_bounds(offline_selector).is_some());
        assert!(cx.debug_bounds(revoked_selector).is_none());
        // The local row offers Rename only.
        let row = cx.debug_bounds(local_selector).unwrap();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        draw(cx, &clock);
        click(cx, &clock, &format!("machine-menu-{local}"));
        cx.simulate_keystrokes("down down enter");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("machine-name-input").is_some()
        });
        cx.simulate_keystrokes("escape");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("machine-name-input").is_none()
        });
        let row = cx.debug_bounds(remote_selector).unwrap();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        draw(cx, &clock);
        click(cx, &clock, &format!("machine-menu-{}", remote.machine_id));
        cx.simulate_keystrokes("down enter");
        fill(cx, &clock, "machine-name-input", "  Studio Mac  ");
        cx.simulate_keystrokes("enter");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("machine-name-Studio Mac").is_some()
        });
        assert_eq!(
            runtime
                .block_on(client.get_machine_catalog(None))
                .unwrap()
                .iter()
                .find(|c| c.id == remote.machine_id)
                .unwrap()
                .name,
            "Studio Mac"
        );
        // Status and registration changes reach the visible Settings card.
        let executable = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("lankide-machine");
        let mut reconnected = OwnedMachine(
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
            Box::leak(format!("machine-online-{}", remote.machine_id).into_boxed_str());
        wait(cx, &clock, |cx| cx.debug_bounds(remote_online).is_some());
        reconnected.0.kill().unwrap();
        reconnected.0.wait().unwrap();
        wait(cx, &clock, |cx| cx.debug_bounds(offline_selector).is_some());
        // Keep Settings open across a Runtime restart. The existing Desktop
        // subscription must reconnect and receive subsequent catalog changes.
        runtime.block_on(rig.restart_runtime()).unwrap();
        let (url, _) = rig.local_pairing_endpoint().unwrap();
        wait(cx, &clock, |cx| cx.debug_bounds(online_selector).is_some());
        let added_home = tempfile::tempdir_in("/tmp").unwrap();
        let code = runtime.block_on(client.create_pairing_code()).unwrap();
        let added = runtime
            .block_on(pair(
                &MachineDaemonPaths::for_home(added_home.path()).unwrap(),
                &url,
                &code.code,
            ))
            .unwrap();
        let added_selector: &'static str =
            Box::leak(format!("machine-{}", added.machine_id).into_boxed_str());
        wait(cx, &clock, |cx| cx.debug_bounds(added_selector).is_some());
        // Offline Machines can be removed through the row menu.
        let row = cx.debug_bounds(added_selector).unwrap();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        draw(cx, &clock);
        click(cx, &clock, &format!("machine-menu-{}", added.machine_id));
        cx.simulate_keystrokes("down down enter");
        click(cx, &clock, "confirm-remove-machine");
        wait(cx, &clock, |cx| cx.debug_bounds(added_selector).is_none());

        click(cx, &clock, "pair-machine");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("pairing-command").is_some()
        });
        assert!(cx.debug_bounds("pairing-address-input").is_none());
        click(cx, &clock, "copy-pairing-command");
        let automatic = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
        let endpoint = automatic.split_whitespace().nth(2).unwrap();
        assert!(!endpoint.starts_with("wss://198.18.") && !endpoint.starts_with("wss://198.19."));
        let advertised = endpoint
            .strip_prefix("wss://")
            .unwrap()
            .parse::<std::net::SocketAddr>()
            .unwrap();
        assert!(!advertised.ip().is_loopback());
        assert_ne!(advertised.port(), 0);
        assert!(
            runtime
                .block_on(client.get_remote_access())
                .unwrap()
                .address
                .is_some_and(|address| endpoint == format!("wss://{address}"))
        );
        click(cx, &clock, "change-pairing-address");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("pairing-address-input").is_some()
        });
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap().to_string();
        drop(socket);
        fill(cx, &clock, "pairing-address-input", &address);
        click(cx, &clock, "enable-machine-access");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("pairing-command").is_some()
        });
        assert!(cx.debug_bounds("pairing-code").is_none());
        assert!(cx.debug_bounds("pairing-waiting-hint").is_none());
        cx.executor().advance_clock(Duration::from_secs(30));
        wait(cx, &clock, |cx| {
            cx.debug_bounds("pairing-waiting-hint").is_some()
        });
        click(cx, &clock, "copy-pairing-command");
        let copied = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
        std::thread::sleep(Duration::from_millis(1100));
        let original = copied;
        let deadline = Instant::now() + Duration::from_secs(10);
        let copied = loop {
            click(cx, &clock, "copy-pairing-command");
            let refreshed = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
            if refreshed != original {
                break refreshed;
            }
            assert!(Instant::now() < deadline, "expired command was not renewed");
            std::thread::yield_now();
        };
        let args = copied.split_whitespace().collect::<Vec<_>>();
        assert_eq!(args[0], "lankide-machine");
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
            .join("lankide-machine");
        let mut paired_process = OwnedMachine(
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
        wait(cx, &clock, |cx| {
            cx.debug_bounds("pairing-connected").is_some()
        });
        wait(cx, &clock, |cx| {
            cx.debug_bounds("pairing-connected").is_none()
        });
        // Removing the idle online Machine removes its row and stops its daemon.
        let paired = lankide_machine::read_credentials(paired_home.path())
            .unwrap()
            .unwrap();
        let paired_selector: &'static str =
            Box::leak(format!("machine-{}", paired.machine_id).into_boxed_str());
        let row = cx.debug_bounds(paired_selector).unwrap();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        draw(cx, &clock);
        click(cx, &clock, &format!("machine-menu-{}", paired.machine_id));
        cx.simulate_keystrokes("down down enter");
        click(cx, &clock, "confirm-remove-machine");
        wait(cx, &clock, |cx| cx.debug_bounds(paired_selector).is_none());
        wait(cx, &clock, |_| {
            paired_process.0.try_wait().unwrap().is_some()
        });
        click(cx, &clock, "language-select");
        cx.simulate_keystrokes("down enter");
        draw(cx, &clock);
        click(cx, &clock, "nav-agents");
        click(cx, &clock, "empty-add-agent");
        fill(cx, &clock, "agent-id-input", "builder");
        fill(cx, &clock, "agent-name-input", "Builder");
        fill(cx, &clock, "agent-model-input", "scripted");
        click(cx, &clock, "agent-machine-select");
        cx.simulate_keystrokes("enter");
        click(cx, &clock, "save-agent");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("agent-card-builder").is_some()
        });
        assert_eq!(
            runtime
                .block_on(client.get_agent("team", "builder"))
                .unwrap()
                .machine_id,
            local
        );
        // Edit through the same public form, choose the offline Machine.
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx, &clock);
        click(cx, &clock, "edit-agent-0");
        click(cx, &clock, "agent-machine-select");
        assert!(cx.debug_bounds("machine-choice-name-Studio Mac").is_some());
        runtime
            .block_on(client.rename_machine(&remote.machine_id, "Office Mac"))
            .unwrap();
        wait(cx, &clock, |cx| {
            cx.debug_bounds("machine-choice-name-Office Mac").is_some()
        });
        click(cx, &clock, &format!("machine-choice-{}", remote.machine_id));
        click(cx, &clock, "save-agent");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("agent-machine-select").is_none()
        });
        assert_eq!(
            runtime
                .block_on(client.get_agent("team", "builder"))
                .unwrap()
                .machine_id,
            remote.machine_id
        );
        click(cx, &clock, "add-agent");
        fill(cx, &clock, "agent-id-input", "remote_builder");
        fill(cx, &clock, "agent-name-input", "Remote Builder");
        fill(cx, &clock, "agent-model-input", "scripted");
        click(cx, &clock, "agent-machine-select");
        click(cx, &clock, &format!("machine-choice-{}", remote.machine_id));
        click(cx, &clock, "save-agent");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("agent-card-remote_builder").is_some()
        });
        assert_eq!(
            runtime
                .block_on(client.get_agent("team", "remote_builder"))
                .unwrap()
                .machine_id,
            remote.machine_id
        );
        click(cx, &clock, "nav-settings");
        let row = cx.debug_bounds(remote_selector).unwrap();
        cx.simulate_mouse_move(row.center(), None, Modifiers::default());
        draw(cx, &clock);
        click(cx, &clock, &format!("machine-menu-{}", remote.machine_id));
        cx.simulate_keystrokes("down down enter");
        click(cx, &clock, "confirm-remove-machine");
        wait(cx, &clock, |cx| {
            cx.debug_bounds("machine-remove-error").is_some()
        });
        assert!(cx.debug_bounds(remote_selector).is_some());
        click(cx, &clock, "cancel-remove-machine");
        cx.update(|_, cx| cx.quit());
    });
    gpui::run_test(1, &[], 0, &mut |dispatcher, _| scenario(dispatcher), None);
    runtime.block_on(rig.into_inner().close()).unwrap();
}

/// Run the public Desktop bootstrap and app-quit lifecycle with real helpers.
/// A subprocess provides private environment variables without mutating this test process.
#[test]
fn owned_helpers_stop_on_app_quit_while_reused_helpers_stay_running() {
    const CHILD: &str = "LANKIDE_QUIT_E2E_CHILD";
    const NAME: &str = concat!(
        module_path!(),
        "::owned_helpers_stop_on_app_quit_while_reused_helpers_stay_running"
    );
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::Builder::new()
            .prefix("lkq-")
            .tempdir_in("/tmp")
            .unwrap();
        let current = std::env::current_exe().unwrap();
        let binaries = current.parent().unwrap().parent().unwrap();
        let executable = directory.path().join("quit-e2e");
        std::fs::hard_link(&current, &executable).unwrap();
        for name in ["lankided", "lankide-machine"] {
            std::fs::hard_link(binaries.join(name), directory.path().join(name)).unwrap();
        }
        let home = directory.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let output = std::process::Command::new(executable)
            .args(["--exact", NAME.split_once("::").unwrap().1, "--nocapture"])
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
    let pids: Vec<String> = ["runtime.lock", "machine.lock"]
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
        let clock = std::cell::Cell::new(Instant::now());
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
        assert!(!owner.local_home.join("run/machine.lock").exists());
    });
    gpui::run_test(1, &[], 0, &mut |dispatcher, _| scenario(dispatcher), None);
}
