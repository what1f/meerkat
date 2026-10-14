use lankide_core::{
    AgentId, ChannelId, MessageAuthorType, PersonId, ProviderId, ResourceId, WorkspaceId,
};
use lankide_storage::{
    AgentRecord, AgentSpec, AttemptAuthorization, AttemptCapability, AttentionRecord, CommitResult,
    DatabaseConfig, DispatchReason, ExecutionAuthSpec, ExecutionEventDisposition, ExecutionState,
    GuardedSendResult, InboxDispatch, InboxState, IncomingMessage, IncomingReaction, IngestEvent,
    MessageInput, ReactionAction, ReactionOrigin, RecordExecutionEvent, ResourceMetadata,
    ResourceView, SecretValue, SendOutcome, SendRequest, Storage, TeamRole, ToolInvocationRequest,
    WorkspaceRecord, WorkspaceSetupStatus, WorkspaceSpec,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn storage_business_flow_uses_real_sqlite_wal() {
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_millis();
    let started = Instant::now();
    let run_id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos()
    );
    let artifact_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/e2e-artifacts/storage")
        .join(&run_id);
    fs::create_dir_all(&artifact_dir).expect("create artifact directory");

    let (sender, receiver) = mpsc::channel();
    let worker_run_id = run_id.clone();
    let worker_artifact_dir = artifact_dir.clone();
    let worker_started = started;
    std::thread::spawn(move || {
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            run_scenario(&worker_run_id, &worker_artifact_dir)
        }));
        let result = match outcome {
            Ok(Ok(summary)) => {
                write_result(
                    &worker_artifact_dir,
                    &worker_run_id,
                    "passed",
                    started_at,
                    worker_started.elapsed().as_millis(),
                    summary,
                );
                Ok(())
            }
            Ok(Err(error)) => {
                write_result(
                    &worker_artifact_dir,
                    &worker_run_id,
                    "failed",
                    started_at,
                    worker_started.elapsed().as_millis(),
                    json!({ "error": "storage scenario returned an error" }),
                );
                Err(error)
            }
            Err(payload) => {
                let error = payload
                    .downcast_ref::<&str>()
                    .map(|value| (*value).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "scenario panicked".to_owned());
                write_result(
                    &worker_artifact_dir,
                    &worker_run_id,
                    "failed",
                    started_at,
                    worker_started.elapsed().as_millis(),
                    json!({ "error": "storage scenario panicked" }),
                );
                Err(error)
            }
        };
        let _ = sender.send(result);
    });

    match receiver.recv_timeout(Duration::from_secs(45)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("storage scenario failed: {error}"),
        Err(_) => {
            write_result(
                &artifact_dir,
                &run_id,
                "timed_out",
                started_at,
                started.elapsed().as_millis(),
                json!({ "error": "hard timeout after 45 seconds" }),
            );
            panic!("storage scenario exceeded hard timeout")
        }
    }
}

