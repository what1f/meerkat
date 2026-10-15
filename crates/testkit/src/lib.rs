#![forbid(unsafe_code)]

//! Shared business-scenario fixtures and safe live prerequisite probes.
//!
//! The prerequisite probe is intentionally a public, CLI-friendly boundary.
//! It reports only check names, status, paths, non-sensitive account metadata,
//! missing prerequisites, and remediation hints. It never returns credential
//! values or subprocess output.

mod artifacts;
pub use artifacts::ScenarioArtifact;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const CRATE_NAME: &str = "lankide-testkit";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioDescription {
    pub name: String,
    pub purpose: String,
}

impl ScenarioDescription {
    pub fn new(name: impl Into<String>, purpose: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            purpose: purpose.into(),
        }
    }
}

/// The gates exposed by the unattended prerequisite runner.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Gate {
    Hermetic,
    FeishuLive,
    TelegramLiveReadonly,
    TelegramLiveMutating,
    HarnessLive,
}

impl Gate {
    pub const ALL: [Self; 5] = [
        Self::Hermetic,
        Self::FeishuLive,
        Self::TelegramLiveReadonly,
        Self::TelegramLiveMutating,
        Self::HarnessLive,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hermetic => "hermetic",
            Self::FeishuLive => "feishu-live",
            Self::TelegramLiveReadonly => "telegram-live-readonly",
            Self::TelegramLiveMutating => "telegram-live-mutating",
            Self::HarnessLive => "harness-live",
        }
    }
}

/// Selects which gate must be green for a command invocation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum GateSelection {
    All,
    Hermetic,
    FeishuLive,
    TelegramLiveReadonly,
    TelegramLiveMutating,
    HarnessLive,
}

impl GateSelection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Hermetic => "hermetic",
            Self::FeishuLive => "feishu-live",
            Self::TelegramLiveReadonly => "telegram-live-readonly",
            Self::TelegramLiveMutating => "telegram-live-mutating",
            Self::HarnessLive => "harness-live",
        }
    }

    fn includes(self, gate: Gate) -> bool {
        match self {
            Self::All => true,
            Self::Hermetic => gate == Gate::Hermetic,
            Self::FeishuLive => matches!(gate, Gate::Hermetic | Gate::FeishuLive),
            Self::TelegramLiveReadonly => {
                matches!(gate, Gate::Hermetic | Gate::TelegramLiveReadonly)
            }
            Self::TelegramLiveMutating => matches!(
                gate,
                Gate::Hermetic | Gate::TelegramLiveReadonly | Gate::TelegramLiveMutating
            ),
            Self::HarnessLive => matches!(gate, Gate::Hermetic | Gate::HarnessLive),
        }
    }
}

/// A check result safe to print in logs or attach to a machine-readable report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub gate: Gate,
    pub status: CheckStatus,
    pub detail: String,
    pub remediation: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CheckStatus {
    Pass,
    Missing,
    Failed,
    NotRun,
}

impl CheckStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Missing => "missing",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }
}

/// The result of one complete probe invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProbeReport {
    pub selected_gate: GateSelection,
    pub checks: Vec<CheckResult>,
}

impl ProbeReport {
    /// Returns false for a missing, failed, or not-run required check.
    pub fn is_green(&self) -> bool {
        self.checks
            .iter()
            .filter(|check| self.selected_gate.includes(check.gate))
            .all(|check| check.status == CheckStatus::Pass)
    }

    pub fn has_status(&self, name: &str, status: CheckStatus) -> bool {
        self.checks
            .iter()
            .any(|check| check.name == name && check.status == status)
    }

