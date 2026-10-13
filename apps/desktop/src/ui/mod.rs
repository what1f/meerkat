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

mod components;
mod dialogs;
mod pages;
mod reaction_picker;

use components::*;
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
                    Ok(Ok(())) => this.reload_workspaces(cx),
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

    fn selected_workspace(&self) -> Option<&Workspace> {
        let id = self.selected_workspace_id.as_deref()?;
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    fn provider_supports_capability(&self, provider_id: &str, capability: &str) -> bool {
        self.provider_capabilities
            .get(provider_id)
            .is_some_and(|capabilities| capabilities.iter().any(|item| item == capability))
    }

    fn select_workspace_provider(&mut self, provider_id: &str, cx: &mut Context<Self>) {
        self.workspace_provider = provider_id.to_owned();
        if !self.provider_supports_capability(provider_id, PROVIDER_CAPABILITY_TASK_SYSTEM) {
            self.workspace_task_enabled = false;
        }
        self.notify_dialog_changed(cx);
    }

    fn reload_workspaces(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let workspace_client = self.client.clone();
        let capability_client = self.client.clone();
        self.workspace_request += 1;
        let request = self.workspace_request;
        cx.spawn(async move |this, cx| {
            let (result, provider_capability_catalog) = tokio::join!(
                workspace_client.list_workspaces(),
                capability_client.get_provider_capability_catalog()
            );
            let _ = this.update(cx, |this, cx| {
                if this.workspace_request != request {
                    return;
                }
                if let Ok(catalog) = provider_capability_catalog {
                    this.provider_capabilities = catalog
                        .into_iter()
                        .map(|entry| (entry.provider_id, entry.capabilities))
                        .collect();
                }
                match result {
                    Ok(workspaces) => {
                        this.loading = false;
                        this.workspace_loaded = true;
                        this.workspaces = workspaces;
                        this.workspace_startup_retries = 0;
                        let selected = this
                            .selected_workspace_id
                            .clone()
                            .filter(|id| {
                                this.workspaces.iter().any(|workspace| &workspace.id == id)
                            })
                            .or_else(|| {
                                this.workspaces
                                    .first()
                                    .map(|workspace| workspace.id.clone())
                            });
                        if this.selected_workspace_id != selected {
                            this.selection_epoch += 1;
                            this.agents.clear();
                            this.images.clear();
                            this.reaction_options.clear();
                        }
                        this.selected_workspace_id = selected;
                        if this.selected_workspace_id.is_some() {
                            this.reload_agents(cx);
                        } else {
                            this.finish_boot(cx);
                        }
                    }
                    // Background services report their own outcome; until then a
                    // failed request only means the Runtime is still starting.
                    Err(_) if this.startup_pending || this.workspace_startup_retries < 50 => {
                        this.workspace_startup_retries =
                            this.workspace_startup_retries.saturating_add(1);
                        this.schedule_workspace_startup_reload(cx);
                    }
                    Err(error) => {
                        this.loading = false;
                        this.error = Some(UiMessage::client(error));
                        this.finish_boot(cx);
                    }
                };
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn schedule_workspace_startup_reload(&mut self, cx: &mut Context<Self>) {
        let request = self.workspace_request;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.workspace_request == request {
                    this.reload_workspaces(cx);
                }
            });
            cx.refresh();
        })
        .detach();
    }

    fn reload_agents(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.selected_workspace_id.clone() else {
            self.agents.clear();
            return;
        };
        self.loading = true;
        let client = self.client.clone();
        self.agent_request += 1;
        let request = self.agent_request;
        let epoch = self.selection_epoch;
        cx.spawn(async move |this, cx| {
            let result = client.list_agents(&workspace_id).await;
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.agent_request != request {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(agents) => {
                        this.agents = agents;
                        this.reload_images(cx);
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                this.finish_boot(cx);
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn reload_images(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace() else {
            return;
        };
        let workspace_id = workspace.id.clone();
        let avatars = self
            .agents
            .iter()
            .filter_map(|agent| {
                if let Avatar::Path(path) = &agent.avatar {
                    Some((path.clone(), agent.id.clone()))
                } else {
                    None
                }
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        self.images.retain(|path, _| avatars.contains_key(path));
        let missing = avatars
            .into_iter()
            .filter(|(path, _)| !self.images.contains_key(path))
            .collect::<Vec<_>>();
        self.image_request += 1;
        let request = self.image_request;
        let epoch = self.selection_epoch;
        if missing.is_empty() {
            return;
        }
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let mut loaded = Vec::new();
            let mut failed = false;
            for (path, agent_id) in missing {
                match client.get_agent_avatar(&workspace_id, &agent_id).await {
                    Ok(image) => {
                        if image.path != path {
                            failed = true;
                            continue;
                        }
                        let decoded = (|| {
                            let mut reader =
                                image::ImageReader::new(std::io::Cursor::new(image.bytes))
                                    .with_guessed_format()
                                    .ok()?;
                            let mut limits = image::Limits::default();
                            limits.max_image_width = Some(4096);
                            limits.max_image_height = Some(4096);
                            limits.max_alloc = Some(64 * 1024 * 1024);
                            reader.limits(limits);
                            let pixels = convert_rgba_to_bgra(
                                reader.decode().ok()?.thumbnail(1200, 1200).into_rgba8(),
                            );
                            Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
                        })();
                        if let Some(image) = decoded {
                            loaded.push((path, image));
                        } else {
                            failed = true;
                        }
                    }
                    Err(_) => failed = true,
                }
            }
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.image_request != request {
                    return;
                }
                this.images.extend(loaded);
                if failed && this.error.is_none() {
                    this.error = Some(UiMessage::localized(
                        "部分图片未能加载，请刷新重试。",
                        "Some images could not load. Refresh to try again.",
                    ));
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn agent_avatar(&self, agent: &Agent, size: f32) -> DesktopAvatar {
        let image = if let Avatar::Path(path) = &agent.avatar {
            self.images.get(path).cloned()
        } else {
            None
        };
        DesktopAvatar::new(agent.name.clone(), self.theme())
            .size(px(size))
            .when_some(image, |avatar, image| avatar.src(image))
    }

    fn reload_provider_status(&mut self, cx: &mut Context<Self>) {
        self.telegram_manager_ready = false;
        self.provider_request += 1;
        let request = self.provider_request;
        let epoch = self.selection_epoch;
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        if workspace.provider.id != "telegram" {
            return;
        }
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client
                .invoke_provider_interface(&workspace.id, "telegram.workspace.status", json!({}))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.provider_request != request {
                    return;
                }
                match result {
                    Ok(status) => {
                        this.telegram_manager_ready = status
                            .get("manager_bot_ready")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn update_thinking_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let levels: Vec<String> = match selected_harness_id(&self.agent_harness, cx).as_str() {
            "pi" => vec!["off", "minimal", "low", "medium", "high", "xhigh"],
            "claude_code" => vec!["off", "low", "medium", "high", "xhigh", "max"],
            "codex" => vec!["none", "minimal", "low", "medium", "high", "xhigh"],
            _ => vec![],
        }
        .into_iter()
        .map(str::to_owned)
        .collect();
        let current = selected_thinking_level(&self.agent_thinking, cx);
        let selected = if levels.contains(&current) {
            Some(current)
        } else {
            levels.first().cloned()
        };
        self.agent_thinking.update(cx, |select, cx| {
            select.set_items(
                levels
                    .iter()
                    .map(|level| ThinkingChoice::new(level))
                    .collect(),
                window,
                cx,
            );
            if let Some(selected) = selected {
                select.set_selected_value(&selected, window, cx);
            }
        });
    }

    fn load_reaction_catalog(&mut self, cx: &mut Context<Self>) {
        self.reaction_request += 1;
        let request = self.reaction_request;
        let epoch = self.selection_epoch;
        let workspace_id = self.selected_workspace_id.clone();
        let client = self.client.clone();
        self.reaction_loading = true;
        self.reaction_error = None;
        cx.spawn(async move |this, cx| {
            let result = client.get_reaction_catalog(workspace_id.as_deref()).await;
            let _ = this.update(cx, |this, cx| {
                if this.selection_epoch != epoch || this.reaction_request != request {
                    return;
                }
                this.reaction_loading = false;
                match result {
                    Ok(options) => {
                        this.reaction_options = options;
                        this.reaction_error = None;
                    }
                    Err(error) => {
                        this.reaction_options.clear();
                        this.reaction_error = Some(UiMessage::client(error));
                    }
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn select_workspace(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.selected_workspace_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.selection_epoch += 1;
        self.selected_workspace_id = Some(id);
        self.agents.clear();
        self.images.clear();
        self.reaction_options.clear();
        self.reaction_loading = false;
        self.reaction_error = None;
        self.modal = None;
        self.error = None;
        self.reload_agents(cx);
        cx.notify();
    }

    fn reset_workspace_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.workspace_id_input, "", window, cx);
        set_input(&self.workspace_name_input, "", window, cx);
        set_input(&self.workspace_manager_token_input, "", window, cx);
        self.workspace_provider = "feishu".into();
        self.workspace_task_enabled = false;
        self.workspace_id_error = None;
        self.workspace_name_error = None;
        self.workspace_token_error = None;
        self.error = None;
    }

    fn open_create_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.reset_workspace_form(window, cx);
        self.modal = Some(Modal::CreateWorkspace { step: 0 });
        cx.notify();
    }

    fn reset_agent_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input(&self.agent_id_input, "", window, cx);
        set_input(&self.agent_name_input, "", window, cx);
        set_input(&self.agent_role_input, "", window, cx);
        set_input(&self.agent_bio_input, "", window, cx);
        set_textarea(&self.agent_soul_input, "", window, cx);
        self.agent_environment_rows = vec![new_environment_row("", "", window, cx)];
        set_input(&self.agent_model_input, "", window, cx);
        set_input(&self.agent_cli_input, "", window, cx);
        set_input(&self.agent_provider_handle_input, "", window, cx);
        set_input(&self.agent_secret_input, "", window, cx);
        self.agent_harness.update(cx, |select, cx| {
            select.set_selected_value(&"pi".to_string(), window, cx)
        });
        self.agent_thinking.update(cx, |select, cx| {
            select.set_selected_value(&"high".to_string(), window, cx)
        });
        self.agent_reaction = None;
        self.advanced_open = false;
        self.agent_mode_new = true;
        self.agent_provider_handle_edited = false;
        self.agent_id_error = None;
        self.agent_name_error = None;
        self.agent_model_error = None;
        self.agent_provider_handle_error = None;
        self.agent_secret_error = None;
        self.error = None;
    }

    fn open_create_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.reset_agent_form(window, cx);
        self.reload_provider_status(cx);
        self.modal = Some(Modal::CreateAgent);
        self.update_thinking_choices(window, cx);
        cx.notify();
    }

    fn add_environment_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_environment_rows
            .push(new_environment_row("", "", window, cx));
        cx.notify();
    }

    fn remove_environment_variable(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.agent_environment_rows.len() {
            self.agent_environment_rows.remove(index);
            cx.notify();
        }
    }

    fn record_agent_onboarding_state(&mut self, state: &AgentOnboardingState) {
        let key = (state.workspace_id.clone(), state.agent_id.clone());
        if state.status == OnboardingStatus::Ready {
            self.onboarding_retries.remove(&key);
            self.onboarding_steps.remove(&key);
        } else {
            self.onboarding_retries.insert(key.clone());
            self.onboarding_steps.insert(key, state.step.clone());
        }
    }

    fn finish_agent_onboarding(
        &mut self,
        workspace_id: &str,
        agent_id: &str,
        cx: &mut Context<Self>,
    ) {
        let key = (workspace_id.to_owned(), agent_id.to_owned());
        let repaired_permissions = self
            .onboarding_steps
            .get(&key)
            .is_some_and(|step| step == "permissions");
        self.onboarding_retries.remove(&key);
        self.onboarding_steps.remove(&key);

        if repaired_permissions && self.pending_agent_save_retry.as_deref() == Some(agent_id) {
            self.pending_agent_save_retry = None;
            self.onboarding_return_to_agent = None;
            self.modal = Some(Modal::EditAgent(agent_id.to_owned()));
            self.submit_agent(Some(agent_id.to_owned()), cx);
        } else if repaired_permissions
            && self.onboarding_return_to_agent.as_deref() == Some(agent_id)
        {
            self.onboarding_return_to_agent = None;
            self.error = None;
            self.modal = Some(Modal::EditAgent(agent_id.to_owned()));
            self.notify_dialog_changed(cx);
        } else {
            self.onboarding_return_to_agent = None;
            self.modal = None;
            self.reload_agents(cx);
        }
    }

    fn open_edit_agent(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(agent) = self.agents.iter().find(|agent| agent.id == id).cloned() else {
            return;
        };
        set_input(&self.agent_id_input, &agent.id, window, cx);
        set_input(&self.agent_name_input, &agent.name, window, cx);
        set_input(&self.agent_role_input, &agent.role, window, cx);
        set_input(&self.agent_bio_input, &agent.bio, window, cx);
        set_textarea(&self.agent_soul_input, &agent.soul, window, cx);
        self.agent_environment_rows = if agent.environment_variables.is_empty() {
            vec![new_environment_row("", "", window, cx)]
        } else {
            agent
                .environment_variables
                .iter()
                .map(|(key, value)| new_environment_row(key, value, window, cx))
                .collect()
        };
        set_input(&self.agent_model_input, &agent.model, window, cx);
        set_input(&self.agent_cli_input, &agent.cli_path, window, cx);
        set_input(
            &self.agent_provider_handle_input,
            &agent.provider_handle,
            window,
            cx,
        );
        self.agent_harness.update(cx, |select, cx| {
            select.set_selected_value(&agent.harness_id, window, cx)
        });
        self.agent_thinking.update(cx, |select, cx| {
            select.set_selected_value(&agent.thinking_level.label().to_string(), window, cx)
        });
        self.agent_reaction = agent.auto_reaction.clone();
        self.pending_agent_avatar = None;
        self.agent_provider_handle_edited = true;
        self.agent_id_error = None;
        self.agent_name_error = None;
        self.agent_model_error = None;
        self.agent_provider_handle_error = None;
        self.agent_secret_error = None;
        self.load_reaction_catalog(cx);
        self.advanced_open = false;
        self.error = None;
        self.modal = Some(Modal::EditAgent(id));
        let client = self.client.clone();
        let workspace_id = agent.workspace_id.clone();
        let agent_id = agent.id.clone();
        cx.spawn(async move |this, cx| {
            let state = client.get_agent_onboarding(&workspace_id, &agent_id).await;
            let _ = this.update(cx, |this, cx| {
                match state {
                    Ok(state) => this.record_agent_onboarding_state(&state),
                    Err(_) => {}
                }
                if matches!(&this.modal, Some(Modal::EditAgent(id)) if id == &agent_id) {
                    this.notify_dialog_changed(cx);
                }
            });
            cx.refresh();
        })
        .detach();
        cx.notify();
    }

    fn open_delete_agent(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() || !self.agents.iter().any(|agent| agent.id == id) {
            return;
        }
        self.error = None;
        self.modal = Some(Modal::DeleteAgent(id));
        cx.notify();
    }

    fn delete_agent(&mut self, id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace_id) = self.selected_workspace_id.clone() else {
            return;
        };
        self.busy = Some(BusyOperation::DeleteAgent);
        self.error = None;
        self.notify_dialog_changed(cx);
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let result = client.delete_agent(&workspace_id, &id).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(()) => {
                        // Invalidate any list response that was started before the
                        // deletion completed. Otherwise a stale response can restore
                        // the deleted Agent in the cards after we remove it locally.
                        this.agent_request += 1;
                        this.agents.retain(|agent| agent.id != id);
                        this.modal = None;
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn choose_agent_avatar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.language.text("选择", "Choose").into()),
        });
        let app = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let Some(path) = selection
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next())
            else {
                return;
            };
            let result = (|| {
                let bytes = std::fs::read(&path).map_err(|_| ())?;
                if bytes.is_empty() || bytes.len() > MAX_AGENT_AVATAR_BYTES {
                    return Err(());
                }
                let format = image::guess_format(&bytes).map_err(|_| ())?;
                let media_type = match format {
                    image::ImageFormat::Png => "image/png",
                    image::ImageFormat::Jpeg => "image/jpeg",
                    image::ImageFormat::WebP => "image/webp",
                    image::ImageFormat::Gif => "image/gif",
                    _ => return Err(()),
                };
                let pixels = convert_rgba_to_bgra(
                    image::load_from_memory_with_format(&bytes, format)
                        .map_err(|_| ())?
                        .to_rgba8(),
                );
                Ok(PendingAgentAvatar {
                    media_type: media_type.into(),
                    bytes,
                    image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
                })
            })();
            let _ = window.update(|_, cx| {
                let _ = app.update(cx, |this, cx| {
                    match result {
                        Ok(avatar) => {
                            this.pending_agent_avatar = Some(avatar);
                            this.error = None;
                        }
                        Err(()) => {
                            this.error = Some(UiMessage::localized(
                                "请选择不超过 3 MB 的 PNG、JPEG、WebP 或 GIF 图片。",
                                "Choose a PNG, JPEG, WebP, or GIF image up to 3 MB.",
                            ));
                        }
                    }
                    this.notify_dialog_changed(cx);
                });
            });
        })
        .detach();
    }

    fn open_workspace_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        set_input(&self.settings_name_input, &workspace.name, window, cx);
        set_textarea(&self.settings_rules_input, &workspace.rules, window, cx);
        set_input(
            &self.settings_task_name_input,
            workspace
                .provider_config
                .get("task_list_name")
                .and_then(Value::as_str)
                .unwrap_or(self.language.text("团队任务", "Team tasks")),
            window,
            cx,
        );
        self.settings_task_enabled = workspace.task_system_enabled;
        self.page = Page::Workspace;
        self.error = None;
        cx.notify();
    }

    fn submit_workspace(&mut self, window_handle: AnyWindowHandle, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let id = input_value(&self.workspace_id_input, cx);
        let name = input_value(&self.workspace_name_input, cx);
        self.workspace_id_error = if id.trim().is_empty() {
            Some(UiMessage::localized(
                "请填写 Workspace ID。",
                "Enter a Workspace ID.",
            ))
        } else if !valid_id(&id) {
            Some(UiMessage::localized(
                "只能使用小写字母、数字和下划线。",
                "Use lowercase letters, digits and underscores only.",
            ))
        } else {
            None
        };
        self.workspace_name_error = name
            .trim()
            .is_empty()
            .then(|| UiMessage::localized("请填写工作区名称。", "Enter a workspace name."));
        if self.workspace_id_error.is_some() || self.workspace_name_error.is_some() {
            self.error = None;
            cx.notify();
            return;
        }
        let provider_id = self.workspace_provider.clone();
        let manager_bot_token = input_value(&self.workspace_manager_token_input, cx);
        if provider_id == "telegram" && manager_bot_token.trim().is_empty() {
            self.workspace_token_error = Some(UiMessage::localized(
                "请填写 Manager Bot Token。",
                "Enter a Manager Bot Token.",
            ));
            cx.notify();
            return;
        }
        self.workspace_token_error = None;
        let task_enabled = self.workspace_task_enabled
            && self.provider_supports_capability(&provider_id, PROVIDER_CAPABILITY_TASK_SYSTEM);
        let provider_secrets = if provider_id == "telegram" {
            json!({"manager_bot_token": manager_bot_token})
        } else {
            json!({})
        };
        let draft = WorkspaceDraft {
            id,
            name: name.trim().into(),
            rules: String::new(),
            provider_id,
            provider_config: json!({"task_system_enabled": task_enabled}),
            provider_secrets,
        };
        let client = self.client.clone();
        self.busy = Some(BusyOperation::CreateWorkspace);
        self.error = None;
        self.notify_dialog_changed(cx);
        cx.spawn(async move |this, cx| {
            let created = client.create_workspace(draft).await;
            let notification = this.update(cx, |this, cx| {
                this.busy = None;
                let notification = match created {
                    Ok(workspace) => {
                        this.error = None;
                        this.selection_epoch += 1;
                        this.selected_workspace_id = Some(workspace.id.clone());
                        this.reaction_options.clear();
                        this.telegram_manager_ready = false;
                        this.workspaces.push(workspace);
                        this.agents.clear();
                        this.images.clear();
                        this.modal = None;
                        Some((
                            NotificationType::Success,
                            UiMessage::localized("工作区已创建。", "Workspace created."),
                        ))
                    }
                    Err(ClientError::DuplicateWorkspaceId(_)) => {
                        this.workspace_id_error = Some(UiMessage::localized(
                            "这个 Workspace ID 已经被使用。",
                            "This Workspace ID is already in use.",
                        ));
                        this.error = None;
                        None
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        None
                    }
                };
                cx.notify();
                notification.map(|(notification_type, message)| {
                    (notification_type, message.text(this.language))
                })
            });
            if let Ok(Some((notification_type, message))) = notification {
                push_notification(cx, window_handle, notification_type, message);
            }
            cx.refresh();
        })
        .detach();
    }

    fn submit_agent(&mut self, editing: Option<String>, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        let workspace_id = workspace.id.clone();
        let id = input_value(&self.agent_id_input, cx);
        let name = input_value(&self.agent_name_input, cx);
        let model = input_value(&self.agent_model_input, cx);
        let harness_id = selected_harness_id(&self.agent_harness, cx);
        let provider_handle = input_value(&self.agent_provider_handle_input, cx);
        let provider_secret = input_value(&self.agent_secret_input, cx);
        self.agent_id_error = if id.trim().is_empty() {
            Some(UiMessage::localized(
                "请填写 Agent ID。",
                "Enter an Agent ID.",
            ))
        } else if !valid_id(&id) {
            Some(UiMessage::localized(
                "只能使用小写字母、数字和下划线。",
                "Use lowercase letters, digits and underscores only.",
            ))
        } else {
            None
        };
        self.agent_name_error = name
            .trim()
            .is_empty()
            .then(|| UiMessage::localized("请填写 Agent 名称。", "Enter an Agent name."));
        self.agent_model_error = if model.trim().is_empty() {
            Some(UiMessage::localized(
                "请填写模型名称。",
                "Enter a model name.",
            ))
        } else if harness_id == "pi"
            && model.trim().starts_with("gpt-")
            && !model.trim().contains('/')
        {
            Some(UiMessage::localized(
                format!("请填写完整模型 ID，例如 openai-codex/{}。", model.trim()),
                format!(
                    "Enter the complete model ID, for example openai-codex/{}.",
                    model.trim()
                ),
            ))
        } else {
            None
        };
        self.agent_provider_handle_error = None;
        self.agent_secret_error = None;
        if editing.is_none() && !self.agent_mode_new {
            if workspace.provider.id == "feishu" && provider_handle.trim().is_empty() {
                self.agent_provider_handle_error =
                    Some(UiMessage::localized("请填写 App ID。", "Enter an App ID."));
            }
            if provider_secret.trim().is_empty() {
                self.agent_secret_error = Some(if workspace.provider.id == "telegram" {
                    UiMessage::localized("请填写 Bot Token。", "Enter a Bot Token.")
                } else {
                    UiMessage::localized("请填写 App Secret。", "Enter an App Secret.")
                });
            }
        }
        if self.agent_id_error.is_some()
            || self.agent_name_error.is_some()
            || self.agent_model_error.is_some()
            || self.agent_provider_handle_error.is_some()
            || self.agent_secret_error.is_some()
        {
            self.error = None;
            cx.notify();
            return;
        }
        let thinking_level =
            match ThinkingLevel::parse(&selected_thinking_level(&self.agent_thinking, cx)) {
                Ok(level) => level,
                Err(error) => {
                    self.error = Some(UiMessage::client(error));
                    cx.notify();
                    return;
                }
            };
        let auto_reaction = self.agent_reaction.clone();
        let role = input_value(&self.agent_role_input, cx);
        let bio = input_value(&self.agent_bio_input, cx);
        let soul = textarea_value(&self.agent_soul_input, cx);
        let cli_path = input_value(&self.agent_cli_input, cx);
        let environment_variables =
            match collect_environment_variables(&self.agent_environment_rows, cx) {
                Ok(values) => values,
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            };
        let client = self.client.clone();
        self.busy = Some(BusyOperation::SaveAgent);
        self.error = None;
        self.notify_dialog_changed(cx);

        if let Some(editing_id) = editing {
            let Some(mut agent) = self
                .agents
                .iter()
                .find(|agent| agent.id == editing_id)
                .cloned()
            else {
                return;
            };
            agent.name = name.trim().into();
            agent.role = role;
            agent.bio = bio;
            agent.soul = soul;
            agent.harness_id = harness_id;
            agent.model = model;
            agent.thinking_level = thinking_level;
            agent.auto_reaction = auto_reaction;
            agent.cli_path = cli_path;
            agent.environment_variables = environment_variables;
            agent.provider_handle = provider_handle;
            set_provider_handle(
                &mut agent.provider_agent_config,
                &workspace.provider.id,
                &agent.provider_handle,
            );
            let pending_avatar = self.pending_agent_avatar.clone();
            let workspace_id = agent.workspace_id.clone();
            let retry_agent_id = editing_id.clone();
            let check_feishu_permissions = workspace.provider.id == "feishu";
            cx.spawn(async move |this, cx| {
                let result = async {
                    // Provider identity is established by onboarding and is not an
                    // editable profile field. Refresh it before saving so a stale
                    // dialog cannot erase a connection while changing the model.
                    let current = client.get_agent(&workspace_id, &editing_id).await?;
                    agent.provider_handle = current.provider_handle;
                    agent.provider_agent_config = current.provider_agent_config;
                    let (upload, uploaded) = if let Some(avatar) = pending_avatar {
                        (
                            Some(AgentAvatarUpload {
                                media_type: avatar.media_type,
                                bytes: avatar.bytes,
                            }),
                            Some(avatar.image),
                        )
                    } else {
                        (None, None)
                    };
                    client
                        .update_agent(agent, upload)
                        .await
                        .map(|agent| (agent, uploaded))
                }
                .await;
                let onboarding = if check_feishu_permissions
                    && matches!(&result, Err(ClientError::Provider(_)))
                {
                    client
                        .get_agent_onboarding(&workspace_id, &retry_agent_id)
                        .await
                        .ok()
                } else {
                    None
                };
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok((updated, uploaded)) => {
                            let updated_id = updated.id.clone();
                            if let Some(image) = uploaded
                                && let Avatar::Path(path) = &updated.avatar
                            {
                                this.images.insert(path.clone(), image);
                            }
                            if let Some(current) =
                                this.agents.iter_mut().find(|a| a.id == updated.id)
                            {
                                *current = updated;
                            }
                            this.pending_agent_avatar = None;
                            if this.pending_agent_save_retry.as_deref() == Some(updated_id.as_str()) {
                                this.pending_agent_save_retry = None;
                            }
                            this.modal = None;
                        }
                        Err(error) => {
                            if let Some(state) = onboarding.filter(|state| {
                                state.step == "permissions"
                                    && state.status != OnboardingStatus::Ready
                            }) {
                                this.record_agent_onboarding_state(&state);
                                this.pending_agent_save_retry = Some(retry_agent_id);
                                this.error = Some(UiMessage::localized(
                                    "需要先更新飞书权限，完成后将自动重试保存。",
                                    "Update Feishu permissions first. Saving will retry automatically when it finishes.",
                                ));
                            } else {
                                this.error = Some(UiMessage::client(error));
                            }
                        }
                    }
                    cx.notify();
                });
                cx.refresh();
            })
            .detach();
        } else {
            let avatar = Avatar::Custom(name.clone());
            let provider_id = workspace.provider.id.as_str();
            let mut provider_handle = provider_handle;
            if self.agent_mode_new && provider_id == "telegram" && provider_handle.trim().is_empty()
            {
                provider_handle = telegram_bot_username(&workspace.id, &id);
            }
            let mut provider_agent_config = json!({
                "mode": if self.agent_mode_new { "new" } else { "existing" },
            });
            set_provider_handle(&mut provider_agent_config, provider_id, &provider_handle);
            let provider_agent_secrets = if self.agent_mode_new {
                json!({})
            } else if provider_id == "telegram" {
                json!({"bot_token": provider_secret})
            } else {
                json!({"app_secret": provider_secret})
            };
            let draft = AgentDraft {
                id,
                name: name.trim().into(),
                role,
                bio,
                avatar,
                soul,
                computer_id: "local".into(),
                harness_id,
                model,
                thinking_level,
                auto_reaction,
                cli_path,
                environment_variables,
                provider_handle,
                provider_agent_config,
                provider_agent_secrets,
            };
            let mode_new = self.agent_mode_new;
            cx.spawn(async move |this, cx| {
                let result = client.create_agent(&workspace_id, draft).await;
                let onboarding = match &result {
                    Ok(agent) if mode_new => Some(client
                        .advance_agent_onboarding(
                            &agent.workspace_id,
                            &agent.id,
                            "start",
                            json!({}),
                        )
                        .await),
                    Ok(agent) => Some(client
                        .get_agent_onboarding(&agent.workspace_id, &agent.id)
                        .await),
                    Err(_) => None,
                };
                let _ = this.update(cx, |this, cx| {
                    this.busy = None;
                    match result {
                        Ok(created) => {
                            let retry_key = (created.workspace_id.clone(), created.id.clone());
                            this.agents.push(created);
                            match onboarding {
                                Some(Ok(state)) => {
                                    this.record_agent_onboarding_state(&state);
                                    if state.status != OnboardingStatus::Ready {
                                        this.modal = Some(Modal::AgentOnboarding(state.clone()));
                                        this.schedule_agent_onboarding_poll(state, cx);
                                    } else {
                                        this.modal = None;
                                    }
                                }
                                Some(Err(error)) => {
                                    this.onboarding_retries.insert(retry_key);
                                    this.modal = None;
                                    this.error = Some(UiMessage::localized(
                                        format!(
                                            "Agent 已创建，但尚未连接。请在 Agent 编辑页完成连接。 {}",
                                            friendly_error(&error, DesktopLanguage::SimplifiedChinese)
                                        ),
                                        format!(
                                            "Agent created, but it is not connected yet. Open the Agent editor to connect it. {}",
                                            friendly_error(&error, DesktopLanguage::English)
                                        ),
                                    ));
                                }
                                None => unreachable!("successful creation always checks onboarding"),
                            }
                        }
                        Err(ClientError::DuplicateAgentId(_)) => {
                            this.agent_id_error = Some(UiMessage::localized(
                                "这个 Agent ID 已经被使用。",
                                "This Agent ID is already in use.",
                            ));
                            this.error = None;
                        }
                        Err(ClientError::InvalidAgentId) => {
                            this.agent_id_error = Some(UiMessage::localized(
                                "只能使用小写字母、数字和下划线。",
                                "Use lowercase letters, digits and underscores only.",
                            ));
                            this.error = None;
                        }
                        Err(error) => this.error = Some(UiMessage::client(error)),
                    }
                    cx.notify();
                });
                cx.refresh();
            })
            .detach();
        }
    }

    fn save_workspace_settings(&mut self, window_handle: AnyWindowHandle, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(mut workspace) = self.selected_workspace().cloned() else {
            return;
        };
        let name = input_value(&self.settings_name_input, cx);
        if name.trim().is_empty() {
            self.error = Some(UiMessage::localized(
                "请填写工作区名称。",
                "Enter a workspace name.",
            ));
            cx.notify();
            return;
        }
        workspace.name = name.trim().into();
        workspace.rules = textarea_value(&self.settings_rules_input, cx);
        let task_system_enabled = self.settings_task_enabled
            && self.provider_supports_capability(
                &workspace.provider.id,
                PROVIDER_CAPABILITY_TASK_SYSTEM,
            );
        workspace.task_system_enabled = task_system_enabled;
        workspace.provider_config["task_system_enabled"] = json!(task_system_enabled);
        workspace.provider_config["task_list_name"] =
            json!(input_value(&self.settings_task_name_input, cx).trim());
        let client = self.client.clone();
        self.busy = Some(BusyOperation::SaveWorkspace);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.update_workspace(workspace).await;
            let notification = this.update(cx, |this, cx| {
                this.busy = None;
                let notification = match result {
                    Ok(updated) => {
                        if let Some(current) =
                            this.workspaces.iter_mut().find(|w| w.id == updated.id)
                        {
                            *current = updated;
                        }
                        Some((
                            NotificationType::Success,
                            UiMessage::localized("工作区设置已保存。", "Workspace settings saved."),
                        ))
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        None
                    }
                };
                cx.notify();
                notification.map(|(notification_type, message)| {
                    (notification_type, message.text(this.language))
                })
            });
            if let Ok(Some((notification_type, message))) = notification {
                push_notification(cx, window_handle, notification_type, message);
            }
            cx.refresh();
        })
        .detach();
    }

    fn toggle_runtime_state(&mut self, window_handle: AnyWindowHandle, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace) = self.selected_workspace().cloned() else {
            return;
        };
        let next = if workspace.status == WorkspaceStatus::Active {
            WorkspaceStatus::Paused
        } else {
            WorkspaceStatus::Active
        };
        let id = workspace.id;
        let client = self.client.clone();
        self.busy = Some(BusyOperation::ToggleRuntime);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = client.set_workspace_state(&id, next).await;
            let notification = this.update(cx, |this, cx| {
                this.busy = None;
                let notification = match result {
                    Ok(updated) => {
                        let observed = updated.observed_state;
                        if let Some(current) =
                            this.workspaces.iter_mut().find(|w| w.id == updated.id)
                        {
                            *current = updated;
                        }
                        use lankide_client::WorkspaceObservedState;
                        this.error = None;
                        match observed {
                            WorkspaceObservedState::Running if next == WorkspaceStatus::Active => {
                                Some((
                                    NotificationType::Success,
                                    UiMessage::localized("已启用。", "Enabled."),
                                ))
                            }
                            WorkspaceObservedState::Paused => Some((
                                NotificationType::Success,
                                UiMessage::localized("已暂停。", "Paused."),
                            )),
                            WorkspaceObservedState::Error => {
                                this.error = Some(UiMessage::localized(
                                    "需要处理，请检查设置后重试。",
                                    "Needs attention. Check the settings and retry.",
                                ));
                                None
                            }
                            _ => Some((
                                NotificationType::Info,
                                UiMessage::localized(
                                    "正在更新运行状态，请稍候。",
                                    "Updating runtime status. Please wait.",
                                ),
                            )),
                        }
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        None
                    }
                };
                cx.notify();
                notification.map(|(notification_type, message)| {
                    (notification_type, message.text(this.language))
                })
            });
            if let Ok(Some((notification_type, message))) = notification {
                push_notification(cx, window_handle, notification_type, message);
            }
            cx.refresh();
        })
        .detach();
    }

    fn advance_agent_onboarding(&mut self, state: AgentOnboardingState, cx: &mut Context<Self>) {
        if self.onboarding_checking {
            return;
        }
        let client = self.client.clone();
        self.onboarding_checking = true;
        self.error = None;
        self.notify_dialog_changed(cx);
        self.onboarding_poll_epoch = self.onboarding_poll_epoch.wrapping_add(1);
        let request_epoch = self.onboarding_poll_epoch;
        let retry_state = state.clone();
        let (action, payload) = next_agent_onboarding_request(&state);
        let workspace_id = state.workspace_id;
        let agent_id = state.agent_id;
        cx.spawn(async move |this, cx| {
            let result = client
                .advance_agent_onboarding(&workspace_id, &agent_id, action, payload)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.onboarding_poll_epoch != request_epoch {
                    return;
                }
                this.onboarding_checking = false;
                match result {
                    Ok(next) if next.status == OnboardingStatus::Ready => {
                        this.finish_agent_onboarding(&workspace_id, &agent_id, cx);
                    }
                    Ok(next) => {
                        this.record_agent_onboarding_state(&next);
                        this.modal = Some(Modal::AgentOnboarding(next.clone()));
                        this.schedule_agent_onboarding_poll(next, cx);
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        this.schedule_agent_onboarding_poll(retry_state, cx);
                    }
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn resume_agent_onboarding(&mut self, agent_id: String, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(workspace_id) = self.selected_workspace_id.clone() else {
            return;
        };
        self.onboarding_return_to_agent = Some(agent_id.clone());
        let client = self.client.clone();
        self.busy = Some(BusyOperation::ResumeAgentOnboarding);
        self.error = None;
        self.notify_dialog_changed(cx);
        cx.spawn(async move |this, cx| {
            let current = client.get_agent_onboarding(&workspace_id, &agent_id).await;
            let result = match current {
                Ok(state)
                    if state
                        .payload
                        .get("verification_uri")
                        .and_then(serde_json::Value::as_str)
                        .is_none()
                        && state.status != OnboardingStatus::Ready =>
                {
                    client
                        .advance_agent_onboarding(&workspace_id, &agent_id, "start", json!({}))
                        .await
                }
                other => other,
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(state) if state.status == OnboardingStatus::Ready => {
                        this.finish_agent_onboarding(&workspace_id, &agent_id, cx);
                    }
                    Ok(state) => {
                        this.record_agent_onboarding_state(&state);
                        this.modal = Some(Modal::AgentOnboarding(state.clone()));
                        this.schedule_agent_onboarding_poll(state, cx);
                    }
                    Err(error) => this.error = Some(UiMessage::client(error)),
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }

    fn schedule_agent_onboarding_poll(
        &mut self,
        state: AgentOnboardingState,
        cx: &mut Context<Self>,
    ) {
        let Some(session_key) = onboarding_session_key(&state.payload) else {
            return;
        };
        if matches!(
            state.status,
            OnboardingStatus::Ready | OnboardingStatus::Error
        ) {
            return;
        }
        let interval = state
            .payload
            .get("interval_seconds")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 30);
        let workspace_id = state.workspace_id.clone();
        let agent_id = state.agent_id.clone();
        let payload = state.payload.clone();
        let client = self.client.clone();
        self.onboarding_poll_epoch = self.onboarding_poll_epoch.wrapping_add(1);
        let poll_epoch = self.onboarding_poll_epoch;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(interval))
                .await;
            let should_poll = this
                .update(cx, |this, _| {
                    this.onboarding_poll_epoch == poll_epoch
                        && this.busy.is_none()
                        && !this.onboarding_checking
                        && matches!(
                            &this.modal,
                            Some(Modal::AgentOnboarding(current))
                                if current.workspace_id == workspace_id
                                    && current.agent_id == agent_id
                                    && onboarding_session_key(&current.payload).as_deref()
                                        == Some(session_key.as_str())
                        )
                })
                .unwrap_or(false);
            if !should_poll {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.onboarding_checking = true;
                this.error = None;
                cx.notify();
            });
            let result = client
                .advance_agent_onboarding(&workspace_id, &agent_id, "poll", payload)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.onboarding_poll_epoch != poll_epoch {
                    return;
                }
                this.onboarding_checking = false;
                match result {
                    Ok(next) if next.status == OnboardingStatus::Ready => {
                        this.finish_agent_onboarding(&workspace_id, &agent_id, cx);
                    }
                    Ok(next) => {
                        this.record_agent_onboarding_state(&next);
                        this.modal = Some(Modal::AgentOnboarding(next.clone()));
                        this.schedule_agent_onboarding_poll(next, cx);
                    }
                    Err(error) => {
                        this.error = Some(UiMessage::client(error));
                        this.schedule_agent_onboarding_poll(state, cx);
                    }
                }
                cx.notify();
            });
            cx.refresh();
        })
        .detach();
    }
}

