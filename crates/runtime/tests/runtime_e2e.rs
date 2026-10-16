#[path = "../../../tests/support/registered_machine.rs"]
mod registered_machine;
#[path = "../../protocol/tests/support/ws.rs"]
mod ws;
use lankide_core::{
    AgentId, AttentionLevel, ChannelAbility, ChannelId, MessageId, PersonId, ProviderId,
    ScheduleInterval, WorkspaceId,
};
use lankide_protocol::{
    ActiveAttempt, AgentAttention, AgentAvatarUpload, AgentRef, AgentRunContext,
    AgentRuntimeProfile, AssignExecution, AssignmentResult, AttentionMode, CapabilityInvokeRequest,
    ChannelAttention, ContentKind, ContentRequest, CreateAgentRequest, CreateWorkspaceRequest,
    DEFAULT_MAX_MESSAGE_BYTES, DeleteAgentRequest, Delivery, DeliveryKind, DeliveryResult,
    ExecutionEvent, ExecutionEventKind, ExecutionRef, HarnessConfig, ListWorkspacesRequest,
    MachineFrame, MachineHeartbeat, MachineHello, ManagementSubscription, PROTOCOL_VERSION,
    ResourceChunk, ResourceDescriptor, ResourceRequest, RuntimePeer, SendMessageRequest,
    SessionAction, SessionDirective, TransportConfig, UpdateAgentRequest, UpdateSelfProfileRequest,
    UpdateWorkspaceRequest, WorkspaceRef, machine_frame, runtime_frame,
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
    AgentRuntimeConfig, AttentionPolicy, ExecutionProgress, ExecutionReply, MachineCompletion,
    MachineError, MachineExecutor, MachineFuture, MachineOutcome, ProviderFactoryRegistry,
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
    let socket = temp.path().join("run");
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        scenario(&db).await.map_err(|e| format!("scenario: {e}"))?;
        attempt_recovery_scenario(&attempt_db)
            .await
            .map_err(|e| format!("attempt_recovery_scenario: {e}"))?;
        pause_drain_scenario(&pause_db)
            .await
            .map_err(|e| format!("pause_drain_scenario: {e}"))?;
        collaboration_fencing_scenario(&collaboration_db)
            .await
            .map_err(|e| format!("collaboration_fencing_scenario: {e}"))?;
        application_scenario(&application_db, &socket)
            .await
            .map_err(|e| format!("application_scenario: {e}"))?;
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
        retry: Default::default(),
    };
    let mut runtime = scenario_runtime(config.clone()).map_err(err)?;
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
                machine_id: "fixture-machine".into(),
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
    )
    .await?;
    register(
        &mut runtime,
        workspace_b.clone(),
        provider_id,
        state_b.clone(),
    )
    .await?;
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
    let no_reply = || MachineOutcome::Succeeded {
        output: "completed without sending".into(),
        reply: None,
    };
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace_b,
                    &agent_b,
                    &ScriptMachine::new(state_b.clone(), None, no_reply()),
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
                    &ScriptMachine::new(state_b.clone(), None, no_reply()),
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
                    &ScriptMachine::new(state_b.clone(), None, no_reply()),
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
        runtime.storage().message_count(&workspace_a).map_err(err)? == 5,
        "canonical dedup",
    )?;
    state_a.push_send(SendMode::Sent("provider_reply_1".into()));
    let machine = ScriptMachine::new(
        state_a.clone(),
        Some(message(
            &channel,
            "chat_a",
            "late",
            "arrived during execution",
            &agent_a,
            "human",
        )),
        MachineOutcome::Succeeded {
            output: "answer one".into(),
            reply: Some(ExecutionReply {
                channel_id: channel.clone(),
                provider_chat_id: "chat_a".into(),
                content: "answer one".into(),
            }),
        },
    );
    let progress = runtime
        .execute_ready(&workspace_a, &agent_a, &machine, 20)
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
    let assignments = machine.assignments.lock().map_err(err)?.clone();
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
            && !rendered.contains("self echo")
            && rendered.contains("later two")
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
        machine_id: provider_contexts[0].machine_id.clone(),
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
        log.iter().position(|v| v == "auto_reaction") < log.iter().position(|v| v == "machine"),
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
            &ScriptMachine::new(
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
                    &ScriptMachine::new(
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
                    &ScriptMachine::new(
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
            &ScriptMachine::new(state_a.clone(), None, MachineOutcome::Cancelled),
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

    // A cancelled batch is terminal. Only a new message starts another run.
    state_a
        .emit(message(
            &channel,
            "chat_a",
            "after-cancel",
            "new request",
            &agent_a,
            "human",
        ))
        .await?;
    pull(&runtime, &workspace_a, 28).await?;
    state_a.push_send(SendMode::Unknown);
    let pending = runtime
        .execute_ready(
            &workspace_a,
            &agent_a,
            &ScriptMachine::new(
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

    let mut restarted = scenario_runtime(config).map_err(err)?;
    register(
        &mut restarted,
        workspace_a.clone(),
        ProviderId::new("fixture").map_err(err)?,
        state_a.clone(),
    )
    .await?;
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
            .await
            .map_err(err)?
            .len()
            == 1,
        "recover execution",
    )?;
    ensure(
        restarted
            .recoverable_sends(Some(&workspace_a))
            .await
            .map_err(err)?
            .len()
            == 1,
        "recover send",
    )?;
    ensure(
        matches!(
            restarted
                .resolve_unknown_send(&workspace_a, &request_key, "reconciled_id".into(), 30)
                .await
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
    let mut runtime = scenario_runtime(RuntimeConfig {
        database: DatabaseConfig::new(db),
        retry: Default::default(),
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
            machine_id: "fixture-machine".into(),
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
    )
    .await?;
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
    let machine = Arc::new(GateMachine {
        release: tokio::sync::Mutex::new(Some(release_rx)),
        started: tokio::sync::Notify::new(),
        outcome: no_reply_outcome(),
    });
    let task_runtime = runtime.clone();
    let task_machine = machine.clone();
    let task_workspace = workspace.clone();
    let task_agent = agent.clone();
    let execution = tokio::spawn(async move {
        task_runtime
            .execute_ready(&task_workspace, &task_agent, task_machine.as_ref(), 3)
            .await
    });
    machine.started.notified().await;
    runtime.pause_workspace(&workspace, 4).await.map_err(err)?;
    ensure(
        state.shutdowns.load(Ordering::SeqCst) == 0
            && runtime.next_provider_event(&workspace).await.is_err()
            && runtime
                .execute_ready(
                    &workspace,
                    &agent,
                    &ScriptMachine::new(state.clone(), None, no_reply_outcome()),
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
    let mut runtime = scenario_runtime(RuntimeConfig {
        database: DatabaseConfig::new(db),
        retry: Default::default(),
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
                machine_id: "fixture-machine".into(),
                workspace_id: workspace.clone(),
                id: agent.clone(),
                name: agent.to_string(),
                now: 1,
            })
            .map_err(err)?;
    }
    let state = Arc::new(FakeState::default());
    register(&mut runtime, workspace.clone(), provider_id, state.clone()).await?;
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
    let clark_machine = Arc::new(GateMachine {
        release: tokio::sync::Mutex::new(Some(release_rx)),
        started: tokio::sync::Notify::new(),
        outcome: successful_reply(&channel, "team_chat", "Clark stale reply"),
    });
    let task_runtime = runtime.clone();
    let task_machine = clark_machine.clone();
    let task_workspace = workspace.clone();
    let task_clark = clark.clone();
    let clark_execution = tokio::spawn(async move {
        task_runtime
            .execute_ready(&task_workspace, &task_clark, task_machine.as_ref(), 3)
            .await
    });
    clark_machine.started.notified().await;

    state.push_send(SendMode::Sent("provider_bruce_message".into()));
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace,
                    &bruce,
                    &ScriptMachine::new(
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
        matches!(callback, lankide_runtime::CoordinatedEvent::Dropped),
        "an outbound echo neither creates a message nor wakes an Agent",
    )?;
    ensure(
        matches!(
            runtime
                .execute_ready(
                    &workspace,
                    &clark,
                    &ScriptMachine::new(state.clone(), None, no_reply_outcome()),
                    6
                )
                .await
                .map_err(err)?,
            ExecutionProgress::Idle
        ),
        "the echo does not start another turn",
    )?;
    state
        .emit(message(
            &channel,
            "team_chat",
            "human-followup",
            "please continue",
            &clark,
            "human",
        ))
        .await?;
    pull(&runtime, &workspace, 6).await?;
    let reader = ScriptMachine::new(state.clone(), None, no_reply_outcome());
    let next = runtime
        .execute_ready(&workspace, &clark, &reader, 6)
        .await
        .map_err(err)?;
    let ExecutionProgress::Committed {
        execution_id,
        committed: true,
    } = next
    else {
        return Err("human follow-up did not execute".into());
    };
    let snapshot_id = runtime
        .storage()
        .execution(execution_id)
        .map_err(err)?
        .snapshot_id;
    let history = runtime
        .storage()
        .snapshot_chat_messages(snapshot_id, &channel, "team_chat", None, 50, 100)
        .map_err(err)?;
    ensure(
        history
            .iter()
            .any(|message| message.content == "Bruce introduced himself"),
        "the next human-triggered turn can read the canonical Agent message through chat history",
    )?;
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
    unmentioned_callback.sender.id = bruce.to_string();
    unmentioned_callback.sender.kind = EventSenderType::Agent;
    let before_echo = runtime.storage().message_count(&workspace).map_err(err)?;
    state.emit(unmentioned_callback).await?;
    let unmentioned_callback = pull(&runtime, &workspace, 7).await?;
    ensure(
        matches!(
            unmentioned_callback,
            lankide_runtime::CoordinatedEvent::Dropped
        ) && runtime.storage().message_count(&workspace).map_err(err)? == before_echo,
        "an Agent-ID echo with an unknown native ID creates no row or turn",
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
    let runtime = scenario_runtime(RuntimeConfig {
        database: DatabaseConfig::new(db),
        retry: Default::default(),
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
            machine_id: "fixture-machine".into(),
            workspace_id: workspace_id.clone(),
            id: agent_id.clone(),
            name: "Attempt Agent".into(),
            now: 2,
        })
        .map_err(err)?;
    let mut placement = runtime
        .storage()
        .agent_record(&workspace_id, &agent_id)
        .map_err(err)?;
    placement.machine_id = "machine-attempt".into();
    runtime
        .storage()
        .update_agent_record(placement)
        .map_err(err)?;
    runtime
        .storage()
        .ingest_event(IngestEvent {
            receiving_agent_id: None,
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
    let broker = runtime.new_machine_broker();
    let hello = MachineHello {
        device_credential: String::new(),
        protocol_version: PROTOCOL_VERSION,
        machine_id: "machine-attempt".into(),
        instance_id: "instance-stable".into(),
        machine_version: "1".into(),
        os: "test".into(),
        arch: "test".into(),
        harnesses: Vec::new(),
    };
    let mut first = broker.connect(hello.clone()).await.map_err(err)?;
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
    let executor = broker.executor("machine-attempt");
    let execution_task = tokio::spawn(async move { executor.execute(assignment).await });
    ensure(
        matches!(
            first.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::AssignExecution(_))
            }))
        ),
        "attempt assignment",
    )?;
    broker
        .handle_frame(
            "machine-attempt",
            "instance-stable",
            MachineFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::AssignmentResult(AssignmentResult {
                    failure: None,
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
            "machine-attempt",
            "instance-stable",
            MachineFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::DeliveryResult(DeliveryResult {
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
            "machine-attempt",
            "instance-stable",
            MachineFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::ExecutionEvent(ExecutionEvent {
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
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
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
                .await
                .map_err(err)?;
        } else {
            broker
                .handle_frame(
                    "machine-attempt",
                    "instance-stable",
                    MachineFrame {
                        protocol_version: lankide_protocol::PROTOCOL_VERSION,
                        frame: Some(machine_frame::Frame::Heartbeat(MachineHeartbeat {
                            machine_id: "machine-attempt".into(),
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
            matches!(result, Err(MachineError { code }) if code == expected_code),
            "lease expiry must release pending execution",
        )?;
        ensure(
            matches!(
                first.next().await,
                Some(Ok(lankide_protocol::RuntimeFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
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
            .disconnect("machine-attempt", "instance-stable")
            .await
            .map_err(err)?;
        runtime.shutdown().await.map_err(err)?;
        return Ok(());
    }
    broker
        .disconnect("machine-attempt", "instance-stable")
        .await
        .map_err(err)?;
    ensure(
        !execution_task.is_finished(),
        "disconnect preserves the waiting execution",
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

    let mut resumed = broker.connect(hello).await.map_err(err)?;
    let _ = resumed.next().await;
    broker
        .handle_frame(
            "machine-attempt",
            "instance-stable",
            MachineFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::Heartbeat(MachineHeartbeat {
                    machine_id: "machine-attempt".into(),
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
    let completed = MachineFrame {
        protocol_version: lankide_protocol::PROTOCOL_VERSION,
        frame: Some(machine_frame::Frame::ExecutionEvent(ExecutionEvent {
            execution: Some(execution.clone()),
            sequence: 2,
            occurred_at: 7,
            kind: ExecutionEventKind::Completed as i32,
            payload: br#"{"kind":"completed","final_output":"done"}"#.to_vec(),
        })),
    };
    broker
        .handle_frame("machine-attempt", "instance-stable", completed.clone())
        .await
        .map_err(err)?;
    let completion = tokio::time::timeout(Duration::from_secs(2), execution_task)
        .await
        .map_err(err)?
        .map_err(err)?
        .map_err(|error| error.code)?;
    ensure(
        completion.delivery_accepted
            && matches!(completion.outcome, MachineOutcome::Succeeded { .. }),
        "re-adopted waiter keeps acceptance and receives completion",
    )?;
    ensure(
        matches!(
            resumed.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::ExecutionEventAck(_))
            }))
        ),
        "completed durable ack",
    )?;
    ensure(
        matches!(
            resumed.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::ExecutionCommitted(_))
            }))
        ),
        "commit follows durable transaction",
    )?;
    broker
        .handle_frame("machine-attempt", "instance-stable", completed)
        .await
        .map_err(err)?;
    let _ = resumed.next().await;
    ensure(
        matches!(
            resumed.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
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
        .disconnect("machine-attempt", "instance-stable")
        .await
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
    registered_machine::register(&storage, &["fixture-machine"]).map_err(err)?;
    for machine_id in ["machine-a", "machine-b", "machine-delete"] {
        storage
            .pair_machine(
                &lankide_storage::MachineRecord {
                    is_local: false,
                    machine_id: machine_id.into(),
                    display_name: machine_id.into(),
                    credential_hash: Sha256::digest(format!("fixture-{machine_id}").as_bytes())
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                    paired_at: 1,
                    revoked_at: None,
                    last_seen_at: None,
                    os: "test".into(),
                    version: "1".into(),
                    inventory_json: serde_json::to_string(&vec![
                        lankide_protocol::link::HarnessInfo {
                            harness_id: "fixture".into(),
                            ..Default::default()
                        },
                    ])
                    .unwrap(),
                },
                None,
                600_000,
            )
            .map_err(err)?;
    }
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
            machine_id: "fixture-machine".into(),
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
    agent.machine_id = "machine-a".into();
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
        receiving_agent_id: None,
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
        "machine_id": "machine-a",
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
            "machine-a",
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
        retry: Default::default(),
    };
    let application = RuntimeApplication::open(config.clone(), factories.clone())
        .await
        .map_err(err)?;
    let recovered = application.recover(5).await.map_err(err)?;
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
            && !application.readiness().machine_connected,
        "management readiness does not require Machine",
    )?;
    let broker = application.machine_broker();
    let mut first_connection = broker
        .connect(MachineHello {
            device_credential: "fixture-machine-a".into(),
            protocol_version: PROTOCOL_VERSION,
            machine_id: "machine-a".into(),
            instance_id: "instance-1".into(),
            machine_version: "1".into(),
            os: "test".into(),
            arch: "test".into(),
            harnesses: Vec::new(),
        })
        .await
        .map_err(err)?;
    ensure(
        matches!(
            first_connection.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::Hello(_)),
            }))
        ),
        "Machine hello",
    )?;
    let mut second_connection = broker
        .connect(MachineHello {
            device_credential: "fixture-machine-a".into(),
            protocol_version: PROTOCOL_VERSION,
            machine_id: "machine-a".into(),
            instance_id: "instance-2".into(),
            machine_version: "1".into(),
            os: "test".into(),
            arch: "test".into(),
            harnesses: Vec::new(),
        })
        .await
        .map_err(err)?;
    ensure(
        matches!(
            first_connection.next().await,
            Some(Ok(lankide_protocol::RuntimeFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(runtime_frame::Frame::Shutdown(_)),
            }))
        ),
        "Machine reconnect replaces old connection",
    )?;
    let _ = second_connection.next().await;
    broker
        .disconnect("machine-a", "instance-1")
        .await
        .map_err(err)?;
    ensure(
        broker.is_connected("machine-a"),
        "stale disconnect keeps reconnected Machine",
    )?;
    let same_instance = MachineHello {
        device_credential: String::new(),
        protocol_version: PROTOCOL_VERSION,
        machine_id: "machine-same-instance".into(),
        instance_id: "stable-instance".into(),
        machine_version: "1".into(),
        os: "test".into(),
        arch: "test".into(),
        harnesses: Vec::new(),
    };
    let (old_connection_id, mut old_connection) = broker
        .connect_tracked(same_instance.clone())
        .await
        .map_err(err)?;
    let _ = old_connection.next().await;
    let (_new_connection_id, mut new_connection) =
        broker.connect_tracked(same_instance).await.map_err(err)?;
    let _ = new_connection.next().await;
    broker
        .disconnect_connection(
            "machine-same-instance",
            "stable-instance",
            old_connection_id,
        )
        .await
        .map_err(err)?;
    ensure(
        broker.is_connected("machine-same-instance"),
        "old same-instance stream keeps reconnected Machine",
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
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if follow_storage
                .find_message(
                    &workspace_id,
                    &channel_id,
                    "application-steer-chat",
                    "application-follow-chat",
                )
                .map_err(err)?
                .is_some()
            {
                return Ok::<_, String>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "follow message acceptance timed out")??;
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
    let waiting_message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(message_id) = verification_storage
                .find_message(
                    &workspace_id,
                    &channel_id,
                    "application-steer-chat",
                    "application-steer-message",
                )
                .map_err(err)?
            {
                if let Some(item) = verification_storage
                    .inbox_items(&workspace_id, &agent_id)
                    .map_err(err)?
                    .into_iter()
                    .find(|item| item.source_id == message_id)
                {
                    return Ok::<_, String>(item);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "queued message acceptance timed out")??;
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
        ws::serve_local(
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
        if socket.join("agent.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ensure(
        socket.join("agent.sock").exists(),
        "scenario Agent socket started",
    )?;
    let client =
        lankide_client::http::HttpManagementTransport::local(socket, DEFAULT_MAX_MESSAGE_BYTES);
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
        unknown_interface.code() == lankide_protocol::Code::NotFound,
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
        invalid_interface.code() == lankide_protocol::Code::InvalidArgument,
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
            machine_id: "machine-b".into(),
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
            machine_id: None,
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
        failed.is_err_and(|status| status.code() == lankide_protocol::Code::FailedPrecondition),
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
        onboarding.status == lankide_protocol::OnboardingStatus::Ready as i32
            && onboarding.verification_uri.is_empty()
            && onboarding.poll_interval_seconds == 0,
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
        serde_json::from_str::<Value>(&advanced_onboarding.payload_json)
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
    let mut agent_client =
        lankide_protocol::connect_uds(socket.join("agent.sock"), DEFAULT_MAX_MESSAGE_BYTES)
            .await
            .map_err(err)?;
    let denied = agent_client
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
        denied.code() == lankide_protocol::Code::PermissionDenied
            && !denied.message().contains("token_hash")
            && !denied.message().contains("must-not-leak"),
        "capability authorization is redacted",
    )?;
    let content = agent_client
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
    let profile = agent_client
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
    let profile = agent_client
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
    let historical = agent_client
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
            machine_id: None,
            avatar: None,
            attention: None,
            provider_agent_config_json: Some(json!({"deny_echo":true}).to_string()),
            provider_agent_secrets_json: None,
        })
        .await
        .map_err(|error| format!("mid-execution config update: {error}"))?;
    let listed_capabilities = agent_client
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
    let unread = agent_client
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
    let recall = agent_client
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
    // An Agent cannot post in an unknown conversation.
    let outside = agent_client
        .invoke_capability(CapabilityInvokeRequest {
            context: Some(context.clone()),
            capability_name: "message_send".into(),
            invocation_id: "send-outside".into(),
            arguments_json: json!({
                "channel_id": "group_chat",
                "chat_id": "chat-not-in-task",
                "content": "hello"
            })
            .to_string(),
        })
        .await
        .map_err(err)?
        .into_inner();
    ensure(
        !outside.success
            && outside.error.is_some_and(|error| {
                error.code == "invalid_request" && !error.retryable && !error.message.is_empty()
            }),
        "send to an inaccessible conversation is explained, not retried",
    )?;
    let unauthorized = agent_client
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
        let invoked = agent_client
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
    let (machine_frames, machine_input) = tokio::sync::mpsc::channel(8);
    machine_frames
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::Hello(MachineHello {
                device_credential: "fixture-machine-a".into(),
                protocol_version: PROTOCOL_VERSION,
                machine_id: "machine-a".into(),
                instance_id: "instance-2".into(),
                machine_version: "test".into(),
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
    let mut machine_output = ws::connect_stream(
        &socket,
        tokio_stream::wrappers::ReceiverStream::new(machine_input),
    )
    .await
    .map_err(err)?
    .into_inner();
    let _ = machine_output.message().await.map_err(err)?;
    let gate = Arc::new(tokio::sync::Notify::new());
    *state.tool_gate.lock().unwrap() = Some(gate.clone());
    machine_frames
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::CapabilityRequest(
                lankide_protocol::CapabilityRequest {
                    request_id: "slow-echo".into(),
                    method: "invoke_capability".into(),
                    context: Some(context.clone()),
                    offset: 0,
                    eof: true,
                    payload: lankide_protocol::agent::encode(&CapabilityInvokeRequest {
                        context: Some(context.clone()),
                        invocation_id: "slow-echo".into(),
                        capability_name: "echo".into(),
                        arguments_json: json!({"value":"delayed"}).to_string(),
                    })
                    .unwrap(),
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
    machine_frames
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::Heartbeat(MachineHeartbeat {
                machine_id: "machine-a".into(),
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
            let observed_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            // Leases use Runtime receipt time, not the peer's send timestamp.
            if current.lease_expires_at >= heartbeat_at + 15_000
                && current.lease_expires_at <= observed_at + 15_000
            {
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
    let tool_result = tokio::time::timeout(Duration::from_secs(2), machine_output.message())
        .await
        .map_err(err)?
        .map_err(err)?
        .ok_or("tool stream closed")?;
    ensure(
        matches!(tool_result.frame, Some(runtime_frame::Frame::CapabilityResponse(result)) if result.error_code == 0 && lankide_protocol::agent::decode::<lankide_protocol::CapabilityInvokeResult>(&result.payload).unwrap().success),
        "delayed tool result delivered after heartbeat",
    )?;
    let provider_lookup = agent_client
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
    let uploaded = agent_client
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
    let mut downloaded = agent_client
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
        denied_image.code() == lankide_protocol::Code::PermissionDenied,
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
        denied_workspace.code() == lankide_protocol::Code::PermissionDenied,
        "profile image workspace isolation",
    )?;

    state.push_send(SendMode::Sent("standard-tool-send".into()));
    let standard_send = agent_client
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
    let unsupported_edit = agent_client
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
        ("invalid-edit-input", json!({}), "invalid_request"),
    ] {
        machine_frames
            .send(MachineFrame {
                protocol_version: lankide_protocol::PROTOCOL_VERSION,
                frame: Some(machine_frame::Frame::CapabilityRequest(
                    lankide_protocol::CapabilityRequest {
                        request_id: id.into(),
                        method: "invoke_capability".into(),
                        context: Some(context.clone()),
                        offset: 0,
                        eof: true,
                        payload: lankide_protocol::agent::encode(&CapabilityInvokeRequest {
                            context: Some(context.clone()),
                            invocation_id: id.into(),
                            capability_name: "message_edit".into(),
                            arguments_json: arguments.to_string(),
                        })
                        .unwrap(),
                    },
                )),
            })
            .await
            .map_err(err)?;
        let response = tokio::time::timeout(Duration::from_secs(2), machine_output.message())
            .await
            .map_err(err)?
            .map_err(err)?
            .ok_or("tool failure stream closed")?;
        let Some(runtime_frame::Frame::CapabilityResponse(response)) = response.frame else {
            return Err("tool failure was not returned to Machine".into());
        };
        let result: lankide_protocol::CapabilityInvokeResult =
            lankide_protocol::agent::decode(&response.payload).map_err(err)?;
        ensure(
            !result.success
                && result.invocation_id == id
                && result
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == expected_code),
            "tool failure must reach the native session",
        )?;
        serde_json::from_str::<Value>(&result.structured_content_json)
            .map_err(|_| "tool failure payload must be consumable by Machine")?;
    }
    state.edit_supported.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        let edited = agent_client
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
    let unsupported_recall = agent_client
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
    let failed_recall = agent_client
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
        let recalled = agent_client
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
    let attachment_only = agent_client
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

    let attention = agent_client
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
    let frozen_messages = agent_client
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
    let guarded = agent_client
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
    let refreshed_messages = agent_client
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
    let forced = agent_client
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
            machine_id: "machine-delete".into(),
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
                .pending_agent_file_deletions("machine-delete")
                .map_err(err)?
                == vec![(delete_workspace.clone(), delete_agent.clone())],
        "Agent deletion must commit before Machine cleanup",
    )?;
    deletion_storage.close().map_err(err)?;

    let (delete_machine_frames, delete_machine_input) = tokio::sync::mpsc::channel(4);
    delete_machine_frames
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::Hello(MachineHello {
                device_credential: "fixture-machine-delete".into(),
                protocol_version: PROTOCOL_VERSION,
                machine_id: "machine-delete".into(),
                instance_id: "instance-delete".into(),
                machine_version: "test".into(),
                os: "test".into(),
                arch: "test".into(),
                harnesses: vec![lankide_protocol::HarnessInfo {
                    harness_id: "fixture".into(),
                    display_name: "Fixture".into(),
                    availability: lankide_protocol::HarnessAvailability::Available as i32,
                    ..Default::default()
                }],
            })),
        })
        .await
        .map_err(err)?;
    let mut delete_machine_output = ws::connect_stream(
        &socket,
        tokio_stream::wrappers::ReceiverStream::new(delete_machine_input),
    )
    .await
    .map_err(err)?
    .into_inner();
    let cleanup = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let frame = delete_machine_output
                .message()
                .await
                .map_err(err)?
                .ok_or("delete Machine stream closed")?;
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
    delete_machine_frames
        .send(MachineFrame {
            protocol_version: lankide_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::DeleteAgentFilesResult(
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
            registered_machine::register(&storage, &["fixture-machine"]).map_err(err)?;
            let pending = storage
                .pending_agent_file_deletions("machine-delete")
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
            machine_id: "machine-delete".into(),
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
    drop(delete_machine_frames);
    drop(delete_machine_output);
    drop(machine_frames);
    drop(machine_output);
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
    let report = restarted.recover(10).await.map_err(err)?;
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

async fn register(
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
        .await
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
        machine_id: "fixture-machine".into(),
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
        receiving_agent_id: None,
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
        receiving_agent_id: None,
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

fn successful_reply(channel: &ChannelId, chat: &str, content: &str) -> MachineOutcome {
    MachineOutcome::Succeeded {
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
fn no_reply_outcome() -> MachineOutcome {
    MachineOutcome::Succeeded {
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

struct ScriptMachine {
    state: Arc<FakeState>,
    inject: Mutex<Option<ProviderEvent>>,
    outcome: MachineOutcome,
    assignments: Mutex<Vec<AssignExecution>>,
}

struct GateMachine {
    release: tokio::sync::Mutex<Option<oneshot::Receiver<()>>>,
    started: tokio::sync::Notify,
    outcome: MachineOutcome,
}
impl MachineExecutor for GateMachine {
    fn execute(&self, _: AssignExecution) -> MachineFuture<'_, MachineCompletion> {
        Box::pin(async move {
            self.started.notify_one();
            let receiver = self
                .release
                .lock()
                .await
                .take()
                .ok_or_else(|| MachineError {
                    code: "gate_reused".into(),
                })?;
            receiver.await.map_err(|_| MachineError {
                code: "gate_closed".into(),
            })?;
            Ok(MachineCompletion {
                delivery_accepted: true,
                outcome: self.outcome.clone(),
            })
        })
    }
}
impl ScriptMachine {
    fn new(state: Arc<FakeState>, inject: Option<ProviderEvent>, outcome: MachineOutcome) -> Self {
        Self {
            state,
            inject: Mutex::new(inject),
            outcome,
            assignments: Mutex::new(Vec::new()),
        }
    }
}
impl MachineExecutor for ScriptMachine {
    fn execute(&self, assignment: AssignExecution) -> MachineFuture<'_, MachineCompletion> {
        Box::pin(async move {
            self.state.log.lock().expect("log").push("machine".into());
            self.assignments
                .lock()
                .expect("assignments")
                .push(assignment);
            let inject = self.inject.lock().expect("inject").take();
            if let Some(event) = inject {
                self.state
                    .emit(event)
                    .await
                    .map_err(|code| MachineError { code })?;
            }
            Ok(MachineCompletion {
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
    fn save<'a>(&'a self, value: ResourceUpload) -> ProviderFuture<'a, lankide_core::ResourceRef> {
        Box::pin(async move {
            Ok(lankide_core::ResourceRef {
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
    registered_machine::register(&storage, &["fixture-machine"]).unwrap();
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
            machine_id: "fixture-machine".into(),
            workspace_id: w.clone(),
            id: a.clone(),
            name: "Before".into(),
            now: 2,
        })
        .unwrap();
    let mut record = storage.agent_record(&w, &a).unwrap();
    storage
        .pair_machine(
            &lankide_storage::MachineRecord {
                is_local: false,
                machine_id: "profile-machine".into(),
                display_name: "Profile Machine".into(),
                credential_hash: "profile-credential-hash".into(),
                paired_at: 1,
                revoked_at: None,
                last_seen_at: None,
                os: "test".into(),
                version: "1".into(),
                inventory_json: serde_json::to_string(&["fixture", "pi", "codex"].map(|id| {
                    lankide_protocol::link::HarnessInfo {
                        harness_id: id.into(),
                        ..Default::default()
                    }
                }))
                .unwrap(),
            },
            None,
            600_000,
        )
        .unwrap();
    record.machine_id = "profile-machine".into();
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
            retry: Default::default(),
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
struct NeverAssignedMachine;
impl MachineExecutor for NeverAssignedMachine {
    fn execute(&self, _: AssignExecution) -> MachineFuture<'_, MachineCompletion> {
        Box::pin(async { panic!("failure must happen before assignment") })
    }
}
async fn prompt_failure_recovery_scenario() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = scenario_runtime(RuntimeConfig {
        database: DatabaseConfig::new(temp.path().join("runtime.db")),
        retry: Default::default(),
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
            machine_id: "fixture-machine".into(),
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
        .await
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
        .execute_ready(&w, &a, &NeverAssignedMachine, 3)
        .await;
    assert!(
        result.is_err(),
        "failure injection must reach dynamic prompt construction"
    );
    let pinned = runtime.storage().has_active_execution(&w).unwrap();
    let recovery = runtime.recoverable_executions(Some(&w)).await.unwrap();
    let stored = runtime.storage().execution(1).unwrap();
    assert_eq!(
        stored.error_code.as_deref(),
        Some("input_preparation_failed")
    );
    assert_eq!(
        stored.error_message.as_deref(),
        Some("runtime_provider_call: external (prompt_failure)")
    );
    assert!(matches!(
        runtime
            .execute_ready(&w, &a, &NeverAssignedMachine, 60_002)
            .await
            .unwrap(),
        ExecutionProgress::Idle
    ));
    runtime.shutdown().await.unwrap();
    drop(runtime);
    let factories = ProviderFactoryRegistry::default();
    factories
        .register(Arc::new(ApplicationFactory {
            provider_id: ProviderId::new("fixture").unwrap(),
            state: Arc::new(FakeState::default()),
        }))
        .unwrap();
    let application = RuntimeApplication::open(
        RuntimeConfig {
            database: DatabaseConfig::new(temp.path().join("runtime.db")),
            retry: Default::default(),
        },
        factories,
    )
    .await
    .unwrap();
    let details = RuntimePeerAdapter::new(application.clone())
        .get_execution_details(lankide_protocol::GetExecutionDetailsRequest {
            request_id: "preparation-reason".into(),
            execution_id: 1,
        })
        .await
        .unwrap();
    assert_eq!(details.state, "failed");
    assert!(details.attempts.is_empty());
    assert_eq!(details.error_code, stored.error_code);
    assert_eq!(details.error_message, stored.error_message);
    application.shutdown().await.unwrap();
    assert!(
        !pinned,
        "prompt failure left active execution and claimed inbox: {recovery:?}"
    );
}

async fn durable_ingress_retry_scenario() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("runtime.db");
    let runtime = scenario_runtime(RuntimeConfig {
        database: DatabaseConfig::new(&path),
        retry: Default::default(),
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
            machine_id: "fixture-machine".into(),
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
        .await
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
        let broker = app.machine_broker();
        let mut connection = broker
            .connect(MachineHello {
                device_credential: String::new(),
                protocol_version: PROTOCOL_VERSION,
                machine_id: "profile-machine".into(),
                instance_id: "config-audit".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let _ = connection.next().await.unwrap().unwrap();
        broker
            .handle_frame(
                "profile-machine",
                "config-audit",
                MachineFrame {
                    protocol_version: lankide_protocol::PROTOCOL_VERSION,
                    frame: Some(machine_frame::Frame::Heartbeat(MachineHeartbeat {
                        machine_id: "profile-machine".into(),
                        instance_id: "config-audit".into(),
                        sent_at: 0,
                        active_attempts: Vec::new(),
                    })),
                },
            )
            .await
            .unwrap();
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
                let frame = connection.next().await.expect("Machine stream").unwrap();
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

fn scenario_runtime(config: RuntimeConfig) -> Result<Runtime, lankide_runtime::RuntimeError> {
    let runtime = Runtime::new(config)?;
    registered_machine::register(runtime.storage(), &["fixture-machine", "machine-attempt"])?;
    Ok(runtime)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dm_task_can_read_and_post_to_received_group() {
    let (temp, app, state) = profile_application().await;
    let storage = Storage::open(DatabaseConfig::new(temp.path().join("runtime.db"))).unwrap();
    let workspace = WorkspaceId::new("profile_workspace").unwrap();
    let agent = AgentId::new("profile_agent").unwrap();
    let channel = ChannelId::new("chat").unwrap();
    let owner_dm = IngestEvent {
        receiving_agent_id: Some(agent.clone()),
        workspace_id: workspace.clone(),
        provider_id: ProviderId::new("fixture").unwrap(),
        channel_id: channel.clone(),
        channel_name: "Chat".into(),
        provider_chat_id: "owner-dm".into(),
        chat_name: "Owner".into(),
        provider_event_id: Some("owner-request".into()),
        provider_message_id: "owner-request".into(),
        message: MessageInput {
            sender_kind: lankide_core::MessageAuthorType::Human,
            sender_id: "owner".into(),
            sender_name: "Owner".into(),
            content: "Go to the group and @Diana".into(),
            mentions: vec![],
            reply_to_provider_message_id: None,
            resources: vec![],
            occurred_at: 3,
        },
        dispatch: vec![InboxDispatch {
            agent_id: agent.clone(),
            available_at: 3,
            trigger_reason: DispatchReason::Mention,
        }],
        now: 3,
    };
    storage.ingest_event(owner_dm.clone()).unwrap();
    let token = "group-access-token";
    let execution = storage
        .create_execution_with_auth(
            &workspace,
            &agent,
            "owner DM",
            4,
            ExecutionAuthSpec {
                attempt: 1,
                token_hash: Sha256::digest(token.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                expires_at: i64::MAX,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(execution.snapshot.chats.len(), 1);
    assert_eq!(
        execution.snapshot.chats[0].chat.provider_chat_id,
        "owner-dm"
    );
    let frozen = json!({"machine_id":"profile-machine", "harness_id":"fixture", "model":"fixture", "thinking_level":"low", "trigger_reason":"owner DM", "chats":[]}).to_string();
    storage
        .freeze_attempt_authorization(AttemptAuthorization {
            execution_id: execution.execution_id,
            attempt: 1,
            context_fingerprint: Sha256::digest(frozen.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            context_json: frozen,
            capabilities: ["chat_messages", "message_send"]
                .map(|name| AttemptCapability {
                    name: name.into(),
                    display_name: name.into(),
                    description: name.into(),
                    kind: lankide_protocol::CapabilityKind::Tool as i32,
                    input_schema_json: "{}".into(),
                    output_schema_json: "{}".into(),
                })
                .to_vec(),
        })
        .unwrap();
    storage
        .mark_execution_running(execution.execution_id, 4)
        .unwrap();
    storage
        .offer_execution_attempt(
            execution.execution_id,
            1,
            "profile-machine",
            "group-test",
            i64::MAX,
            4,
        )
        .unwrap();
    storage
        .accept_execution_attempt(execution.execution_id, 1, true, 4)
        .unwrap();
    let context = AgentRunContext {
        execution: Some(ExecutionRef {
            execution_id: execution.execution_id,
            attempt: 1,
        }),
        execution_token: token.into(),
    };
    let sink = state.sink.lock().unwrap().clone().unwrap();
    let emit_group = |chat: &str, id: &str, receiver: AgentId| {
        let mut event = message(&channel, chat, id, id, &agent, "owner");
        event.receiving_agent_id = Some(receiver);
        if let ProviderEventKind::Message(message) = &mut event.kind {
            message.mentioned_agent_ids.clear();
        }
        event
    };
    sink.emit(emit_group("team-group", "group-history", agent.clone()))
        .await
        .unwrap();
    // The Provider's acknowledgement means the event and access grant are durable.
    let invoke = |name: &str, id: &str, arguments: Value| CapabilityInvokeRequest {
        context: Some(context.clone()),
        capability_name: name.into(),
        invocation_id: id.into(),
        arguments_json: arguments.to_string(),
    };
    let send_args = json!({"channel_id":"chat", "chat_id":"team-group", "content":"@Diana hello"});
    let unread = app
        .invoke_capability(invoke("message_send", "before-read", send_args.clone()))
        .await
        .unwrap();
    assert!(!unread.success);
    assert!(
        unread
            .error
            .is_some_and(|error| !error.retryable && error.message.contains("chat_messages")),
        "an unread member conversation must explain how to read it"
    );
    let mut forced_unread_args = send_args.clone();
    forced_unread_args["force"] = json!(true);
    let unread = app
        .invoke_capability(invoke(
            "message_send",
            "force-before-read",
            forced_unread_args,
        ))
        .await
        .unwrap();
    assert!(
        !unread.success
            && unread
                .error
                .is_some_and(|error| !error.retryable && error.message.contains("chat_messages"))
    );
    let read_args = json!({"channel_id":"chat", "chat_id":"team-group"});
    let read = app
        .invoke_capability(invoke("chat_messages", "read-group", read_args.clone()))
        .await
        .unwrap();
    assert!(
        read.success && read.structured_content_json.contains("group-history"),
        "DM task must read its received group: {read:?}"
    );
    state.push_send(SendMode::Sent("group-post".into()));
    assert!(
        app.invoke_capability(invoke("message_send", "post-group", send_args.clone()))
            .await
            .unwrap()
            .success
    );
    sink.emit(emit_group("team-group", "new-group-message", agent.clone()))
        .await
        .unwrap();
    let blocked = app
        .invoke_capability(invoke("message_send", "stale-post", send_args.clone()))
        .await
        .unwrap();
    assert!(
        !blocked.success
            && blocked
                .error
                .is_some_and(|error| error.code == "new_messages")
    );
    let read = app
        .invoke_capability(invoke("chat_messages", "reread-group", read_args))
        .await
        .unwrap();
    assert!(read.success && read.structured_content_json.contains("new-group-message"));
    state.push_send(SendMode::Sent("group-post-after-read".into()));
    assert!(
        app.invoke_capability(invoke("message_send", "post-after-read", send_args))
            .await
            .unwrap()
            .success
    );
    // Reading an older page must not acknowledge newer messages in that group.
    sink.emit(emit_group("history-group", "older-note", agent.clone()))
        .await
        .unwrap();
    sink.emit(emit_group("history-group", "latest-note", agent.clone()))
        .await
        .unwrap();
    let latest = storage
        .find_message(&workspace, &channel, "history-group", "latest-note")
        .unwrap()
        .unwrap();
    let page = app.invoke_capability(invoke("chat_messages", "older-page", json!({"channel_id":"chat", "chat_id":"history-group", "before_message_id":latest, "limit":1}))).await.unwrap();
    assert!(
        page.success
            && page.structured_content_json.contains("older-note")
            && !page.structured_content_json.contains("latest-note")
    );
    let stale = app
        .invoke_capability(invoke(
            "message_send",
            "older-page-post",
            json!({"channel_id":"chat", "chat_id":"history-group", "content":"outdated"}),
        ))
        .await
        .unwrap();
    assert!(
        !stale.success
            && stale
                .error
                .is_some_and(|error| error.code == "new_messages")
    );
    // A known group received by another Agent must not grant this Agent access.
    app.create_agent(CreateAgentRequest {
        request_id: "other-member".into(),
        agent: Some(AgentRef {
            workspace_id: workspace.to_string(),
            agent_id: "other_agent".into(),
        }),
        name: "Other".into(),
        machine_id: "profile-machine".into(),
        harness_id: "fixture".into(),
        model: "fixture".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    sink.emit(emit_group(
        "private-group",
        "private-history",
        AgentId::new("other_agent").unwrap(),
    ))
    .await
    .unwrap();
    for name in ["chat_messages", "message_send"] {
        let result = app
            .invoke_capability(invoke(
                name,
                &format!("forbidden-{name}"),
                json!({"channel_id":"chat", "chat_id":"private-group", "content":"forbidden"}),
            ))
            .await
            .unwrap();
        assert!(!result.success && result.error.is_some_and(|error| !error.retryable));
    }
    // An existing DM outside the task is readable only by its receiving Agent,
    // even when another Agent's DM has the same native chat and message IDs.
    let mut own_dm = owner_dm;
    own_dm.provider_chat_id = "shared-dm-id".into();
    own_dm.provider_message_id = "shared-native-id".into();
    own_dm.provider_event_id = Some("own-dm-event".into());
    own_dm.message.content = "own DM history".into();
    own_dm.dispatch.clear();
    storage.ingest_event(own_dm.clone()).unwrap();
    own_dm.receiving_agent_id = Some(AgentId::new("other_agent").unwrap());
    own_dm.provider_event_id = Some("other-dm-event".into());
    own_dm.message.content = "other Agent private history".into();
    storage.ingest_event(own_dm).unwrap();
    let own_dm = app
        .invoke_capability(invoke(
            "chat_messages",
            "read-own-dm",
            json!({"channel_id":"chat", "chat_id":"shared-dm-id"}),
        ))
        .await
        .unwrap();
    assert!(
        own_dm.success
            && own_dm.structured_content_json.contains("own DM history")
            && !own_dm
                .structured_content_json
                .contains("other Agent private history")
    );
    app.shutdown().await.unwrap();
    storage.close().unwrap();
}