    pub fn render_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn render_text(&self) -> String {
        let mut output = String::new();
        output.push_str("lankide prerequisite probe\n");
        output.push_str("selected_gate: ");
        output.push_str(self.selected_gate.as_str());
        output.push('\n');
        for check in &self.checks {
            output.push_str(check.gate.as_str());
            output.push('\t');
            output.push_str(&check.name);
            output.push('\t');
            output.push_str(check.status.as_str());
            output.push('\t');
            output.push_str(&safe_text(&check.detail));
            if let Some(remediation) = &check.remediation {
                output.push('\t');
                output.push_str("remediation=");
                output.push_str(&safe_text(remediation));
            }
            output.push('\n');
        }
        output.push_str("result: ");
        output.push_str(if self.is_green() { "pass" } else { "not_ready" });
        output.push('\n');
        output
    }
}

/// Configuration for the probe. It contains paths and switches only; no secret
/// values are accepted by this API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeConfig {
    pub selected_gate: GateSelection,
    pub run_live_checks: bool,
    pub feishu_fixture_root: PathBuf,
    pub telegram_fixture: PathBuf,
    pub telegram_mutating_chat_id: Option<String>,
    pub authorized_telegram_chat_id: String,
    pub pi_path: Option<PathBuf>,
    pub pi_models_path: PathBuf,
    pub codex_path: Option<PathBuf>,
    pub codex_config_path: PathBuf,
    pub claude_code_path: PathBuf,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self::from_environment(GateSelection::All, true)
    }
}

impl ProbeConfig {
    pub fn from_environment(selected_gate: GateSelection, run_live_checks: bool) -> Self {
        let fixture_root = env_path("LANKIDE_E2E_TEAM_ROOT");
        let telegram_fixture = env::var_os("LANKIDE_TELEGRAM_E2E_FIXTURE")
            .map(PathBuf::from)
            .unwrap_or_default();

        Self {
            selected_gate,
            run_live_checks,
            feishu_fixture_root: fixture_root,
            telegram_fixture,
            telegram_mutating_chat_id: env::var("LANKIDE_TELEGRAM_E2E_CHAT_ID")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            authorized_telegram_chat_id: env::var("LANKIDE_TELEGRAM_AUTHORIZED_FIXTURE_CHAT_ID")
                .unwrap_or_default(),
            pi_path: env::var_os("LANKIDE_PI_PATH").map(PathBuf::from),
            pi_models_path: env_path("LANKIDE_PI_MODELS_PATH"),
            codex_path: env::var_os("LANKIDE_CODEX_PATH").map(PathBuf::from),
            codex_config_path: env_path("LANKIDE_CODEX_CONFIG_PATH"),
            claude_code_path: env_path("LANKIDE_CLAUDE_CODE_PATH"),
        }
    }
}

/// Runs all checks. The function never prints, invokes a model, or performs a
/// Telegram mutation. Live Provider tests perform remote credential validation
/// only after this local prerequisite probe succeeds.
pub fn run_prerequisite_probe(config: &ProbeConfig) -> ProbeReport {
    let mut report = ProbeReport {
        selected_gate: config.selected_gate,
        checks: Vec::new(),
    };

    report.checks.push(pass(
        "testkit_probe",
        Gate::Hermetic,
        "testkit prerequisite probe is available",
    ));

    if config.selected_gate.includes(Gate::FeishuLive) {
        let feishu = inspect_feishu_fixture(&config.feishu_fixture_root);
        report.checks.push(feishu.fixture_check());

        report
            .checks
            .push(feishu.app_credentials_check(config.run_live_checks));
        report
            .checks
            .push(feishu.oauth_check(config.run_live_checks));
        report
            .checks
            .push(feishu.scopes_check(config.run_live_checks));
        report.checks.push(feishu.test_chat_check());
        report.checks.push(feishu.tasklist_check());
    } else {
        report.checks.extend(not_run_checks(
            [
                "feishu_fixture_exists",
                "feishu_app_credentials_valid",
                "feishu_user_oauth_valid",
                "required_scopes",
                "test_chat",
                "tasklist",
            ],
            Gate::FeishuLive,
            "Feishu gate was not selected",
        ));
    }

    if config.selected_gate.includes(Gate::TelegramLiveReadonly) {
        let telegram = inspect_telegram_fixture(&config.telegram_fixture, config);
        report.checks.push(telegram.workspace_check());
        report.checks.push(telegram.agent_bots_check());
        report.checks.push(telegram.manager_bot_check());
        report.checks.push(telegram.mutating_chat_check(config));
    } else {
        report.checks.extend(not_run_checks(
            ["telegram_workspace", "agent_bots", "manager_bot"],
            Gate::TelegramLiveReadonly,
            "Telegram read-only gate was not selected",
        ));
        report.checks.push(not_run(
            "telegram_mutating_chat_explicit",
            Gate::TelegramLiveMutating,
            "Telegram mutating gate was not selected",
        ));
    }

    report.checks.extend(inspect_harnesses(config));
    report
}