pub(super) fn next_agent_onboarding_request(state: &AgentOnboardingState) -> (&'static str, Value) {
    let can_poll = onboarding_session_key(&state.payload).is_some();
    if can_poll {
        ("poll", state.payload.clone())
    } else {
        ("start", json!({}))
    }
}

fn onboarding_session_key(payload: &Value) -> Option<String> {
    payload
        .get("device_code")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("registration:{value}"))
        .or_else(|| {
            (payload.get("phase").and_then(Value::as_str) == Some("permissions"))
                .then(|| payload.get("app_id").and_then(Value::as_str))
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("permissions:{value}"))
        })
        .or_else(|| {
            payload
                .get("bot_username")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("telegram:{value}"))
        })
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

fn set_input(
    input: &Entity<InputState>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<LankideApp>,
) {
    input.update(cx, |state, cx| {
        state.replace_all(value.to_string(), window, cx)
    });
}

fn set_textarea(
    input: &Entity<TextareaState>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<LankideApp>,
) {
    input.update(cx, |state, cx| {
        state.replace_all(value.to_string(), window, cx)
    });
}

fn input_value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn new_environment_row(
    key: &str,
    value: &str,
    window: &mut Window,
    cx: &mut Context<LankideApp>,
) -> AgentEnvironmentRow {
    AgentEnvironmentRow {
        key: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("KEY")
                .default_value(key.to_string())
        }),
        value: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("VALUE")
                .default_value(value.to_string())
        }),
    }
}

