use desktop_ui::{
    ContentPane, DesktopAvatar, DesktopBanner, DesktopButton, DesktopCenteredMessage,
    DesktopCheckbox, DesktopChip, DesktopColorIcon, DesktopDialog, DesktopDialogFooter,
    DesktopDialogHeader, DesktopEmptyState, DesktopFormField, DesktopInput, DesktopLanguage,
    DesktopSelect, DesktopShell, DesktopSteps, DesktopTextarea, DesktopTheme, PaneHeader, Sidebar,
    SidebarItem, ThemePreference, apply_component_theme, configure_window_material, desktop_choice,
    desktop_dialog_button, desktop_dialog_cancel, desktop_settings_group, desktop_settings_row,
    desktop_settings_section_title,
};
use gpui::prelude::{FluentBuilder as _, StatefulInteractiveElement as _};
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IndexPath, Root, Sizable as _, Theme as ComponentTheme,
    ThemeMode as ComponentThemeMode, WindowExt as _,
    input::{InputContentType, InputEvent, InputState, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::{Notification, NotificationType},
    select::{SelectEvent, SelectItem, SelectState},
    spinner::Spinner,
};
use lankide_client::{
    Agent, AgentAvatarUpload, AgentDraft, AgentOnboardingState, Avatar, ClientError,
    ManagementClient, OnboardingStatus, PROVIDER_CAPABILITY_TASK_SYSTEM, ReactionOption,
    ThinkingLevel, Workspace, WorkspaceDraft, WorkspaceStatus,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

const BOOT_LOADER_DELAY: Duration = Duration::from_millis(300);
const BOOT_LOADER_MIN_VISIBLE: Duration = Duration::from_millis(600);

mod agent_actions;
mod components;
mod computers;
pub(crate) use computers::local_computer_id;
mod dialogs;
mod errors;
pub(super) mod onboarding;
mod pages;
mod reaction_picker;
mod workspace_actions;

use agent_actions::{new_environment_row, selected_harness_id};
use components::*;
use computers::ComputerChoice;
use errors::friendly_error;
pub(super) use onboarding::localized_agent_onboarding_copy;
use onboarding::telegram_bot_username;
use reaction_picker::ReactionPicker;

const MAX_AGENT_AVATAR_BYTES: usize = 3 * 1024 * 1024;

fn convert_rgba_to_bgra(mut pixels: image::RgbaImage) -> image::RgbaImage {
    // GPUI RenderImage stores decoded pixels in BGRA order.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    pixels
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Agents,
    Workspace,
    Settings,
}

#[derive(Clone)]
enum Modal {
    CreateWorkspace { step: usize },
    CreateAgent,
    PairComputer,
    EditAgent(String),
    DeleteAgent(String),
    AgentOnboarding(AgentOnboardingState),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BusyOperation {
    DeleteAgent,
    CreateWorkspace,
    SaveAgent,
    SaveWorkspace,
    ToggleRuntime,
    ResumeAgentOnboarding,
}

#[derive(Clone)]
struct AgentEnvironmentRow {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

#[derive(Clone)]
struct PendingAgentAvatar {
    media_type: String,
    bytes: Vec<u8>,
    image: Arc<RenderImage>,
}

#[derive(Clone)]
pub(super) enum UiMessage {
    Localized { zh: String, en: String },
    Client(ClientError),
}

impl UiMessage {
    pub(super) fn localized(zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Localized {
            zh: zh.into(),
            en: en.into(),
        }
    }

    pub(super) fn client(error: ClientError) -> Self {
        Self::Client(error)
    }

    pub(super) fn text(&self, language: DesktopLanguage) -> String {
        match self {
            Self::Localized { zh, en } => match language {
                DesktopLanguage::SimplifiedChinese => zh.clone(),
                DesktopLanguage::English => en.clone(),
            },
            Self::Client(error) => friendly_error(error, language),
        }
    }
}

struct AppNotification;

fn push_notification(
    cx: &mut gpui::AsyncApp,
    window_handle: AnyWindowHandle,
    notification_type: NotificationType,
    message: impl Into<SharedString>,
) {
    let notification = Notification::new()
        .id::<AppNotification>()
        .with_type(notification_type)
        .message(message);
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.push_notification(notification, cx)
    });
}

pub struct LankideApp {
    client: Arc<dyn ManagementClient>,
    started: bool,
    page: Page,
    workspaces: Vec<Workspace>,
    provider_capabilities: HashMap<String, Vec<String>>,
    selected_workspace_id: Option<String>,
    selection_epoch: u64,
    workspace_request: u64,
    workspace_startup_retries: u8,
    workspace_loaded: bool,
    booting: bool,
    boot_loader_shown_at: Option<Instant>,
    startup_pending: bool,
    agent_request: u64,
    provider_request: u64,
    reaction_request: u64,
    image_request: u64,
    images: std::collections::HashMap<String, Arc<RenderImage>>,
    agents: Vec<Agent>,
    onboarding_retries: std::collections::HashSet<(String, String)>,
    onboarding_steps: std::collections::HashMap<(String, String), String>,
    onboarding_return_to_agent: Option<String>,
    pending_agent_save_retry: Option<String>,
    onboarding_poll_epoch: u64,
    onboarding_checking: bool,
    telegram_manager_ready: bool,
    loading: bool,
    busy: Option<BusyOperation>,
    error: Option<UiMessage>,
    modal: Option<Modal>,
    modal_presented: bool,
    language: DesktopLanguage,
    language_select: Entity<SelectState<Vec<String>>>,
    theme_preference: ThemePreference,
    dark: bool,
    hovered_agent_id: Option<String>,

    workspace_id_input: Entity<InputState>,
    workspace_name_input: Entity<InputState>,
    workspace_manager_token_input: Entity<InputState>,
    workspace_provider: String,
    workspace_task_enabled: bool,
    workspace_id_error: Option<UiMessage>,
    workspace_name_error: Option<UiMessage>,
    workspace_token_error: Option<UiMessage>,

    settings_name_input: Entity<InputState>,
    settings_rules_input: Entity<TextareaState>,
    settings_task_name_input: Entity<InputState>,
    settings_task_enabled: bool,

    agent_id_input: Entity<InputState>,
    agent_name_input: Entity<InputState>,
    agent_role_input: Entity<InputState>,
    agent_bio_input: Entity<InputState>,
    agent_soul_input: Entity<TextareaState>,
    agent_environment_rows: Vec<AgentEnvironmentRow>,
    agent_model_input: Entity<InputState>,
    agent_cli_input: Entity<InputState>,
    agent_provider_handle_input: Entity<InputState>,
    agent_secret_input: Entity<InputState>,
    agent_id_error: Option<UiMessage>,
    agent_name_error: Option<UiMessage>,
    agent_model_error: Option<UiMessage>,
    agent_provider_handle_error: Option<UiMessage>,
    agent_secret_error: Option<UiMessage>,
    local_computer_id: Option<String>,
    pairing_address: Option<String>,
    pairing_pin: Option<String>,
    pairing_address_input: Entity<InputState>,
    pairing_editing: bool,
    pairing_copied: bool,
    pairing_connected: Option<String>,
    computers: Vec<lankide_client::ComputerOption>,
    computer_request: u64,
    pairing_code: Option<lankide_client::PairingCode>,
    pairing_loading: bool,
    pairing_request: u64,
    pairing_waiting_hint: bool,
    pairing_hint_task: Option<gpui::Task<()>>,
    settings_computers_task: Option<gpui::Task<()>>,
    agent_computer: Entity<SelectState<Vec<ComputerChoice>>>,
    agent_harness: Entity<SelectState<Vec<HarnessChoice>>>,
    agent_thinking: Entity<SelectState<Vec<ThinkingChoice>>>,
    agent_reaction: Option<String>,
    reaction_options: Vec<ReactionOption>,
    reaction_loading: bool,
    reaction_error: Option<UiMessage>,
    pending_agent_avatar: Option<PendingAgentAvatar>,
    advanced_open: bool,
    agent_mode_new: bool,
    agent_provider_handle_edited: bool,
}

pub(crate) struct LankideFrame {
    view: Entity<LankideApp>,
}

impl LankideFrame {
    pub(crate) fn new(
        view: Entity<LankideApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_in(&view, window, |_, _, window, cx| {
            cx.notify();
            window.refresh();
        })
        .detach();
        Self { view }
    }
}

impl Render for LankideFrame {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);
        div()
            .relative()
            .size_full()
            .child(self.view.clone())
            .children(dialog_layer.map(|layer| {
                div()
                    // Keep the layer identity stable. Changing this key on every
                    // app notification remounts the Dialog and restarts its
                    // entrance animation, including for ordinary text input.
                    .id("lankide-dialog-layer")
                    .child(layer)
            }))
            .children(notification_layer)
    }
}

