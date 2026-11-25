use crate::{
    ActiveConnection, BuiltinHarness, COMPACTION_RESERVE, HARNESS_ID, MAX_OUTPUT_TOKENS,
    classify_agent_failure,
};
use futures_util::StreamExt;
use jimu_agent::{
    AgentEvent as JimuAgentEvent, AgentTool, AgentToolResult, ApiKeyResolver, ToolProgressCallback,
};
use jimu_ai::oauth::CODEX_DEFAULT_BASE_URL;
use jimu_ai::{
    ApiKind, AssistantMessage, AssistantMessageEvent, BoxFuture, CancelToken, Client, ContentPart,
    HttpTransport, Model, StopReason, TextContent, ThinkingLevel,
    ToolDefinition as JimuToolDefinition,
};
use jimu_http::ReqwestTransport;
use jimu_kit::{
    InstructionDiscoveryConfig, KitError, KitEvent, KitSession, KitSessionConfig, KitToolSpec,
    SkillDiscoveryConfig, WalkBoundary,
};
use jimu_session::{CompactionConfig, JsonlSessionStore, SessionError};
use jimu_tools::{
    BuiltinToolKind, BuiltinToolSet, EnvProvider, ExecCommandTool, ProcessSessionManager, WakeGate,
    WebFetchConfig, WebFetchTool, WorkspaceTools, WorkspaceToolsConfig, WriteStdinTool,
};
use meerkat_harness::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    HarnessError, HarnessErrorCode, HarnessFuture, HarnessSession, SessionEntry, SessionHealth,
    SessionRequest, SteerReceipt, ToolDefinition, ToolResultReceipt, ToolResultRequest,
    prepare_native_skills,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;

type EventSender = mpsc::UnboundedSender<Result<AgentEvent, HarnessError>>;

struct ActiveTurn {
    sender: EventSender,
    sequence: u64,
}

struct SessionState {
    entry: SessionEntry,
    health: Mutex<SessionHealth>,
    active: Mutex<Option<ActiveTurn>>,
    cancel_reason: Mutex<Option<String>>,
    pending_tools: Mutex<HashMap<String, oneshot::Sender<ToolResultRequest>>>,
    applied_tool_results: Mutex<HashMap<String, Value>>,
}

impl SessionState {
    fn new(native_session_id: String, config_fingerprint: String) -> Self {
        Self {
            entry: SessionEntry {
                harness_id: HARNESS_ID.into(),
                native_session_id,
                config_fingerprint,
                health: SessionHealth::Healthy,
            },
            health: Mutex::new(SessionHealth::Healthy),
            active: Mutex::new(None),
            cancel_reason: Mutex::new(None),
            pending_tools: Mutex::new(HashMap::new()),
            applied_tool_results: Mutex::new(HashMap::new()),
        }
    }

    fn current_health(&self) -> SessionHealth {
        *self.health.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_health(&self, next: SessionHealth) {
        let mut health = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if *health != SessionHealth::Closed || next == SessionHealth::Closed {
            *health = next;
        }
    }

    fn emit(&self, kind: AgentEventKind) {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(turn) = active.as_mut() {
            turn.sequence += 1;
            let _ = turn.sender.send(Ok(AgentEvent {
                sequence: turn.sequence,
                kind,
            }));
        }
    }

    fn finish_turn(&self, kind: AgentEventKind) {
        let next_health = match &kind {
            AgentEventKind::Failed { .. } => SessionHealth::Broken,
            AgentEventKind::Cancelled { .. } => SessionHealth::Cancelled,
            _ => SessionHealth::Healthy,
        };
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(turn) = active.as_mut() {
            turn.sequence += 1;
            let _ = turn.sender.send(Ok(AgentEvent {
                sequence: turn.sequence,
                kind,
            }));
        }
        active.take();
        self.pending_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.set_health(next_health);
    }
}

struct HostToolBridge {
    definition: JimuToolDefinition,
    state: Arc<SessionState>,
}

impl HostToolBridge {
    fn new(tool: ToolDefinition, state: Arc<SessionState>) -> Self {
        let parameters = if tool.input_schema.is_object() || tool.input_schema.is_boolean() {
            tool.input_schema
        } else {
            json!({ "type": "object" })
        };
        Self {
            definition: JimuToolDefinition {
                name: tool.name,
                description: tool.description,
                parameters,
            },
            state,
        }
    }
}

impl AgentTool for HostToolBridge {
    fn definition(&self) -> JimuToolDefinition {
        self.definition.clone()
    }

