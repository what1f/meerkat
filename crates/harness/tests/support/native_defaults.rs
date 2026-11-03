use meerkat_harness::{
    AgentEventKind, ExecuteRequest, HarnessCapability, HarnessConfig, HarnessFactory, ModelRef,
    SessionRequest, ToolDefinition, ToolResultError, ToolResultRequest,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio_stream::StreamExt;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// S7.2: An Agent starts its native Harness with full access and home skills,
/// keeps native context/settings, and sees a newly authored skill on its next run.
pub async fn agent_native_defaults(factory: &dyn HarnessFactory, fixture: &Path) -> Result {
    for existing_native_skills in [false, true] {
        let home = tempfile::tempdir()?;
        let agent = home.path().join("agent");
        let native_config = home.path().join("native-config");
        let id = &factory.info().harness_id;
        let record = home.path().join("start.jsonl");
        let cli = home.path().join("native-cli");
        fs::copy(fixture, &cli)?;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700))?;
        fs::create_dir_all(agent.join("skills/bug-triage"))?;
        let skill = "---\nname: bug-triage\ndescription: Diagnose a reported bug.\n---\nRead the report before editing.\n";
        fs::write(agent.join("skills/bug-triage/SKILL.md"), skill)?;
        for name in ["AGENTS.md", "CLAUDE.md"] {
            fs::write(agent.join(name), "Keep the project instructions.")?;
        }
        fs::create_dir_all(&native_config)?;
        // All overlay inputs are local, so this scenario never reads real auth.
        for name in [
            "auth.json",
            "models.json",
            "settings.json",
            "config.toml",
            ".credentials.json",
            ".claude.json",
        ] {
            fs::write(
                native_config.join(name),
                if name.ends_with("toml") {
                    "# project settings\n"
                } else {
                    "{}"
                },
            )?;
        }
        let native_skills = match id.as_str() {
            "claude_code" => agent.join(".claude/skills"),
            "codex" => agent.join(".agents/skills"),
            _ => agent.join("skills"),
        };
        if existing_native_skills && id != "pi" {
            fs::create_dir_all(native_skills.join("project-skill"))?;
            fs::write(
                native_skills.join("project-skill/SKILL.md"),
                "Keep the native skill.",
            )?;
        }
        let private = agent.join(".meerkat").join(id).join("native-home");
        fs::create_dir_all(&private)?;
        let credential = if id == "claude_code" {
            ".credentials.json"
        } else {
            "auth.json"
        };
        fs::write(private.join(credential), "agent-owned credential")?;
        let config = HarnessConfig {
            launch_arguments: Default::default(),
            harness_id: id.clone(),
            model: ModelRef::new("native-model", "Native model"),
            thinking_level: "low".into(),
            cli_path: cli,
            config_fingerprint: "native-defaults".into(),
            environment: BTreeMap::from([
                ("HOME".into(), home.path().display().to_string()),
                ("NATIVE_ADAPTER".into(), id.clone()),
                ("NATIVE_START_RECORD".into(), record.display().to_string()),
                ("PI_NATIVE_FAULT".into(), "success".into()),
                (
                    "PI_TEST_VERSION".into(),
                    if existing_native_skills {
                        "0.99.0"
                    } else {
                        "0.83.0"
                    }
                    .into(),
                ),
                (
                    "PI_CODING_AGENT_DIR".into(),
                    native_config.display().to_string(),
                ),
                ("CODEX_HOME".into(), native_config.display().to_string()),
                (
                    "CLAUDE_CONFIG_DIR".into(),
                    native_config.display().to_string(),
                ),
                (
                    "MEERKAT_CLAUDE_CONFIG_DIR".into(),
                    native_config.display().to_string(),
                ),
                (
                    "MEERKAT_RUNTIME_RUN".into(),
                    home.path().join("run").display().to_string(),
                ),
            ]),
        };
        let probe = factory.probe(&config).await?;
        assert_eq!(
            probe.availability,
            meerkat_harness::HarnessAvailability::Available
        );
        let capabilities = probe.capabilities.clone();
        assert_eq!(
            capabilities,
            [
                HarnessCapability::SystemPrompt,
                HarnessCapability::Streaming,
                HarnessCapability::Resume,
                HarnessCapability::Steer,
                HarnessCapability::Cancel,
                HarnessCapability::Tools
            ]
            .into_iter()
            .collect()
        );
        for run in 0..2 {
            if run == 1 {
                fs::write(native_config.join("settings.json"), "{\"user_setting\":2}")?;
                fs::write(native_config.join("config.toml"), "# refreshed settings\n")?;
                fs::create_dir_all(agent.join("skills/new-skill"))?;
                fs::write(
                    agent.join("skills/new-skill/SKILL.md"),
                    "Authored after the first run.",
                )?;
            }
            let request = SessionRequest {
                cli_version: probe.version.clone(),
                permission_policy: Default::default(),
                config: config.clone(),
                working_directory: agent.clone(),
                system_prompt: "Follow the project instructions.".into(),
                native_session_id: None,
                tools: ["report", "message_send", "attention_set", "chat_messages"]
                    .into_iter()
                    .map(|name| ToolDefinition {
                        name: name.into(),
                        description: format!("Use {name}"),
                        input_schema: json!({"type":"object"}),
                        output_schema: json!({"type":"object"}),
                    })
                    .collect(),
            };
            let session = factory.start(request.clone()).await?;
            assert!(session.reusable());
            let native_id = session.entry().native_session_id;
            for turn in 0..2 {
                if turn > 0 {
                    session.rebind(request.clone()).await?;
                    assert_eq!(session.entry().native_session_id, native_id);
                }
                let mut events = session
                    .execute(ExecuteRequest {
                        input: "Use the bug-triage skill.".into(),
                    })
                    .await?;
                tokio::time::timeout(Duration::from_secs(5), async {
                    while let Some(event) = events.next().await {
                        let event = event?;
                        if let AgentEventKind::ToolStarted {
                            tool_call_id,
                            tool_name,
                            arguments,
                        } = &event.kind
                        {
                            assert!(id == "codex" || id == "claude_code");
                            assert_eq!(tool_name, "report");
                            assert_eq!(arguments["value"], "namespace-roundtrip");
                            session
                                .tool_result(ToolResultRequest {
                                    tool_call_id: tool_call_id.clone(),
                                    success: id != "claude_code",
                                    content_markdown: if id == "claude_code" {
                                        String::new()
                                    } else {
                                        "namespace-roundtrip".into()
                                    },
                                    structured_content: json!({}),
                                    resources: Vec::new(),
                                    error: (id == "claude_code").then(|| ToolResultError {
                                        code: "permission_denied".into(),
                                        message: "Access to this conversation was denied.".into(),
                                        retryable: false,
                                    }),
                                })
                                .await?;
                        }
                        if matches!(event.kind, AgentEventKind::Completed { .. }) {
                            return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
                        }
                    }
                    Err("native turn ended without completing".into())
                })
                .await??;
            }
            session.close().await?;
            assert_eq!(
                fs::read_to_string(private.join(credential))?,
                "agent-owned credential"
            );
            let inherited: Vec<String> =
                serde_json::from_slice(&fs::read(private.join(".meerkat-inherited.json"))?)?;
            assert!(!inherited.iter().any(|name| name == credential));
            let settings = if id == "codex" {
                "config.toml"
            } else {
                "settings.json"
            };
            assert_eq!(
                fs::read_to_string(private.join(settings))?,
                fs::read_to_string(native_config.join(settings))?
            );
            assert!(!native_config.join("sessions").exists());
            assert!(!native_config.join("projects").exists());
            assert!(!native_config.join("skills/bug-triage").exists());
            assert_eq!(
                fs::read_to_string(native_skills.join("bug-triage/SKILL.md"))?,
                skill
            );
            if run == 1 {
                assert!(native_skills.join("new-skill/SKILL.md").is_file());
            }
        }
        let recorded: Vec<Value> = fs::read_to_string(&record)?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let launches: Vec<_> = recorded
            .iter()
            .filter(|entry| {
                entry.get("argv").is_some()
                    && !entry["argv"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|arg| arg == "--version" || arg == "--help")
            })
            .collect();
        assert_eq!(
            launches.len(),
            2,
            "two turns must share each native process"
        );
        assert_eq!(
            recorded
                .iter()
                .filter(|entry| entry["argv"]
                    .as_array()
                    .is_some_and(|args| args.iter().any(|arg| arg == "--version")))
                .count(),
            1,
            "sessions use the version the Machine detected"
        );
        for launch in launches {
            assert_eq!(
                Path::new(launch["cwd"].as_str().unwrap()).canonicalize()?,
                agent.canonicalize()?
            );
            let argv: Vec<_> = launch["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            match id.as_str() {
                "pi" => {
                    assert!(argv.contains(&"--approve"));
                    for flag in [
                        "--no-skills",
                        "--no-context-files",
                        "--no-prompt-templates",
                        "--no-approve",
                    ] {
                        assert!(!argv.contains(&flag));
                    }
                    for flag in ["--extension", "--no-themes"] {
                        assert!(argv.contains(&flag));
                    }
                    let index = argv.iter().position(|arg| *arg == "--skill").unwrap();
                    assert_eq!(Path::new(argv[index + 1]), agent.join("skills"));
                    assert_eq!(
                        launch["environment"]["PI_CODING_AGENT_DIR"].as_str(),
                        agent.join(".meerkat").join(id).join("native-home").to_str()
                    );
                }
                "claude_code" => {
                    let index = argv
                        .iter()
                        .position(|arg| *arg == "--permission-mode")
                        .unwrap();
                    assert_eq!(launch["environment"]["ENABLE_TOOL_SEARCH"], "true");
                    assert_eq!(argv[index + 1], "bypassPermissions");
                    assert!(!argv.contains(&"--setting-sources"));
                    assert!(!argv.contains(&"--strict-mcp-config"));
                    assert!(!argv.contains(&"--allowedTools"));
                    assert!(argv.contains(&"--append-system-prompt"));
                    assert!(!argv.contains(&"--system-prompt"));
                    let index = argv.iter().position(|arg| *arg == "--mcp-config").unwrap();
                    let mcp: Value = serde_json::from_str(&fs::read_to_string(argv[index + 1])?)?;
                    assert!(mcp["mcpServers"]["meerkat"].is_object());
                    assert_eq!(mcp["mcpServers"]["meerkat"]["alwaysLoad"], true);
                    assert_eq!(
                        launch["environment"]["CLAUDE_CONFIG_DIR"].as_str(),
                        agent.join(".meerkat").join(id).join("native-home").to_str()
                    );
                }
                "codex" => {
                    assert_eq!(argv, ["app-server", "--listen", "stdio://"]);
                    assert_eq!(
                        launch["environment"]["CODEX_HOME"].as_str(),
                        agent.join(".meerkat").join(id).join("native-home").to_str()
                    );
                }
                _ => unreachable!(),
            }
            assert_eq!(launch["environment"]["HOME"].as_str(), home.path().to_str());
        }
        if id == "codex" {
            let threads: Vec<_> = recorded
                .iter()
                .filter(|entry| entry["command"]["method"] == "thread/start")
                .collect();
            assert_eq!(threads.len(), 2);
            for thread in threads {
                let params = &thread["command"]["params"];
                assert_eq!(params["approvalPolicy"], "never");
                assert_eq!(params["sandbox"], "danger-full-access");
                assert!(params.get("permissions").is_none());
                let namespace = &params["dynamicTools"][0];
                assert_eq!(namespace["type"], "namespace");
                assert_eq!(namespace["name"], "meerkat");
                let tools = namespace["tools"].as_array().unwrap();
                assert_eq!(tools[0]["name"], "report");
                assert_eq!(tools[0]["deferLoading"], true);
                for tool in tools.iter().skip(1) {
                    assert_eq!(tool["deferLoading"], false);
                }
            }
        }
        if id == "claude_code" {
            for entry in recorded.iter().filter(|v| v.get("mcp_tools").is_some()) {
                let tools = entry["mcp_tools"].as_array().unwrap();
                assert_eq!(tools.len(), 4);
                for tool in tools {
                    assert_eq!(
                        tool["_meta"]["anthropic/alwaysLoad"],
                        tool["name"] != "report"
                    );
                }
            }
            assert_eq!(
                recorded
                    .iter()
                    .filter(|v| v.get("mcp_tools").is_some())
                    .count(),
                2
            );
        }
        if id == "pi" {
            let probes: Vec<_> = recorded
                .iter()
                .filter(|v| v.get("extension_tools").is_some())
                .collect();
            assert_eq!(probes.len(), 2);
            for probe in probes {
                let tools = probe["extension_tools"].as_array().unwrap();
                assert_eq!(tools.len(), 4);
                if existing_native_skills {
                    assert!(
                        tools
                            .iter()
                            .any(|t| t["name"] == "meerkat_0_report" && t["exposure"] == "deferred")
                    );
                    assert!(
                        probe["active"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|t| t == "tool_search")
                    );
                } else {
                    assert!(tools.iter().all(|t| t["name"] != "meerkat_0_report"));
                    assert_eq!(probe["discovered"][0]["name"], "report");
                    assert_eq!(probe["described"]["input_schema"]["type"], "object");
                    assert_eq!(probe["invoked"]["tool_name"], "report");
                }
            }
        }
        if id == "pi" {
            assert!(!factory.models(&config).await?.is_empty());
            let recorded: Vec<Value> = fs::read_to_string(&record)?
                .lines()
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?;
            let catalog = recorded
                .iter()
                .rev()
                .find(|entry| entry.get("argv").is_some())
                .unwrap();
            for flag in ["--no-skills", "--no-context-files", "--no-prompt-templates"] {
                assert!(!catalog["argv"].as_array().unwrap().contains(&json!(flag)));
            }
        }
        for name in ["AGENTS.md", "CLAUDE.md"] {
            assert_eq!(
                fs::read_to_string(agent.join(name))?,
                "Keep the project instructions."
            );
        }
        if existing_native_skills && id != "pi" {
            assert_eq!(
                fs::read_to_string(native_skills.join("project-skill/SKILL.md"))?,
                "Keep the native skill."
            );
        }
        assert_eq!(
            fs::read_to_string(native_config.join("config.toml"))?,
            "# refreshed settings\n"
        );
    }
    Ok(())
}
