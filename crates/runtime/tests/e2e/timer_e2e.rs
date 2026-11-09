use super::*;
use meerkat_protocol::TimerRequest;

// An Agent sets itself a timer from a conversation; the owner can remove it and undo that; when
// it comes due the Agent wakes in that conversation with the note it left itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_timer_wakes_it_in_the_conversation() {
    let (temp, app, _state) = profile_application().await;
    let storage = Storage::open(DatabaseConfig::new(temp.path().join("runtime.db"))).unwrap();
    let workspace = WorkspaceId::new("profile_workspace").unwrap();
    let agent = AgentId::new("profile_agent").unwrap();
    let channel = ChannelId::new("chat").unwrap();
    storage
        .ingest_event(IngestEvent {
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
                sender_kind: meerkat_core::MessageAuthorType::Human,
                sender_id: "owner".into(),
                sender_name: "Owner".into(),
                content: "Remind me to check the dashboard".into(),
                mentions: vec![],
                reply_to_provider_message_id: None,
                resources: vec![],
                occurred_at: 3,
            },
            dispatch: vec![InboxDispatch {
                agent_id: agent.clone(),
                available_at: 3,
                trigger_reason: DispatchReason::DirectMessage,
            }],
            now: 3,
        })
        .unwrap();
    let token = "timer-token";
    let execution = open_attempt(&storage, &workspace, &agent, token, 4);
    let context = AgentRunContext {
        execution: Some(ExecutionRef {
            execution_id: execution,
            attempt: 1,
        }),
        execution_token: token.into(),
    };
    let invoke = |name: &str, arguments: Value| CapabilityInvokeRequest {
        context: Some(context.clone()),
        capability_name: name.into(),
        invocation_id: format!("{name}-{arguments}"),
        arguments_json: arguments.to_string(),
    };
    let here = json!({"channel_id": "chat", "chat_id": "owner-dm"});
    let with = |extra: Value| {
        let mut arguments = here.clone();
        arguments
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        arguments
    };

    // Times that cannot work are explained to the Agent, not stored.
    for arguments in [
        with(json!({"prompt": "Too late", "at": "2001-01-01T09:00:00Z"})),
        with(json!({"prompt": "Too often", "every_minutes": 1})),
        with(json!({"prompt": "Never"})),
        json!({"channel_id": "chat", "chat_id": "someone-else", "prompt": "Elsewhere", "in_minutes": 5}),
    ] {
        let refused = app
            .invoke_capability(invoke("timer_set", arguments))
            .await
            .unwrap();
        assert!(
            !refused.success && refused.error.is_some_and(|error| !error.message.is_empty()),
            "an impossible timer is refused with a reason"
        );
    }
    assert!(
        app.list_timers(workspace_request())
            .await
            .unwrap()
            .is_empty()
    );

    let note = "Check the dashboard and tell the owner";
    let due = now() + 2_000;
    let at = jiff::Timestamp::from_millisecond(due).unwrap().to_string();
    let set = app
        .invoke_capability(invoke("timer_set", with(json!({"prompt": note, "at": at}))))
        .await
        .unwrap();
    assert!(set.success, "timer set: {set:?}");
    let listed = app
        .invoke_capability(invoke("timer_list", json!({})))
        .await
        .unwrap();
    assert!(listed.success && listed.structured_content_json.contains(note));

    // The owner sees it on the team wall, removes it, and takes that back.
    let timers = app.list_timers(workspace_request()).await.unwrap();
    assert_eq!(timers.len(), 1);
    let timer = &timers[0];
    assert_eq!(timer.prompt, note);
    assert_eq!(timer.chat_name, "Owner");
    assert_eq!(timer.agent, profile_agent_ref());
    let request = TimerRequest {
        request_id: "remove".into(),
        workspace: workspace_request().workspace,
        timer_id: timer.timer_id,
    };
    app.cancel_timer(request.clone()).await.unwrap();
    assert!(
        app.list_timers(workspace_request())
            .await
            .unwrap()
            .is_empty()
    );
    app.restore_timer(request).await.unwrap();
    assert_eq!(app.list_timers(workspace_request()).await.unwrap().len(), 1);

    // The run ends; the timer then wakes the Agent once in the same conversation.
    assert!(storage.cancel_execution(execution, 5).unwrap());
    tokio::time::timeout(Duration::from_secs(10), async {
        while !app
            .list_timers(workspace_request())
            .await
            .unwrap()
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("a one-time timer fires and leaves the wall");
    let woken = storage
        .create_execution(&workspace, &agent, "timer", now())
        .unwrap()
        .expect("the timer queued work for the Agent");
    let chats = &woken.snapshot.chats;
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].chat.provider_chat_id, "owner-dm");
    assert!(chats[0].trigger_reasons.iter().any(|reason| matches!(
        reason,
        DispatchReason::Timer { timer_id, prompt } if *timer_id == timer.timer_id && prompt == note
    )));
    app.shutdown().await.unwrap();
    storage.close().unwrap();
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn workspace_request() -> meerkat_protocol::GetWorkspaceRequest {
    meerkat_protocol::GetWorkspaceRequest {
        request_id: "timers".into(),
        workspace: Some(WorkspaceRef {
            workspace_id: "profile_workspace".into(),
        }),
    }
}

// An accepted attempt on the profile Machine with the timer tools granted.
fn open_attempt(
    storage: &Storage,
    workspace: &WorkspaceId,
    agent: &AgentId,
    token: &str,
    now: i64,
) -> i64 {
    let execution = storage
        .create_execution_with_auth(
            workspace,
            agent,
            "owner DM",
            now,
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
            capabilities: ["timer_set", "timer_list", "timer_cancel"]
                .map(|name| AttemptCapability {
                    name: name.into(),
                    display_name: name.into(),
                    description: name.into(),
                    kind: meerkat_protocol::CapabilityKind::Tool as i32,
                    input_schema_json: "{}".into(),
                    output_schema_json: "{}".into(),
                })
                .to_vec(),
        })
        .unwrap();
    storage
        .mark_execution_running(execution.execution_id, now)
        .unwrap();
    storage
        .offer_execution_attempt(
            execution.execution_id,
            1,
            "profile-machine",
            "timer-test",
            i64::MAX,
            now,
        )
        .unwrap();
    storage
        .accept_execution_attempt(execution.execution_id, 1, true, now)
        .unwrap();
    execution.execution_id
}