fn run_scenario(run_id: &str, artifact_dir: &Path) -> Result<serde_json::Value, String> {
    let temp_home = tempfile::Builder::new()
        .prefix(&format!("lankide-storage-{run_id}-"))
        .tempdir()
        .map_err(|error| error.to_string())?;
    let db_path = temp_home.path().join("runtime.sqlite3");
    let config = DatabaseConfig::new(&db_path);

    let storage = Storage::open(config.clone()).map_err(|error| error.to_string())?;
    assert_eq!(storage.schema_version().unwrap(), 17);
    assert_eq!(storage.sqlite_settings().unwrap().journal_mode, "wal");
    assert!(storage.sqlite_settings().unwrap().foreign_keys);

    let provider = ProviderId::new("telegram").unwrap();
    let provider_b = ProviderId::new("feishu").unwrap();
    let workspace_a = WorkspaceId::new("alpha").unwrap();
    let workspace_b = WorkspaceId::new("beta").unwrap();
    let agent_id = AgentId::new("helper").unwrap();
    let channel_id = ChannelId::new("messages").unwrap();
    for (workspace_id, name, workspace_provider) in [
        (&workspace_a, "Alpha", &provider),
        (&workspace_b, "Beta", &provider_b),
    ] {
        storage
            .create_workspace(WorkspaceSpec {
                id: workspace_id.clone(),
                name: name.to_owned(),
                provider_id: workspace_provider.clone(),
                provider_config: json!({ "tasks": true }),
                provider_secret: json!({ "token": format!("secret-{name}") }),
                active: true,
                now: 1_000,
            })
            .unwrap();
        storage
            .create_agent(AgentSpec {
                workspace_id: workspace_id.clone(),
                id: agent_id.clone(),
                name: "Same visible name".to_owned(),
                now: 1_001,
            })
            .unwrap();
    }
    assert_eq!(storage.list_workspaces().unwrap().len(), 2);
    assert_eq!(storage.list_agents(&workspace_a).unwrap().len(), 1);
    assert_eq!(storage.list_agents(&workspace_b).unwrap().len(), 1);
    let person = storage
        .get_or_create_person(&workspace_a, &provider, "telegram-user-42", 1_010)
        .unwrap();
    assert_eq!(person.person_id.as_str().len(), 6);
    assert!(PersonId::new(person.person_id.to_string()).is_ok());
    assert_eq!(person.team_role, TeamRole::External);
    assert_eq!(person.provider_state_revision, 0);
    assert_eq!(
        storage
            .get_or_create_person(&workspace_a, &provider, "telegram-user-42", 1_011)
            .unwrap()
            .person_id,
        person.person_id
    );
    let person = storage
        .compare_and_set_person_state(
            &workspace_a,
            &person.person_id,
            0,
            &json!({"username":"leo"}),
            1_012,
        )
        .unwrap();
    assert_eq!(person.provider_state_revision, 1);
    assert_eq!(person.provider_identity_state, json!({"username":"leo"}));
    assert!(
        storage
            .compare_and_set_person_state(
                &workspace_a,
                &person.person_id,
                0,
                &json!({"username":"stale"}),
                1_013,
            )
            .is_err()
    );
    let person = storage
        .initialize_team_lead(&workspace_a, &person.person_id, 1_014)
        .unwrap();
    assert_eq!(person.team_role, TeamRole::Lead);
    let teammate = storage
        .get_or_create_person(&workspace_a, &provider, "telegram-user-84", 1_015)
        .unwrap();
    let teammate = storage
        .update_team_member(
            &workspace_a,
            &person.person_id,
            &teammate.person_id,
            TeamRole::Member,
            1_016,
        )
        .unwrap();
    assert_eq!(teammate.team_role, TeamRole::Member);
    let teammate = storage
        .update_team_member(
            &workspace_a,
            &person.person_id,
            &teammate.person_id,
            TeamRole::Lead,
            1_017,
        )
        .unwrap();
    assert_eq!(teammate.team_role, TeamRole::Lead);
    storage
        .get_or_create_person(&workspace_a, &provider, "telegram-user-external", 1_018)
        .unwrap();
    let members = storage.list_human_team_members(&workspace_a).unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.iter().any(|value| {
        value.person_id == teammate.person_id && value.team_role == TeamRole::Lead
    }));
    assert!(
        members
            .iter()
            .all(|value| value.team_role != TeamRole::External),
        "external people must not appear in the Team member list"
    );
    assert!(
        storage
            .list_human_team_members(&workspace_b)
            .unwrap()
            .is_empty(),
        "Team members must remain scoped to one Workspace"
    );
    assert_eq!(
        storage
            .person(&workspace_a, &person.person_id)
            .unwrap()
            .unwrap()
            .team_role,
        TeamRole::Member
    );
    let ordinary = serde_json::to_string(&storage.workspace(&workspace_a).unwrap()).unwrap();
    assert!(!ordinary.contains("secret-Alpha"));
    assert!(
        storage.provider_secret(&workspace_a).unwrap() == json!({ "token": "secret-Alpha" }),
        "explicit secret retrieval mismatch"
    );

    let workspace_a_record = storage
        .upsert_workspace_record(WorkspaceRecord {
            id: workspace_a.clone(),
            name: "Alpha full".to_owned(),
            provider_id: provider.clone(),
            provider_config: json!({"mode":"polling"}),
            provider_state: json!({}),
            rules: "Always be concise".to_owned(),
            cover_resource_id: None,
            setup_status: WorkspaceSetupStatus::Pending,
            setup_error_code: None,
            secret_ref: None,
            active: true,
            created_at: 1_000,
            updated_at: 1_100,
        })
        .unwrap();
    let workspace_b_record = storage
        .upsert_workspace_record(WorkspaceRecord {
            id: workspace_b.clone(),
            name: "Beta full".to_owned(),
            provider_id: provider_b.clone(),
            provider_config: json!({"mode":"webhook"}),
            provider_state: json!({}),
            rules: "Work in public".to_owned(),
            cover_resource_id: None,
            setup_status: WorkspaceSetupStatus::Ready,
            setup_error_code: None,
            secret_ref: None,
            active: true,
            created_at: 1_000,
            updated_at: 1_100,
        })
        .unwrap();
    assert_eq!(workspace_a_record.provider_id, provider);
    assert_eq!(workspace_b_record.provider_id, provider_b);
    let setup_error = storage
        .set_workspace_setup(
            &workspace_a,
            WorkspaceSetupStatus::Error,
            Some("provider_auth_failed"),
            1_101,
        )
        .unwrap();
    assert_eq!(setup_error.setup_status, WorkspaceSetupStatus::Error);
    assert_eq!(
        setup_error.setup_error_code.as_deref(),
        Some("provider_auth_failed")
    );
    let setup_ready = storage
        .set_workspace_setup(&workspace_a, WorkspaceSetupStatus::Ready, None, 1_102)
        .unwrap();
    assert_eq!(setup_ready.setup_status, WorkspaceSetupStatus::Ready);
    assert!(setup_ready.setup_error_code.is_none());

    let agent_a = storage
        .upsert_agent_record(agent_record(
            &workspace_a,
            &agent_id,
            "telegram_sender",
            1_110,
        ))
        .unwrap();
    let agent_b = storage
        .upsert_agent_record(agent_record(
            &workspace_b,
            &agent_id,
            "feishu_sender",
            1_111,
        ))
        .unwrap();
    assert_eq!(agent_a.name, agent_b.name);
    assert_ne!(agent_a.provider_agent_config, agent_b.provider_agent_config);
    let agent_a = storage
        .commit_agent_record(agent_a, &json!({"bot_token":"agent-secret-alpha"}))
        .unwrap();
    let agent_secret = storage
        .read_secret(&workspace_a, "agent:helper:bot_token")
        .unwrap()
        .unwrap();
    assert_eq!(agent_secret.expose_secret(), "agent-secret-alpha");
    assert!(
        !serde_json::to_string(&agent_a)
            .unwrap()
            .contains("agent-secret-alpha")
    );
    let mut invalid_attention = agent_a.clone();
    invalid_attention.attention.insert(
        ChannelId::new("invalid_schedule").unwrap(),
        AttentionRecord::Schedule {
            interval_minutes: 7,
        },
    );
    assert!(storage.upsert_agent_record(invalid_attention).is_err());

    let departed_id = AgentId::new("departed").unwrap();
    let departed = storage
        .commit_agent_record(
            agent_record(&workspace_a, &departed_id, "departed_sender", 1_115),
            &json!({"bot_token":"departed-secret"}),
        )
        .unwrap();
    assert!(storage.delete_agent(&workspace_a, &departed_id).unwrap());
    assert!(storage.agent_record(&workspace_a, &departed_id).is_err());
    assert_eq!(
        storage.pending_agent_file_deletions("local").unwrap(),
        vec![(workspace_a.clone(), departed_id.clone())]
    );
    assert!(
        storage
            .read_secret(&workspace_a, "agent:departed:bot_token")
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .complete_agent_file_deletion(&workspace_a, &departed_id)
            .unwrap()
    );
    assert!(
        storage
            .pending_agent_file_deletions("local")
            .unwrap()
            .is_empty()
    );
    let mut recreation = departed;
    recreation.name = "Recreated Agent".into();
    recreation.created_at = 1_116;
    recreation.updated_at = 1_116;
    let recreated = storage
        .insert_new_agent_record(recreation, &json!({}))
        .unwrap();
    assert_eq!(recreated.name, "Recreated Agent");
    let replacement_id = AgentId::new("replacement").unwrap();
    let replacement = storage
        .commit_agent_record(
            agent_record(&workspace_a, &replacement_id, "replacement_sender", 1_116),
            &json!({}),
        )
        .unwrap();

    let secret_ref = storage
        .put_secret(
            &workspace_a,
            "bot_token",
            SecretValue::new("named-secret-alpha"),
            1_120,
        )
        .unwrap();
    assert_eq!(secret_ref.revision, 1);
    let secret = storage
        .read_secret(&workspace_a, "bot_token")
        .unwrap()
        .unwrap();
    assert_eq!(secret.expose_secret(), "named-secret-alpha");
    assert!(!format!("{secret:?}").contains("named-secret-alpha"));
    assert!(
        !format!("{:?}", storage.workspace_record(&workspace_a).unwrap())
            .contains("named-secret-alpha")
    );
    assert_eq!(
        storage
            .workspace_record(&workspace_a)
            .unwrap()
            .secret_ref
            .unwrap()
            .revision,
        1
    );

    storage
        .provider_kv_put(&workspace_a, "telegram", "cursor", b"alpha-cursor", 1_130)
        .unwrap();
    storage
        .provider_kv_put(&workspace_b, "telegram", "cursor", b"beta-cursor", 1_131)
        .unwrap();
    storage
        .provider_kv_put(&workspace_a, "tasks", "cursor", b"task-cursor", 1_132)
        .unwrap();
    assert_eq!(
        storage
            .provider_kv_get(&workspace_a, "telegram", "cursor")
            .unwrap(),
        Some(b"alpha-cursor".to_vec())
    );
    assert_eq!(
        storage
            .provider_kv_get(&workspace_b, "telegram", "cursor")
            .unwrap(),
        Some(b"beta-cursor".to_vec())
    );
    assert_eq!(
        storage
            .provider_kv_get(&workspace_a, "tasks", "cursor")
            .unwrap(),
        Some(b"task-cursor".to_vec())
    );

    let allocated_a1 = storage.allocate_resource_id(&workspace_a, 1_139).unwrap();
    let allocated_a2 = storage.allocate_resource_id(&workspace_a, 1_140).unwrap();
    let allocated_b1 = storage.allocate_resource_id(&workspace_b, 1_141).unwrap();
    assert_eq!(allocated_a1.get(), 1);
    assert_eq!(allocated_a2.get(), 2);
    assert_eq!(allocated_b1.get(), 1);

    let resource = ResourceMetadata {
        workspace_id: workspace_a.clone(),
        resource_id: ResourceId::new(7).unwrap(),
        file_name: "restart.txt".to_owned(),
        media_type: "text/plain".to_owned(),
        size_bytes: 7,
        sha256: "b".repeat(64),
        storage_ref: "resources/7".to_owned(),
        created_at: 1_142,
    };
    assert_eq!(
        storage.upsert_resource_metadata(resource.clone()).unwrap(),
        resource
    );

    let mut first_event = event(
        &workspace_a,
        &provider,
        &channel_id,
        "chat-a",
        "provider-message-1",
        "first",
        2_000,
        vec![dispatch(&agent_id, DispatchReason::Mention, 2_000)],
    );
    first_event.message.sender_id = teammate.person_id.to_string();
    first_event.message.sender_name = "Example User".to_owned();
    first_event.message.mentions = vec![agent_id.clone()];
    first_event.message.resources = vec![ResourceView {
        resource_id: ResourceId::new(1).unwrap(),
        file_name: "note.txt".to_owned(),
        media_type: "text/plain".to_owned(),
        size_bytes: 4,
        sha256: "a".repeat(64),
    }];
    let first = storage.ingest_event(first_event).unwrap();
    let duplicate = storage
        .ingest_event(event(
            &workspace_a,
            &provider,
            &channel_id,
            "chat-a",
            "provider-message-1",
            "duplicate payload ignored",
            2_001,
            vec![dispatch(&agent_id, DispatchReason::Follow, 2_001)],
        ))
        .unwrap();
    assert_eq!(first.message_id, duplicate.message_id);
    assert!(duplicate.duplicate);
    assert_eq!(storage.message_count(&workspace_a).unwrap(), 1);
    let member_views = storage.list_human_team_member_views(&workspace_a).unwrap();
    assert!(member_views.iter().any(|value| {
        value.person_id == teammate.person_id
            && value.team_role == TeamRole::Lead
            && value.name.as_deref() == Some("Example User")
    }));
    assert_eq!(
        storage.inbox_items(&workspace_a, &agent_id).unwrap().len(),
        1
    );

    let mut later_chat_a = Vec::new();
    for index in 2..=4 {
        later_chat_a.push(
            storage
                .ingest_event(event(
                    &workspace_a,
                    &provider,
                    &channel_id,
                    "chat-a",
                    &format!("provider-message-a-{index}"),
                    &format!("later {index}"),
                    2_000 + index,
                    Vec::new(),
                ))
                .unwrap(),
        );
    }
    let last_before_snapshot = later_chat_a.last().unwrap();

    let chat_b_message = storage
        .ingest_event(event(
            &workspace_a,
            &provider,
            &channel_id,
            "chat-b",
            "provider-message-2",
            "second chat",
            2_002,
            vec![dispatch(&agent_id, DispatchReason::Follow, 2_002)],
        ))
        .unwrap();
    let execution = storage
        .create_execution_with_auth(
            &workspace_a,
            &agent_id,
            "messages",
            3_000,
            ExecutionAuthSpec {
                attempt: 1,
                token_hash: "sha256:test-token-hash".to_owned(),
                expires_at: 9_000,
            },
        )
        .unwrap()
        .expect("two pending chats create an execution");
    let auth = storage
        .execution_auth_context(execution.execution_id, 1)
        .unwrap()
        .expect("execution auth persisted");
    assert_eq!(auth.workspace_id, workspace_a);
    assert_eq!(auth.agent_id, agent_id);
    assert!(!format!("{auth:?}").contains("test-token-hash"));
    assert!(
        storage
            .execution_auth_context(execution.execution_id, 2)
            .unwrap()
            .is_none()
    );
    let authorization = AttemptAuthorization {
        execution_id: execution.execution_id,
        attempt: 1,
        context_json: json!({"frozen":true}).to_string(),
        context_fingerprint: "fingerprint-1".into(),
        capabilities: vec![AttemptCapability {
            name: "message_send".into(),
            display_name: "message_send".into(),
            description: "send".into(),
            kind: 1,
            input_schema_json: "{}".into(),
            output_schema_json: "{}".into(),
        }],
    };
    storage
        .freeze_attempt_authorization(authorization.clone())
        .unwrap();
    storage
        .freeze_attempt_authorization(authorization.clone())
        .unwrap();
    assert_eq!(
        storage
            .attempt_authorization(execution.execution_id, 1)
            .unwrap(),
        Some(authorization.clone())
    );
    let mut changed_authorization = authorization;
    changed_authorization.capabilities[0].name = "guessed".into();
    assert!(
        storage
            .freeze_attempt_authorization(changed_authorization)
            .is_err()
    );
    assert_eq!(execution.chats.len(), 2);
    assert_eq!(execution.inbox_item_ids.len(), 2);
    assert_eq!(execution.snapshot.chats.len(), 2);
    assert_eq!(execution.snapshot.chats[0].trigger_messages.len(), 1);
    let chat_a_snapshot = execution
        .snapshot
        .chats
        .iter()
        .find(|chat| chat.chat.chat_id == first.chat_id)
        .unwrap();
    assert_eq!(
        chat_a_snapshot
            .recent_messages
            .iter()
            .map(|message| message.message_id)
            .collect::<Vec<_>>(),
        vec![
            first.message_id,
            later_chat_a[1].message_id,
            later_chat_a[2].message_id,
        ]
    );
    assert_eq!(
        chat_a_snapshot
            .mentioned_message
            .as_ref()
            .map(|message| message.message_id),
        Some(first.message_id)
    );
    let frozen_first = execution
        .snapshot
        .chats
        .iter()
        .flat_map(|chat| chat.trigger_messages.iter())
        .find(|message| message.message_id == first.message_id)
        .unwrap();
    assert_eq!(frozen_first.mentions, vec![agent_id.clone()]);
    assert_eq!(frozen_first.resources[0].file_name, "note.txt");
    assert!(
        storage
            .mark_execution_running(execution.execution_id, 3_001)
            .unwrap()
    );
    assert!(
        storage
            .mark_delivery_delivered(execution.delivery_id, 3_002)
            .unwrap()
    );

    let mut late_event = event(
        &workspace_a,
        &provider,
        &channel_id,
        "chat-a",
        "provider-message-3",
        "arrived during execution",
        3_003,
        vec![dispatch(&agent_id, DispatchReason::DirectMessage, 3_003)],
    );
    late_event.message.reply_to_provider_message_id = Some("provider-message-1".to_owned());
    let late = storage.ingest_event(late_event).unwrap();
    let full = storage
        .snapshot_message(execution.snapshot_id, first.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(full.content, frozen_first.content);
    assert_eq!(full.resources, frozen_first.resources);
    let historical = storage
        .snapshot_message(execution.snapshot_id, later_chat_a[0].message_id)
        .unwrap()
        .unwrap();
    assert_eq!(historical.message_id, later_chat_a[0].message_id);
    assert!(
        storage
            .snapshot_message(execution.snapshot_id, late.message_id)
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .snapshot_message(i64::MAX, first.message_id)
            .unwrap()
            .is_none()
    );
    assert!(
        execution
            .snapshot
            .chats
            .iter()
            .all(|chat| chat.read_to_message_id != late.message_id)
    );
    assert_eq!(
        storage.snapshot(execution.snapshot_id).unwrap(),
        execution.snapshot
    );

    let guarded_request = SendRequest {
        workspace_id: workspace_a.clone(),
        channel_id: channel_id.clone(),
        provider_chat_id: "chat-a".to_owned(),
        request_key: "guarded-proactive-1".to_owned(),
        sender_agent_id: agent_id.clone(),
        content: "confirm after review".to_owned(),
        now: 3_003,
    };
    assert!(matches!(
        storage
            .begin_send_guarded(
                execution.snapshot_id,
                &channel_id,
                "chat-a",
                false,
                guarded_request.clone(),
            )
            .unwrap(),
        GuardedSendResult::NewMessages {
            snapshot_message_id,
            latest_message_id,
        } if latest_message_id == late.message_id && snapshot_message_id == last_before_snapshot.message_id
    ));
    let refreshed = storage
        .snapshot_message(execution.snapshot_id, late.message_id)
        .unwrap()
        .expect("the guarded send must expose the message that blocked it");
    assert_eq!(refreshed.content, "arrived during execution");
    assert_eq!(
        storage.snapshot(execution.snapshot_id).unwrap().chats[0].read_to_message_id,
        late.message_id
    );
    let forced = match storage
        .begin_send_guarded(
            execution.snapshot_id,
            &channel_id,
            "chat-a",
            true,
            guarded_request,
        )
        .unwrap()
    {
        GuardedSendResult::Ready(attempt) => attempt,
        other => panic!("forced guarded send was rejected: {other:?}"),
    };
    assert!(forced.created);
    assert!(
        storage
            .finish_send(
                "guarded-proactive-1",
                SendOutcome::Sent {
                    provider_message_id: "guarded-provider-1".to_owned(),
                },
                3_004,
            )
            .unwrap()
            .changed
    );

    let response = storage
        .begin_execution_send(
            execution.execution_id,
            SendRequest {
                workspace_id: workspace_a.clone(),
                channel_id: channel_id.clone(),
                provider_chat_id: "chat-a".to_owned(),
                request_key: "response-1".to_owned(),
                sender_agent_id: agent_id.clone(),
                content: "done".to_owned(),
                now: 3_004,
            },
        )
        .unwrap();
    let provider_message = storage
        .provider_message(&workspace_a, first.message_id)
        .unwrap()
        .expect("Provider message view");
    assert_eq!(
        provider_message.provider_message_id.as_deref(),
        Some("provider-message-1")
    );
    assert_eq!(provider_message.channel_id, channel_id);
    assert_eq!(provider_message.provider_chat_id, "chat-a");
    storage
        .set_chat_attention(
            &workspace_a,
            &agent_id,
            &channel_id,
            "chat-a",
            &AttentionRecord::Mute,
            2_100,
        )
        .unwrap();
    storage
        .set_chat_attention(
            &workspace_a,
            &agent_id,
            &channel_id,
            "chat-b",
            &AttentionRecord::Schedule {
                interval_minutes: 5,
            },
            2_101,
        )
        .unwrap();
    assert_eq!(
        storage
            .chat_attention(&workspace_a, &agent_id, &channel_id, "chat-a")
            .unwrap(),
        Some(AttentionRecord::Mute)
    );
    assert_eq!(
        storage
            .chat_attention(&workspace_a, &agent_id, &channel_id, "chat-b")
            .unwrap(),
        Some(AttentionRecord::Schedule {
            interval_minutes: 5
        })
    );
    assert!(
        storage
            .finish_send(
                "response-1",
                SendOutcome::Sent {
                    provider_message_id: "provider-response-1".to_owned()
                },
                3_005
            )
            .unwrap()
            .changed
    );
    assert!(
        !storage
            .finish_send(
                "response-1",
                SendOutcome::Sent {
                    provider_message_id: "provider-response-1".to_owned()
                },
                3_005
            )
            .unwrap()
            .changed
    );
    assert!(
        storage
            .finish_send(
                "response-1",
                SendOutcome::Sent {
                    provider_message_id: "different-provider-response".to_owned()
                },
                3_005
            )
            .is_err()
    );
    let self_echo = storage
        .ingest_event(event(
            &workspace_a,
            &provider,
            &channel_id,
            "chat-a",
            "provider-response-1",
            "provider echo of the Agent reply",
            3_005,
            vec![
                dispatch(&agent_id, DispatchReason::Follow, 3_005),
                dispatch(&replacement_id, DispatchReason::Follow, 3_005),
            ],
        ))
        .unwrap();
    assert!(self_echo.duplicate);
    assert_eq!(self_echo.new_inbox_items.len(), 1);
    assert_eq!(self_echo.new_inbox_items[0].0, replacement_id);
    assert!(
        storage
            .inbox_items(&workspace_a, &agent_id)
            .unwrap()
            .iter()
            .all(|item| item.source_id != self_echo.message_id),
        "an Agent must not receive its own Provider echo"
    );
    assert!(
        storage
            .commit_execution(execution.execution_id, "done", Some("response-1"), 3_006)
            .unwrap()
            == CommitResult::Committed
    );
    assert!(
        storage
            .commit_execution(
                execution.execution_id,
                "duplicate",
                Some("response-1"),
                3_007
            )
            .unwrap()
            == CommitResult::AlreadyCommitted
    );
    let cursors = storage.cursors(&workspace_a, &agent_id).unwrap();
    assert_eq!(cursors.len(), 2);
    assert_eq!(
        cursors
            .iter()
            .find(|cursor| cursor.chat_id == first.chat_id)
            .unwrap()
            .read_to_message_id,
        late.message_id
    );
    assert_eq!(
        cursors
            .iter()
            .find(|cursor| cursor.chat_id == chat_b_message.chat_id)
            .unwrap()
            .read_to_message_id,
        chat_b_message.message_id
    );
    let pending = storage.inbox_items(&workspace_a, &agent_id).unwrap();
    assert_eq!(
        pending
            .iter()
            .filter(|item| item.state == InboxState::Pending)
            .count(),
        0
    );
    assert_eq!(
        pending
            .iter()
            .find(|item| item.source_id == late.message_id)
            .unwrap()
            .state,
        InboxState::Consumed
    );

    let reaction = IncomingReaction {
        receiving_agent_id: None,
        workspace_id: workspace_a.clone(),
        provider_id: provider.clone(),
        channel_id: channel_id.clone(),
        provider_chat_id: "chat-a".to_owned(),
        target_provider_message_id: "provider-message-3".to_owned(),
        sender_kind: MessageAuthorType::Human,
        sender_id: "human-2".to_owned(),
        sender_name: "Second Human".to_owned(),
        reaction: "eyes".to_owned(),
        action: ReactionAction::Add,
        origin: ReactionOrigin::Provider,
        provider_event_id: "reaction-event-1".to_owned(),
        dispatch: vec![dispatch(
            &agent_id,
            DispatchReason::Reaction {
                reaction: "eyes".to_owned(),
            },
            3_100,
        )],
        occurred_at: 3_100,
        now: 3_100,
    };
    let mut unknown_reaction = reaction.clone();
    unknown_reaction.target_provider_message_id = "provider-message-outside-runtime".to_owned();
    unknown_reaction.provider_event_id = "reaction-event-outside-runtime".to_owned();
    let ignored = storage.record_reaction(unknown_reaction).unwrap();
    assert!(ignored.ignored_unknown_target);
    assert_eq!(ignored.reaction_id, None);
    assert_eq!(ignored.target_message_id, None);
    assert!(ignored.inbox_item_ids.is_empty());
    let recorded = storage.record_reaction(reaction.clone()).unwrap();
    assert!(!recorded.duplicate);
    assert!(!recorded.ignored_unknown_target);
    assert_eq!(recorded.inbox_item_ids.len(), 1);
    assert!(storage.record_reaction(reaction).unwrap().duplicate);
    let inbox_before_auto = storage.inbox_items(&workspace_a, &agent_id).unwrap().len();
    let auto = storage
        .record_reaction(IncomingReaction {
            receiving_agent_id: None,
            workspace_id: workspace_a.clone(),
            provider_id: provider.clone(),
            channel_id: channel_id.clone(),
            provider_chat_id: "chat-a".to_owned(),
            target_provider_message_id: "provider-message-3".to_owned(),
            sender_kind: MessageAuthorType::Agent,
            sender_id: agent_id.to_string(),
            sender_name: "Same visible name".to_owned(),
            reaction: "thinking".to_owned(),
            action: ReactionAction::Add,
            origin: ReactionOrigin::AutoReaction,
            provider_event_id: "auto-reaction-event-1".to_owned(),
            dispatch: vec![dispatch(
                &agent_id,
                DispatchReason::Reaction {
                    reaction: "thinking".to_owned(),
                },
                3_101,
            )],
            occurred_at: 3_101,
            now: 3_101,
        })
        .unwrap();
    assert!(auto.inbox_item_ids.is_empty());
    assert_eq!(
        storage.inbox_items(&workspace_a, &agent_id).unwrap().len(),
        inbox_before_auto
    );

    assert!(
        storage
            .set_workspace_active(&workspace_a, false, 3_200)
            .unwrap()
    );
    assert!(
        !storage
            .set_workspace_active(&workspace_a, false, 3_201)
            .unwrap()
    );
    assert!(!storage.workspace(&workspace_a).unwrap().active);
    assert!(
        storage
            .set_workspace_active(&workspace_a, true, 3_202)
            .unwrap()
    );
    assert!(storage.workspace(&workspace_a).unwrap().active);

    let failed = storage
        .create_execution(&workspace_a, &agent_id, "late message", 4_000)
        .unwrap()
        .unwrap();
    let failed_late = failed
        .snapshot
        .chats
        .iter()
        .flat_map(|chat| chat.trigger_messages.iter())
        .find(|message| message.message_id == late.message_id)
        .unwrap();
    assert_eq!(
        failed_late.reply_to_provider_message_id.as_deref(),
        Some("provider-message-1")
    );
    assert_eq!(
        failed
            .snapshot
            .chats
            .iter()
            .map(|chat| chat.trigger_reasons.len())
            .sum::<usize>(),
        1
    );
    storage
        .mark_execution_running(failed.execution_id, 4_001)
        .unwrap();
    storage
        .mark_delivery_delivered(failed.delivery_id, 4_002)
        .unwrap();
    storage
        .begin_execution_send(
            failed.execution_id,
            SendRequest {
                workspace_id: workspace_a.clone(),
                channel_id: channel_id.clone(),
                provider_chat_id: "chat-a".to_owned(),
                request_key: "response-unknown".to_owned(),
                sender_agent_id: agent_id.clone(),
                content: "uncertain".to_owned(),
                now: 4_003,
            },
        )
        .unwrap();
    storage
        .finish_send("response-unknown", SendOutcome::Unknown, 4_004)
        .unwrap();
    assert_eq!(
        storage
            .commit_execution(
                failed.execution_id,
                "not committed",
                Some("response-unknown"),
                4_005
            )
            .unwrap(),
        CommitResult::NotReady
    );
    assert!(
        storage
            .fail_execution_retry_after(
                failed.execution_id,
                "input_preparation_failed",
                4_100,
                4_006,
            )
            .unwrap()
    );
    assert!(
        storage
            .create_execution(&workspace_a, &agent_id, "deferred", 4_099)
            .unwrap()
            .is_none(),
        "a stable preparation failure must not hot-loop"
    );
    let deferred = storage
        .create_execution(&workspace_a, &agent_id, "deferred", 4_100)
        .unwrap()
        .expect("Inbox becomes retryable after the delay");
    assert!(
        storage
            .cancel_execution(deferred.execution_id, 4_101)
            .unwrap()
    );
    assert!(
        !storage
            .fail_execution(failed.execution_id, "again", 4_003)
            .unwrap()
    );
    assert_eq!(
        storage.execution(failed.execution_id).unwrap().state,
        ExecutionState::Failed
    );
    assert_eq!(
        storage
            .cursor(&workspace_a, &agent_id, late.chat_id)
            .unwrap(),
        Some(late.message_id)
    );

    // A real disk-backed Attempt survives a short disconnect, while a new
    // Computer instance makes the old Attempt terminal without consuming its Inbox.
    let lost = storage
        .create_execution(&workspace_a, &agent_id, "attempt-recovery", 4_200)
        .unwrap()
        .unwrap();
    storage
        .offer_execution_attempt(lost.execution_id, 1, "local", "instance-a", 4_300, 4_201)
        .unwrap();
    assert!(
        storage
            .accept_execution_attempt(lost.execution_id, 1, true, 4_202)
            .unwrap()
    );
    assert!(
        storage
            .mark_attempt_delivery(lost.execution_id, 1, lost.delivery_id, true, 4_203)
            .unwrap()
    );
    let started = storage
        .record_execution_event(RecordExecutionEvent {
            execution_id: lost.execution_id,
            attempt: 1,
            computer_id: "local".into(),
            computer_instance_id: "instance-a".into(),
            sequence: 1,
            kind: "started".into(),
            payload: br#"{"kind":"started"}"#.to_vec(),
            occurred_at: 4_204,
            now: 4_204,
        })
        .unwrap();
    assert_eq!(started.max_saved_sequence, 1);
    storage
        .mark_execution_recovering(lost.execution_id, 4_205)
        .unwrap();
    let continued = storage
        .reconcile_computer_heartbeat(
            "local",
            "instance-a",
            &[(lost.execution_id, 1)],
            4_206,
            4_400,
        )
        .unwrap();
    assert_eq!(continued.continued, vec![(lost.execution_id, 1)]);
    let replaced = storage
        .reconcile_computer_heartbeat("local", "instance-b", &[], 4_207, 4_500)
        .unwrap();
    assert_eq!(replaced.lost, vec![(lost.execution_id, 1)]);
    let late_completed = storage
        .record_execution_event(RecordExecutionEvent {
            execution_id: lost.execution_id,
            attempt: 1,
            computer_id: "local".into(),
            computer_instance_id: "instance-a".into(),
            sequence: 2,
            kind: "completed".into(),
            payload: br#"{"kind":"completed","final_output":"late"}"#.to_vec(),
            occurred_at: 4_208,
            now: 4_208,
        })
        .unwrap();
    assert_eq!(
        late_completed.disposition,
        ExecutionEventDisposition::StaleAttempt
    );
    assert_eq!(
        storage.execution(lost.execution_id).unwrap().state,
        ExecutionState::Failed
    );
    assert_eq!(
        storage
            .cursor(&workspace_a, &agent_id, late.chat_id)
            .unwrap(),
        Some(late.message_id)
    );

    let resumed = storage
        .create_execution(&workspace_a, &agent_id, "next-execution", 4_210)
        .unwrap()
        .unwrap();
    storage
        .offer_execution_attempt(resumed.execution_id, 1, "local", "instance-b", 4_500, 4_211)
        .unwrap();
    storage
        .accept_execution_attempt(resumed.execution_id, 1, true, 4_212)
        .unwrap();
    storage
        .mark_attempt_delivery(resumed.execution_id, 1, resumed.delivery_id, true, 4_213)
        .unwrap();
    for (sequence, kind, payload) in [
        (1, "started", br#"{"kind":"started"}"#.as_slice()),
        (
            2,
            "completed",
            br#"{"kind":"completed","final_output":"durable"}"#.as_slice(),
        ),
    ] {
        storage
            .record_execution_event(RecordExecutionEvent {
                execution_id: resumed.execution_id,
                attempt: 1,
                computer_id: "local".into(),
                computer_instance_id: "instance-b".into(),
                sequence,
                kind: kind.into(),
                payload: payload.to_vec(),
                occurred_at: 4_213 + sequence as i64,
                now: 4_213 + sequence as i64,
            })
            .unwrap();
    }
    assert_eq!(
        storage.execution(resumed.execution_id).unwrap().state,
        ExecutionState::Succeeded
    );
    let replayed_commit = storage
        .record_execution_event(RecordExecutionEvent {
            execution_id: resumed.execution_id,
            attempt: 1,
            computer_id: "local".into(),
            computer_instance_id: "instance-b".into(),
            sequence: 2,
            kind: "completed".into(),
            payload: br#"{"kind":"completed","final_output":"durable"}"#.to_vec(),
            occurred_at: 4_216,
            now: 4_216,
        })
        .unwrap();
    assert_eq!(
        replayed_commit.disposition,
        ExecutionEventDisposition::AlreadyCommitted
    );
    assert_eq!(
        storage
            .execution_events(resumed.execution_id, 1)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        storage
            .inbox_items(&workspace_a, &agent_id)
            .unwrap()
            .iter()
            .find(|item| item.source_id == late.message_id)
            .unwrap()
            .state,
        InboxState::Consumed
    );

    storage
        .ingest_event(event(
            &workspace_a,
            &provider,
            &channel_id,
            "chat-a",
            "provider-message-cancel",
            "cancel this execution",
            4_217,
            vec![dispatch(&agent_id, DispatchReason::DirectMessage, 4_217)],
        ))
        .unwrap();

    let leased = storage
        .create_execution(&workspace_a, &agent_id, "lease", 4_218)
        .unwrap()
        .unwrap();
    storage
        .offer_execution_attempt(leased.execution_id, 1, "local", "instance-b", 4_219, 4_218)
        .unwrap();
    storage
        .accept_execution_attempt(leased.execution_id, 1, true, 4_218)
        .unwrap();
    assert_eq!(
        storage.expire_execution_attempt_leases(4_219).unwrap(),
        vec![(leased.execution_id, 1)]
    );
    assert_eq!(
        storage.execution(leased.execution_id).unwrap().state,
        ExecutionState::Failed
    );

    let cancelled = storage
        .create_execution(&workspace_a, &agent_id, "cancel", 4_220)
        .unwrap()
        .unwrap();
    storage
        .mark_execution_running(cancelled.execution_id, 4_101)
        .unwrap();
    storage
        .mark_delivery_delivered(cancelled.delivery_id, 4_102)
        .unwrap();
    storage
        .begin_execution_send(
            cancelled.execution_id,
            SendRequest {
                workspace_id: workspace_a.clone(),
                channel_id: channel_id.clone(),
                provider_chat_id: "chat-a".to_owned(),
                request_key: "response-failed".to_owned(),
                sender_agent_id: agent_id.clone(),
                content: "failed".to_owned(),
                now: 4_103,
            },
        )
        .unwrap();
    storage
        .finish_send(
            "response-failed",
            SendOutcome::Failed {
                code: "provider_rejected".to_owned(),
            },
            4_104,
        )
        .unwrap();
    assert_eq!(
        storage
            .commit_execution(
                cancelled.execution_id,
                "not committed",
                Some("response-failed"),
                4_105
            )
            .unwrap(),
        CommitResult::NotReady
    );
    assert!(
        storage
            .cancel_execution(cancelled.execution_id, 4_106)
            .unwrap()
    );
    assert_eq!(
        storage.execution(cancelled.execution_id).unwrap().state,
        ExecutionState::Cancelled
    );
    assert_eq!(
        storage
            .cursor(&workspace_a, &agent_id, late.chat_id)
            .unwrap(),
        Some(response.message_id)
    );

    let tool = ToolInvocationRequest {
        execution_id: execution.execution_id,
        attempt: 1,
        tool_call_id: "tool-1".to_owned(),
        tool_name: "send_message".to_owned(),
        arguments: json!({ "text": "hello" }),
        now: 5_000,
    };
    let begun = storage.begin_tool_invocation(tool.clone()).unwrap();
    assert!(begun.created);
    storage
        .finish_tool_invocation(&tool, json!({ "ok": true }), 5_001)
        .unwrap();
    let replay = storage.begin_tool_invocation(tool.clone()).unwrap();
    assert!(!replay.created);
    assert_eq!(replay.result, Some(json!({ "ok": true })));
    let mut conflicting_tool = tool;
    conflicting_tool.arguments = json!({ "text": "different" });
    assert!(
        storage.begin_tool_invocation(conflicting_tool).is_err(),
        "one Tool call identity must not execute different business input"
    );

    let failed_tool = ToolInvocationRequest {
        execution_id: execution.execution_id,
        attempt: 1,
        tool_call_id: "tool-failed".to_owned(),
        tool_name: "send_message".to_owned(),
        arguments: json!({ "text": "cannot deliver" }),
        now: 5_010,
    };
    assert!(
        storage
            .begin_tool_invocation(failed_tool.clone())
            .unwrap()
            .created
    );
    assert!(
        storage
            .fail_tool_invocation(&failed_tool, "provider_unavailable", 5_011)
            .unwrap()
    );
    let failed_replay = storage.begin_tool_invocation(failed_tool).unwrap();
    assert_eq!(failed_replay.state, "failed");
    assert_eq!(
        failed_replay.result,
        Some(json!({ "error": "provider_unavailable" }))
    );

    let send = SendRequest {
        workspace_id: workspace_a.clone(),
        channel_id: channel_id.clone(),
        provider_chat_id: "chat-a".to_owned(),
        request_key: "send-1".to_owned(),
        sender_agent_id: agent_id.clone(),
        content: "outbound".to_owned(),
        now: 5_100,
    };
    let send_first = storage.begin_send(send.clone()).unwrap();
    let send_replay = storage.begin_send(send.clone()).unwrap();
    assert!(send_first.created);
    assert!(!send_replay.created);
    assert_eq!(send_first.message_id, send_replay.message_id);
    let mut conflicting_send = send;
    conflicting_send.content = "different outbound content".to_owned();
    assert!(
        storage.begin_send(conflicting_send).is_err(),
        "one send identity must not represent different business content"
    );

    let inbox_before_echo = storage.inbox_items(&workspace_a, &agent_id).unwrap().len();
    let mut echo_event = event(
        &workspace_a,
        &provider,
        &channel_id,
        "chat-a",
        "provider-echo-1",
        "agent echo",
        5_150,
        Vec::new(),
    );
    echo_event.message.sender_kind = MessageAuthorType::Agent;
    echo_event.message.sender_id = agent_id.to_string();
    echo_event.message.sender_name = "Same visible name".to_owned();
    let echo = storage.ingest_event(echo_event).unwrap();
    assert!(echo.inbox_item_ids.is_empty());
    assert_eq!(
        storage.inbox_items(&workspace_a, &agent_id).unwrap().len(),
        inbox_before_echo
    );

    storage
        .begin_send(SendRequest {
            workspace_id: workspace_a.clone(),
            channel_id: channel_id.clone(),
            provider_chat_id: "chat-a".to_owned(),
            request_key: "send-conflict".to_owned(),
            sender_agent_id: agent_id.clone(),
            content: "conflict".to_owned(),
            now: 5_160,
        })
        .unwrap();
    let conflict = storage.finish_send(
        "send-conflict",
        SendOutcome::Sent {
            provider_message_id: "provider-response-1".to_owned(),
        },
        5_161,
    );
    assert!(conflict.is_err());
    assert_eq!(
        storage.send_attempt("send-conflict").unwrap().state,
        "sending"
    );
    storage
        .finish_send("send-conflict", SendOutcome::Unknown, 5_162)
        .unwrap();

    let recovering = storage
        .create_execution(&workspace_a, &agent_id, "restart", 5_200)
        .unwrap()
        .unwrap();
    storage
        .mark_execution_running(recovering.execution_id, 5_201)
        .unwrap();
    drop(storage);

    let reopened = Storage::open(config.clone()).unwrap();
    assert_eq!(reopened.list_workspace_records().unwrap().len(), 2);
    assert_eq!(
        reopened.workspace_record(&workspace_b).unwrap().provider_id,
        provider_b
    );
    assert_eq!(
        reopened.list_agent_records(&workspace_a).unwrap(),
        vec![recreated.clone(), agent_a.clone(), replacement.clone()]
    );
    assert_eq!(
        reopened.agent_record(&workspace_b, &agent_id).unwrap(),
        agent_b
    );
    assert_eq!(
        reopened
            .read_secret(&workspace_a, "bot_token")
            .unwrap()
            .unwrap()
            .expose_secret(),
        "named-secret-alpha"
    );
    assert_eq!(
        reopened
            .resource_metadata(&workspace_a, ResourceId::new(7).unwrap())
            .unwrap(),
        Some(resource)
    );
    let recovery = reopened.recover_startup(5_202).unwrap();
    assert_eq!(recovery.transitioned_executions, 1);
    assert_eq!(recovery.transitioned_sends, 1);
    assert!(
        recovery
            .executions
            .iter()
            .any(|view| view.execution.execution_id == recovering.execution_id)
    );
    assert!(
        recovery
            .sends
            .iter()
            .any(|view| view.attempt.request_key == "send-1")
    );
    let recovery_again = reopened.recover_startup(5_203).unwrap();
    assert_eq!(recovery_again.transitioned_executions, 0);
    assert_eq!(recovery_again.transitioned_sends, 0);
    assert_eq!(recovery_again.executions, recovery.executions);
    assert_eq!(recovery_again.sends, recovery.sends);
    assert_eq!(
        reopened.send_attempt("send-1").unwrap().state.as_str(),
        "unknown"
    );
    assert_eq!(
        reopened.execution(recovering.execution_id).unwrap().state,
        ExecutionState::Recovering
    );
    let reconciled = reopened
        .finish_send(
            "send-conflict",
            SendOutcome::Sent {
                provider_message_id: "provider-response-reconciled".to_owned(),
            },
            5_204,
        )
        .unwrap();
    assert!(reconciled.changed);
    assert_eq!(reconciled.attempt.state, "sent");
    assert!(
        !reopened
            .finish_send(
                "send-conflict",
                SendOutcome::Sent {
                    provider_message_id: "provider-response-reconciled".to_owned(),
                },
                5_205,
            )
            .unwrap()
            .changed
    );
    assert!(
        reopened
            .recoverable_executions(Some(&workspace_a))
            .unwrap()
            .iter()
            .any(|view| view.execution.execution_id == recovering.execution_id)
    );
    assert!(
        reopened
            .recoverable_sends(Some(&workspace_a))
            .unwrap()
            .iter()
            .any(|view| view.attempt.request_key == "send-1")
    );

    let before = reopened.message_count(&workspace_a).unwrap();
    let fault = rusqlite::Connection::open(&db_path).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_message AFTER INSERT ON messages BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
    let injected = reopened.receive_message(message(
        &workspace_a,
        &channel_id,
        "chat-rollback",
        "provider-message-rollback",
        "must roll back",
        6_000,
    ));
    fault.execute_batch("DROP TRIGGER fail_message").unwrap();
    drop(fault);
    assert!(injected.is_err());
    assert_eq!(reopened.message_count(&workspace_a).unwrap(), before);
    assert!(
        reopened
            .find_message(
                &workspace_a,
                &channel_id,
                "chat-rollback",
                "provider-message-rollback"
            )
            .unwrap()
            .is_none()
    );

    let before_fk = reopened.message_count(&workspace_a).unwrap();
    let mut bad = event(
        &workspace_a,
        &provider,
        &channel_id,
        "chat-fk",
        "provider-message-fk",
        "must roll back on FK",
        6_100,
        vec![dispatch(
            &AgentId::new("missing").unwrap(),
            DispatchReason::Follow,
            6_100,
        )],
    );
    bad.chat_name = "FK rollback".to_owned();
    assert!(reopened.ingest_event(bad).is_err());
    assert_eq!(reopened.message_count(&workspace_a).unwrap(), before_fk);
    assert!(
        reopened
            .find_message(&workspace_a, &channel_id, "chat-fk", "provider-message-fk")
            .unwrap()
            .is_none()
    );

    let missing_workspace = WorkspaceId::new("missing_workspace").unwrap();
    let missing_agent = agent_record(&missing_workspace, &agent_id, "missing", 6_200);
    assert!(reopened.upsert_agent_record(missing_agent).is_err());
    assert_eq!(
        reopened.list_agent_records(&workspace_a).unwrap(),
        vec![recreated, agent_a, replacement]
    );
    assert!(
        reopened
            .put_secret(
                &missing_workspace,
                "token",
                SecretValue::new("must-not-persist"),
                6_201
            )
            .is_err()
    );
    let mut invalid_workspace = reopened.workspace_record(&workspace_a).unwrap();
    invalid_workspace.name = "  ".to_owned();
    assert!(reopened.upsert_workspace_record(invalid_workspace).is_err());
    assert_eq!(
        reopened.workspace_record(&workspace_a).unwrap().name,
        "Alpha full"
    );

    let second = Storage::open(config).unwrap();
    assert_eq!(second.message_count(&workspace_a).unwrap(), before);
    second.close().unwrap();
    let summary = json!({
        "schema_version": reopened.schema_version().unwrap(),
        "journal_mode": reopened.sqlite_settings().unwrap().journal_mode,
        "foreign_keys": reopened.sqlite_settings().unwrap().foreign_keys,
        "workspace_count": reopened.list_workspaces().unwrap().len(),
        "message_count": reopened.message_count(&workspace_a).unwrap(),
        "execution_state_after_restart": reopened.execution(recovering.execution_id).unwrap().state.as_str(),
        "workspace_record_count": reopened.list_workspace_records().unwrap().len(),
        "agent_record_roundtrip": true,
        "provider_namespace_isolated": true,
        "recover_startup_idempotent": true,
        "wal_checkpoint_on_close": true,
        "secret_values_in_artifact": false,
        "temp_home_cleaned": true
    });
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    reopened.close().unwrap();
    assert!(
        !wal_path.exists() || fs::metadata(&wal_path).unwrap().len() == 0,
        "close must checkpoint the WAL"
    );
    verify_v14_hard_delete_migration(temp_home.path())?;
    drop(temp_home);
    assert!(
        !db_path.exists(),
        "temporary database must be removed after handles close"
    );
    let _ = artifact_dir;
    Ok(summary)
}

fn verify_v14_hard_delete_migration(root: &Path) -> Result<(), String> {
    let database = root.join("schema-14.sqlite3");
    let workspace_id = WorkspaceId::new("legacy_workspace").unwrap();
    let agent_id = AgentId::new("legacy_agent").unwrap();
    let storage =
        Storage::open(DatabaseConfig::new(&database)).map_err(|error| error.to_string())?;
    storage
        .create_workspace(WorkspaceSpec {
            id: workspace_id.clone(),
            name: "Legacy workspace".into(),
            provider_id: ProviderId::new("telegram").unwrap(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .map_err(|error| error.to_string())?;
    storage
        .commit_agent_record(
            agent_record(&workspace_id, &agent_id, "legacy_sender", 2),
            &json!({"bot_token":"legacy-secret"}),
        )
        .map_err(|error| error.to_string())?;
    storage.close().map_err(|error| error.to_string())?;

    let legacy = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    legacy
        .execute_batch(
            "ALTER TABLE execution_attempts DROP COLUMN failure_json;
             ALTER TABLE agents ADD COLUMN deleted_at INTEGER;
             DROP TABLE agent_file_deletions;
             CREATE TABLE agent_file_deletions(
                 workspace_id TEXT NOT NULL,
                 agent_id TEXT NOT NULL,
                 computer_id TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 PRIMARY KEY(workspace_id,agent_id),
                 FOREIGN KEY(workspace_id,agent_id)
                     REFERENCES agents(workspace_id,agent_id) ON DELETE RESTRICT
             );
             UPDATE agents SET deleted_at=3
                 WHERE workspace_id='legacy_workspace' AND agent_id='legacy_agent';
             INSERT INTO agent_file_deletions(workspace_id,agent_id,computer_id,created_at)
                 VALUES('legacy_workspace','legacy_agent','local',3);
             PRAGMA user_version=14;",
        )
        .map_err(|error| error.to_string())?;
    drop(legacy);

    let migrated =
        Storage::open(DatabaseConfig::new(&database)).map_err(|error| error.to_string())?;
    assert_eq!(migrated.schema_version().unwrap(), 17);
    assert!(migrated.agent_record(&workspace_id, &agent_id).is_err());
    assert_eq!(
        migrated.pending_agent_file_deletions("local").unwrap(),
        vec![(workspace_id, agent_id)]
    );
    migrated.close().map_err(|error| error.to_string())?;

    let verified = rusqlite::Connection::open(&database).map_err(|error| error.to_string())?;
    let deleted_at_columns: i64 = verified
        .query_row(
            "SELECT count(*) FROM pragma_table_info('agents') WHERE name='deleted_at'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let cleanup_foreign_keys: i64 = verified
        .query_row(
            "SELECT count(*) FROM pragma_foreign_key_list('agent_file_deletions')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    assert_eq!(deleted_at_columns, 0);
    assert_eq!(cleanup_foreign_keys, 0);
    Ok(())
}

fn message(
    workspace_id: &WorkspaceId,
    channel_id: &ChannelId,
    provider_chat_id: &str,
    provider_message_id: &str,
    content: &str,
    now: i64,
) -> IncomingMessage {
    IncomingMessage {
        workspace_id: workspace_id.clone(),
        channel_id: channel_id.clone(),
        channel_name: "Messages".to_owned(),
        provider_chat_id: provider_chat_id.to_owned(),
        chat_name: provider_chat_id.to_owned(),
        provider_message_id: provider_message_id.to_owned(),
        sender_id: "human-1".to_owned(),
        sender_name: "Human".to_owned(),
        content: content.to_owned(),
        occurred_at: now,
        now,
    }
}

fn dispatch(
    agent_id: &AgentId,
    trigger_reason: DispatchReason,
    available_at: i64,
) -> InboxDispatch {
    InboxDispatch {
        agent_id: agent_id.clone(),
        available_at,
        trigger_reason,
    }
}

fn agent_record(
    workspace_id: &WorkspaceId,
    agent_id: &AgentId,
    provider_sender: &str,
    now: i64,
) -> AgentRecord {
    AgentRecord {
        workspace_id: workspace_id.clone(),
        agent_id: agent_id.clone(),
        name: "Same visible name".to_owned(),
        role: "assistant".to_owned(),
        bio: "Helps the team".to_owned(),
        avatar: Some("avatar.png".to_owned()),
        soul: "Be useful".to_owned(),
        computer_id: "local".to_owned(),
        harness_id: "codex".to_owned(),
        model: "gpt-5.6-luna".to_owned(),
        thinking: "low".to_owned(),
        auto_reaction: Some("eyes".to_owned()),
        cli_path: Some("/usr/bin/true".to_owned()),
        env: BTreeMap::from([("LANG".to_owned(), "en_US.UTF-8".to_owned())]),
        provider_agent_config: json!({"sender_id":provider_sender}),
        provider_state: json!({}),
        session_config: json!({"resume":true}),
        attention: BTreeMap::from([
            (
                ChannelId::new("group_chat").unwrap(),
                AttentionRecord::Mention,
            ),
            (
                ChannelId::new("scheduled").unwrap(),
                AttentionRecord::Schedule {
                    interval_minutes: 5,
                },
            ),
        ]),
        created_at: 1_001,
        updated_at: now,
    }
}

#[allow(clippy::too_many_arguments)]
fn event(
    workspace_id: &WorkspaceId,
    provider_id: &ProviderId,
    channel_id: &ChannelId,
    provider_chat_id: &str,
    provider_message_id: &str,
    content: &str,
    now: i64,
    dispatch: Vec<InboxDispatch>,
) -> IngestEvent {
    IngestEvent {
        receiving_agent_id: None,
        workspace_id: workspace_id.clone(),
        provider_id: provider_id.clone(),
        channel_id: channel_id.clone(),
        channel_name: "Messages".to_owned(),
        provider_chat_id: provider_chat_id.to_owned(),
        chat_name: provider_chat_id.to_owned(),
        provider_event_id: Some(format!("event-{provider_message_id}")),
        provider_message_id: provider_message_id.to_owned(),
        message: MessageInput {
            sender_kind: MessageAuthorType::Human,
            sender_id: "human-1".to_owned(),
            sender_name: "Human".to_owned(),
            content: content.to_owned(),
            mentions: Vec::new(),
            reply_to_provider_message_id: None,
            resources: Vec::new(),
            occurred_at: now,
        },
        dispatch,
        now,
    }
}

fn write_result(
    directory: &Path,
    run_id: &str,
    status: &str,
    started_at: u128,
    duration_ms: u128,
    details: serde_json::Value,
) {
    let payload = json!({
        "run_id": run_id,
        "module": "storage",
        "classification": "hermetic",
        "started_at_unix_ms": started_at,
        "duration_ms": duration_ms,
        "prerequisite_check": { "status": "passed", "requirements": ["writable temporary filesystem"] },
        "scenarios": [{ "name": "storage_runtime_records_events_commit_and_restart_recovery", "status": status }],
        "created_resource_ids": [],
        "cleanup": { "status": if status == "passed" { "passed" } else { "best_effort" } },
        "final_status": status,
        "status": status,
        "details": details
    });
    let encoded = serde_json::to_vec_pretty(&payload).expect("encode result artifact");
    fs::write(directory.join("result.json"), encoded).expect("write result artifact");
}

#[test]
fn legacy_direct_chat_keeps_its_history_and_ids_after_receiver_migration() {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("legacy.sqlite3");
    let workspace = WorkspaceId::new("team").unwrap();
    let provider = ProviderId::new("telegram").unwrap();
    let channel = ChannelId::new("private_chat").unwrap();
    let bruce = AgentId::new("bruce").unwrap();
    let clark = AgentId::new("clark").unwrap();
    let diana = AgentId::new("diana").unwrap();
    let storage = Storage::open(DatabaseConfig::new(&database)).unwrap();
    storage
        .create_workspace(WorkspaceSpec {
            id: workspace.clone(),
            name: "Team".into(),
            provider_id: provider.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .unwrap();
    for agent in [&bruce, &clark, &diana] {
        storage
            .commit_agent_record(
                agent_record(&workspace, agent, agent.as_str(), 2),
                &json!({}),
            )
            .unwrap();
    }
    let dispatch = |agent: &AgentId| InboxDispatch {
        agent_id: agent.clone(),
        available_at: 3,
        trigger_reason: DispatchReason::DirectMessage,
    };
    let legacy = storage
        .ingest_event(event(
            &workspace,
            &provider,
            &channel,
            "alice",
            "1",
            "Bruce history",
            3,
            vec![dispatch(&bruce)],
        ))
        .unwrap();
    let ambiguous = storage
        .ingest_event(event(
            &workspace,
            &provider,
            &channel,
            "bob",
            "1",
            "Ambiguous history",
            3,
            vec![dispatch(&bruce), dispatch(&clark)],
        ))
        .unwrap();
    let contaminated = storage
        .ingest_event(event(
            &workspace,
            &provider,
            &channel,
            "feishu-p2p",
            "1",
            "Shared DM history",
            3,
            vec![dispatch(&bruce), dispatch(&clark), dispatch(&diana)],
        ))
        .unwrap();
    storage.close().unwrap();
    // A v15 database is the persistent outer boundary. Recreate its actual chat schema.
    let fixture = rusqlite::Connection::open(&database).unwrap();
    fixture.execute_batch("PRAGMA foreign_keys=OFF;
        CREATE TABLE chats_v15(chat_id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,channel_id TEXT NOT NULL,provider_chat_id TEXT NOT NULL,name TEXT,first_message_id INTEGER,last_message_id INTEGER,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,FOREIGN KEY(workspace_id,channel_id)REFERENCES channels(workspace_id,channel_id),UNIQUE(workspace_id,channel_id,provider_chat_id));
        INSERT INTO chats_v15 SELECT chat_id,workspace_id,channel_id,provider_chat_id,name,first_message_id,last_message_id,created_at,updated_at FROM chats;
        DROP TABLE chats; ALTER TABLE chats_v15 RENAME TO chats;
        ALTER TABLE execution_attempts DROP COLUMN failure_json;
        PRAGMA user_version=15;").unwrap();
    // Production shape: all three Agents dispatched and tried to reply; only
    // Bruce's bot could send (5 sent, 8 failed, 13 failed).
    for (agent, count, state) in [
        (&bruce, 5, "sent"),
        (&clark, 8, "failed"),
        (&diana, 13, "failed"),
    ] {
        for attempt in 0..count {
            fixture.execute(
                "INSERT INTO messages(chat_id,direction,sender_kind,sender_id,content_markdown,send_status,occurred_at,created_at,updated_at) VALUES(?1,'outbound','agent',?2,'Reply',?3,3,3,3)",
                rusqlite::params![contaminated.chat_id, agent.as_str(), state],
            ).unwrap();
            fixture.execute(
                "INSERT INTO message_send_attempts(message_id,request_key,state,started_at,finished_at) VALUES(?1,?2,?3,3,3)",
                rusqlite::params![fixture.last_insert_rowid(), format!("legacy-{}-{attempt}", agent.as_str()), state],
            ).unwrap();
        }
    }
    drop(fixture);
    verify_migration_sample(database.as_os_str());
    let storage = Storage::open(DatabaseConfig::new(&database)).unwrap();
    assert_eq!(storage.schema_version().unwrap(), 17);
    assert!(storage.sqlite_settings().unwrap().foreign_keys);
    for agent in [&clark, &diana] {
        assert!(
            storage
                .create_execution(&workspace, agent, "queued DM", 4)
                .unwrap()
                .is_none()
        );
    }

    let mut incoming = event(
        &workspace,
        &provider,
        &channel,
        "alice",
        "2",
        "Bruce followup",
        4,
        vec![dispatch(&bruce)],
    );
    incoming.receiving_agent_id = Some(bruce.clone());
    assert_eq!(
        storage.ingest_event(incoming.clone()).unwrap().chat_id,
        legacy.chat_id
    );
    incoming.receiving_agent_id = Some(clark.clone());
    incoming.message.content = "Clark conversation".into();
    incoming.dispatch = vec![dispatch(&clark)];
    let separate = storage.ingest_event(incoming).unwrap();
    assert_ne!(separate.chat_id, legacy.chat_id);
    assert_eq!(
        storage
            .find_message_for_receiver(&workspace, &channel, "alice", "2", Some(&clark))
            .unwrap(),
        Some(separate.message_id)
    );
    assert_ne!(
        storage
            .find_message_for_receiver(&workspace, &channel, "alice", "2", Some(&bruce))
            .unwrap(),
        Some(separate.message_id)
    );
    assert!(
        storage
            .inbox_items(&workspace, &bruce)
            .unwrap()
            .iter()
            .filter(|item| item.chat_id == ambiguous.chat_id)
            .all(|item| item.state == InboxState::Consumed)
    );
    let mut new_bob = event(
        &workspace,
        &provider,
        &channel,
        "bob",
        "2",
        "Only Bruce",
        4,
        vec![dispatch(&bruce)],
    );
    new_bob.receiving_agent_id = Some(bruce.clone());
    assert_ne!(
        storage.ingest_event(new_bob).unwrap().chat_id,
        ambiguous.chat_id
    );
    let mut followup = event(
        &workspace,
        &provider,
        &channel,
        "feishu-p2p",
        "2",
        "Bruce followup",
        4,
        vec![dispatch(&bruce)],
    );
    followup.receiving_agent_id = Some(bruce.clone());
    assert_eq!(
        storage.ingest_event(followup.clone()).unwrap().chat_id,
        contaminated.chat_id
    );
    for agent in [&clark, &diana] {
        let items = storage.inbox_items(&workspace, agent).unwrap();
        let wrong_items: Vec<_> = items
            .iter()
            .filter(|item| item.chat_id == contaminated.chat_id)
            .collect();
        assert_eq!(wrong_items.len(), 1);
        assert!(
            wrong_items
                .iter()
                .all(|item| item.state == InboxState::Consumed)
        );
        followup.receiving_agent_id = Some(agent.clone());
        followup.dispatch = vec![dispatch(agent)];
        assert_ne!(
            storage.ingest_event(followup.clone()).unwrap().chat_id,
            contaminated.chat_id
        );
    }
    assert!(
        storage
            .inbox_items(&workspace, &bruce)
            .unwrap()
            .iter()
            .filter(|item| item.chat_id == contaminated.chat_id)
            .all(|item| item.state == InboxState::Pending)
    );
    let resumed = storage
        .create_execution(&workspace, &bruce, "queued DM", 4)
        .unwrap()
        .unwrap();
    assert!(resumed.snapshot.chats.iter().any(|chat| {
        chat.chat.chat_id == contaminated.chat_id
            && chat
                .trigger_messages
                .iter()
                .any(|message| message.message_id == contaminated.message_id)
    }));
    storage.cancel_execution(resumed.execution_id, 4).unwrap();
    assert_eq!(storage.message_count(&workspace).unwrap(), 35);
    storage.close().unwrap();
    let verified = rusqlite::Connection::open(database).unwrap();
    assert_eq!(
        verified
            .query_row(
                "SELECT content_markdown FROM messages WHERE message_id=?1",
                [contaminated.message_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Shared DM history"
    );
    assert_eq!(
        verified
            .query_row(
                "SELECT content_markdown FROM messages WHERE message_id=?1",
                [legacy.message_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "Bruce history"
    );
    assert_eq!(
        verified
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    verify_optional_migration_sample();
}

// Exercise Storage::open against a consistent copy, never the supplied database.
fn verify_optional_migration_sample() {
    let Some(sample) = std::env::var_os("LANKIDE_MIGRATION_SAMPLE") else {
        return;
    };
    verify_migration_sample(&sample);
}

fn verify_migration_sample(sample: &std::ffi::OsStr) {
    let home = tempfile::tempdir().unwrap();
    let database = home.path().join("sample.sqlite3");
    let source =
        rusqlite::Connection::open_with_flags(sample, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    source
        .execute("VACUUM INTO ?1", [database.to_str().unwrap()])
        .unwrap();
    drop(source);
    let before = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        before
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        15,
        "migration sample must use the legacy v15 schema"
    );
    let messages = |connection: &rusqlite::Connection| {
        let mut query = connection
            .prepare("SELECT * FROM messages ORDER BY message_id")
            .unwrap();
        let columns = query.column_count();
        query
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get::<_, rusqlite::types::Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let history = messages(&before);
    let chats = before
        .prepare("SELECT chat_id,channel_id FROM chats ORDER BY chat_id")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let mut expected = Vec::new();
    for (chat_id, channel) in &chats {
        let mut owner = String::new();
        if channel == "private_chat" {
            let senders = before.prepare("SELECT DISTINCT m.sender_id FROM message_send_attempts a JOIN messages m USING(message_id) WHERE m.chat_id=?1 AND m.sender_kind='agent' AND m.direction='outbound' AND a.state='sent' AND m.sender_id IS NOT NULL").unwrap()
                .query_map([chat_id], |r| r.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            if senders.len() == 1 {
                owner = senders[0].clone();
            } else if senders.is_empty() {
                let inbox_agents = before
                    .prepare("SELECT DISTINCT agent_id FROM agent_inbox_items WHERE chat_id=?1")
                    .unwrap()
                    .query_map([chat_id], |r| r.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                if inbox_agents.len() == 1 {
                    owner = inbox_agents[0].clone();
                }
            }
        }
        expected.push((*chat_id, owner));
    }
    let inbox = before.prepare("SELECT inbox_item_id,chat_id,agent_id,state FROM agent_inbox_items ORDER BY inbox_item_id").unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    drop(before);
    let storage = Storage::open(DatabaseConfig::new(&database)).unwrap();
    assert_eq!(storage.schema_version().unwrap(), 17);
    storage.close().unwrap();
    let after = rusqlite::Connection::open(database).unwrap();
    assert_eq!(
        history,
        messages(&after),
        "migration must preserve every history row"
    );
    assert_eq!(
        after
            .query_row("SELECT count(*) FROM chats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        chats.len() as i64
    );
    for (chat_id, owner) in &expected {
        assert_eq!(
            &after
                .query_row(
                    "SELECT receiving_agent_id FROM chats WHERE chat_id=?1",
                    [chat_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            owner
        );
    }
    for (id, chat_id, agent, state) in inbox {
        let direct = chats
            .iter()
            .any(|(id, channel)| *id == chat_id && channel == "private_chat");
        let owner = &expected.iter().find(|(id, _)| *id == chat_id).unwrap().1;
        let expected_state = if direct && state == "pending" && agent != *owner {
            "consumed"
        } else {
            &state
        };
        assert_eq!(
            after
                .query_row(
                    "SELECT state FROM agent_inbox_items WHERE inbox_item_id=?1",
                    [id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            expected_state
        );
    }
    assert_eq!(
        after
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}