#[derive(Clone, Debug, Default)]
struct FeishuFixture {
    fixture_exists: bool,
    credentials_present: bool,
    oauth_shape_valid: bool,
    app_id_matches: bool,
    test_chat_valid: bool,
    tasklist_valid: bool,
    required_scopes: Vec<String>,
    oauth_dir: Option<PathBuf>,
    cli_source_dir: Option<PathBuf>,
    fixture_root: PathBuf,
    issue: Option<String>,
}

impl FeishuFixture {
    fn fixture_check(&self) -> CheckResult {
        if self.fixture_exists {
            pass(
                "feishu_fixture_exists",
                Gate::FeishuLive,
                format!(
                    "fixture files available under {}",
                    self.fixture_root.display()
                ),
            )
        } else {
            missing(
                "feishu_fixture_exists",
                Gate::FeishuLive,
                self.issue
                    .clone()
                    .unwrap_or_else(|| "required fixture files are missing".to_owned()),
                Some("provide LANKIDE_E2E_TEAM_ROOT with the application fixture"),
            )
        }
    }

    fn app_credentials_check(&self, run_live: bool) -> CheckResult {
        if !self.credentials_present || !self.app_id_matches {
            return missing(
                "feishu_app_credentials_valid",
                Gate::FeishuLive,
                "app credential fields are incomplete",
                Some("repair the owner-only Feishu application fixture"),
            );
        }
        if !run_live {
            return not_run(
                "feishu_app_credentials_valid",
                Gate::FeishuLive,
                "live credential validation was not requested",
            );
        }
        pass(
            "feishu_app_credentials_valid",
            Gate::FeishuLive,
            "Feishu application credentials are configured; the Provider E2E performs remote validation",
        )
    }

    fn oauth_check(&self, run_live: bool) -> CheckResult {
        if !self.oauth_shape_valid {
            return missing(
                "feishu_user_oauth_valid",
                Gate::FeishuLive,
                "user OAuth fixture is incomplete",
                Some("restore the configured owner-only OAuth directory and actor metadata"),
            );
        }
        if !run_live {
            return not_run(
                "feishu_user_oauth_valid",
                Gate::FeishuLive,
                "live OAuth validation was not requested",
            );
        }
        pass(
            "feishu_user_oauth_valid",
            Gate::FeishuLive,
            "Feishu OAuth fixture is configured; the Provider E2E performs remote validation",
        )
    }

    fn scopes_check(&self, run_live: bool) -> CheckResult {
        if self.required_scopes.is_empty() {
            return missing(
                "required_scopes",
                Gate::FeishuLive,
                "no required Feishu scopes are recorded",
                Some("restore required_scopes in the E2E metadata fixture"),
            );
        }
        if !run_live {
            return not_run(
                "required_scopes",
                Gate::FeishuLive,
                format!(
                    "{} required scopes recorded; live grant check not requested",
                    self.required_scopes.len()
                ),
            );
        }
        pass(
            "required_scopes",
            Gate::FeishuLive,
            "required Feishu scopes are recorded; the Provider E2E validates the remote grants",
        )
    }

    fn test_chat_check(&self) -> CheckResult {
        if self.test_chat_valid {
            pass(
                "test_chat",
                Gate::FeishuLive,
                "Lankide E2E chat metadata is present",
            )
        } else {
            missing(
                "test_chat",
                Gate::FeishuLive,
                "Lankide E2E chat metadata is incomplete",
                Some("restore test_chat metadata in feishu-e2e.json"),
            )
        }
    }

