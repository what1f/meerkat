use lankide_core::{
    AgentId, AttentionLevel, ChannelAbility, ChannelId, MessageId, PersonId, ProviderId,
    ScheduleInterval, WorkspaceId,
};
use lankide_protocol::{
    ActiveAttempt, AgentAttention, AgentAvatarUpload, AgentRef, AgentRunContext,
    AgentRuntimeProfile, AssignExecution, AssignmentResult, AttentionMode, CapabilityInvokeRequest,
    ChannelAttention, ComputerFrame, ComputerHeartbeat, ComputerHello, ContentKind, ContentRequest,
    CreateAgentRequest, CreateWorkspaceRequest, DEFAULT_MAX_MESSAGE_BYTES, DeleteAgentRequest,
    Delivery, DeliveryKind, DeliveryResult, ExecutionEvent, ExecutionEventKind, ExecutionRef,
    HarnessConfig, ListWorkspacesRequest, ManagementSubscription, PROTOCOL_VERSION, ResourceChunk,
    ResourceDescriptor, ResourceRequest, SendMessageRequest, SessionAction, SessionDirective,
    TransportConfig, UpdateAgentRequest, UpdateSelfProfileRequest, UpdateWorkspaceRequest,
    WorkspaceRef, computer_frame, connect_uds, runtime_frame, serve_uds,
};
use lankide_provider::{
    AgentContext, AgentRuntimeContext, AutoReactionContext, ChannelDefaultAttention,
    ChannelDefinition, EditMessageContext, EventSender, EventSenderType, EventSink, HostServices,
    InitContext, InterfaceDefinition, InvocationHandler, JsonSchema, LogLevel, MessageEvent,
    OperationProgress, OperationUpdate, Provider, ProviderError, ProviderEvent, ProviderEventKind,
    ProviderFuture, ProviderInfo, ProviderLogger, ProviderStorage, ReactionAction, ReactionEvent,
    RecallMessageContext, ResourceSaver, ResourceUpload, SecretReader, SecretValue,
    SendMessageContext, SendMessageResult, ToolDefinition, WorkspaceContext,
};
use lankide_runtime::{
    AgentRuntimeConfig, AttentionPolicy, ComputerCompletion, ComputerError, ComputerExecutor,
    ComputerFuture, ComputerOutcome, ExecutionProgress, ExecutionReply, ProviderFactoryRegistry,
    ProviderTypeFactory, Runtime, RuntimeApplication, RuntimeConfig, RuntimePeerAdapter,
};
use lankide_storage::{
    AgentSpec, AttemptAuthorization, AttemptCapability, AttentionRecord, DatabaseConfig,
    DispatchReason, ExecutionAuthSpec, InboxDispatch, InboxState, IngestEvent, MessageInput,
    Storage, TeamRole, ToolInvocationRequest, WorkspaceRecord, WorkspaceSpec,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_e2e() {
    let run_id = format!(
        "runtime-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let temp = tempfile::Builder::new()
        .prefix(&run_id)
        .tempdir()
        .expect("temp root");
    let db = temp.path().join("runtime.db");
    let application_db = temp.path().join("application.db");
    let attempt_db = temp.path().join("attempt.db");
    let pause_db = temp.path().join("pause.db");
    let collaboration_db = temp.path().join("collaboration.db");
    let socket = temp.path().join("runtime.sock");
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        scenario(&db).await?;
        attempt_recovery_scenario(&attempt_db).await?;
        pause_drain_scenario(&pause_db).await?;
        collaboration_fencing_scenario(&collaboration_db).await?;
        application_scenario(&application_db, &socket).await?;
        clearing_environment_scenario().await;
        local_agent_settings_scenario().await;
        agent_avatar_scenario().await;
        failed_agent_update_scenario().await;
        prompt_failure_recovery_scenario().await;
        durable_ingress_retry_scenario().await;
        configured_assignment_scenario().await;
        Ok(())
    })
    .await;
    let scenario_result = match result {
        Ok(result) => result,
        Err(_) => Err("hard_timeout".to_owned()),
    };
    let cleanup = temp.close().map(|_| "removed").unwrap_or("failed");
    let artifact_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e-artifacts/runtime")
        .join(&run_id);
    fs::create_dir_all(&artifact_dir).expect("artifact directory");
    let artifact = artifact_dir.join("result.json");
    fs::write(
        &artifact,
        serde_json::to_vec_pretty(&json!({
            "run_id": run_id,
            "success": scenario_result.is_ok(),
            "error": scenario_result.as_ref().err(),
            "cleanup": cleanup,
        }))
        .expect("result json"),
    )
    .expect("write result");
    assert_eq!(
        cleanup,
        "removed",
        "cleanup failed; artifact={}",
        artifact.display()
    );
    assert!(
        scenario_result.is_ok(),
        "{}; artifact={}",
        scenario_result.unwrap_err(),
        artifact.display()
    );
}

async fn scenario(db: &Path) -> Result<(), String> {
    let provider_id = ProviderId::new("fixture").map_err(err)?;
    let workspace_a = WorkspaceId::new("workspace_a").map_err(err)?;
    let workspace_b = WorkspaceId::new("workspace_b").map_err(err)?;
    let agent_a = AgentId::new("agent_a").map_err(err)?;
    let agent_b = AgentId::new("agent_b").map_err(err)?;
    let channel = ChannelId::new("chat").map_err(err)?;
    let config = RuntimeConfig {
        database: DatabaseConfig::new(db),
    };
    let mut runtime = Runtime::new(config.clone()).map_err(err)?;
    for (workspace, agent) in [(&workspace_a, &agent_a), (&workspace_b, &agent_b)] {
        runtime
            .storage()
            .create_workspace(WorkspaceSpec {
                id: workspace.clone(),
                name: workspace.to_string(),
                provider_id: provider_id.clone(),
                provider_config: json!({"runtime": workspace.as_str()}),
                provider_secret: json!({}),
                active: true,
                now: 1,
            })
            .map_err(err)?;
        runtime
            .storage()
            .create_agent(AgentSpec {
                workspace_id: workspace.clone(),
                id: agent.clone(),
                name: agent.to_string(),
                now: 1,
            })
            .map_err(err)?;
    }
    let state_a = Arc::new(FakeState::default());
    let state_b = Arc::new(FakeState::default());
    register(
        &mut runtime,
        workspace_a.clone(),
        provider_id.clone(),
        state_a.clone(),
    )?;
    register(
        &mut runtime,
        workspace_b.clone(),
        provider_id,
        state_b.clone(),
    )?;
    runtime
        .configure_agent(
            &workspace_a,
            agent_config(&workspace_a, &agent_a, &channel, "provider_agent_a"),
        )
        .await
        .map_err(err)?;
    runtime
        .configure_agent(
            &workspace_b,
            agent_config(&workspace_b, &agent_b, &channel, "provider_agent_b"),
        )
        .await
        .map_err(err)?;
    runtime.start().await.map_err(err)?;
    ensure(
        state_a.inits.load(Ordering::SeqCst) == 1,
        "workspace A init",
    )?;
    ensure(
        state_b.inits.load(Ordering::SeqCst) == 1,
        "workspace B init",
    )?;

    let always_channel = ChannelId::new("always").map_err(err)?;
    let scheduled_channel = ChannelId::new("scheduled").map_err(err)?;
    let muted_channel = ChannelId::new("muted").map_err(err)?;
    for event in [
        message(
            &always_channel,
            "always_chat",
            "always_1",
            "always",
            &agent_b,
            "human",
        ),
        message(
            &scheduled_channel,
            "scheduled_chat",
            "scheduled_1",
            "scheduled",
            &agent_b,
            "human",
        ),
        message(
            &muted_channel,
            "muted_chat",
            "muted_1",
            "muted",
            &agent_b,
            "human",
        ),
    ] {
        state_b.emit(event).await?;
        pull(&runtime, &workspace_b, 100).await?;
    }
    ensure(
        runtime
            .storage()
            .inbox_items(&workspace_b, &agent_b)
            .map_err(err)?
            .len()
            == 2,
        "always and schedule enqueue while mute does not",
    )?;
    let no_reply = || ComputerOutcome::Succeeded {
        output: "completed without sending".into(),
        reply: None,
    };
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace_b,
                    &agent_b,
                    &ScriptComputer::new(state_b.clone(), None, no_reply()),
                    101
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "always executes immediately",
    )?;
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace_b,
                    &agent_b,
                    &ScriptComputer::new(state_b.clone(), None, no_reply()),
                    102
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Idle
        ),
        "schedule waits for aligned boundary",
    )?;
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace_b,
                    &agent_b,
                    &ScriptComputer::new(state_b.clone(), None, no_reply()),
                    300_000
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "schedule executes at aligned boundary",
    )?;

    state_a.auto_reaction_failures.store(1, Ordering::SeqCst);
    let mut later_one = message(&channel, "chat_a", "m3", "later one", &agent_a, "human");
    let mut later_two = message(&channel, "chat_a", "m4", "later two", &agent_a, "human");
    let long_message = "x".repeat(205);
    let mut later_three = message(&channel, "chat_a", "m5", &long_message, &agent_a, "human");
    for event in [&mut later_one, &mut later_two, &mut later_three] {
        if let ProviderEventKind::Message(message) = &mut event.kind {
            message.mentioned_agent_ids.clear();
        }
    }
    for event in [
        message(&channel, "chat_a", "m1", "first", &agent_a, "human"),
        message(&channel, "chat_a", "m1", "first", &agent_a, "human"),
        message(&channel, "chat_b", "m2", "second", &agent_a, "human"),
        later_one,
        later_two,
        later_three,
        message(
            &channel,
            "chat_a",
            "self",
            "self echo",
            &agent_a,
            "provider_agent_a",
        ),
    ] {
        state_a.emit(event).await?;
    }
    let first = pull(&runtime, &workspace_a, 10).await?;
    ensure(
        matches!(first, lankide_runtime::CoordinatedEvent::Message { ref result, .. } if !result.duplicate),
        "auto reaction failure must not suppress ingest",
    )?;
    let duplicate = pull(&runtime, &workspace_a, 11).await?;
    ensure(
        matches!(duplicate, lankide_runtime::CoordinatedEvent::Message { ref result, .. } if result.duplicate),
        "duplicate callback",
    )?;
    pull(&runtime, &workspace_a, 12).await?;
    pull(&runtime, &workspace_a, 13).await?;
    pull(&runtime, &workspace_a, 14).await?;
    pull(&runtime, &workspace_a, 15).await?;
    pull(&runtime, &workspace_a, 16).await?;
    ensure(
        runtime.storage().message_count(&workspace_a).map_err(err)? == 6,
        "canonical dedup",
    )?;
    state_a.push_send(SendMode::Sent("provider_reply_1".into()));
    let computer = ScriptComputer::new(
        state_a.clone(),
        Some(message(
            &channel,
            "chat_a",
            "late",
            "arrived during execution",
            &agent_a,
            "human",
        )),
        ComputerOutcome::Succeeded {
            output: "answer one".into(),
            reply: Some(ExecutionReply {
                channel_id: channel.clone(),
                provider_chat_id: "chat_a".into(),
                content: "answer one".into(),
            }),
        },
    );
    let progress = runtime
        .execute_ready(&workspace_a, &agent_a, &computer, 20)
        .await
        .map_err(err)?;
    ensure(
        matches!(
            progress,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "first commit",
    )?;
    let assignments = computer.assignments.lock().map_err(err)?.clone();
    let rendered = &assignments[0]
        .initial_delivery
        .as_ref()
        .ok_or("missing delivery")?
        .rendered_markdown;
    ensure(
        rendered.contains("first")
            && rendered.contains("second")
            && rendered.contains(&format!("{}…", "x".repeat(199)))
            && !rendered.contains(&"x".repeat(200))
            && rendered.contains("self echo")
            && !rendered.contains("later two")
            && !rendered.contains("later one"),
        "multi-chat snapshot",
    )?;
    ensure(
        rendered.contains("未读事项：")
            && rendered.contains("channel_id=chat，chat_id=chat_a")
            && rendered.contains("channel_id=chat，chat_id=chat_b")
            && rendered.contains("[message_id="),
        "Harness receives readable Chat and Message targets",
    )?;
    ensure(
        !rendered.contains("Target: provider=")
            && !rendered.contains("read_to_message_id")
            && !rendered.contains("provider=fixture"),
        "Harness input does not expose Runtime transport fields",
    )?;
    ensure(
        !rendered.contains("arrived during execution"),
        "snapshot must stay fixed",
    )?;
    ensure(
        assignments[0].system_prompt.contains("agent_a")
            && assignments[0].system_prompt.contains("## 身份")
            && assignments[0].system_prompt.contains("## 团队成员")
            && assignments[0]
                .system_prompt
                .contains("Name: agent_a；Role: Agent；ID: agent_a")
            && assignments[0].system_prompt.contains("## 每轮工作")
            && assignments[0]
                .system_prompt
                .contains("provider prompt for agent_a")
            && assignments[0].system_prompt.contains("## 完成本轮"),
        "Runtime and Provider prompts compose into one Agent contract",
    )?;
    ensure(
        assignments[0].tools.len() == 8,
        "standard and dynamic tools",
    )?;
    ensure(assignments[0].cli_commands.len() == 1, "dynamic cli")?;
    let provider_contexts = state_a.runtime_contexts.lock().map_err(err)?.clone();
    ensure(
        provider_contexts
            .iter()
            .all(|context| context.chats.len() == 2),
        "provider receives every frozen Chat",
    )?;
    ensure(
        provider_contexts.iter().all(|context| {
            context
                .chats
                .iter()
                .any(|chat| chat.chat_id == "chat_a" && chat.provider_id.as_str() == "fixture")
                && context.chats.iter().any(|chat| chat.chat_id == "chat_b")
        }),
        "provider receives explicit Chat targets",
    )?;
    let first_execution = assignments[0]
        .execution
        .as_ref()
        .ok_or("missing execution ref")?
        .execution_id;
    let tool_request = ToolInvocationRequest {
        execution_id: first_execution,
        attempt: 1,
        tool_call_id: "tool_1".into(),
        tool_name: "echo".into(),
        arguments: json!({"value": "ok"}),
        now: 20,
    };
    let frozen_spec = lankide_provider::AgentRuntimeSpec {
        agent_config: provider_contexts[0].agent_config.clone(),
        provider_agent_config: provider_contexts[0].provider_agent_config.clone(),
        computer_id: provider_contexts[0].computer_id.clone(),
        harness_id: provider_contexts[0].harness_id.clone(),
        model: provider_contexts[0].model.clone(),
        thinking_level: provider_contexts[0].thinking_level.clone(),
        trigger_reason: provider_contexts[0].trigger_reason.clone(),
        chats: provider_contexts[0].chats.clone(),
        unread_snapshot: provider_contexts[0].unread_snapshot.clone(),
    };
    let tool_first = runtime
        .invoke_provider_tool(
            &workspace_a,
            agent_a.clone(),
            frozen_spec.clone(),
            tool_request.clone(),
        )
        .await
        .map_err(err)?;
    let tool_repeat = runtime
        .invoke_provider_tool(&workspace_a, agent_a.clone(), frozen_spec, tool_request)
        .await
        .map_err(err)?;
    ensure(tool_first == tool_repeat, "tool callback idempotency")?;
    let log = state_a.log.lock().map_err(err)?.clone();
    ensure(
        log.iter().position(|v| v == "auto_reaction") < log.iter().position(|v| v == "computer"),
        "auto reaction precedes dispatch",
    )?;
    let cursors_after_first = runtime
        .storage()
        .cursors(&workspace_a, &agent_a)
        .map_err(err)?;
    ensure(cursors_after_first.len() == 2, "multi-chat cursor commit")?;

    pull(&runtime, &workspace_a, 21).await?;
    let before_failure = runtime
        .storage()
        .cursors(&workspace_a, &agent_a)
        .map_err(err)?;
    state_a.push_send(SendMode::Fail);
    let failed = runtime
        .execute_ready(
            &workspace_a,
            &agent_a,
            &ScriptComputer::new(
                state_a.clone(),
                None,
                successful_reply(&channel, "chat_a", "retry answer"),
            ),
            22,
        )
        .await
        .map_err(err)?;
    ensure(
        matches!(failed, ExecutionProgress::Failed { .. }),
        "send failure",
    )?;
    ensure(
        runtime
            .storage()
            .cursors(&workspace_a, &agent_a)
            .map_err(err)?
            == before_failure,
        "failure cannot advance cursor",
    )?;
    state_a.push_send(SendMode::Sent("provider_reply_retry".into()));
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace_a,
                    &agent_a,
                    &ScriptComputer::new(
                        state_a.clone(),
                        None,
                        successful_reply(&channel, "chat_a", "retry answer"),
                    ),
                    23,
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "retry commits",
    )?;

    state_a
        .emit(reaction(
            &channel,
            "chat_a",
            "message-outside-runtime-history",
            "reaction_unknown_target",
        ))
        .await?;
    let ignored_reaction = pull(&runtime, &workspace_a, 24).await?;
    ensure(
        matches!(ignored_reaction, lankide_runtime::CoordinatedEvent::Reaction(ref value) if value.ignored_unknown_target && value.inbox_item_ids.is_empty()),
        "unknown historical reaction is accepted without blocking later events",
    )?;
    state_a
        .emit(reaction(&channel, "chat_a", "late", "reaction_1"))
        .await?;
    let reaction = pull(&runtime, &workspace_a, 25).await?;
    ensure(
        matches!(reaction, lankide_runtime::CoordinatedEvent::Reaction(ref value) if !value.duplicate && value.inbox_item_ids.len() == 1),
        "reaction trigger",
    )?;
    state_a.push_send(SendMode::Sent("provider_reaction_reply".into()));
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace_a,
                    &agent_a,
                    &ScriptComputer::new(
                        state_a.clone(),
                        None,
                        successful_reply(&channel, "chat_a", "reaction answer"),
                    ),
                    26,
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "reaction execution",
    )?;

    state_a
        .emit(message(
            &channel,
            "chat_a",
            "cancel",
            "cancel me",
            &agent_a,
            "human",
        ))
        .await?;
    pull(&runtime, &workspace_a, 26).await?;
    let before_cancel = runtime
        .storage()
        .cursors(&workspace_a, &agent_a)
        .map_err(err)?;
    let cancelled = runtime
        .execute_ready(
            &workspace_a,
            &agent_a,
            &ScriptComputer::new(state_a.clone(), None, ComputerOutcome::Cancelled),
            27,
        )
        .await
        .map_err(err)?;
    ensure(
        matches!(cancelled, ExecutionProgress::Cancelled { .. }),
        "cancel",
    )?;
    ensure(
        runtime
            .storage()
            .cursors(&workspace_a, &agent_a)
            .map_err(err)?
            == before_cancel,
        "cancel cursor",
    )?;

    state_a.push_send(SendMode::Unknown);
    let pending = runtime
        .execute_ready(
            &workspace_a,
            &agent_a,
            &ScriptComputer::new(
                state_a.clone(),
                None,
                successful_reply(&channel, "chat_a", "unknown answer"),
            ),
            28,
        )
        .await
        .map_err(err)?;
    let request_key = match pending {
        ExecutionProgress::PendingSend { request_key, .. } => request_key,
        other => return Err(format!("expected unknown send, got {other:?}")),
    };
    runtime.shutdown().await.map_err(err)?;
    drop(runtime);

    let mut restarted = Runtime::new(config).map_err(err)?;
    register(
        &mut restarted,
        workspace_a.clone(),
        ProviderId::new("fixture").map_err(err)?,
        state_a.clone(),
    )?;
    restarted
        .configure_agent(
            &workspace_a,
            agent_config(&workspace_a, &agent_a, &channel, "provider_agent_a"),
        )
        .await
        .map_err(err)?;
    restarted.start().await.map_err(err)?;
    ensure(
        restarted
            .recoverable_executions(Some(&workspace_a))
            .map_err(err)?
            .len()
            == 1,
        "recover execution",
    )?;
    ensure(
        restarted
            .recoverable_sends(Some(&workspace_a))
            .map_err(err)?
            .len()
            == 1,
        "recover send",
    )?;
    ensure(
        matches!(
            restarted
                .resolve_unknown_send(&workspace_a, &request_key, "reconciled_id".into(), 30)
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "reconcile unknown",
    )?;

    restarted
        .pause_workspace(&workspace_a, 31)
        .await
        .map_err(err)?;
    ensure(
        restarted.next_provider_event(&workspace_a).await.is_err(),
        "paused gate",
    )?;
    restarted
        .resume_workspace(&workspace_a, 32)
        .await
        .map_err(err)?;
    ensure(
        state_a.inits.load(Ordering::SeqCst) >= 3,
        "resume creates fresh host",
    )?;
    let lead = restarted
        .storage()
        .get_or_create_person(
            &workspace_a,
            &ProviderId::new("fixture").map_err(err)?,
            "lead",
            90,
        )
        .map_err(err)?;
    restarted
        .storage()
        .initialize_team_lead(&workspace_a, &lead.person_id, 90)
        .map_err(err)?;
    let member = restarted
        .storage()
        .get_or_create_person(
            &workspace_a,
            &ProviderId::new("fixture").map_err(err)?,
            "member",
            90,
        )
        .map_err(err)?;
    let command = restarted
        .coordinate_event(
            &workspace_a,
            message(
                &channel,
                "chat_a",
                "team-command",
                &format!("/team lead @Team Member[{}]", member.person_id),
                &agent_a,
                lead.person_id.as_str(),
            ),
            91,
        )
        .await
        .map_err(err)?;
    ensure(
        matches!(command, lankide_runtime::CoordinatedEvent::Message { ref result, .. } if result.new_inbox_items.is_empty()),
        "team command must not dispatch to Agents",
    )?;
    ensure(
        restarted
            .storage()
            .person(&workspace_a, &member.person_id)
            .map_err(err)?
            .is_some_and(|person| person.team_role == TeamRole::Lead),
        "team lead transfer",
    )?;
    ensure(
        restarted
            .storage()
            .person(
                &workspace_a,
                &PersonId::new(lead.person_id.to_string()).map_err(err)?,
            )
            .map_err(err)?
            .is_some_and(|person| person.team_role == TeamRole::Member),
        "previous lead becomes member",
    )?;
    restarted.shutdown().await.map_err(err)?;
    Ok(())
}

