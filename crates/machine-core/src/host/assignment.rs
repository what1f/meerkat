use super::connection::send_frame;
use super::control::replay_assignment_pending;
use super::events::run_events;
use super::proxy::download_resource;
use super::{ActiveExecution, Inner, MachineError, StartingExecution};
use crate::resources::{agent_directory, replace_resource_links};
use crate::state::{
    AssignmentLifecycle, PersistedAssignment, PersistedSession, SessionRecovery, assignment_key,
};
use meerkat_harness::{
    ExecuteRequest, HarnessConfig as NativeHarnessConfig, HarnessErrorCode, HarnessSession,
    ModelRef, SessionRequest, ToolDefinition as NativeToolDefinition,
};
use meerkat_protocol::{
    AgentRunContext, AssignExecution, AssignmentResult, DeliveryResult, ExecutionRef, MachineFrame,
    SessionAction, machine_frame,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

pub(super) async fn handle_delete_agent_files(
    inner: &Inner,
    request: meerkat_protocol::DeleteAgentFiles,
) {
    let result = async {
        let agent = request
            .agent
            .as_ref()
            .ok_or_else(|| MachineError::InvalidAssignment("missing_agent".into()))?;
        let key = format!("{}/{}", agent.workspace_id, agent.agent_id);
        if let Some(starting) = inner.starting.lock().await.get(&key) {
            let _ = starting.cancel.send(Some("Agent was deleted".into()));
        }
        let cancellations = inner
            .active
            .lock()
            .await
            .values()
            .filter(|active| active.agent_key == key)
            .map(|active| {
                active.cancel.send_replace(Some(super::Cancellation {
                    reason: "Agent was deleted".into(),
                    force_close: false,
                }));
                active.terminated.clone()
            })
            .collect::<Vec<_>>();
        for terminated in cancellations {
            super::liveness::wait_for_stop(terminated, inner.liveness.cancel_grace).await;
        }
        super::sessions::close(inner, &key).await;
        super::relocation::release(&inner.root, agent)?;
        let directory = agent_directory(&inner.root, &agent.workspace_id, &agent.agent_id)?;
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(MachineError::Io(error)),
        }
    }
    .await;
    let (success, error_code, error_message) = match result {
        Ok(()) => (true, String::new(), String::new()),
        Err(error) => (false, error_code(&error).into(), public_error(&error)),
    };
    send_frame(
        inner,
        MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::DeleteAgentFilesResult(
                meerkat_protocol::DeleteAgentFilesResult {
                    request_id: request.request_id,
                    success,
                    error_code,
                    error_message,
                },
            )),
        },
    )
    .await;
}

pub(super) async fn handle_assignment(inner: Arc<Inner>, assign: AssignExecution) {
    let execution = match assign.execution.clone() {
        Some(value) if value.execution_id > 0 && value.attempt > 0 => value,
        _ => return,
    };
    let key = assignment_key(execution.execution_id, execution.attempt);
    let agent_key = assign
        .workspace
        .as_ref()
        .zip(assign.agent.as_ref())
        .and_then(|(workspace, profile)| profile.agent.as_ref().map(|agent| (workspace, agent)))
        .filter(|(workspace, agent)| agent.workspace_id == workspace.workspace_id)
        .map(|(workspace, agent)| format!("{}/{}", workspace.workspace_id, agent.agent_id));
    let remembered = inner
        .state
        .lock()
        .await
        .accepted_assignments
        .contains_key(&key);
    let failure_agent_id = assign
        .agent
        .as_ref()
        .and_then(|profile| profile.agent.as_ref())
        .map(|agent| agent.agent_id.clone())
        .unwrap_or_default();
    let mut cancel = None;
    let result = if let Some(agent_key) = agent_key.as_ref().filter(|_| !remembered) {
        let (cancel_tx, mut cancel_rx) = watch::channel(None);
        let mut starting = inner.starting.lock().await;
        let active = inner.active.lock().await;
        if starting.contains_key(agent_key)
            || active.values().any(|value| &value.agent_key == agent_key)
        {
            Err(MachineError::InvalidAssignment("agent_busy".into()))
        } else {
            starting.insert(
                agent_key.clone(),
                StartingExecution {
                    assignment_key: key.clone(),
                    cancel: cancel_tx,
                },
            );
            drop(active);
            drop(starting);
            cancel = Some(agent_key.clone());
            tokio::select! {
                biased;
                changed = cancel_rx.changed() => {
                    let _ = changed;
                    Err(MachineError::InvalidAssignment("assignment_cancelled".into()))
                }
                result = prepare_assignment(inner.clone(), assign, execution.clone(), key.clone()) => result,
            }
        }
    } else {
        prepare_assignment(inner.clone(), assign, execution.clone(), key.clone()).await
    };
    if let Some(agent_key) = cancel {
        let mut starting = inner.starting.lock().await;
        if starting
            .get(&agent_key)
            .is_some_and(|value| value.assignment_key == key)
        {
            starting.remove(&agent_key);
        }
    }
    if let Err(error) = result {
        let failure = match &error {
            MachineError::Harness(error) => super::failure::attempt_failure(&error.failure),
            _ => {
                use meerkat_harness::{FailureCategory, HarnessFailure};
                let category = match &error {
                    MachineError::InvalidAssignment(code) if code == "assignment_cancelled" => {
                        FailureCategory::Cancelled
                    }
                    MachineError::InvalidAssignment(code) if code == "agent_busy" => {
                        FailureCategory::Conflict
                    }
                    MachineError::Io(_)
                    | MachineError::Transport(_)
                    | MachineError::Rpc(_)
                    | MachineError::Resource(_)
                    | MachineError::ResourceIntegrityMismatch => FailureCategory::Transient,
                    _ => FailureCategory::Configuration,
                };
                super::failure::attempt_failure(&HarnessFailure {
                    code: error_code(&error).into(),
                    category,
                    retryable: matches!(
                        category,
                        FailureCategory::Transient | FailureCategory::Conflict
                    ),
                    detail: error.to_string(),
                })
            }
        };
        super::failure::log_failure(&inner, &execution, &failure_agent_id, &failure);
        send_frame(
            &inner,
            MachineFrame {
                protocol_version: meerkat_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::AssignmentResult(AssignmentResult {
                    failure: Some(failure),
                    execution: Some(execution),
                    accepted: false,
                    error_code: error_code(&error).into(),
                    error_message: public_error(&error),
                })),
            },
        )
        .await;
    }
}