    fn execute(
        &self,
        tool_call_id: String,
        args: Value,
        cancel: CancelToken,
        _on_update: ToolProgressCallback,
    ) -> BoxFuture<'_, Result<AgentToolResult, String>> {
        let tool_name = self.definition.name.clone();
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Ok(AgentToolResult::error("Operation aborted"));
            }
            let (tx, rx) = oneshot::channel();
            state
                .pending_tools
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(tool_call_id.clone(), tx);
            state.emit(AgentEventKind::ToolStarted {
                tool_call_id: tool_call_id.clone(),
                tool_name,
                arguments: args,
            });
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    state
                        .pending_tools
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&tool_call_id);
                    return Ok(AgentToolResult::error("Operation aborted"));
                }
                received = rx => match received {
                    Ok(req) => req,
                    Err(_) => return Ok(AgentToolResult::error("Tool call cancelled")),
                }
            };
            Ok(tool_result_to_agent_result(result))
        })
    }
}

fn tool_result_to_agent_result(mut req: ToolResultRequest) -> AgentToolResult {
    if let Some(error) = &req.error {
        if req.content_markdown.is_empty() {
            req.content_markdown = error.message.clone();
        }
        if req.structured_content.is_null()
            || req
                .structured_content
                .as_object()
                .is_some_and(|map| map.is_empty())
        {
            req.structured_content = json!({
                "code": error.code,
                "retryable": error.retryable,
            });
        }
    }
    let text = if !req.content_markdown.is_empty() {
        req.content_markdown
    } else if !req.structured_content.is_null() {
        req.structured_content.to_string()
    } else {
        String::new()
    };
    let structured = (!req.structured_content.is_null()).then_some(req.structured_content);
    AgentToolResult {
        content: vec![ContentPart::Text(TextContent::plain(text))],
        details: Value::Null,
        structured_content: structured,
        is_error: !req.success,
        terminate: false,
    }
}

fn parse_thinking_level(raw: &str) -> Result<ThinkingLevel, HarnessError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "off" => Ok(ThinkingLevel::Off),
        "disable" | "none" => Ok(ThinkingLevel::None),
        "minimal" => Ok(ThinkingLevel::Minimal),
        "low" => Ok(ThinkingLevel::Low),
        "medium" => Ok(ThinkingLevel::Medium),
        "high" => Ok(ThinkingLevel::High),
        "xhigh" => Ok(ThinkingLevel::Xhigh),
        "max" => Ok(ThinkingLevel::Max),
        other => Err(HarnessError::invalid_config(format!(
            "Unsupported thinking level: {other}"
        ))),
    }
}

fn validate_session_id(id: &str) -> Result<(), HarnessError> {
    if id.is_empty() {
        return Err(HarnessError::invalid_config(
            "resume requires native_session_id",
        ));
    }
    let path = Path::new(id);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(HarnessError::invalid_config("Invalid native_session_id"));
    }
    Ok(())
}

fn session_file_path(working_directory: &Path, native_session_id: &str) -> PathBuf {
    working_directory
        .join(".meerkat/builtin/sessions")
        .join(format!("{native_session_id}.jsonl"))
}