async fn pause_drain_scenario(db: &Path) -> Result<(), String> {
    let workspace = WorkspaceId::new("pause_workspace").map_err(err)?;
    let agent = AgentId::new("pause_agent").map_err(err)?;
    let channel = ChannelId::new("chat").map_err(err)?;
    let state = Arc::new(FakeState::default());
    let mut runtime = Runtime::new(RuntimeConfig {
        database: DatabaseConfig::new(db),
    })
    .map_err(err)?;
    runtime
        .storage()
        .create_workspace(WorkspaceSpec {
            id: workspace.clone(),
            name: "Pause Workspace".into(),
            provider_id: ProviderId::new("fixture").map_err(err)?,
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .map_err(err)?;
    runtime
        .storage()
        .create_agent(AgentSpec {
            workspace_id: workspace.clone(),
            id: agent.clone(),
            name: "Pause Agent".into(),
            now: 1,
        })
        .map_err(err)?;
    register(
        &mut runtime,
        workspace.clone(),
        ProviderId::new("fixture").map_err(err)?,
        state.clone(),
    )?;
    runtime
        .configure_agent(
            &workspace,
            agent_config(&workspace, &agent, &channel, "provider_pause_agent"),
        )
        .await
        .map_err(err)?;
    runtime.start().await.map_err(err)?;
    runtime
        .coordinate_event(
            &workspace,
            message(
                &channel,
                "pause-chat",
                "pause-trigger",
                "start",
                &agent,
                "human",
            ),
            2,
        )
        .await
        .map_err(err)?;
    let runtime = Arc::new(runtime);
    let (release_tx, release_rx) = oneshot::channel();
    let computer = Arc::new(GateComputer {
        release: tokio::sync::Mutex::new(Some(release_rx)),
        started: tokio::sync::Notify::new(),
        outcome: no_reply_outcome(),
    });
    let task_runtime = runtime.clone();
    let task_computer = computer.clone();
    let task_workspace = workspace.clone();
    let task_agent = agent.clone();
    let execution = tokio::spawn(async move {
        task_runtime
            .execute_ready(&task_workspace, &task_agent, task_computer.as_ref(), 3)
            .await
    });
    computer.started.notified().await;
    runtime.pause_workspace(&workspace, 4).await.map_err(err)?;
    ensure(
        state.shutdowns.load(Ordering::SeqCst) == 0
            && runtime.next_provider_event(&workspace).await.is_err()
            && runtime
                .execute_ready(
                    &workspace,
                    &agent,
                    &ScriptComputer::new(state.clone(), None, no_reply_outcome()),
                    5,
                )
                .await
                .is_err(),
        "pause blocks consumption and new scheduling while draining",
    )?;
    release_tx
        .send(())
        .map_err(|_| "release execution".to_owned())?;
    ensure(
        matches!(
            execution.await.map_err(err)?.map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "active execution commits while workspace is paused",
    )?;
    runtime
        .finish_paused_workspace(&workspace)
        .await
        .map_err(err)?;
    ensure(
        state.shutdowns.load(Ordering::SeqCst) == 1,
        "paused Provider shuts down after active execution",
    )?;
    runtime.resume_workspace(&workspace, 6).await.map_err(err)?;
    ensure(
        state.inits.load(Ordering::SeqCst) == 2,
        "resume creates a fresh Provider host",
    )?;
    runtime.shutdown().await.map_err(err)?;
    Ok(())
}

async fn collaboration_fencing_scenario(db: &Path) -> Result<(), String> {
    let provider_id = ProviderId::new("fixture").map_err(err)?;
    let workspace = WorkspaceId::new("collaboration_workspace").map_err(err)?;
    let bruce = AgentId::new("bruce").map_err(err)?;
    let clark = AgentId::new("clark").map_err(err)?;
    let channel = ChannelId::new("chat").map_err(err)?;
    let mut runtime = Runtime::new(RuntimeConfig {
        database: DatabaseConfig::new(db),
    })
    .map_err(err)?;
    runtime
        .storage()
        .create_workspace(WorkspaceSpec {
            id: workspace.clone(),
            name: "Collaboration".into(),
            provider_id: provider_id.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .map_err(err)?;
    for agent in [&bruce, &clark] {
        runtime
            .storage()
            .create_agent(AgentSpec {
                workspace_id: workspace.clone(),
                id: agent.clone(),
                name: agent.to_string(),
                now: 1,
            })
            .map_err(err)?;
    }
    let state = Arc::new(FakeState::default());
    register(&mut runtime, workspace.clone(), provider_id, state.clone())?;
    runtime
        .configure_agent(
            &workspace,
            agent_config(&workspace, &bruce, &channel, "provider_agent_bruce"),
        )
        .await
        .map_err(err)?;
    runtime
        .configure_agent(
            &workspace,
            agent_config(&workspace, &clark, &channel, "provider_agent_clark"),
        )
        .await
        .map_err(err)?;
    runtime.start().await.map_err(err)?;

    let mut prompt = message(
        &channel,
        "team_chat",
        "prompt",
        "introduce yourselves",
        &bruce,
        "human",
    );
    if let ProviderEventKind::Message(message) = &mut prompt.kind {
        message.mentioned_agent_ids.push(clark.clone());
    }
    state.emit(prompt).await?;
    pull(&runtime, &workspace, 2).await?;

    let runtime = Arc::new(runtime);
    let (release_tx, release_rx) = oneshot::channel();
    let clark_computer = Arc::new(GateComputer {
        release: tokio::sync::Mutex::new(Some(release_rx)),
        started: tokio::sync::Notify::new(),
        outcome: successful_reply(&channel, "team_chat", "Clark stale reply"),
    });
    let task_runtime = runtime.clone();
    let task_computer = clark_computer.clone();
    let task_workspace = workspace.clone();
    let task_clark = clark.clone();
    let clark_execution = tokio::spawn(async move {
        task_runtime
            .execute_ready(&task_workspace, &task_clark, task_computer.as_ref(), 3)
            .await
    });
    clark_computer.started.notified().await;

    state.push_send(SendMode::Sent("provider_bruce_message".into()));
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace,
                    &bruce,
                    &ScriptComputer::new(
                        state.clone(),
                        None,
                        successful_reply(&channel, "team_chat", "Bruce introduced himself"),
                    ),
                    4,
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "first Agent reply commits",
    )?;
    release_tx
        .send(())
        .map_err(|_| "release stale collaboration execution".to_owned())?;
    ensure(
        matches!(
            clark_execution.await.map_err(err)?.map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "stale automatic reply is suppressed and the execution commits",
    )?;
    ensure(
        state.provider_send_keys.lock().map_err(err)?.len() == 1,
        "stale automatic reply must not reach the Provider",
    )?;

    let callback = message(
        &channel,
        "team_chat",
        "provider_bruce_message",
        "Bruce introduced himself",
        &clark,
        "provider_agent_bruce",
    );
    state.emit(callback).await?;
    let callback = pull(&runtime, &workspace, 5).await?;
    ensure(
        matches!(callback, lankide_runtime::CoordinatedEvent::Message { ref result, .. } if result.duplicate),
        "Provider callback merges with the canonical outbound message",
    )?;
    let reader = ScriptComputer::new(state.clone(), None, no_reply_outcome());
    ensure(
        matches!(
            runtime
                .execute_ready(&workspace, &clark, &reader, 6)
                .await
                .map_err(err)?,
            ExecutionProgress::Committed {
                committed: true,
                ..
            }
        ),
        "next execution reads the merged Agent message",
    )?;
    let assignments = reader.assignments.lock().map_err(err)?;
    ensure(
        assignments
            .first()
            .and_then(|assignment| assignment.initial_delivery.as_ref())
            .is_some_and(|delivery| {
                delivery
                    .rendered_markdown
                    .contains("Bruce introduced himself")
            }),
        "other Agent outbound messages enter the readable snapshot",
    )?;
    drop(assignments);
    let mut unmentioned_callback = message(
        &channel,
        "team_chat",
        "provider_bruce_unmentioned",
        "Bruce adds an unmentioned follow-up",
        &clark,
        "provider_agent_bruce",
    );
    if let ProviderEventKind::Message(message) = &mut unmentioned_callback.kind {
        message.mentioned_agent_ids.clear();
    }
    state.emit(unmentioned_callback).await?;
    let unmentioned_callback = pull(&runtime, &workspace, 7).await?;
    ensure(
        matches!(unmentioned_callback, lankide_runtime::CoordinatedEvent::Message { ref result, .. } if result.new_inbox_items.is_empty()),
        "an unmentioned Agent message must not start another Agent turn",
    )?;
    runtime.shutdown().await.map_err(err)?;
    Ok(())
}

async fn attempt_recovery_scenario(db: &Path) -> Result<(), String> {
    attempt_lifecycle_scenario(db, 0).await?;
    attempt_lifecycle_scenario(&db.with_extension("lease.db"), 1).await?;
    attempt_lifecycle_scenario(&db.with_extension("missing.db"), 2).await
}

async fn attempt_lifecycle_scenario(db: &Path, failure_mode: u8) -> Result<(), String> {
    let workspace_id = WorkspaceId::new("attempt_workspace").map_err(err)?;
    let agent_id = AgentId::new("attempt_agent").map_err(err)?;
    let provider_id = ProviderId::new("fixture").map_err(err)?;
    let channel_id = ChannelId::new("chat").map_err(err)?;
    let runtime = Runtime::new(RuntimeConfig {
        database: DatabaseConfig::new(db),
    })
    .map_err(err)?;
    runtime
        .storage()
        .create_workspace(WorkspaceSpec {
            id: workspace_id.clone(),
            name: "Attempt Workspace".into(),
            provider_id: provider_id.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: false,
            now: 1,
        })
        .map_err(err)?;
    runtime
        .storage()
        .create_agent(AgentSpec {
            workspace_id: workspace_id.clone(),
            id: agent_id.clone(),
            name: "Attempt Agent".into(),
            now: 2,
        })
        .map_err(err)?;
    runtime
        .storage()
        .ingest_event(IngestEvent {
            workspace_id: workspace_id.clone(),
            provider_id,
            channel_id: channel_id.clone(),
            channel_name: "Chat".into(),
            provider_chat_id: "attempt-chat".into(),
            chat_name: "Attempt Chat".into(),
            provider_event_id: Some("attempt-event".into()),
            provider_message_id: "attempt-message".into(),
            message: MessageInput {
                sender_kind: lankide_core::MessageAuthorType::Human,
                sender_id: "human".into(),
                sender_name: "Human".into(),
                content: "run".into(),
                mentions: vec![agent_id.clone()],
                reply_to_provider_message_id: None,
                resources: Vec::new(),
                occurred_at: 3,
            },
            dispatch: vec![InboxDispatch {
                agent_id: agent_id.clone(),
                available_at: 3,
                trigger_reason: DispatchReason::Mention,
            }],
            now: 3,
        })
        .map_err(err)?;
    let created = runtime
        .storage()
        .create_execution(&workspace_id, &agent_id, "attempt", 4)
        .map_err(err)?
        .ok_or("attempt execution missing")?;
    let broker = runtime.new_computer_broker();
    let hello = ComputerHello {
        protocol_version: PROTOCOL_VERSION,
        computer_id: "computer-attempt".into(),
        instance_id: "instance-stable".into(),
        computer_version: "1".into(),
        os: "test".into(),
        arch: "test".into(),
        harnesses: Vec::new(),
    };
    let mut first = broker.connect(hello.clone()).map_err(err)?;
    let _ = first.next().await;
    let execution = ExecutionRef {
        execution_id: created.execution_id,
        attempt: 1,
    };
    let assignment = AssignExecution {
        execution: Some(execution.clone()),
        workspace: Some(WorkspaceRef {
            workspace_id: workspace_id.to_string(),
        }),
        agent: Some(AgentRuntimeProfile {
            agent: Some(AgentRef {
                workspace_id: workspace_id.to_string(),
                agent_id: agent_id.to_string(),
            }),
            name: "Attempt Agent".into(),
            role: String::new(),
            bio: String::new(),
        }),
        harness: Some(HarnessConfig::default()),
        session: Some(SessionDirective::default()),
        initial_delivery: Some(Delivery {
            delivery_id: created.delivery_id,
            kind: DeliveryKind::Initial as i32,
            rendered_markdown: "run".into(),
            snapshot_id: created.snapshot_id,
            trigger_reasons: vec!["mention".into()],
            resources: Vec::new(),
            created_at: 4,
        }),
        system_prompt: String::new(),
        tools: Vec::new(),
        cli_commands: Vec::new(),
        resources: Vec::new(),
        execution_token: "token".into(),
        created_at: 4,
    };
    let executor = broker.executor("computer-attempt");
    let execution_task = tokio::spawn(async move { executor.execute(assignment).await });
    ensure(
        matches!(
            first.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::AssignExecution(_))
            }))
        ),
        "attempt assignment",
    )?;
    broker
        .handle_frame(
            "computer-attempt",
            "instance-stable",
            ComputerFrame {
                frame: Some(computer_frame::Frame::AssignmentResult(AssignmentResult {
                    execution: Some(execution.clone()),
                    accepted: true,
                    error_code: String::new(),
                    error_message: String::new(),
                })),
            },
        )
        .await
        .map_err(err)?;
    broker
        .handle_frame(
            "computer-attempt",
            "instance-stable",
            ComputerFrame {
                frame: Some(computer_frame::Frame::DeliveryResult(DeliveryResult {
                    execution: Some(execution.clone()),
                    delivery_id: created.delivery_id,
                    accepted: true,
                    error_code: String::new(),
                    error_message: String::new(),
                })),
            },
        )
        .await
        .map_err(err)?;
    broker
        .handle_frame(
            "computer-attempt",
            "instance-stable",
            ComputerFrame {
                frame: Some(computer_frame::Frame::ExecutionEvent(ExecutionEvent {
                    execution: Some(execution.clone()),
                    sequence: 1,
                    occurred_at: 5,
                    kind: ExecutionEventKind::Started as i32,
                    payload: br#"{"kind":"started"}"#.to_vec(),
                })),
            },
        )
        .await
        .map_err(err)?;
    ensure(
        matches!(
            first.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::ExecutionEventAck(_))
            }))
        ),
        "durable started ack",
    )?;
    if failure_mode > 0 {
        let expected_code = if failure_mode == 1 {
            "execution_lease_expired"
        } else {
            "execution_attempt_lost"
        };
        if failure_mode == 1 {
            broker
                .expire_leases(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64
                        + 30_000,
                )
                .map_err(err)?;
        } else {
            broker
                .handle_frame(
                    "computer-attempt",
                    "instance-stable",
                    ComputerFrame {
                        frame: Some(computer_frame::Frame::Heartbeat(ComputerHeartbeat {
                            computer_id: "computer-attempt".into(),
                            instance_id: "instance-stable".into(),
                            sent_at: SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64
                                + 30_000,
                            active_attempts: Vec::new(),
                        })),
                    },
                )
                .await
                .map_err(err)?;
        }
        let result = tokio::time::timeout(Duration::from_secs(2), execution_task)
            .await
            .map_err(err)?
            .map_err(err)?;
        ensure(
            matches!(result, Err(ComputerError { code }) if code == expected_code),
            "lease expiry must release pending execution",
        )?;
        ensure(
            matches!(
                first.next().await,
                Some(Ok(lankide_protocol::RuntimeFrame {
                    frame: Some(runtime_frame::Frame::CancelExecution(_))
                }))
            ),
            "expired attempt cancellation",
        )?;
        ensure(
            runtime
                .storage()
                .execution(created.execution_id)
                .map_err(err)?
                .state
                == lankide_storage::ExecutionState::Failed,
            "lost execution is finalized",
        )?;
        broker
            .disconnect("computer-attempt", "instance-stable")
            .map_err(err)?;
        runtime.shutdown().await.map_err(err)?;
        return Ok(());
    }
    broker
        .disconnect("computer-attempt", "instance-stable")
        .map_err(err)?;
    ensure(
        execution_task.await.map_err(err)?.is_err(),
        "disconnect releases waiting execution",
    )?;
    ensure(
        runtime
            .storage()
            .execution(created.execution_id)
            .map_err(err)?
            .state
            == lankide_storage::ExecutionState::Recovering,
        "execution recovering",
    )?;

    let mut resumed = broker.connect(hello).map_err(err)?;
    let _ = resumed.next().await;
    broker
        .handle_frame(
            "computer-attempt",
            "instance-stable",
            ComputerFrame {
                frame: Some(computer_frame::Frame::Heartbeat(ComputerHeartbeat {
                    computer_id: "computer-attempt".into(),
                    instance_id: "instance-stable".into(),
                    sent_at: 6,
                    active_attempts: vec![ActiveAttempt {
                        execution: Some(execution.clone()),
                        last_event_sequence: 1,
                        harness_process_state: "active".into(),
                    }],
                })),
            },
        )
        .await
        .map_err(err)?;
    ensure(
        runtime
            .storage()
            .execution(created.execution_id)
            .map_err(err)?
            .state
            == lankide_storage::ExecutionState::Running,
        "same instance continues",
    )?;
    let completed = ComputerFrame {
        frame: Some(computer_frame::Frame::ExecutionEvent(ExecutionEvent {
            execution: Some(execution.clone()),
            sequence: 2,
            occurred_at: 7,
            kind: ExecutionEventKind::Completed as i32,
            payload: br#"{"kind":"completed","final_output":"done"}"#.to_vec(),
        })),
    };
    broker
        .handle_frame("computer-attempt", "instance-stable", completed.clone())
        .await
        .map_err(err)?;
    ensure(
        matches!(
            resumed.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::ExecutionEventAck(_))
            }))
        ),
        "completed durable ack",
    )?;
    ensure(
        matches!(
            resumed.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::ExecutionCommitted(_))
            }))
        ),
        "commit follows durable transaction",
    )?;
    broker
        .handle_frame("computer-attempt", "instance-stable", completed)
        .await
        .map_err(err)?;
    let _ = resumed.next().await;
    ensure(
        matches!(
            resumed.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::ExecutionCommitted(_))
            }))
        ),
        "committed replay",
    )?;
    ensure(
        runtime
            .storage()
            .execution(created.execution_id)
            .map_err(err)?
            .state
            == lankide_storage::ExecutionState::Succeeded,
        "attempt committed",
    )?;
    ensure(
        runtime
            .storage()
            .execution_events(created.execution_id, 1)
            .map_err(err)?
            .len()
            == 2,
        "events committed once",
    )?;
    ensure(
        runtime
            .storage()
            .cursors(&workspace_id, &agent_id)
            .map_err(err)?
            .len()
            == 1,
        "cursor committed once",
    )?;
    broker
        .disconnect("computer-attempt", "instance-stable")
        .map_err(err)?;
    runtime.shutdown().await.map_err(err)?;
    Ok(())
}