async fn prepare_assignment(
    inner: Arc<Inner>,
    assign: AssignExecution,
    execution: ExecutionRef,
    key: String,
) -> Result<(), MachineError> {
    let fingerprint = assignment_fingerprint(&assign);
    let remembered_assignment = inner
        .state
        .lock()
        .await
        .accepted_assignments
        .get(&key)
        .cloned();
    if let Some(remembered) = remembered_assignment {
        if remembered.fingerprint != fingerprint {
            return Err(MachineError::InvalidAssignment(
                "assignment_conflict".into(),
            ));
        }
        send_frame(
            &inner,
            MachineFrame {
                protocol_version: meerkat_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::AssignmentResult(AssignmentResult {
                    failure: None,
                    execution: Some(execution.clone()),
                    accepted: true,
                    error_code: String::new(),
                    error_message: String::new(),
                })),
            },
        )
        .await;
        send_frame(
            &inner,
            MachineFrame {
                protocol_version: meerkat_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::DeliveryResult(remembered.delivery)),
            },
        )
        .await;
        replay_assignment_pending(&inner, &key).await;
        return Ok(());
    }
    let workspace = assign
        .workspace
        .as_ref()
        .ok_or_else(|| MachineError::InvalidAssignment("missing_workspace".into()))?;
    let profile = assign
        .agent
        .as_ref()
        .ok_or_else(|| MachineError::InvalidAssignment("missing_agent".into()))?;
    let agent = profile
        .agent
        .as_ref()
        .ok_or_else(|| MachineError::InvalidAssignment("missing_agent_ref".into()))?;
    if agent.workspace_id != workspace.workspace_id {
        return Err(MachineError::InvalidAssignment(
            "agent_workspace_mismatch".into(),
        ));
    }
    let agent_key = format!("{}/{}", workspace.workspace_id, agent.agent_id);
    {
        let active = inner.active.lock().await;
        if active.values().any(|value| value.agent_key == agent_key) {
            return Err(MachineError::InvalidAssignment("agent_busy".into()));
        }
    }
    let wire_config = assign
        .harness
        .as_ref()
        .ok_or_else(|| MachineError::InvalidAssignment("missing_harness".into()))?;
    let factory = inner
        .factories
        .get(&wire_config.harness_id)
        .cloned()
        .ok_or_else(|| MachineError::InvalidAssignment("harness_unavailable".into()))?;
    let directive = assign
        .session
        .as_ref()
        .ok_or_else(|| MachineError::InvalidAssignment("missing_session".into()))?;
    let delivery = assign
        .initial_delivery
        .as_ref()
        .ok_or_else(|| MachineError::InvalidAssignment("missing_delivery".into()))?;
    let agent_home = agent_directory(&inner.root, &workspace.workspace_id, &agent.agent_id)?;
    std::fs::create_dir_all(&agent_home)?;
    let agent_home = agent_home.canonicalize()?;
    // Publish the configured personality verbatim before the Harness loads
    // workspace instructions. Atomic replacement never follows a file symlink.
    {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new_in(&agent_home)?;
        file.write_all(profile.soul.as_bytes())?;
        file.persist(agent_home.join("AGENTS.md"))
            .map_err(|error| MachineError::Io(error.error))?;
    }
    let context = AgentRunContext {
        execution: Some(execution.clone()),
        execution_token: assign.execution_token.clone(),
    };
    let mut local_resources = Vec::new();
    for resource in assign.resources.iter().chain(delivery.resources.iter()) {
        if local_resources
            .iter()
            .any(|(id, _)| *id == resource.resource_id)
        {
            continue;
        }
        let path = download_resource(
            inner.clone(),
            resource,
            context.clone(),
            &agent_home.join("resources"),
        )
        .await?;
        local_resources.push((resource.resource_id, path));
    }
    let mut input = replace_resource_links(&delivery.rendered_markdown, &local_resources);
    if let Some(file) = input.strip_prefix("content_file:") {
        let path = crate::resolve_content_file(&agent_home, file)?;
        input = std::fs::read_to_string(path)?;
    }
    let default_config = inner
        .defaults
        .get(&wire_config.harness_id)
        .ok_or_else(|| MachineError::InvalidAssignment("harness_unavailable".into()))?;
    let mut native_config =
        native_config(wire_config, &directive.config_fingerprint, default_config)?;
    super::records::prepare(
        &inner.root,
        &inner.output_records,
        &execution,
        &mut native_config,
    )?;
    let proxy =
        super::proxy::AttemptProxy::start(inner.clone(), context.clone(), agent_home.clone())?;
    native_config
        .environment
        .retain(|key, _| !key.starts_with("MEERKAT_RUNTIME_"));
    // Execution credentials belong to this attempt, never to editable Agent configuration.
    native_config.environment.extend([
        (
            "MEERKAT_AGENT_SOCKET".into(),
            proxy.path.display().to_string(),
        ),
        (
            "MEERKAT_RUN_CONTEXT".into(),
            proxy.context_path.display().to_string(),
        ),
        (
            "MEERKAT_EXECUTION_ID".into(),
            execution.execution_id.to_string(),
        ),
        (
            "MEERKAT_EXECUTION_ATTEMPT".into(),
            execution.attempt.to_string(),
        ),
        (
            "MEERKAT_EXECUTION_TOKEN".into(),
            assign.execution_token.clone(),
        ),
        (
            "MEERKAT_AGENT_HOME".into(),
            agent_home.display().to_string(),
        ),
    ]);
    let mut system_prompt = assign.system_prompt.clone();
    crate::capability_cli::inject(
        inner.capability_cli.as_deref(),
        &inner.root.join(".machine/bin"),
        &agent_home,
        &mut native_config,
        &assign.cli_commands,
        &mut system_prompt,
    )?;
    let tools = assign
        .tools
        .iter()
        .map(|tool| {
            Ok(NativeToolDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: serde_json::from_str(&tool.input_schema_json)
                    .map_err(|_| MachineError::InvalidAssignment("invalid_tool_schema".into()))?,
                output_schema: serde_json::from_str(&tool.output_schema_json)
                    .map_err(|_| MachineError::InvalidAssignment("invalid_tool_schema".into()))?,
            })
        })
        .collect::<Result<Vec<_>, MachineError>>()?;
    let session_key = format!("{agent_key}/{}", directive.logical_session_id);
    let remembered = inner.state.lock().await.sessions.get(&session_key).cloned();
    let requested_native = directive.native_session_id.clone().or_else(|| {
        remembered
            .as_ref()
            .filter(|entry| {
                entry.harness_id == wire_config.harness_id
                    && entry.config_fingerprint == directive.config_fingerprint
            })
            .map(|entry| entry.native_session_id.clone())
    });
    // Probe the executable with its plain defaults, not this Agent's setup.
    let plain = NativeHarnessConfig {
        cli_path: native_config.cli_path.clone(),
        ..default_config.clone()
    };
    let capabilities = factory.capabilities(&plain);
    let cli_version = inner.cli_versions.version(factory.as_ref(), &plain).await;
    let resume = SessionAction::try_from(directive.action).ok()
        == Some(SessionAction::ResumeOrStart)
        && requested_native.is_some()
        && capabilities.contains(&meerkat_harness::HarnessCapability::Resume);
    let request = SessionRequest {
        cli_version,
        permission_policy: Default::default(),
        config: native_config,
        working_directory: agent_home.clone(),
        system_prompt,
        tools,
        native_session_id: if resume {
            requested_native.clone()
        } else {
            None
        },
    };
    let identity = super::sessions::identity(&request, &session_key, &profile.soul);
    let cached = if resume {
        super::sessions::take(
            &inner,
            &agent_key,
            &identity,
            requested_native.as_deref(),
            &request,
        )
        .await
    } else {
        super::sessions::close(&inner, &agent_key).await;
        None
    };
    let reused = cached.is_some();
    let mut context_lost = false;
    let session: Arc<dyn HarnessSession> = if let Some(session) = cached {
        session
    } else if resume {
        match factory.resume(request.clone()).await {
            Ok(session) => session.into(),
            Err(error) if error.code == HarnessErrorCode::SessionNotFound => {
                context_lost = true;
                factory
                    .start(SessionRequest {
                        native_session_id: None,
                        ..request
                    })
                    .await?
                    .into()
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        factory.start(request).await?.into()
    };
    let entry = session.entry();
    // A resumed session already carries this context; a fresh one (first
    // start, Runtime-requested new session, or a lost native session) gets it
    // ahead of the delivery.
    let fresh = context_lost || (!reused && !resume);
    if fresh {
        input = format!(
            "{}{input}",
            fresh_session_context(&agent_home, &assign.recovery_markdown)?
        );
    }
    let stream = match session.execute(ExecuteRequest { input }).await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = session.close().await;
            return Err(error.into());
        }
    };
    {
        let mut state = inner.state.lock().await;
        state.accepted_assignments.insert(
            key.clone(),
            PersistedAssignment {
                fingerprint,
                delivery: DeliveryResult {
                    execution: Some(execution.clone()),
                    delivery_id: delivery.delivery_id,
                    accepted: true,
                    error_code: String::new(),
                    error_message: String::new(),
                },
                lifecycle: AssignmentLifecycle::Active,
            },
        );
        state.sessions.insert(
            session_key.clone(),
            PersistedSession {
                harness_id: entry.harness_id,
                native_session_id: entry.native_session_id,
                config_fingerprint: entry.config_fingerprint,
                recovery: SessionRecovery::Interrupted,
            },
        );
        state.save(&inner.state_path)?;
    }
    let (cancel_tx, cancel_rx) = watch::channel(None);
    let started = tokio::time::Instant::now();
    let (progress_tx, progress_rx) = watch::channel(started);
    let (terminated_tx, terminated_rx) = watch::channel(None);
    let delivery_lock = Arc::new(tokio::sync::Mutex::new(()));
    let can_cancel = capabilities.contains(&meerkat_harness::HarnessCapability::Cancel);
    inner.active.lock().await.insert(
        key.clone(),
        ActiveExecution {
            terminated: terminated_rx.clone(),
            cancel: cancel_tx.clone(),
            progress: progress_tx,
            agent_key,
            session_identity: identity,
            capabilities,
            session: session.clone(),
            agent_home: agent_home.clone(),
            context: context.clone(),
            delivery_lock: delivery_lock.clone(),
            accepting_deliveries: true,
            accepting_tool_results: true,
            last_event_sequence: 0,
        },
    );
    tokio::spawn(super::liveness::monitor(
        inner.liveness.clone(),
        started,
        session.clone(),
        can_cancel,
        progress_rx,
        cancel_rx,
        cancel_tx,
        terminated_tx,
    ));
    send_frame(
        &inner,
        MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::AssignmentResult(AssignmentResult {
                failure: None,
                execution: Some(execution.clone()),
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        },
    )
    .await;
    send_frame(
        &inner,
        MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::DeliveryResult(DeliveryResult {
                execution: Some(execution.clone()),
                delivery_id: delivery.delivery_id,
                accepted: true,
                error_code: String::new(),
                error_message: String::new(),
            })),
        },
    )
    .await;
    // Shutdown waits for this task, which saves the terminal state last.
    let events = tokio::spawn(run_events(
        inner.clone(),
        key,
        session_key,
        execution,
        context,
        agent_home,
        session,
        stream,
        terminated_rx,
        delivery_lock,
        proxy,
    ));
    let mut tasks = inner.assignment_tasks.lock().await;
    tasks.retain(|task| !task.is_finished());
    tasks.push(events);
    Ok(())
}

