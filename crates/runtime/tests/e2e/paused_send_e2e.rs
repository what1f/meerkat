use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paused_workspace_keeps_successful_agent_send() {
    let temp = tempfile::tempdir().unwrap();
    tokio::time::timeout(
        Duration::from_secs(10),
        paused_send(&temp.path().join("paused-send.db")),
    )
    .await
    .unwrap()
    .unwrap();
}

async fn paused_send(db: &Path) -> Result<(), String> {
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
    let send_request = meerkat_storage::SendRequest {
        workspace_id: workspace.clone(),
        channel_id: channel.clone(),
        provider_chat_id: "pause-chat".into(),
        request_key: "paused-agent-send".into(),
        sender_agent_id: agent.clone(),
        content: "Work completed.".into(),
        now: 5,
    };
    let attempt = runtime
        .storage()
        .begin_send(send_request.clone())
        .map_err(err)?;
    state.push_send(SendMode::Sent("paused-provider-message".into()));
    let result = runtime
        .send_message(
            &workspace,
            agent.clone(),
            meerkat_provider::SendMessageSpec {
                channel_id: channel,
                chat_id: "pause-chat".into(),
                message_id: MessageId::new(attempt.message_id).map_err(err)?,
                idempotency_key: send_request.request_key.clone(),
                content: send_request.content,
                reply_to: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
                force: false,
                provider_agent_config: json!({"tenant":"fixture"}),
            },
        )
        .await
        .map_err(err)?;
    ensure(
        result.status == meerkat_provider::SendMessageStatus::Sent,
        "paused send returns success",
    )?;
    let stored = runtime
        .storage()
        .send_attempt(&send_request.request_key)
        .map_err(err)?;
    ensure(
        stored.state == "sent"
            && stored.provider_message_id.as_deref() == Some("paused-provider-message"),
        "confirmed send remains sent while the execution drains",
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
        "execution finishes after the paused send",
    )?;
    runtime
        .finish_paused_workspace(&workspace)
        .await
        .map_err(err)?;
    Ok(())
}
