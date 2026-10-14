use lankide_testkit::{
    CheckStatus, GateSelection, ProbeConfig, ProbeReport, run_prerequisite_probe,
};
use std::fs;
use std::path::Path;

#[test]
fn scenario_evidence_requires_completed_cleanup_and_durable_results() {
    use lankide_testkit::ScenarioArtifact;
    let fixture = temporary_fixture_root();
    let root = fixture.path();
    let success = ScenarioArtifact::new(root, "evidence", "success.1").unwrap();
    let read = |artifact: &ScenarioArtifact| -> serde_json::Value {
        serde_json::from_slice(&fs::read(artifact.dir.join("result.json")).unwrap()).unwrap()
    };
    assert_eq!(read(&success)["status"], "running");
    success.record("message_sent", "message-owned-by-this-scenario", "ok");
    success.record_cleanup("message_recalled", "message-owned-by-this-scenario", true);
    success.complete_cleanup();
    success.finish(Ok(())).unwrap();
    let result = read(&success);
    assert_eq!(result["status"], "passed");
    assert_eq!(result["cleanup"], "passed");
    assert_eq!(
        fs::read_to_string(success.dir.join("cleanup.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert!(
        ScenarioArtifact::new(root, "evidence", "success.1").is_err(),
        "duplicate run must not retain an old passing result"
    );

    let failed = ScenarioArtifact::new(root, "evidence", "cleanup-failed").unwrap();
    failed.record_cleanup("message_recalled", "owned-message", false);
    failed.complete_cleanup();
    assert!(
        failed.finish(Ok(())).is_err(),
        "successful scenario cannot hide failed cleanup"
    );
    assert_eq!(read(&failed)["status"], "failed");
    assert_eq!(read(&failed)["cleanup"], "failed");

    let incomplete = ScenarioArtifact::new(root, "evidence", "cleanup-incomplete").unwrap();
    assert!(
        incomplete.finish(Ok(())).is_err(),
        "cleanup must be explicitly finished"
    );
    assert_eq!(read(&incomplete)["cleanup"], "incomplete");

    let blocked = ScenarioArtifact::new(root, "evidence", "blocked-result").unwrap();
    fs::remove_file(blocked.dir.join("result.json")).unwrap();
    fs::create_dir(blocked.dir.join("result.json")).unwrap();
    blocked.complete_cleanup();
    assert!(
        blocked.finish(Ok(())).is_err(),
        "failed result publication must fail the gate"
    );
    drop((success, failed, incomplete, blocked));
}

#[test]
fn concurrent_scenario_suites_publish_independent_evidence_with_the_same_run_id() {
    use lankide_testkit::ScenarioArtifact;
    use std::sync::{Arc, Barrier};

    let ready = Arc::new(Barrier::new(32));
    std::thread::scope(|threads| {
        let workers: Vec<_> = (0..32)
            .map(|_| {
                let ready = ready.clone();
                threads.spawn(move || {
                    ready.wait();
                    let fixture = temporary_fixture_root();
                    let root = fixture.path();
                    let artifact = ScenarioArtifact::new(root, "evidence", "same-run").unwrap();
                    artifact.record("message_sent", "owned-message", "ok");
                    artifact.record_cleanup("message_recalled", "owned-message", true);
                    artifact.complete_cleanup();
                    artifact.finish(Ok(())).unwrap();
                    let result: serde_json::Value = serde_json::from_slice(
                        &fs::read(artifact.dir.join("result.json")).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(result["status"], "passed");
                    assert_eq!(result["cleanup"], "passed");
                    drop(artifact);
                    root.to_path_buf()
                })
            })
            .collect();
        let roots: std::collections::HashSet<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(roots.len(), 32, "each suite must own its fixture directory");
    });
}

#[test]
fn prerequisite_probe_runs_complete_hermetic_business_scenario_without_leaking_secrets() {
    let fixture = temporary_fixture_root();
    let result = run_scenario(fixture.path());
    result.expect("testkit prerequisite probe E2E scenario failed");
}

#[test]
fn hermetic_gate_does_not_require_live_fixture_database_or_harness_configuration() {
    let sentinel = std::env::temp_dir().join(format!(
        "lankide-testkit-live-prerequisites-must-not-be-read-{}",
        std::process::id()
    ));
    let report = run_prerequisite_probe(&ProbeConfig {
        selected_gate: GateSelection::Hermetic,
        run_live_checks: false,
        feishu_fixture_root: sentinel.join("feishu-fixture"),
        telegram_database: sentinel.join("telegram.db"),
        telegram_mutating_chat_id: None,
        authorized_telegram_chat_id: "-1001234567890".to_owned(),
        pi_path: Some(sentinel.join("pi")),
        pi_models_path: sentinel.join("models.json"),
        codex_path: Some(sentinel.join("codex")),
        codex_config_path: sentinel.join("codex.toml"),
        claude_code_path: sentinel.join("claude-code"),
    });

    assert!(
        report.is_green(),
        "hermetic gate should not need live prerequisites"
    );
    assert!(report.has_status("testkit_probe", CheckStatus::Pass));
    assert!(
        report
            .checks
            .iter()
            .filter(|check| check.gate != lankide_testkit::Gate::Hermetic)
            .all(|check| check.status == CheckStatus::NotRun)
    );
    assert!(!report.render_text().contains("feishu-fixture"));
}

fn run_scenario(fixture: &Path) -> Result<(), String> {
    create_feishu_fixture(fixture)?;
    create_telegram_fixture(&fixture.join("telegram.db"))?;
    create_harness_fixture(fixture)?;

    let secret_marker = "fixture-secret-must-not-appear";
    let config = ProbeConfig {
        selected_gate: GateSelection::Hermetic,
        run_live_checks: false,
        feishu_fixture_root: fixture.to_path_buf(),
        telegram_database: fixture.join("telegram.db"),
        telegram_mutating_chat_id: None,
        authorized_telegram_chat_id: "-1001234567890".to_owned(),
        pi_path: Some(fixture.join("pi")),
        pi_models_path: fixture.join("models.json"),
        codex_path: Some(fixture.join("codex")),
        codex_config_path: fixture.join("codex.toml"),
        claude_code_path: fixture.join("claude-code"),
    };
    let original_database =
        fs::read(&config.telegram_database).map_err(|_| "read fixture bytes")?;
    let mut readonly = config.clone();
    readonly.selected_gate = GateSelection::TelegramLiveReadonly;
    let stopped = run_prerequisite_probe(&readonly);
    for name in ["telegram_workspace", "agent_bots", "manager_bot"] {
        if !stopped.has_status(name, CheckStatus::Pass) {
            return Err(format!(
                "stopped WAL database probe failed: {}",
                stopped.render_text()
            ));
        }
    }
    if fs::read(&config.telegram_database).map_err(|_| "read probed fixture")? != original_database
    {
        return Err("read-only prerequisite probe modified database bytes".into());
    }
    let report = run_prerequisite_probe(&config);
    let text = report.render_text();
    let json = report
        .render_json()
        .map_err(|error| format!("render JSON report: {error}"))?;
    let decoded: ProbeReport = serde_json::from_str(&json)
        .map_err(|error| format!("decode machine-readable report: {error}"))?;
    if decoded != report {
        return Err("machine-readable report did not round-trip".to_owned());
    }

    if !report.is_green() {
        return Err(format!("hermetic probe should pass:\n{text}"));
    }
    for name in [
        "feishu_fixture_exists",
        "feishu_app_credentials_valid",
        "feishu_user_oauth_valid",
        "required_scopes",
        "test_chat",
        "tasklist",
        "feishu_runtime_exclusive",
        "telegram_workspace",
        "agent_bots",
        "manager_bot",
        "telegram_mutating_chat_explicit",
        "legacy_runtime_exclusive",
        "pi_executable",
        "pi_model_config",
        "codex_executable",
        "codex_model_config",
        "claude_code_executable_path",
    ] {
        if !report.checks.iter().any(|check| check.name == name) {
            return Err(format!("missing public probe check: {name}"));
        }
    }
    if report.has_status("feishu_app_credentials_valid", CheckStatus::NotRun)
        && report.has_status("feishu_user_oauth_valid", CheckStatus::NotRun)
        && report.has_status("required_scopes", CheckStatus::NotRun)
    {
        // The hermetic gate is green only because the live checks are visible
        // as not_run and excluded by explicit gate selection.
    } else {
        return Err("live Feishu checks must be visible as not_run in hermetic mode".to_owned());
    }
    if text.contains(secret_marker) || json.contains(secret_marker) {
        return Err("secret marker leaked into the public report".to_owned());
    }
    if text.contains("app_secret") || json.contains("app_secret") {
        return Err("credential field name leaked into the public report".to_owned());
    }
    Ok(())
}

fn temporary_fixture_root() -> tempfile::TempDir {
    // Reserve the directory atomically: concurrent scenarios can observe the
    // same clock timestamp even when it is expressed in nanoseconds.
    let root = tempfile::Builder::new()
        .prefix("lankide-testkit-e2e-")
        .tempdir()
        .expect("reserve private fixture directory");
    fs::create_dir_all(root.path().join(".lankide/bootstrap")).expect("create temporary fixture");
    root
}

fn create_feishu_fixture(root: &Path) -> Result<(), String> {
    let bootstrap = root.join(".lankide/bootstrap");
    fs::write(
        bootstrap.join("feishu-main.json"),
        r#"{"schema_version":1,"providers":[{"type":"feishu","app_id":"fixture-app","app_secret":"fixture-secret-must-not-appear","domain":"feishu"}]}"#,
    )
    .map_err(|error| format!("write Feishu main fixture: {error}"))?;
    fs::write(
        bootstrap.join("feishu-e2e.json"),
        r#"{"schema_version":1,"provider_id":"feishu-main","app_id":"fixture-app","brand":"feishu","test_chat":{"chat_id":"oc_fixture","name":"Lankide E2E","chat_type":"private_group"},"test_actor":{"kind":"feishu_user_oauth","user_open_id":"ou_fixture","oauth_config_dir":".lankide/lark-cli-oauth","cli_source_dir":".lankide/cli","required_scopes":["im:message","task:task:read"]},"task":{"tasklist_guid":"tasklist-fixture","tasklist_name":"Lankide E2E"}}"#,
    )
    .map_err(|error| format!("write Feishu E2E fixture: {error}"))?;
    fs::create_dir_all(root.join(".lankide/lark-cli-oauth"))
        .map_err(|error| format!("create OAuth fixture: {error}"))?;
    fs::create_dir_all(root.join(".lankide/cli"))
        .map_err(|error| format!("create CLI fixture: {error}"))?;
    set_owner_only(&bootstrap.join("feishu-main.json"))?;
    set_owner_only(&bootstrap.join("feishu-e2e.json"))?;
    Ok(())
}

fn create_telegram_fixture(path: &Path) -> Result<(), String> {
    let schema = r#"
CREATE TABLE team_metadata (im_adapter_id TEXT);
CREATE TABLE provider_namespaces (provider_id TEXT, provider_type TEXT, enabled INTEGER);
CREATE TABLE agent_provider_identities (agent_id TEXT, provider_id TEXT, enabled INTEGER);
CREATE TABLE managed_integrations (kind_id TEXT, account_id TEXT, username TEXT);
CREATE TABLE sources (external_id TEXT, source_type TEXT, display_name TEXT);
INSERT INTO team_metadata VALUES ('telegram');
INSERT INTO provider_namespaces VALUES ('telegram-main', 'telegram', 1);
INSERT INTO agent_provider_identities VALUES ('agent', 'telegram-main', 1);
INSERT INTO managed_integrations VALUES ('telegram', 'manager', 'manager_bot');
INSERT INTO sources VALUES ('-1001234567890', 'group', 'Example Team');
"#;
    let database = rusqlite::Connection::open(path).map_err(|_| "create Telegram fixture")?;
    database
        .execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|_| "enable WAL")?;
    database
        .execute_batch(schema)
        .map_err(|_| "write Telegram metadata")?;
    drop(database);
    if path.with_extension("db-wal").exists() {
        return Err("closed fixture must not retain a WAL file".into());
    }
    Ok(())
}

fn create_harness_fixture(root: &Path) -> Result<(), String> {
    fs::write(
        root.join("models.json"),
        r#"{"providers":{"fixture":{"models":[{"id":"deepseek-v4-flash"}]}}}"#,
    )
    .map_err(|error| format!("write Pi model fixture: {error}"))?;
    fs::write(root.join("codex.toml"), "model = \"gpt-5.6-luna\"\n")
        .map_err(|error| format!("write Codex config fixture: {error}"))?;
    for (name, content) in [
        ("pi", "#!/bin/sh\nprintf '%s\\n' '--model'\n"),
        ("codex", "#!/bin/sh\nprintf '%s\\n' '--model'\n"),
        ("claude-code", "#!/bin/sh\nprintf '%s\\n' '--model'\n"),
    ] {
        let path = root.join(name);
        fs::write(&path, content).map_err(|error| format!("write {name} fixture: {error}"))?;
        set_executable(&path)?;
    }
    Ok(())
}

fn set_owner_only(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| format!("stat fixture file: {error}"))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("set fixture file permissions: {error}"))?;
    }
    Ok(())
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| format!("stat executable fixture: {error}"))?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("set executable fixture permissions: {error}"))?;
    }
    Ok(())
}