pub(super) fn native_config(
    config: &meerkat_protocol::HarnessConfig,
    fingerprint: &str,
    defaults: &NativeHarnessConfig,
) -> Result<NativeHarnessConfig, MachineError> {
    if Path::new(&config.executable_path)
        .file_name()
        .and_then(|v| v.to_str())
        == Some("dcc")
        && config.harness_id != "claude_code"
    {
        return Err(MachineError::InvalidAssignment(
            "dcc_requires_claude_code".into(),
        ));
    }
    let mut environment = defaults.environment.clone();
    environment.extend(
        config
            .environment_variables
            .iter()
            .map(|entry| (entry.name.clone(), entry.value.clone())),
    );
    let result = NativeHarnessConfig {
        harness_id: config.harness_id.clone(),
        model: ModelRef::new(config.model.clone(), config.model.clone()),
        thinking_level: config.thinking_level.clone(),
        launch_arguments: config.launch_arguments.clone(),
        cli_path: if config.executable_path.trim().is_empty() {
            defaults.cli_path.clone()
        } else {
            PathBuf::from(&config.executable_path)
        },
        environment,
        config_fingerprint: fingerprint.to_owned(),
    };
    result.validate()?;
    Ok(result)
}

fn assignment_fingerprint(assign: &AssignExecution) -> String {
    let mut stable = assign.clone();
    stable.execution_token.clear();
    let digest = Sha256::digest(serde_json::to_vec(&stable).expect("assignment JSON"));
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn error_code(error: &MachineError) -> &'static str {
    match error {
        MachineError::InvalidAssignment(value) if value == "agent_busy" => "agent_busy",
        MachineError::InvalidAssignment(value) if value == "duplicate_assignment" => {
            "duplicate_assignment"
        }
        MachineError::InvalidAssignment(value) if value == "assignment_conflict" => {
            "assignment_conflict"
        }
        MachineError::ResourceIntegrityMismatch => "resource_integrity_mismatch",
        MachineError::Resource(_) => "resource_error",
        MachineError::PathOutsideAgentHome => "path_outside_agent_home",
        MachineError::Harness(_) => "harness_error",
        _ => "invalid_assignment",
    }
}