async fn application_scenario(db: &Path, socket: &Path) -> Result<(), String> {
    let provider_id = ProviderId::new("fixture").map_err(err)?;
    let workspace_id = WorkspaceId::new("application_a").map_err(err)?;
    let agent_id = AgentId::new("application_agent").map_err(err)?;
    let channel_id = ChannelId::new("chat").map_err(err)?;
    let token = "application-token";
    let storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    storage
        .create_workspace(WorkspaceSpec {
            id: workspace_id.clone(),
            name: "Application A".into(),
            provider_id: provider_id.clone(),
            provider_config: json!({"kind": "fixture"}),
            provider_secret: json!({"token": "must-not-leak"}),
            active: true,
            now: 1,
        })
        .map_err(err)?;
    storage
        .create_agent(AgentSpec {
            workspace_id: workspace_id.clone(),
            id: agent_id.clone(),
            name: "Application Agent".into(),
            now: 2,
        })
        .map_err(err)?;
    let mut agent = storage
        .agent_record(&workspace_id, &agent_id)
        .map_err(err)?;
    agent.role = "assistant".into();
    agent.bio = "fixture".into();
    agent.computer_id = "computer-a".into();
    agent.harness_id = "fixture".into();
    agent.model = "fixture".into();
    agent.thinking = "low".into();
    agent
        .attention
        .insert(channel_id.clone(), AttentionRecord::Mention);
    storage.upsert_agent_record(agent).map_err(err)?;
    let application_trigger = format!(
        "application trigger {} END_OF_LONG_TRIGGER",
        "说明🙂".repeat(120)
    );
    let initial_event = IngestEvent {
        workspace_id: workspace_id.clone(),
        provider_id: provider_id.clone(),
        channel_id: channel_id.clone(),
        channel_name: "Chat".into(),
        provider_chat_id: "application-chat".into(),
        chat_name: "Application Chat".into(),
        provider_event_id: Some("application-event-1".into()),
        provider_message_id: "application-message-1".into(),
        message: MessageInput {
            sender_kind: lankide_core::MessageAuthorType::Human,
            sender_id: "human".into(),
            sender_name: "Human".into(),
            content: application_trigger.clone(),
            mentions: vec![agent_id.clone()],
            reply_to_provider_message_id: None,
            resources: Vec::new(),
            occurred_at: 3,
        },
        dispatch: vec![InboxDispatch {
            agent_id: agent_id.clone(),
            available_at: 3,
            trigger_reason: DispatchReason::Mention,
        }],
        now: 3,
    };
    let mut oldest_message_id = None;
    for index in 0..4 {
        let mut historical = initial_event.clone();
        historical.provider_event_id = Some(format!("historical-event-{index}"));
        historical.provider_message_id = format!("historical-message-{index}");
        historical.message.content = format!("historical {index}: {application_trigger}");
        historical.message.mentions.clear();
        historical.dispatch.clear();
        let stored = storage.ingest_event(historical).map_err(err)?;
        oldest_message_id.get_or_insert(stored.message_id);
    }
    let oldest_message_id = oldest_message_id.unwrap();
    let ingested = storage.ingest_event(initial_event).map_err(err)?;
    let token_hash = Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let execution = storage
        .create_execution_with_auth(
            &workspace_id,
            &agent_id,
            "application",
            4,
            ExecutionAuthSpec {
                attempt: 1,
                token_hash,
                expires_at: i64::MAX,
            },
        )
        .map_err(err)?
        .ok_or("application execution was not created")?;
    let frozen_context_json = json!({
        "agent_config": {},
        "provider_agent_config": {},
        "computer_id": "computer-a",
        "harness_id": "fixture",
        "model": "fixture",
        "thinking_level": "low",
        "trigger_reason": "application",
        "chats": [{
            "provider_id": "fixture",
            "channel_id": "chat",
            "channel_name": "Chat",
            "chat_id": "application-chat",
            "chat_name": "Application Chat"
        }],
        "unread_snapshot": {"chats":[{"channel_id":"chat","chat_id":"application-chat","read_to_message_id": ingested.message_id}]}
    }).to_string();
    let frozen_context_fingerprint = Sha256::digest(frozen_context_json.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    storage
        .freeze_attempt_authorization(AttemptAuthorization {
            execution_id: execution.execution_id,
            attempt: 1,
            context_json: frozen_context_json,
            context_fingerprint: frozen_context_fingerprint,
            capabilities: [
                "message_send",
                "message_recall",
                "message_edit",
                "chat_messages",
                "unread_current",
                "attention_set",
            ]
            .into_iter()
            .map(|name| AttemptCapability {
                name: name.into(),
                display_name: name.into(),
                description: name.into(),
                kind: lankide_protocol::CapabilityKind::Tool as i32,
                input_schema_json: "{}".into(),
                output_schema_json: "{}".into(),
            })
            .chain(std::iter::once(AttemptCapability {
                name: "echo".into(),
                display_name: "echo".into(),
                description: "echo".into(),
                kind: lankide_protocol::CapabilityKind::Tool as i32,
                input_schema_json: "{}".into(),
                output_schema_json: "{}".into(),
            }))
            .chain(std::iter::once(AttemptCapability {
                name: "runtime_lookup".into(),
                display_name: "runtime_lookup".into(),
                description: "runtime_lookup".into(),
                kind: lankide_protocol::CapabilityKind::Tool as i32,
                input_schema_json: "{}".into(),
                output_schema_json: "{}".into(),
            }))
            .collect(),
        })
        .map_err(err)?;
    storage
        .mark_execution_running(execution.execution_id, 4)
        .map_err(err)?;
    storage
        .offer_execution_attempt(
            execution.execution_id,
            1,
            "computer-a",
            "instance-2",
            i64::MAX,
            4,
        )
        .map_err(err)?;
    storage
        .accept_execution_attempt(execution.execution_id, 1, true, 4)
        .map_err(err)?;
    storage.close().map_err(err)?;

    let state = Arc::new(FakeState::default());
    let factories = ProviderFactoryRegistry::default();
    factories
        .register(Arc::new(ApplicationFactory {
            provider_id: provider_id.clone(),
            state: state.clone(),
        }))
        .map_err(err)?;
    let config = RuntimeConfig {
        database: DatabaseConfig::new(db),
    };
    let application = RuntimeApplication::open(config.clone(), factories.clone())
        .await
        .map_err(err)?;
    let recovered = application.recover(5).map_err(err)?;
    ensure(
        recovered
            .recovering_execution_ids
            .contains(&execution.execution_id),
        "application recovery report",
    )?;
    application.start().await.map_err(err)?;
    ensure(
        application.readiness().management_ready
            && application.readiness().providers_started
            && !application.readiness().computer_connected,
        "management readiness does not require Computer",
    )?;
    let broker = application.computer_broker();
    let mut first_connection = broker
        .connect(ComputerHello {
            protocol_version: PROTOCOL_VERSION,
            computer_id: "computer-a".into(),
            instance_id: "instance-1".into(),
            computer_version: "1".into(),
            os: "test".into(),
            arch: "test".into(),
            harnesses: Vec::new(),
        })
        .map_err(err)?;
    ensure(
        matches!(
            first_connection.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::Hello(_)),
            }))
        ),
        "Computer hello",
    )?;
    let mut second_connection = broker
        .connect(ComputerHello {
            protocol_version: PROTOCOL_VERSION,
            computer_id: "computer-a".into(),
            instance_id: "instance-2".into(),
            computer_version: "1".into(),
            os: "test".into(),
            arch: "test".into(),
            harnesses: Vec::new(),
        })
        .map_err(err)?;
    ensure(
        matches!(
            first_connection.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                frame: Some(runtime_frame::Frame::Shutdown(_)),
            }))
        ),
        "Computer reconnect replaces old connection",
    )?;
    let _ = second_connection.next().await;
    broker.disconnect("computer-a", "instance-1").map_err(err)?;
    ensure(
        broker.is_connected("computer-a"),
        "stale disconnect keeps reconnected Computer",
    )?;
    let same_instance = ComputerHello {
        protocol_version: PROTOCOL_VERSION,
        computer_id: "computer-same-instance".into(),
        instance_id: "stable-instance".into(),
        computer_version: "1".into(),
        os: "test".into(),
        arch: "test".into(),
        harnesses: Vec::new(),
    };
    let (old_connection_id, mut old_connection) =
        broker.connect_tracked(same_instance.clone()).map_err(err)?;
    let _ = old_connection.next().await;
    let (_new_connection_id, mut new_connection) =
        broker.connect_tracked(same_instance).map_err(err)?;
    let _ = new_connection.next().await;
    broker
        .disconnect_connection(
            "computer-same-instance",
            "stable-instance",
            old_connection_id,
        )
        .map_err(err)?;
    ensure(
        broker.is_connected("computer-same-instance"),
        "old same-instance stream keeps reconnected Computer",
    )?;
    for _ in 0..100 {
        if state.sink.lock().map_err(err)?.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut follow_chat = message(
        &channel_id,
        "application-steer-chat",
        "application-follow-chat",
        "ordinary message",
        &agent_id,
        "human",
    );
    if let ProviderEventKind::Message(message) = &mut follow_chat.kind {
        message.mentioned_agent_ids.clear();
    }
    state.emit(follow_chat).await?;
    let follow_storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    follow_storage
        .set_chat_attention(
            &workspace_id,
            &agent_id,
            &channel_id,
            "application-steer-chat",
            &AttentionRecord::Always,
            100,
        )
        .map_err(err)?;
    follow_storage.close().map_err(err)?;
    state
        .emit(message(
            &channel_id,
            "application-steer-chat",
            "application-steer-message",
            "please adjust the active work",
            &agent_id,
            "human",
        ))
        .await?;
    let verification_storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    let waiting_message = verification_storage
        .inbox_items(&workspace_id, &agent_id)
        .map_err(err)?
        .into_iter()
        .max_by_key(|item| item.inbox_item_id)
        .ok_or("waiting Inbox item missing")?;
    ensure(
        waiting_message.state == InboxState::Pending
            && waiting_message.claimed_execution_id.is_none(),
        "messages received during active work wait for the next business run",
    )?;
    verification_storage.close().map_err(err)?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server_socket = socket.to_path_buf();
    let peer = RuntimePeerAdapter::new(application.clone());
    let server = tokio::spawn(async move {
        serve_uds(
            server_socket,
            peer,
            TransportConfig {
                protocol_version: PROTOCOL_VERSION,
                max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            },
            async move {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ensure(socket.exists(), "runtime UDS started")?;
    let mut client = connect_uds(socket, DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(err)?;
    let listed = client
        .list_workspaces(ListWorkspacesRequest {
            request_id: "list-1".into(),
        })
        .await
        .map_err(|error| format!("list workspaces: {error}"))?
        .into_inner();
    ensure(listed.workspaces.len() == 1, "restored workspace")?;
    ensure(
        !format!("{listed:?}").contains("must-not-leak"),
        "secret isolation",
    )?;
    let reactions = client
        .get_reaction_catalog(lankide_protocol::CatalogRequest {
            request_id: "reaction-catalog".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        reactions.reactions.len() == 1 && reactions.reactions[0].reaction_id == "fixture_reaction",
        "management reaction catalog must come from workspace Provider",
    )?;
    let interface_catalog = client
        .list_provider_interfaces(lankide_protocol::ListProviderInterfacesRequest {
            request_id: "interface-list-1".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
        })
        .await
        .map_err(|error| format!("provider interface catalog: {error}"))?
        .into_inner();
    ensure(
        interface_catalog
            .interfaces
            .iter()
            .map(|interface| interface.interface_id.as_str())
            .collect::<Vec<_>>()
            == ["fixture.alpha", "fixture.echo"]
            && interface_catalog.interfaces.iter().all(|interface| {
                serde_json::from_str::<Value>(&interface.input_schema_json).is_ok()
                    && serde_json::from_str::<Value>(&interface.output_schema_json).is_ok()
            }),
        "provider interface catalog ordering and schema",
    )?;
    let interface = client
        .invoke_provider_interface(lankide_protocol::InvokeProviderInterfaceRequest {
            request_id: "interface-1".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            interface_id: "fixture.echo".into(),
            input_json: json!({"value":"ok"}).to_string(),
        })
        .await
        .map_err(|error| format!("provider interface: {error}"))?
        .into_inner();
    ensure(
        interface.output_json == json!({"value":"ok"}).to_string(),
        "provider interface output",
    )?;
    let unknown_interface = client
        .invoke_provider_interface(lankide_protocol::InvokeProviderInterfaceRequest {
            request_id: "interface-unknown".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            interface_id: "fixture.unknown".into(),
            input_json: "{}".into(),
        })
        .await
        .expect_err("unknown provider interface must fail");
    ensure(
        unknown_interface.code() == tonic::Code::NotFound,
        "unknown provider interface status",
    )?;
    let invalid_interface = client
        .invoke_provider_interface(lankide_protocol::InvokeProviderInterfaceRequest {
            request_id: "interface-invalid".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            interface_id: "fixture.echo".into(),
            input_json: "[]".into(),
        })
        .await
        .expect_err("invalid provider interface input must fail");
    ensure(
        invalid_interface.code() == tonic::Code::InvalidArgument,
        "invalid provider interface status",
    )?;
    let mut subscription = client
        .subscribe_management_events(ManagementSubscription {
            request_id: "subscription-1".into(),
            workspace: None,
            after_sequence: 0,
        })
        .await
        .map_err(err)?
        .into_inner();
    let workspace_a_sink = state
        .sink
        .lock()
        .map_err(err)?
        .clone()
        .ok_or("workspace A event sink")?;
    client
        .create_workspace(CreateWorkspaceRequest {
            request_id: "create-workspace-1".into(),
            workspace_id: "application_b".into(),
            name: "Application B".into(),
            rules: "Be helpful".into(),
            provider_id: provider_id.to_string(),
            provider_config_json: "{}".into(),
            provider_secrets_json: json!({"token":"runtime-secret"}).to_string(),
        })
        .await
        .map_err(err)?;
    let event = subscription
        .message()
        .await
        .map_err(err)?
        .ok_or("missing management event")?;
    ensure(
        event
            .workspace
            .is_some_and(|value| value.workspace_id == "application_b"),
        "management subscription",
    )?;
    let created_agent = client
        .create_agent(CreateAgentRequest {
            request_id: "create-agent-1".into(),
            agent: Some(AgentRef {
                workspace_id: "application_b".into(),
                agent_id: "agent_b".into(),
            }),
            name: "Agent B".into(),
            role: "assistant".into(),
            bio: String::new(),
            harness_id: "fixture".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            auto_reaction: String::new(),
            soul: String::new(),
            cli_path: String::new(),
            environment_variables: Vec::new(),
            computer_id: "computer-b".into(),
            attention: Some(AgentAttention {
                channels: vec![ChannelAttention {
                    channel_id: "chat".into(),
                    mode: AttentionMode::Always as i32,
                    interval_minutes: 0,
                }],
            }),
            provider_agent_config_json: json!({"tenant":"opaque"}).to_string(),
            provider_agent_secrets_json: json!({"bot_token":"agent-private"}).to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        created_agent.attention.is_some_and(|value| {
            value.channels.len() == 4
                && value.channels.iter().any(|channel| {
                    channel.channel_id == "chat" && channel.mode == AttentionMode::Always as i32
                })
        }),
        "agent attention roundtrip",
    )?;
    let updated_agent = client
        .update_agent(UpdateAgentRequest {
            request_id: "update-agent-1".into(),
            agent: created_agent.agent.clone(),
            name: Some("Agent B Updated".into()),
            role: None,
            bio: None,
            harness_id: None,
            model: None,
            thinking_level: None,
            auto_reaction: None,
            soul: None,
            cli_path: None,
            environment_variables: None,
            computer_id: None,
            avatar: None,
            attention: None,
            provider_agent_config_json: Some(json!({"tenant":"opaque-updated"}).to_string()),
            provider_agent_secrets_json: None,
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        updated_agent.provider_agent_config_json == json!({"tenant":"opaque-updated"}).to_string(),
        "provider agent config update roundtrip",
    )?;
    client
        .update_workspace(UpdateWorkspaceRequest {
            request_id: "update-workspace-1".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: "application_b".into(),
            }),
            name: None,
            rules: None,
            provider_config_json: Some(json!({"revision":2}).to_string()),
            cover_resource_id: None,
            provider_secrets_json: Some(json!({"token":"updated-secret"}).to_string()),
        })
        .await
        .map_err(err)?;
    let lifecycle_log = state.log.lock().map_err(err)?.clone();
    ensure(
        lifecycle_log.iter().any(|entry| {
            entry.contains("create_workspace:") && entry.contains("secret_present=true")
        }),
        "Provider create_workspace did not receive staged secrets",
    )?;
    ensure(
        lifecycle_log
            .iter()
            .any(|entry| entry.contains("update_workspace:") && entry.contains("revision")),
        "Provider update_workspace did not receive opaque configs",
    )?;
    ensure(
        lifecycle_log
            .iter()
            .any(|entry| entry.contains("create_agent:") && entry.contains("opaque")),
        "Provider create_agent did not receive opaque config",
    )?;
    ensure(
        lifecycle_log
            .iter()
            .any(|entry| entry.contains("update_agent:") && entry.contains("opaque-updated")),
        "Provider update_agent did not receive old and new opaque configs",
    )?;
    state.fail_lifecycle.store(true, Ordering::SeqCst);
    let failed = client
        .create_workspace(CreateWorkspaceRequest {
            request_id: "create-workspace-failed".into(),
            workspace_id: "provider_failed".into(),
            name: "Provider Failed".into(),
            rules: String::new(),
            provider_id: provider_id.to_string(),
            provider_config_json: json!({"must_not_commit":true}).to_string(),
            provider_secrets_json: json!({"token":"must-not-persist"}).to_string(),
        })
        .await;
    ensure(
        failed.is_err_and(|status| status.code() == tonic::Code::FailedPrecondition),
        "provider lifecycle failure was not reported as a recoverable Provider problem",
    )?;
    state.fail_lifecycle.store(false, Ordering::SeqCst);
    let listed_after_failure = client
        .list_workspaces(ListWorkspacesRequest {
            request_id: "list-after-provider-failure".into(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !listed_after_failure.workspaces.iter().any(|workspace| {
            workspace
                .workspace
                .as_ref()
                .is_some_and(|value| value.workspace_id == "provider_failed")
        }),
        "failed Provider operation left a Workspace",
    )?;
    let onboarding = client
        .get_agent_onboarding(lankide_protocol::GetAgentOnboardingRequest {
            request_id: "agent-onboarding-1".into(),
            agent: Some(AgentRef {
                workspace_id: "application_b".into(),
                agent_id: "agent_b".into(),
            }),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        onboarding.status == lankide_protocol::OnboardingStatus::Ready as i32,
        "agent onboarding",
    )?;
    let onboarding_payload = json!({
        "device_code": "runtime-e2e-device",
        "verification_uri": "https://example.invalid/verify"
    });
    let advanced_onboarding = client
        .advance_agent_onboarding(lankide_protocol::AdvanceAgentOnboardingRequest {
            request_id: "agent-onboarding-2".into(),
            agent: Some(AgentRef {
                workspace_id: "application_b".into(),
                agent_id: "agent_b".into(),
            }),
            action: "start".into(),
            field_values_json: onboarding_payload.to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        advanced_onboarding.fields.len() == 1
            && advanced_onboarding.fields[0].name == "opaque"
            && serde_json::from_str::<Value>(&advanced_onboarding.fields[0].value)
                .ok()
                .as_ref()
                == Some(&onboarding_payload),
        "agent onboarding opaque payload did not survive Runtime protocol conversion",
    )?;

    let context = AgentRunContext {
        execution: Some(ExecutionRef {
            execution_id: execution.execution_id,
            attempt: 1,
        }),
        execution_token: token.into(),
    };
    let denied = client
        .get_content(ContentRequest {
            context: Some(AgentRunContext {
                execution: context.execution.clone(),
                execution_token: "wrong-token".into(),
            }),
            kind: ContentKind::Message as i32,
            content_id: ingested.message_id.to_string(),
        })
        .await
        .expect_err("invalid execution token must be rejected");
    ensure(
        denied.code() == tonic::Code::PermissionDenied
            && !denied.message().contains("token_hash")
            && !denied.message().contains("must-not-leak"),
        "capability authorization is redacted",
    )?;
    let content = client
        .get_content(ContentRequest {
            context: Some(context.clone()),
            kind: ContentKind::Message as i32,
            content_id: ingested.message_id.to_string(),
        })
        .await
        .map_err(|error| format!("get content: {error}"))?
        .into_inner();
    ensure(
        content.content_markdown == application_trigger,
        "content gateway",
    )?;
    let profile = client
        .get_self_profile(context.clone())
        .await
        .map_err(|error| format!("get self profile: {error}"))?
        .into_inner();
    ensure(
        profile.agent.as_ref().is_some_and(|agent| {
            agent.workspace_id == workspace_id.as_str() && agent.agent_id == agent_id.as_str()
        }),
        "self profile was not derived from execution context",
    )?;
    let profile = client
        .update_self_profile(UpdateSelfProfileRequest {
            context: Some(context.clone()),
            role: Some("Runtime editor".into()),
            ..Default::default()
        })
        .await
        .map_err(|error| format!("update self profile: {error}"))?
        .into_inner();
    ensure(profile.role == "Runtime editor", "self profile update")?;
    ensure(
        !execution.snapshot.chats.iter().any(|chat| {
            chat.recent_messages
                .iter()
                .chain(&chat.trigger_messages)
                .any(|message| message.message_id == oldest_message_id)
        }),
        "historical content must be outside the short prompt preview",
    )?;
    let historical = client
        .get_content(ContentRequest {
            context: Some(context.clone()),
            kind: ContentKind::Message as i32,
            content_id: oldest_message_id.to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        historical.content_markdown == format!("historical 0: {application_trigger}"),
        "full historical content remains readable within the frozen chat",
    )?;
    client
        .update_agent(UpdateAgentRequest {
            request_id: "change-config-during-execution".into(),
            agent: Some(AgentRef {
                workspace_id: workspace_id.to_string(),
                agent_id: agent_id.to_string(),
            }),
            name: None,
            role: None,
            bio: None,
            harness_id: None,
            model: None,
            thinking_level: None,
            auto_reaction: None,
            soul: None,
            cli_path: None,
            environment_variables: None,
            computer_id: None,
            avatar: None,
            attention: None,
            provider_agent_config_json: Some(json!({"deny_echo":true}).to_string()),
            provider_agent_secrets_json: None,
        })
        .await
        .map_err(|error| format!("mid-execution config update: {error}"))?;
    let listed_capabilities = client
        .list_capabilities(context.clone())
        .await
        .map_err(|error| format!("list capabilities: {error}"))?
        .into_inner();
    ensure(
        [
            "message_send",
            "message_recall",
            "message_edit",
            "chat_messages",
            "unread_current",
            "attention_set",
            "echo",
            "runtime_lookup",
        ]
        .iter()
        .all(|name| {
            listed_capabilities
                .capabilities
                .iter()
                .any(|capability| &capability.name == name)
        }),
        "frozen capability grant",
    )?;
    let unread = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "unread_current".into(),
            invocation_id: "unread-1".into(),
            arguments_json: "{}".into(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        unread.success && unread.structured_content_json.contains("application-chat"),
        "unread current",
    )?;
    let recall = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_recall".into(),
            invocation_id: "recall-1".into(),
            arguments_json: json!({"message_id": 1}).to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !recall.success
            && recall
                .error
                .is_some_and(|error| error.code == "message_not_found"),
        "recall unknown internal message result",
    )?;
    let unauthorized = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "guessed".into(),
            invocation_id: "guessed-1".into(),
            arguments_json: "{}".into(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !unauthorized.success
            && unauthorized
                .error
                .is_some_and(|error| error.code == "not_authorized"),
        "unauthorized capability",
    )?;
    for _ in 0..2 {
        let invoked = client
            .invoke_capability(CapabilityInvokeRequest {
                context: Some(context.clone()),
                capability_name: "echo".into(),
                invocation_id: "echo-idempotent".into(),
                arguments_json: json!({"value":"once"}).to_string(),
            })
            .await
            .map_err(err)?
            .into_inner();
        ensure(invoked.success, "provider capability")?;
    }
    ensure(
        state.tool_calls.load(Ordering::SeqCst) == 1,
        "tool call idempotency",
    )?;
    // A delayed external Tool must not block the same transport's heartbeat.
    let (computer_frames, computer_input) = tokio::sync::mpsc::channel(8);
    computer_frames
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::Hello(ComputerHello {
                protocol_version: PROTOCOL_VERSION,
                computer_id: "computer-a".into(),
                instance_id: "instance-2".into(),
                computer_version: "test".into(),
                os: "test".into(),
                arch: "test".into(),
                harnesses: vec![lankide_protocol::HarnessInfo {
                    harness_id: "pi".into(),
                    display_name: "Pi".into(),
                    availability: lankide_protocol::HarnessAvailability::Available as i32,
                    ..Default::default()
                }],
            })),
        })
        .await
        .map_err(err)?;
    let mut computer_output = client
        .connect_computer(tokio_stream::wrappers::ReceiverStream::new(computer_input))
        .await
        .map_err(err)?
        .into_inner();
    let _ = computer_output.message().await.map_err(err)?;
    let gate = Arc::new(tokio::sync::Notify::new());
    *state.tool_gate.lock().unwrap() = Some(gate.clone());
    computer_frames
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::ToolCall(
                lankide_protocol::ToolCall {
                    execution: context.execution.clone(),
                    execution_token: token.into(),
                    tool_call_id: "slow-echo".into(),
                    tool_name: "echo".into(),
                    arguments_json: json!({"value":"delayed"}).to_string(),
                },
            )),
        })
        .await
        .map_err(err)?;
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.tool_calls.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "delayed tool never started")?;
    let heartbeat_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    computer_frames
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::Heartbeat(ComputerHeartbeat {
                computer_id: "computer-a".into(),
                instance_id: "instance-2".into(),
                sent_at: heartbeat_at,
                active_attempts: vec![ActiveAttempt {
                    execution: context.execution.clone(),
                    last_event_sequence: 0,
                    harness_process_state: "active".into(),
                }],
            })),
        })
        .await
        .map_err(err)?;
    let heartbeat_storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let current = heartbeat_storage
                .execution_attempt(execution.execution_id, 1)
                .unwrap()
                .unwrap();
            if current.lease_expires_at == heartbeat_at + 15_000 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "slow tool blocked heartbeat renewal")?;
    heartbeat_storage.close().map_err(err)?;
    *state.tool_gate.lock().unwrap() = None;
    gate.notify_one();
    let tool_result = tokio::time::timeout(Duration::from_secs(2), computer_output.message())
        .await
        .map_err(err)?
        .map_err(err)?
        .ok_or("tool stream closed")?;
    ensure(
        matches!(tool_result.frame, Some(runtime_frame::Frame::ToolResult(result)) if result.success),
        "delayed tool result delivered after heartbeat",
    )?;
    let provider_lookup = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "runtime_lookup".into(),
            invocation_id: "runtime-lookup-1".into(),
            arguments_json: json!({"message_id":ingested.message_id}).to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    let provider_lookup_content =
        serde_json::from_str::<Value>(&provider_lookup.structured_content_json).map_err(err)?;
    ensure(
        provider_lookup.success
            && provider_lookup_content["message_id"] == ingested.message_id
            && provider_lookup_content["provider_message_id"] == "application-message-1"
            && provider_lookup_content["channel_id"] == "chat"
            && provider_lookup_content["chat_id"] == "application-chat",
        "Provider Runtime message SDK",
    )?;
    let resource_bytes = b"application-resource".to_vec();
    let resource_sha = Sha256::digest(&resource_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let uploaded = client
        .upload_resource(tokio_stream::iter(vec![ResourceChunk {
            transfer_id: "upload-1".into(),
            resource_id: 0,
            offset: 0,
            bytes: resource_bytes.clone(),
            eof: true,
            descriptor: Some(ResourceDescriptor {
                file_name: "note.txt".into(),
                media_type: "text/plain".into(),
                size_bytes: resource_bytes.len() as u64,
                sha256: resource_sha,
                context: Some(context.clone()),
            }),
        }]))
        .await
        .map_err(|error| format!("upload resource: {error}"))?
        .into_inner();
    let mut downloaded = client
        .download_resource(ResourceRequest {
            resource_id: uploaded.resource_id,
            transfer_id: "download-1".into(),
            context: Some(context.clone()),
        })
        .await
        .map_err(|error| format!("download resource: {error}"))?
        .into_inner();
    let mut downloaded_bytes = Vec::new();
    while let Some(chunk) = downloaded.message().await.map_err(err)? {
        downloaded_bytes.extend(chunk.bytes);
    }
    ensure(downloaded_bytes == resource_bytes, "resource roundtrip")?;
    let image_bytes = include_bytes!("assets/profile.png").to_vec();
    let image = client
        .upload_workspace_image(lankide_protocol::UploadWorkspaceImageRequest {
            request_id: "upload-profile-image".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            file_name: "profile.png".into(),
            media_type: "image/png".into(),
            bytes: image_bytes.clone(),
        })
        .await
        .map_err(err)?
        .into_inner()
        .resource
        .ok_or("uploaded profile image metadata missing")?;
    for invalid_cover in [i64::MAX, uploaded.resource_id] {
        client
            .update_workspace(lankide_protocol::UpdateWorkspaceRequest {
                request_id: "invalid-cover".into(),
                workspace: Some(WorkspaceRef {
                    workspace_id: workspace_id.to_string(),
                }),
                cover_resource_id: Some(invalid_cover),
                ..Default::default()
            })
            .await
            .expect_err("missing or non-image cover must not commit");
        let unchanged = client
            .get_workspace(lankide_protocol::GetWorkspaceRequest {
                request_id: "unchanged-after-invalid-cover".into(),
                workspace: Some(WorkspaceRef {
                    workspace_id: workspace_id.to_string(),
                }),
            })
            .await
            .map_err(err)?
            .into_inner();
        ensure(
            unchanged.cover.is_none(),
            "invalid cover did not corrupt Workspace listing",
        )?;
    }
    client
        .update_workspace(lankide_protocol::UpdateWorkspaceRequest {
            request_id: "set-profile-image".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            cover_resource_id: Some(image.resource_id),
            ..Default::default()
        })
        .await
        .map_err(err)?;
    let response = client
        .get_workspace_image(lankide_protocol::WorkspaceImageRequest {
            request_id: "profile-image".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            resource_id: image.resource_id,
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        response.bytes == image_bytes && response.resource == Some(image.clone()),
        "real profile image resource bytes",
    )?;
    let denied_image = client
        .get_workspace_image(lankide_protocol::WorkspaceImageRequest {
            request_id: "not-a-profile".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: workspace_id.to_string(),
            }),
            resource_id: uploaded.resource_id,
        })
        .await
        .expect_err("attachment must not be exposed as a profile image");
    ensure(
        denied_image.code() == tonic::Code::PermissionDenied,
        "profile resource allowlist",
    )?;
    let denied_workspace = client
        .get_workspace_image(lankide_protocol::WorkspaceImageRequest {
            request_id: "wrong-image-workspace".into(),
            workspace: Some(WorkspaceRef {
                workspace_id: "application_b".into(),
            }),
            resource_id: image.resource_id,
        })
        .await
        .expect_err("image must remain workspace-scoped");
    ensure(
        denied_workspace.code() == tonic::Code::PermissionDenied,
        "profile image workspace isolation",
    )?;

    state.push_send(SendMode::Sent("standard-tool-send".into()));
    let standard_send = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_send".into(),
            invocation_id: "standard-send-1".into(),
            arguments_json: json!({
                "channel_id": "chat",
                "chat_id": "application-chat",
                "content": "standard reply",
                "reply_to": ingested.message_id,
                "attachments": [{
                    "resource_id": uploaded.resource_id,
                    "file_name": uploaded.file_name.clone(),
                    "media_type": uploaded.media_type.clone(),
                    "size_bytes": uploaded.size_bytes,
                    "sha256": uploaded.sha256.clone()
                }]
            })
            .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(standard_send.success, "standard message send")?;
    let sent_message_id = serde_json::from_str::<Value>(&standard_send.structured_content_json)
        .map_err(err)?["message_id"]
        .as_i64()
        .ok_or("standard send message id")?;
    let unsupported_edit = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_edit".into(),
            invocation_id: "edit-unsupported".into(),
            arguments_json: json!({"message_id":sent_message_id,"content":"edited reply"})
                .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !unsupported_edit.success
            && unsupported_edit
                .error
                .is_some_and(|error| error.code == "edit_unsupported"),
        "edit unsupported result",
    )?;
    for (id, arguments, expected_code) in [
        (
            "edit-unsupported",
            json!({"message_id":sent_message_id,"content":"edited reply"}),
            "edit_unsupported",
        ),
        ("invalid-edit-input", json!({}), "tool_unavailable"),
    ] {
        computer_frames
            .send(ComputerFrame {
                frame: Some(computer_frame::Frame::ToolCall(
                    lankide_protocol::ToolCall {
                        execution: context.execution.clone(),
                        execution_token: token.into(),
                        tool_call_id: id.into(),
                        tool_name: "message_edit".into(),
                        arguments_json: arguments.to_string(),
                    },
                )),
            })
            .await
            .map_err(err)?;
        let response = tokio::time::timeout(Duration::from_secs(2), computer_output.message())
            .await
            .map_err(err)?
            .map_err(err)?
            .ok_or("tool failure stream closed")?;
        let Some(runtime_frame::Frame::ToolResult(result)) = response.frame else {
            return Err("tool failure was not returned to Computer".into());
        };
        ensure(
            !result.success
                && result.tool_call_id == id
                && result
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == expected_code),
            "tool failure must reach the native session",
        )?;
        serde_json::from_str::<Value>(&result.structured_content_json)
            .map_err(|_| "tool failure payload must be consumable by Computer")?;
    }
    state.edit_supported.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        let edited = client
            .invoke_capability(CapabilityInvokeRequest {
                context: Some(context.clone()),
                capability_name: "message_edit".into(),
                invocation_id: "edit-idempotent".into(),
                arguments_json: json!({"message_id":sent_message_id,"content":"edited reply"})
                    .to_string(),
            })
            .await
            .map_err(err)?
            .into_inner();
        ensure(edited.success, "supported edit result")?;
    }
    ensure(
        state.edit_calls.load(Ordering::SeqCst) == 2,
        "edit tool call idempotency",
    )?;
    let unsupported_recall = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_recall".into(),
            invocation_id: "recall-unsupported".into(),
            arguments_json: json!({"message_id":sent_message_id}).to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !unsupported_recall.success
            && unsupported_recall
                .error
                .is_some_and(|error| error.code == "recall_unsupported"),
        "recall unsupported result",
    )?;
    state.recall_failures.store(1, Ordering::SeqCst);
    let failed_recall = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_recall".into(),
            invocation_id: "recall-provider-error".into(),
            arguments_json: json!({"message_id":sent_message_id}).to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !failed_recall.success
            && failed_recall
                .error
                .is_some_and(|error| error.code == "recall_failed"),
        "recall Provider error is a Tool result",
    )?;
    state.recall_supported.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        let recalled = client
            .invoke_capability(CapabilityInvokeRequest {
                context: Some(context.clone()),
                capability_name: "message_recall".into(),
                invocation_id: "recall-idempotent".into(),
                arguments_json: json!({"message_id":sent_message_id}).to_string(),
            })
            .await
            .map_err(err)?
            .into_inner();
        ensure(recalled.success, "supported recall result")?;
    }
    ensure(
        state.recall_calls.load(Ordering::SeqCst) == 3,
        "recall tool call idempotency",
    )?;
    state.push_send(SendMode::Sent("attachment-only-send".into()));
    let attachment_only = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_send".into(),
            invocation_id: "attachment-only-send".into(),
            arguments_json: json!({
                "channel_id":"chat", "chat_id":"application-chat",
                "attachments":[{
                    "resource_id":uploaded.resource_id,
                    "file_name":uploaded.file_name.clone(),
                    "media_type":uploaded.media_type.clone(),
                    "size_bytes":uploaded.size_bytes,
                    "sha256":uploaded.sha256.clone()
                }]
            })
            .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(attachment_only.success, "attachment-only standard send")?;

    let attention = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "attention_set".into(),
            invocation_id: "attention-application-chat".into(),
            arguments_json: json!({
                "channel_id": "chat",
                "chat_id": "application-chat",
                "level": "mute"
            })
            .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        attention.success,
        &format!("per-Chat attention tool: {attention:?}"),
    )?;
    let attention_storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    ensure(
        attention_storage
            .chat_attention(&workspace_id, &agent_id, &channel_id, "application-chat")
            .map_err(err)?
            == Some(AttentionRecord::Mute),
        "attention is stored for one Chat",
    )?;
    let inbox_before_muted_message = attention_storage
        .inbox_items(&workspace_id, &agent_id)
        .map_err(err)?
        .len();
    attention_storage.close().map_err(err)?;

    workspace_a_sink
        .emit(message(
            &channel_id,
            "application-chat",
            "application-message-2",
            "new while running",
            &agent_id,
            "human",
        ))
        .await
        .map_err(err)?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let muted_storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    ensure(
        muted_storage
            .inbox_items(&workspace_id, &agent_id)
            .map_err(err)?
            .len()
            == inbox_before_muted_message,
        "muting one Chat suppresses its later dispatch",
    )?;
    muted_storage.close().map_err(err)?;
    let frozen_messages = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "chat_messages".into(),
            invocation_id: "messages-current-1".into(),
            arguments_json: json!({"channel_id":"chat","chat_id":"application-chat","limit":50})
                .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        frozen_messages.success
            && frozen_messages
                .structured_content_json
                .contains("application trigger")
            && !frozen_messages
                .structured_content_json
                .contains("new while running")
            && !frozen_messages
                .structured_content_json
                .contains("standard reply"),
        "chat messages frozen boundary",
    )?;
    let history: Value =
        serde_json::from_str(&frozen_messages.structured_content_json).map_err(err)?;
    ensure(
        history["messages"].as_array().is_some_and(|messages| {
            messages.iter().any(|message| {
                message["message_id"] == ingested.message_id
                    && message["content"] == application_trigger
            })
        }),
        "chat_messages must return the full long message, including its tail",
    )?;
    let guarded = client
        .send_message(SendMessageRequest {
            context: Some(context.clone()),
            channel_id: channel_id.to_string(),
            chat_id: "application-chat".into(),
            content_markdown: "reply".into(),
            reply_to_message_id: None,
            resources: vec![uploaded],
            idempotency_key: "application-send-guarded".into(),
            force: false,
        })
        .await
        .map_err(|error| format!("guarded send: {error}"))?
        .into_inner();
    ensure(
        !guarded.accepted
            && guarded
                .error
                .is_some_and(|value| value.code == "new_messages"),
        "new messages guard",
    )?;
    let refreshed_messages = client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "chat_messages".into(),
            invocation_id: "messages-after-guard-1".into(),
            arguments_json: json!({"channel_id":"chat","chat_id":"application-chat","limit":50})
                .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        refreshed_messages.success
            && refreshed_messages
                .structured_content_json
                .contains("new while running"),
        "the message that triggered the send guard becomes readable in the same execution",
    )?;
    state.push_send(SendMode::Sent("application-provider-send".into()));
    let forced = client
        .send_message(SendMessageRequest {
            context: Some(context),
            channel_id: channel_id.to_string(),
            chat_id: "application-chat".into(),
            content_markdown: "reply".into(),
            reply_to_message_id: None,
            resources: Vec::new(),
            idempotency_key: "application-send-forced".into(),
            force: true,
        })
        .await
        .map_err(|error| format!("forced send: {error}"))?
        .into_inner();
    ensure(forced.accepted, "forced send")?;
    let provider_send_keys = state.provider_send_keys.lock().map_err(err)?.clone();
    ensure(
        provider_send_keys.last().is_some_and(|key| key.len() == 64)
            && provider_send_keys
                .iter()
                .all(|key| key != "application-send-forced"),
        "Provider send idempotency is scoped to the authorized execution instead of exposing a resettable local request key",
    )?;

    let deleted_agent = client
        .create_agent(CreateAgentRequest {
            request_id: "create-agent-for-delete".into(),
            agent: Some(AgentRef {
                workspace_id: "application_b".into(),
                agent_id: "agent_for_delete".into(),
            }),
            name: "Agent for delete".into(),
            role: String::new(),
            bio: String::new(),
            harness_id: "fixture".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            auto_reaction: String::new(),
            soul: String::new(),
            cli_path: String::new(),
            environment_variables: Vec::new(),
            computer_id: "computer-delete".into(),
            attention: None,
            provider_agent_config_json: "{}".into(),
            provider_agent_secrets_json: "{}".into(),
        })
        .await
        .map_err(|error| format!("create Agent for delete: {error}"))?
        .into_inner();
    let deleted = client
        .delete_agent(DeleteAgentRequest {
            request_id: "delete-agent-rpc".into(),
            agent: deleted_agent.agent.clone(),
        })
        .await
        .map_err(|error| format!("delete Agent RPC: {error}"))?
        .into_inner();
    ensure(
        deleted.request_id == "delete-agent-rpc" && deleted.agent == deleted_agent.agent,
        "delete Agent RPC response",
    )?;
    let delete_workspace = WorkspaceId::new("application_b").map_err(err)?;
    let delete_agent = AgentId::new("agent_for_delete").map_err(err)?;
    let deletion_storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    ensure(
        deletion_storage
            .agent_record(&delete_workspace, &delete_agent)
            .is_err()
            && deletion_storage
                .pending_agent_file_deletions("computer-delete")
                .map_err(err)?
                == vec![(delete_workspace.clone(), delete_agent.clone())],
        "Agent deletion must commit before Computer cleanup",
    )?;
    deletion_storage.close().map_err(err)?;

    let (delete_computer_frames, delete_computer_input) = tokio::sync::mpsc::channel(4);
    delete_computer_frames
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::Hello(ComputerHello {
                protocol_version: PROTOCOL_VERSION,
                computer_id: "computer-delete".into(),
                instance_id: "instance-delete".into(),
                computer_version: "test".into(),
                os: "test".into(),
                arch: "test".into(),
                harnesses: Vec::new(),
            })),
        })
        .await
        .map_err(err)?;
    let mut delete_computer_output = client
        .connect_computer(tokio_stream::wrappers::ReceiverStream::new(
            delete_computer_input,
        ))
        .await
        .map_err(err)?
        .into_inner();
    let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frame = delete_computer_output
                .message()
                .await
                .map_err(err)?
                .ok_or("delete Computer stream closed")?;
            if let Some(runtime_frame::Frame::DeleteAgentFiles(request)) = frame.frame {
                return Ok::<_, String>(request);
            }
        }
    })
    .await
    .map_err(|_| "deferred Agent cleanup timed out".to_owned())??;
    ensure(
        cleanup.agent == deleted.agent,
        "deferred Agent cleanup target",
    )?;
    delete_computer_frames
        .send(ComputerFrame {
            frame: Some(computer_frame::Frame::DeleteAgentFilesResult(
                lankide_protocol::DeleteAgentFilesResult {
                    request_id: cleanup.request_id,
                    success: true,
                    error_code: String::new(),
                    error_message: String::new(),
                },
            )),
        })
        .await
        .map_err(err)?;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let storage = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
            let pending = storage
                .pending_agent_file_deletions("computer-delete")
                .map_err(err)?;
            storage.close().map_err(err)?;
            if pending.is_empty() {
                return Ok::<_, String>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "deferred Agent cleanup acknowledgement timed out".to_owned())??;

    let recreated_agent = client
        .create_agent(CreateAgentRequest {
            request_id: "recreate-agent-rpc".into(),
            agent: Some(AgentRef {
                workspace_id: "application_b".into(),
                agent_id: "agent_for_delete".into(),
            }),
            name: "Recreated Agent".into(),
            role: String::new(),
            bio: String::new(),
            harness_id: "fixture".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            auto_reaction: String::new(),
            soul: String::new(),
            cli_path: String::new(),
            environment_variables: Vec::new(),
            computer_id: "computer-delete".into(),
            attention: None,
            provider_agent_config_json: "{}".into(),
            provider_agent_secrets_json: "{}".into(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        recreated_agent.name == "Recreated Agent"
            && recreated_agent
                .agent
                .is_some_and(|agent| agent.agent_id == "agent_for_delete"),
        "a deleted Agent ID must be reusable",
    )?;

    drop(subscription);
    drop(delete_computer_frames);
    drop(delete_computer_output);
    drop(computer_frames);
    drop(computer_output);
    drop(client);
    shutdown_tx
        .send(())
        .map_err(|_| "server shutdown".to_owned())?;
    server.await.map_err(err)?.map_err(err)?;
    application.shutdown().await.map_err(err)?;
    drop(application);
    let failed_workspace = WorkspaceId::new("provider_failed").map_err(err)?;
    let storage_after_failure = Storage::open(DatabaseConfig::new(db)).map_err(err)?;
    let workspace_b = WorkspaceId::new("application_b").map_err(err)?;
    let agent_b = AgentId::new("agent_b").map_err(err)?;
    ensure(
        storage_after_failure
            .workspace_provider_state(&workspace_b)
            .map_err(err)?
            == json!({
                "created_by_provider": true,
                "updated_by_provider": true
            }),
        "Workspace Provider state did not survive management update",
    )?;
    let stored_agent_provider_state = storage_after_failure
        .agent_provider_state(&workspace_b, &agent_b)
        .map_err(err)?;
    ensure(
        stored_agent_provider_state
            == json!({
                "created_by_provider": true,
                "updated_by_provider": true
            }),
        &format!(
            "Agent Provider state did not survive management update: {stored_agent_provider_state}"
        ),
    )?;
    ensure(
        storage_after_failure
            .read_secret(&failed_workspace, "token")
            .map_err(err)?
            .is_none(),
        "failed Provider operation left an orphan secret",
    )?;
    storage_after_failure.close().map_err(err)?;

    let restarted = RuntimeApplication::open(config, factories)
        .await
        .map_err(err)?;
    let report = restarted.recover(10).map_err(err)?;
    ensure(
        report
            .recovering_execution_ids
            .contains(&execution.execution_id),
        "restart keeps active execution recoverable",
    )?;
    restarted.start().await.map_err(err)?;
    ensure(
        state.inits.load(Ordering::SeqCst) >= 4,
        "two workspace restart",
    )?;
    restarted.shutdown().await.map_err(err)?;
    Ok(())
}

struct ApplicationFactory {
    provider_id: ProviderId,
    state: Arc<FakeState>,
}
impl ProviderTypeFactory for ApplicationFactory {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.provider_id.clone(),
            name: "Fixture".into(),
            description: "Messaging fixture".into(),
            version: "1".into(),
            icon: String::new(),
        }
    }
    fn create(
        &self,
        workspace: &WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn lankide_provider::ProviderHostInstance>, ProviderError> {
        Ok(Arc::new(lankide_provider::ProviderHost::new(
            FakeProvider {
                id: self.provider_id.clone(),
                state: self.state.clone(),
            },
            workspace.id.clone(),
            workspace.provider_config.clone(),
            services,
        )?))
    }
}

