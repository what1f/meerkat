//! Built-in Agent business scenario against the real binary and a local
//! OpenAI-compatible Chat Completions endpoint, connected by API key.

mod fake_model;

use meerkat_harness::{
    AgentEventKind, AgentEventStream, ExecuteRequest, FailureCategory, HarnessAccountMethod,
    HarnessAvailability, HarnessConfig, HarnessCredential, HarnessFactory, HarnessSession,
    ModelRef, SessionRequest, ToolDefinition, ToolResultRequest,
};
use meerkat_harness_builtin::BuiltinHarness;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio_stream::StreamExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const API_KEY: &str = "sk-e2e-secret";

fn binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MEERKAT_BUILTIN_AGENT") {
        return Some(PathBuf::from(path));
    }
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    ["debug", "release"]
        .into_iter()
        .map(|profile| {
            target
                .join(profile)
                .join(meerkat_harness_builtin::EXECUTABLE)
        })
        .find(|path| path.is_file())
}

fn request(cli: &PathBuf, agent: &PathBuf, native_session_id: Option<String>) -> SessionRequest {
    SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config: HarnessConfig {
            launch_arguments: Vec::new(),
            harness_id: "builtin".into(),
            model: ModelRef::new("fake-model", "Fake model"),
            thinking_level: String::new(),
            cli_path: cli.clone(),
            environment: BTreeMap::new(),
            config_fingerprint: "builtin-e2e".into(),
        },
        working_directory: agent.clone(),
        system_prompt: "You are the Meerkat test member.".into(),
        tools: vec![ToolDefinition {
            name: "report".into(),
            description: "Report a value".into(),
            input_schema: json!({"type":"object","properties":{"value":{"type":"string"}}}),
            output_schema: json!({"type":"object"}),
        }],
        native_session_id,
    }
}

/// Collect events until the terminal one, answering tool calls on the way.
async fn run(
    session: &dyn HarnessSession,
    mut events: AgentEventStream,
) -> Result<Vec<AgentEventKind>> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events.next().await {
            let event = event?;
            if let AgentEventKind::ToolStarted { tool_call_id, .. } = &event.kind {
                session
                    .tool_result(ToolResultRequest {
                        tool_call_id: tool_call_id.clone(),
                        success: true,
                        content_markdown: "reported-ok".into(),
                        structured_content: json!({}),
                        resources: Vec::new(),
                        error: None,
                    })
                    .await?;
            }
            let terminal = event.is_terminal();
            seen.push(event.kind);
            if terminal {
                break;
            }
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })
    .await??;
    Ok(seen)
}

fn final_output(events: &[AgentEventKind]) -> Option<&str> {
    events.iter().find_map(|event| match event {
        AgentEventKind::Completed { final_output } => Some(final_output.as_str()),
        _ => None,
    })
}

