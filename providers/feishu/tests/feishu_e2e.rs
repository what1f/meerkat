//! Feishu module-level E2E entrypoint.
//!
//! The ignored case is the single module acceptance scenario: it exercises the
//! public wire/normalization surface and then drives the public ProviderHost
//! through the native Feishu websocket and REST client, while the user OAuth
//! actor is the independent inbound event producer. Credentials are read only
//! from the owner-only bootstrap fixture and never enter an argument, log, or
//! artifact.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flate2::read::GzDecoder;
use lankide_core::ResourceRef;
use lankide_core::{AgentId, ChannelId, MessageId, PersonId, WorkspaceId};
use lankide_provider::{
    AgentRuntimeSpec, AgentSpec, AutoReactionSpec, ChannelDefaultAttention, HostServices, LogLevel,
    MessageAttachment, OperationProgress, OperationUpdate, Provider, ProviderAgentFile,
    ProviderEvent, ProviderEventKind, ProviderFuture, ProviderHost, ProviderLogger,
    ProviderMessage, ProviderOnboardingStatus, ProviderPersonIdentity, ProviderResource,
    ProviderStorage, ProviderTeamMember, RecallMessageSpec, ResourceSaver, ResourceUpload,
    SecretReader, SecretValue, SendMessageSpec, SendMessageStatus, TeamRole,
};
use lankide_provider_feishu::{
    Auth, Credentials, Domain, EventSource, FeishuClient, FeishuProvider, Method,
    NormalizationConfig, NormalizedEvent, RawEvent, Request, ReqwestTransport, Response,
    STANDARD_BOT_EVENTS, STANDARD_BOT_TENANT_SCOPES, SourceError, Transport, TransportError,
    authorization_url, extract_agent_ids, normalize_event, render_card_markdown,
};
use lankide_testkit::{
    GateSelection, ProbeConfig, ScenarioArtifact as Artifact, run_prerequisite_probe,
};
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time::timeout;

/// Exercise the Feishu wire contract as part of the complete provider
/// scenario.  These checks intentionally live behind the live gate instead
/// of being separate function-level tests: the acceptance command proves the
/// public normalization/rendering/diagnostic behavior together with the real
/// Feishu API flow below.
fn exercise_feishu_contract() -> Result<(), String> {
    let authorization = authorization_url(&Domain::Feishu, "cli_e2e").map_err(|e| e.to_string())?;
    let authorization = url::Url::parse(&authorization).map_err(|e| e.to_string())?;
    let query = authorization
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    let compressed = URL_SAFE_NO_PAD
        .decode(
            query
                .get("addons")
                .ok_or("authorization manifest missing")?
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut raw_manifest = String::new();
    decoder
        .read_to_string(&mut raw_manifest)
        .map_err(|e| e.to_string())?;
    let manifest: Value = serde_json::from_str(&raw_manifest).map_err(|e| e.to_string())?;
    if authorization.host_str() != Some("open.feishu.cn")
        || query.get("clientID").map(|value| value.as_ref()) != Some("cli_e2e")
        || manifest["scopes"]["tenant"].as_array().map(Vec::len) != Some(25)
        || manifest["events"]["items"]["tenant"]
            .as_array()
            .map(Vec::len)
            != Some(5)
    {
        return Err("Feishu installation link does not request the product capabilities".into());
    }
    let content = "请看 @Alpha[agent_alpha] 和 @Unknown[unknown]";
    if extract_agent_ids(content) != vec!["agent_alpha", "unknown"] {
        return Err("Feishu mention marker extraction changed".to_owned());
    }
    let (rendered, mentions) = render_card_markdown(
        content,
        &[lankide_provider_feishu::MentionTarget::new(
            "agent_alpha",
            "ou_e2e",
            "E2E Agent",
        )],
    );
    if !rendered.contains("<at id=ou_e2e></at>")
        || !rendered.contains("@Unknown[unknown]")
        || mentions.len() != 2
    {
        return Err("Feishu Markdown mention rendering changed".to_owned());
    }

    let raw = json!({
        "header": {
            "app_id": "cli_e2e",
            "event_id": "callback-1",
            "event_type": "im.message.receive_v1",
            "create_time": "1700000000000",
            "tenant_key": "tenant-e2e"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {"open_id": "ou_human"}
            },
            "message": {
                "message_id": "om_e2e",
                "parent_id": "om_parent",
                "chat_id": "oc_e2e",
                "chat_type": "group",
                "message_type": "text",
                "content": "{\"text\":\"hello @_user_1\"}",
                "create_time": "1700000000000",
                "mentions": [{
                    "key": "@_user_1",
                    "id": {"open_id": "ou_agent"},
                    "name": "E2E Agent",
                    "mentioned_type": "User"
                }]
            }
        }
    })
    .to_string();
    let config = NormalizationConfig::new("cli_e2e").with_agent("ou_agent", "agent_e2e");
    let first = normalize_event(raw.as_bytes(), &config)
        .map_err(|_| "Feishu message normalization rejected a valid event".to_owned())?;
    let second = normalize_event(raw.as_bytes(), &config)
        .map_err(|_| "Feishu message normalization was not repeatable".to_owned())?;
    if first != second {
        return Err("Feishu message normalization is not stable".to_owned());
    }
    let NormalizedEvent::Message(message) = first else {
        return Err("Feishu normalization did not produce a message".to_owned());
    };
    if message.provider_message_id != "om_e2e"
        || message.event_id != "im.message.receive:om_e2e"
        || message.reply_to_provider_message_id.as_deref() != Some("om_parent")
        || !message.mentioned_agent_ids.is_empty()
        || message.content != "hello @E2E Agent[ou_agent]"
        || message.mentions.first().is_none_or(|mention| {
            mention.kind != lankide_provider_feishu::SenderKind::Human || mention.agent_id.is_some()
        })
    {
        return Err("Feishu message identity, reply, or mention mapping changed".to_owned());
    }

    let nullable_raw = json!({
        "header": {
            "app_id": "cli_e2e",
            "event_id": "callback-null-fields",
            "event_type": "im.message.receive_v1",
            "create_time": null,
            "tenant_key": null
        },
        "event": {
            "sender": {
                "sender_type": null,
                "sender_id": null
            },
            "message": {
                "message_id": "om_nullable",
                "root_id": null,
                "parent_id": null,
                "thread_id": null,
                "chat_id": "oc_e2e",
                "chat_type": "group",
                "message_type": "text",
                "content": "{\"text\":\"nullable\"}",
                "create_time": null,
                "update_time": null,
                "mentions": null
            }
        }
    })
    .to_string();
    let nullable_event = normalize_event(
        nullable_raw.as_bytes(),
        &NormalizationConfig::new("cli_e2e"),
    )
    .map_err(|_| "Feishu nullable optional fields no longer decode".to_owned())?;
    let NormalizedEvent::Message(nullable_message) = nullable_event else {
        return Err("Feishu nullable event did not produce a message".to_owned());
    };
    if nullable_message.provider_message_id != "om_nullable"
        || nullable_message.reply_to_provider_message_id.is_some()
        || nullable_message.thread_id.is_some()
        || !nullable_message.mentions.is_empty()
    {
        return Err("Feishu nullable optional fields were not normalized safely".to_owned());
    }

    let missing_scope = r#"{"type":"authorization","missing_scopes":["im:message.p2p_msg:get_as_user","im:message.reactions:read"]}"#;
    if safe_cli_diagnostic(missing_scope)
        != "Feishu user OAuth fixture is missing required scope(s): im:message.p2p_msg:get_as_user, im:message.reactions:read"
    {
        return Err("Feishu missing-scope diagnostic changed".to_owned());
    }
    if safe_cli_diagnostic("authorization token: do-not-record")
        != "CLI diagnostic omitted because it contained sensitive credential text"
    {
        return Err("Feishu diagnostic secret redaction changed".to_owned());
    }
    Ok(())
}

