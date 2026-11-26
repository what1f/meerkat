//! Built-in Agent business scenario running in-process via `jimu-*` against a
//! local OpenAI-compatible Chat Completions endpoint, connected by API key.

mod fake_model;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use meerkat_harness::{
    AgentEventKind, AgentEventStream, ExecuteRequest, FailureCategory, HarnessAccountMethod,
    HarnessAvailability, HarnessConfig, HarnessCredential, HarnessFactory, HarnessSession,
    ModelRef, SessionRequest, ToolDefinition, ToolResultRequest, ToolResultResource,
};
use meerkat_harness_builtin::BuiltinHarness;
use serde_json::json;
use std::{collections::BTreeMap, path::Path, path::PathBuf, time::Duration};
use tokio_stream::StreamExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const API_KEY: &str = "sk-e2e-secret";
const API_KEY_B: &str = "sk-e2e-secret-second";
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn request(agent: &Path, native_session_id: Option<String>) -> SessionRequest {
    SessionRequest {
        cli_version: None,
        permission_policy: Default::default(),
        config: HarnessConfig {
            launch_arguments: Vec::new(),
            harness_id: "builtin".into(),
            model: ModelRef::new("fake-model", "Fake model"),
            thinking_level: String::new(),
            cli_path: PathBuf::from(meerkat_harness_builtin::HARNESS_ID),
            environment: BTreeMap::new(),
            config_fingerprint: "builtin-e2e".into(),
        },
        working_directory: agent.to_path_buf(),
        system_prompt: "You are the Meerkat test member.".into(),
        tools: vec![ToolDefinition {
            name: "report".into(),
            description: "Report a value".into(),
            input_schema: json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}),
            output_schema: json!({"type":"object"}),
        }],
        native_session_id,
    }
}

/// Collect events until the terminal one, answering host tool calls on the way.
async fn run(
    session: &dyn HarnessSession,
    events: AgentEventStream,
) -> Result<Vec<AgentEventKind>> {
    run_with_resources(session, events, Vec::new()).await
}