fn register(
    runtime: &mut Runtime,
    workspace: WorkspaceId,
    provider_id: ProviderId,
    state: Arc<FakeState>,
) -> Result<(), String> {
    let factory_workspace = workspace.clone();
    runtime
        .register_provider_factory(workspace, move || {
            lankide_provider::ProviderHost::new(
                FakeProvider {
                    id: provider_id.clone(),
                    state: state.clone(),
                },
                factory_workspace.clone(),
                json!({"workspace": factory_workspace.as_str()}),
                services(),
            )
        })
        .map_err(err)
}

fn agent_config(
    workspace: &WorkspaceId,
    agent: &AgentId,
    channel: &ChannelId,
    provider_sender_id: &str,
) -> AgentRuntimeConfig {
    AgentRuntimeConfig {
        agent_id: agent.clone(),
        provider_sender_id: provider_sender_id.into(),
        attention: BTreeMap::from([
            (channel.clone(), AttentionPolicy::Mention),
            (
                ChannelId::new("always").expect("always id"),
                AttentionPolicy::Always,
            ),
            (
                ChannelId::new("scheduled").expect("scheduled id"),
                AttentionPolicy::Schedule(ScheduleInterval::Minutes5),
            ),
            (
                ChannelId::new("muted").expect("muted id"),
                AttentionPolicy::Mute,
            ),
        ]),
        auto_reaction: Some("eyes".into()),
        agent_config: json!({"soul": "test"}),
        provider_agent_config: json!({"tenant": "fixture"}),
        agent_profile: AgentRuntimeProfile {
            agent: Some(AgentRef {
                workspace_id: workspace.to_string(),
                agent_id: agent.to_string(),
            }),
            name: agent.to_string(),
            role: "tester".into(),
            bio: "runtime e2e".into(),
        },
        computer_id: "local".into(),
        harness: HarnessConfig {
            harness_id: "scripted".into(),
            executable_path: "/fixture".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            environment_variables: Vec::new(),
        },
        session: SessionDirective {
            logical_session_id: format!("session_{agent}"),
            action: SessionAction::StartNew as i32,
            native_session_id: None,
            config_fingerprint: "fixture".into(),
        },
        execution_token: "fixture-token".into(),
    }
}