    fn tasklist_check(&self) -> CheckResult {
        if self.tasklist_valid {
            pass(
                "tasklist",
                Gate::FeishuLive,
                "Lankide E2E tasklist metadata is ready",
            )
        } else {
            missing(
                "tasklist",
                Gate::FeishuLive,
                "Lankide E2E tasklist metadata is incomplete",
                Some("restore task metadata in feishu-e2e.json"),
            )
        }
    }
}

#[derive(Clone, Debug, Default)]
struct TelegramFixture {
    fixture_exists: bool,
    fixture_readable: bool,
    workspace_ready: bool,
    agent_bot_count: usize,
    manager_bot_present: bool,
    authorized_fixture_chat_present: bool,
    explicit_chat_present: Option<bool>,
    issue: Option<String>,
}

impl TelegramFixture {
    fn workspace_check(&self) -> CheckResult {
        if self.workspace_ready {
            pass(
                "telegram_workspace",
                Gate::TelegramLiveReadonly,
                "Telegram workspace fixture and enabled provider are available",
            )
        } else {
            missing(
                "telegram_workspace",
                Gate::TelegramLiveReadonly,
                self.issue
                    .clone()
                    .unwrap_or_else(|| "Telegram workspace metadata is incomplete".to_owned()),
                Some("provide LANKIDE_TELEGRAM_E2E_FIXTURE for an isolated test workspace"),
            )
        }
    }

    fn agent_bots_check(&self) -> CheckResult {
        if self.agent_bot_count > 0 {
            pass(
                "agent_bots",
                Gate::TelegramLiveReadonly,
                format!(
                    "{} enabled Agent bot identity entries are available",
                    self.agent_bot_count
                ),
            )
        } else {
            missing(
                "agent_bots",
                Gate::TelegramLiveReadonly,
                "no enabled Telegram Agent bot identity is configured",
                Some("configure at least one enabled Agent bot in the isolated Telegram workspace"),
            )
        }
    }

    fn manager_bot_check(&self) -> CheckResult {
        if self.manager_bot_present {
            pass(
                "manager_bot",
                Gate::TelegramLiveReadonly,
                "Telegram Manager Bot metadata is available",
            )
        } else {
            missing(
                "manager_bot",
                Gate::TelegramLiveReadonly,
                "Telegram Manager Bot metadata is missing",
                Some("configure the managed Telegram integration in the isolated workspace"),
            )
        }
    }

    fn mutating_chat_check(&self, config: &ProbeConfig) -> CheckResult {
        if !config.selected_gate.includes(Gate::TelegramLiveMutating) {
            return not_run(
                "telegram_mutating_chat_explicit",
                Gate::TelegramLiveMutating,
                "Telegram mutating gate was not selected",
            );
        }
        if !self.fixture_exists || !self.fixture_readable {
            return missing(
                "telegram_mutating_chat_explicit",
                Gate::TelegramLiveMutating,
                "Telegram workspace cannot be inspected",
                Some("repair the Telegram workspace fixture before any write test"),
            );
        }
        match self.explicit_chat_present {
            Some(true) => pass(
                "telegram_mutating_chat_explicit",
                Gate::TelegramLiveMutating,
                if config.telegram_mutating_chat_id.is_some() {
                    "explicit Telegram E2E chat is present in the workspace"
                } else if self.authorized_fixture_chat_present {
                    "authorized Telegram E2E fixture chat is present"
                } else {
                    "configured Telegram E2E chat is present"
                },
            ),
            Some(false) => missing(
                "telegram_mutating_chat_explicit",
                Gate::TelegramLiveMutating,
                "configured Telegram E2E chat is not present in the workspace",
                Some("set LANKIDE_TELEGRAM_E2E_CHAT_ID to an isolated authorized chat"),
            ),
            None => missing(
                "telegram_mutating_chat_explicit",
                Gate::TelegramLiveMutating,
                "no authorized Telegram E2E chat is configured",
                Some("set LANKIDE_TELEGRAM_E2E_CHAT_ID or use the authorized fixture chat"),
            ),
        }
    }
}