#[tokio::test]
async fn builtin_agent_api_key_session_scenario() -> Result {
    let Some(cli) = binary() else {
        eprintln!(
            "SKIP builtin E2E: Built-in Agent binary not built; run `tools/builtin-agent/build debug`"
        );
        return Ok(());
    };
    let model = fake_model::FakeModel::start(API_KEY)?;
    let root = tempfile::tempdir()?;
    let home = root.path().join("machine/.machine/builtin-agent");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&agent)?;
    let factory = BuiltinHarness::new(&home);

    // Signed out: the member fails with an authentication category.
    assert_eq!(factory.account(), None);
    let error = match factory.start(request(&cli, &agent, None)).await {
        Ok(_) => return Err("signed-out start must fail".into()),
        Err(error) => error,
    };
    assert_eq!(error.failure.category, FailureCategory::Authentication);
    assert!(!error.message.to_lowercase().contains("fx"));

    // The key is checked against the endpoint before anything is saved.
    let rejected = factory
        .set_credential(Some(HarnessCredential::ApiKey {
            base_url: format!("{}/v1", model.url()),
            api_key: "sk-wrong".into(),
            model: None,
        }))
        .await
        .err()
        .ok_or("a wrong key must be rejected")?;
    assert_eq!(rejected.failure.code, "invalid_key");
    assert!(!format!("{rejected:?}").contains("sk-wrong"));
    let closed = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let unreachable = factory
        .set_credential(Some(HarnessCredential::ApiKey {
            base_url: format!("http://127.0.0.1:{closed}/v1"),
            api_key: API_KEY.into(),
            model: None,
        }))
        .await
        .err()
        .ok_or("a closed endpoint must be rejected")?;
    assert_eq!(unreachable.failure.code, "unreachable");
    assert_eq!(factory.account(), None);

    // Without a model, the first listed one becomes the default.
    factory
        .set_credential(Some(HarnessCredential::ApiKey {
            base_url: format!("{}/v1", model.url()),
            api_key: API_KEY.into(),
            model: None,
        }))
        .await?;
    let account = factory.account().ok_or("signed in")?;
    assert_eq!(account.method, HarnessAccountMethod::ApiKey);
    assert_eq!(account.label.as_deref(), Some("127.0.0.1"));
    let settings = std::fs::read_to_string(home.join("settings.json"))?;
    assert!(!settings.contains(API_KEY), "the key stays out of settings");
    let settings: serde_json::Value = serde_json::from_str(&settings)?;
    assert_eq!(settings["models"]["custom"], "fake-model");
    assert_eq!(
        settings["providers"]["custom"]["model_metadata"]["fake-model"],
        json!({"context_window": 200_000, "max_output_tokens": 128_000, "supports_tool_use": true})
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&home)?.permissions().mode() & 0o777,
            0o700
        );
        for name in ["settings.json", "api-key"] {
            let mode = std::fs::metadata(home.join(name))?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    let probe = factory.probe(&request(&cli, &agent, None).config).await?;
    assert_eq!(probe.availability, HarnessAvailability::Available);
    let version = probe.version.unwrap_or_default();
    assert!(
        version.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "{version}"
    );
    let models = factory.models(&request(&cli, &agent, None).config).await?;
    let listed: Vec<_> = models
        .iter()
        .map(|m| (m.id.as_str(), m.name.as_str()))
        .collect();
    assert_eq!(
        listed,
        [("fake-model", "Fake Model"), ("fake-mini", "fake-mini")]
    );
    assert!(
        models
            .iter()
            .all(|m| m.thinking_levels == ["disable", "low", "medium", "high", "xhigh", "max"]),
        "{models:?}"
    );

    // Start and stream.
    let session = factory.start(request(&cli, &agent, None)).await?;
    let native_id = session.entry().native_session_id;
    let events = run(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "Hello. The code word is kiwi.".into(),
            })
            .await?,
    )
    .await?;
    assert_eq!(final_output(&events), Some("Hello world"), "{events:?}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEventKind::AssistantDelta { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEventKind::ReasoningDelta { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEventKind::Usage { input_tokens, .. } if *input_tokens > 0))
    );
    let first = model.requests().pop().ok_or("model was called")?;
    assert_eq!(first.body["model"], "fake-model");
    assert!(
        first.body.get("reasoning_effort").is_none(),
        "a member without a thinking level sends no effort"
    );
    assert_eq!(first.body["max_tokens"], 128_000, "{}", first.body);
    assert_eq!(
        first.authorization.as_deref(),
        Some(&*format!("Bearer {API_KEY}"))
    );
    assert!(
        first
            .body
            .to_string()
            .contains("You are the Meerkat test member.")
    );

    // One Meerkat tool call through MCP.
    let events = run(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "TOOL: report something".into(),
            })
            .await?,
    )
    .await?;
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEventKind::ToolStarted { tool_name, arguments, .. }
                if tool_name == "report" && arguments["value"] == "roundtrip"
        )),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEventKind::ToolFinished { success: true, .. }))
    );
    assert!(
        final_output(&events).is_some_and(|text| text.contains("reported-ok")),
        "{events:?}"
    );

    // Steer a running turn.
    let mut stream = session
        .execute(ExecuteRequest {
            input: "WAIT then answer".into(),
        })
        .await?;
    let started = stream.next().await.ok_or("started")??;
    assert!(matches!(started.kind, AgentEventKind::Started));
    tokio::time::sleep(Duration::from_millis(800)).await;
    session
        .steer("steer-1".into(), "STEERED: also mention pears".into())
        .await?;
    let events = run(session.as_ref(), stream).await?;
    assert_eq!(final_output(&events), Some("steered"), "{events:?}");

    session.close().await?;

    // A member with a thinking level, on another model, sends that effort.
    let mut thinking = request(&cli, &agent, None);
    thinking.config.model = ModelRef::new("fake-mini", "fake-mini");
    thinking.config.thinking_level = "high".into();
    let session = factory.start(thinking).await?;
    let events = run(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "Hello again".into(),
            })
            .await?,
    )
    .await?;
    assert_eq!(final_output(&events), Some("Hello world"), "{events:?}");
    let sent = model.requests().pop().ok_or("model was called")?;
    assert_eq!(sent.body["model"], "fake-mini");
    assert_eq!(sent.body["reasoning_effort"], "high");
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("settings.json"))?)?;
    assert_eq!(
        settings["providers"]["custom"]["model_metadata"]["fake-mini"]["context_window"],
        200_000
    );
    session.close().await?;

    // A member with thinking turned off asks the model for no reasoning.
    let mut off = request(&cli, &agent, None);
    off.config.thinking_level = "disable".into();
    let session = factory.start(off).await?;
    let events = run(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "Hello once more".into(),
            })
            .await?,
    )
    .await?;
    assert_eq!(final_output(&events), Some("Hello world"), "{events:?}");
    let sent = model.requests().pop().ok_or("model was called")?;
    assert_eq!(sent.body["reasoning_effort"], "none");
    assert_eq!(sent.body["max_tokens"], 128_000);
    session.close().await?;

    // Resume after a restart keeps the conversation.
    let restarted = BuiltinHarness::new(&home);
    let session = restarted
        .resume(request(&cli, &agent, Some(native_id.clone())))
        .await?;
    assert_eq!(session.entry().native_session_id, native_id);
    let events = run(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "RECALL the code word".into(),
            })
            .await?,
    )
    .await?;
    assert_eq!(final_output(&events), Some("kiwi"), "{events:?}");

    // Cancel a running turn.
    let mut stream = session
        .execute(ExecuteRequest {
            input: "HANG forever".into(),
        })
        .await?;
    stream.next().await.ok_or("started")??;
    tokio::time::sleep(Duration::from_millis(800)).await;
    session.cancel("user stopped".into()).await?;
    let events = run(session.as_ref(), stream).await?;
    assert!(
        matches!(events.last(), Some(AgentEventKind::Cancelled { .. })),
        "{events:?}"
    );
    session.close().await?;

    // Sign-out removes the credentials.
    restarted.set_credential(None).await?;
    assert_eq!(restarted.account(), None);
    assert!(!home.join("api-key").exists());
    Ok(())
}