#[tokio::test]
async fn feishu_provider_exposes_im_reaction_and_optional_task_capabilities() {
    exercise_feishu_contract().expect("Feishu public contract");
    let refreshing_transport = RefreshingTokenTransport::default();
    let client = FeishuClient::new(
        refreshing_transport.clone(),
        Credentials::app("refresh-app", "refresh-secret"),
    )
    .expect("refreshing Feishu client");
    client.bot_info(Auth::Tenant).expect("retry expired token");
    assert_eq!(
        refreshing_transport.token_requests.load(Ordering::SeqCst),
        2
    );
    assert_eq!(refreshing_transport.bot_requests.load(Ordering::SeqCst), 2);
    let agent_id = AgentId::new("agent_e2e").expect("Agent id");
    let storage = Arc::new(MemoryStorage::default());
    storage.insert_agent(agent_id.as_str());
    let mut lead = storage
        .get_or_create_person("on_human_e2e")
        .await
        .expect("Team Lead");
    lead.team_role = TeamRole::Lead;
    lead.provider_state = json!({
        "external_id_type":"union_id",
        "open_ids_by_app":{"cli_e2e":"ou_human_e2e"}
    });
    storage
        .people
        .lock()
        .expect("person storage")
        .insert(lead.external_id.clone(), lead);
    storage
        .put_workspace_state(json!({
            "tasklist_guid": "tasklist_e2e",
            "identity_directory": {
                "agents": {
                    "agent_e2e": {
                        "app_id": "cli_e2e",
                        "tenant_key": "tenant_e2e",
                        "open_ids_by_app": {"cli_e2e": "ou_agent_e2e"},
                        "display_name": "E2E Agent"
                    }
                }
            }
        }))
        .await
        .expect("workspace Provider state");
    let host = ProviderHost::new(
        FeishuProvider::new()
            .without_event_source()
            .with_transport(FeishuFixtureTransport),
        WorkspaceId::new("feishu_capabilities").expect("workspace id"),
        json!({"task_system_enabled": true}),
        live_services_with_storage(
            "fixture-secret".into(),
            Arc::new(LiveLogger::default()),
            storage.clone(),
        ),
    )
    .expect("Feishu ProviderHost");
    let reactions = host.reaction_catalog();
    assert_eq!(reactions.len(), 279);
    assert!(reactions.iter().any(|reaction| reaction.id == "GLANCE"));
    assert!(reactions.iter().any(|reaction| reaction.id == "ClownFace"));
    assert!(reactions.iter().any(|reaction| reaction.id == "Shrug"));
    assert!(
        reactions
            .iter()
            .all(|reaction| !reaction.id.is_empty() && !reaction.name.is_empty())
    );
    let glance = reactions
        .iter()
        .find(|reaction| reaction.id == "GLANCE")
        .expect("GLANCE reaction");
    assert_eq!(glance.name, "看");
    let channels = host.channels().await.expect("Feishu channels");
    assert!(channels.iter().any(|channel| {
        channel.id.as_str() == "group_chat"
            && channel.default_attention == ChannelDefaultAttention::Follow
    }));
    host.init().await.expect("Feishu Provider init");
    host.create_agent(
        agent_id.clone(),
        AgentSpec {
            name: "E2E Agent".into(),
            role: "QA".into(),
            bio: String::new(),
            avatar: None,
            soul: String::new(),
            computer_id: "local".into(),
            harness_id: "pi".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            auto_reaction: None,
            cli_path: None,
            environment_variables: Vec::new(),
            provider_agent_config: json!({
                "mode": "existing",
                "app_id": "cli_e2e",
                "domain": "feishu"
            }),
        },
    )
    .await
    .expect("Feishu Agent connection");
    host.create_agent(
        agent_id.clone(),
        AgentSpec {
            name: "E2E Agent".into(),
            role: "QA".into(),
            bio: String::new(),
            avatar: None,
            soul: String::new(),
            computer_id: "local".into(),
            harness_id: "pi".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            auto_reaction: None,
            cli_path: None,
            environment_variables: Vec::new(),
            provider_agent_config: json!({"mode":"new","app_id":""}),
        },
    )
    .await
    .expect("Feishu Agent connection recovers verified app identity");
    let prompt = host
        .system_prompt(agent_id.clone(), standalone_runtime_spec("cli_e2e"))
        .await
        .expect("Feishu system prompt");
    assert!(prompt.contains("飞书 IM"));
    assert!(prompt.contains("飞书任务"));
    assert!(prompt.contains("负责人"));
    assert!(!prompt.contains("Runtime"));
    assert!(!prompt.contains("Provider"));

    let tools = host
        .tools(agent_id, standalone_runtime_spec("cli_e2e"))
        .await
        .expect("Feishu Agent tools");
    let tool_ids = tools
        .iter()
        .map(|tool| tool.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for expected in [
        "feishu_reaction_catalog",
        "feishu_reaction_add",
        "feishu_reaction_list",
        "feishu_reaction_remove",
        "feishu_task_create",
        "feishu_task_get",
        "feishu_task_list",
        "feishu_task_update",
        "feishu_task_member_add",
        "feishu_task_member_remove",
        "feishu_task_complete",
        "feishu_task_comments",
        "feishu_task_comment",
        "feishu_person_profile",
    ] {
        assert!(tool_ids.contains(expected), "missing Agent tool {expected}");
    }
    let reaction_catalog = tools
        .iter()
        .find(|tool| tool.id == "feishu_reaction_catalog")
        .expect("reaction catalog tool")
        .handler
        .invoke(json!({"query":"GLANCE"}))
        .await
        .expect("reaction catalog result");
    assert_eq!(reaction_catalog["items"][0]["emoji_type"], "GLANCE");
    assert_eq!(
        reaction_catalog["items"][0]["description"],
        "看。灰黑色猫脸，眼睛向一侧瞥，和经典的狗头差不多"
    );
    let mut person = storage
        .get_or_create_person("on_profile")
        .await
        .expect("profile person");
    person.team_role = TeamRole::Member;
    person.provider_state = json!({"external_id_type":"union_id"});
    storage
        .people
        .lock()
        .expect("person storage")
        .insert("on_profile".to_owned(), person.clone());
    let profile = tools
        .iter()
        .find(|tool| tool.id == "feishu_person_profile")
        .expect("profile tool")
        .handler
        .invoke(json!({"person_id":person.person_id}))
        .await
        .expect("profile result");
    assert_eq!(profile["name"], "Profile Nickname");
    assert_eq!(profile["signature"], "Profile signature");
    assert_eq!(profile["team_role"], "member");
    assert!(profile["avatar_path"].as_str().unwrap().ends_with(".jpg"));
}

#[tokio::test]
async fn feishu_task_flow_projects_the_typed_team_roster() {
    let agent_id = AgentId::new("alpha").expect("Agent id");
    let other_agent_id = AgentId::new("beta").expect("Agent id");
    let nonmember_agent_id = AgentId::new("gamma").expect("Agent id");
    let storage = Arc::new(MemoryStorage::default());
    storage.insert_agent(agent_id.as_str());
    storage.insert_agent(other_agent_id.as_str());
    storage.insert_agent(nonmember_agent_id.as_str());

    let mut lead = storage
        .get_or_create_person("on_lead")
        .await
        .expect("Team Lead");
    lead.team_role = TeamRole::Lead;
    lead.provider_state = json!({
        "external_id_type":"union_id",
        "open_ids_by_app":{
            "cli_alpha":"ou_lead",
            "cli_beta":"ou_lead_beta",
            "cli_gamma":"ou_lead_gamma"
        }
    });
    let lead_id = lead.person_id.clone();
    let mut teammate = storage
        .get_or_create_person("on_teammate")
        .await
        .expect("Team member");
    teammate.team_role = TeamRole::Member;
    teammate.provider_state = json!({
        "external_id_type":"union_id",
        "open_ids_by_app":{
            "cli_alpha":"ou_teammate",
            "cli_beta":"ou_teammate_beta",
            "cli_gamma":"ou_teammate_gamma"
        }
    });
    let teammate_id = teammate.person_id.clone();
    let mut external = storage
        .get_or_create_person("on_external")
        .await
        .expect("external person");
    external.provider_state = json!({
        "external_id_type":"union_id",
        "open_ids_by_app":{"cli_alpha":"ou_external"}
    });
    {
        let mut people = storage.people.lock().expect("person storage");
        people.insert(lead.external_id.clone(), lead);
        people.insert(teammate.external_id.clone(), teammate);
        people.insert(external.external_id.clone(), external);
    }
    storage
        .put_workspace_state(json!({
            "identity_directory":{"agents":{
                "beta":{
                    "app_id":"cli_beta",
                    "tenant_key":"tenant_flow",
                    "open_ids_by_app":{"cli_alpha":"ou_beta"},
                    "display_name":"Beta"
                },
                "gamma":{
                    "app_id":"cli_gamma",
                    "tenant_key":"tenant_flow",
                    "open_ids_by_app":{"cli_alpha":"ou_gamma","cli_beta":"ou_gamma_beta"},
                    "display_name":"Gamma"
                }
            }}
        }))
        .await
        .expect("workspace state");

    let transport = TaskFlowTransport::default();
    let host = ProviderHost::new(
        FeishuProvider::new()
            .without_event_source()
            .with_transport(transport.clone()),
        WorkspaceId::new("task_flow").expect("workspace id"),
        json!({"task_system_enabled":true,"task_list_name":"团队任务"}),
        live_services_with_storage(
            "fixture-secret".into(),
            Arc::new(LiveLogger::default()),
            storage.clone(),
        ),
    )
    .expect("Feishu ProviderHost");
    host.init().await.expect("Feishu Provider init");
    host.create_agent(agent_id.clone(), fixture_agent_spec("Alpha", "cli_alpha"))
        .await
        .expect("Feishu Agent connection");
    host.create_agent(
        other_agent_id.clone(),
        fixture_agent_spec("Beta", "cli_beta"),
    )
    .await
    .expect("second Feishu Agent connection");
    host.create_agent(
        nonmember_agent_id.clone(),
        fixture_agent_spec("Gamma", "cli_gamma"),
    )
    .await
    .expect("third Feishu Agent connection");

    let runtime = standalone_runtime_spec("cli_alpha");
    let created = host
        .invoke_tool_in_context(
            agent_id.clone(),
            runtime.clone(),
            "feishu_task_create",
            json!({
                "summary":"Typed roster task",
                "assignee_ids":[other_agent_id.as_str()],
                "follower_ids":[teammate_id.as_str()]
            }),
        )
        .await
        .expect("create task");
    assert_eq!(created["ok"], true);
    let task_id = created["task"]["task_id"]
        .as_str()
        .expect("created task id")
        .to_owned();
    let assignees = created["task"]["assignee_ids"]
        .as_array()
        .expect("assignees");
    assert!(assignees.iter().any(|value| value == lead_id.as_str()));
    assert!(assignees.iter().any(|value| value == agent_id.as_str()));
    assert!(
        assignees
            .iter()
            .any(|value| value == other_agent_id.as_str())
    );

    let added = host
        .invoke_tool_in_context(
            agent_id.clone(),
            runtime.clone(),
            "feishu_task_member_add",
            json!({
                "task_id":task_id,
                "member_ids":[nonmember_agent_id.as_str()],
                "role":"assignee"
            }),
        )
        .await
        .expect("add task assignee");
    assert_eq!(added["ok"], true, "add task assignee: {added}");
    assert!(
        added["task"]["assignee_ids"]
            .as_array()
            .is_some_and(|values| values
                .iter()
                .any(|value| value == nonmember_agent_id.as_str()))
    );

    let conflicting = host
        .invoke_tool_in_context(
            agent_id.clone(),
            runtime.clone(),
            "feishu_task_member_add",
            json!({
                "task_id":task_id,
                "member_ids":[teammate_id.as_str()],
                "role":"assignee"
            }),
        )
        .await
        .expect("reject conflicting task role");
    assert_eq!(conflicting["error"]["code"], "duplicate_task_role");

    let removed = host
        .invoke_tool_in_context(
            agent_id.clone(),
            runtime.clone(),
            "feishu_task_member_remove",
            json!({
                "task_id":task_id,
                "member_ids":[nonmember_agent_id.as_str()],
                "role":"assignee"
            }),
        )
        .await
        .expect("remove task assignee");
    assert_eq!(removed["ok"], true, "remove task assignee: {removed}");
    assert!(
        removed["task"]["assignee_ids"]
            .as_array()
            .is_some_and(|values| values
                .iter()
                .all(|value| value != nonmember_agent_id.as_str()))
    );

    let second = host
        .invoke_tool_in_context(
            agent_id.clone(),
            runtime.clone(),
            "feishu_task_create",
            json!({"summary":"Second task"}),
        )
        .await
        .expect("create second task");
    assert_ne!(second["task"]["task_id"], task_id);

    let followed = host
        .invoke_tool_in_context(
            AgentId::new("alpha").unwrap(),
            standalone_runtime_spec("cli_alpha"),
            "feishu_task_create",
            json!({"summary":"Follower task","follower_ids":["beta"]}),
        )
        .await
        .expect("create follower task");
    let followed_task_id = followed["task"]["task_id"]
        .as_str()
        .expect("follower task id")
        .to_owned();
    let follower_read = host
        .invoke_tool_in_context(
            other_agent_id.clone(),
            standalone_runtime_spec("cli_beta"),
            "feishu_task_get",
            json!({"task_id":followed_task_id}),
        )
        .await
        .expect("follower read");
    let follower_write = host
        .invoke_tool_in_context(
            other_agent_id.clone(),
            standalone_runtime_spec("cli_beta"),
            "feishu_task_update",
            json!({"task_id":followed_task_id,"summary":"forbidden"}),
        )
        .await
        .expect("follower write refusal");
    let follower_member_write = host
        .invoke_tool_in_context(
            other_agent_id.clone(),
            standalone_runtime_spec("cli_beta"),
            "feishu_task_member_add",
            json!({"task_id":followed_task_id,"member_ids":["gamma"],"role":"assignee"}),
        )
        .await
        .expect("follower member write refusal");
    assert_eq!(follower_read["ok"], true, "follower read: {follower_read}");
    assert_eq!(follower_write["error"]["code"], "not_assignee");
    assert_eq!(follower_member_write["error"]["code"], "not_assignee");
    let nonmember_read = host
        .invoke_tool_in_context(
            nonmember_agent_id,
            standalone_runtime_spec("cli_gamma"),
            "feishu_task_get",
            json!({"task_id":followed_task_id}),
        )
        .await
        .expect("nonmember read refusal");
    assert_eq!(nonmember_read["error"]["code"], "not_task_member");

    let comment = host
        .invoke_tool_in_context(
            agent_id.clone(),
            runtime.clone(),
            "feishu_task_comment",
            json!({
                "task_id":task_id,
                "content":format!("@Lead[{}] 和 @Beta[beta] 请确认", lead_id)
            }),
        )
        .await
        .expect("create task comment");
    assert_eq!(comment["ok"], true);

    for content in [
        "<at id=ou_lead></at>",
        "<at id=alpha></at>",
        "<AT id=\"ou_lead\"></AT>",
        "@Nobody[unknown]",
    ] {
        let rejected = host
            .invoke_tool_in_context(
                agent_id.clone(),
                runtime.clone(),
                "feishu_task_comment",
                json!({"task_id":task_id,"content":content}),
            )
            .await
            .expect("tool rejection");
        assert_ne!(
            rejected["ok"], true,
            "invalid mention must not be sent: {content}"
        );
    }
    for tool in ["feishu_task_get", "feishu_task_comments"] {
        let read = host
            .invoke_tool_in_context(
                agent_id.clone(),
                runtime.clone(),
                tool,
                json!({"task_id":task_id}),
            )
            .await
            .expect("read normalized comments");
        let content = read["comments"][0]["content"]
            .as_str()
            .expect("comment content");
        assert_eq!(
            content,
            format!("@成员[{}] @Gamma[gamma] @未识别成员 请确认", lead_id)
        );
        assert!(read["comments"][1]["creator"].is_null());
        assert!(!read["comments"].to_string().contains("ou_"));
        assert!(!read["comments"].to_string().contains("cli_"));
        if tool == "feishu_task_get" {
            host.invoke_tool_in_context(
                agent_id.clone(),
                runtime.clone(),
                "feishu_task_comment",
                json!({"task_id":task_id,"content":content}),
            )
            .await
            .expect("reply using normalized comment");
        }
    }

    assert_eq!(
        storage
            .workspace_state()
            .await
            .expect("persisted workspace state")["tasklist_guid"],
        "tasklist_flow"
    );

    let state = transport.state.lock().expect("task flow state");
    assert_eq!(state.comment_bodies.len(), 2);
    assert_eq!(
        state.comment_bodies[1]["content"],
        "<at id=ou_lead></at> <at id=ou_gamma></at> @未识别成员 请确认"
    );
    assert_eq!(state.create_bodies.len(), 3);
    let first_token = state.create_bodies[0]["client_token"]
        .as_str()
        .expect("first client token");
    let second_token = state.create_bodies[1]["client_token"]
        .as_str()
        .expect("second client token");
    assert_ne!(first_token, second_token);
    let acl = state
        .tasklist_create_bodies
        .first()
        .and_then(|body| body["members"].as_array())
        .expect("Tasklist ACL");
    for (id, member_type, role) in [
        ("cli_beta", "app", "editor"),
        ("ou_lead", "user", "viewer"),
        ("ou_teammate", "user", "viewer"),
    ] {
        assert!(acl.iter().any(|member| {
            member["id"] == id && member["type"] == member_type && member["role"] == role
        }));
    }
    assert!(acl.iter().all(|member| member["id"] != "ou_external"));
    assert_eq!(state.task_member_add_bodies.len(), 1);
    assert_eq!(
        state.task_member_add_bodies[0]["members"],
        json!([{"id":"cli_gamma","type":"app","role":"assignee"}])
    );
    assert_eq!(state.task_member_remove_bodies.len(), 1);
    assert_eq!(
        state.task_member_remove_bodies[0]["members"],
        json!([{"id":"cli_gamma","type":"app","role":"assignee"}])
    );
    assert_eq!(
        state.comment_bodies[0]["content"],
        "<at id=ou_lead></at> 和 <at id=ou_beta></at> 请确认"
    );
}

#[tokio::test]
async fn existing_agent_onboarding_repairs_missing_published_permissions() {
    let transport = AuthorizationUpgradeTransport::default();
    let host = ProviderHost::new(
        FeishuProvider::new()
            .without_event_source()
            .with_transport(transport.clone()),
        WorkspaceId::new("feishu_authorization_upgrade").expect("workspace id"),
        json!({"domain":"feishu"}),
        live_services("fixture-secret".into(), Arc::new(LiveLogger::default())),
    )
    .expect("Feishu ProviderHost");
    let agent_id = AgentId::new("agent_e2e").expect("Agent id");
    let config = json!({
        "mode": "existing",
        "app_id": "cli_authorization_upgrade",
        "domain": "feishu"
    });

    let current = host
        .agent_onboarding(
            agent_id.clone(),
            "E2E Agent".into(),
            config.clone(),
            "get".into(),
            json!({}),
        )
        .await
        .expect("inspect existing Agent permissions");
    assert_eq!(current.step, "authorization_update");
    assert_eq!(current.verification_uri, None);
    assert_eq!(current.poll_interval_seconds, Some(5));
    assert_eq!(current.status, ProviderOnboardingStatus::ActionRequired);
    assert_eq!(transport.config_updates.load(Ordering::SeqCst), 0);
    assert_eq!(transport.publishes.load(Ordering::SeqCst), 0);

    let updating = host
        .agent_onboarding(
            agent_id.clone(),
            "E2E Agent".into(),
            config.clone(),
            "start".into(),
            json!({}),
        )
        .await
        .expect("update existing Agent permissions");
    assert_eq!(updating.step, "authorization_update");
    assert_eq!(updating.poll_interval_seconds, Some(5));
    assert_eq!(updating.status, ProviderOnboardingStatus::Verifying);
    assert_eq!(transport.config_updates.load(Ordering::SeqCst), 1);
    assert_eq!(transport.publishes.load(Ordering::SeqCst), 1);

    let ready = host
        .agent_onboarding(
            agent_id,
            "E2E Agent".into(),
            config,
            "poll".into(),
            updating.opaque_payload,
        )
        .await
        .expect("confirm updated existing Agent permissions");
    assert_eq!(ready.status, ProviderOnboardingStatus::Ready);
}

#[tokio::test]
async fn existing_agent_onboarding_falls_back_to_owner_confirmation() {
    let transport = AuthorizationUpgradeTransport::default();
    transport.reject_updates.store(true, Ordering::SeqCst);
    let host = ProviderHost::new(
        FeishuProvider::new()
            .without_event_source()
            .with_transport(transport.clone()),
        WorkspaceId::new("feishu_authorization_confirmation").expect("workspace id"),
        json!({"domain":"feishu"}),
        live_services("fixture-secret".into(), Arc::new(LiveLogger::default())),
    )
    .expect("Feishu ProviderHost");
    let agent_id = AgentId::new("agent_e2e").expect("Agent id");
    let config = json!({
        "mode": "existing",
        "app_id": "cli_authorization_confirmation",
        "domain": "feishu"
    });

    let confirmation = host
        .agent_onboarding(
            agent_id.clone(),
            "E2E Agent".into(),
            config.clone(),
            "start".into(),
            json!({}),
        )
        .await
        .expect("fall back to owner confirmation");
    assert_eq!(confirmation.step, "authorization_update");
    assert_eq!(confirmation.poll_interval_seconds, Some(5));
    assert_eq!(
        confirmation.verification_uri.as_deref(),
        confirmation.opaque_payload["verification_uri"].as_str()
    );
    assert_eq!(
        confirmation.status,
        ProviderOnboardingStatus::ActionRequired
    );
    assert!(
        confirmation.opaque_payload["verification_uri"]
            .as_str()
            .is_some_and(|link| link.contains("clientID=cli_authorization_confirmation"))
    );
    assert_eq!(transport.config_updates.load(Ordering::SeqCst), 1);
    assert_eq!(transport.publishes.load(Ordering::SeqCst), 0);

    let waiting = host
        .agent_onboarding(
            agent_id.clone(),
            "E2E Agent".into(),
            config.clone(),
            "poll".into(),
            confirmation.opaque_payload,
        )
        .await
        .expect("keep owner confirmation available while waiting");
    assert_eq!(waiting.status, ProviderOnboardingStatus::ActionRequired);
    assert_eq!(
        waiting.verification_uri.as_deref(),
        waiting.opaque_payload["verification_uri"].as_str()
    );
    assert_eq!(waiting.poll_interval_seconds, Some(5));

    transport.authorized.store(true, Ordering::SeqCst);
    let ready = host
        .agent_onboarding(
            agent_id,
            "E2E Agent".into(),
            config,
            "poll".into(),
            waiting.opaque_payload,
        )
        .await
        .expect("observe owner-confirmed permissions");
    assert_eq!(ready.status, ProviderOnboardingStatus::Ready);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_provider_reconciles_one_agent_across_app_scoped_open_ids() {
    let app_a_events = Arc::new(Mutex::new(VecDeque::new()));
    let app_b_events = Arc::new(Mutex::new(VecDeque::new()));
    let queues = BTreeMap::from([
        ("app_a".to_owned(), app_a_events.clone()),
        ("app_b".to_owned(), app_b_events.clone()),
    ]);
    let provider = FeishuProvider::new()
        .with_transport(FeishuIdentityTransport)
        .with_event_source_factory(move |_, credentials, _| {
            let queue = queues
                .get(&credentials.app_id)
                .cloned()
                .ok_or_else(|| SourceError::Failed("unknown fixture App".into()))?;
            Ok(Box::new(SharedQueueSource { queue }))
        });
    let storage = Arc::new(MemoryStorage::default());
    let resource_saver = Arc::new(LiveResourceSaver::default());
    let mut services = live_services_with_storage(
        "fixture-secret".into(),
        Arc::new(LiveLogger::default()),
        storage.clone(),
    );
    services.resource_saver = resource_saver.clone();
    let host = ProviderHost::new(
        provider,
        WorkspaceId::new("feishu_identity").expect("workspace id"),
        json!({"task_system_enabled": false}),
        services,
    )
    .expect("Feishu ProviderHost");
    host.init().await.expect("Feishu Provider init");
    for (id, name, app_id) in [("alpha", "Alpha", "app_a"), ("beta", "Beta", "app_b")] {
        host.create_agent(
            AgentId::new(id).expect("Agent id"),
            AgentSpec {
                name: name.into(),
                role: String::new(),
                bio: String::new(),
                avatar: None,
                soul: String::new(),
                computer_id: "local".into(),
                harness_id: "pi".into(),
                model: "fixture".into(),
                thinking_level: "low".into(),
                auto_reaction: None,
                cli_path: None,
                environment_variables: Vec::new(),
                provider_agent_config: json!({"mode":"existing","app_id":app_id}),
            },
        )
        .await
        .expect("Feishu Agent connection");
    }

    app_a_events
        .lock()
        .expect("App A queue")
        .push_back(RawEvent::new("task.task.updated_v1", b"{}".to_vec()));
    app_a_events
        .lock()
        .expect("App A queue")
        .push_back(identity_message(
            "app_a",
            "identity-a-first",
            "ou_beta_seen_by_a",
            "on_beta",
        ));
    let first = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("first identity event timeout")
        .expect("first identity event");
    let (first, acknowledgement) = first.into_parts();
    acknowledgement.accept();
    assert!(message_mentions(&first).is_empty());
    let human_person_id = first.sender.id.clone();
    assert!(PersonId::new(human_person_id.clone()).is_ok());

    app_b_events
        .lock()
        .expect("App B queue")
        .push_back(identity_message(
            "app_b",
            "identity-b-own",
            "ou_beta_own",
            "on_beta",
        ));
    let second = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("second identity event timeout")
        .expect("second identity event");
    let (second, acknowledgement) = second.into_parts();
    acknowledgement.accept();
    assert_eq!(message_mentions(&second), vec!["beta"]);
    assert_eq!(second.sender.id, human_person_id);
    let workspace_state = storage
        .workspace_state()
        .await
        .expect("workspace identity state after cross-App observation");
    assert_eq!(
        workspace_state["identity_directory"]["agents"]["beta"]["open_ids_by_app"]["app_a"],
        "ou_beta_seen_by_a",
        "the first App's unresolved Bot mention must reconcile as soon as the owning App identifies it"
    );

    app_a_events
        .lock()
        .expect("App A queue")
        .push_back(identity_message(
            "app_a",
            "identity-a-resolved",
            "ou_beta_seen_by_a",
            "on_beta",
        ));
    let resolved = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("resolved identity event timeout")
        .expect("resolved identity event");
    let (resolved, acknowledgement) = resolved.into_parts();
    acknowledgement.accept();
    assert_eq!(message_mentions(&resolved), vec!["beta"]);
    let human = storage
        .people
        .lock()
        .expect("person storage")
        .get("on_human")
        .cloned()
        .expect("union identity");
    assert_eq!(human.person_id.as_str(), human_person_id);
    assert_eq!(
        human.provider_state["open_ids_by_app"],
        json!({"app_a":"ou_human_app_a","app_b":"ou_human_app_b"})
    );

    app_a_events
        .lock()
        .expect("App A queue")
        .push_back(identity_bot_sender_message(
            "app_a",
            "identity-sender-a",
            "ou_alpha_own",
            "on_alpha",
        ));
    let owning_sender = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("owning App sender event timeout")
        .expect("owning App sender event");
    let (owning_sender, acknowledgement) = owning_sender.into_parts();
    acknowledgement.accept();
    assert_eq!(owning_sender.sender.id, "alpha");
    app_b_events
        .lock()
        .expect("App B queue")
        .push_back(identity_bot_sender_message(
            "app_b",
            "identity-sender-b",
            "ou_alpha_seen_by_b",
            "on_alpha",
        ));
    let observed_sender = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("cross-App sender event timeout")
        .expect("cross-App sender event");
    let (observed_sender, acknowledgement) = observed_sender.into_parts();
    acknowledgement.accept();
    assert_eq!(observed_sender.sender.id, "alpha");
    let sender_state = storage
        .workspace_state()
        .await
        .expect("workspace identity state after sender observations");
    assert_eq!(
        sender_state["identity_directory"]["agents"]["alpha"]["open_ids_by_app"]["app_b"],
        "ou_alpha_seen_by_b",
        "bot sender callbacks must populate the cross-App mention identity"
    );

    app_a_events
        .lock()
        .expect("App A queue")
        .push_back(identity_attachment_message("app_a", "attachment-a"));
    app_b_events
        .lock()
        .expect("App B queue")
        .push_back(identity_attachment_message("app_b", "attachment-b"));
    let mut resource_ids = Vec::new();
    for _ in 0..2 {
        let event = timeout(Duration::from_secs(2), host.next_event())
            .await
            .expect("shared attachment event timeout")
            .expect("shared attachment event");
        let (event, acknowledgement) = event.into_parts();
        acknowledgement.accept();
        let ProviderEventKind::Message(message) = event.kind else {
            panic!("shared attachment did not produce a message");
        };
        assert_eq!(message.provider_message_id, "shared-attachment");
        resource_ids.push(message.attachments[0].resource_id);
    }
    assert_eq!(resource_ids[0], resource_ids[1]);
    assert_eq!(
        resource_saver
            .uploads
            .lock()
            .expect("resource uploads")
            .len(),
        1,
        "the same Feishu resource observed by two Apps must be saved once"
    );
    host.shutdown().await.expect("Feishu Provider shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_provider_does_not_block_later_events_when_an_attachment_is_unavailable() {
    let events = Arc::new(Mutex::new(VecDeque::new()));
    let source_events = events.clone();
    let provider = FeishuProvider::new()
        .with_transport(FeishuFixtureTransport)
        .with_event_source_factory(move |_, _, _| {
            Ok(Box::new(SharedQueueSource {
                queue: source_events.clone(),
            }))
        });
    let logger = Arc::new(LiveLogger::default());
    let host = ProviderHost::new(
        provider,
        WorkspaceId::new("feishu_attachment_failure").expect("workspace id"),
        json!({"task_system_enabled": false}),
        live_services("fixture-secret".into(), logger.clone()),
    )
    .expect("Feishu ProviderHost");
    host.init().await.expect("Feishu Provider init");
    host.create_agent(
        AgentId::new("agent_e2e").expect("Agent id"),
        AgentSpec {
            name: "E2E Agent".into(),
            role: String::new(),
            bio: String::new(),
            avatar: None,
            soul: String::new(),
            computer_id: "local".into(),
            harness_id: "pi".into(),
            model: "fixture".into(),
            thinking_level: "low".into(),
            auto_reaction: None,
            cli_path: None,
            environment_variables: Vec::new(),
            provider_agent_config: json!({"mode":"existing","app_id":"cli_e2e"}),
        },
    )
    .await
    .expect("Feishu Agent connection");

    let raw_message = |event_id: &str, message_type: &str, content: Value| {
        RawEvent::new(
            "im.message.receive_v1",
            json!({
                "header": {
                    "app_id": "cli_e2e",
                    "event_id": event_id,
                    "event_type": "im.message.receive_v1",
                    "create_time": "1700000000000",
                    "tenant_key": "tenant_e2e"
                },
                "event": {
                    "sender": {
                        "sender_type": "user",
                        "sender_id": {"open_id": "ou_human"}
                    },
                    "message": {
                        "message_id": event_id,
                        "chat_id": "oc_attachment_failure",
                        "chat_type": "group",
                        "message_type": message_type,
                        "content": content.to_string(),
                        "create_time": "1700000000000",
                        "mentions": []
                    }
                }
            })
            .to_string(),
        )
    };
    {
        let mut queue = events.lock().expect("event queue");
        queue.push_back(raw_message(
            "attachment-unavailable",
            "image",
            json!({"image_key":"img_unavailable"}),
        ));
        queue.push_back(raw_message(
            "message-after-attachment",
            "text",
            json!({"text":"still delivered"}),
        ));
    }

    let first = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("attachment event timeout")
        .expect("attachment event");
    let (first, acknowledgement) = first.into_parts();
    acknowledgement.accept();
    assert_eq!(first.sender.name, "Human Nickname");
    match first.kind {
        ProviderEventKind::Message(message) => {
            assert_eq!(message.provider_message_id, "attachment-unavailable");
            assert!(message.attachments.is_empty());
        }
        other => panic!("unexpected first event: {other:?}"),
    }
    let second = timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("event after attachment timeout")
        .expect("event after attachment");
    let (second, acknowledgement) = second.into_parts();
    acknowledgement.accept();
    match second.kind {
        ProviderEventKind::Message(message) => {
            assert_eq!(message.provider_message_id, "message-after-attachment");
            assert_eq!(message.content, "still delivered");
        }
        other => panic!("unexpected second event: {other:?}"),
    }
    assert!(
        logger
            .messages
            .lock()
            .expect("logger")
            .iter()
            .any(|message| message.contains("accepted without that attachment"))
    );
    host.shutdown().await.expect("Feishu Provider shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit LANKIDE_E2E_LIVE=1 and the secure Feishu fixture"]
async fn feishu_live_gate() {
    assert_eq!(env::var("LANKIDE_E2E_LIVE").as_deref(), Ok("1"));
    let run_id = env::var("LANKIDE_E2E_RUN_ID").expect("LANKIDE_E2E_RUN_ID is required");
    if let Err(error) = validate_run_id(&run_id) {
        panic!("invalid live run id: {error}");
    }
    let marker = format!("[lankide-e2e:{run_id}]");
    let artifact = Artifact::new(
        std::path::Path::new("target/e2e-artifacts"),
        "provider-feishu",
        &run_id,
    )
    .expect("create E2E artifact");

    let result = run_feishu_scenario(&run_id, &marker, &artifact).await;
    if let Some(error) = result.as_ref().err() {
        artifact.record_diagnostic("scenario_error", &safe_cli_diagnostic(error));
    }
    artifact.finish(result).expect("Feishu live gate failed");
}

#[derive(Clone, Debug, Deserialize)]
struct BootstrapFile {
    providers: Vec<BootstrapProvider>,
}

#[derive(Clone, Debug, Deserialize)]
struct BootstrapProvider {
    #[serde(rename = "type")]
    provider_type: String,
    app_id: String,
    app_secret: String,
    #[serde(default)]
    domain: String,
}

#[derive(Clone, Debug, Deserialize)]
struct E2EMetadata {
    app_id: String,
    test_chat: E2EChat,
    test_actor: E2EActor,
    agent: E2EAgent,
    task: E2ETask,
}

#[derive(Clone, Debug, Deserialize)]
struct E2EChat {
    chat_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct E2EActor {
    user_open_id: String,
    oauth_config_dir: String,
    cli_source_dir: String,
}

#[derive(Clone, Debug, Deserialize)]
struct E2EAgent {
    bot_open_id: String,
    #[serde(default)]
    display_name: String,
}

#[derive(Clone, Debug, Deserialize)]
struct E2ETask {
    tasklist_guid: String,
}

#[derive(Clone, Debug)]
struct FeishuFixture {
    app_id: String,
    app_secret: String,
    domain: String,
    chat_id: String,
    oauth_dir: PathBuf,
    cli_dir: PathBuf,
    bot_open_id: String,
    bot_name: String,
    user_open_id: String,
    tasklist_guid: String,
}

fn load_fixture() -> Result<FeishuFixture, String> {
    let root = env::var_os("LANKIDE_E2E_TEAM_ROOT")
        .map(PathBuf::from)
        .ok_or_else(|| "LANKIDE_E2E_TEAM_ROOT is required".to_owned())?;
    let root =
        fs::canonicalize(&root).map_err(|_| "Feishu fixture root is unavailable".to_owned())?;
    let main_path = root.join(".lankide/bootstrap/feishu-main.json");
    let metadata_path = root.join(".lankide/bootstrap/feishu-e2e.json");
    let main: BootstrapFile = read_owner_only_json(&main_path)?;
    let metadata: E2EMetadata = read_owner_only_json(&metadata_path)?;
    let provider = main
        .providers
        .into_iter()
        .find(|provider| provider.provider_type == "feishu")
        .ok_or_else(|| "Feishu bootstrap provider is missing".to_owned())?;
    if provider.app_id != metadata.app_id
        || provider.app_id.trim().is_empty()
        || provider.app_secret.trim().is_empty()
        || metadata.test_chat.chat_id.trim().is_empty()
        || metadata.test_actor.user_open_id.trim().is_empty()
        || metadata.agent.bot_open_id.trim().is_empty()
        || metadata.task.tasklist_guid.trim().is_empty()
    {
        return Err("Feishu live fixture fields are incomplete".to_owned());
    }
    let oauth_dir = resolve_fixture_path(&root, &metadata.test_actor.oauth_config_dir);
    let cli_dir = resolve_fixture_path(&root, &metadata.test_actor.cli_source_dir);
    if !oauth_dir.is_dir() || !cli_dir.is_dir() {
        return Err("Feishu OAuth or lark-cli directory is unavailable".to_owned());
    }
    let oauth_mode = fs::metadata(&oauth_dir)
        .map_err(|_| "Feishu OAuth directory metadata is unavailable".to_owned())?
        .permissions()
        .mode();
    if oauth_mode & 0o077 != 0 {
        return Err("Feishu OAuth directory permissions are too broad".to_owned());
    }
    Ok(FeishuFixture {
        app_id: provider.app_id,
        app_secret: provider.app_secret,
        domain: if provider.domain.trim().is_empty() {
            "feishu".to_owned()
        } else {
            provider.domain
        },
        chat_id: metadata.test_chat.chat_id,
        oauth_dir,
        cli_dir,
        bot_open_id: metadata.agent.bot_open_id,
        bot_name: if metadata.agent.display_name.trim().is_empty() {
            "Agent".to_owned()
        } else {
            metadata.agent.display_name
        },
        user_open_id: metadata.test_actor.user_open_id,
        tasklist_guid: metadata.task.tasklist_guid,
    })
}

fn task_runtime_spec(fixture: &FeishuFixture) -> AgentRuntimeSpec {
    AgentRuntimeSpec {
        agent_config: json!({}),
        provider_agent_config: json!({
            "app_id": fixture.app_id,
            "tasklist_guid": fixture.tasklist_guid,
        }),
        computer_id: "computer_e2e".to_owned(),
        harness_id: "harness_e2e".to_owned(),
        model: "model_e2e".to_owned(),
        thinking_level: "off".to_owned(),
        trigger_reason: "feishu_task_e2e".to_owned(),
        chats: Vec::new(),
        unread_snapshot: json!({}),
    }
}

fn live_agent_spec(fixture: &FeishuFixture) -> AgentSpec {
    AgentSpec {
        name: fixture.bot_name.clone(),
        role: "E2E".into(),
        bio: String::new(),
        avatar: None,
        soul: String::new(),
        computer_id: "local".into(),
        harness_id: "pi".into(),
        model: "fixture".into(),
        thinking_level: "low".into(),
        auto_reaction: None,
        cli_path: None,
        environment_variables: Vec::new(),
        provider_agent_config: json!({
            "mode": "existing",
            "app_id": fixture.app_id,
            "domain": fixture.domain
        }),
    }
}

fn fixture_agent_spec(name: &str, app_id: &str) -> AgentSpec {
    AgentSpec {
        name: name.to_owned(),
        role: "E2E".into(),
        bio: String::new(),
        avatar: None,
        soul: String::new(),
        computer_id: "local".into(),
        harness_id: "pi".into(),
        model: "fixture".into(),
        thinking_level: "low".into(),
        auto_reaction: None,
        cli_path: None,
        environment_variables: Vec::new(),
        provider_agent_config: json!({
            "mode":"existing",
            "app_id":app_id,
            "domain":"feishu"
        }),
    }
}

fn standalone_runtime_spec(app_id: &str) -> AgentRuntimeSpec {
    AgentRuntimeSpec {
        agent_config: json!({}),
        provider_agent_config: json!({"app_id": app_id}),
        computer_id: "computer_e2e".to_owned(),
        harness_id: "harness_e2e".to_owned(),
        model: "model_e2e".to_owned(),
        thinking_level: "off".to_owned(),
        trigger_reason: "feishu_capabilities_e2e".to_owned(),
        chats: Vec::new(),
        unread_snapshot: json!({}),
    }
}

fn read_owner_only_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let metadata =
        fs::metadata(path).map_err(|_| "required Feishu fixture is unavailable".to_owned())?;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("Feishu fixture permissions are too broad".to_owned());
    }
    let bytes = fs::read(path).map_err(|_| "required Feishu fixture cannot be read".to_owned())?;
    serde_json::from_slice(&bytes).map_err(|_| "Feishu fixture JSON is invalid".to_owned())
}

fn resolve_fixture_path(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn validate_run_id(run_id: &str) -> Result<(), &'static str> {
    // The longest request key is `lankide-{run_id}-inbound-file`, while
    // Feishu accepts at most 50 characters for an idempotency key.
    if run_id.is_empty() || run_id.len() > 30 {
        return Err("run id must contain 1..30 characters");
    }
    if !run_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("run id contains an unsafe path character");
    }
    Ok(())
}

#[derive(Clone)]
struct LiveSecretReader {
    values: Arc<BTreeMap<String, String>>,
}

impl SecretReader for LiveSecretReader {
    fn read<'a>(&'a self, name: &'a str) -> ProviderFuture<'a, SecretValue> {
        Box::pin(async move {
            self.values
                .get(name)
                .cloned()
                .map(SecretValue::new)
                .ok_or(lankide_provider::ProviderError::SecretAccessDenied)
        })
    }
}

#[derive(Default)]
struct MemoryStorage {
    values: Mutex<BTreeMap<String, Vec<u8>>>,
    messages: Mutex<BTreeMap<i64, ProviderMessage>>,
    resources: Mutex<BTreeMap<i64, ProviderResource>>,
    people: Mutex<BTreeMap<String, ProviderPersonIdentity>>,
    agents: Mutex<BTreeSet<String>>,
}

#[derive(Clone, Default)]
struct TaskFlowTransport {
    state: Arc<Mutex<TaskFlowState>>,
}

#[derive(Default)]
struct TaskFlowState {
    create_bodies: Vec<Value>,
    tasklist_create_bodies: Vec<Value>,
    tasklist_member_bodies: Vec<Value>,
    comment_bodies: Vec<Value>,
    task_member_add_bodies: Vec<Value>,
    task_member_remove_bodies: Vec<Value>,
    tasks: BTreeMap<String, Value>,
    tasklist_members: Vec<Value>,
}

impl Transport for TaskFlowTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        if request
            .path
            .ends_with("/auth/v3/tenant_access_token/internal")
        {
            return json_response(
                200,
                json!({"code":0,"tenant_access_token":"task-token","expire":7200}),
            );
        }
        if request.path.ends_with("/bot/v3/info") {
            return json_response(
                200,
                json!({"code":0,"bot":{"open_id":"ou_alpha","app_name":"Alpha"}}),
            );
        }
        if request.path.ends_with("/task/v2/tasklists") && request.method == Method::Get {
            let state = self.state.lock().expect("task flow state");
            let items = if state.tasklist_create_bodies.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "guid":"tasklist_flow",
                    "name":"团队任务",
                    "owner":{"id":"cli_alpha","type":"app","role":"owner"},
                    "members":state.tasklist_members
                })]
            };
            return json_response(
                200,
                json!({"code":0,"data":{"items":items,"has_more":false}}),
            );
        }
        if request.path.ends_with("/task/v2/tasklists") && request.method == Method::Post {
            let body: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            let mut state = self.state.lock().expect("task flow state");
            state.tasklist_members = body["members"].as_array().cloned().unwrap_or_default();
            state.tasklist_create_bodies.push(body);
            return json_response(
                200,
                json!({"code":0,"data":{"tasklist":{
                    "guid":"tasklist_flow",
                    "name":"团队任务",
                    "owner":{"id":"cli_alpha","type":"app","role":"owner"},
                    "members":state.tasklist_members
                }}}),
            );
        }
        if request.path.ends_with("/task/v2/tasklists/tasklist_flow")
            && request.method == Method::Get
        {
            let state = self.state.lock().expect("task flow state");
            return json_response(
                200,
                json!({"code":0,"data":{"tasklist":{
                    "guid":"tasklist_flow",
                    "name":"团队任务",
                    "owner":{"id":"cli_alpha","type":"app","role":"owner"},
                    "members":state.tasklist_members
                }}}),
            );
        }
        if request.path.ends_with("/tasklist_flow/add_members") {
            let body: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            let mut state = self.state.lock().expect("task flow state");
            state.tasklist_member_bodies.push(body.clone());
            state
                .tasklist_members
                .extend(body["members"].as_array().cloned().unwrap_or_default());
            return json_response(
                200,
                json!({"code":0,"data":{"tasklist":{
                    "guid":"tasklist_flow",
                    "name":"团队任务",
                    "owner":{"id":"cli_alpha","type":"app","role":"owner"},
                    "members":state.tasklist_members
                }}}),
            );
        }
        if request.path.contains("/task/v2/tasks/")
            && (request.path.ends_with("/add_members") || request.path.ends_with("/remove_members"))
            && request.method == Method::Post
        {
            let body: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            let suffix = request
                .path
                .split("/task/v2/tasks/")
                .nth(1)
                .ok_or_else(|| TransportError::Failed("task member path missing".to_owned()))?;
            let (guid, operation) = suffix.split_once('/').ok_or_else(|| {
                TransportError::Failed("task member operation missing".to_owned())
            })?;
            let requested = body["members"].as_array().cloned().unwrap_or_default();
            let mut state = self.state.lock().expect("task flow state");
            if operation == "add_members" {
                state.task_member_add_bodies.push(body);
            } else {
                state.task_member_remove_bodies.push(body);
            }
            let task = state
                .tasks
                .get_mut(guid)
                .ok_or_else(|| TransportError::Failed("unknown fixture task".to_owned()))?;
            let members = task["members"]
                .as_array_mut()
                .ok_or_else(|| TransportError::Failed("fixture task members missing".to_owned()))?;
            if operation == "add_members" {
                for member in requested {
                    if !members.iter().any(|existing| existing == &member) {
                        members.push(member);
                    }
                }
            } else {
                members.retain(|existing| !requested.iter().any(|member| member == existing));
            }
            return json_response(200, json!({"code":0,"data":{"task":task.clone()}}));
        }
        if request.path.ends_with("/task/v2/tasks") && request.method == Method::Post {
            let body: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            let mut state = self.state.lock().expect("task flow state");
            let guid = format!("task_{}", state.create_bodies.len() + 1);
            let tasklist_guid = body
                .pointer("/tasklists/0/tasklist_guid")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let task = json!({
                "guid":guid,
                "summary":body["summary"].clone(),
                "description":body["description"].clone(),
                "members":body["members"].clone(),
                "tasklists":[{"tasklist_guid":tasklist_guid}],
                "tasklist_guid":tasklist_guid
            });
            state.create_bodies.push(body);
            state.tasks.insert(guid.clone(), task.clone());
            return json_response(200, json!({"code":0,"data":{"task":task}}));
        }
        if request.path.contains("/task/v2/tasks/") && request.method == Method::Get {
            let guid = request.path.rsplit('/').next().unwrap_or_default();
            let task = self
                .state
                .lock()
                .expect("task flow state")
                .tasks
                .get(guid)
                .cloned()
                .ok_or_else(|| TransportError::Failed("unknown fixture task".to_owned()))?;
            return json_response(200, json!({"code":0,"data":{"task":task}}));
        }
        if request.path.ends_with("/task/v2/comments") && request.method == Method::Post {
            let body: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            self.state
                .lock()
                .expect("task flow state")
                .comment_bodies
                .push(body);
            return json_response(
                200,
                json!({"code":0,"data":{"comment":{"comment_id":"comment_flow"}}}),
            );
        }
        if request.path.ends_with("/task/v2/comments") && request.method == Method::Get {
            return json_response(
                200,
                json!({"code":0,"data":{"items":[
                {"comment_id":"cross_app","content":"<at id=\"ou_lead_beta\"></at> <at id='ou_gamma_beta'></at> <at id=ou_unknown></at> 请确认","creator":{"type":"app","id":"cli_beta"}},
                {"comment_id":"unknown_author","content":"ordinary text","creator":{"type":"user","id":"ou_unknown"}}
            ],"has_more":false}}),
            );
        }
        Err(TransportError::Failed(format!(
            "unexpected Feishu task flow request: {}",
            request.path
        )))
    }
}