pub(crate) fn observe_dialog_root(
    frame: &Entity<LankideFrame>,
    root: &Entity<Root>,
    window: &mut Window,
    cx: &mut App,
) {
    let frame = frame.clone();
    window
        .observe(root, cx, move |_, window, cx| {
            frame.update(cx, |_, cx| cx.notify());
            window.refresh();
            let frame = frame.clone();
            window.on_next_frame(move |window, cx| {
                frame.update(cx, |_, cx| cx.notify());
                window.refresh();
            });
        })
        .detach();
}

#[derive(Clone)]
pub(crate) struct InitialUiState {
    pub workspaces: Vec<Workspace>,
    pub selected_workspace_id: Option<String>,
    pub agents: Vec<Agent>,
}

impl LankideApp {
    pub fn new_with_startup(
        client: Arc<dyn ManagementClient>,
        startup_result: tokio::sync::oneshot::Receiver<Result<(), ClientError>>,
        local_home: std::path::PathBuf,
        initial_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut app = Self::new(client, None, None, initial_dark, window, cx);
        app.startup_pending = true;
        cx.spawn(async move |this, cx| {
            let result = startup_result.await;
            let _ = this.update(cx, |this, cx| {
                this.startup_pending = false;
                match result {
                    Ok(Ok(())) => {
                        this.read_local_computer(&local_home);
                        this.reload_workspaces(cx);
                    }
                    Ok(Err(error)) => {
                        // Drop the pending workspace retry so it cannot clear this error.
                        this.workspace_request += 1;
                        this.loading = false;
                        this.error = Some(UiMessage::client(error));
                        this.finish_boot(cx);
                        cx.notify();
                    }
                    Err(_) => {}
                }
            });
            cx.refresh();
        })
        .detach();
        app
    }