async fn pull(
    runtime: &Runtime,
    workspace: &WorkspaceId,
    now: i64,
) -> Result<lankide_runtime::CoordinatedEvent, String> {
    let event = runtime
        .next_provider_event(workspace)
        .await
        .map_err(err)?
        .ok_or("event stream closed")?;
    runtime
        .accept_provider_event(workspace, event, now)
        .await
        .map_err(err)
}

fn message(
    channel: &ChannelId,
    chat: &str,
    id: &str,
    content: &str,
    mentioned: &AgentId,
    sender_id: &str,
) -> ProviderEvent {
    ProviderEvent {
        channel_id: channel.clone(),
        provider_chat_id: chat.into(),
        sender: EventSender {
            id: sender_id.into(),
            kind: if sender_id.starts_with("provider_agent") {
                EventSenderType::Agent
            } else {
                EventSenderType::Human
            },
            name: sender_id.into(),
        },
        kind: ProviderEventKind::Message(MessageEvent {
            provider_message_id: id.into(),
            content: content.into(),
            reply_to_provider_message_id: None,
            mentioned_agent_ids: vec![mentioned.clone()],
            attachments: Vec::new(),
        }),
        provider_data: json!({"chat_name": chat, "event_id": format!("event_{id}")}),
    }
}

fn reaction(channel: &ChannelId, chat: &str, target: &str, event_id: &str) -> ProviderEvent {
    let sender = EventSender {
        id: "reactor".into(),
        kind: EventSenderType::Human,
        name: "Reactor".into(),
    };
    ProviderEvent {
        channel_id: channel.clone(),
        provider_chat_id: chat.into(),
        sender: sender.clone(),
        kind: ProviderEventKind::Reaction(ReactionEvent {
            target_provider_message_id: target.into(),
            sender,
            reaction: "thumbsup".into(),
            action: ReactionAction::Add,
        }),
        provider_data: json!({"event_id": event_id}),
    }
}