pub(crate) async fn open_session(
    harness: BuiltinHarness,
    request: SessionRequest,
    resume: bool,
) -> Result<Box<dyn HarnessSession>, HarnessError> {
    if request.config.harness_id != HARNESS_ID {
        return Err(HarnessError::invalid_config("Harness identity mismatch"));
    }
    if request.config.model.model_id.trim().is_empty() {
        return Err(HarnessError::invalid_config("model_id is empty"));
    }
    if request.config.config_fingerprint.trim().is_empty() {
        return Err(HarnessError::invalid_config("config_fingerprint is empty"));
    }

    let connection = harness.active_connection()?;
    let opened_auth_revision = harness.auth_revision();
    harness.describe_model(&request.config.model.model_id)?;

    let native_session_id = if resume {
        let id = request.native_session_id.clone().unwrap_or_default();
        validate_session_id(&id)?;
        let path = session_file_path(&request.working_directory, &id);
        if !path.is_file() {
            return Err(HarnessError::new(
                HarnessErrorCode::SessionNotFound,
                format!("Session not found: {id}"),
            ));
        }
        id
    } else {
        uuid::Uuid::new_v4().to_string()
    };

    let skills_dir = request.working_directory.join("skills");
    prepare_native_skills(&request.working_directory, &skills_dir)?;

    let thinking_level = parse_thinking_level(&request.config.thinking_level)?;

    let transport = Arc::new(
        ReqwestTransport::new()
            .map_err(|error| HarnessError::new(HarnessErrorCode::Io, error.to_string()))?,
    );
    let transport_dyn: Arc<dyn HttpTransport> = Arc::clone(&transport) as Arc<dyn HttpTransport>;
    let provider = Arc::new(Client::new(Arc::clone(&transport_dyn)));

    let (model, api_key, get_api_key, max_tokens, headers) = match connection {
        ActiveConnection::ApiKey { base_url, api_key } => {
            let mut model = Model::new(
                request.config.model.model_id.clone(),
                "custom",
                ApiKind::OpenaiCompletions,
                base_url,
            );
            model.name = if request.config.model.display_name.is_empty() {
                request.config.model.model_id.clone()
            } else {
                request.config.model.display_name.clone()
            };
            model.context_window = harness.context_window();
            model.max_output_tokens = MAX_OUTPUT_TOKENS;
            model.supports_images = true;
            model.supports_reasoning = true;
            (
                model,
                Some(api_key),
                None,
                Some(MAX_OUTPUT_TOKENS),
                Vec::new(),
            )
        }
        ActiveConnection::Chatgpt { credentials } => {
            let mut model = Model::new(
                request.config.model.model_id.clone(),
                "openai-codex",
                ApiKind::OpenaiCodexResponses,
                CODEX_DEFAULT_BASE_URL,
            );
            model.name = if request.config.model.display_name.is_empty() {
                request.config.model.model_id.clone()
            } else {
                request.config.model.display_name.clone()
            };
            model.context_window = harness.context_window();
            model.max_output_tokens = MAX_OUTPUT_TOKENS;
            model.supports_images = true;
            model.supports_reasoning = true;

            let mut headers = Vec::new();
            if let Some(account_id) = credentials.account_id.filter(|id| !id.is_empty()) {
                headers.push(("chatgpt-account-id".to_owned(), Some(account_id)));
            }

            let harness_for_key = harness.clone();
            let transport_for_key = Arc::clone(&transport);
            let resolver: ApiKeyResolver = Arc::new(move |_provider| {
                let harness = harness_for_key.clone();
                let transport = Arc::clone(&transport_for_key);
                Box::pin(async move {
                    harness
                        .fresh_codex_credentials(&transport, false)
                        .await
                        .ok()
                        .map(|creds| creds.access_token)
                })
            });
            (model, None, Some(resolver), None, headers)
        }
    };

    let state = Arc::new(SessionState::new(
        native_session_id.clone(),
        request.config.config_fingerprint.clone(),
    ));

    let environment = Arc::new(Mutex::new(request.config.environment.clone()));
    let env_for_tools = Arc::clone(&environment);
    let env_provider: EnvProvider = Arc::new(move || {
        let snapshot: Vec<(String, String)> = env_for_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Box::pin(async move { snapshot })
    });

    let mut registered_names = HashSet::new();
    let mut tools: Vec<KitToolSpec> = Vec::new();
    for tool_def in request.tools {
        registered_names.insert(tool_def.name.clone());
        let bridge: Arc<dyn AgentTool> =
            Arc::new(HostToolBridge::new(tool_def, Arc::clone(&state)));
        tools.push(KitToolSpec::resident(bridge));
    }

    // Build deterministic built-in workspace tools: file/patch/search tools,
    // managed interactive shell (`exec_command` + `write_stdin`), and `web_fetch`.
    let static_tool_set = BuiltinToolSet::all().without(BuiltinToolKind::ExecCommand);
    let ws_config = WorkspaceToolsConfig::new(&request.working_directory)
        .with_enabled_tools(static_tool_set)
        .with_env_provider(env_provider);
    let mut builtin_tools = WorkspaceTools::build(ws_config.clone());

    let processes = ProcessSessionManager::new(Arc::new(ws_config), WakeGate::new(), None);
    builtin_tools.push(Arc::new(ExecCommandTool::new(processes.clone())));
    builtin_tools.push(Arc::new(WriteStdinTool::new(processes.clone())));
    builtin_tools.push(Arc::new(WebFetchTool::new(
        transport_dyn,
        WebFetchConfig::default(),
    )));

    for ws_tool in builtin_tools {
        if registered_names.insert(ws_tool.definition().name) {
            tools.push(KitToolSpec::resident(ws_tool));
        }
    }

    let store_path = session_file_path(&request.working_directory, &native_session_id);
    let store = Arc::new(JsonlSessionStore::new(store_path));

    let skill_discovery = SkillDiscoveryConfig {
        cwd: Some(request.working_directory.clone()),
        walk_boundary: WalkBoundary::CurrentDirOnly,
        project_skill_dirs: vec!["skills".into()],
        include_agents_skills: true,
        include_user_skills: true,
        ..SkillDiscoveryConfig::default()
    };

    let instruction_discovery = InstructionDiscoveryConfig {
        cwd: Some(request.working_directory.clone()),
        walk_boundary: WalkBoundary::CurrentDirOnly,
        include_user_instructions: false,
        include_agents_dir: false,
        ..InstructionDiscoveryConfig::default()
    };

    let requested_system_prompt = request.system_prompt;
    let mut kit_config = KitSessionConfig::new(model)
        .with_system_prompt(requested_system_prompt.clone())
        .with_skill_discovery(skill_discovery)
        .with_auto_reload_skills(true)
        .with_instruction_discovery(instruction_discovery)
        .with_store(store)
        .with_compaction(CompactionConfig {
            enabled: true,
            compaction_reserve_tokens: Some(COMPACTION_RESERVE),
            ..CompactionConfig::default()
        });
    kit_config.enable_deferred_gateway = false;
    kit_config.thinking_level = thinking_level;
    kit_config.tools = tools;
    kit_config.cwd = Some(request.working_directory.display().to_string());
    kit_config.api_key = api_key;
    kit_config.get_api_key = get_api_key;
    kit_config.max_tokens = max_tokens;
    kit_config.headers = headers;

    let kit = KitSession::new(provider, kit_config).map_err(|error| match error {
        KitError::Session(SessionError::InvalidSessionFile(detail)) if resume => HarnessError::new(
            HarnessErrorCode::SessionNotFound,
            format!("Session file is unreadable and requires recovery: {detail}"),
        ),
        other => HarnessError::new(
            HarnessErrorCode::BindFailed,
            format!("Could not start Meerkat session: {other}"),
        ),
    })?;
    if resume {
        if !requested_system_prompt.is_empty() && kit.system_prompt() != requested_system_prompt {
            kit.set_system_prompt(requested_system_prompt)
                .map_err(|error| {
                    HarnessError::new(HarnessErrorCode::InvalidState, error.to_string())
                })?;
        }
        if kit.thinking_level() != thinking_level {
            kit.set_thinking_level(thinking_level).map_err(|error| {
                HarnessError::new(HarnessErrorCode::InvalidState, error.to_string())
            })?;
        }
        kit.reload_instructions();
    }

    Ok(Box::new(BuiltinSession {
        harness,
        kit: Arc::new(kit),
        processes,
        state,
        opened_auth_revision,
        environment,
        tool_lock: tokio::sync::Mutex::new(()),
    }))
}