fn textarea_value(input: &Entity<TextareaState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn selected_harness_id(select: &Entity<SelectState<Vec<HarnessChoice>>>, cx: &App) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| "pi".into())
}

fn selected_thinking_level(select: &Entity<SelectState<Vec<ThinkingChoice>>>, cx: &App) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| "high".into())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn set_provider_handle(config: &mut Value, provider_id: &str, handle: &str) {
    let Value::Object(values) = config else {
        *config = json!({});
        return set_provider_handle(config, provider_id, handle);
    };
    values.insert("provider_handle".into(), Value::String(handle.into()));
    let provider_key = match provider_id {
        "telegram" => "bot_username",
        "feishu" => "app_id",
        _ => return,
    };
    values.insert(provider_key.into(), Value::String(handle.into()));
}

fn telegram_bot_username(workspace_id: &str, agent_id: &str) -> String {
    fn normalize(value: &str) -> String {
        value
            .to_ascii_lowercase()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_owned()
    }
    let workspace = normalize(workspace_id);
    let workspace = workspace
        .strip_prefix("telegram_")
        .map(|value| format!("tg_{value}"))
        .unwrap_or(workspace);
    let agent = normalize(agent_id);
    let agent = &agent[..agent.len().min(14)];
    let workspace_limit = 27usize.saturating_sub(agent.len()).max(1);
    let workspace = &workspace[..workspace.len().min(workspace_limit)];
    format!("{workspace}_{agent}_bot")
}