fn successful_reply(channel: &ChannelId, chat: &str, content: &str) -> ComputerOutcome {
    ComputerOutcome::Succeeded {
        output: content.into(),
        reply: Some(ExecutionReply {
            channel_id: channel.clone(),
            provider_chat_id: chat.into(),
            content: content.into(),
        }),
    }
}

fn ensure(value: bool, message: &str) -> Result<(), String> {
    value.then_some(()).ok_or_else(|| message.to_owned())
}
fn no_reply_outcome() -> ComputerOutcome {
    ComputerOutcome::Succeeded {
        output: "done".into(),
        reply: None,
    }
}
fn err(value: impl std::fmt::Display) -> String {
    value.to_string()
}

#[derive(Default)]
struct FakeState {
    sink: Mutex<Option<EventSink>>,
    emit_tail: Mutex<Option<oneshot::Receiver<()>>>,
    sends: Mutex<VecDeque<SendMode>>,
    provider_send_keys: Mutex<Vec<String>>,
    log: Mutex<Vec<String>>,
    runtime_contexts: Mutex<Vec<AgentRuntimeContext>>,
    inits: AtomicUsize,
    auto_reaction_failures: AtomicUsize,
    fail_lifecycle: AtomicBool,
    tool_calls: AtomicUsize,
    tool_gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
    recall_calls: AtomicUsize,
    recall_supported: AtomicBool,
    recall_failures: AtomicUsize,
    edit_calls: AtomicUsize,
    edit_supported: AtomicBool,
    shutdowns: AtomicUsize,
}
impl FakeState {
    async fn emit(&self, event: ProviderEvent) -> Result<(), String> {
        let sink = self.sink.lock().map_err(err)?.clone().ok_or("no sink")?;
        // Preserve the external source's order while the scenario drives acceptance.
        let (done, next) = oneshot::channel();
        let previous = self.emit_tail.lock().map_err(err)?.replace(next);
        tokio::spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = sink.emit(event).await;
            let _ = done.send(());
            result
        });
        Ok(())
    }
    fn push_send(&self, mode: SendMode) {
        self.sends.lock().expect("sends").push_back(mode);
    }
}
enum SendMode {
    Sent(String),
    Unknown,
    Fail,
}