async fn run_with_resources(
    session: &dyn HarnessSession,
    mut events: AgentEventStream,
    resources: Vec<ToolResultResource>,
) -> Result<Vec<AgentEventKind>> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(event) = events.next().await {
            let event = event?;
            if let AgentEventKind::ToolStarted {
                tool_call_id,
                tool_name,
                ..
            } = &event.kind
            {
                let content_markdown = match tool_name.as_str() {
                    "audit" => "audit-ok".into(),
                    _ => "reported-ok".into(),
                };
                session
                    .tool_result(ToolResultRequest {
                        tool_call_id: tool_call_id.clone(),
                        success: true,
                        content_markdown,
                        structured_content: json!({}),
                        resources: resources.clone(),
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
    let model = fake_model::FakeModel::start(API_KEY)?;
    let model_b = fake_model::FakeModel::start(API_KEY_B)?;
    let root = tempfile::tempdir()?;
    let home = root.path().join("machine/.machine/builtin-agent");
    let agent = root.path().join("agent");
    std::fs::create_dir_all(&agent)?;
    std::fs::write(
        agent.join("AGENTS.md"),
        "Workspace persona: verify jimu instructions.",
    )?;
    let factory = BuiltinHarness::new(&home);

    // Signed out: the member fails with an authentication category.
    assert_eq!(factory.account(), None);
    let error = match factory.start(request(&agent, None)).await {
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

    let probe = factory.probe(&request(&agent, None).config).await?;
    assert_eq!(probe.availability, HarnessAvailability::Available);
    let version = probe.version.unwrap_or_default();
    assert!(
        version.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "{version}"
    );
    let models = factory.models(&request(&agent, None).config).await?;
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
    let session = factory.start(request(&agent, None)).await?;
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
    // Cancel after terminal state must not claim acceptance.
    let late_cancel = session.cancel("too late".into()).await?;
    assert!(!late_cancel.accepted);

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
    let first_body = first.body.to_string();
    assert!(first_body.contains("You are the Meerkat test member."));
    assert!(
        first_body.contains("Workspace persona: verify jimu instructions."),
        "AGENTS.md should be discovered and injected: {first_body}"
    );
    let tool_names: Vec<&str> = first.body["tools"]
        .as_array()
        .ok_or("tools array missing")?
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    for expected in [
        "read",
        "write",
        "edit",
        "apply_patch",
        "find",
        "grep",
        "exec_command",
        "write_stdin",
        "web_fetch",
        "tool_search",
        "call_tool",
    ] {
        assert!(
            tool_names.contains(&expected),
            "missing resident tool `{expected}` in {tool_names:?}"
        );
    }
    assert!(
        !tool_names.contains(&"report"),
        "dynamic host tools should live behind the deferred gateway, not top-level tools"
    );
    assert!(
        first_body.contains("report"),
        "deferred host tool catalog should list `report`: {first_body}"
    );

    // Add a skill dynamically after session start; auto_reload_skills picks it up before next turn.
    let skill_dir = agent.join("skills/hot-skill");
    std::fs::create_dir_all(&skill_dir)?;
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: hot-skill\ndescription: Dynamically added skill\n---\nUse hot-skill.\n",
    )?;

    // One Meerkat host tool call via deferred gateway (`tool_search` -> `call_tool`),
    // returning a PNG resource that must be forwarded to the model as an image block.
    let report_png = agent.join("report.png");
    std::fs::write(&report_png, TINY_PNG)?;
    let events = run_with_resources(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "TOOL: report something".into(),
            })
            .await?,
        vec![ToolResultResource {
            local_path: report_png,
            file_name: "report.png".into(),
            media_type: "image/png".into(),
            size_bytes: TINY_PNG.len() as u64,
            sha256: "tiny-png-e2e".into(),
        }],
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
    let tool_turn_req = model.requests().pop().ok_or("tool turn request")?;
    let tool_turn_body = tool_turn_req.body.to_string();
    assert!(
        tool_turn_body.contains("hot-skill"),
        "auto_reload_skills should surface newly added skill: {tool_turn_body}"
    );
    let expected_image_url = format!("data:image/png;base64,{}", STANDARD.encode(TINY_PNG));
    let has_tool_image = tool_turn_req.body["messages"]
        .as_array()
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message["content"].as_array().is_some_and(|parts| {
                    parts.iter().any(|part| {
                        part["type"] == "image_url"
                            && part["image_url"]["url"] == expected_image_url
                    })
                })
            })
        });
    assert!(
        has_tool_image,
        "outbound model request must include the PNG ToolResultResource image block: {tool_turn_body}"
    );

    // Steer a running turn.
    let mut stream = session
        .execute(ExecuteRequest {
            input: "WAIT then answer".into(),
        })
        .await?;
    let started = stream.next().await.ok_or("started")??;
    assert!(matches!(started.kind, AgentEventKind::Started));
    session
        .steer("steer-1".into(), "STEERED: also mention pears".into())
        .await?;
    let events = run(session.as_ref(), stream).await?;
    assert_eq!(final_output(&events), Some("steered"), "{events:?}");

    session.close().await?;

    // A member with a thinking level, on a non-default model, sends that effort and
    // stays reusable across `rebind` (`describe_model` must not bump `auth_revision`).
    let mut thinking = request(&agent, None);
    thinking.config.model = ModelRef::new("fake-mini", "fake-mini");
    thinking.config.thinking_level = "xhigh".into();
    let session = factory.start(thinking.clone()).await?;
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
    assert_eq!(sent.body["reasoning_effort"], "xhigh");
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("settings.json"))?)?;
    assert_eq!(
        settings["providers"]["custom"]["model_metadata"]["fake-mini"]["context_window"],
        200_000
    );
    session.rebind(thinking).await?;
    session.close().await?;

    // A member with thinking turned off asks the model for no reasoning.
    let mut off = request(&agent, None);
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

    // Resume after a restart with a modified host tool schema and a newly added host tool.
    // Both must be searchable and executable through the deferred gateway.
    let restarted = BuiltinHarness::new(&home);
    let mut resume_with_updated_tools = request(&agent, Some(native_id.clone()));
    resume_with_updated_tools.tools = vec![
        ToolDefinition {
            name: "report".into(),
            description: "Report a value with a tag".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "value": {"type": "string"},
                    "tag": {"type": "string"}
                },
                "required": ["value", "tag"]
            }),
            output_schema: json!({"type": "object"}),
        },
        ToolDefinition {
            name: "audit".into(),
            description: "Record an audit note".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"note": {"type": "string"}},
                "required": ["note"]
            }),
            output_schema: json!({"type": "object"}),
        },
    ];
    let session = restarted.resume(resume_with_updated_tools).await?;
    assert_eq!(session.entry().native_session_id, native_id);
    let events = run(
        session.as_ref(),
        session
            .execute(ExecuteRequest {
                input: "DEFERRED_UPDATE: inspect and invoke both tools".into(),
            })
            .await?,
    )
    .await?;
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEventKind::ToolStarted { tool_name, arguments, .. }
                if tool_name == "report"
                    && arguments["value"] == "v2"
                    && arguments["tag"] == "updated"
        )),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEventKind::ToolStarted { tool_name, arguments, .. }
                if tool_name == "audit" && arguments["note"] == "checked"
        )),
        "{events:?}"
    );
    assert_eq!(
        final_output(&events),
        Some("deferred-updated-ok"),
        "{events:?}"
    );
    session.close().await?;

    // Switch Machine connection to a second endpoint (`model_b`, new API key, same model ID)
    // and resume the exact same persisted session. The resumed session must route exclusively
    // to `model_b` and never leak `API_KEY_B` to the old `model` endpoint.
    let model_a_requests_before_switch = model.requests().len();
    restarted
        .set_credential(Some(HarnessCredential::ApiKey {
            base_url: format!("{}/v1", model_b.url()),
            api_key: API_KEY_B.into(),
            model: None,
        }))
        .await?;
    let session = restarted
        .resume(request(&agent, Some(native_id.clone())))
        .await?;
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
    assert_eq!(
        model.requests().len(),
        model_a_requests_before_switch,
        "resumed session must never send requests or new credentials to the old base_url"
    );
    let switched_req = model_b
        .requests()
        .pop()
        .ok_or("second endpoint must receive the resumed request")?;
    assert_eq!(
        switched_req.authorization.as_deref(),
        Some(&*format!("Bearer {API_KEY_B}"))
    );

    // Cancel a running turn.
    let mut stream = session
        .execute(ExecuteRequest {
            input: "HANG forever".into(),
        })
        .await?;
    stream.next().await.ok_or("started")??;
    let cancel = session.cancel("user stopped".into()).await?;
    assert!(cancel.accepted);
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