fn collect_environment_variables(
    rows: &[AgentEnvironmentRow],
    cx: &App,
) -> Result<Vec<(String, String)>, UiMessage> {
    let mut values = Vec::new();
    for row in rows {
        let key = input_value(&row.key, cx);
        let value = input_value(&row.value, cx);
        let key = key.trim();
        if key.is_empty() && value.trim().is_empty() {
            continue;
        }
        if key.is_empty() {
            return Err(UiMessage::localized(
                "请填写环境变量名称。",
                "Enter an environment variable name.",
            ));
        }
        if key.chars().any(char::is_whitespace) || key.contains('=') {
            return Err(UiMessage::localized(
                "环境变量名称不能包含空格或等号。",
                "Environment variable names cannot contain spaces or equals signs.",
            ));
        }
        values.push((key.to_string(), value));
    }
    Ok(values)
}

fn friendly_error(error: &ClientError, language: DesktopLanguage) -> String {
    match error {
        ClientError::DuplicateWorkspaceId(_) => language
            .text(
                "这个 Workspace ID 已经被使用。",
                "This Workspace ID is already in use.",
            )
            .into(),
        ClientError::DuplicateAgentId(_) => language
            .text(
                "这个 Agent ID 已经被使用。",
                "This Agent ID is already in use.",
            )
            .into(),
        ClientError::InvalidAgentId => language
            .text(
                "Agent ID 只能使用小写字母、数字和下划线。",
                "Agent IDs may contain lowercase letters, digits and underscores only.",
            )
            .into(),
        ClientError::WorkspaceNotFound(_) => language
            .text(
                "找不到这个工作区，请刷新后重试。",
                "Workspace not found. Refresh to try again.",
            )
            .into(),
        ClientError::AgentNotFound(_) => language
            .text(
                "找不到这位 Agent，请刷新后重试。",
                "Agent not found. Refresh to try again.",
            )
            .into(),
        ClientError::Validation(message) => format!(
            "{} {message}",
            language.text("请检查填写内容：", "Check the entered values:")
        ),
        ClientError::Provider(_) => language
            .text(
                "暂时无法完成这个操作，请稍后重试。",
                "The chat service could not complete this action. Try again later.",
            )
            .into(),
        ClientError::Unavailable(_) | ClientError::Transport(_) | ClientError::StreamEnded => {
            language
                .text(
                    "暂时无法连接 Lankide，请稍后重试。",
                    "Cannot connect to Lankide. Try again shortly.",
                )
                .into()
        }
        ClientError::Protocol(_) => language
            .text(
                "暂时无法完成操作，请稍后重试。",
                "Could not complete this action. Try again shortly.",
            )
            .into(),
    }
}

