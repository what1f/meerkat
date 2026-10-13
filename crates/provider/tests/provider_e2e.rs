use lankide_core::{
    AgentId, AttentionLevel, ChannelAbility, ChannelId, MessageId, ProviderId, WorkspaceId,
};
use lankide_protocol::ResourceRef;
use lankide_provider::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;

const SECRET: &str = "must-never-appear";

#[derive(Default)]
struct State {
    calls: Mutex<Vec<String>>,
    sent: Mutex<Vec<(MessageId, Option<MessageId>, usize, usize)>>,
    sink: Mutex<Option<EventSink>>,
    first_emitted: AtomicBool,
    second_emitted: AtomicBool,
    first_notify: Notify,
    second_notify: Notify,
    secret_denied: AtomicBool,
}
impl State {
    fn call(&self, value: &str) {
        self.calls.lock().unwrap().push(value.into());
    }
}

struct DeniedSecrets;
impl SecretReader for DeniedSecrets {
    fn read<'a>(&'a self, _: &'a str) -> ProviderFuture<'a, SecretValue> {
        Box::pin(async { Err(ProviderError::SecretAccessDenied) })
    }
}
#[derive(Default)]
struct MemoryStorage(Mutex<BTreeMap<String, Vec<u8>>>);
impl ProviderStorage for MemoryStorage {
    fn get<'a>(&'a self, key: &'a str) -> ProviderFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(key).cloned()) })
    }
    fn put<'a>(&'a self, key: &'a str, value: Vec<u8>) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.0.lock().unwrap().insert(key.into(), value);
            Ok(())
        })
    }
}
struct NoopLogger;
impl ProviderLogger for NoopLogger {
    fn log(&self, _: LogLevel, _: &str) {}
}
struct Progress;
impl OperationProgress for Progress {
    fn report<'a>(&'a self, _: OperationUpdate) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
struct Saver;
impl ResourceSaver for Saver {
    fn save<'a>(&'a self, value: ResourceUpload) -> ProviderFuture<'a, ResourceRef> {
        Box::pin(async move {
            Ok(resource(
                70,
                &value.file_name,
                &value.media_type,
                value.bytes.len() as u64,
            ))
        })
    }
}

struct EchoHandler;
impl InvocationHandler for EchoHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move { Ok(json!({"echo": input["value"]})) })
    }
}
struct ScopedToolHandler {
    agent_id: String,
    permission: String,
}
impl InvocationHandler for ScopedToolHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let agent_id = self.agent_id.clone();
        let permission = self.permission.clone();
        Box::pin(async move {
            Ok(json!({
                "task": input["task"],
                "state": "done",
                "agent_id": agent_id,
                "permission": permission,
            }))
        })
    }
}
struct ScopedCliHandler {
    agent_id: String,
}
impl InvocationHandler for ScopedCliHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let agent_id = self.agent_id.clone();
        Box::pin(async move { Ok(json!({"argv": input, "agent_id": agent_id})) })
    }
}
struct InvalidOutputHandler;
impl InvocationHandler for InvalidOutputHandler {
    fn invoke<'a>(&'a self, _: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move { Ok(json!({"wrong": true})) })
    }
}
struct SlowHandler;
impl InvocationHandler for SlowHandler {
    fn invoke<'a>(&'a self, _: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(json!({}))
        })
    }
}