struct FakeProvider {
    id: ProviderId,
    state: Arc<FakeState>,
}
impl Provider for FakeProvider {
    fn reaction_catalog(&self) -> Vec<lankide_provider::ReactionDefinition> {
        vec![lankide_provider::ReactionDefinition {
            id: "fixture_reaction".into(),
            name: "Reaction".into(),
            emoji: "👍".into(),
        }]
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.id.clone(),
            name: "Fixture".into(),
            description: "Runtime external peer".into(),
            version: "1".into(),
            icon: String::new(),
        }
    }
    fn channels<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async {
            Ok(vec![
                ChannelDefinition::new(
                    ChannelId::new("chat")
                        .map_err(|_| ProviderError::InvalidInput { field: "id" })?,
                    "Chat",
                    ChannelDefaultAttention::Mention,
                    [
                        AttentionLevel::Mention,
                        AttentionLevel::Follow,
                        AttentionLevel::Schedule,
                    ],
                    [ChannelAbility::Reply, ChannelAbility::Reaction],
                )?,
                ChannelDefinition::new(
                    ChannelId::new("always")
                        .map_err(|_| ProviderError::InvalidInput { field: "id" })?,
                    "Always",
                    ChannelDefaultAttention::Follow,
                    [AttentionLevel::Follow],
                    [ChannelAbility::Reply],
                )?,
                ChannelDefinition::new(
                    ChannelId::new("scheduled")
                        .map_err(|_| ProviderError::InvalidInput { field: "id" })?,
                    "Scheduled",
                    ChannelDefaultAttention::Mute,
                    [AttentionLevel::Schedule],
                    [ChannelAbility::Reply],
                )?,
                ChannelDefinition::new(
                    ChannelId::new("muted")
                        .map_err(|_| ProviderError::InvalidInput { field: "id" })?,
                    "Muted",
                    ChannelDefaultAttention::Mute,
                    [AttentionLevel::Mention, AttentionLevel::Follow],
                    [ChannelAbility::Reply],
                )?,
            ])
        })
    }
    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state.inits.fetch_add(1, Ordering::SeqCst);
            *self
                .state
                .sink
                .lock()
                .map_err(|_| ProviderError::External {
                    operation: "lock",
                    retryable: false,
                })? = Some(context.event_sink().clone());
            context
                .register_interface(InterfaceDefinition {
                    id: "fixture.alpha".into(),
                    input_schema: JsonSchema::new(json!({"type":"object"}))?,
                    output_schema: JsonSchema::new(json!({"type":"object"}))?,
                    handler: Arc::new(Echo),
                })
                .await?;
            context
                .register_interface(InterfaceDefinition {
                    id: "fixture.echo".into(),
                    input_schema: JsonSchema::new(json!({"type":"object","required":["value"]}))?,
                    output_schema: JsonSchema::new(json!({"type":"object","required":["value"]}))?,
                    handler: Arc::new(Echo),
                })
                .await?;
            Ok(())
        })
    }
    fn shutdown<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, ()> {
        self.state.shutdowns.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
    fn create_workspace<'a>(&'a self, context: WorkspaceContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if self.state.fail_lifecycle.load(Ordering::SeqCst) {
                return Err(ProviderError::External {
                    operation: "create_workspace",
                    retryable: false,
                });
            }
            let secret = context.provider_secrets().read("token").await?;
            self.state.log.lock().expect("log").push(format!(
                "create_workspace:{}:secret_present={}",
                context.new_provider_config,
                !secret.expose_secret().is_empty()
            ));
            context
                .storage()
                .put_workspace_state(json!({"created_by_provider":true}))
                .await?;
            Ok(())
        })
    }
    fn update_workspace<'a>(&'a self, context: WorkspaceContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if self.state.fail_lifecycle.load(Ordering::SeqCst) {
                return Err(ProviderError::External {
                    operation: "update_workspace",
                    retryable: false,
                });
            }
            self.state.log.lock().expect("log").push(format!(
                "update_workspace:{}:{}",
                context.old_provider_config.clone().unwrap_or_default(),
                context.new_provider_config
            ));
            let mut provider_state = context.storage().workspace_state().await?;
            provider_state["updated_by_provider"] = json!(true);
            context
                .storage()
                .put_workspace_state(provider_state)
                .await?;
            Ok(())
        })
    }
    fn create_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if self.state.fail_lifecycle.load(Ordering::SeqCst) {
                return Err(ProviderError::External {
                    operation: "create_agent",
                    retryable: false,
                });
            }
            let secret_present = if context.new_provider_agent_config.get("tenant").is_some() {
                !context
                    .provider_secrets()
                    .read("bot_token")
                    .await?
                    .expose_secret()
                    .is_empty()
            } else {
                false
            };
            self.state.log.lock().expect("log").push(format!(
                "create_agent:{}:secret_present={secret_present}",
                context.new_provider_agent_config,
            ));
            let mut provider_state = context.storage().agent_state(&context.agent_id).await?;
            provider_state["created_by_provider"] = json!(true);
            context
                .storage()
                .put_agent_state(&context.agent_id, provider_state)
                .await?;
            Ok(())
        })
    }
    fn update_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if self.state.fail_lifecycle.load(Ordering::SeqCst) {
                return Err(ProviderError::External {
                    operation: "update_agent",
                    retryable: false,
                });
            }
            self.state.log.lock().expect("log").push(format!(
                "update_agent:{}:{}",
                context
                    .old_provider_agent_config
                    .clone()
                    .unwrap_or_default(),
                context.new_provider_agent_config
            ));
            let mut provider_state = context.storage().agent_state(&context.agent_id).await?;
            provider_state["updated_by_provider"] = json!(true);
            context
                .storage()
                .put_agent_state(&context.agent_id, provider_state)
                .await?;
            Ok(())
        })
    }
    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult> {
        self.state
            .provider_send_keys
            .lock()
            .expect("provider send keys")
            .push(context.idempotency_key);
        Box::pin(async move {
            match self
                .state
                .sends
                .lock()
                .expect("sends")
                .pop_front()
                .unwrap_or(SendMode::Fail)
            {
                SendMode::Sent(id) => SendMessageResult::sent(id),
                SendMode::Unknown => Ok(SendMessageResult::unknown()),
                SendMode::Fail => Err(ProviderError::External {
                    operation: "send",
                    retryable: true,
                }),
            }
        })
    }
    fn recall_message<'a>(&'a self, _: RecallMessageContext) -> ProviderFuture<'a, ()> {
        self.state.recall_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self
                .state
                .recall_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_sub(1)
                })
                .is_ok()
            {
                return Err(ProviderError::External {
                    operation: "recall_message",
                    retryable: true,
                });
            }
            if self.state.recall_supported.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(ProviderError::Unsupported {
                    operation: "recall_message",
                })
            }
        })
    }
    fn edit_message<'a>(&'a self, context: EditMessageContext) -> ProviderFuture<'a, ()> {
        self.state.edit_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.state.edit_supported.load(Ordering::SeqCst) {
                self.state
                    .log
                    .lock()
                    .expect("log")
                    .push(format!("edit_message:{}", context.content));
                Ok(())
            } else {
                Err(ProviderError::Unsupported {
                    operation: "edit_message",
                })
            }
        })
    }
    fn auto_reaction<'a>(&'a self, _: AutoReactionContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.state
                .log
                .lock()
                .expect("log")
                .push("auto_reaction".into());
            if self
                .state
                .auto_reaction_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_sub(1)
                })
                .is_ok()
            {
                Err(ProviderError::External {
                    operation: "reaction",
                    retryable: true,
                })
            } else {
                Ok(())
            }
        })
    }
    fn system_prompt<'a>(&'a self, context: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        self.state
            .runtime_contexts
            .lock()
            .expect("runtime contexts")
            .push(context.clone());
        Box::pin(async move { Ok(format!("provider prompt for {}", context.agent_id)) })
    }
    fn tools<'a>(
        &'a self,
        context: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<ToolDefinition>> {
        Box::pin(async move {
            if context
                .provider_agent_config
                .get("deny_echo")
                .and_then(Value::as_bool)
                == Some(true)
            {
                return Ok(Vec::new());
            }
            Ok(vec![
                ToolDefinition {
                    id: "echo".into(),
                    description: "Echo input".into(),
                    input_schema: JsonSchema::new(json!({"type": "object"}))?,
                    output_schema: JsonSchema::new(json!({"type": "object"}))?,
                    handler: Arc::new(CountedEcho(self.state.clone())),
                },
                ToolDefinition {
                    id: "runtime_lookup".into(),
                    description: "Read a Runtime message through the Provider SDK".into(),
                    input_schema: JsonSchema::new(json!({
                        "type": "object",
                        "required": ["message_id"],
                        "properties": {"message_id": {"type": "integer", "minimum": 1}}
                    }))?,
                    output_schema: JsonSchema::new(json!({"type": "object"}))?,
                    handler: Arc::new(RuntimeLookup(context.clone())),
                },
            ])
        })
    }
    fn cli_commands<'a>(
        &'a self,
        _: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<lankide_provider::CliCommandDefinition>> {
        Box::pin(async {
            Ok(vec![lankide_provider::CliCommandDefinition {
                id: "fixture".into(),
                description: "Fixture command".into(),
                handler: Arc::new(Echo),
            }])
        })
    }
}

struct Echo;
impl InvocationHandler for Echo {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move { Ok(input) })
    }
}

struct CountedEcho(Arc<FakeState>);
impl InvocationHandler for CountedEcho {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let gate = self.0.tool_gate.lock().unwrap().clone();
        self.0.tool_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.notified().await;
            }
            Ok(input)
        })
    }
}

struct RuntimeLookup(AgentRuntimeContext);
impl InvocationHandler for RuntimeLookup {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let raw_message_id = input.get("message_id").and_then(Value::as_i64).ok_or(
                ProviderError::InvalidInput {
                    field: "message_id",
                },
            )?;
            let message_id =
                MessageId::new(raw_message_id).map_err(|_| ProviderError::InvalidInput {
                    field: "message_id",
                })?;
            let message = self.0.storage().message(message_id).await?;
            Ok(json!({
                "message_id": message.message_id.get(),
                "provider_message_id": message.provider_message_id,
                "channel_id": message.channel_id.as_str(),
                "chat_id": message.chat_id,
            }))
        })
    }
}

struct ScriptComputer {
    state: Arc<FakeState>,
    inject: Mutex<Option<ProviderEvent>>,
    outcome: ComputerOutcome,
    assignments: Mutex<Vec<AssignExecution>>,
}

struct GateComputer {
    release: tokio::sync::Mutex<Option<oneshot::Receiver<()>>>,
    started: tokio::sync::Notify,
    outcome: ComputerOutcome,
}
impl ComputerExecutor for GateComputer {
    fn execute(&self, _: AssignExecution) -> ComputerFuture<'_, ComputerCompletion> {
        Box::pin(async move {
            self.started.notify_one();
            let receiver = self
                .release
                .lock()
                .await
                .take()
                .ok_or_else(|| ComputerError {
                    code: "gate_reused".into(),
                })?;
            receiver.await.map_err(|_| ComputerError {
                code: "gate_closed".into(),
            })?;
            Ok(ComputerCompletion {
                delivery_accepted: true,
                outcome: self.outcome.clone(),
            })
        })
    }
}
impl ScriptComputer {
    fn new(state: Arc<FakeState>, inject: Option<ProviderEvent>, outcome: ComputerOutcome) -> Self {
        Self {
            state,
            inject: Mutex::new(inject),
            outcome,
            assignments: Mutex::new(Vec::new()),
        }
    }
}
impl ComputerExecutor for ScriptComputer {
    fn execute(&self, assignment: AssignExecution) -> ComputerFuture<'_, ComputerCompletion> {
        Box::pin(async move {
            self.state.log.lock().expect("log").push("computer".into());
            self.assignments
                .lock()
                .expect("assignments")
                .push(assignment);
            let inject = self.inject.lock().expect("inject").take();
            if let Some(event) = inject {
                self.state
                    .emit(event)
                    .await
                    .map_err(|code| ComputerError { code })?;
            }
            Ok(ComputerCompletion {
                delivery_accepted: true,
                outcome: self.outcome.clone(),
            })
        })
    }
}

#[derive(Default)]
struct Services {
    values: Mutex<HashMap<String, Vec<u8>>>,
}
impl SecretReader for Services {
    fn read<'a>(&'a self, _: &'a str) -> ProviderFuture<'a, SecretValue> {
        Box::pin(async { Ok(SecretValue::new("fixture")) })
    }
}
impl ProviderStorage for Services {
    fn get<'a>(&'a self, key: &'a str) -> ProviderFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(self.values.lock().expect("values").get(key).cloned()) })
    }
    fn put<'a>(&'a self, key: &'a str, value: Vec<u8>) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.values
                .lock()
                .expect("values")
                .insert(key.into(), value);
            Ok(())
        })
    }
}
impl ProviderLogger for Services {
    fn log(&self, _: LogLevel, _: &str) {}
}
impl ResourceSaver for Services {
    fn save<'a>(
        &'a self,
        value: ResourceUpload,
    ) -> ProviderFuture<'a, lankide_protocol::ResourceRef> {
        Box::pin(async move {
            Ok(lankide_protocol::ResourceRef {
                resource_id: 1,
                file_name: value.file_name,
                media_type: value.media_type,
                size_bytes: value.bytes.len() as u64,
                sha256: "0".repeat(64),
            })
        })
    }
}
impl OperationProgress for Services {
    fn report<'a>(&'a self, _: OperationUpdate) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
