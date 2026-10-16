#[path = "support/host.rs"]
mod host;
#[path = "support/http.rs"]
mod http;
#[path = "support/server.rs"]
mod server;

use host::{Services, agent_spec};
use lankide_core::{AgentId, ChannelId, MessageId};
use lankide_provider::*;
use lankide_provider_feishu::FeishuProvider;
use serde_json::{Value, json};
use server::{FILE, FeishuServer};
use std::time::Duration;

fn callback(id: &str, message: Value) -> Value {
    json!({"schema":"2.0","header":{"event_id":id,"event_type":"im.message.receive_v1","app_id":"cli_beta","tenant_key":"tenant"},
        "event":{"sender":{"sender_type":"user","sender_id":{"open_id":"ou_alice","union_id":"on_alice"}},"message":message}})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_message_mentioning_a_bot_reaches_its_agent() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "app_secret", "local-secret");
    let host = ProviderHost::new(
        FeishuProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":format!("{}/",server.http.endpoint),"task_system_enabled":false}),
        services.host(),
    )
    .unwrap();
    let host = std::sync::Arc::new(host);
    let scenario_host = host.clone();
    let server = std::sync::Arc::new(server);
    let scenario_server = server.clone();
    let outcome = tokio::spawn(async move {
        let host = scenario_host;
        let server = scenario_server;
        host.init().await.unwrap();
        host.create_agent(bot.clone(), agent_spec(json!({"app_id":"cli_beta"})))
            .await
            .unwrap();
        server.deliver("cli_beta", callback("mention", json!({"message_id":"om_mention","chat_id":"oc_team","chat_type":"group","create_time":"1000",
            "message_type":"text","content":json!({"text":"@_user_1 hello"}).to_string(),"mentions":[{"key":"@_user_1","id":{"open_id":"ou_beta"},"name":"Bot B","mentioned_type":"bot"}]})));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event())
            .await
            .unwrap()
            .unwrap()
            .into_parts();
        assert_eq!(event.provider_chat_id, "oc_team");
        let ProviderEventKind::Message(message) = event.kind else {
            panic!("expected message")
        };
        assert_eq!(message.mentioned_agent_ids, vec![bot.clone()]);
        assert!(message.content.contains("[beta]"));
        ack.accept();
        tokio::time::timeout(Duration::from_secs(30), async {
            while server.acknowledgements() < 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("mention acknowledgement did not reach the server");
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
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "app_secret", "local-secret");
    let host = ProviderHost::new(
        FeishuProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":format!("{}/",server.http.endpoint),"task_system_enabled":false}),
        services.host(),
    )
    .unwrap();
    let host = std::sync::Arc::new(host);
    let scenario_host = host.clone();
    let server = std::sync::Arc::new(server);
    let scenario_server = server.clone();
    let outcome = tokio::spawn(async move {
        let host = scenario_host;
        let server = scenario_server;
        host.init().await.unwrap();
        host.create_agent(bot.clone(), agent_spec(json!({"app_id":"cli_beta"})))
            .await
            .unwrap();
        server.deliver("cli_beta", callback("file", json!({"message_id":"om_file","chat_id":"oc_team","chat_type":"group","create_time":"2000",
            "message_type":"file","content":json!({"file_key":"inbound-file","file_name":"inbound.bin"}).to_string()})));
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
        let parent = services.remember("oc_team", &message.provider_message_id);
        ack.accept();
        let sent = host
            .send_message(
                bot.clone(),
                SendMessageSpec {
                    channel_id: ChannelId::new("group_chat").unwrap(),
                    chat_id: "oc_team".into(),
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
                auto_reaction: "THUMBSUP".into(),
                channel_id: ChannelId::new("group_chat").unwrap(),
                chat_id: "oc_team".into(),
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
                chat_id: "oc_team".into(),
                message_id: MessageId::new(100).unwrap(),
                provider_message_id: sent.provider_message_id.unwrap(),
                provider_agent_config: json!({}),
            },
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            while server.acknowledgements() < 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let deliveries = server.deliveries();
        assert_eq!(deliveries.len(), 2);
        for delivery in &deliveries {
            assert_eq!(delivery.chat, "oc_team");
            assert_eq!(delivery.parent.as_deref(), Some("om_file"));
        }
        assert_eq!(deliveries[1].body["msg_type"], "file");
        let requests = server.requests();
        let upload = requests
            .iter()
            .find(|r| r.path == "/open-apis/im/v1/files")
            .unwrap();
        assert_eq!(upload.part("file"), FILE);
        assert_eq!(upload.part("file_name"), b"inbound.bin");
        let replies: Vec<_> = requests
            .iter()
            .filter(|r| r.path == "/open-apis/im/v1/messages/om_file/reply")
            .collect();
        assert_eq!(replies.len(), 2);
        let reply = replies[1].json();
        assert_eq!(reply["msg_type"], "file");
        assert_eq!(
            serde_json::from_str::<Value>(reply["content"].as_str().unwrap()).unwrap()["file_key"],
            "uploaded-file"
        );
        let reaction = requests
            .iter()
            .find(|r| r.path == "/open-apis/im/v1/messages/om_file/reactions")
            .unwrap()
            .json();
        assert_eq!(reaction["reaction_type"]["emoji_type"], "THUMBSUP");
        assert!(
            requests
                .iter()
                .any(|r| r.method == "DELETE" && r.path == "/open-apis/im/v1/messages/om_sent_1")
        );
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
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let alpha = AgentId::new("alpha").unwrap();
    let beta = AgentId::new("beta").unwrap();
    services.agent(&alpha, "app_secret", "alpha-secret");
    services.agent(&beta, "app_secret", "beta-secret");
    let host = ProviderHost::new(
        FeishuProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":format!("{}/",server.http.endpoint),"task_system_enabled":false}),
        services.host(),
    )
    .unwrap();
    let host = std::sync::Arc::new(host);
    let scenario_host = host.clone();
    let server = std::sync::Arc::new(server);
    let scenario_server = server.clone();
    let outcome = tokio::spawn(async move {
        let host = scenario_host;
        let server = scenario_server;
        host.init().await.unwrap();
        for (agent, app) in [(&alpha, "cli_alpha"), (&beta, "cli_beta")] {
            host.create_agent(agent.clone(), agent_spec(json!({"app_id":app}))).await.unwrap();
        }
        server.deliver("cli_beta", callback("dm-beta", json!({"message_id":"om_dm_beta","chat_id":"oc_dm_beta","chat_type":"p2p","create_time":"1000",
            "message_type":"text","content":json!({"text":"only beta"}).to_string()})));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event()).await.unwrap().unwrap().into_parts();
        assert_eq!(event.receiving_agent_id, Some(beta.clone()));
        assert_ne!(event.receiving_agent_id, Some(alpha.clone()));
        assert_eq!(event.channel_id.as_str(), "private_chat");
        assert_eq!(event.provider_chat_id, "oc_dm_beta");
        let ProviderEventKind::Message(message) = event.kind else { panic!("expected DM") };
        assert_eq!(message.content, "only beta");
        ack.accept();
        server.deliver("cli_beta", json!({"schema":"2.0","header":{"event_id":"dm-reaction","event_type":"im.message.reaction.created_v1","app_id":"cli_beta","tenant_key":"tenant"},
            "event":{"message_id":"om_dm_beta","operator_type":"user","user_id":{"open_id":"ou_alice","union_id":"on_alice"},"reaction_type":{"emoji_type":"THUMBSUP"},"action_time":"2000"}}));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event()).await.unwrap().unwrap().into_parts();
        assert_eq!(event.receiving_agent_id, Some(beta.clone()));
        assert_ne!(event.receiving_agent_id, Some(alpha));
        assert_eq!(event.channel_id.as_str(), "private_chat");
        assert_eq!(event.provider_chat_id, "oc_dm_beta");
        let ProviderEventKind::Reaction(reaction) = event.kind else { panic!("expected DM reaction") };
        assert_eq!(reaction.target_provider_message_id, "om_dm_beta");
        ack.accept();
        server.deliver("cli_beta", callback("group-beta", json!({"message_id":"om_group_beta","chat_id":"oc_group","chat_type":"group","create_time":"3000",
            "message_type":"text","content":json!({"text":"team note"}).to_string()})));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event()).await.unwrap().unwrap().into_parts();
        assert_eq!(event.channel_id.as_str(), "group_chat");
        assert_eq!(event.receiving_agent_id, Some(beta.clone()));
        ack.accept();
        server.deliver("cli_beta", json!({"schema":"2.0","header":{"event_id":"group-reaction","event_type":"im.message.reaction.created_v1","app_id":"cli_beta","tenant_key":"tenant"},
            "event":{"message_id":"om_group_beta","operator_type":"user","user_id":{"open_id":"ou_alice","union_id":"on_alice"},"reaction_type":{"emoji_type":"THUMBSUP"},"action_time":"4000"}}));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(30), host.next_event()).await.unwrap().unwrap().into_parts();
        assert_eq!(event.receiving_agent_id, None);
        assert_eq!(event.channel_id.as_str(), "group_chat");
        assert_eq!(event.provider_chat_id, "oc_group");
        let ProviderEventKind::Reaction(reaction) = event.kind else { panic!("expected group reaction") };
        assert_eq!(reaction.target_provider_message_id, "om_group_beta");
        ack.accept();
        tokio::time::timeout(Duration::from_secs(30), async {
            while server.acknowledgements() < 4 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("DM acknowledgements did not reach server");
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
async fn a_deleted_reaction_target_does_not_block_the_next_message() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "app_secret", "local-secret");
    let host = ProviderHost::new(
        FeishuProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":server.http.endpoint,"task_system_enabled":false}),
        services.host(),
    )
    .unwrap();
    host.init().await.unwrap();
    host.create_agent(bot.clone(), agent_spec(json!({"app_id":"cli_beta"})))
        .await
        .unwrap();
    server.deliver("cli_beta", json!({"schema":"2.0","header":{"event_id":"deleted-reaction","event_type":"im.message.reaction.created_v1","app_id":"cli_beta","tenant_key":"tenant"},
        "event":{"message_id":"om_deleted","operator_type":"user","user_id":{"open_id":"ou_alice","union_id":"on_alice"},"reaction_type":{"emoji_type":"THUMBSUP"},"action_time":"2000"}}));
    server.deliver("cli_beta", callback("after-deleted", json!({"message_id":"om_next","chat_id":"oc_team","chat_type":"p2p","create_time":"3000",
        "message_type":"text","content":json!({"text":"next message"}).to_string()})));
    let result = tokio::time::timeout(Duration::from_secs(3), host.next_event()).await;
    let (event, ack) = result
        .expect("deleted target blocked the next message")
        .unwrap()
        .into_parts();
    assert_eq!(event.receiving_agent_id, Some(bot));
    assert!(matches!(event.kind, ProviderEventKind::Message(ref m) if m.content == "next message"));
    ack.accept();
    tokio::time::timeout(Duration::from_secs(3), async {
        while server.acknowledgements() != 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    host.shutdown().await.unwrap();
    assert_eq!(
        services
            .logs
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("Feishu event skipped:"))
            .count(),
        1
    );
}

async fn event_host(
    server: &FeishuServer,
    services: &std::sync::Arc<Services>,
) -> ProviderHost<FeishuProvider> {
    let bot = AgentId::new("beta").unwrap();
    services.agent(&bot, "app_secret", "local-secret");
    let host = ProviderHost::new(
        FeishuProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":server.http.endpoint,"task_system_enabled":false,
            "event_retry_initial_ms":20,"event_retry_max_ms":80}),
        services.host(),
    )
    .unwrap();
    host.init().await.unwrap();
    host.create_agent(bot, agent_spec(json!({"app_id":"cli_beta"})))
        .await
        .unwrap();
    host
}
fn team_message(id: &str) -> Value {
    callback(
        id,
        json!({"message_id":format!("om_{id}"),"chat_id":"oc_team","chat_type":"group","create_time":"1000",
        "message_type":"text","content":json!({"text":id}).to_string()}),
    )
}
fn reaction(id: &str) -> Value {
    json!({"schema":"2.0","header":{"event_id":format!("reaction-{id}"),"event_type":"im.message.reaction.created_v1","app_id":"cli_beta","tenant_key":"tenant"},
        "event":{"message_id":format!("om_{id}"),"operator_type":"user","user_id":{"open_id":"ou_alice","union_id":"on_alice"},"reaction_type":{"emoji_type":"THUMBSUP"},"action_time":"2000"}})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_temporary_im_outage_delivers_the_reaction_and_next_message_once() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = event_host(&server, &services).await;
    server.deliver("cli_beta", team_message("before"));
    let delivery = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap();
    delivery.into_parts().1.accept();
    server.fail_requests("/open-apis/im/v1/messages/om_before", 503, 8);
    server.fail_requests("/open-apis/im/v1/chats/oc_team", 400, 1);
    server.deliver("cli_beta", reaction("before"));
    server.deliver("cli_beta", team_message("after"));
    let (event, receipt) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    assert!(
        matches!(event.kind, ProviderEventKind::Reaction(ref r) if r.target_provider_message_id == "om_before")
    );
    receipt.accept();
    let (event, receipt) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    assert!(matches!(event.kind, ProviderEventKind::Message(ref m) if m.content == "after"));
    receipt.accept();
    assert!(
        tokio::time::timeout(Duration::from_millis(200), host.next_event())
            .await
            .is_err(),
        "duplicate delivery"
    );
    host.shutdown().await.unwrap();
    assert_eq!(server.acknowledgements(), 3);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_event_does_not_block_later_messages() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = event_host(&server, &services).await;
    let mut malformed = team_message("malformed");
    malformed["event"]["message"]["message_id"] = json!("");
    server.deliver("cli_beta", malformed);
    server.deliver("cli_beta", team_message("after"));
    let (event, receipt) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    assert!(matches!(event.kind, ProviderEventKind::Message(ref m) if m.content == "after"));
    receipt.accept();
    host.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_agents_send_and_react_in_one_chat_with_their_own_identity() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = std::sync::Arc::new(event_host(&server, &services).await);
    for id in ["alpha", "gamma"] {
        let agent = AgentId::new(id).unwrap();
        services.agent(&agent, "app_secret", "local-secret");
        host.create_agent(agent, agent_spec(json!({"app_id":format!("cli_{id}")})))
            .await
            .unwrap();
    }
    let parent = services.remember("oc_team", "om_file");
    let mut jobs = tokio::task::JoinSet::new();
    for (index, id) in ["alpha", "beta", "gamma"].into_iter().enumerate() {
        let host = host.clone();
        jobs.spawn(async move {
            let agent = AgentId::new(id).unwrap();
            host.send_message(
                agent.clone(),
                SendMessageSpec {
                    channel_id: ChannelId::new("group_chat").unwrap(),
                    chat_id: "oc_team".into(),
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
                    auto_reaction: "THUMBSUP".into(),
                    channel_id: ChannelId::new("group_chat").unwrap(),
                    chat_id: "oc_team".into(),
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
    let deliveries = server.deliveries();
    assert_eq!(deliveries.len(), 3);
    let requests = server.requests();
    for id in ["alpha", "beta", "gamma"] {
        let sent: Vec<_> = deliveries
            .iter()
            .filter(|d| d.sender == format!("cli_{id}"))
            .collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].chat, "oc_team");
        assert!(
            sent[0].body["content"]
                .as_str()
                .unwrap()
                .contains(&format!("hello from {id}"))
        );
        let reactions: Vec<_> = requests
            .iter()
            .filter(|r| {
                r.path == "/open-apis/im/v1/messages/om_file/reactions"
                    && r.header("authorization") == format!("Bearer tenant-cli_{id}")
            })
            .collect();
        assert_eq!(reactions.len(), 1);
        assert_eq!(
            reactions[0].json()["reaction_type"]["emoji_type"],
            "THUMBSUP"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_png_messages_keep_media_type_and_group_name() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = event_host(&server, &services).await;
    for (id, bot) in [("human-image", false), ("bot-image", true)] {
        let mut event = callback(
            id,
            json!({"message_id":format!("om_{id}"),"chat_id":"oc_team","chat_type":"group","create_time":"1000",
            "message_type":"image","content":json!({"image_key":"inbound-image"}).to_string()}),
        );
        if bot {
            event["event"]["sender"] = json!({"sender_type":"app","sender_id":{"open_id":"ou_alpha","union_id":"on_alpha"}});
        }
        server.deliver("cli_beta", event);
        let (event, receipt) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
            .await
            .unwrap()
            .unwrap()
            .into_parts();
        let ProviderEventKind::Message(message) = event.kind else {
            panic!("expected image");
        };
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(message.attachments[0].media_type, "image/png");
        assert_eq!(message.attachments[0].file_name, "image.png");
        assert_eq!(event.provider_data["chat_name"], "Design team");
        receipt.accept();
    }
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_unresolved_agent_mentions_fail_before_sending() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = event_host(&server, &services).await;
    let alpha = AgentId::new("alpha").unwrap();
    services.agent(&alpha, "app_secret", "local-secret");
    host.create_agent(
        alpha,
        agent_spec(json!({"mode":"existing","app_id":"cli_alpha"})),
    )
    .await
    .unwrap();
    let send = |key: &str, content: &str| SendMessageSpec {
        channel_id: ChannelId::new("group_chat").unwrap(),
        chat_id: "oc_team".into(),
        message_id: MessageId::new(100).unwrap(),
        idempotency_key: key.into(),
        content: content.into(),
        reply_to: None,
        mentions: vec![],
        attachments: vec![],
        force: false,
        provider_agent_config: json!({}),
    };
    let bare = host
        .send_message(
            AgentId::new("beta").unwrap(),
            send("bare-mention", "@Bot B please check"),
        )
        .await;
    assert!(
        matches!(&bare, Err(ProviderError::InvalidInput { field }) if field.contains("@名称[成员ID]")),
        "a plain @Name would reach nobody: {bare:?}"
    );
    let result = host
        .send_message(
            AgentId::new("beta").unwrap(),
            SendMessageSpec {
                channel_id: ChannelId::new("group_chat").unwrap(),
                chat_id: "oc_team".into(),
                message_id: MessageId::new(100).unwrap(),
                idempotency_key: "unresolved-mention".into(),
                content: "@Alpha[alpha] please check".into(),
                reply_to: None,
                mentions: vec![],
                attachments: vec![],
                force: false,
                provider_agent_config: json!({}),
            },
        )
        .await;
    host.shutdown().await.unwrap();
    assert!(
        matches!(result, Err(ProviderError::InvalidInput { .. })),
        "unresolved mention must give a clear input error: {result:?}"
    );
    assert!(server.deliveries().is_empty());
}

async fn mock_runtime(
    server: &FeishuServer,
    services: &std::sync::Arc<Services>,
) -> std::sync::Arc<lankide_runtime::Runtime> {
    use lankide_protocol::{AgentRuntimeProfile, HarnessConfig, SessionDirective};
    let runtime = std::sync::Arc::new(services.runtime());
    for id in ["alpha", "beta"] {
        let agent = AgentId::new(id).unwrap();
        if services
            .db
            .agent_record(&services.workspace, &agent)
            .is_err()
        {
            services.agent(&agent, "app_secret", "local-secret");
        }
        let mut record = services
            .db
            .agent_record(&services.workspace, &agent)
            .unwrap();
        record.provider_agent_config = json!({"mode":"existing","app_id":format!("cli_{id}")});
        services.db.update_agent_record(record).unwrap();
    }
    let endpoint = server.http.endpoint.clone();
    let host_services = services.clone();
    runtime
        .register_provider_factory(services.workspace.clone(), move || {
            ProviderHost::new(
                FeishuProvider::new(),
                host_services.workspace.clone(),
                json!({"api_endpoint":endpoint,"task_system_enabled":true}),
                host_services.host(),
            )
        })
        .await
        .unwrap();
    for id in ["alpha", "beta"] {
        runtime
            .configure_agent(
                &services.workspace,
                lankide_runtime::AgentRuntimeConfig {
                    agent_id: AgentId::new(id).unwrap(),
                    provider_sender_id: id.into(),
                    attention: std::collections::BTreeMap::from([(
                        ChannelId::new("group_chat").unwrap(),
                        lankide_runtime::AttentionPolicy::Mention,
                    )]),
                    auto_reaction: None,
                    agent_config: json!({}),
                    provider_agent_config: json!({"app_id":format!("cli_{id}")}),
                    agent_profile: AgentRuntimeProfile {
                        name: id.into(),
                        ..Default::default()
                    },
                    machine_id: "fixture-machine".into(),
                    harness: HarnessConfig::default(),
                    session: SessionDirective::default(),
                    execution_token: "fixture".into(),
                },
            )
            .await
            .unwrap();
    }
    runtime.start().await.unwrap();
    runtime
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_attachment_echoes_leave_one_sent_message_and_one_mention() {
    use lankide_storage::{SendOutcome, SendRequest};
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let runtime = mock_runtime(&server, &services).await;
    let channel = ChannelId::new("group_chat").unwrap();
    server.deliver("cli_beta", team_message("start"));
    let delivery = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    runtime
        .accept_provider_event(&services.workspace, delivery, 1)
        .await
        .unwrap();
    // Observe Alpha's stable App identity before Beta mentions Alpha.
    let mut mention = team_message("identify-alpha");
    mention["event"]["sender"] =
        json!({"sender_type":"app","sender_id":{"open_id":"ou_alpha","union_id":"on_alpha"}});
    mention["event"]["message_sender"] =
        json!({"id":"cli_alpha","id_type":"app_id","sender_type":"app"});
    server.deliver("cli_beta", mention);
    let delivery = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    runtime
        .accept_provider_event(&services.workspace, delivery, 2)
        .await
        .unwrap();
    let file = services
        .save(ResourceUpload {
            file_name: "report.csv".into(),
            media_type: "text/csv".into(),
            bytes: FILE.to_vec(),
        })
        .await
        .unwrap();
    let image = services
        .save(ResourceUpload {
            file_name: "ok.png".into(),
            media_type: "image/png".into(),
            bytes: server::PNG.to_vec(),
        })
        .await
        .unwrap();
    let send = services
        .db
        .begin_send(SendRequest {
            workspace_id: services.workspace.clone(),
            channel_id: channel.clone(),
            provider_chat_id: "oc_team".into(),
            request_key: "live-attachments".into(),
            sender_agent_id: AgentId::new("beta").unwrap(),
            content: "@Alpha[alpha] Report".into(),
            now: 3,
        })
        .unwrap();
    let before_alpha = services
        .db
        .inbox_items(&services.workspace, &AgentId::new("alpha").unwrap())
        .unwrap()
        .len();
    let before_beta = services
        .db
        .inbox_items(&services.workspace, &AgentId::new("beta").unwrap())
        .unwrap()
        .len();
    server.echo_sends();
    let (uploading, release) = server.hold_next_upload();
    let receiver = {
        let runtime = runtime.clone();
        let workspace = services.workspace.clone();
        tokio::spawn(async move {
            uploading.await.unwrap();
            for index in 0..3 {
                let delivery = tokio::time::timeout(
                    Duration::from_secs(5),
                    runtime.next_provider_event(&workspace),
                )
                .await
                .unwrap()
                .unwrap()
                .unwrap();
                assert!(matches!(
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        runtime.accept_provider_event(&workspace, delivery, 4),
                    )
                    .await
                    .expect("Agent echo must be dropped while the upload is pending")
                    .unwrap(),
                    lankide_runtime::CoordinatedEvent::Dropped
                ));
                if index == 0 {
                    release.send(()).unwrap();
                }
            }
        })
    };
    let sent = runtime
        .send_message(
            &services.workspace,
            AgentId::new("beta").unwrap(),
            SendMessageSpec {
                channel_id: channel.clone(),
                chat_id: "oc_team".into(),
                message_id: MessageId::new(send.message_id).unwrap(),
                idempotency_key: send.request_key.clone(),
                content: "@Alpha[alpha] Report".into(),
                reply_to: None,
                mentions: vec![],
                attachments: vec![
                    MessageAttachment {
                        resource: file.clone(),
                        caption: None,
                    },
                    MessageAttachment {
                        resource: image.clone(),
                        caption: None,
                    },
                ],
                force: false,
                provider_agent_config: json!({}),
            },
        )
        .await
        .unwrap();
    let finish = services.db.finish_send(
        &send.request_key,
        SendOutcome::Sent {
            provider_message_id: sent.provider_message_id.unwrap(),
        },
        5,
    );
    receiver.await.unwrap();
    // The platform can repeat any part after the HTTP send has finished.
    let mut late = team_message("late-attachment-echo");
    late["event"]["sender"] =
        json!({"sender_type":"app","sender_id":{"open_id":"ou_beta","union_id":"on_beta"}});
    late["event"]["message"]["message_id"] = json!("om_sent_3");
    server.deliver("cli_beta", late);
    let event = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        runtime
            .accept_provider_event(&services.workspace, event, 6)
            .await
            .unwrap(),
        lankide_runtime::CoordinatedEvent::Dropped
    ));
    assert_eq!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("alpha").unwrap())
            .unwrap()
            .len(),
        before_alpha + 1,
        "one canonical mention wakes the recipient"
    );
    assert_eq!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("beta").unwrap())
            .unwrap()
            .len(),
        before_beta,
        "sender never wakes itself"
    );
    let snapshot = services
        .db
        .create_execution(
            &services.workspace,
            &AgentId::new("alpha").unwrap(),
            "mention",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        )
        .unwrap()
        .unwrap()
        .snapshot;
    assert_eq!(snapshot.chats[0].chat.chat_name, "Design team");
    let outbound = snapshot
        .chats
        .iter()
        .flat_map(|chat| &chat.recent_messages)
        .find(|message| message.message_id == send.message_id)
        .unwrap();
    assert_eq!(outbound.resources.len(), 2);
    assert_eq!(outbound.resources[0].resource_id.get(), file.resource_id);
    assert!(
        snapshot
            .chats
            .iter()
            .flat_map(|chat| &chat.trigger_reasons)
            .any(|reason| matches!(reason, lankide_storage::DispatchReason::Mention))
    );
    runtime.shutdown().await.unwrap();
    assert!(
        finish.is_ok(),
        "send must succeed after early echoes: {finish:?}"
    );
    assert_eq!(
        services.db.send_attempt(&send.request_key).unwrap().state,
        "sent"
    );
    assert_eq!(
        services.db.message_count(&services.workspace).unwrap(),
        2,
        "one human message plus one multipart outbound; Agent observations create no rows"
    );
    for id in ["om_sent_1", "om_sent_2", "om_sent_3"] {
        assert_eq!(
            services
                .db
                .find_message(&services.workspace, &channel, "oc_team", id)
                .unwrap(),
            Some(send.message_id)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn human_message_is_processed_while_another_agent_uploads() {
    use lankide_storage::SendRequest;
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let runtime = mock_runtime(&server, &services).await;
    let channel = ChannelId::new("group_chat").unwrap();
    server.deliver("cli_beta", team_message("before-upload"));
    let delivery = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    runtime
        .accept_provider_event(&services.workspace, delivery, 0)
        .await
        .unwrap();
    let file = services
        .save(ResourceUpload {
            file_name: "report.csv".into(),
            media_type: "text/csv".into(),
            bytes: FILE.to_vec(),
        })
        .await
        .unwrap();
    let send = services
        .db
        .begin_send(SendRequest {
            workspace_id: services.workspace.clone(),
            channel_id: channel.clone(),
            provider_chat_id: "oc_team".into(),
            request_key: "concurrent-upload".into(),
            sender_agent_id: AgentId::new("beta").unwrap(),
            content: String::new(),
            now: 1,
        })
        .unwrap();
    let (uploading, release) = server.hold_next_upload();
    let sending = {
        let runtime = runtime.clone();
        let workspace = services.workspace.clone();
        tokio::spawn(async move {
            runtime
                .send_message(
                    &workspace,
                    AgentId::new("beta").unwrap(),
                    SendMessageSpec {
                        channel_id: channel,
                        chat_id: "oc_team".into(),
                        message_id: MessageId::new(send.message_id).unwrap(),
                        idempotency_key: send.request_key,
                        content: String::new(),
                        reply_to: None,
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
        })
    };
    tokio::time::timeout(Duration::from_secs(3), uploading)
        .await
        .unwrap()
        .unwrap();
    let mut human = team_message("human-during-upload");
    human["header"]["app_id"] = json!("cli_alpha");
    human["event"]["message"]["content"] =
        json!({"text":"@_user_1 please help"}).to_string().into();
    human["event"]["message"]["mentions"] = json!([
        {"key":"@_user_1","id":{"open_id":"ou_alpha","union_id":"on_alpha"},"name":"Alpha","mentioned_type":"bot"}
    ]);
    server.deliver("cli_alpha", human);
    let delivery = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.next_provider_event(&services.workspace),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let coordinated = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.accept_provider_event(&services.workspace, delivery, 2),
    )
    .await
    .expect("human message must be processed before the upload is released")
    .unwrap();
    assert!(matches!(
        coordinated,
        lankide_runtime::CoordinatedEvent::Message { .. }
    ));
    assert_eq!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("alpha").unwrap())
            .unwrap()
            .len(),
        1
    );
    assert!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("beta").unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(!sending.is_finished(), "attachment send is still uploading");
    release.send(()).unwrap();
    sending.await.unwrap().unwrap();
    assert_eq!(services.db.message_count(&services.workspace).unwrap(), 3);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mock_agent_dm_reactions_are_recorded_without_waking_the_actor() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let runtime = mock_runtime(&server, &services).await;
    server.deliver("cli_beta",callback("human-dm",json!({"message_id":"om_dm","chat_id":"oc_dm","chat_type":"p2p","create_time":"1000","message_type":"text","content":json!({"text":"Please check"}).to_string()})));
    let delivery = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    runtime
        .accept_provider_event(&services.workspace, delivery, 1)
        .await
        .unwrap();
    server.deliver("cli_beta",json!({"schema":"2.0","header":{"event_id":"own-dm-reaction","event_type":"im.message.reaction.created_v1","app_id":"cli_beta","tenant_key":"tenant"},"event":{"message_id":"om_dm","operator_type":"app","app_id":"cli_beta","reaction_type":{"emoji_type":"OK"},"action_time":"2000"}}));
    let delivery = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.next_provider_event(&services.workspace),
    )
    .await;
    if let Ok(Ok(Some(delivery))) = delivery {
        let result = runtime
            .accept_provider_event(&services.workspace, delivery, 2)
            .await
            .unwrap();
        assert!(
            matches!(result,lankide_runtime::CoordinatedEvent::Reaction(ref result) if result.reaction_id.is_some())
        );
    } else {
        runtime.shutdown().await.unwrap();
        panic!("the bot's DM reaction must be recorded");
    }
    assert_eq!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("beta").unwrap())
            .unwrap()
            .len(),
        1
    );
    assert!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("alpha").unwrap())
            .unwrap()
            .is_empty()
    );
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_existing_bot_binding_resolves_pending_app_observations() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = event_host(&server, &services).await;
    let mut observation = team_message("unbound-alpha");
    observation["event"]["sender"] = json!({"sender_type":"app","sender_id":{"open_id":"ou_alpha_via_beta","union_id":"on_alpha"}});
    observation["event"]["message_sender"] =
        json!({"id":"cli_alpha","id_type":"app_id","sender_type":"app","tenant_key":"tenant"});
    server.deliver("cli_beta", observation);
    let (event, ack) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    assert_eq!(event.sender.kind, EventSenderType::Bot);
    ack.accept();
    let alpha = AgentId::new("alpha").unwrap();
    services.agent(&alpha, "app_secret", "local-secret");
    host.create_agent(
        alpha,
        agent_spec(json!({"mode":"existing","app_id":"cli_alpha"})),
    )
    .await
    .unwrap();
    let state = services.workspace_state().await.unwrap();
    host.shutdown().await.unwrap();
    let identity = &state["identity_directory"]["agents"]["alpha"];
    assert_eq!(identity["union_id"], "on_alpha");
    assert_eq!(identity["tenant_key"], "tenant");
    assert_eq!(identity["open_ids_by_app"]["cli_beta"], "ou_alpha_via_beta");
    assert_eq!(state["identity_directory"]["unresolved_bots"], json!({}));
}