pub(crate) struct BuiltinSession {
    harness: BuiltinHarness,
    kit: Arc<KitSession>,
    processes: ProcessSessionManager,
    state: Arc<SessionState>,
    opened_auth_revision: u64,
    environment: Arc<Mutex<BTreeMap<String, String>>>,
    tool_lock: tokio::sync::Mutex<()>,
}

impl HarnessSession for BuiltinSession {
    fn entry(&self) -> SessionEntry {
        let mut entry = self.state.entry.clone();
        entry.health = self.state.current_health();
        entry
    }

    fn reusable(&self) -> bool {
        true
    }

    fn rebind<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, ()> {
        Box::pin(async move {
            let active = self
                .state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            if self.state.current_health() != SessionHealth::Healthy
                || active
                || self.kit.is_running()
                || self.harness.auth_revision() != self.opened_auth_revision
            {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "Session is not reusable",
                ));
            }
            *self.environment.lock().unwrap_or_else(|e| e.into_inner()) =
                request.config.environment;
            prepare_native_skills(
                &request.working_directory,
                &request.working_directory.join("skills"),
            )?;
            self.kit.reload_skills();
            self.kit.reload_instructions();
            self.harness
                .describe_model(&request.config.model.model_id)?;
            if !request.system_prompt.is_empty()
                && self.kit.system_prompt() != request.system_prompt
            {
                self.kit
                    .set_system_prompt(request.system_prompt)
                    .map_err(|error| {
                        HarnessError::new(HarnessErrorCode::InvalidState, error.to_string())
                    })?;
            }
            let level = parse_thinking_level(&request.config.thinking_level)?;
            if self.kit.thinking_level() != level {
                self.kit.set_thinking_level(level).map_err(|error| {
                    HarnessError::new(HarnessErrorCode::InvalidState, error.to_string())
                })?;
            }
            Ok(())
        })
    }

    fn process_group_id(&self) -> Option<u32> {
        None
    }

    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async move {
            if self.state.current_health() == SessionHealth::Closed {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "Session is closed",
                ));
            }
            let (tx, rx) = mpsc::unbounded_channel();
            {
                let mut active = self.state.active.lock().unwrap_or_else(|e| e.into_inner());
                if active.is_some() || self.kit.is_running() {
                    return Err(HarnessError::new(
                        HarnessErrorCode::InvalidState,
                        "Execution already active",
                    ));
                }
                self.state
                    .cancel_reason
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                self.state
                    .pending_tools
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                self.state
                    .applied_tool_results
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                self.kit.drain_steering();
                *active = Some(ActiveTurn {
                    sender: tx,
                    sequence: 0,
                });
            }

            self.state.emit(AgentEventKind::Started);
            self.state.emit(AgentEventKind::NativeSessionBound {
                native_session_id: self.state.entry.native_session_id.clone(),
            });

            let kit = Arc::clone(&self.kit);
            let state = Arc::clone(&self.state);
            let input = request.input;

            tokio::spawn(async move {
                drive_turn(kit, state, input).await;
            });

            Ok(Box::pin(UnboundedReceiverStream::new(rx)) as AgentEventStream)
        })
    }

    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async move {
            let active_guard = self.state.active.lock().unwrap_or_else(|e| e.into_inner());
            if active_guard.is_none() {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "No turn is running",
                ));
            }
            // Enqueue while holding `active_guard` so `drive_turn` cannot settle the turn
            // without either consuming or draining this delivery.
            self.kit.steer(delivery_id.clone(), input);
            drop(active_guard);
            Ok(SteerReceipt {
                delivery_id,
                accepted: true,
            })
        })
    }

    fn tool_result<'a>(
        &'a self,
        request: ToolResultRequest,
    ) -> HarnessFuture<'a, ToolResultReceipt> {
        Box::pin(async move {
            let _guard = self.tool_lock.lock().await;
            if self
                .state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none()
            {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "Tool result requires active execution",
                ));
            }
            let value = json!(request);
            let previous = self
                .state
                .applied_tool_results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&request.tool_call_id)
                .cloned();
            if let Some(previous) = previous {
                if previous != value {
                    return Err(HarnessError::new(
                        HarnessErrorCode::InvalidState,
                        "Conflicting tool result",
                    ));
                }
                return Ok(ToolResultReceipt {
                    tool_call_id: request.tool_call_id,
                    already_applied: true,
                });
            }
            let sender = self
                .state
                .pending_tools
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&request.tool_call_id)
                .ok_or_else(|| {
                    HarnessError::new(HarnessErrorCode::InvalidState, "Tool call not observed")
                })?;
            let tool_call_id = request.tool_call_id.clone();
            let success = request.success;
            sender.send(request).map_err(|_| {
                HarnessError::new(
                    HarnessErrorCode::StreamInterrupted,
                    "Tool execution already closed",
                )
            })?;
            self.state
                .applied_tool_results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(tool_call_id.clone(), value.clone());
            self.state.emit(AgentEventKind::ToolFinished {
                tool_call_id: tool_call_id.clone(),
                success,
                output: value,
            });
            Ok(ToolResultReceipt {
                tool_call_id,
                already_applied: false,
            })
        })
    }

    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async move {
            {
                let mut cancel_reason = self
                    .state
                    .cancel_reason
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if cancel_reason.is_none() {
                    *cancel_reason = Some(reason);
                }
            }
            self.kit.abort().await;
            self.state.set_health(SessionHealth::Cancelled);
            Ok(CancelReceipt { accepted: true })
        })
    }

    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            let closed = self.state.current_health() == SessionHealth::Closed;
            if !closed {
                let was_active = self
                    .state
                    .active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some();
                if was_active {
                    {
                        let mut cancel_reason = self
                            .state
                            .cancel_reason
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if cancel_reason.is_none() {
                            *cancel_reason = Some("Session closed".into());
                        }
                    }
                    self.kit.abort().await;
                }
                self.processes.shutdown().await;
                self.state
                    .pending_tools
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                self.state.set_health(SessionHealth::Closed);
            }
            Ok(CloseReceipt {
                already_closed: closed,
            })
        })
    }
}

