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
            apply_component_theme(DesktopTheme::light(), cx);
        });
        let mounted = Arc::new(std::sync::Mutex::new(None));
        let capture = mounted.clone();
        let view_client = client.clone();
        let local_home = rig.runtime_home().to_path_buf();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(Ok(())).unwrap();
        let (root, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                LankideApp::new_with_startup(view_client, rx, local_home, false, window, cx)
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
        click(cx, "pair-computer");
        wait(cx, |cx| cx.debug_bounds("pairing-code").is_some());
        assert!(
            cx.debug_bounds("copy-pairing-command").is_none(),
            "loopback address exposed as a remote command"
        );
        click(cx, "copy-pairing-code");
        let copied = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
        assert!(!copied.is_empty());
        let paired_home = tempfile::tempdir_in("/tmp").unwrap();
        runtime
            .block_on(pair(
                &ComputerDaemonPaths::for_home(paired_home.path()).unwrap(),
                &url,
                &copied,
            ))
            .unwrap();
        click(cx, "close-pairing");
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
