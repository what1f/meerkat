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