fn inspect_feishu_fixture(root: &Path) -> FeishuFixture {
    let main_path = root.join("feishu-app.json");
    let metadata_path = root.join("feishu-e2e.json");
    let mut fixture = FeishuFixture {
        fixture_root: root.to_path_buf(),
        ..Default::default()
    };
    let main = match read_owner_only_json(&main_path) {
        Ok(value) => value,
        Err(issue) => {
            fixture.issue = Some(issue);
            return fixture;
        }
    };
    let metadata = match read_owner_only_json(&metadata_path) {
        Ok(value) => value,
        Err(issue) => {
            fixture.issue = Some(issue);
            return fixture;
        }
    };
    fixture.fixture_exists = true;

    let app_id = main
        .get("app_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let app_secret = main
        .get("app_secret")
        .and_then(Value::as_str)
        .unwrap_or_default();
    fixture.app_id_matches =
        metadata.get("app_id").and_then(Value::as_str) == Some(app_id) && !app_id.is_empty();
    fixture.credentials_present = fixture.app_id_matches && !app_secret.is_empty();

    let actor = metadata.get("test_actor");
    let oauth_dir = actor
        .and_then(|actor| actor.get("oauth_config_dir"))
        .and_then(Value::as_str)
        .map(|path| resolve_fixture_path(root, path));
    let cli_source_dir = actor
        .and_then(|actor| actor.get("cli_source_dir"))
        .and_then(Value::as_str)
        .map(|path| resolve_fixture_path(root, path));
    let oauth_dir_valid = oauth_dir.as_deref().is_some_and(|path| path.is_dir());
    let cli_source_valid = cli_source_dir.as_deref().is_some_and(|path| path.is_dir());
    let actor_kind_valid = actor
        .and_then(|actor| actor.get("kind"))
        .and_then(Value::as_str)
        == Some("feishu_user_oauth");
    let user_open_id_present = actor
        .and_then(|actor| actor.get("user_open_id"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    fixture.oauth_shape_valid =
        actor_kind_valid && user_open_id_present && oauth_dir_valid && cli_source_valid;
    fixture.oauth_dir = oauth_dir;
    fixture.cli_source_dir = cli_source_dir;
    fixture.required_scopes = actor
        .and_then(|actor| actor.get("required_scopes"))
        .and_then(Value::as_array)
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(Value::as_str)
                .filter(|scope| !scope.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    fixture.test_chat_valid = metadata
        .get("test_chat")
        .and_then(|chat| chat.get("chat_id"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
        && metadata
            .get("test_chat")
            .and_then(|chat| chat.get("name"))
            .and_then(Value::as_str)
            == Some("Lankide E2E");
    fixture.tasklist_valid = metadata
        .get("task")
        .and_then(|task| task.get("tasklist_guid"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
        && metadata
            .get("task")
            .and_then(|task| task.get("tasklist_name"))
            .and_then(Value::as_str)
            == Some("Lankide E2E");
    fixture
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TelegramLiveFixture {
    pub provider_id: String,
    pub agent_id: String,
    pub account_id: String,
    pub username: String,
    pub token: String,
    pub manager_account_id: String,
    pub manager_token: String,
    pub chat_ids: Vec<String>,
}

pub fn read_telegram_fixture(path: &Path) -> Result<TelegramLiveFixture, String> {
    serde_json::from_value(read_owner_only_json(path)?)
        .map_err(|_| "Telegram fixture is invalid".into())
}

fn inspect_telegram_fixture(path: &Path, config: &ProbeConfig) -> TelegramFixture {
    let mut fixture = TelegramFixture {
        fixture_exists: path.is_file(),
        ..Default::default()
    };
    let data = match read_telegram_fixture(path) {
        Ok(data) => data,
        Err(issue) => {
            fixture.issue = Some(issue);
            return fixture;
        }
    };
    fixture.fixture_readable = true;
    fixture.workspace_ready = !data.provider_id.is_empty();
    fixture.agent_bot_count = usize::from(
        !data.agent_id.is_empty() && !data.account_id.is_empty() && !data.token.is_empty(),
    );
    fixture.manager_bot_present =
        !data.manager_account_id.is_empty() && !data.manager_token.is_empty();
    fixture.authorized_fixture_chat_present =
        is_telegram_chat_id(&config.authorized_telegram_chat_id)
            && data.chat_ids.contains(&config.authorized_telegram_chat_id);
    let target = config
        .telegram_mutating_chat_id
        .as_ref()
        .unwrap_or(&config.authorized_telegram_chat_id);
    fixture.explicit_chat_present =
        Some(is_telegram_chat_id(target) && data.chat_ids.contains(target));
    fixture
}

fn inspect_harnesses(config: &ProbeConfig) -> Vec<CheckResult> {
    if !config.selected_gate.includes(Gate::HarnessLive) {
        return [
            "pi_executable",
            "pi_model_config",
            "codex_executable",
            "codex_model_config",
            "claude_code_executable_path",
        ]
        .into_iter()
        .map(|name| not_run(name, Gate::HarnessLive, "Harness gate was not selected"))
        .collect();
    }
    let pi = config
        .pi_path
        .as_deref()
        .and_then(resolve_executable_path)
        .or_else(|| resolve_executable("pi"));
    let codex = config
        .codex_path
        .as_deref()
        .and_then(resolve_executable_path)
        .or_else(|| resolve_executable("codex"));
    let claude_code = if config.claude_code_path.as_os_str().is_empty() {
        resolve_executable("dcc")
    } else {
        resolve_executable_path(&config.claude_code_path)
    };
    vec![
        harness_executable_check("pi_executable", pi.as_deref()),
        model_file_check(
            "pi_model_config",
            &config.pi_models_path,
            "deepseek-v4-flash",
        ),
        harness_executable_check("codex_executable", codex.as_deref()),
        codex_model_check(&config.codex_config_path),
        claude_code_check(claude_code.as_deref()),
    ]
}

fn harness_executable_check(name: &str, path: Option<&Path>) -> CheckResult {
    match path {
        Some(path) => pass(
            name,
            Gate::HarnessLive,
            format!("executable available at {}", path.display()),
        ),
        None => missing(
            name,
            Gate::HarnessLive,
            "required Harness executable is not available",
            Some("install the configured Harness CLI or set its *_PATH override"),
        ),
    }
}

fn model_file_check(name: &str, path: &Path, model: &str) -> CheckResult {
    match fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
    {
        Some(value) if json_contains_model(&value, model) => pass(
            name,
            Gate::HarnessLive,
            format!("requested model {} is present in {}", model, path.display()),
        ),
        Some(_) => missing(
            name,
            Gate::HarnessLive,
            format!(
                "requested model {} is not present in the model configuration",
                model
            ),
            Some("add the configured low-cost E2E model to the Harness model configuration"),
        ),
        None => missing(
            name,
            Gate::HarnessLive,
            format!("model configuration is unavailable at {}", path.display()),
            Some("restore the Harness model configuration"),
        ),
    }
}

fn codex_model_check(path: &Path) -> CheckResult {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => {
            return missing(
                "codex_model_config",
                Gate::HarnessLive,
                format!(
                    "Codex model configuration is unavailable at {}",
                    path.display()
                ),
                Some("restore Codex configuration for gpt-5.6-luna"),
            );
        }
    };
    let model_ok = toml_key_equals(&contents, "model", "gpt-5.6-luna");
    if model_ok {
        pass(
            "codex_model_config",
            Gate::HarnessLive,
            "Codex is configured for gpt-5.6-luna; the E2E runner supplies low reasoning per invocation",
        )
    } else {
        missing(
            "codex_model_config",
            Gate::HarnessLive,
            "Codex is not configured for the requested E2E model",
            Some("configure gpt-5.6-luna and low reasoning for the E2E invocation"),
        )
    }
}

fn claude_code_check(path: Option<&Path>) -> CheckResult {
    let Some(path) = path else {
        return missing(
            "claude_code_executable_path",
            Gate::HarnessLive,
            "configured Claude Code CLI Path is not executable",
            Some("set LANKIDE_CLAUDE_CODE_PATH to an executable Claude Code-compatible CLI"),
        );
    };
    if command_supports_model_flag(path) {
        pass(
            "claude_code_executable_path",
            Gate::HarnessLive,
            format!(
                "configured Claude Code CLI Path is usable at {}",
                path.display()
            ),
        )
    } else {
        missing(
            "claude_code_executable_path",
            Gate::HarnessLive,
            "configured Claude Code CLI Path does not expose the model option",
            Some("use the configured Claude Code CLI Path with a model option"),
        )
    }
}

fn read_owner_only_json(path: &Path) -> Result<Value, String> {
    let metadata = fs::metadata(path)
        .map_err(|_| format!("required fixture file is missing: {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "fixture file permissions are too broad: {}",
                path.display()
            ));
        }
    }
    let contents = fs::read_to_string(path)
        .map_err(|_| format!("fixture file cannot be read: {}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|_| format!("fixture file is not valid JSON: {}", path.display()))
}

fn resolve_fixture_path(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn env_path(name: &str) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or_default()
}

fn resolve_executable(name: &str) -> Option<PathBuf> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return resolve_executable_path(Path::new(name));
    }
    env::split_paths(&env::var_os("PATH")?)
        .map(|directory| directory.join(name))
        .find_map(|path| resolve_executable_path(&path))
}

fn is_telegram_chat_id(value: &str) -> bool {
    let value = value.trim();
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn resolve_executable_path(path: &Path) -> Option<PathBuf> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(path.to_path_buf())
}

fn command_supports_model_flag(path: &Path) -> bool {
    let mut command = Command::new(path);
    command
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = match command.output() {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };
    String::from_utf8(output.stdout)
        .map(|output| output.contains("--model"))
        .unwrap_or(false)
}

fn json_contains_model(value: &Value, model: &str) -> bool {
    match value {
        Value::String(value) => value == model,
        Value::Array(values) => values.iter().any(|value| json_contains_model(value, model)),
        Value::Object(values) => values
            .values()
            .any(|value| json_contains_model(value, model)),
        _ => false,
    }
}

fn toml_key_equals(contents: &str, key: &str, expected: &str) -> bool {
    contents.lines().any(|line| {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((actual_key, value)) = line.split_once('=') else {
            return false;
        };
        actual_key.trim() == key && value.trim().trim_matches('"') == expected
    })
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn not_run_checks<const N: usize>(names: [&str; N], gate: Gate, detail: &str) -> Vec<CheckResult> {
    names
        .into_iter()
        .map(|name| not_run(name, gate, detail))
        .collect()
}

fn pass(name: &str, gate: Gate, detail: impl Into<String>) -> CheckResult {
    result(name, gate, CheckStatus::Pass, detail, None)
}

fn missing(
    name: &str,
    gate: Gate,
    detail: impl Into<String>,
    remediation: Option<&str>,
) -> CheckResult {
    result(
        name,
        gate,
        CheckStatus::Missing,
        detail,
        remediation.map(str::to_owned),
    )
}

fn not_run(name: &str, gate: Gate, detail: impl Into<String>) -> CheckResult {
    result(name, gate, CheckStatus::NotRun, detail, None)
}

fn result(
    name: &str,
    gate: Gate,
    status: CheckStatus,
    detail: impl Into<String>,
    remediation: Option<String>,
) -> CheckResult {
    CheckResult {
        name: name.to_owned(),
        gate,
        status,
        detail: safe_text(&detail.into()),
        remediation: remediation.map(|value| safe_text(&value)),
    }
}
