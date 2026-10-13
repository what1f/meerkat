#![forbid(unsafe_code)]

//! Shared business-scenario fixtures and safe live prerequisite probes.
//!
//! The prerequisite probe is intentionally a public, CLI-friendly boundary.
//! It reports only check names, status, paths, non-sensitive account metadata,
//! missing prerequisites, and remediation hints. It never returns credential
//! values or subprocess output.

mod artifacts;
pub use artifacts::ScenarioArtifact;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const OLD_RUNTIME_PATTERNS: &[&str] = &["[c]om\\.lankide\\.daemon", "[l]ankide-daemon --port"];
const OLD_DESKTOP_PATTERNS: &[&str] = &["[l]ankide-desktop"];
const OLD_LAUNCH_LABELS: &[&str] = &["com.lankide.daemon", "com.lankide.desktop"];

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
    pub telegram_database: PathBuf,
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
        let telegram_database = env::var_os("LANKIDE_TELEGRAM_E2E_DATABASE")
            .map(PathBuf::from)
            .unwrap_or_default();

        Self {
            selected_gate,
            run_live_checks,
            feishu_fixture_root: fixture_root,
            telegram_database,
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
        report.checks.push(if !config.run_live_checks {
            not_run(
                "feishu_runtime_exclusive",
                Gate::FeishuLive,
                "live mutation exclusivity check was not requested",
            )
        } else {
            match legacy_runtime_blocker(&config.telegram_database) {
                Ok(None) => pass(
                    "feishu_runtime_exclusive",
                    Gate::FeishuLive,
                    "legacy Lankide consumers are not running",
                ),
                Ok(Some(blocker)) => missing(
                    "feishu_runtime_exclusive",
                    Gate::FeishuLive,
                    format!("{blocker} is running; shared Bot mutation is blocked"),
                    Some("stop the old consumer before an authorized exclusive Feishu E2E"),
                ),
                Err(_) => missing(
                    "feishu_runtime_exclusive",
                    Gate::FeishuLive,
                    "could not prove that shared Bot consumers are stopped",
                    Some("check the local process/service inspection tools before live E2E"),
                ),
            }
        });
    } else {
        report.checks.extend(not_run_checks(
            [
                "feishu_fixture_exists",
                "feishu_app_credentials_valid",
                "feishu_user_oauth_valid",
                "required_scopes",
                "test_chat",
                "tasklist",
                "feishu_runtime_exclusive",
            ],
            Gate::FeishuLive,
            "Feishu gate was not selected",
        ));
    }

    if config.selected_gate.includes(Gate::TelegramLiveReadonly) {
        let telegram = inspect_telegram_database(&config.telegram_database, config);
        report.checks.push(telegram.workspace_check());
        report.checks.push(telegram.agent_bots_check());
        report.checks.push(telegram.manager_bot_check());
        report.checks.push(telegram.mutating_chat_check(config));
        report.checks.push(telegram.legacy_runtime_check(config));
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
        report.checks.push(not_run(
            "legacy_runtime_exclusive",
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
                Some("provide LANKIDE_E2E_TEAM_ROOT with the bootstrap fixture"),
            )
        }
    }

    fn app_credentials_check(&self, run_live: bool) -> CheckResult {
        if !self.credentials_present || !self.app_id_matches {
            return missing(
                "feishu_app_credentials_valid",
                Gate::FeishuLive,
                "app credential fields are incomplete",
                Some("repair the owner-only Feishu bootstrap fixture"),
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
    database_exists: bool,
    database_readable: bool,
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
                "Telegram workspace database and enabled provider are available",
            )
        } else {
            missing(
                "telegram_workspace",
                Gate::TelegramLiveReadonly,
                self.issue
                    .clone()
                    .unwrap_or_else(|| "Telegram workspace metadata is incomplete".to_owned()),
                Some("provide LANKIDE_TELEGRAM_E2E_DATABASE for an isolated test workspace"),
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
        if !self.database_exists || !self.database_readable {
            return missing(
                "telegram_mutating_chat_explicit",
                Gate::TelegramLiveMutating,
                "Telegram workspace cannot be inspected",
                Some("repair the Telegram workspace database before any write test"),
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

    fn legacy_runtime_check(&self, config: &ProbeConfig) -> CheckResult {
        if !config.run_live_checks || !config.selected_gate.includes(Gate::TelegramLiveMutating) {
            return not_run(
                "legacy_runtime_exclusive",
                Gate::TelegramLiveMutating,
                "live mutation exclusivity check was not requested",
            );
        }
        if !self.database_exists {
            return missing(
                "legacy_runtime_exclusive",
                Gate::TelegramLiveMutating,
                "Telegram workspace is unavailable for the exclusivity gate",
                Some("stop the old Runtime and provide the isolated Telegram workspace"),
            );
        }
        match legacy_runtime_blocker(&config.telegram_database) {
            Ok(None) => pass(
                "legacy_runtime_exclusive",
                Gate::TelegramLiveMutating,
                "legacy Lankide daemon and Desktop are not running",
            ),
            Ok(Some(blocker)) => missing(
                "legacy_runtime_exclusive",
                Gate::TelegramLiveMutating,
                format!("{blocker} is running; mutation is blocked"),
                Some("stop the old Lankide daemon and Desktop before a Telegram mutating E2E"),
            ),
            Err(_) => missing(
                "legacy_runtime_exclusive",
                Gate::TelegramLiveMutating,
                "could not prove that the legacy Lankide daemon and Desktop are stopped",
                Some("install pgrep, launchctl, and lsof, then stop the old daemon and Desktop"),
            ),
        }
    }
}

fn inspect_feishu_fixture(root: &Path) -> FeishuFixture {
    let main_path = root.join(".lankide/bootstrap/feishu-main.json");
    let metadata_path = root.join(".lankide/bootstrap/feishu-e2e.json");
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

    let provider = main
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|providers| {
            providers
                .iter()
                .find(|provider| provider.get("type").and_then(Value::as_str) == Some("feishu"))
        });
    let app_id = provider
        .and_then(|provider| provider.get("app_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let app_secret = provider
        .and_then(|provider| provider.get("app_secret"))
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

fn inspect_telegram_database(path: &Path, config: &ProbeConfig) -> TelegramFixture {
    let mut fixture = TelegramFixture {
        database_exists: path.is_file(),
        ..Default::default()
    };
    if !fixture.database_exists {
        fixture.issue = Some(format!(
            "Telegram workspace database not found at {}",
            path.display()
        ));
        return fixture;
    }
    let database = match rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(database) => database,
        Err(_) => {
            fixture.issue = Some("Telegram fixture database could not be opened read-only".into());
            return fixture;
        }
    };
    let _ = database.busy_timeout(Duration::from_secs(2));
    fixture.database_readable = true;
    fixture.workspace_ready = sqlite_bool(
        &database,
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM team_metadata WHERE im_adapter_id = 'telegram') AND EXISTS (SELECT 1 FROM provider_namespaces WHERE provider_type = 'telegram' AND enabled = 1) THEN 'yes' ELSE 'no' END;",
    ) == Some(true);
    fixture.agent_bot_count = sqlite_count(
        &database,
        "SELECT COUNT(*) FROM agent_provider_identities AS api JOIN provider_namespaces AS p ON p.provider_id = api.provider_id WHERE p.provider_type = 'telegram' AND api.enabled = 1;",
    )
    .unwrap_or_default();
    fixture.manager_bot_present = sqlite_bool(
        &database,
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM managed_integrations WHERE kind_id = 'telegram' AND account_id <> '' AND username <> '') THEN 'yes' ELSE 'no' END;",
    ) == Some(true);
    fixture.authorized_fixture_chat_present = is_telegram_chat_id(
        &config.authorized_telegram_chat_id,
    ) && database
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sources WHERE external_id = ?1 AND source_type = 'group')",
            params![config.authorized_telegram_chat_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    let target_chat_id = config
        .telegram_mutating_chat_id
        .as_deref()
        .unwrap_or(&config.authorized_telegram_chat_id);
    if is_telegram_chat_id(target_chat_id) {
        let query = format!(
            "SELECT CASE WHEN EXISTS (SELECT 1 FROM sources WHERE external_id = '{}' AND source_type = 'group') THEN 'yes' ELSE 'no' END;",
            target_chat_id
        );
        fixture.explicit_chat_present = sqlite_bool(&database, &query);
    } else {
        fixture.explicit_chat_present = Some(false);
    }
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

fn sqlite_bool(database: &rusqlite::Connection, query: &str) -> Option<bool> {
    match sqlite_query(database, query)?.as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn sqlite_count(database: &rusqlite::Connection, query: &str) -> Option<usize> {
    sqlite_query(database, query)?.parse().ok()
}

fn sqlite_query(database: &rusqlite::Connection, query: &str) -> Option<String> {
    database
        .query_row(query, [], |row| match row.get_ref(0)? {
            rusqlite::types::ValueRef::Integer(value) => Ok(value.to_string()),
            rusqlite::types::ValueRef::Text(value) => {
                String::from_utf8(value.to_vec()).map_err(|_| rusqlite::Error::InvalidQuery)
            }
            _ => Err(rusqlite::Error::InvalidQuery),
        })
        .ok()
}

fn legacy_runtime_blocker(database: &Path) -> Result<Option<&'static str>, ()> {
    let pgrep = resolve_executable("pgrep").ok_or(())?;
    for pattern in OLD_RUNTIME_PATTERNS {
        let status = Command::new(&pgrep)
            .args(["-f", pattern])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| ())?;
        if status.success() {
            return Ok(Some("legacy Lankide daemon"));
        }
    }
    for pattern in OLD_DESKTOP_PATTERNS {
        let status = Command::new(&pgrep)
            .args(["-f", pattern])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| ())?;
        if status.success() {
            return Ok(Some("legacy Lankide Desktop"));
        }
    }
    let launchctl = resolve_executable("launchctl").ok_or(())?;
    let uid = current_uid()?;
    for label in OLD_LAUNCH_LABELS {
        let target = format!("gui/{uid}/{label}");
        let status = Command::new(&launchctl)
            .args(["print", &target])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| ())?;
        if status.success() {
            return Ok(Some(if *label == "com.lankide.desktop" {
                "legacy Lankide Desktop service"
            } else {
                "legacy Lankide daemon service"
            }));
        }
    }
    let lsof = resolve_executable("lsof").ok_or(())?;
    let output = Command::new(lsof)
        .args(["-t", "--"])
        .arg(database)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ())?;
    if output.status.success() && !output.stdout.is_empty() {
        return Ok(Some("Telegram workspace database handle"));
    }
    if output.status.code() != Some(1) {
        return Err(());
    }
    Ok(None)
}

fn current_uid() -> Result<String, ()> {
    let id = resolve_executable("id").ok_or(())?;
    let output = Command::new(id)
        .arg("-u")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let uid = String::from_utf8(output.stdout).map_err(|_| ())?;
    let uid = uid.trim();
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    Ok(uid.to_owned())
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