pub(super) fn localized_agent_onboarding_copy(
    state: &AgentOnboardingState,
    language: DesktopLanguage,
) -> (String, String) {
    let is_permission_update =
        state.payload.get("flow").and_then(Value::as_str) == Some("authorization_update");
    if is_permission_update {
        return match state.status {
            OnboardingStatus::Ready => (
                language
                    .text("飞书权限已更新", "Feishu permissions updated")
                    .into(),
                language
                    .text("现在可以继续。", "You can continue now.")
                    .into(),
            ),
            OnboardingStatus::Error => (
                language
                    .text("未能更新飞书权限", "Could not update Feishu permissions")
                    .into(),
                language.text("请稍后重试。", "Try again shortly.").into(),
            ),
            OnboardingStatus::ActionRequired
                if state
                    .payload
                    .get("verification_uri")
                    .and_then(Value::as_str)
                    .is_some() =>
            {
                (
                    language
                        .text("在飞书中确认权限更新", "Confirm the update in Feishu")
                        .into(),
                    language
                        .text(
                            "打开下方链接完成确认。",
                            "Open the link below to confirm the update.",
                        )
                        .into(),
                )
            }
            OnboardingStatus::ActionRequired => (
                language
                    .text("更新飞书权限", "Update Feishu permissions")
                    .into(),
                language
                    .text(
                        "更新后即可继续使用完整功能。",
                        "Update permissions to continue using all features.",
                    )
                    .into(),
            ),
            OnboardingStatus::Verifying => (
                language
                    .text("正在更新飞书权限", "Updating Feishu permissions")
                    .into(),
                language
                    .text("完成后将自动继续。", "You can continue when it finishes.")
                    .into(),
            ),
        };
    }
    let is_waiting_for_confirmation = onboarding_session_key(&state.payload).is_some()
        && !matches!(
            state.status,
            OnboardingStatus::Ready | OnboardingStatus::Error
        );
    if is_waiting_for_confirmation {
        if state.provider_id == "feishu" && state.step == "permissions" {
            return (
                language
                    .text(
                        "应用已创建，继续授权",
                        "App created — continue authorization",
                    )
                    .to_owned(),
                language
                    .text(
                        "还需完成 Agent 能力授权，完成后将自动启用。",
                        "Authorize Agent capabilities to finish setup.",
                    )
                    .to_owned(),
            );
        }
        if state.provider_id == "telegram" {
            return (
                language
                    .text("在 Telegram 中确认", "Confirm in Telegram")
                    .to_owned(),
                language
                    .text(
                        "打开下方链接完成 Bot 创建。",
                        "Open the link below to create the Bot.",
                    )
                    .to_owned(),
            );
        }
        return (
            language
                .text("等待确认", "Waiting for confirmation")
                .to_owned(),
            language
                .text(
                    "打开下方链接完成设置",
                    "Open the link below to finish setup",
                )
                .to_owned(),
        );
    }
    let has_link = state
        .payload
        .get("verification_uri")
        .and_then(Value::as_str)
        .is_some();
    let (title_zh, title_en, instruction_zh, instruction_en) = match state.status {
        OnboardingStatus::Ready => (
            "Agent 已启用",
            "Agent enabled",
            "连接已完成。",
            "Connection completed.",
        ),
        OnboardingStatus::Error => (
            "未能完成设置",
            "Setup incomplete",
            "请重新发起设置。",
            "Start setup again.",
        ),
        OnboardingStatus::Verifying => (
            "等待确认",
            "Waiting for confirmation",
            "完成确认后，点击检查状态。",
            "After confirming, check the status.",
        ),
        OnboardingStatus::ActionRequired if has_link => (
            "请完成确认",
            "Confirm setup",
            "打开下方链接完成设置，然后返回检查状态。",
            "Open the link below to finish setup, then return to check the status.",
        ),
        OnboardingStatus::ActionRequired => (
            "连接 Agent",
            "Connect Agent",
            "继续后，按照提示完成连接。",
            "Continue and follow the prompts to connect.",
        ),
    };
    (
        language.text(title_zh, title_en).to_owned(),
        language.text(instruction_zh, instruction_en).to_owned(),
    )
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
