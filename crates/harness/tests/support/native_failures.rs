use lankide_harness::{
    AgentEventKind, ExecuteRequest, FailureCategory, HarnessConfig, HarnessFactory, HarnessFailure,
    ModelRef, SessionRequest,
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, time::Duration};
use tokio_stream::StreamExt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Start an Agent through the adapter's public API with a native process boundary.
pub async fn run(factory: &dyn HarnessFactory, adapter: &str) -> Result<()> {
    let root = tempfile::tempdir()?;
    let cli = root.path().join("native-cli");
    fs::write(&cli, include_str!("native_failure_peer.py"))?;
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700))?;
    for (index, (mode, detail, category, retryable)) in [
        ("missing_cli", "", FailureCategory::Configuration, false),
        (
            "non_executable_cli",
            "",
            FailureCategory::Configuration,
            false,
        ),
        (
            "start_failure",
            "model not found: configured-model",
            FailureCategory::Configuration,
            false,
        ),
        (
            "turn_failure",
            "authentication failed: invalid API key (401)",
            FailureCategory::Authentication,
            false,
        ),
        (
            "turn_failure",
            "rate limit exceeded (429)",
            FailureCategory::RateLimit,
            true,
        ),
        (
            "turn_failure",
            "agent could not complete the request",
            FailureCategory::Agent,
            false,
        ),
        (
            "turn_failure",
            "request timed out",
            FailureCategory::Timeout,
            true,
        ),
        (
            "turn_failure",
            "request cancelled",
            FailureCategory::Cancelled,
            false,
        ),
        (
            "process_exit",
            "native process stopped unexpectedly",
            FailureCategory::Transient,
            true,
        ),
        (
            "process_exit",
            "model not found on this server",
            FailureCategory::Configuration,
            false,
        ),
        (
            "process_exit",
            "authentication failed (401)",
            FailureCategory::Authentication,
            false,
        ),
        (
            "process_exit",
            "rate limit exceeded (429)",
            FailureCategory::RateLimit,
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let cwd = root.path().join(index.to_string());
        let config_dir = cwd.join("config");
        fs::create_dir_all(&config_dir)?;
        // Existing local files avoid copying credentials or settings from a user's home.
        for name in [
            "auth.json",
            "models.json",
            "settings.json",
            "config.toml",
            ".credentials.json",
            ".claude.json",
        ] {
            fs::write(config_dir.join(name), b"")?;
        }
        let request = SessionRequest {
            cli_version: None,
            permission_policy: Default::default(),
            config: HarnessConfig {
                launch_arguments: Default::default(),
                harness_id: adapter.into(),
                model: ModelRef::new("configured-model", "Model"),
                thinking_level: "low".into(),
                cli_path: if mode == "missing_cli" {
                    cwd.join("missing-cli")
                } else if mode == "non_executable_cli" {
                    let path = cwd.join("non-executable");
                    fs::write(&path, b"#!/bin/sh\nexit 0\n")?;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                    path
                } else {
                    cli.clone()
                },
                config_fingerprint: format!("failure-{index}"),
                environment: BTreeMap::from([
                    ("FAILURE_ADAPTER".into(), adapter.into()),
                    ("FAILURE_MODE".into(), mode.into()),
                    ("FAILURE_DETAIL".into(), detail.into()),
                    ("CODEX_HOME".into(), config_dir.display().to_string()),
                    (
                        "PI_CODING_AGENT_DIR".into(),
                        config_dir.display().to_string(),
                    ),
                    ("CLAUDE_CONFIG_DIR".into(), config_dir.display().to_string()),
                    (
                        "LANKIDE_CLAUDE_CONFIG_DIR".into(),
                        config_dir.display().to_string(),
                    ),
                ]),
            },
            working_directory: cwd,
            system_prompt: "Complete the request.".into(),
            tools: vec![],
            native_session_id: None,
        };
        let failure =
            tokio::time::timeout(Duration::from_secs(5), drive(factory, request)).await??;
        assert_eq!(failure.category, category, "{adapter}/{mode}: {failure:?}");
        assert_eq!(
            failure.retryable, retryable,
            "{adapter}/{mode}: {failure:?}"
        );
        assert!(!failure.code.is_empty());
        assert!(
            failure.detail.contains(detail),
            "original detail was lost: {failure:?}"
        );
    }
    Ok(())
}

async fn drive(factory: &dyn HarnessFactory, request: SessionRequest) -> Result<HarnessFailure> {
    let session = match factory.start(request).await {
        Ok(session) => session,
        Err(error) => return Ok(error.failure),
    };
    let outcome = async {
        let mut stream = match session
            .execute(ExecuteRequest {
                input: "Complete the request.".into(),
            })
            .await
        {
            Ok(stream) => stream,
            Err(error) => return Ok(error.failure),
        };
        while let Some(event) = stream.next().await {
            match event {
                Err(error) => return Ok(error.failure),
                Ok(event) => {
                    if let AgentEventKind::Failed { failure, .. } = event.kind {
                        return failure
                            .ok_or_else(|| "adapter did not return a structured failure".into());
                    }
                }
            }
        }
        Err("native execution ended without a failure".into())
    }
    .await;
    session.close().await?;
    outcome
}