pub(super) fn public_error(error: &MachineError) -> String {
    match error {
        MachineError::InvalidAssignment(value) => value.clone(),
        MachineError::PathOutsideAgentHome => "Content file is outside the Agent directory".into(),
        MachineError::Resource(_) | MachineError::ResourceIntegrityMismatch => {
            "A required resource could not be prepared".into()
        }
        MachineError::Harness(error) => error.failure.detail.clone(),
        _ => "The execution could not be prepared".into(),
    }
}

/// Upper bound for the memory index placed into a fresh session.
const MEMORY_INDEX_MAX_BYTES: usize = 12 * 1024;

/// Memory index head plus Runtime-built recovery context, each ending with a
/// blank line so the delivery follows as the last section.
fn fresh_session_context(agent_home: &Path, recovery: &str) -> Result<String, MachineError> {
    let mut context = String::new();
    if let Some(index) = memory_index_block(agent_home)? {
        context.push_str(&index);
        context.push_str("\n\n");
    }
    let recovery = recovery.trim();
    if !recovery.is_empty() {
        context.push_str(recovery);
        context.push_str("\n\n");
    }
    Ok(context)
}

fn memory_index_block(agent_home: &Path) -> Result<Option<String>, MachineError> {
    use std::io::Read;
    let index = agent_home.join("memory/MEMORY.md");
    if !index.try_exists()? {
        return Ok(None);
    }
    let index = crate::resolve_content_file(agent_home, "memory/MEMORY.md")?;
    let mut bytes = Vec::new();
    std::fs::File::open(index)?
        .take((MEMORY_INDEX_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MEMORY_INDEX_MAX_BYTES;
    bytes.truncate(MEMORY_INDEX_MAX_BYTES);
    // Cut on a UTF-8 boundary rather than introducing a broken trailing character.
    while std::str::from_utf8(&bytes).is_err_and(|error| error.error_len().is_none()) {
        bytes.pop();
    }
    let mut index = std::str::from_utf8(&bytes)
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "memory index must be valid UTF-8",
            )
        })?
        .trim_end();
    if truncated && let Some(line_end) = index.rfind('\n') {
        index = &index[..line_end];
    }
    if index.trim().is_empty() {
        return Ok(None);
    }
    let attributes = if truncated {
        r#" shown="head" note="索引超过预载上限，下次空闲时把细节移到分题文件；其余内容请直接读取文件""#
    } else {
        r#" complete="true""#
    };
    Ok(Some(format!(
        "<memory-index file=\"memory/MEMORY.md\"{attributes}>\n{}\n</memory-index>",
        index.trim()
    )))
}