impl MemoryStorage {
    fn insert_agent(&self, agent_id: &str) {
        self.agents
            .lock()
            .expect("Agent storage")
            .insert(agent_id.to_owned());
    }

    fn insert_message(&self, message: ProviderMessage) {
        self.messages
            .lock()
            .expect("message storage")
            .insert(message.message_id.get(), message);
    }

    fn insert_resource(&self, resource: ProviderResource) {
        self.resources
            .lock()
            .expect("resource storage")
            .insert(resource.resource.resource_id, resource);
    }
}

#[derive(Clone, Copy)]
struct FeishuFixtureTransport;

#[derive(Clone, Default)]
struct RefreshingTokenTransport {
    token_requests: Arc<AtomicUsize>,
    bot_requests: Arc<AtomicUsize>,
}

#[derive(Clone, Default)]
struct AuthorizationUpgradeTransport {
    authorized: Arc<AtomicBool>,
    reject_updates: Arc<AtomicBool>,
    config_updates: Arc<AtomicUsize>,
    publishes: Arc<AtomicUsize>,
}

impl Transport for AuthorizationUpgradeTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        if request
            .path
            .ends_with("/auth/v3/tenant_access_token/internal")
        {
            return json_response(
                200,
                json!({
                    "code": 0,
                    "tenant_access_token": "authorization-upgrade-token",
                    "expire": 7200
                }),
            );
        }
        if request.path.contains("/app_versions") && request.method == Method::Get {
            let mut scopes = STANDARD_BOT_TENANT_SCOPES
                .iter()
                .map(|scope| json!({"scope":scope,"token_types":["tenant"]}))
                .collect::<Vec<_>>();
            if !self.authorized.load(Ordering::SeqCst) {
                scopes.retain(|scope| scope["scope"] != "application:application:patch");
            }
            let events = STANDARD_BOT_EVENTS
                .iter()
                .map(|event| json!({"event_type":event}))
                .collect::<Vec<_>>();
            return json_response(
                200,
                json!({
                    "code":0,
                    "data":{"items":[{
                        "version":"1.0.0",
                        "status":1,
                        "publish_time":"1700000000",
                        "scopes":scopes,
                        "event_infos":events
                    }]}
                }),
            );
        }
        if request.path.ends_with("/config") && request.method == Method::Patch {
            let body: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            let adds_patch_scope = body["scope"]["add_scopes"]
                .as_array()
                .is_some_and(|scopes| {
                    scopes.iter().any(|scope| {
                        scope["scope_name"] == "application:application:patch"
                            && scope["token_type"] == "tenant"
                    })
                });
            if !adds_patch_scope {
                return Err(TransportError::Failed(
                    "authorization update omitted the missing scope".into(),
                ));
            }
            self.config_updates.fetch_add(1, Ordering::SeqCst);
            if self.reject_updates.load(Ordering::SeqCst) {
                return json_response(
                    200,
                    json!({
                        "code":99991672,
                        "msg":"Access denied",
                        "error":{"permission_violations":[{
                            "subject":"application:application:patch"
                        }]}
                    }),
                );
            }
            return json_response(200, json!({"code":0}));
        }
        if request.path.ends_with("/publish") && request.method == Method::Post {
            self.publishes.fetch_add(1, Ordering::SeqCst);
            self.authorized.store(true, Ordering::SeqCst);
            return json_response(
                200,
                json!({"code":0,"data":{"version_id":"v2","version":"2.0.0"}}),
            );
        }
        Err(TransportError::Failed(
            "unexpected authorization upgrade request".into(),
        ))
    }
}