fn extract_steering_text(items: Vec<(String, Vec<ContentPart>)>) -> String {
    items
        .into_iter()
        .filter_map(|(_, parts)| {
            let text: String = parts
                .into_iter()
                .filter_map(|p| match p {
                    ContentPart::Text(t) => Some(t.text),
                    ContentPart::Image(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

async fn drive_turn(kit: Arc<KitSession>, state: Arc<SessionState>, initial_input: String) {
    let mut next_input = initial_input;
    let mut last_assistant: Option<AssistantMessage> = None;

    loop {
        if let Some(reason) = state
            .cancel_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            kit.drain_steering();
            state.finish_turn(AgentEventKind::Cancelled { reason });
            return;
        }

        let mut stream = match kit.prompt_stream(next_input) {
            Ok(stream) => stream,
            Err(error) => {
                kit.drain_steering();
                let message = error.to_string();
                let failure = classify_agent_failure("agent_error", &message);
                state.finish_turn(AgentEventKind::Failed {
                    code: "agent_error".into(),
                    message,
                    failure: Some(failure),
                });
                return;
            }
        };

        // Close the registration race: if `cancel()` or `close()` arrived right before `prompt_stream`
        // registered `active_run`, abort immediately now that `active_run` is live.
        if state
            .cancel_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            kit.abort().await;
        }

        let mut saw_agent_end = false;
        let mut stream_error: Option<String> = None;

        while let Some(event) = stream.next().await {
            match event {
                KitEvent::Error(err) => {
                    stream_error = Some(err);
                }
                KitEvent::Session(_) => {}
                KitEvent::Agent(agent_event) => match agent_event {
                    JimuAgentEvent::MessageUpdate {
                        assistant_message_event,
                        ..
                    } => match assistant_message_event {
                        AssistantMessageEvent::TextDelta { delta, .. } if !delta.is_empty() => {
                            state.emit(AgentEventKind::AssistantDelta { text: delta });
                        }
                        AssistantMessageEvent::ThinkingDelta { delta, .. } if !delta.is_empty() => {
                            state.emit(AgentEventKind::ReasoningDelta { text: delta });
                        }
                        _ => {}
                    },
                    JimuAgentEvent::RequestUsage { usage, .. } => {
                        let input_tokens = usage
                            .input
                            .saturating_add(usage.cache_read)
                            .saturating_add(usage.cache_write);
                        let output_tokens = usage.output;
                        if input_tokens > 0 || output_tokens > 0 {
                            state.emit(AgentEventKind::Usage {
                                input_tokens,
                                output_tokens,
                            });
                        }
                    }
                    JimuAgentEvent::TurnEnd { message, .. } => {
                        last_assistant = Some(message);
                    }
                    JimuAgentEvent::AgentEnd { .. } => {
                        saw_agent_end = true;
                    }
                    _ => {}
                },
            }
        }

        let cancel_reason = state
            .cancel_reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();

        if let Some(reason) = cancel_reason {
            kit.drain_steering();
            state.finish_turn(AgentEventKind::Cancelled { reason });
            return;
        }

        if let Some(err_msg) = stream_error {
            kit.drain_steering();
            let failure = classify_agent_failure("stream_interrupted", &err_msg);
            state.finish_turn(AgentEventKind::Failed {
                code: "stream_interrupted".into(),
                message: err_msg,
                failure: Some(failure),
            });
            return;
        }

        if !saw_agent_end {
            kit.drain_steering();
            let message = "Agent turn ended before completion".to_string();
            let failure = classify_agent_failure("stream_interrupted", &message);
            state.finish_turn(AgentEventKind::Failed {
                code: "stream_interrupted".into(),
                message,
                failure: Some(failure),
            });
            return;
        }

        if let Some(ref msg) = last_assistant {
            match msg.stop_reason {
                StopReason::Aborted => {
                    kit.drain_steering();
                    state.finish_turn(AgentEventKind::Cancelled {
                        reason: "Turn cancelled".into(),
                    });
                    return;
                }
                StopReason::Error => {
                    kit.drain_steering();
                    let message = msg
                        .error_message
                        .clone()
                        .unwrap_or_else(|| "Agent request failed".into());
                    let failure = classify_agent_failure("agent_error", &message);
                    state.finish_turn(AgentEventKind::Failed {
                        code: "agent_error".into(),
                        message,
                        failure: Some(failure),
                    });
                    return;
                }
                _ => {}
            }
        }

        // Atomically check under `state.active` if any `steer()` arrived after `jimu-agent`'s
        // final steering check. If so, run another prompt iteration in the same active turn;
        // otherwise settle the turn while still holding `state.active` so no `steer()` is ever dropped.
        let maybe_followup_steer = {
            let mut active = state.active.lock().unwrap_or_else(|e| e.into_inner());
            let late_steers = kit.drain_steering();
            let combined = extract_steering_text(late_steers);
            if !combined.is_empty() {
                Some(combined)
            } else {
                let final_output = last_assistant
                    .as_ref()
                    .map(AssistantMessage::text_output)
                    .unwrap_or_default();
                let terminal = AgentEventKind::Completed { final_output };
                if let Some(turn) = active.as_mut() {
                    turn.sequence += 1;
                    let _ = turn.sender.send(Ok(AgentEvent {
                        sequence: turn.sequence,
                        kind: terminal,
                    }));
                }
                active.take();
                None
            }
        };

        if let Some(steered_input) = maybe_followup_steer {
            next_input = steered_input;
            continue;
        }

        state
            .pending_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        state.set_health(SessionHealth::Healthy);
        return;
    }
}