struct ConformanceProvider {
    state: Arc<State>,
}
impl Provider for ConformanceProvider {
    fn info(&self) -> ProviderInfo {
        self.state.call("info");
        ProviderInfo {
            id: ProviderId::new("conformance").unwrap(),
            name: "Conformance".into(),
            description: "External peer".into(),
            version: "1.0.0".into(),
            icon: "peer.svg".into(),
        }
    }
    fn channels<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        self.state.call("channels");
        Box::pin(async move {
            assert_eq!(context.workspace_id().as_str(), "ws_provider");
            Ok(vec![ChannelDefinition::new(
                ChannelId::new("group_chat").unwrap(),
                "群聊",
                ChannelDefaultAttention::Mention,
                [AttentionLevel::Follow, AttentionLevel::Mention],
                [
                    ChannelAbility::Reply,
                    ChannelAbility::Attachment,
                    ChannelAbility::Reaction,
                ],
            )?])
        })
    }
    fn create_workspace<'a>(&'a self, context: WorkspaceContext) -> ProviderFuture<'a, ()> {
        self.state.call("create_workspace");
        let state = self.state.clone();
        Box::pin(async move {
            let error = context.provider_secrets().read("denied").await.unwrap_err();
            assert_eq!(error, ProviderError::SecretAccessDenied);
            assert!(!format!("{context:?} {error:?}").contains(SECRET));
            state.secret_denied.store(true, Ordering::Release);
            Ok(())
        })
    }
    fn update_workspace<'a>(&'a self, context: WorkspaceContext) -> ProviderFuture<'a, ()> {
        self.state.call("update_workspace");
        Box::pin(async move {
            assert!(context.old_provider_config.is_some());
            Ok(())
        })
    }
    fn create_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        self.state.call("create_agent");
        Box::pin(async move {
            assert!(context.old_provider_agent_config.is_none());
            Ok(())
        })
    }
    fn update_agent<'a>(&'a self, context: AgentContext) -> ProviderFuture<'a, ()> {
        self.state.call("update_agent");
        Box::pin(async move {
            assert!(context.old_provider_agent_config.is_some());
            Ok(())
        })
    }
    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        self.state.call("init");
        let state = self.state.clone();
        Box::pin(async move {
            let object = schema(json!({"type":"object"}));
            context
                .register_interface(InterfaceDefinition {
                    id: "peer.echo".into(),
                    input_schema: schema(json!({"type":"object","required":["value"]})),
                    output_schema: schema(json!({"type":"object","required":["echo"]})),
                    handler: Arc::new(EchoHandler),
                })
                .await?;
            assert_eq!(
                context
                    .register_interface(InterfaceDefinition {
                        id: "peer.echo".into(),
                        input_schema: schema(json!({"type":"object"})),
                        output_schema: schema(json!({"type":"object"})),
                        handler: Arc::new(EchoHandler),
                    })
                    .await
                    .unwrap_err(),
                ProviderError::DuplicateRegistration {
                    id: "peer.echo".into()
                }
            );
            context
                .register_interface(InterfaceDefinition {
                    id: "peer.invalid_output".into(),
                    input_schema: schema(json!({"type":"object"})),
                    output_schema: schema(json!({"type":"object","required":["result"]})),
                    handler: Arc::new(InvalidOutputHandler),
                })
                .await?;
            context
                .register_interface(InterfaceDefinition {
                    id: "name.decided.at.runtime".into(),
                    input_schema: object.clone(),
                    output_schema: object,
                    handler: Arc::new(SlowHandler),
                })
                .await?;
            let saved = context
                .save_resource(ResourceUpload {
                    file_name: "inbound.png".into(),
                    media_type: "image/png".into(),
                    bytes: vec![1, 2, 3],
                })
                .await?;
            *state.sink.lock().unwrap() = Some(context.event_sink().clone());
            let sink = context.event_sink().clone();
            tokio::spawn(async move {
                sink.emit(message_event(saved)).await.unwrap();
                state.first_emitted.store(true, Ordering::Release);
                state.first_notify.notify_waiters();
                sink.emit(reaction_event(ReactionAction::Add))
                    .await
                    .unwrap();
                state.second_emitted.store(true, Ordering::Release);
                state.second_notify.notify_waiters();
                sink.emit(reaction_event(ReactionAction::Remove))
                    .await
                    .unwrap();
                sink.emit(custom_event()).await.unwrap();
            });
            Ok(())
        })
    }
    fn system_prompt<'a>(&'a self, context: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        self.state.call("system_prompt");
        Box::pin(async move {
            assert_eq!(context.chats.len(), 2);
            assert_eq!(context.chats[0].provider_id.as_str(), "fixture");
            assert_eq!(context.chats[0].channel_id.as_str(), "group_chat");
            assert_eq!(context.chats[1].chat_id, "room-2");
            Ok(format!("Use peer task tools as {}.", context.agent_id))
        })
    }
    fn tools<'a>(
        &'a self,
        context: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<ToolDefinition>> {
        self.state.call("tools");
        Box::pin(async move {
            let permission = context
                .provider_agent_config
                .get("permission")
                .and_then(Value::as_str)
                .unwrap_or("complete")
                .to_owned();
            let id = format!("peer_task_{permission}");
            let definition = ToolDefinition {
                id: id.clone(),
                description: "Complete task".into(),
                input_schema: schema(json!({"type":"object","required":["task"]})),
                output_schema: schema(json!({"type":"object","required":["state"]})),
                handler: Arc::new(ScopedToolHandler {
                    agent_id: context.agent_id.to_string(),
                    permission: permission.clone(),
                }),
            };
            if permission == "duplicate" {
                return Ok(vec![definition.clone(), definition]);
            }
            Ok(vec![definition])
        })
    }
    fn cli_commands<'a>(
        &'a self,
        context: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<CliCommandDefinition>> {
        self.state.call("cli_commands");
        Box::pin(async move {
            Ok(vec![CliCommandDefinition {
                id: "peer.inspect".into(),
                description: "Inspect".into(),
                handler: Arc::new(ScopedCliHandler {
                    agent_id: context.agent_id.to_string(),
                }),
            }])
        })
    }
    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult> {
        self.state.call("send_message");
        self.state.sent.lock().unwrap().push((
            context.message_id,
            context.reply_to,
            context.mentions.len(),
            context.attachments.len(),
        ));
        Box::pin(async move {
            if context.force {
                Ok(SendMessageResult::unknown())
            } else {
                SendMessageResult::sent(format!("remote-{}", context.message_id))
            }
        })
    }
    fn recall_message<'a>(&'a self, context: RecallMessageContext) -> ProviderFuture<'a, ()> {
        self.state.call("recall_message");
        Box::pin(async move {
            assert_eq!(context.provider_message_id, "remote-501");
            Ok(())
        })
    }
    fn auto_reaction<'a>(&'a self, context: AutoReactionContext) -> ProviderFuture<'a, ()> {
        self.state.call("auto_reaction");
        Box::pin(async move {
            assert_eq!(context.trigger_messages.len(), 1);
            Ok(())
        })
    }
    fn shutdown<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        self.state.call("shutdown");
        Box::pin(async move {
            assert!(!context.shutdown_signal().is_shutdown());
            Ok(())
        })
    }
}