impl Transport for RefreshingTokenTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        if request
            .path
            .ends_with("/auth/v3/tenant_access_token/internal")
        {
            let request_number = self.token_requests.fetch_add(1, Ordering::SeqCst) + 1;
            return json_response(
                200,
                json!({
                    "code": 0,
                    "tenant_access_token": format!("token-{request_number}"),
                    "expire": 7200
                }),
            );
        }
        if request.path.ends_with("/bot/v3/info") {
            let request_number = self.bot_requests.fetch_add(1, Ordering::SeqCst) + 1;
            if request_number == 1 {
                return json_response(401, json!({"code":99991663,"msg":"expired"}));
            }
            return json_response(
                200,
                json!({
                    "code": 0,
                    "bot": {"open_id":"ou_refresh", "app_name":"Refresh"}
                }),
            );
        }
        Err(TransportError::Failed("unexpected refresh request".into()))
    }
}

fn json_response(status: u16, body: Value) -> Result<Response, TransportError> {
    Ok(Response {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: serde_json::to_vec(&body)
            .map_err(|error| TransportError::Failed(error.to_string()))?,
    })
}

impl Transport for FeishuFixtureTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        let body = if request
            .path
            .ends_with("/auth/v3/tenant_access_token/internal")
        {
            json!({
                "code": 0,
                "tenant_access_token": "fixture-tenant-token",
                "expire": 7200
            })
        } else if request.path.ends_with("/bot/v3/info") {
            json!({
                "code": 0,
                "bot": {"open_id": "ou_agent_e2e", "app_name": "E2E Agent"}
            })
        } else if request.path.ends_with("/task/v2/tasklists/tasklist_e2e") {
            json!({
                "code":0,
                "data":{"tasklist":{
                    "guid":"tasklist_e2e",
                    "name":"团队任务",
                    "owner":{"id":"cli_e2e","type":"app","role":"owner"},
                    "members":[{"id":"ou_human_e2e","type":"user","role":"viewer"}]
                }}
            })
        } else if request.path.contains("/contact/v3/users/on_profile") {
            json!({
                "code":0,
                "data":{"user":{
                    "open_id":"ou_profile",
                    "union_id":"on_profile",
                    "name":"Profile Human",
                    "nickname":"Profile Nickname",
                    "description":"Profile signature",
                    "avatar":{"avatar_origin":"https://avatar.example/profile.jpg"}
                }}
            })
        } else if request.path.contains("/contact/v3/users/ou_human") {
            json!({
                "code":0,
                "data":{"user":{
                    "open_id":"ou_human",
                    "union_id":"on_human",
                    "name":"Human Name",
                    "nickname":"Human Nickname"
                }}
            })
        } else if request.path == "https://avatar.example/profile.jpg" {
            return Ok(Response {
                status: 200,
                headers: vec![("content-type".into(), "image/jpeg".into())],
                body: b"fixture-avatar".to_vec(),
            });
        } else {
            return Err(TransportError::Failed(format!(
                "unexpected Feishu fixture request: {}",
                request.path
            )));
        };
        Ok(Response {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&body)
                .map_err(|error| TransportError::Failed(error.to_string()))?,
        })
    }
}