fn services() -> HostServices {
    let services = Arc::new(Services::default());
    HostServices {
        secret_reader: services.clone(),
        secret_writer: Arc::new(lankide_provider::DeniedSecretWriter),
        storage: services.clone(),
        logger: services.clone(),
        resource_saver: services.clone(),
        operation_progress: services,
        event_capacity: 32,
        invocation_timeout: Duration::from_secs(1),
    }
}

async fn profile_application() -> (tempfile::TempDir, RuntimeApplication, Arc<FakeState>) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("runtime.db");
    let w = WorkspaceId::new("profile_workspace").unwrap();
    let a = AgentId::new("profile_agent").unwrap();
    let p = ProviderId::new("fixture").unwrap();
    let storage = Storage::open(DatabaseConfig::new(&db)).unwrap();
    storage
        .create_workspace(WorkspaceSpec {
            id: w.clone(),
            name: "Audit".into(),
            provider_id: p.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .unwrap();
    storage
        .create_agent(AgentSpec {
            workspace_id: w.clone(),
            id: a.clone(),
            name: "Before".into(),
            now: 2,
        })
        .unwrap();
    let mut record = storage.agent_record(&w, &a).unwrap();
    record.computer_id = "local".into();
    record.harness_id = "fixture".into();
    record.model = "fixture".into();
    record.thinking = "low".into();
    record.env.insert("REMOVE_ME".into(), "old_value".into());
    storage.update_agent_record(record).unwrap();
    drop(storage);
    let state = Arc::new(FakeState::default());
    let factories = ProviderFactoryRegistry::default();
    factories
        .register(Arc::new(ApplicationFactory {
            provider_id: p,
            state: state.clone(),
        }))
        .unwrap();
    let app = RuntimeApplication::open(
        RuntimeConfig {
            database: DatabaseConfig::new(&db),
        },
        factories,
    )
    .await
    .unwrap();
    app.start().await.unwrap();
    (temp, app, state)
}

fn profile_agent_ref() -> Option<AgentRef> {
    Some(AgentRef {
        workspace_id: "profile_workspace".into(),
        agent_id: "profile_agent".into(),
    })
}

async fn clearing_environment_scenario() {
    let (_temp, app, _state) = profile_application().await;
    let result = app
        .update_agent(UpdateAgentRequest {
            request_id: "clear-environment".into(),
            agent: profile_agent_ref(),
            environment_variables: Some(lankide_protocol::EnvironmentVariableSet {
                entries: vec![],
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    assert!(
        result.environment_variables.is_empty(),
        "empty replacement retained old environment: {:?}",
        result.environment_variables
    );
}

async fn local_agent_settings_scenario() {
    let (_temp, app, state) = profile_application().await;
    state.log.lock().unwrap().clear();
    state.fail_lifecycle.store(true, Ordering::SeqCst);
    let result = app
        .update_agent(UpdateAgentRequest {
            request_id: "update-local-settings".into(),
            agent: profile_agent_ref(),
            role: Some("Product lead".into()),
            harness_id: Some("fixture".into()),
            model: Some("local-model".into()),
            thinking_level: Some("high".into()),
            auto_reaction: Some("thumbsup".into()),
            soul: Some("Prefer concise decisions.".into()),
            environment_variables: Some(lankide_protocol::EnvironmentVariableSet {
                entries: vec![lankide_protocol::EnvironmentVariable {
                    name: "MODE".into(),
                    value: "local".into(),
                }],
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let log = state.log.lock().unwrap().clone();
    assert_eq!(result.role, "Product lead");
    assert_eq!(result.harness_id, "fixture");
    assert_eq!(result.model, "local-model");
    assert_eq!(result.thinking_level, "high");
    assert_eq!(result.auto_reaction, "thumbsup");
    assert_eq!(result.soul, "Prefer concise decisions.");
    assert_eq!(result.environment_variables.len(), 1);
    assert!(
        !log.iter().any(|entry| entry.starts_with("update_agent:")),
        "local-only settings reached the Provider: {log:?}"
    );
    app.shutdown().await.unwrap();
}

async fn agent_avatar_scenario() {
    let (temp, app, state) = profile_application().await;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut encoded, image::ImageFormat::Jpeg)
        .unwrap();
    let bytes = encoded.into_inner();
    let result = app
        .update_agent(UpdateAgentRequest {
            request_id: "update-agent-avatar".into(),
            agent: profile_agent_ref(),
            avatar: Some(AgentAvatarUpload {
                media_type: "image/jpeg".into(),
                bytes: bytes.clone(),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let log = state.log.lock().unwrap().clone();
    assert_eq!(
        result.avatar_path,
        "resources/profile_workspace/profile_agent_avatar.jpg"
    );
    let avatar_path = temp
        .path()
        .join("resources/profile_workspace/profile_agent_avatar.jpg");
    assert!(avatar_path.is_file(), "named Agent avatar was not stored");
    assert_eq!(fs::read(&avatar_path).unwrap(), bytes);
    let fetched = app
        .get_agent_avatar(lankide_protocol::GetAgentAvatarRequest {
            request_id: "get-agent-avatar".into(),
            agent: profile_agent_ref(),
        })
        .await
        .unwrap()
        .avatar
        .unwrap();
    assert_eq!(fetched.path, result.avatar_path);
    assert_eq!(fetched.bytes, bytes);
    assert!(log.iter().any(|entry| {
        entry.contains("update_agent:")
            && entry.contains("profile_agent_avatar.jpg")
            && entry.contains("avatar_bytes")
    }));
    app.shutdown().await.unwrap();
    let storage = Storage::open(DatabaseConfig::new(temp.path().join("runtime.db"))).unwrap();
    assert_eq!(
        storage
            .agent_record(
                &WorkspaceId::new("profile_workspace").unwrap(),
                &AgentId::new("profile_agent").unwrap(),
            )
            .unwrap()
            .avatar
            .as_deref(),
        Some("resources/profile_workspace/profile_agent_avatar.jpg")
    );
    storage.close().unwrap();
}

async fn failed_agent_update_scenario() {
    let (_temp, app, state) = profile_application().await;
    state.fail_lifecycle.store(true, Ordering::SeqCst);
    let update = app
        .update_agent(UpdateAgentRequest {
            request_id: "failed-agent-update".into(),
            agent: profile_agent_ref(),
            name: Some("Should not persist".into()),
            ..Default::default()
        })
        .await;
    assert!(
        update.is_err(),
        "failure injection must reach real update path"
    );
    let read = app
        .get_agent(lankide_protocol::GetAgentRequest {
            request_id: "read-agent-after-failure".into(),
            agent: profile_agent_ref(),
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    assert_eq!(
        read.name, "Before",
        "failed update nevertheless persisted new name"
    );
}

struct PromptFailureProvider(FakeProvider);
impl Provider for PromptFailureProvider {
    fn info(&self) -> ProviderInfo {
        self.0.info()
    }
    fn channels<'a>(&'a self, c: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        self.0.channels(c)
    }
    fn init<'a>(&'a self, c: InitContext) -> ProviderFuture<'a, ()> {
        self.0.init(c)
    }
    fn send_message<'a>(&'a self, c: SendMessageContext) -> ProviderFuture<'a, SendMessageResult> {
        self.0.send_message(c)
    }
    fn system_prompt<'a>(&'a self, _: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        Box::pin(async {
            Err(ProviderError::External {
                operation: "prompt_failure",
                retryable: true,
            })
        })
    }
}
struct NeverAssignedComputer;
impl ComputerExecutor for NeverAssignedComputer {
    fn execute(&self, _: AssignExecution) -> ComputerFuture<'_, ComputerCompletion> {
        Box::pin(async { panic!("failure must happen before assignment") })
    }
}
async fn prompt_failure_recovery_scenario() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Runtime::new(RuntimeConfig {
        database: DatabaseConfig::new(temp.path().join("runtime.db")),
    })
    .unwrap();
    let w = WorkspaceId::new("prompt_workspace").unwrap();
    let a = AgentId::new("prompt_agent").unwrap();
    let p = ProviderId::new("fixture").unwrap();
    let ch = ChannelId::new("chat").unwrap();
    runtime
        .storage()
        .create_workspace(WorkspaceSpec {
            id: w.clone(),
            name: "Audit".into(),
            provider_id: p.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .unwrap();
    runtime
        .storage()
        .create_agent(AgentSpec {
            workspace_id: w.clone(),
            id: a.clone(),
            name: "Audit".into(),
            now: 1,
        })
        .unwrap();
    let state = Arc::new(FakeState::default());
    let fw = w.clone();
    runtime
        .register_provider_factory(w.clone(), move || {
            lankide_provider::ProviderHost::new(
                PromptFailureProvider(FakeProvider {
                    id: p.clone(),
                    state: state.clone(),
                }),
                fw.clone(),
                json!({}),
                services(),
            )
        })
        .unwrap();
    runtime
        .configure_agent(&w, agent_config(&w, &a, &ch, "provider_agent_audit"))
        .await
        .unwrap();
    runtime.start().await.unwrap();
    runtime
        .coordinate_event(
            &w,
            message(&ch, "prompt_chat", "prompt_message", "wake up", &a, "human"),
            2,
        )
        .await
        .unwrap();
    let result = runtime
        .execute_ready(&w, &a, &NeverAssignedComputer, 3)
        .await;
    assert!(
        result.is_err(),
        "failure injection must reach dynamic prompt construction"
    );
    let pinned = runtime.storage().has_active_execution(&w).unwrap();
    let recovery = runtime.recoverable_executions(Some(&w)).unwrap();
    runtime.shutdown().await.unwrap();
    assert!(
        !pinned,
        "prompt failure left active execution and claimed inbox: {recovery:?}"
    );
}

async fn durable_ingress_retry_scenario() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("runtime.db");
    let runtime = Runtime::new(RuntimeConfig {
        database: DatabaseConfig::new(&path),
    })
    .unwrap();
    let w = WorkspaceId::new("receipt_workspace").unwrap();
    let a = AgentId::new("receipt_agent").unwrap();
    let p = ProviderId::new("fixture").unwrap();
    let ch = ChannelId::new("chat").unwrap();
    runtime
        .storage()
        .create_workspace(WorkspaceSpec {
            id: w.clone(),
            name: "Receipt".into(),
            provider_id: p.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .unwrap();
    runtime
        .storage()
        .create_agent(AgentSpec {
            workspace_id: w.clone(),
            id: a.clone(),
            name: "Receipt".into(),
            now: 1,
        })
        .unwrap();
    let state = Arc::new(FakeState::default());
    let source = state.clone();
    let fw = w.clone();
    runtime
        .register_provider_factory(w.clone(), move || {
            lankide_provider::ProviderHost::new(
                FakeProvider {
                    id: p.clone(),
                    state: source.clone(),
                },
                fw.clone(),
                json!({}),
                services(),
            )
        })
        .unwrap();
    runtime
        .configure_agent(&w, agent_config(&w, &a, &ch, "provider_receipt"))
        .await
        .unwrap();
    runtime.start().await.unwrap();
    let fault = rusqlite::Connection::open(&path).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_message AFTER INSERT ON messages BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
    for attempt in 0..3 {
        let sink = state.sink.lock().unwrap().clone().unwrap();
        let event = message(
            &ch,
            "receipt_chat",
            "receipt_message",
            "durable",
            &a,
            "human",
        );
        let producer = tokio::spawn(async move { sink.emit(event).await });
        let delivery =
            tokio::time::timeout(Duration::from_secs(2), runtime.next_provider_event(&w))
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        assert!(
            !producer.is_finished(),
            "dequeue must not acknowledge delivery"
        );
        let result = runtime
            .accept_provider_event(&w, delivery, 2 + attempt)
            .await;
        let receipt = tokio::time::timeout(Duration::from_secs(2), producer)
            .await
            .unwrap()
            .unwrap();
        if attempt == 0 {
            assert!(result.is_err());
            assert!(receipt.is_err(), "failed commit must not acknowledge");
            assert_eq!(runtime.storage().message_count(&w).unwrap(), 0);
            assert!(runtime.storage().inbox_items(&w, &a).unwrap().is_empty());
            fault.execute_batch("DROP TRIGGER fail_message").unwrap();
        } else {
            result.unwrap();
            receipt.unwrap();
            assert_eq!(runtime.storage().message_count(&w).unwrap(), 1);
            assert_eq!(runtime.storage().inbox_items(&w, &a).unwrap().len(), 1);
        }
    }
    runtime.shutdown().await.unwrap();
}

async fn configured_assignment_scenario() {
    let mut fingerprints = Vec::new();
    for variant in 0..7 {
        let (_temp, app, state) = profile_application().await;
        let model = if variant == 2 {
            "account/another-user-model"
        } else {
            "account/user-model"
        };
        let thinking = if variant == 3 { "off" } else { "xhigh" };
        let cli = if variant == 4 {
            "/custom/other-cli"
        } else {
            "/custom/native-cli"
        };
        let env_value = if variant == 5 { "second" } else { "first" };
        let harness = if variant == 6 { "codex" } else { "pi" };
        app.update_agent(UpdateAgentRequest {
            request_id: format!("configure-{variant}"),
            agent: profile_agent_ref(),
            model: Some(model.into()),
            thinking_level: Some(thinking.into()),
            cli_path: Some(cli.into()),
            harness_id: Some(harness.into()),
            environment_variables: Some(lankide_protocol::EnvironmentVariableSet {
                entries: vec![lankide_protocol::EnvironmentVariable {
                    name: "USER_SETTING".into(),
                    value: env_value.into(),
                }],
            }),
            ..Default::default()
        })
        .await
        .unwrap();
        let broker = app.computer_broker();
        let mut connection = broker
            .connect(ComputerHello {
                protocol_version: PROTOCOL_VERSION,
                computer_id: "local".into(),
                instance_id: "config-audit".into(),
                ..Default::default()
            })
            .unwrap();
        let _ = connection.next().await.unwrap().unwrap();
        state
            .emit(message(
                &ChannelId::new("chat").unwrap(),
                "config-chat",
                "config-message",
                "Read configured input",
                &AgentId::new("profile_agent").unwrap(),
                "human",
            ))
            .await
            .unwrap();
        let assignment = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = connection.next().await.expect("Computer stream").unwrap();
                if let Some(runtime_frame::Frame::AssignExecution(assignment)) = frame.frame {
                    break assignment;
                }
            }
        })
        .await
        .expect("configuration must reach an actual Runtime assignment");
        let configured = assignment.harness.unwrap();
        assert_eq!(configured.model, model);
        assert_eq!(configured.thinking_level, thinking);
        assert_eq!(configured.executable_path, cli);
        assert_eq!(configured.harness_id, harness);
        assert_eq!(
            configured.environment_variables,
            vec![lankide_protocol::EnvironmentVariable {
                name: "USER_SETTING".into(),
                value: env_value.into()
            }]
        );
        let fingerprint = assignment.session.unwrap().config_fingerprint;
        assert_eq!(
            fingerprint.len(),
            64,
            "session fingerprint must be a digest, not a fixed label"
        );
        fingerprints.push(fingerprint);
        drop(connection);
        drop(broker);
        state.runtime_contexts.lock().unwrap().clear();
        app.shutdown().await.unwrap();
    }
    assert_eq!(
        fingerprints[0], fingerprints[1],
        "same effective configuration must remain resumable"
    );
    for fingerprint in &fingerprints[2..] {
        assert_ne!(
            fingerprint, &fingerprints[0],
            "changing a native setting must invalidate the old session configuration"
        );
    }
}