struct DefaultProvider;
impl Provider for DefaultProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ProviderId::new("defaults").unwrap(),
            name: "Defaults".into(),
            description: "".into(),
            version: "1".into(),
            icon: "".into(),
        }
    }
    fn channels<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn init<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn send_message<'a>(&'a self, _: SendMessageContext) -> ProviderFuture<'a, SendMessageResult> {
        Box::pin(async { Ok(SendMessageResult::unknown()) })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn third_party_provider_conforms_through_public_host() {
    let run_id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let artifact = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/provider-e2e")
        .join(&run_id)
        .join("artifact/result.json");
    let result = tokio::time::timeout(Duration::from_secs(10), scenario()).await;
    fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    let (status, detail) = match &result {
        Ok(Ok(())) => ("passed", "ok"),
        Ok(Err(e)) => ("failed", e.as_str()),
        Err(_) => ("timed_out", "hard_timeout"),
    };
    fs::write(
        &artifact,
        serde_json::to_vec_pretty(
            &json!({"run_id":run_id,"module":"provider","status":status,"detail":detail}),
        )
        .unwrap(),
    )
    .unwrap();
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("{error}"),
        Err(_) => panic!("provider E2E hard timeout; artifact={}", artifact.display()),
    }
}

async fn scenario() -> Result<(), String> {
    let state = Arc::new(State::default());
    let host = make_host(
        ConformanceProvider {
            state: state.clone(),
        },
        Duration::from_millis(40),
    );
    assert_eq!(host.info().description, "External peer");
    assert!(
        host.channels().await.unwrap()[0]
            .abilities
            .contains(&ChannelAbility::Attachment)
    );
    assert_eq!(
        host.list_interfaces().await.unwrap_err(),
        ProviderError::NotFound {
            entity: "workspace"
        }
    );
    let workspace = WorkspaceSpec {
        name: "Provider WS".into(),
        cover: None,
        rules: "Be useful".into(),
        provider_config: json!({"tasks":true,"secret":SECRET}),
    };
    host.create_workspace(workspace.clone()).await.unwrap();
    host.update_workspace(json!({"tasks":false}), workspace)
        .await
        .unwrap();
    let agent = agent_spec();
    host.create_agent(agent_id(), agent.clone()).await.unwrap();
    host.update_agent(agent_id(), json!({"old":true}), agent)
        .await
        .unwrap();
    host.init().await.unwrap();
    let first_catalog = host.list_interfaces().await.unwrap();
    let second_catalog = host.list_interfaces().await.unwrap();
    assert_eq!(first_catalog, second_catalog);
    assert_eq!(
        first_catalog
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "name.decided.at.runtime",
            "peer.echo",
            "peer.invalid_output"
        ]
    );
    assert!(
        first_catalog
            .iter()
            .all(|entry| entry.description.is_none())
    );
    assert_eq!(
        first_catalog[1].input_schema,
        json!({"type":"object","required":["value"]})
    );
    assert_eq!(
        first_catalog[1].output_schema,
        json!({"type":"object","required":["echo"]})
    );
    let serialized_catalog = serde_json::to_string(&first_catalog).unwrap();
    assert!(!serialized_catalog.contains("handler"));
    assert!(!serialized_catalog.contains(SECRET));
    let first_delivery = host.next_event().await.unwrap();
    assert!(
        !state.first_emitted.load(Ordering::Acquire),
        "queueing is not durable acceptance"
    );
    let (first, acknowledgement) = first_delivery.into_parts();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            wait_flag(&state.first_emitted, &state.first_notify)
        )
        .await
        .is_err()
    );
    acknowledgement.accept();
    wait_flag(&state.first_emitted, &state.first_notify).await;
    let ProviderEventKind::Message(message) = first.kind else {
        panic!("message first")
    };
    assert_eq!(
        message.mentioned_agent_ids,
        vec![agent_id(), AgentId::new("reviewer").unwrap()]
    );
    assert_eq!(
        message.reply_to_provider_message_id.as_deref(),
        Some("remote-parent")
    );
    assert_eq!(message.attachments[0].file_name, "inbound.png");
    let (add, acknowledgement) = host.next_event().await.unwrap().into_parts();
    acknowledgement.accept();
    wait_flag(&state.second_emitted, &state.second_notify).await;
    let (remove, acknowledgement) = host.next_event().await.unwrap().into_parts();
    acknowledgement.accept();
    let (custom, acknowledgement) = host.next_event().await.unwrap().into_parts();
    acknowledgement.accept();
    assert!(matches!(
        add.kind,
        ProviderEventKind::Reaction(ReactionEvent {
            action: ReactionAction::Add,
            ..
        })
    ));
    assert!(matches!(
        remove.kind,
        ProviderEventKind::Reaction(ReactionEvent {
            action: ReactionAction::Remove,
            ..
        })
    ));
    assert!(
        matches!(custom.kind, ProviderEventKind::Custom(CustomEvent { ref target_agent_ids, .. }) if target_agent_ids.len() == 2)
    );
    let (first_echo, second_echo) = tokio::join!(
        host.invoke_interface("peer.echo", json!({"value":"hello"})),
        host.invoke_interface("peer.echo", json!({"value":"world"}))
    );
    assert_eq!(first_echo.unwrap(), json!({"echo":"hello"}));
    assert_eq!(second_echo.unwrap(), json!({"echo":"world"}));
    assert_eq!(
        host.invoke_interface("name.decided.at.runtime", json!({}))
            .await
            .unwrap_err(),
        ProviderError::Timeout {
            operation: "custom_interface"
        }
    );
    assert!(matches!(
        host.invoke_interface("peer.echo", json!({}))
            .await
            .unwrap_err(),
        ProviderError::SchemaMismatch { .. }
    ));
    assert!(matches!(
        host.invoke_interface("peer.invalid_output", json!({}))
            .await
            .unwrap_err(),
        ProviderError::SchemaMismatch { location: "output" }
    ));
    assert_eq!(
        host.invoke_interface("peer.unknown", json!({}))
            .await
            .unwrap_err(),
        ProviderError::UnknownRegistration {
            id: "peer.unknown".into()
        }
    );
    let runtime = runtime_spec();
    let reviewer = AgentId::new("reviewer").unwrap();
    let reviewer_runtime = runtime_spec_with_permission("read");
    let (agent_prompt, reviewer_prompt) = tokio::join!(
        host.system_prompt(agent_id(), runtime.clone()),
        host.system_prompt(reviewer.clone(), reviewer_runtime.clone())
    );
    assert_eq!(agent_prompt.unwrap(), "Use peer task tools as agent_one.");
    assert_eq!(reviewer_prompt.unwrap(), "Use peer task tools as reviewer.");
    let (complete, read) = tokio::join!(
        host.invoke_tool_in_context(
            agent_id(),
            runtime.clone(),
            "peer_task_complete",
            json!({"task":"T1"})
        ),
        host.invoke_tool_in_context(
            reviewer.clone(),
            reviewer_runtime.clone(),
            "peer_task_read",
            json!({"task":"T2"})
        )
    );
    let complete = complete.unwrap();
    let read = read.unwrap();
    assert_eq!(complete["agent_id"], "agent_one");
    assert_eq!(complete["permission"], "complete");
    assert_eq!(read["agent_id"], "reviewer");
    assert_eq!(read["permission"], "read");
    assert_eq!(
        host.invoke_tool_in_context(
            reviewer.clone(),
            reviewer_runtime.clone(),
            "peer_task_complete",
            json!({"task":"T1"})
        )
        .await
        .unwrap_err(),
        ProviderError::UnknownRegistration {
            id: "peer_task_complete".into()
        }
    );
    assert!(matches!(
        host.tools(agent_id(), runtime_spec_with_permission("duplicate"))
            .await
            .unwrap_err(),
        ProviderError::DuplicateRegistration { .. }
    ));
    assert_eq!(
        host.tools(agent_id(), runtime_spec_with_permission("invalid.name"))
            .await
            .unwrap_err(),
        ProviderError::InvalidInput {
            field: "capability_id"
        }
    );
    let (agent_cli, reviewer_cli) = tokio::join!(
        host.invoke_cli_in_context(
            agent_id(),
            runtime.clone(),
            "peer.inspect",
            vec!["T1".into()]
        ),
        host.invoke_cli_in_context(
            reviewer,
            reviewer_runtime,
            "peer.inspect",
            vec!["T2".into()]
        )
    );
    assert_eq!(agent_cli.unwrap()["agent_id"], "agent_one");
    assert_eq!(reviewer_cli.unwrap()["agent_id"], "reviewer");
    let send = SendMessageSpec {
        channel_id: channel_id(),
        chat_id: "chat-7".into(),
        message_id: MessageId::new(501).unwrap(),
        idempotency_key: "provider-e2e-send-501".into(),
        content: "@Agent One[agent_one] see attachment".into(),
        reply_to: Some(MessageId::new(499).unwrap()),
        mentions: vec![agent_id()],
        attachments: vec![MessageAttachment {
            resource: resource(80, "notes.md", "text/markdown", 12),
            caption: None,
        }],
        force: false,
        provider_agent_config: json!({}),
    };
    assert_eq!(
        host.send_message(agent_id(), send.clone())
            .await
            .unwrap()
            .provider_message_id
            .as_deref(),
        Some("remote-501")
    );
    let mut unknown = send;
    unknown.force = true;
    unknown.message_id = MessageId::new(502).unwrap();
    assert_eq!(
        host.send_message(agent_id(), unknown).await.unwrap(),
        SendMessageResult::unknown()
    );
    host.recall_message(
        agent_id(),
        RecallMessageSpec {
            channel_id: channel_id(),
            chat_id: "chat-7".into(),
            message_id: MessageId::new(501).unwrap(),
            provider_message_id: "remote-501".into(),
            provider_agent_config: json!({}),
        },
    )
    .await
    .unwrap();
    host.auto_reaction(
        agent_id(),
        AutoReactionSpec {
            auto_reaction: "eyes".into(),
            channel_id: channel_id(),
            chat_id: "chat-7".into(),
            trigger_messages: vec![MessageId::new(501).unwrap()],
            provider_agent_config: json!({}),
        },
    )
    .await
    .unwrap();
    let defaults = make_host(DefaultProvider, Duration::from_millis(40));
    let default_workspace = WorkspaceSpec {
        name: "Defaults".into(),
        cover: None,
        rules: String::new(),
        provider_config: json!({}),
    };
    defaults
        .create_workspace(default_workspace.clone())
        .await
        .unwrap();
    defaults
        .update_workspace(json!({}), default_workspace)
        .await
        .unwrap();
    defaults
        .create_agent(agent_id(), agent_spec())
        .await
        .unwrap();
    defaults
        .update_agent(agent_id(), json!({}), agent_spec())
        .await
        .unwrap();
    defaults.init().await.unwrap();
    assert!(
        defaults
            .system_prompt(agent_id(), runtime_spec())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        defaults
            .tools(agent_id(), runtime_spec())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        defaults
            .cli_commands(agent_id(), runtime_spec())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        defaults
            .recall_message(
                agent_id(),
                RecallMessageSpec {
                    channel_id: channel_id(),
                    chat_id: "x".into(),
                    message_id: MessageId::new(1).unwrap(),
                    provider_message_id: "x".into(),
                    provider_agent_config: json!({})
                }
            )
            .await
            .unwrap_err(),
        ProviderError::Unsupported {
            operation: "recall_message"
        }
    );
    defaults
        .auto_reaction(
            agent_id(),
            AutoReactionSpec {
                auto_reaction: "eyes".into(),
                channel_id: channel_id(),
                chat_id: "x".into(),
                trigger_messages: vec![],
                provider_agent_config: json!({}),
            },
        )
        .await
        .unwrap();
    defaults.shutdown().await.unwrap();
    host.shutdown().await.unwrap();
    assert_eq!(
        host.list_interfaces().await.unwrap_err(),
        ProviderError::NotFound {
            entity: "workspace"
        }
    );
    let event = custom_event();
    assert_eq!(
        state
            .sink
            .lock()
            .unwrap()
            .clone()
            .unwrap()
            .emit(event)
            .await
            .unwrap_err(),
        ProviderError::EventSinkClosed
    );
    assert!(state.secret_denied.load(Ordering::Acquire));
    let calls = state.calls.lock().unwrap();
    for required in [
        "info",
        "channels",
        "create_workspace",
        "update_workspace",
        "create_agent",
        "update_agent",
        "init",
        "system_prompt",
        "tools",
        "cli_commands",
        "send_message",
        "recall_message",
        "auto_reaction",
        "shutdown",
    ] {
        assert!(
            calls.iter().any(|call| call == required),
            "missing {required}"
        );
    }
    Ok(())
}