struct SharedQueueSource {
    queue: Arc<Mutex<VecDeque<RawEvent>>>,
}

impl EventSource for SharedQueueSource {
    fn next(&mut self) -> Result<Option<RawEvent>, SourceError> {
        Ok(self
            .queue
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front()))
    }

    fn acknowledge(&mut self) -> Result<(), SourceError> {
        Ok(())
    }

    fn close(&mut self) {}
}

#[derive(Clone, Copy)]
struct FeishuIdentityTransport;

impl Transport for FeishuIdentityTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        if request
            .path
            .contains("/messages/shared-attachment/resources/img_shared")
        {
            return Ok(Response {
                status: 200,
                headers: vec![("content-type".into(), "image/png".into())],
                body: b"shared-image".to_vec(),
            });
        }
        let body = if request
            .path
            .ends_with("/auth/v3/tenant_access_token/internal")
        {
            let input: Value = serde_json::from_slice(&request.body)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            let app_id = input
                .get("app_id")
                .and_then(Value::as_str)
                .ok_or_else(|| TransportError::Failed("fixture app_id missing".into()))?;
            json!({
                "code": 0,
                "tenant_access_token": format!("token-{app_id}"),
                "expire": 7200
            })
        } else if request.path.ends_with("/bot/v3/info") {
            let authorization = request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.as_str())
                .unwrap_or_default();
            let (open_id, name) = if authorization.ends_with("token-app_a") {
                ("ou_alpha_own", "Alpha")
            } else if authorization.ends_with("token-app_b") {
                ("ou_beta_own", "Beta")
            } else {
                return Err(TransportError::Failed(
                    "unknown fixture authorization".into(),
                ));
            };
            json!({"code":0,"bot":{"open_id":open_id,"app_name":name}})
        } else {
            return Err(TransportError::Failed(format!(
                "unexpected Feishu identity request: {}",
                request.path
            )));
        };
        Ok(Response {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&body)
                .map_err(|error| TransportError::Failed(error.to_string()))?,
        })
    }
}

