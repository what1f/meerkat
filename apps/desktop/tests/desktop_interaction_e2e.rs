#![recursion_limit = "256"]
#[path = "../src/assets.rs"]
mod assets;
#[path = "../../../crates/client/tests/support/management.rs"]
mod management_fixture;
#[path = "../src/ui/mod.rs"]
mod ui;
use desktop_ui::{DesktopLanguage, DesktopTheme, apply_component_theme};
use gpui::{
    AppContext, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point,
    px,
};
use gpui_component::Root;
use lankide_client::ManagementClient;
use management_fixture::MockManagementClient;
use std::{sync::Arc, time::Duration};
use ui::onboarding::next_agent_onboarding_request;
use ui::{
    InitialUiState, LankideApp, LankideFrame, UiMessage, localized_agent_onboarding_copy,
    observe_dialog_root,
};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.draw(cx).clear(cx);
    });
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not rendered"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
}
fn click_and_draw_one_frame(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not rendered"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}
fn fill(cx: &mut VisualTestContext, selector: &'static str, value: &str) {
    click(cx, selector);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input(value);
    draw(cx);
}
async fn run_workspace_agent_and_onboarding_dialog_workflows(cx: &mut TestAppContext) {
    let cached_error = UiMessage::client(lankide_client::ClientError::Unavailable(
        "transport detail must not be shown".into(),
    ));
    assert_eq!(
        cached_error.text(DesktopLanguage::English),
        "Cannot connect to Lankide. Try again shortly."
    );
    assert_eq!(
        cached_error.text(DesktopLanguage::SimplifiedChinese),
        "暂时无法连接 Lankide，请稍后重试。"
    );
    let raw_provider_state = lankide_client::AgentOnboardingState {
        workspace_id: "ui_e2e".into(),
        agent_id: "builder".into(),
        provider_id: "feishu".into(),
        step: "create_app".into(),
        status: lankide_client::OnboardingStatus::ActionRequired,
        verification_uri: Some("https://open.feishu.cn/example".into()),
        poll_interval_seconds: None,
        payload: serde_json::json!({"verification_uri":"https://open.feishu.cn/example"}),
        error: None,
    };
    assert_eq!(
        localized_agent_onboarding_copy(&raw_provider_state, DesktopLanguage::English),
        (
            "Confirm setup".into(),
            "Open the link below to finish setup, then return to check the status.".into()
        )
    );
    let localized =
        localized_agent_onboarding_copy(&raw_provider_state, DesktopLanguage::SimplifiedChinese);
    assert!(!localized.1.starts_with("飞书："));
    let (action, payload) = next_agent_onboarding_request(&raw_provider_state);
    assert_eq!(action, "start");
    assert_eq!(payload, serde_json::json!({}));
    let mut polling_state = raw_provider_state.clone();
    polling_state.poll_interval_seconds = Some(5);
    polling_state.payload = serde_json::json!({"opaque": "provider-owned"});
    assert_eq!(
        localized_agent_onboarding_copy(&polling_state, DesktopLanguage::SimplifiedChinese),
        ("等待确认".into(), "打开下方链接完成设置".into())
    );
    let (action, payload) = next_agent_onboarding_request(&polling_state);
    assert_eq!(action, "poll");
    assert_eq!(payload, polling_state.payload);
    let mut permissions_state = raw_provider_state.clone();
    permissions_state.step = "permissions".into();
    permissions_state.poll_interval_seconds = Some(5);
    permissions_state.payload = serde_json::json!({
        "phase": "permissions",
        "app_id": "cli_permissions",
        "verification_uri": "https://open.feishu.cn/page/launcher"
    });
    assert_eq!(
        localized_agent_onboarding_copy(&permissions_state, DesktopLanguage::SimplifiedChinese),
        (
            "应用已创建，继续授权".into(),
            "还需完成 Agent 能力授权，完成后将自动启用。".into()
        )
    );
    assert_eq!(
        localized_agent_onboarding_copy(&permissions_state, DesktopLanguage::English),
        (
            "App created — continue authorization".into(),
            "Authorize Agent capabilities to finish setup.".into()
        )
    );
    let (action, payload) = next_agent_onboarding_request(&permissions_state);
    assert_eq!(action, "poll");
    assert_eq!(payload, permissions_state.payload);
    permissions_state.step = "authorization_update".into();
    permissions_state.verification_uri = None;
    permissions_state.payload = serde_json::json!({
        "phase": "permissions",
        "flow": "authorization_update",
        "app_id": "cli_permissions",
        "submitted": true
    });
    assert_eq!(
        localized_agent_onboarding_copy(&permissions_state, DesktopLanguage::SimplifiedChinese),
        ("更新飞书权限".into(), "更新后即可继续使用完整功能。".into())
    );
    permissions_state.verification_uri = Some("https://open.feishu.cn/page/launcher".into());
    assert_eq!(
        localized_agent_onboarding_copy(&permissions_state, DesktopLanguage::SimplifiedChinese),
        (
            "在飞书中确认权限更新".into(),
            "打开下方链接完成确认。".into()
        )
    );
    cx.update(|cx| {
        gpui_component::init(cx);
        apply_component_theme(DesktopTheme::light(), cx);
    });
    let client = MockManagementClient::sample();
    let view_client = Arc::new(client.clone());
    {
        let mounted = Arc::new(std::sync::Mutex::new(None));
        let mounted_for_build = mounted.clone();
        let (root, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|cx| {
                LankideApp::new(
                    view_client,
                    None,
                    Some(InitialUiState {
                        workspaces: vec![],
                        selected_workspace_id: None,
                        agents: vec![],
                    }),
                    false,
                    window,
                    cx,
                )
            });
            let frame = cx.new(|cx| LankideFrame::new(view.clone(), window, cx));
            *mounted_for_build.lock().unwrap() = Some((view, frame.clone()));
            Root::new(frame, window, cx).bordered(false)
        });
        let (_view, frame) = mounted.lock().unwrap().take().unwrap();
        cx.update(|window, cx| observe_dialog_root(&frame, &root, window, cx));
        click_and_draw_one_frame(cx, "first-workspace");
        assert!(
            cx.debug_bounds("workspace-id-input").is_some(),
            "Root dialog changes must redraw the child-owned dialog layer in one frame"
        );
        click(cx, "cancel-workspace");
        click(cx, "nav-settings");
        click(cx, "language-select");
        cx.simulate_keystrokes("down enter");
        draw(cx);
        click(cx, "nav-agents");
        click(cx, "first-workspace");
        fill(cx, "workspace-id-input", "ui_e2e");
        fill(cx, "workspace-name-input", "UI E2E");
        click_and_draw_one_frame(cx, "workspace-next");
        assert!(
            cx.debug_bounds("provider-feishu").is_some(),
            "workspace wizard step changes must redraw the dialog in one frame"
        );
        click(cx, "create-workspace-submit");
        assert_eq!(client.get_workspace("ui_e2e").await.unwrap().name, "UI E2E");
        click(cx, "add-agent");
        fill(cx, "agent-id-input", "builder");
        fill(cx, "agent-name-input", "Builder");
        click(cx, "save-agent");
        assert!(
            client.get_agent("ui_e2e", "builder").await.is_err(),
            "empty model must not create Agent"
        );
        assert!(cx.debug_bounds("model-directory").is_none());
        fill(cx, "agent-model-input", "manual-model-id");
        client
            .fail_next_onboarding(lankide_client::ClientError::Protocol(
                "temporary onboarding failure".into(),
            ))
            .await;
        click(cx, "save-agent");
        assert_eq!(
            client.get_agent("ui_e2e", "builder").await.unwrap().name,
            "Builder"
        );
        assert!(cx.debug_bounds("agent-card-builder").is_some());
        assert!(
            cx.debug_bounds("application-error").is_some(),
            "activation failure must remain visible"
        );
        assert!(
            cx.debug_bounds("application-notice").is_none(),
            "failed activation cannot show success"
        );
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-0");
        assert!(
            cx.debug_bounds("connect-agent").is_some(),
            "connection recovery belongs in the Agent editor"
        );
        assert!(
            cx.debug_bounds("resume-agent-0").is_none(),
            "Agent cards must not expose onboarding recovery actions"
        );
        click(cx, "connect-agent");
        assert!(cx.debug_bounds("application-error").is_none());
        assert!(
            cx.debug_bounds("application-notice").is_none(),
            "Agent onboarding completion must not leave a persistent page banner"
        );
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        let mut connected = client.get_agent("ui_e2e", "builder").await.unwrap();
        connected.provider_handle = "cli_builder".into();
        connected.provider_agent_config = serde_json::json!({
            "mode":"existing",
            "app_id":"cli_builder",
            "provider_handle":"cli_builder",
            "domain":"feishu"
        });
        client.update_agent(connected, None).await.unwrap();
        client
            .set_agent_onboarding(lankide_client::AgentOnboardingState {
                workspace_id: "ui_e2e".into(),
                agent_id: "builder".into(),
                provider_id: "feishu".into(),
                step: "authorization_update".into(),
                status: lankide_client::OnboardingStatus::ActionRequired,
                verification_uri: None,
                poll_interval_seconds: Some(5),
                payload: serde_json::json!({
                    "phase":"permissions",
                    "flow":"authorization_update",
                    "app_id":"cli_builder",
                    "submitted":false,
                    "interval_seconds":5
                }),
                error: None,
            })
            .await;
        click(cx, "edit-agent-0");
        assert!(
            cx.debug_bounds("update-feishu-permissions").is_some(),
            "existing Feishu Agents must expose missing permission recovery in the editor"
        );
        fill(cx, "agent-name-input", "Builder Updated");
        client
            .fail_next(lankide_client::ClientError::Provider(
                "missing application permission".into(),
            ))
            .await;
        click(cx, "save-agent");
        assert!(cx.debug_bounds("update-feishu-permissions").is_some());
        click(cx, "update-feishu-permissions");
        assert_eq!(
            client.get_agent("ui_e2e", "builder").await.unwrap().name,
            "Builder Updated",
            "saving must retry automatically after the permission update completes"
        );
        assert_eq!(
            client
                .get_agent("ui_e2e", "builder")
                .await
                .unwrap()
                .provider_handle,
            "cli_builder",
            "editing a stale Agent must preserve the verified Provider identity"
        );
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-0");
        let save_gate = client.delay_next_agent_update("builder").await;
        click_and_draw_one_frame(cx, "save-agent");
        assert!(
            cx.debug_bounds("desktop-button-loading-save-agent")
                .is_some(),
            "saving an Agent profile must show a visible loading indicator"
        );
        save_gate.notify_one();
        draw(cx);
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-0");
        click(cx, "reaction-picker-trigger");
        let reaction_panel = cx.debug_bounds("reaction-picker-content").unwrap();
        let reaction_trigger = cx.debug_bounds("reaction-picker-trigger").unwrap();
        assert!(
            reaction_panel.origin.y + reaction_panel.size.height <= reaction_trigger.origin.y,
            "reaction picker must open above its trigger: panel={reaction_panel:?} trigger={reaction_trigger:?}"
        );
        assert!(
            reaction_panel.origin.y >= px(0.),
            "reaction picker must stay inside the window"
        );
        let last_reaction_y = cx.debug_bounds("reaction-test-47").unwrap().origin.y;
        cx.simulate_event(ScrollWheelEvent {
            position: reaction_panel.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-1000.))),
            ..Default::default()
        });
        draw(cx);
        let last_reaction = cx.debug_bounds("reaction-test-47").unwrap();
        assert!(
            last_reaction.origin.y < last_reaction_y,
            "the reaction grid must move when the user scrolls"
        );
        assert!(
            last_reaction.origin.y + last_reaction.size.height
                <= reaction_panel.origin.y + reaction_panel.size.height,
            "the final reaction must be fully reachable inside the rounded panel"
        );
        click(cx, "agent-name-input");
        assert!(
            cx.debug_bounds("reaction-picker-content").is_none(),
            "clicking outside must dismiss the reaction picker"
        );
        click(cx, "reaction-picker-trigger");
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(
            cx.debug_bounds("reaction-picker-content").is_none(),
            "Escape must dismiss the reaction picker without closing the editor"
        );
        assert!(cx.debug_bounds("cancel-agent").is_some());
        click(cx, "reaction-picker-trigger");
        click(cx, "reaction-none");
        click(cx, "reaction-picker-trigger");
        click(cx, "cancel-agent");
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-0");
        assert!(
            cx.debug_bounds("reaction-picker-content").is_none(),
            "reopening the Agent editor must start with the reaction picker closed"
        );
        click(cx, "advanced-toggle");
        let footer = cx.debug_bounds("cancel-agent").unwrap();
        let viewport_height = cx.update(|window, _| window.viewport_size().height);
        assert!(
            footer.origin.y + footer.size.height <= viewport_height,
            "advanced Agent settings must keep the dialog footer inside the viewport"
        );
        let scroll_area = cx.debug_bounds("agent-edit-scroll-area").unwrap();
        cx.simulate_event(ScrollWheelEvent {
            position: scroll_area.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            ..Default::default()
        });
        draw(cx);
        let advanced = cx.debug_bounds("agent-advanced-settings").unwrap();
        assert!(
            advanced.bottom() <= scroll_area.bottom() - px(20.),
            "scrolling to the bottom must reveal the complete advanced settings card with space above the footer"
        );
        click(cx, "cancel-agent");
        click(cx, "add-agent");
        click(cx, "agent-mode-existing");
        fill(cx, "agent-id-input", "existing_builder");
        fill(cx, "agent-name-input", "Existing Builder");
        fill(cx, "agent-app-id-input", "cli_existing_builder");
        fill(
            cx,
            "agent-app-secret-input",
            "external-provider-test-secret",
        );
        assert!(cx.debug_bounds("model-directory").is_none());
        fill(cx, "agent-model-input", "manual-model-id");
        client
            .fail_next_onboarding(lankide_client::ClientError::Protocol(
                "temporary onboarding read failure".into(),
            ))
            .await;
        click(cx, "save-agent");
        assert!(client.get_agent("ui_e2e", "existing_builder").await.is_ok());
        assert!(cx.debug_bounds("application-error").is_some());
        assert!(cx.debug_bounds("application-notice").is_none());
        let card = cx.debug_bounds("agent-card-existing_builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-1");
        assert!(cx.debug_bounds("connect-agent").is_some());
        click(cx, "connect-agent");
        assert!(cx.debug_bounds("application-error").is_none());
        assert!(
            cx.debug_bounds("application-notice").is_none(),
            "recovered onboarding must not leave a persistent page banner"
        );
        let card = cx.debug_bounds("agent-card-existing_builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "delete-agent-1");
        assert!(cx.debug_bounds("confirm-delete-agent").is_some());
        click(cx, "confirm-delete-agent");
        assert!(
            client
                .get_agent("ui_e2e", "existing_builder")
                .await
                .is_err()
        );
        assert!(
            cx.debug_bounds("confirm-delete-agent").is_none(),
            "successful deletion must close the confirmation dialog"
        );
        assert!(cx.debug_bounds("agent-card-existing_builder").is_none());

        let onboarding_gate = client.delay_next_onboarding("builder").await;
        client
            .set_agent_onboarding(lankide_client::AgentOnboardingState {
                workspace_id: "ui_e2e".into(),
                agent_id: "builder".into(),
                provider_id: "feishu".into(),
                step: "create_app".into(),
                status: lankide_client::OnboardingStatus::ActionRequired,
                verification_uri: Some("https://open.feishu.cn/example".into()),
                poll_interval_seconds: Some(5),
                payload: serde_json::json!({
                    "device_code": "pending-device",
                    "verification_uri": "https://open.feishu.cn/example",
                    "interval_seconds": 5
                }),
                error: None,
            })
            .await;
        let card = cx.debug_bounds("agent-card-builder").unwrap();
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        click(cx, "edit-agent-0");
        click(cx, "connect-agent");
        for _ in 0..5 {
            draw(cx);
        }
        assert!(
            cx.debug_bounds("desktop-dialog-close").is_some(),
            "checking onboarding must keep the dialog close action available"
        );
        assert!(
            cx.debug_bounds("agent-onboarding-spinner").is_some(),
            "automatic onboarding polling must show its waiting indicator"
        );
        assert!(
            cx.debug_bounds("continue-agent-onboarding").is_none(),
            "automatic onboarding polling must not render a redundant manual check"
        );
        click(cx, "close-agent-onboarding");
        assert!(
            cx.debug_bounds("continue-agent-onboarding").is_none(),
            "onboarding must close while a check is in flight"
        );
        onboarding_gate.notify_one();
        draw(cx);
        assert!(
            cx.debug_bounds("continue-agent-onboarding").is_none(),
            "a late onboarding result must not reopen a dismissed dialog"
        );
    }
    let race_client = MockManagementClient::sample();
    let mut failed_workspace = race_client.get_workspace("example-team").await.unwrap();
    failed_workspace.observed_state = lankide_client::WorkspaceObservedState::Error;
    race_client
        .update_workspace(failed_workspace)
        .await
        .unwrap();
    let workspaces = race_client.list_workspaces().await.unwrap();
    let mut stored = race_client
        .get_agent("example-team", "clark")
        .await
        .unwrap();
    stored.pause_reason = Some("model not found".into());
    stored.avatar = lankide_client::Avatar::Path("resources/example-team/clark_avatar.png".into());
    race_client.update_agent(stored, None).await.unwrap();
    let agents = race_client.list_agents("example-team").await.unwrap();
    let delayed = race_client.delay_next_agents("tg-team").await;
    let view_client = Arc::new(race_client.clone());
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            LankideApp::new(
                view_client,
                None,
                Some(InitialUiState {
                    workspaces,
                    selected_workspace_id: Some("example-team".into()),
                    agents,
                }),
                false,
                window,
                cx,
            )
        });
        let frame = cx.new(|cx| LankideFrame::new(view, window, cx));
        Root::new(frame, window, cx).bordered(false)
    });
    draw(cx);
    assert!(cx.debug_bounds("agent-paused").is_some());
    assert!(cx.debug_bounds("resume-agent-clark").is_some());
    assert_eq!(
        race_client.image_requests().await,
        vec![("example-team".into(), "clark".into())],
        "persisted Agent avatars must load from the initial Desktop state"
    );
    click(cx, "workspace-switcher");
    cx.simulate_keystrokes("down down enter");
    draw(cx);
    assert!(
        cx.debug_bounds("agent-card-clark").is_none(),
        "switch must clear previous workspace cards immediately"
    );
    click(cx, "workspace-switcher");
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(cx.debug_bounds("agent-card-clark").is_some());
    delayed.notify_one();
    draw(cx);
    assert!(
        cx.debug_bounds("agent-card-clark").is_some(),
        "late response replaced selected workspace"
    );
    assert!(cx.debug_bounds("agent-card-bruce").is_none());
    assert_eq!(
        race_client.image_requests().await,
        vec![
            ("example-team".into(), "clark".into()),
            ("example-team".into(), "clark".into())
        ]
    );
    assert!(
        cx.debug_bounds("application-error").is_none(),
        "real PNG image failed to decode/render"
    );
    click(cx, "nav-workspace");
    assert!(
        cx.debug_bounds("toggle-runtime").is_none(),
        "runtime controls must not remain on the workspace configuration page"
    );
    click(cx, "nav-settings");
    assert!(
        cx.debug_bounds("workspace-switcher").is_none(),
        "runtime settings must not be tied to a workspace switcher"
    );
    let toggle_gate = race_client.delay_next_workspace_state("example-team").await;
    click_and_draw_one_frame(cx, "toggle-runtime");
    assert!(
        cx.debug_bounds("toggle-runtime-loading").is_some(),
        "the runtime state action must show its own loading indicator"
    );
    assert!(
        cx.debug_bounds("desktop-button-loading-save-workspace")
            .is_none(),
        "workspace state changes must not make the save button look busy"
    );
    toggle_gate.notify_one();
    draw(cx);
    assert!(
        cx.debug_bounds("application-error").is_some(),
        "a failed observed state must remain an error after a successful state request"
    );
    assert!(
        cx.debug_bounds("application-notice").is_none(),
        "desired state alone cannot produce a success notice"
    );
    click(cx, "nav-agents");
    assert!(cx.debug_bounds("agent-paused").is_some());
    click(cx, "resume-agent-clark");
    assert!(
        race_client
            .get_agent("example-team", "clark")
            .await
            .unwrap()
            .pause_reason
            .is_none()
    );
    assert!(cx.debug_bounds("agent-paused").is_none());
}
#[::core::prelude::v1::test]
fn workspace_agent_and_onboarding_dialog_workflows() {
    gpui::run_test(
        1,
        &[],
        0,
        &mut |dispatcher, _seed| {
            let executor = Arc::new(dispatcher.clone());
            let mut cx = TestAppContext::build(
                dispatcher.clone(),
                Some("workspace_agent_and_onboarding_dialog_workflows"),
            );
            let _entity_refcounts = cx.app.borrow().ref_counts_drop_handle();

            gpui::ForegroundExecutor::new(executor.clone())
                .block_test(run_workspace_agent_and_onboarding_dialog_workflows(&mut cx));

            drop(executor);
            cx.run_until_parked();
            cx.update(|cx| {
                cx.background_executor().forbid_parking();
                cx.quit();
            });
            cx.run_until_parked();
            drop(cx);
            dispatcher.drain_tasks();
            drop(dispatcher);
        },
        None,
    );
}