fn task_spec() -> AgentRuntimeSpec {
    AgentRuntimeSpec {
        agent_config: json!({}),
        provider_agent_config: json!({"app_id":"cli_beta"}),
        machine_id: "fixture-machine".into(),
        harness_id: "pi".into(),
        model: "model".into(),
        thinking_level: "low".into(),
        trigger_reason: "mention".into(),
        chats: vec![],
        unread_snapshot: json!({}),
    }
}
async fn task_host(
    server: &FeishuServer,
    services: &std::sync::Arc<Services>,
) -> ProviderHost<FeishuProvider> {
    let lead = services.get_or_create_person("on_alice").await.unwrap();
    services
        .db
        .set_person_team_role(&services.workspace, &lead.person_id, TeamRole::Lead, 1)
        .unwrap();
    services.compare_and_set_person_state(&lead.person_id,lead.state_revision,json!({"external_id_type":"union_id","open_ids_by_app":{"cli_beta":"ou_alice","cli_alpha":"ou_alice"}})).await.unwrap();
    for id in ["alpha", "beta"] {
        services.agent(&AgentId::new(id).unwrap(), "app_secret", "local-secret");
    }
    let host = ProviderHost::new(
        FeishuProvider::new(),
        services.workspace.clone(),
        json!({"api_endpoint":server.http.endpoint,"task_system_enabled":true}),
        services.host(),
    )
    .unwrap();
    host.init().await.unwrap();
    for id in ["alpha", "beta"] {
        host.create_agent(
            AgentId::new(id).unwrap(),
            agent_spec(json!({"mode":"existing","app_id":format!("cli_{id}")})),
        )
        .await
        .unwrap();
    }
    host
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_task_due_input_is_actionable_and_accepts_a_date() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = task_host(&server, &services).await;
    let invalid = host
        .invoke_tool_in_context(
            AgentId::new("beta").unwrap(),
            task_spec(),
            "feishu_task_create",
            json!({"summary":"Release","due":{"date":"2026-10-20"}}),
        )
        .await
        .unwrap();
    assert_eq!(
        invalid["error"]["code"], "invalid_input",
        "invalid date shape is not a transient platform failure: {invalid}"
    );
    let created = host
        .invoke_tool_in_context(
            AgentId::new("beta").unwrap(),
            task_spec(),
            "feishu_task_create",
            json!({"summary":"Release","due":"2026-10-20"}),
        )
        .await
        .unwrap();
    assert_eq!(
        created["ok"], true,
        "plain date must be accepted: {created}"
    );
    assert_eq!(created["task"]["due"]["is_all_day"], true);
    host.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_task_comments_mention_members_and_events_name_the_actor() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = task_host(&server, &services).await;
    let created = host
        .invoke_tool_in_context(
            AgentId::new("beta").unwrap(),
            task_spec(),
            "feishu_task_create",
            json!({"summary":"Release","assignee_ids":["alpha"]}),
        )
        .await
        .unwrap();
    let task_id = created["task"]["task_id"].as_str().unwrap();
    // Resolve the bot under Beta's App through a real delivered message.
    let mut observation = team_message("task-alpha-identity");
    observation["event"]["sender"] = json!({"sender_type":"app","sender_id":{"open_id":"ou_alpha_via_beta","union_id":"on_alpha"}});
    observation["event"]["message_sender"] =
        json!({"id":"cli_alpha","id_type":"app_id","sender_type":"app"});
    server.deliver("cli_beta", observation);
    let (_, ack) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    ack.accept();
    let lead = services.get_or_create_person("on_alice").await.unwrap();
    let bare = host
        .invoke_tool_in_context(
            AgentId::new("beta").unwrap(),
            task_spec(),
            "feishu_task_comment",
            json!({"task_id":task_id,"content":"@Bot B check the report"}),
        )
        .await
        .unwrap();
    assert_eq!(
        bare["error"]["code"], "invalid_input",
        "a plain @Name would reach nobody: {bare}"
    );
    let commented=host.invoke_tool_in_context(AgentId::new("beta").unwrap(),task_spec(),"feishu_task_comment",json!({"task_id":task_id,"content":format!("@Alpha[alpha] @Alice[{}] check the report",lead.person_id)})).await.unwrap();
    assert_eq!(
        commented["ok"], true,
        "task mentions must work without a prior cross-App IM mention: {commented}"
    );
    let comment_id = commented["comment_id"].as_str().unwrap();
    // Task events also reach Apps that cannot read the task.
    server.deliver("cli_beta",json!({"schema":"2.0","header":{"event_id":"foreign-task","event_type":"task.task.update_tenant_v1","app_id":"cli_beta","tenant_key":"tenant","create_time":"2998"},"event":{"task_guid":"task_by_someone_else","event_types":["task_create"]}}));
    // Feishu also reports the comment as a task update without the comment.
    server.deliver("cli_beta",json!({"schema":"2.0","header":{"event_id":"comment-update-copy","event_type":"task.task.update_tenant_v1","app_id":"cli_beta","tenant_key":"tenant","create_time":"2999"},"event":{"task_guid":task_id,"event_types":["task_comment_create"]}}));
    server.deliver("cli_beta",json!({"schema":"2.0","header":{"event_id":"comment-created","event_type":"task.task.comment.updated_v1","app_id":"cli_beta","tenant_key":"tenant","create_time":"3000"},"event":{"task_id":task_id,"comment_id":comment_id,"obj_type":1}}));
    let (event, ack) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
        .await
        .unwrap()
        .unwrap()
        .into_parts();
    let posted = server
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path.split('?').next().unwrap().ends_with("/comments"))
        .unwrap()
        .json();
    assert!(
        posted["rich_content"]
            .as_str()
            .unwrap()
            .contains("<at id=ou_alpha_via_beta></at>")
    );
    assert!(
        posted["rich_content"]
            .as_str()
            .unwrap()
            .contains("<at id=ou_alice></at>")
    );
    assert_eq!(event.sender.id, "beta");
    assert_eq!(event.sender.kind, EventSenderType::Agent);
    let canonical = event.clone();
    let ProviderEventKind::Custom(custom) = event.kind else {
        panic!("expected task event")
    };
    assert!(
        custom.subject_name.contains("Release"),
        "{}",
        custom.subject_name
    );
    assert!(
        custom.subject_name.contains("check the report"),
        "{}",
        custom.subject_name
    );
    assert!(
        custom
            .target_agent_ids
            .contains(&AgentId::new("alpha").unwrap())
    );
    assert!(
        custom
            .mentioned_agent_ids
            .contains(&AgentId::new("alpha").unwrap())
    );
    assert!(
        !custom
            .target_agent_ids
            .contains(&AgentId::new("beta").unwrap())
    );
    ack.accept();
    host.shutdown().await.unwrap();
    let runtime = mock_runtime(&server, &services).await;
    runtime
        .coordinate_event(&services.workspace, canonical.clone(), 4)
        .await
        .unwrap();
    runtime
        .coordinate_event(&services.workspace, canonical, 5)
        .await
        .unwrap();
    let alpha = AgentId::new("alpha").unwrap();
    assert_eq!(
        services
            .db
            .inbox_items(&services.workspace, &alpha)
            .unwrap()
            .len(),
        1
    );
    assert!(
        services
            .db
            .inbox_items(&services.workspace, &AgentId::new("beta").unwrap())
            .unwrap()
            .is_empty()
    );
    let snapshot = services
        .db
        .create_execution(&services.workspace, &alpha, "task mention", 6)
        .unwrap()
        .unwrap()
        .snapshot;
    assert!(
        snapshot
            .chats
            .iter()
            .flat_map(|chat| &chat.trigger_reasons)
            .any(|reason| matches!(reason, lankide_storage::DispatchReason::Mention))
    );
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_agent_starts_a_dm_with_a_known_human() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = task_host(&server, &services).await;
    let human = services.get_or_create_person("on_alice").await.unwrap();
    let sent = host
        .send_message(
            AgentId::new("beta").unwrap(),
            SendMessageSpec {
                channel_id: ChannelId::new("private_chat").unwrap(),
                chat_id: format!("person:{}", human.person_id),
                message_id: MessageId::new(100).unwrap(),
                idempotency_key: "start-human-dm".into(),
                content: "Hello Alice".into(),
                reply_to: None,
                mentions: vec![],
                attachments: vec![],
                force: false,
                provider_agent_config: json!({}),
            },
        )
        .await;
    host.shutdown().await.unwrap();
    assert!(
        sent.is_ok(),
        "known team members can receive a first direct message: {sent:?}"
    );
    let request = server
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.path.starts_with("/open-apis/im/v1/messages"))
        .unwrap();
    assert!(request.path.contains("receive_id_type=open_id"));
    assert_eq!(request.json()["receive_id"], "ou_alice");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_task_events_describe_actions_without_waking_the_actor() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let host = task_host(&server, &services).await;
    let beta = AgentId::new("beta").unwrap();
    let created = host
        .invoke_tool_in_context(
            beta.clone(),
            task_spec(),
            "feishu_task_create",
            json!({"summary":"Release","assignee_ids":["alpha"]}),
        )
        .await
        .unwrap();
    let id = created["task"]["task_id"].as_str().unwrap();
    for (kind, word) in [
        ("task_create", "创建"),
        ("task_assignees_update", "负责人"),
        ("task_desc_update", "描述"),
        ("task_completed_update", "完成"),
    ] {
        let call = match kind {
            "task_desc_update" => Some((
                "feishu_task_update",
                json!({"task_id":id,"description":"Ship the report"}),
            )),
            "task_completed_update" => Some(("feishu_task_complete", json!({"task_id":id}))),
            _ => None,
        };
        if let Some((tool, input)) = call {
            let result = host
                .invoke_tool_in_context(beta.clone(), task_spec(), tool, input)
                .await
                .unwrap();
            assert_eq!(result["ok"], true, "{result}");
        }
        server.deliver("cli_beta",json!({"schema":"2.0","header":{"event_id":format!("task-event-{kind}"),"event_type":"task.task.update_user_access_v2","app_id":"cli_beta","tenant_key":"tenant","create_time":match kind {"task_desc_update"=>"3000","task_completed_update"=>"4000",_=>"2000"}},"event":{"task_guid":id,"event_types":[kind]}}));
        let (event, ack) = tokio::time::timeout(Duration::from_secs(3), host.next_event())
            .await
            .unwrap()
            .unwrap()
            .into_parts();
        assert_eq!(
            event.sender.id, "beta",
            "local task actions retain their actor"
        );
        let ProviderEventKind::Custom(custom) = event.kind else {
            panic!("expected task event")
        };
        assert!(
            custom.subject_name.contains("Release") && custom.subject_name.contains(word),
            "{}",
            custom.subject_name
        );
        assert!(
            custom
                .target_agent_ids
                .contains(&AgentId::new("alpha").unwrap())
        );
        assert!(!custom.target_agent_ids.contains(&beta));
        ack.accept();
    }
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_first_dm_uses_the_returned_chat_for_history_and_echoes() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let runtime = mock_runtime(&server, &services).await;
    let person = services.get_or_create_person("on_alice").await.unwrap();
    services
        .db
        .set_person_team_role(&services.workspace, &person.person_id, TeamRole::Member, 1)
        .unwrap();
    services
        .compare_and_set_person_state(
            &person.person_id,
            person.state_revision,
            json!({"external_id_type":"union_id","open_ids_by_app":{"cli_beta":"ou_alice"}}),
        )
        .await
        .unwrap();
    let beta = AgentId::new("beta").unwrap();
    let channel = ChannelId::new("private_chat").unwrap();
    let target = services
        .db
        .prepare_direct_chat(
            &services.workspace,
            &beta,
            &channel,
            "私信",
            &person.person_id,
            1,
        )
        .unwrap();
    let attempt = services
        .db
        .begin_send(lankide_storage::SendRequest {
            workspace_id: services.workspace.clone(),
            channel_id: channel.clone(),
            provider_chat_id: target.clone(),
            sender_agent_id: beta.clone(),
            request_key: "first-dm".into(),
            content: "Hello Alice".into(),
            now: 1,
        })
        .unwrap();
    let sent = runtime
        .send_message(
            &services.workspace,
            beta.clone(),
            SendMessageSpec {
                channel_id: channel.clone(),
                chat_id: target,
                message_id: MessageId::new(attempt.message_id).unwrap(),
                idempotency_key: attempt.request_key.clone(),
                content: "Hello Alice".into(),
                reply_to: None,
                mentions: vec![],
                attachments: vec![],
                force: false,
                provider_agent_config: json!({}),
            },
        )
        .await
        .unwrap();
    assert_eq!(sent.provider_chat_id.as_deref(), Some("oc_direct_alice"));
    assert_eq!(
        services
            .db
            .find_message_for_receiver(
                &services.workspace,
                &channel,
                "oc_direct_alice",
                sent.provider_message_id.as_ref().unwrap(),
                Some(&beta)
            )
            .unwrap(),
        Some(attempt.message_id)
    );
    assert_eq!(
        services
            .db
            .prepare_direct_chat(
                &services.workspace,
                &beta,
                &channel,
                "私信",
                &person.person_id,
                2
            )
            .unwrap(),
        "oc_direct_alice"
    );
    let mut echo = team_message("first-dm-echo");
    echo["event"]["sender"] =
        json!({"sender_type":"app","sender_id":{"open_id":"ou_beta","union_id":"on_beta"}});
    echo["event"]["message"]["message_id"] = json!(sent.provider_message_id.unwrap());
    echo["event"]["message"]["chat_id"] = json!("oc_direct_alice");
    echo["event"]["message"]["chat_type"] = json!("p2p");
    server.deliver("cli_beta", echo);
    let event = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        runtime
            .accept_provider_event(&services.workspace, event, 3)
            .await
            .unwrap(),
        lankide_runtime::CoordinatedEvent::Dropped
    ));
    assert_eq!(services.db.message_count(&services.workspace).unwrap(), 1);
    assert_eq!(services.db.send_attempt("first-dm").unwrap().state, "sent");
    assert!(
        services
            .db
            .inbox_items(&services.workspace, &beta)
            .unwrap()
            .is_empty()
    );
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_group_name_is_kept_in_the_readable_snapshot() {
    let server = FeishuServer::start();
    let services = Services::new("feishu");
    let runtime = mock_runtime(&server, &services).await;
    let mut named = team_message("named-chat");
    named["event"]["message"]["mentions"] =
        json!([{"key":"@_user_1","id":{"open_id":"ou_beta"},"name":"Beta","mentioned_type":"bot"}]);
    named["event"]["message"]["content"] = json!(json!({"text":"@_user_1 hello"}).to_string());
    server.deliver("cli_beta", named);
    let event = runtime
        .next_provider_event(&services.workspace)
        .await
        .unwrap()
        .unwrap();
    runtime
        .accept_provider_event(&services.workspace, event, 1)
        .await
        .unwrap();
    let snapshot = services
        .db
        .create_execution(
            &services.workspace,
            &AgentId::new("beta").unwrap(),
            "chat name",
            2,
        )
        .unwrap()
        .unwrap()
        .snapshot;
    assert_eq!(snapshot.chats[0].chat.chat_name, "Design team");
    runtime.shutdown().await.unwrap();
}