fn identity_message(app_id: &str, event_id: &str, open_id: &str, union_id: &str) -> RawEvent {
    RawEvent::new(
        "im.message.receive_v1",
        json!({
            "header": {
                "app_id": app_id,
                "event_id": event_id,
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000000",
                "tenant_key": "tenant_e2e"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": format!("ou_human_{app_id}"),
                        "union_id": "on_human"
                    }
                },
                "message": {
                    "message_id": event_id,
                    "chat_id": "oc_identity",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"hello @_user_1\"}",
                    "create_time": "1700000000000",
                    "mentions": [{
                        "key": "@_user_1",
                        "id": {"open_id": open_id, "union_id": union_id},
                        "name": "Beta",
                        "mentioned_type": "bot"
                    }]
                }
            }
        })
        .to_string(),
    )
}

fn identity_bot_sender_message(
    app_id: &str,
    event_id: &str,
    open_id: &str,
    union_id: &str,
) -> RawEvent {
    RawEvent::new(
        "im.message.receive_v1",
        json!({
            "header": {
                "app_id": app_id,
                "event_id": event_id,
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000000",
                "tenant_key": "tenant_e2e"
            },
            "event": {
                "sender": {
                    "sender_type": "bot",
                    "sender_id": {"open_id": open_id, "union_id": union_id}
                },
                "message": {
                    "message_id": event_id,
                    "chat_id": "oc_identity",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"hello from bot\"}",
                    "create_time": "1700000000000",
                    "mentions": []
                }
            }
        })
        .to_string(),
    )
}

fn identity_attachment_message(app_id: &str, event_id: &str) -> RawEvent {
    RawEvent::new(
        "im.message.receive_v1",
        json!({
            "header": {
                "app_id": app_id,
                "event_id": event_id,
                "event_type": "im.message.receive_v1",
                "create_time": "1700000000000",
                "tenant_key": "tenant_e2e"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": {
                        "open_id": format!("ou_human_{app_id}"),
                        "union_id": "on_human"
                    }
                },
                "message": {
                    "message_id": "shared-attachment",
                    "chat_id": "oc_identity",
                    "chat_type": "group",
                    "message_type": "image",
                    "content": "{\"image_key\":\"img_shared\"}",
                    "create_time": "1700000000000",
                    "mentions": []
                }
            }
        })
        .to_string(),
    )
}

fn message_mentions(event: &ProviderEvent) -> Vec<&str> {
    match &event.kind {
        ProviderEventKind::Message(message) => message
            .mentioned_agent_ids
            .iter()
            .map(AgentId::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

impl ProviderStorage for MemoryStorage {
    fn get<'a>(&'a self, key: &'a str) -> ProviderFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            Ok(self
                .values
                .lock()
                .ok()
                .and_then(|values| values.get(key).cloned()))
        })
    }

    fn put<'a>(&'a self, key: &'a str, value: Vec<u8>) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if let Ok(mut values) = self.values.lock() {
                values.insert(key.to_owned(), value);
            }
            Ok(())
        })
    }

    fn message<'a>(&'a self, message_id: MessageId) -> ProviderFuture<'a, ProviderMessage> {
        Box::pin(async move {
            self.messages
                .lock()
                .ok()
                .and_then(|messages| messages.get(&message_id.get()).cloned())
                .ok_or(lankide_provider::ProviderError::NotFound { entity: "message" })
        })
    }

    fn resource<'a>(&'a self, resource: &'a ResourceRef) -> ProviderFuture<'a, ProviderResource> {
        Box::pin(async move {
            self.resources
                .lock()
                .ok()
                .and_then(|resources| resources.get(&resource.resource_id).cloned())
                .filter(|stored| stored.resource == *resource)
                .ok_or(lankide_provider::ProviderError::NotFound { entity: "resource" })
        })
    }

    fn get_or_create_person<'a>(
        &'a self,
        external_id: &'a str,
    ) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            let mut people = self.people.lock().expect("person storage");
            if let Some(person) = people.get(external_id) {
                return Ok(person.clone());
            }
            let person = ProviderPersonIdentity {
                person_id: PersonId::new(format!("{:06x}", people.len() + 1)).unwrap(),
                external_id: external_id.to_owned(),
                team_role: TeamRole::External,
                provider_state: json!({}),
                state_revision: 0,
            };
            people.insert(external_id.to_owned(), person.clone());
            Ok(person)
        })
    }

    fn person<'a>(&'a self, person_id: &'a PersonId) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.people
                .lock()
                .expect("person storage")
                .values()
                .find(|person| &person.person_id == person_id)
                .cloned()
                .ok_or(lankide_provider::ProviderError::NotFound { entity: "person" })
        })
    }

    fn team_members(&self) -> ProviderFuture<'_, Vec<ProviderTeamMember>> {
        Box::pin(async move {
            let humans = self
                .people
                .lock()
                .expect("person storage")
                .values()
                .filter(|person| person.team_role != TeamRole::External)
                .cloned()
                .map(ProviderTeamMember::Human)
                .collect::<Vec<_>>();
            let agents = self
                .agents
                .lock()
                .expect("Agent storage")
                .iter()
                .filter_map(|agent_id| AgentId::new(agent_id.clone()).ok())
                .map(|agent_id| ProviderTeamMember::Agent { agent_id })
                .collect::<Vec<_>>();
            Ok(humans.into_iter().chain(agents).collect())
        })
    }

    fn compare_and_set_person_state<'a>(
        &'a self,
        person_id: &'a PersonId,
        expected_revision: i64,
        state: Value,
    ) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            let mut people = self.people.lock().expect("person storage");
            let person = people
                .values_mut()
                .find(|person| &person.person_id == person_id)
                .ok_or(lankide_provider::ProviderError::NotFound { entity: "person" })?;
            if person.state_revision != expected_revision {
                return Err(lankide_provider::ProviderError::External {
                    operation: "person_state_conflict",
                    retryable: true,
                });
            }
            person.provider_state = state;
            person.state_revision += 1;
            Ok(person.clone())
        })
    }

    fn save_agent_file<'a>(
        &'a self,
        upload: ResourceUpload,
    ) -> ProviderFuture<'a, ProviderAgentFile> {
        Box::pin(async move {
            Ok(ProviderAgentFile {
                resource: ResourceRef {
                    resource_id: 99,
                    file_name: upload.file_name.clone(),
                    media_type: upload.media_type,
                    size_bytes: upload.bytes.len() as u64,
                    sha256: "fixture".to_owned(),
                },
                path: format!("/tmp/{}", upload.file_name),
            })
        })
    }
}

#[derive(Clone, Default)]
struct LiveLogger {
    messages: Arc<Mutex<Vec<String>>>,
}

impl ProviderLogger for LiveLogger {
    fn log(&self, level: LogLevel, message: &str) {
        if let Ok(mut messages) = self.messages.lock() {
            messages.push(format!("{level:?}: {message}"));
        }
    }
}

#[derive(Default)]
struct LiveResourceSaver {
    next_id: AtomicI64,
    uploads: Mutex<Vec<ResourceUpload>>,
}

impl ResourceSaver for LiveResourceSaver {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, ResourceRef> {
        Box::pin(async move {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
            let size_bytes = upload.bytes.len() as u64;
            if let Ok(mut uploads) = self.uploads.lock() {
                uploads.push(upload.clone());
            }
            Ok(ResourceRef {
                resource_id: id,
                file_name: upload.file_name,
                media_type: upload.media_type,
                size_bytes,
                sha256: "0".repeat(64),
            })
        })
    }
}

struct NoopProgress;

impl OperationProgress for NoopProgress {
    fn report<'a>(&'a self, _update: OperationUpdate) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn live_services(secret: String, logger: Arc<LiveLogger>) -> HostServices {
    live_services_with_storage(secret, logger, Arc::new(MemoryStorage::default()))
}

fn live_services_with_storage(
    secret: String,
    logger: Arc<LiveLogger>,
    storage: Arc<MemoryStorage>,
) -> HostServices {
    let mut values = BTreeMap::new();
    values.insert("app_secret".to_owned(), secret.clone());
    values.insert("agent:agent_e2e:app_secret".to_owned(), secret);
    HostServices {
        secret_reader: Arc::new(LiveSecretReader {
            values: Arc::new(values),
        }),
        secret_writer: Arc::new(lankide_provider::DeniedSecretWriter),
        storage,
        logger,
        resource_saver: Arc::new(LiveResourceSaver::default()),
        operation_progress: Arc::new(NoopProgress),
        event_capacity: 64,
        invocation_timeout: Duration::from_secs(30),
    }
}

fn cli_json(
    cli_dir: &Path,
    oauth_dir: &Path,
    current_dir: &Path,
    args: &[String],
) -> Result<Value, String> {
    let mut temporary_binary = None;
    let mut command = if current_dir == cli_dir {
        let mut command = Command::new("go");
        command.current_dir(cli_dir).args(["run", "."]);
        command
    } else {
        let binary = tempfile::Builder::new()
            .prefix("lark-cli-e2e-")
            .tempfile_in(current_dir)
            .map_err(|_| "temporary lark-cli binary could not be created".to_owned())?;
        let build = Command::new("go")
            .current_dir(cli_dir)
            .arg("build")
            .arg("-o")
            .arg(binary.path())
            .arg(".")
            .output()
            .map_err(|_| "lark-cli build could not start".to_owned())?;
        if !build.status.success() {
            return Err(format!(
                "lark-cli build failed: {}",
                safe_cli_diagnostic(&String::from_utf8_lossy(&build.stderr))
            ));
        }
        let mut command = Command::new(binary.path());
        command.current_dir(current_dir);
        temporary_binary = Some(binary);
        command
    };
    let mut child = command
        .args(args)
        .env("LARKSUITE_CLI_CONFIG_DIR", oauth_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "lark-cli process could not start".to_owned())?;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|_| "lark-cli output could not be read".to_owned())?;
                drop(temporary_binary.take());
                if !status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(format!(
                        "lark-cli operation failed (exit {}): {}",
                        status.code().unwrap_or(-1),
                        safe_cli_diagnostic(&format!("{stderr}\n{stdout}"))
                    ));
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let start = stdout
                    .find('{')
                    .ok_or_else(|| "lark-cli returned no JSON object".to_owned())?;
                let end = stdout
                    .rfind('}')
                    .ok_or_else(|| "lark-cli returned incomplete JSON".to_owned())?;
                return serde_json::from_str(&stdout[start..=end])
                    .map_err(|_| "lark-cli returned invalid JSON".to_owned());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("lark-cli operation timed out".to_owned());
            }
            Err(_) => return Err("lark-cli process status could not be read".to_owned()),
        }
    }
}

fn safe_cli_diagnostic(stderr: &str) -> String {
    let lower_output = stderr.to_ascii_lowercase();
    let missing_scopes = [
        "im:message.p2p_msg:get_as_user",
        "im:message.reactions:read",
    ]
    .into_iter()
    .filter(|scope| lower_output.contains(scope))
    .collect::<Vec<_>>();
    if !missing_scopes.is_empty() {
        return format!(
            "Feishu user OAuth fixture is missing required scope(s): {}",
            missing_scopes.join(", ")
        );
    }
    if ["token", "secret", "authorization", "cookie", "password"]
        .iter()
        .any(|sensitive| lower_output.contains(sensitive))
    {
        return "CLI diagnostic omitted because it contained sensitive credential text".to_owned();
    }
    let line = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or("no CLI diagnostic")
        .replace("\r", " ");
    if line.len() > 300 {
        format!("{}…", line.chars().take(300).collect::<String>())
    } else {
        line
    }
}

fn find_string(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(object) => {
            if let Some(value) = object.get(key).and_then(Value::as_str) {
                if !value.trim().is_empty() {
                    return Some(value.to_owned());
                }
            }
            object.values().find_map(|value| find_string(value, key))
        }
        Value::Array(values) => values.iter().find_map(|value| find_string(value, key)),
        _ => None,
    }
}

async fn cli_call(
    fixture: FeishuFixture,
    current_dir: PathBuf,
    args: Vec<String>,
) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        cli_json(&fixture.cli_dir, &fixture.oauth_dir, &current_dir, &args)
    })
    .await
    .map_err(|_| "lark-cli task failed".to_owned())?
}

