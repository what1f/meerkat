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