    pub fn new(
        client: Arc<dyn ManagementClient>,
        startup_error: Option<ClientError>,
        initial_state: Option<InitialUiState>,
        initial_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = DesktopLanguage::default();
        let workspace_id_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("以小写字母开头", "Start with a lowercase letter"))
        });
        let workspace_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("填写工作区名称", "Enter workspace name"))
        });
        let workspace_manager_token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("填写 Manager Bot Token", "Enter Manager Bot Token"))
                .masked(true)
        });
        let settings_name_input = cx.new(|cx| InputState::new(window, cx));
        let settings_rules_input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(8, 20));
        let settings_task_name_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.text("团队任务", "Team tasks"))
        });
        let agent_id_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("以小写字母开头", "Start with a lowercase letter"))
        });
        let agent_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("填写显示名称", "Enter display name"))
        });
        let agent_role_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("职位或职责", "Role or responsibility"))
        });
        let agent_bio_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(language.text(
                "一句话介绍这位 Agent",
                "Describe this Agent in one sentence",
            ))
        });
        let agent_soul_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(6, 18)
                .placeholder(language.text(
                    "描述它的工作方式、偏好和边界…",
                    "Describe how it works, its preferences and boundaries…",
                ))
        });
        let agent_environment_rows = vec![new_environment_row("", "", window, cx)];
        let agent_model_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(language.text("选择或填写模型 ID", "Choose or enter model ID"))
        });
        let agent_cli_input = cx.new(|cx| InputState::new(window, cx));
        let agent_provider_handle_input = cx.new(|cx| InputState::new(window, cx));
        let agent_secret_input = cx.new(|cx| InputState::new(window, cx).masked(true));
        let agent_harness = cx.new(|cx| {
            SelectState::new(
                vec![
                    HarnessChoice::new("pi", "Pi"),
                    HarnessChoice::new("claude_code", "Claude Code"),
                    HarnessChoice::new("codex", "Codex"),
                ],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let agent_thinking = cx.new(|cx| {
            SelectState::new(
                vec![
                    ThinkingChoice::new("none"),
                    ThinkingChoice::new("off"),
                    ThinkingChoice::new("minimal"),
                    ThinkingChoice::new("low"),
                    ThinkingChoice::new("medium"),
                    ThinkingChoice::new("high"),
                    ThinkingChoice::new("xhigh"),
                    ThinkingChoice::new("max"),
                ],
                Some(IndexPath::default().row(5)),
                window,
                cx,
            )
        });
        let agent_computer =
            cx.new(|cx| SelectState::new(Vec::<ComputerChoice>::new(), None, window, cx));
        let language_select = cx.new(|cx| {
            SelectState::new(
                vec!["简体中文".into(), "English".into()],
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        cx.subscribe_in(
            &language_select,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                let language = if *value == "English" {
                    DesktopLanguage::English
                } else {
                    DesktopLanguage::SimplifiedChinese
                };
                this.set_language(language, window, cx);
            },
        )
        .detach();
        cx.subscribe_in(
            &workspace_id_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && this.workspace_id_error.take().is_some() {
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &workspace_name_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && this.workspace_name_error.take().is_some()
                {
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &workspace_manager_token_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change)
                    && this.workspace_token_error.take().is_some()
                {
                    cx.notify();
                }
            },
        )
        .detach();
        let generated_handle_input = agent_provider_handle_input.clone();
        cx.subscribe_in(
            &agent_id_input,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let cleared_error = this.agent_id_error.take().is_some();
                if this.agent_provider_handle_edited
                    || !this.agent_mode_new
                    || this
                        .selected_workspace()
                        .map(|workspace| workspace.provider.id.as_str())
                        != Some("telegram")
                {
                    if cleared_error {
                        cx.notify();
                    }
                    return;
                }
                let workspace_id = this.selected_workspace_id.as_deref().unwrap_or_default();
                let username = telegram_bot_username(workspace_id, &input.read(cx).value());
                generated_handle_input
                    .update(cx, |state, cx| state.set_value(username, window, cx));
                cx.notify();
            },
        )
        .detach();
        cx.subscribe_in(
            &agent_name_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && this.agent_name_error.take().is_some() {
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &agent_model_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let cleared_error = this.agent_model_error.take().is_some();
                    let editing = matches!(&this.modal, Some(Modal::EditAgent(_)));
                    if cleared_error || editing {
                        cx.notify();
                    }
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &agent_provider_handle_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.agent_provider_handle_edited = true;
                    if this.agent_provider_handle_error.take().is_some() {
                        cx.notify();
                    }
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &agent_secret_input,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && this.agent_secret_error.take().is_some() {
                    cx.notify();
                }
            },
        )
        .detach();

        cx.subscribe_in(
            &agent_harness,
            window,
            |this, _, event: &SelectEvent<Vec<HarnessChoice>>, window, cx| {
                if matches!(event, SelectEvent::Confirm(Some(_))) {
                    this.update_thinking_choices(window, cx);
                }
            },
        )
        .detach();

        let has_initial_state = initial_state.is_some();
        let initial_state = initial_state.unwrap_or_else(|| InitialUiState {
            workspaces: Vec::new(),
            selected_workspace_id: None,
            agents: Vec::new(),
        });

        let mut app = Self {
            client,
            started: has_initial_state,
            page: Page::Agents,
            workspaces: initial_state.workspaces,
            provider_capabilities: HashMap::new(),
            selected_workspace_id: initial_state.selected_workspace_id,
            selection_epoch: 0,
            workspace_request: 0,
            workspace_startup_retries: 0,
            workspace_loaded: has_initial_state,
            booting: !has_initial_state,
            boot_loader_shown_at: None,
            startup_pending: false,
            agent_request: 0,
            provider_request: 0,
            reaction_request: 0,
            image_request: 0,
            onboarding_retries: std::collections::HashSet::new(),
            onboarding_steps: std::collections::HashMap::new(),
            onboarding_return_to_agent: None,
            pending_agent_save_retry: None,
            onboarding_poll_epoch: 0,
            onboarding_checking: false,
            images: std::collections::HashMap::new(),
            agents: initial_state.agents,
            telegram_manager_ready: false,
            loading: false,
            busy: None,
            error: startup_error.map(UiMessage::client),
            modal: None,
            modal_presented: false,
            language,
            language_select,
            theme_preference: ThemePreference::System,
            dark: initial_dark,
            hovered_agent_id: None,
            workspace_id_input,
            workspace_name_input,
            workspace_manager_token_input,
            workspace_provider: "feishu".into(),
            workspace_task_enabled: false,
            workspace_id_error: None,
            workspace_name_error: None,
            workspace_token_error: None,
            settings_name_input,
            settings_rules_input,
            settings_task_name_input,
            settings_task_enabled: false,
            agent_id_input,
            agent_name_input,
            agent_role_input,
            agent_bio_input,
            agent_soul_input,
            agent_environment_rows,
            agent_model_input,
            agent_cli_input,
            agent_provider_handle_input,
            agent_secret_input,
            agent_id_error: None,
            agent_name_error: None,
            agent_model_error: None,
            agent_provider_handle_error: None,
            agent_secret_error: None,
            local_computer_id: None,
            pairing_address: None,
            pairing_pin: None,
            pairing_address_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("192.168.1.20:7443")),
            pairing_editing: false,
            pairing_copied: false,
            pairing_connected: None,
            computers: Vec::new(),
            computer_request: 0,
            pairing_code: None,
            pairing_loading: false,
            pairing_request: 0,
            pairing_waiting_hint: false,
            pairing_hint_task: None,
            settings_computers_task: None,
            agent_computer,
            agent_harness,
            agent_thinking,
            agent_reaction: None,
            reaction_options: Vec::new(),
            reaction_loading: false,
            reaction_error: None,
            pending_agent_avatar: None,
            advanced_open: false,
            agent_mode_new: true,
            agent_provider_handle_edited: false,
        };
        if has_initial_state {
            app.reload_images(cx);
        } else {
            app.schedule_boot_loader(cx);
        }
        app
    }

    /// A warm Runtime answers within a few frames, so the loader only appears
    /// once startup is visibly slow and then stays long enough to read.
    fn schedule_boot_loader(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BOOT_LOADER_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.booting {
                    this.boot_loader_shown_at = Some(Instant::now());
                    cx.notify();
                }
            });
            cx.refresh();
        })
        .detach();
    }

    fn finish_boot(&mut self, cx: &mut Context<Self>) {
        if !self.booting {
            return;
        }
        let remaining = self
            .boot_loader_shown_at
            .map(|shown_at| BOOT_LOADER_MIN_VISIBLE.saturating_sub(shown_at.elapsed()))
            .unwrap_or_default();
        if remaining.is_zero() {
            self.booting = false;
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(remaining).await;
            let _ = this.update(cx, |this, cx| {
                this.booting = false;
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn theme(&self) -> DesktopTheme {
        if self.dark {
            DesktopTheme::dark()
        } else {
            DesktopTheme::light()
        }
    }

    fn notify_dialog_changed(&self, cx: &mut Context<Self>) {
        cx.notify();
    }

    fn tr(&self, zh: &'static str, en: &'static str) -> &'static str {
        self.language.text(zh, en)
    }

    fn set_language(
        &mut self,
        language: DesktopLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.language = language;
        let selected = self.agent_computer.read(cx).selected_value().cloned();
        self.update_computer_choices(selected, window, cx);
        for (input, zh, en) in [
            (
                &self.workspace_id_input,
                "以小写字母开头",
                "Start with a lowercase letter",
            ),
            (
                &self.agent_id_input,
                "以小写字母开头",
                "Start with a lowercase letter",
            ),
            (
                &self.workspace_name_input,
                "填写工作区名称",
                "Enter workspace name",
            ),
            (
                &self.workspace_manager_token_input,
                "填写 Manager Bot Token",
                "Enter Manager Bot Token",
            ),
            (&self.settings_task_name_input, "团队任务", "Team tasks"),
            (&self.agent_name_input, "填写显示名称", "Enter display name"),
            (
                &self.agent_role_input,
                "职位或职责",
                "Role or responsibility",
            ),
            (
                &self.agent_bio_input,
                "一句话介绍这位 Agent",
                "Describe this Agent in one sentence",
            ),
            (
                &self.agent_model_input,
                "选择或填写模型 ID",
                "Choose or enter model ID",
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(language.text(zh, en), window, cx)
            });
        }
        self.agent_soul_input.update(cx, |input, cx| {
            input.set_placeholder(
                language.text(
                    "描述它的工作方式、偏好和边界…",
                    "Describe how it works, its preferences and boundaries…",
                ),
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn set_theme(
        &mut self,
        preference: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.theme_preference = preference;
        self.dark = match preference {
            ThemePreference::System => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            ThemePreference::Light => false,
            ThemePreference::Dark => true,
        };
        ComponentTheme::change(
            if self.dark {
                ComponentThemeMode::Dark
            } else {
                ComponentThemeMode::Light
            },
            Some(window),
            cx,
        );
        apply_component_theme(self.theme(), cx);
        configure_window_material(window, self.dark);
        cx.notify();
    }
}

impl Render for LankideApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.started {
            self.started = true;
            self.reload_workspaces(cx);
        }
        self.sync_modal(window, cx);
        let theme = self.theme();
        if self.booting {
            return boot_loader(
                theme,
                self.tr("正在连接…", "Connecting…"),
                self.boot_loader_shown_at.is_some(),
            );
        }
        let content = match self.page {
            Page::Agents => self.agents_content(cx).into_any_element(),
            Page::Workspace => self.workspace_content(cx).into_any_element(),
            Page::Settings => self.settings_content(cx),
        };
        div()
            .relative()
            .size_full()
            .child(
                DesktopShell::two_pane(theme, self.sidebar(cx), content)
                    .render_dialog_layer(false)
                    .sidebar_widths(px(220.), px(190.), px(300.)),
            )
            .into_any_element()
    }
}

pub(super) fn set_input(
    input: &Entity<InputState>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<LankideApp>,
) {
    input.update(cx, |state, cx| {
        state.replace_all(value.to_string(), window, cx)
    });
}

pub(super) fn set_textarea(
    input: &Entity<TextareaState>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<LankideApp>,
) {
    input.update(cx, |state, cx| {
        state.replace_all(value.to_string(), window, cx)
    });
}

pub(super) fn input_value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

pub(super) fn textarea_value(input: &Entity<TextareaState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn harness_label(id: &str) -> String {
    match id {
        "codex" => "Codex".into(),
        "claude_code" => "Claude Code".into(),
        _ => "Pi".into(),
    }
}

fn model_label(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).to_string()
}