async fn wait_for_message<P: Provider>(
    host: &ProviderHost<P>,
    marker: &str,
    max_wait: Duration,
) -> Result<ProviderEvent, String> {
    let deadline = Instant::now() + max_wait;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("provider event wait timed out".to_owned());
        }
        let event = match timeout(remaining.min(Duration::from_secs(10)), host.next_event()).await {
            Ok(Some(delivery)) => {
                let (event, acknowledgement) = delivery.into_parts();
                acknowledgement.accept();
                event
            }
            Ok(None) => return Err("provider event sink closed".to_owned()),
            Err(_) => continue,
        };
        if let ProviderEventKind::Message(message) = &event.kind
            && (message.content.contains(marker)
                || message
                    .attachments
                    .iter()
                    .any(|attachment| attachment.file_name.contains(marker)))
        {
            return Ok(event);
        }
    }
}

async fn wait_for_provider_message<P: Provider>(
    host: &ProviderHost<P>,
    provider_message_id: &str,
    max_wait: Duration,
) -> Result<ProviderEvent, String> {
    let deadline = Instant::now() + max_wait;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("provider event wait timed out".to_owned());
        }
        let event = match timeout(remaining.min(Duration::from_secs(10)), host.next_event()).await {
            Ok(Some(delivery)) => {
                let (event, acknowledgement) = delivery.into_parts();
                acknowledgement.accept();
                event
            }
            Ok(None) => return Err("provider event sink closed".to_owned()),
            Err(_) => continue,
        };
        if matches!(
            &event.kind,
            ProviderEventKind::Message(message)
                if message.provider_message_id == provider_message_id
        ) {
            return Ok(event);
        }
    }
}

async fn wait_for_task_targets<P: Provider>(
    host: &ProviderHost<P>,
    task_id: &str,
    max_wait: Duration,
) -> Result<Option<Vec<AgentId>>, String> {
    let deadline = Instant::now() + max_wait;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let event = match timeout(remaining.min(Duration::from_secs(5)), host.next_event()).await {
            Ok(Some(delivery)) => {
                let (event, acknowledgement) = delivery.into_parts();
                acknowledgement.accept();
                event
            }
            Ok(None) => return Err("provider event sink closed".to_owned()),
            Err(_) => continue,
        };
        if let ProviderEventKind::Custom(event) = event.kind
            && event.reason == "task"
            && event.subject_id == task_id
        {
            return Ok(Some(event.target_agent_ids));
        }
    }
}

async fn run_feishu_scenario(
    run_id: &str,
    marker: &str,
    artifact: &Artifact,
) -> Result<(), String> {
    let fixture = load_fixture()?;
    let report = tokio::task::spawn_blocking(|| {
        run_prerequisite_probe(&ProbeConfig::from_environment(
            GateSelection::FeishuLive,
            true,
        ))
    })
    .await
    .map_err(|_| "Feishu prerequisite probe task failed".to_owned())?;
    if !report.is_green() {
        return Err(format!(
            "Feishu live prerequisite is not green: {}",
            report.render_text().replace('\n', "; ")
        ));
    }
    exercise_feishu_contract()?;

    let workspace_id =
        WorkspaceId::new("feishu_e2e").map_err(|_| "invalid workspace id".to_owned())?;
    let agent_id = AgentId::new("agent_e2e").map_err(|_| "invalid Agent id".to_owned())?;
    let channel_id = ChannelId::new("group_chat").map_err(|_| "invalid channel id".to_owned())?;
    let logger = Arc::new(LiveLogger::default());
    let off_host = ProviderHost::new(
        FeishuProvider::new().without_event_source(),
        WorkspaceId::new("feishu_e2e_off")
            .map_err(|_| "invalid disabled workspace id".to_owned())?,
        json!({"task_system_enabled": false}),
        live_services(fixture.app_secret.clone(), logger.clone()),
    )
    .map_err(|_| "disabled Feishu ProviderHost could not be created".to_owned())?;
    if off_host
        .channels()
        .await
        .map_err(|_| "disabled Feishu channels could not be listed".to_owned())?
        .iter()
        .any(|channel| channel.id.as_str() == "task")
        || off_host
            .system_prompt(agent_id.clone(), task_runtime_spec(&fixture))
            .await
            .map_err(|_| "disabled Feishu prompt could not be read".to_owned())?
            .contains("飞书任务职责")
    {
        return Err("disabled Feishu Task system remained Agent-visible".to_owned());
    }
    artifact.record("task_system_disabled", "agent_e2e", "ok");

    let storage = Arc::new(MemoryStorage::default());
    storage.insert_agent(agent_id.as_str());
    let mut lead = storage
        .get_or_create_person(&fixture.user_open_id)
        .await
        .map_err(|_| "Feishu Team Lead could not be initialized".to_owned())?;
    lead.team_role = TeamRole::Lead;
    lead.provider_state = json!({
        "external_id_type":"open_id",
        "open_ids_by_app":{fixture.app_id.clone():fixture.user_open_id.clone()}
    });
    let lead_id = lead.person_id.clone();
    storage
        .people
        .lock()
        .map_err(|_| "Feishu Team roster could not be initialized".to_owned())?
        .insert(lead.external_id.clone(), lead);
    storage
        .put_workspace_state(json!({
            "tasklist_guid": fixture.tasklist_guid.clone()
        }))
        .await
        .map_err(|_| "Feishu Provider state could not be initialized".to_owned())?;
    let host = ProviderHost::new(
        FeishuProvider::new(),
        workspace_id,
        json!({
            "domain": fixture.domain.clone(),
            "task_system_enabled": true
        }),
        live_services_with_storage(fixture.app_secret.clone(), logger.clone(), storage.clone()),
    )
    .map_err(|_| "Feishu ProviderHost could not be created".to_owned())?;
    host.init()
        .await
        .map_err(|_| "Feishu ProviderHost initialization failed".to_owned())?;
    host.create_agent(agent_id.clone(), live_agent_spec(&fixture))
        .await
        .map_err(|_| "Feishu Agent connection failed".to_owned())?;
    artifact.record("provider_init", &fixture.app_id, "ok");

    let task_prompt = host
        .system_prompt(agent_id.clone(), task_runtime_spec(&fixture))
        .await
        .map_err(|_| "enabled Feishu Task prompt could not be read".to_owned())?;
    let task_tools = host
        .tools(agent_id.clone(), task_runtime_spec(&fixture))
        .await
        .map_err(|_| "enabled Feishu Task tools could not be listed".to_owned())?;
    let task_tool_ids = task_tools
        .iter()
        .map(|tool| tool.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if task_prompt.is_empty()
        || task_prompt.contains("Runtime")
        || task_prompt.contains("Provider")
        || ![
            "feishu_reaction_catalog",
            "feishu_reaction_add",
            "feishu_reaction_list",
            "feishu_reaction_remove",
            "feishu_task_complete",
            "feishu_task_create",
            "feishu_task_get",
            "feishu_task_list",
            "feishu_task_update",
            "feishu_task_member_add",
            "feishu_task_member_remove",
            "feishu_task_comments",
            "feishu_task_comment",
        ]
        .into_iter()
        .all(|id| task_tool_ids.contains(id))
    {
        return Err("enabled Feishu Task capabilities are incomplete".to_owned());
    }
    artifact.record("task_system_enabled", "agent_e2e", "ok");

    let mut provider_message_ids = Vec::new();
    let mut actor_message_ids = Vec::new();
    let mut task_ids = Vec::new();
    let mut reaction_ids = Vec::new();
    let scenario_result = tokio::time::timeout(Duration::from_secs(360), async {
        let send_args = vec![
            "im".to_owned(),
            "+messages-send".to_owned(),
            "--as".to_owned(),
            "user".to_owned(),
            "--chat-id".to_owned(),
            fixture.chat_id.clone(),
            "--msg-type".to_owned(),
            "text".to_owned(),
            "--content".to_owned(),
            serde_json::to_string(&json!({
                "text": format!(
                    "<at user_id=\"{}\">{}</at> {}",
                    fixture.bot_open_id, fixture.bot_name, marker
                )
            }))
            .map_err(|_| "Feishu mention payload could not be encoded".to_owned())?,
            "--idempotency-key".to_owned(),
            format!("lankide-{run_id}-inbound"),
            "--json".to_owned(),
        ];
        let sent = cli_call(fixture.clone(), fixture.cli_dir.clone(), send_args).await?;
        let inbound_id = find_string(&sent, "message_id")
            .ok_or_else(|| "Feishu user send returned no message ID".to_owned())?;
        actor_message_ids.push(inbound_id.clone());
        artifact.record("actor_send_mention", &inbound_id, "ok");

        let event = wait_for_message(&host, marker, Duration::from_secs(90)).await?;
        let (provider_id, content, mentions) = match &event.kind {
            ProviderEventKind::Message(message) => (
                message.provider_message_id.clone(),
                message.content.clone(),
                message.mentioned_agent_ids.clone(),
            ),
            _ => return Err("Feishu event was not a message".to_owned()),
        };
        if provider_id != inbound_id || !content.contains(marker) || !mentions.contains(&agent_id) {
            return Err("Feishu inbound identity or mention normalization failed".to_owned());
        }
        artifact.record("receive_mention", &provider_id, "ok");

        let inbound_file_name = format!("{marker}-inbound.txt");
        let inbound_file_dir = artifact.dir.clone();
        let inbound_file = inbound_file_dir.join(&inbound_file_name);
        fs::write(&inbound_file, format!("{marker} attachment").as_bytes())
            .map_err(|_| "Feishu attachment fixture could not be written".to_owned())?;
        let file_args = vec![
            "im".to_owned(),
            "+messages-send".to_owned(),
            "--as".to_owned(),
            "user".to_owned(),
            "--chat-id".to_owned(),
            fixture.chat_id.clone(),
            "--file".to_owned(),
            inbound_file_name.clone(),
            "--idempotency-key".to_owned(),
            format!("lankide-{run_id}-inbound-file"),
            "--json".to_owned(),
        ];
        let sent_file = cli_call(fixture.clone(), inbound_file_dir, file_args).await?;
        let inbound_file_id = find_string(&sent_file, "message_id")
            .ok_or_else(|| "Feishu file send returned no message ID".to_owned())?;
        actor_message_ids.push(inbound_file_id.clone());
        artifact.record("actor_send_attachment", &inbound_file_id, "ok");
        let file_event =
            wait_for_provider_message(&host, &inbound_file_id, Duration::from_secs(90)).await?;
        let file_attachment_count = match file_event.kind {
            ProviderEventKind::Message(message) => message.attachments.len(),
            _ => 0,
        };
        if file_attachment_count == 0 {
            return Err("Feishu inbound attachment was not saved".to_owned());
        }
        artifact.record("receive_attachment", &inbound_file_id, "ok");

        let inbound_internal_id =
            MessageId::new(1).map_err(|_| "invalid internal message id".to_owned())?;
        let reply_internal_id =
            MessageId::new(2).map_err(|_| "invalid reply message id".to_owned())?;
        storage.insert_message(ProviderMessage {
            message_id: inbound_internal_id,
            provider_message_id: Some(inbound_id.clone()),
            channel_id: channel_id.clone(),
            chat_id: fixture.chat_id.clone(),
        });
        let reply = host
            .send_message(
                agent_id.clone(),
                SendMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.clone(),
                    message_id: reply_internal_id,
                    idempotency_key: format!("{run_id}-feishu-reply"),
                    content: format!("{marker} reply @Agent[agent_e2e]"),
                    reply_to: Some(inbound_internal_id),
                    mentions: vec![agent_id.clone()],
                    attachments: Vec::new(),
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .map_err(|_| "Feishu Provider reply failed".to_owned())?;
        let reply_id = reply
            .provider_message_id
            .clone()
            .ok_or_else(|| "Feishu Provider reply returned no message ID".to_owned())?;
        if reply.status != SendMessageStatus::Sent {
            return Err("Feishu Provider reply did not return Sent".to_owned());
        }
        provider_message_ids.push(reply_id.clone());
        artifact.record("provider_reply", &reply_id, "ok");

        let outbound_name = format!("{marker}-outbound.txt");
        let outbound_bytes = format!("{marker} outbound attachment").into_bytes();
        let outbound_resource = ResourceRef {
            resource_id: 9001,
            file_name: outbound_name,
            media_type: "text/plain".into(),
            size_bytes: outbound_bytes.len() as u64,
            sha256: "0".repeat(64),
        };
        storage.insert_resource(ProviderResource {
            resource: outbound_resource.clone(),
            bytes: outbound_bytes,
        });
        let outbound = host
            .send_message(
                agent_id.clone(),
                SendMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.clone(),
                    message_id: MessageId::new(3)
                        .map_err(|_| "invalid outbound message id".to_owned())?,
                    idempotency_key: format!("{run_id}-feishu-attachment"),
                    content: marker.to_owned(),
                    reply_to: Some(inbound_internal_id),
                    mentions: Vec::new(),
                    attachments: vec![MessageAttachment {
                        resource: outbound_resource,
                        caption: None,
                    }],
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .map_err(|_| "Feishu Provider attachment send failed".to_owned())?;
        let outbound_id = outbound
            .provider_message_id
            .clone()
            .ok_or_else(|| "Feishu Provider attachment send returned no message ID".to_owned())?;
        if outbound.status != SendMessageStatus::Sent {
            return Err("Feishu Provider attachment send did not return Sent".to_owned());
        }
        provider_message_ids.push(outbound_id.clone());
        artifact.record("provider_send_attachment", &outbound_id, "ok");

        host.auto_reaction(
            agent_id.clone(),
            AutoReactionSpec {
                auto_reaction: "THUMBSUP".to_owned(),
                channel_id: channel_id.clone(),
                chat_id: fixture.chat_id.clone(),
                trigger_messages: vec![inbound_internal_id],
                provider_agent_config: json!({}),
            },
        )
        .await
        .map_err(|_| "Feishu Provider reaction add failed".to_owned())?;
        artifact.record("provider_reaction_add", &inbound_id, "ok");

        let reaction_client = FeishuClient::new(
            ReqwestTransport::new(),
            Credentials::app(fixture.app_id.clone(), fixture.app_secret.clone()),
        )
        .map_err(|_| "Feishu reaction verification client could not be created".to_owned())?;
        let reaction_message_id = inbound_id.clone();
        let reactions = tokio::task::spawn_blocking(move || {
            reaction_client.list_reactions(&reaction_message_id, None, 50, Auth::Tenant)
        })
        .await
        .map_err(|_| "Feishu reaction verification failed".to_owned())?
        .map_err(|_| "Feishu reaction list endpoint failed".to_owned())?;
        if let Some(item) = reactions
            .items
            .into_iter()
            .find(|item| item.emoji == "THUMBSUP")
        {
            reaction_ids.push(item.reaction_id);
        } else {
            return Err("Feishu reaction add was not visible through the API".to_owned());
        }
        artifact.record("verify_reaction", &inbound_id, "ok");

        let create_input = json!({
            "summary": format!("{marker} Agent task"),
            "description": marker
        });
        let created = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_create",
                create_input.clone(),
            )
            .await
            .map_err(|_| "Feishu Agent task create failed".to_owned())?;
        let task_id = created
            .pointer("/task/task_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "Feishu Agent task create returned no task ID".to_owned())?;
        task_ids.push(task_id.clone());
        let created_text = serde_json::to_string(&created).unwrap_or_default();
        if created_text.contains(&fixture.app_id)
            || created_text.contains(&fixture.user_open_id)
            || created_text.contains(&fixture.bot_open_id)
        {
            return Err("Feishu Agent task result exposed a Provider identity".to_owned());
        }
        let assignee_ids = created
            .pointer("/task/assignee_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| "Feishu Agent task returned no assignees".to_owned())?;
        if !assignee_ids.iter().any(|value| value == agent_id.as_str())
            || !assignee_ids.iter().any(|value| value == lead_id.as_str())
        {
            return Err("Feishu task did not assign the calling Agent and Team Lead".to_owned());
        }
        artifact.record("agent_task_create", &task_id, "ok");

        let second = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_create",
                create_input,
            )
            .await
            .map_err(|error| format!("second Feishu Agent task create failed: {error}"))?;
        let second_task_id = second
            .pointer("/task/task_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "second Feishu Agent task returned no task ID".to_owned())?;
        if second_task_id == task_id {
            return Err("separate Feishu task creates reused one client token".to_owned());
        }
        task_ids.push(second_task_id.clone());
        artifact.record("agent_task_create_distinct", &second_task_id, "ok");

        let assignee_targets = wait_for_task_targets(&host, &task_id, Duration::from_secs(45))
            .await?
            .ok_or_else(|| "Feishu assignee task event was not received".to_owned())?;
        if !assignee_targets.contains(&agent_id) {
            return Err("Feishu assignee task event did not target its Agent".to_owned());
        }
        artifact.record("agent_task_dispatch", &task_id, "ok");

        let read = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_get",
                json!({"task_id": task_id}),
            )
            .await
            .map_err(|_| "Feishu Agent task read failed".to_owned())?;
        if read.pointer("/task/can_update").and_then(Value::as_bool) != Some(true) {
            return Err("Feishu task assignee was not allowed to update".to_owned());
        }
        let listed = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_list",
                json!({"page_size": 100}),
            )
            .await
            .map_err(|_| "Feishu Agent task list failed".to_owned())?;
        if !serde_json::to_string(&listed)
            .map(|value| value.contains(&task_id))
            .unwrap_or(false)
        {
            return Err("Feishu Agent task list did not return the assigned task".to_owned());
        }
        artifact.record("agent_task_read", &task_id, "ok");

        let comment = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_comment",
                json!({
                    "task_id":task_id,
                    "content":format!("@Team Lead[{}] {marker}", lead_id)
                }),
            )
            .await
            .map_err(|_| "Feishu Agent task comment failed".to_owned())?;
        let comment_id = comment
            .get("comment_id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Feishu Agent task comment returned no comment ID".to_owned())?;
        let comment_client = FeishuClient::new(
            ReqwestTransport::new(),
            Credentials::app(fixture.app_id.clone(), fixture.app_secret.clone()),
        )
        .map_err(|_| "Feishu comment verification client could not be created".to_owned())?;
        let comment_task_id = task_id.clone();
        let comments = tokio::task::spawn_blocking(move || {
            comment_client.list_task_comments(&comment_task_id, None, 100, Auth::Tenant)
        })
        .await
        .map_err(|_| "Feishu task comment verification failed".to_owned())?
        .map_err(|_| "Feishu task comment list endpoint failed".to_owned())?;
        let native_mention = format!("<at id={}></at>", fixture.user_open_id);
        if !comments
            .items
            .iter()
            .any(|item| item.comment_id == comment_id && item.content.contains(&native_mention))
        {
            return Err("Feishu task comment did not contain a native mention".to_owned());
        }
        artifact.record("agent_task_native_comment_mention", comment_id, "ok");

        let updated = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_update",
                json!({"task_id": task_id, "summary": format!("{marker} updated")}),
            )
            .await
            .map_err(|_| "Feishu Agent task update failed".to_owned())?;
        if updated.pointer("/task/summary").and_then(Value::as_str)
            != Some(format!("{marker} updated").as_str())
        {
            return Err("Feishu Agent task update was not visible".to_owned());
        }
        artifact.record("agent_task_update", &task_id, "ok");

        let completed = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_complete",
                json!({"task_id": task_id}),
            )
            .await
            .map_err(|_| "Feishu Agent task completion failed".to_owned())?;
        let repeated_completion = host
            .invoke_tool_in_context(
                agent_id.clone(),
                task_runtime_spec(&fixture),
                "feishu_task_complete",
                json!({"task_id": task_id}),
            )
            .await
            .map_err(|_| "repeated Feishu Agent task completion failed".to_owned())?;
        if completed.pointer("/task/status").and_then(Value::as_str) != Some("completed")
            || repeated_completion
                .pointer("/task/status")
                .and_then(Value::as_str)
                != Some("completed")
        {
            return Err("Feishu Agent task completion was not idempotent".to_owned());
        }
        artifact.record("agent_task_complete_repeat", &task_id, "ok");

        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "feishu live scenario exceeded hard timeout".to_owned())
    .and_then(|result| result);

    for reaction_id in reaction_ids {
        let client = FeishuClient::new(
            ReqwestTransport::new(),
            Credentials::app(fixture.app_id.clone(), fixture.app_secret.clone()),
        );
        let message_id = actor_message_ids.first().cloned().unwrap_or_default();
        let reaction_id_for_call = reaction_id.clone();
        let result = match client {
            Ok(client) => tokio::task::spawn_blocking(move || {
                client.remove_reaction(&message_id, &reaction_id_for_call, Auth::Tenant)
            })
            .await
            .ok()
            .and_then(Result::ok),
            Err(_) => None,
        };
        artifact.record_cleanup("provider_reaction_remove", &reaction_id, result.is_some());
    }
    for message_id in provider_message_ids.iter().rev() {
        let result = host
            .recall_message(
                agent_id.clone(),
                RecallMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.clone(),
                    message_id: MessageId::new(4)
                        .map_err(|_| "invalid cleanup message id".to_owned())?,
                    provider_message_id: message_id.clone(),
                    provider_agent_config: Value::Null,
                },
            )
            .await;
        artifact.record_cleanup("provider_recall", message_id, result.is_ok());
    }
    for message_id in actor_message_ids.iter().rev() {
        let args = vec![
            "im".to_owned(),
            "messages".to_owned(),
            "delete".to_owned(),
            "--as".to_owned(),
            "user".to_owned(),
            "--params".to_owned(),
            serde_json::to_string(&json!({"message_id": message_id})).unwrap_or_default(),
            "--yes".to_owned(),
            "--json".to_owned(),
        ];
        let result = cli_call(fixture.clone(), fixture.cli_dir.clone(), args).await;
        artifact.record_cleanup("actor_recall", message_id, result.is_ok());
    }
    for task_id in task_ids.iter().rev() {
        let args = vec![
            "task".to_owned(),
            "tasks".to_owned(),
            "delete".to_owned(),
            "--as".to_owned(),
            "user".to_owned(),
            "--task-guid".to_owned(),
            task_id.clone(),
            "--yes".to_owned(),
            "--json".to_owned(),
        ];
        let mut result = cli_call(fixture.clone(), fixture.cli_dir.clone(), args)
            .await
            .map(|_| ());
        if result.is_err() {
            let client = FeishuClient::new(
                ReqwestTransport::new(),
                Credentials::app(fixture.app_id.clone(), fixture.app_secret.clone()),
            );
            let task_id_for_call = task_id.clone();
            result = match client {
                Ok(client) => tokio::task::spawn_blocking(move || {
                    client.delete_task(&task_id_for_call, "open_id", Auth::Tenant)
                })
                .await
                .map_err(|_| "Feishu app task cleanup task failed".to_owned())
                .and_then(|result| result.map_err(|_| "Feishu app task cleanup failed".to_owned())),
                Err(_) => Err("Feishu app task cleanup client could not be created".to_owned()),
            };
        }
        artifact.record_cleanup("task_delete", task_id, result.is_ok());
    }
    let shutdown = tokio::time::timeout(Duration::from_secs(30), host.shutdown()).await;
    artifact.record_cleanup("provider_shutdown", "host", matches!(shutdown, Ok(Ok(()))));
    artifact.complete_cleanup();
    if scenario_result.is_err()
        && let Ok(messages) = logger.messages.lock()
    {
        for message in messages.iter() {
            eprintln!("Feishu provider diagnostic: {message}");
            artifact.record_diagnostic("provider_log", message);
        }
    }
    scenario_result
}

