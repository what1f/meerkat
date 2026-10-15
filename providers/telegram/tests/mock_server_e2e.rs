#[path = "support/host.rs"]
mod host;
#[path = "support/http.rs"]
mod http;
#[path = "support/server.rs"]
mod server;

use host::{Services, agent_spec};
use lankide_core::{AgentId, ChannelId, MessageId};
use lankide_provider::*;
use lankide_provider_telegram::TelegramProvider;
use serde_json::json;
use server::{FILE, TelegramServer};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_message_mentioning_a_bot_reaches_its_agent() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "bot_token", "200:local");
    services
        .db
        .put_secret(
            &services.workspace,
            "manager_bot_token",
            lankide_storage::SecretValue::new("100:manager"),
            1,
        )
        .unwrap();
    let config =
        json!({"api_endpoint":format!("{}/", server.http.endpoint), "poll_timeout_seconds":1});
    let host = ProviderHost::new(
        TelegramProvider::new(),
        services.workspace.clone(),
        config.clone(),
        services.host(),
    )
    .unwrap();
    host.create_workspace(WorkspaceSpec {
        name: "Team".into(),
        cover: None,
        rules: String::new(),
        provider_config: config,
    })
    .await
    .unwrap();
    let host = std::sync::Arc::new(host);
    let scenario_host = host.clone();
    let server = std::sync::Arc::new(server);
    let scenario_server = server.clone();
    let outcome = tokio::spawn(async move {
        let host = scenario_host;
        let server = scenario_server;
        host.init().await.unwrap();
        host.create_agent(bot.clone(), agent_spec(json!({"bot_username":"beta_bot"})))
            .await
            .unwrap();
        server.deliver("200:local", json!({"update_id":1,"message":{"message_id":8,"date":1,
            "chat":{"id":-1001,"type":"supergroup","title":"Team"},"from":{"id":300,"is_bot":false,"first_name":"Alice"},
            "text":"@beta_bot hello","entities":[{"type":"mention","offset":0,"length":9}]}}));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event())
            .await
            .unwrap()
            .unwrap()
            .into_parts();
        assert_eq!(event.provider_chat_id, "-1001");
        assert_eq!(event.receiving_agent_id, Some(bot.clone()));
        let ProviderEventKind::Message(message) = event.kind else {
            panic!("expected message")
        };
        assert_eq!(message.mentioned_agent_ids, vec![bot.clone()]);
        assert_eq!(message.content, "@beta_bot hello");
        ack.accept();
    }).await;
    host.shutdown().await.unwrap();
    if let Err(error) = outcome {
        if error.is_panic() {
            std::panic::resume_unwind(error.into_panic());
        }
        panic!("scenario task failed: {error}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_with_an_attachment_reaches_the_chat_with_the_file() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "bot_token", "200:local");
    services
        .db
        .put_secret(
            &services.workspace,
            "manager_bot_token",
            lankide_storage::SecretValue::new("100:manager"),
            1,
        )
        .unwrap();
    let config =
        json!({"api_endpoint":format!("{}/", server.http.endpoint), "poll_timeout_seconds":1});
    let host = ProviderHost::new(
        TelegramProvider::new(),
        services.workspace.clone(),
        config.clone(),
        services.host(),
    )
    .unwrap();
    host.create_workspace(WorkspaceSpec {
        name: "Team".into(),
        cover: None,
        rules: String::new(),
        provider_config: config,
    })
    .await
    .unwrap();
    let host = std::sync::Arc::new(host);
    let scenario_host = host.clone();
    let server = std::sync::Arc::new(server);
    let scenario_server = server.clone();
    let outcome = tokio::spawn(async move {
        let host = scenario_host;
        let server = scenario_server;
        host.init().await.unwrap();
        host.create_agent(bot.clone(), agent_spec(json!({"bot_username":"beta_bot"})))
            .await
            .unwrap();
        server.deliver("200:local", json!({"update_id":2,"message":{"message_id":9,"date":2,
            "chat":{"id":-1001,"type":"supergroup","title":"Team"},"from":{"id":300,"is_bot":false,"first_name":"Alice"},
            "document":{"file_id":"inbound-file","file_unique_id":"unique","file_name":"inbound.bin","mime_type":"application/octet-stream","file_size":FILE.len()}}}));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event())
            .await
            .unwrap()
            .unwrap()
            .into_parts();
        let ProviderEventKind::Message(message) = event.kind else {
            panic!("expected file")
        };
        assert_eq!(message.attachments.len(), 1);
        let file = message.attachments[0].clone();
        assert_eq!(services.resource(&file).await.unwrap().bytes, FILE);
        let parent = services.remember("-1001", &message.provider_message_id);
        ack.accept();
        let sent = host
            .send_message(
                bot.clone(),
                SendMessageSpec {
                    channel_id: ChannelId::new("group_chat").unwrap(),
                    chat_id: "-1001".into(),
                    message_id: MessageId::new(100).unwrap(),
                    idempotency_key: "reply-with-file".into(),
                    content: "Here is your file".into(),
                    reply_to: Some(parent),
                    mentions: vec![],
                    attachments: vec![MessageAttachment {
                        resource: file,
                        caption: None,
                    }],
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .unwrap();
        assert_eq!(sent.status, SendMessageStatus::Sent);
        host.auto_reaction(
            bot.clone(),
            AutoReactionSpec {
                auto_reaction: "👍".into(),
                channel_id: ChannelId::new("group_chat").unwrap(),
                chat_id: "-1001".into(),
                trigger_messages: vec![parent],
                provider_agent_config: json!({}),
            },
        )
        .await
        .unwrap();
        host.recall_message(
            bot,
            RecallMessageSpec {
                channel_id: ChannelId::new("group_chat").unwrap(),
                chat_id: "-1001".into(),
                message_id: MessageId::new(100).unwrap(),
                provider_message_id: sent.provider_message_id.unwrap(),
                provider_agent_config: json!({}),
            },
        )
        .await
        .unwrap();

        let requests = server.requests();
        let text = requests
            .iter()
            .find(|r| r.path.ends_with("/sendMessage"))
            .unwrap()
            .json();
        assert_eq!(text["chat_id"], -1001);
        assert_eq!(text["reply_parameters"]["message_id"], 9);
        let upload = requests
            .iter()
            .find(|r| r.path.ends_with("/sendDocument"))
            .unwrap();
        assert_eq!(upload.path, "/bot200:local/sendDocument");
        assert_eq!(upload.part("chat_id"), b"-1001");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&upload.part("reply_parameters")).unwrap()["message_id"],
            9
        );
        assert_eq!(upload.part("document"), FILE);
        let reaction = requests
            .iter()
            .find(|r| r.path.ends_with("/setMessageReaction"))
            .unwrap()
            .json();
        assert_eq!(reaction["chat_id"], -1001);
        assert_eq!(reaction["message_id"], 9);
        assert_eq!(reaction["reaction"][0]["emoji"], "👍");
        let recall = requests
            .iter()
            .find(|r| r.path.ends_with("/deleteMessage"))
            .unwrap()
            .json();
        assert_eq!(recall["chat_id"], -1001);
        assert_eq!(recall["message_id"], 101);
    }).await;
    host.shutdown().await.unwrap();
    if let Err(error) = outcome {
        if error.is_panic() {
            std::panic::resume_unwind(error.into_panic());
        }
        panic!("scenario task failed: {error}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2_1_direct_messages_and_reactions_identify_the_receiving_agent() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    let alpha = AgentId::new("alpha").unwrap();
    let beta = AgentId::new("beta").unwrap();
    services.agent(&alpha, "bot_token", "201:local");
    services.agent(&beta, "bot_token", "200:local");
    services
        .db
        .put_secret(
            &services.workspace,
            "manager_bot_token",
            lankide_storage::SecretValue::new("100:manager"),
            1,
        )
        .unwrap();
    let config =
        json!({"api_endpoint":format!("{}/",server.http.endpoint),"poll_timeout_seconds":1});
    let host = ProviderHost::new(
        TelegramProvider::new(),
        services.workspace.clone(),
        config.clone(),
        services.host(),
    )
    .unwrap();
    host.create_workspace(WorkspaceSpec {
        name: "Team".into(),
        cover: None,
        rules: String::new(),
        provider_config: config,
    })
    .await
    .unwrap();
    let host = std::sync::Arc::new(host);
    let scenario_host = host.clone();
    let server = std::sync::Arc::new(server);
    let scenario_server = server.clone();
    let outcome = tokio::spawn(async move {
        let host = scenario_host;
        let server = scenario_server;
        host.init().await.unwrap();
        for (agent, username) in [(&alpha, "alpha_bot"), (&beta, "beta_bot")] {
            host.create_agent(agent.clone(), agent_spec(json!({"bot_username":username}))).await.unwrap();
        }
        let mut chats = vec![];
        // The same person and native message/update IDs are valid in both bot accounts.
        for (agent, token, text) in [(&beta, "200:local", "only beta"), (&alpha, "201:local", "only alpha")] {
            server.deliver(token, json!({"update_id":1,"message":{"message_id":8,"date":1,
                "chat":{"id":300,"type":"private"},"from":{"id":300,"is_bot":false,"first_name":"Alice"},"text":text}}));
            let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event()).await.unwrap().unwrap().into_parts();
            assert_eq!(event.receiving_agent_id, Some(agent.clone()));
            assert_eq!(event.channel_id.as_str(), "private_chat");
            chats.push(event.provider_chat_id);
            let ProviderEventKind::Message(message) = event.kind else { panic!("expected DM") };
            assert_eq!(message.content, text);
            ack.accept();
        }
        assert_eq!(chats, ["300", "300"]);
        for (agent, token) in [(&beta, "200:local"), (&alpha, "201:local")] {
            server.deliver(token, json!({"update_id":2,"message_reaction":{"message_id":8,"date":2,
                "chat":{"id":300,"type":"private"},"user":{"id":300,"is_bot":false,"first_name":"Alice"},
                "old_reaction":[],"new_reaction":[{"type":"emoji","emoji":"👍"}]}}));
            let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event()).await.unwrap().unwrap().into_parts();
            assert_eq!(event.receiving_agent_id, Some(agent.clone()));
            assert_eq!(event.channel_id.as_str(), "private_chat");
            assert_eq!(event.provider_chat_id, "300");
            let ProviderEventKind::Reaction(reaction) = event.kind else { panic!("expected DM reaction") };
            assert_eq!(reaction.target_provider_message_id, "8");
            ack.accept();
        }
    }).await;
    host.shutdown().await.unwrap();
    if let Err(error) = outcome {
        if error.is_panic() {
            std::panic::resume_unwind(error.into_panic());
        }
        panic!("scenario task failed: {error}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_file_does_not_block_the_next_message() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "bot_token", "200:local");
    let host = ProviderHost::new(
        TelegramProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":server.http.endpoint,"poll_timeout_seconds":1}),
        services.host(),
    )
    .unwrap();
    host.init().await.unwrap();
    host.create_agent(bot.clone(), agent_spec(json!({"bot_username":"beta_bot"})))
        .await
        .unwrap();
    server.fail_requests("/bot200:local/getFile", 404, usize::MAX);
    server.deliver("200:local", json!({"update_id":1,"message":{"message_id":8,"date":1,
        "chat":{"id":-1001,"type":"supergroup"},"from":{"id":300,"is_bot":false,"first_name":"Alice"},
        "document":{"file_id":"inbound-file","file_unique_id":"unique","file_name":"gone.bin"}}}));
    server.deliver("200:local", json!({"update_id":2,"message":{"message_id":9,"date":2,
        "chat":{"id":300,"type":"private"},"from":{"id":300,"is_bot":false,"first_name":"Alice"},"text":"next message"}}));
    let result = tokio::time::timeout(Duration::from_secs(3), host.next_event()).await;
    let (event, ack) = result
        .expect("missing file blocked the next message")
        .unwrap()
        .into_parts();
    assert_eq!(event.receiving_agent_id, Some(bot));
    assert!(matches!(event.kind, ProviderEventKind::Message(ref m) if m.content == "next message"));
    ack.accept();
    host.shutdown().await.unwrap();
    assert_eq!(
        services
            .logs
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("Telegram event skipped:"))
            .count(),
        1
    );
}

async fn event_host(
    server: &TelegramServer,
    services: &std::sync::Arc<Services>,
) -> ProviderHost<TelegramProvider> {
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "bot_token", "200:local");
    let host = ProviderHost::new(
        TelegramProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":server.http.endpoint,"poll_timeout_seconds":1,
            "event_retry_initial_ms":20,"event_retry_max_ms":80}),
        services.host(),
    )
    .unwrap();
    host.init().await.unwrap();
    host.create_agent(bot, agent_spec(json!({"bot_username":"beta_bot"})))
        .await
        .unwrap();
    host
}
fn team_message(update: i64, file: bool) -> serde_json::Value {
    let mut value = json!({"update_id":update,"message":{"message_id":update+10,"date":1,
        "chat":{"id":-1001,"type":"supergroup"},"from":{"id":300,"is_bot":false,"first_name":"Alice"}}});
    if file {
        value["message"]["document"] =
            json!({"file_id":"inbound-file","file_unique_id":"unique","file_name":"inbound.bin"});
    } else {
        value["message"]["text"] = json!(format!("message {update}"));
    }
    value
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_temporary_im_outage_delivers_a_batch_once_without_replaying_its_prefix() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    // Queue the whole batch before starting the receiver.
    server.deliver("200:local", team_message(1, false));
    server.deliver("200:local", team_message(2, true));
    server.deliver("200:local", team_message(3, false));
    server.fail_requests("/bot200:local/getFile", 503, 8);
    server.fail_requests("/bot200:local/getUpdates", 429, 1);
    let host = event_host(&server, &services).await;
    for id in 1..=3 {
        let (event, receipt) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
            .await
            .unwrap()
            .unwrap()
            .into_parts();
        let ProviderEventKind::Message(message) = event.kind else {
            panic!("expected message")
        };
        assert_eq!(message.provider_message_id, (id + 10).to_string());
        if id == 2 {
            assert_eq!(
                services
                    .resource(&message.attachments[0])
                    .await
                    .unwrap()
                    .bytes,
                FILE
            );
        }
        receipt.accept();
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(200), host.next_event())
            .await
            .is_err(),
        "duplicate delivery"
    );
    host.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_update_does_not_block_later_messages() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    let mut malformed = team_message(2, false);
    malformed["message"]["message_id"] = json!("invalid message id");
    server.deliver("200:local", malformed);
    server.deliver("200:local", team_message(3, false));
    let host = event_host(&server, &services).await;
    let (event, receipt) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    assert!(matches!(event.kind, ProviderEventKind::Message(ref m) if m.content == "message 3"));
    receipt.accept();
    host.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_agents_send_and_react_in_one_chat_with_their_own_identity() {
    let server = TelegramServer::start();
    let services = Services::new("telegram");
    let host = std::sync::Arc::new(event_host(&server, &services).await);
    for (id, token) in [("alpha", "201:local"), ("gamma", "202:local")] {
        let agent = AgentId::new(id).unwrap();
        services.agent(&agent, "bot_token", token);
        host.create_agent(
            agent,
            agent_spec(json!({"bot_username":format!("{id}_bot")})),
        )
        .await
        .unwrap();
    }
    let parent = services.remember("-1001", "9");
    let mut jobs = tokio::task::JoinSet::new();
    for (index, id) in ["alpha", "beta", "gamma"].into_iter().enumerate() {
        let host = host.clone();
        jobs.spawn(async move {
            let agent = AgentId::new(id).unwrap();
            host.send_message(
                agent.clone(),
                SendMessageSpec {
                    channel_id: ChannelId::new("group_chat").unwrap(),
                    chat_id: "-1001".into(),
                    message_id: MessageId::new(100 + index as i64).unwrap(),
                    idempotency_key: format!("send-{id}"),
                    content: format!("hello from {id}"),
                    reply_to: None,
                    mentions: vec![],
                    attachments: vec![],
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .unwrap();
            host.auto_reaction(
                agent,
                AutoReactionSpec {
                    auto_reaction: "👍".into(),
                    channel_id: ChannelId::new("group_chat").unwrap(),
                    chat_id: "-1001".into(),
                    trigger_messages: vec![parent],
                    provider_agent_config: json!({}),
                },
            )
            .await
            .unwrap();
        });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
    host.shutdown().await.unwrap();
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.path.ends_with("/sendMessage"))
            .count(),
        3
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.path.ends_with("/setMessageReaction"))
            .count(),
        3
    );
    for (id, token) in [
        ("alpha", "201:local"),
        ("beta", "200:local"),
        ("gamma", "202:local"),
    ] {
        let sends: Vec<_> = requests
            .iter()
            .filter(|r| r.path == format!("/bot{token}/sendMessage"))
            .collect();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].json()["chat_id"], -1001);
        assert_eq!(sends[0].json()["text"], format!("hello from {id}"));
        let reactions: Vec<_> = requests
            .iter()
            .filter(|r| r.path == format!("/bot{token}/setMessageReaction"))
            .collect();
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].json()["chat_id"], -1001);
        assert_eq!(reactions[0].json()["message_id"], 9);
        assert_eq!(reactions[0].json()["reaction"][0]["emoji"], "👍");
    }
}