fn make_host<P: Provider>(provider: P, timeout: Duration) -> ProviderHost<P> {
    ProviderHost::new(
        provider,
        WorkspaceId::new("ws_provider").unwrap(),
        json!({"tasks":true,"secret":SECRET}),
        HostServices {
            secret_reader: Arc::new(DeniedSecrets),
            secret_writer: Arc::new(DeniedSecretWriter),
            storage: Arc::new(MemoryStorage::default()),
            logger: Arc::new(NoopLogger),
            resource_saver: Arc::new(Saver),
            operation_progress: Arc::new(Progress),
            event_capacity: 1,
            invocation_timeout: timeout,
        },
    )
    .unwrap()
}
fn schema(value: Value) -> JsonSchema {
    JsonSchema::new(value).unwrap()
}
fn agent_id() -> AgentId {
    AgentId::new("agent_one").unwrap()
}
fn channel_id() -> ChannelId {
    ChannelId::new("group_chat").unwrap()
}
fn resource(id: i64, name: &str, media: &str, size: u64) -> ResourceRef {
    ResourceRef {
        resource_id: id,
        file_name: name.into(),
        media_type: media.into(),
        size_bytes: size,
        sha256: "a".repeat(64),
    }
}
fn sender() -> EventSender {
    EventSender {
        id: "human-1".into(),
        kind: EventSenderType::Human,
        name: "Alice".into(),
    }
}
fn message_event(attachment: ResourceRef) -> ProviderEvent {
    ProviderEvent {
        channel_id: channel_id(),
        provider_chat_id: "chat-7".into(),
        sender: sender(),
        provider_data: json!({"opaque":true}),
        kind: ProviderEventKind::Message(MessageEvent {
            provider_message_id: "remote-500".into(),
            content: "hello @Agent One[agent_one] @Reviewer[reviewer]".into(),
            reply_to_provider_message_id: Some("remote-parent".into()),
            mentioned_agent_ids: vec![agent_id(), AgentId::new("reviewer").unwrap()],
            attachments: vec![attachment],
        }),
    }
}
fn reaction_event(action: ReactionAction) -> ProviderEvent {
    ProviderEvent {
        channel_id: channel_id(),
        provider_chat_id: "chat-7".into(),
        sender: sender(),
        provider_data: json!({}),
        kind: ProviderEventKind::Reaction(ReactionEvent {
            target_provider_message_id: "remote-500".into(),
            sender: sender(),
            reaction: "eyes".into(),
            action,
        }),
    }
}
fn custom_event() -> ProviderEvent {
    ProviderEvent {
        channel_id: channel_id(),
        provider_chat_id: "task-feed".into(),
        sender: sender(),
        provider_data: json!({}),
        kind: ProviderEventKind::Custom(CustomEvent {
            reason: "task_assigned".into(),
            target_agent_ids: vec![agent_id(), AgentId::new("reviewer").unwrap()],
            subject_id: "T1".into(),
            subject_name: "Release".into(),
            data: json!({"provider_only":true}),
        }),
    }
}
fn agent_spec() -> AgentSpec {
    AgentSpec {
        name: "Agent One".into(),
        role: "Engineer".into(),
        bio: "Build".into(),
        avatar: None,
        soul: "Precise".into(),
        computer_id: "local".into(),
        harness_id: "codex".into(),
        model: "model".into(),
        thinking_level: "low".into(),
        auto_reaction: Some("eyes".into()),
        cli_path: None,
        environment_variables: vec![EnvironmentVariable {
            name: "TOKEN".into(),
            value: SECRET.into(),
        }],
        provider_agent_config: json!({"identity":"bot"}),
    }
}
fn runtime_spec() -> AgentRuntimeSpec {
    runtime_spec_with_permission("complete")
}
fn runtime_spec_with_permission(permission: &str) -> AgentRuntimeSpec {
    AgentRuntimeSpec {
        agent_config: json!({}),
        provider_agent_config: json!({"tasks":true,"permission":permission}),
        computer_id: "local".into(),
        harness_id: "codex".into(),
        model: "model".into(),
        thinking_level: "low".into(),
        trigger_reason: "mention".into(),
        chats: vec![
            AgentRuntimeChat {
                provider_id: ProviderId::new("fixture").unwrap(),
                channel_id: channel_id(),
                channel_name: "群聊".into(),
                chat_id: "room-1".into(),
                chat_name: "Room One".into(),
            },
            AgentRuntimeChat {
                provider_id: ProviderId::new("fixture").unwrap(),
                channel_id: ChannelId::new("direct_message").unwrap(),
                channel_name: "私聊".into(),
                chat_id: "room-2".into(),
                chat_name: "Room Two".into(),
            },
        ],
        unread_snapshot: json!({"chats":[{"chat_id":"room-1"},{"chat_id":"room-2"}]}),
    }
}
async fn wait_flag(flag: &AtomicBool, notify: &Notify) {
    while !flag.load(Ordering::Acquire) {
        notify.notified().await;
    }
}