#[test]
fn feishu_ingress_failure_retries_before_acknowledging_source() {
    use lankide_provider_feishu::{EventSink as FeishuSink, FeishuEventDispatcher, SinkError};
    struct ReceiptSource {
        event: Option<RawEvent>,
        acks: Arc<AtomicI64>,
    }
    impl EventSource for ReceiptSource {
        fn next(&mut self) -> Result<Option<RawEvent>, SourceError> {
            Ok(self.event.take())
        }
        fn acknowledge(&mut self) -> Result<(), SourceError> {
            self.acks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn close(&mut self) {}
    }
    struct RejectOnce {
        failed: bool,
        delivered: usize,
    }
    impl FeishuSink for RejectOnce {
        fn emit(&mut self, _: NormalizedEvent) -> Result<(), SinkError> {
            if !self.failed {
                self.failed = true;
                return Err(SinkError("durable storage unavailable".into()));
            }
            self.delivered += 1;
            Ok(())
        }
    }
    let acks = Arc::new(AtomicI64::new(0));
    let body = json!({
        "header": {"app_id": "cli_receipt", "event_type": "im.message.receive_v1", "event_id": "receipt"},
        "event": {
            "sender": {"sender_type": "user", "sender_id": {"open_id": "ou_receipt"}},
            "message": {"message_id": "om_receipt", "chat_id": "oc_receipt", "chat_type": "group",
                "message_type": "text", "content": "{\"text\":\"receipt\"}"}
        }
    });
    let mut dispatcher = FeishuEventDispatcher::new(
        ReceiptSource {
            event: Some(RawEvent::new(
                "im.message.receive_v1",
                body.to_string().into_bytes(),
            )),
            acks: acks.clone(),
        },
        NormalizationConfig::new("cli_receipt"),
    );
    dispatcher.register("im.message.receive_v1").unwrap();
    let mut sink = RejectOnce {
        failed: false,
        delivered: 0,
    };
    assert!(dispatcher.poll_once(&mut sink).is_err());
    assert_eq!(acks.load(Ordering::SeqCst), 0);
    assert!(dispatcher.poll_once(&mut sink).unwrap());
    assert_eq!(acks.load(Ordering::SeqCst), 1);
    assert_eq!(sink.delivered, 1);
    assert!(!dispatcher.poll_once(&mut sink).unwrap());
    dispatcher.stop();
}
