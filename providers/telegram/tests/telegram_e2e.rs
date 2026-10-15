//! Telegram module-level E2E entrypoint.
//!
//! The ignored case is the single module acceptance scenario: it exercises
//! public mention/normalization/polling boundaries and then calls the shared
//! fail-closed prerequisite before the real Manager/Agent Bot flow.

use lankide_core::ResourceRef;
use lankide_core::{AgentId, ChannelId, MessageId, PersonId, WorkspaceId};
use lankide_provider::{
    AgentOnboardingContext, AgentRuntimeSpec, AgentSpec, AutoReactionSpec, EditMessageSpec,
    HostServices, LogLevel, MessageAttachment, OperationProgress, OperationUpdate, ProviderFuture,
    ProviderHost, ProviderLogger, ProviderMessage, ProviderOnboardingStatus,
    ProviderPersonIdentity, ProviderResource, ProviderStorage, RecallMessageSpec, ResourceSaver,
    ResourceUpload, SecretReader, SecretValue, SendMessageSpec, SendMessageStatus, TeamRole,
};
use lankide_provider_telegram::{
    Credentials, NormalizationConfig, PollRequest, Poller, ReqwestTransport, TelegramClient,
    TelegramProvider, Transport, TransportError, normalize_update, render_html_with_mentions,
};
use lankide_provider_telegram::{MentionTarget, dto::User};
use lankide_testkit::{
    GateSelection, ProbeConfig, ScenarioArtifact as Artifact, run_prerequisite_probe,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::env;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Exercise the Telegram wire contract as part of the complete provider
/// scenario.  The acceptance command therefore covers rendering,
/// normalization, and acknowledgement semantics together with the real Bot
/// API flow, without maintaining separate function-level tests.
fn exercise_telegram_contract() -> Result<(), String> {
    let rendered = render_html_with_mentions(
        "ping @Agent[agent_e2e]",
        &[MentionTarget::new(
            "agent_e2e",
            Some(42),
            Some("e2e_bot".to_owned()),
            "E2E Bot",
        )],
    );
    if !rendered.contains("tg://user?id=42") || !rendered.contains("E2E Bot") {
        return Err("Telegram Markdown mention rendering changed".to_owned());
    }

    let response = Response {
        status: 200,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: serde_json::to_vec(&json!({
            "ok": true,
            "result": [{
                "update_id": 41,
                "message": {
                    "message_id": 7,
                    "from": {"id": 9, "is_bot": false, "first_name": "E2E"},
                    "chat": {"id": -1001234567890_i64, "type": "group", "title": "Example Team"},
                    "date": 1700000000,
                    "text": "hello"
                }
            }]
        }))
        .map_err(|_| "Telegram poll fixture could not be encoded".to_owned())?,
    };
    let transport = RepeatingTransport::new(response);
    let bot = User {
        id: 123,
        is_bot: true,
        username: "e2e_bot".to_owned(),
        ..User::default()
    };
    let config = NormalizationConfig::new("telegram", "123", bot.clone());
    let client = TelegramClient::new(transport, Credentials::new("123", "123:fixture"))
        .map_err(|_| "Telegram poll fixture credentials are invalid".to_owned())?;
    let mut poller = Poller::new(client, config);
    let batch = poller
        .poll_once()
        .map_err(|_| "Telegram poll fixture failed".to_owned())?
        .ok_or_else(|| "Telegram poll fixture returned no batch".to_owned())?;
    if poller.offset() != 0 || batch.next_offset != 42 {
        return Err("Telegram poll offset advanced before acknowledgement".to_owned());
    }
    poller
        .acknowledge(&batch)
        .map_err(|_| "Telegram poll acknowledgement failed".to_owned())?;
    if poller.offset() != 42 {
        return Err("Telegram poll offset did not advance after acknowledgement".to_owned());
    }

    let raw = serde_json::to_vec(&json!({
        "update_id": 77,
        "message": {
            "message_id": 8,
            "from": {"id": 9, "is_bot": false, "first_name": "E2E"},
            "chat": {"id": -1001234567890_i64, "type": "group", "title": "Example Team"},
            "date": 1700000000,
            "text": "@e2e_bot hi",
            "entities": [{"type": "mention", "offset": 0, "length": 8}]
        }
    }))
    .map_err(|_| "Telegram normalization fixture could not be encoded".to_owned())?;
    let update = lankide_provider_telegram::dto::Update::from_json(&raw)
        .map_err(|_| "Telegram normalization fixture could not be decoded".to_owned())?;
    let config = NormalizationConfig::new("telegram", "123", bot).with_agent(123, "agent_e2e");
    let events = normalize_update(&update, &config)
        .map_err(|_| "Telegram update normalization failed".to_owned())?;
    if events.len() != 1 {
        return Err("Telegram normalization produced an unexpected event count".to_owned());
    }
    let lankide_provider_telegram::NormalizedEvent::Message(message) = &events[0] else {
        return Err("Telegram normalization did not produce a message".to_owned());
    };
    if message.event_id != "telegram:update:77" || message.mentioned_agent_ids != vec!["agent_e2e"]
    {
        return Err("Telegram update identity or mention mapping changed".to_owned());
    }
    let edited_raw = serde_json::to_vec(&json!({
        "update_id": 78,
        "edited_message": {
            "message_id": 8,
            "from": {"id": 9, "is_bot": false, "first_name": "E2E"},
            "chat": {"id": -1001234567890_i64, "type": "group", "title": "Example Team"},
            "edit_date": 1700000010,
            "text": "edited hello"
        }
    }))
    .map_err(|_| "Telegram edited-message fixture could not be encoded".to_owned())?;
    let edited_update = lankide_provider_telegram::dto::Update::from_json(&edited_raw)
        .map_err(|_| "Telegram edited-message fixture could not be decoded".to_owned())?;
    let edited_events = normalize_update(&edited_update, &config)
        .map_err(|_| "Telegram edited-message normalization failed".to_owned())?;
    let Some(lankide_provider_telegram::NormalizedEvent::Message(edited)) = edited_events.first()
    else {
        return Err("Telegram edited update did not become a normal message".to_owned());
    };
    if !edited.edited || edited.provider_message_id != "8:edit:78" {
        return Err(
            "Telegram edited update did not receive an independent message identity".into(),
        );
    }
    Ok(())
}

#[tokio::test]
async fn telegram_pending_agent_onboarding_resumes_after_reopen() {
    let storage = Arc::new(MemoryStorage::default());
    let services = services_with_storage(
        BTreeMap::new(),
        Arc::new(LiveLogger::default()),
        storage.clone(),
    );
    let workspace_id = WorkspaceId::new("telegram_resume").expect("workspace id");
    let agent_id = AgentId::new("pending_agent").expect("agent id");
    let context = AgentOnboardingContext::new(
        workspace_id.clone(),
        json!({}),
        agent_id.clone(),
        "Pending Agent".into(),
        json!({"mode":"new","bot_username":"pending_agent_bot"}),
        services.clone(),
    );
    let first = lankide_provider_telegram::agent_onboarding(context, "get".into(), json!({}))
        .await
        .expect("initial onboarding");
    assert_eq!(first.status, ProviderOnboardingStatus::ActionRequired);
    assert_eq!(first.verification_uri, None);
    assert_eq!(first.poll_interval_seconds, None);
    let payload = json!({
        "bot_username": "pending_agent_bot",
        "verification_uri": "https://t.me/example",
        "interval_seconds": 60,
    });
    storage
        .put_agent_state(
            &agent_id,
            json!({"onboarding": {"step": "create_bot", "status": "action_required", "payload": payload}}),
        )
        .await
        .expect("save pending onboarding");

    let reopened = AgentOnboardingContext::new(
        workspace_id,
        json!({}),
        agent_id,
        "Pending Agent".into(),
        json!({"mode":"new","bot_username":"pending_agent_bot"}),
        services,
    );
    let restored = lankide_provider_telegram::agent_onboarding(reopened, "get".into(), json!({}))
        .await
        .expect("restored onboarding");
    assert_eq!(restored.step, first.step);
    assert_eq!(restored.status, first.status);
    assert_eq!(restored.opaque_payload, payload);
    assert_eq!(
        restored.verification_uri.as_deref(),
        Some("https://t.me/example")
    );
    assert_eq!(restored.poll_interval_seconds, Some(30));
}

#[tokio::test]
async fn telegram_provider_builds_cross_agent_mention_directory() {
    let transport = IdentityTransport::default();
    let requests = transport.requests.clone();
    let workspace_id = WorkspaceId::new("telegram_mentions").expect("workspace id");
    let alpha = AgentId::new("alpha").expect("alpha agent id");
    let beta = AgentId::new("beta").expect("beta agent id");
    let mut secrets = BTreeMap::new();
    secrets.insert("agent:alpha:bot_token".into(), "100:alpha".into());
    secrets.insert("agent:beta:bot_token".into(), "200:beta".into());
    let storage = Arc::new(MemoryStorage::default());
    storage.insert_message(ProviderMessage {
        message_id: MessageId::new(1).expect("message id"),
        provider_message_id: Some("8:edit:78".into()),
        channel_id: ChannelId::new("group_chat").expect("channel id"),
        chat_id: "-1001".into(),
    });
    let host = ProviderHost::new(
        TelegramProvider::new().with_transport(transport),
        workspace_id,
        json!({"polling": false}),
        services_with_storage(secrets, Arc::new(LiveLogger::default()), storage.clone()),
    )
    .expect("Telegram ProviderHost");
    let reactions = host.reaction_catalog();
    assert_eq!(
        reactions.len(),
        73,
        "all standard Telegram reactions are selectable"
    );
    let unique_reactions: std::collections::HashSet<_> =
        reactions.iter().map(|reaction| &reaction.id).collect();
    assert_eq!(unique_reactions.len(), reactions.len());
    assert!(reactions.iter().any(|reaction| reaction.id == "👍"));
    assert!(reactions.iter().any(|reaction| reaction.id == "❤️"));
    assert!(reactions.iter().any(|reaction| reaction.id == "❤‍🔥"));
    assert!(reactions.iter().any(|reaction| reaction.id == "🤷‍♀"));
    assert!(reactions.iter().any(|reaction| reaction.id == "😡"));
    assert!(
        reactions
            .iter()
            .all(|reaction| reaction.id == reaction.emoji)
    );
    host.init().await.expect("Provider init");
    let status = host
        .invoke_interface("telegram.workspace.status", json!({}))
        .await
        .expect("Telegram workspace status");
    assert_eq!(status["manager_bot_ready"], false);
    host.create_agent(alpha.clone(), existing_agent_spec("Alpha", "alpha_bot"))
        .await
        .expect("Alpha Bot init");
    host.create_agent(beta.clone(), existing_agent_spec("Beta", "beta_bot"))
        .await
        .expect("Beta Bot init");
    let person = storage
        .get_or_create_person("300")
        .await
        .expect("Telegram person");
    let tools = host
        .tools(
            alpha.clone(),
            AgentRuntimeSpec {
                agent_config: json!({}),
                provider_agent_config: json!({}),
                computer_id: "fixture-computer".into(),
                harness_id: "pi".into(),
                model: "fixture".into(),
                thinking_level: "low".into(),
                trigger_reason: "manual".into(),
                chats: Vec::new(),
                unread_snapshot: json!({}),
            },
        )
        .await
        .expect("Telegram tools");
    let profile = tools
        .iter()
        .find(|tool| tool.id == "telegram_person_profile")
        .expect("Telegram profile tool")
        .handler
        .invoke(json!({"person_id":person.person_id}))
        .await
        .expect("Telegram profile");
    assert_eq!(profile["name"], "Profile Human");
    assert_eq!(profile["signature"], "Profile signature");
    assert!(profile.get("team_role").is_none());

    let sent = host
        .send_message(
            alpha,
            SendMessageSpec {
                channel_id: ChannelId::new("group_chat").expect("channel id"),
                chat_id: "-1001".into(),
                message_id: MessageId::new(1).expect("message id"),
                idempotency_key: "telegram-e2e-send-1".into(),
                content: "hello @Beta[beta]".into(),
                reply_to: None,
                mentions: vec![beta],
                attachments: Vec::new(),
                force: false,
                provider_agent_config: json!({}),
            },
        )
        .await
        .expect("send cross-Agent mention");
    assert_eq!(sent.status, SendMessageStatus::Sent);

    let bodies = requests.lock().expect("request journal");
    let send = bodies
        .iter()
        .find(|request| request.path.ends_with("/sendMessage"))
        .expect("sendMessage request");
    let body: Value = serde_json::from_slice(&send.body).expect("sendMessage JSON body");
    let text = body["text"].as_str().expect("rendered Telegram text");
    assert!(text.contains("tg://user?id=200"));
    assert!(text.contains("Beta Bot"));

    drop(bodies);
    host.send_message(
        AgentId::new("alpha").expect("alpha agent id"),
        SendMessageSpec {
            channel_id: ChannelId::new("group_chat").expect("channel id"),
            chat_id: "-1001".into(),
            message_id: MessageId::new(2).expect("message id"),
            idempotency_key: "telegram-e2e-send-2".into(),
            content: "reply to an edited inbound message".into(),
            reply_to: Some(MessageId::new(1).expect("reply target")),
            mentions: Vec::new(),
            attachments: Vec::new(),
            force: false,
            provider_agent_config: json!({}),
        },
    )
    .await
    .expect("reply to edited inbound message");
    let bodies = requests.lock().expect("request journal");
    let reply: Value = bodies
        .iter()
        .filter(|request| request.path.ends_with("/sendMessage"))
        .nth(1)
        .map(|request| serde_json::from_slice(&request.body).expect("reply JSON body"))
        .expect("reply sendMessage request");
    assert_eq!(reply["reply_parameters"]["message_id"], 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit live opt-in, the secure Manager/Agent fixture, and an exclusive authorized chat"]
async fn telegram_live_gate() {
    let run_id = env::var("LANKIDE_E2E_RUN_ID").expect("LANKIDE_E2E_RUN_ID is required");
    validate_run_id(&run_id).expect("invalid Telegram live run id");
    let marker = format!("[lankide-e2e:{run_id}]");
    assert_eq!(env::var("LANKIDE_E2E_LIVE").as_deref(), Ok("1"));
    let artifact = Artifact::new(
        std::path::Path::new("target/e2e-artifacts"),
        "provider-telegram",
        &run_id,
    )
    .expect("create E2E artifact");
    let result = run_telegram_scenario(&run_id, &marker, &artifact).await;
    artifact.finish(result).expect("Telegram live gate failed");
}

struct LivePrerequisites {
    config: ProbeConfig,
}
impl LivePrerequisites {
    fn recheck(&self) -> Result<(), String> {
        let report = run_prerequisite_probe(&self.config);
        if report.is_green() {
            Ok(())
        } else {
            Err(report.render_text())
        }
    }
}

#[derive(Clone)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Clone)]
struct RepeatingTransport {
    response: Arc<Mutex<Response>>,
}

impl RepeatingTransport {
    fn new(response: Response) -> Self {
        Self {
            response: Arc::new(Mutex::new(response)),
        }
    }
}

impl Transport for RepeatingTransport {
    fn execute(
        &self,
        _request: lankide_provider_telegram::Request,
    ) -> Result<lankide_provider_telegram::Response, TransportError> {
        let response = self
            .response
            .lock()
            .map_err(|_| TransportError::Failed("fixture lock poisoned".to_owned()))?
            .clone();
        Ok(lankide_provider_telegram::Response {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }
}

#[derive(Clone, Default)]
struct IdentityTransport {
    requests: Arc<Mutex<Vec<lankide_provider_telegram::Request>>>,
}

impl Transport for IdentityTransport {
    fn execute(
        &self,
        request: lankide_provider_telegram::Request,
    ) -> Result<lankide_provider_telegram::Response, TransportError> {
        let result = if request.path.ends_with("/getMe") {
            let (id, username, first_name) = if request.path.contains("bot100:alpha") {
                (100, "alpha_bot", "Alpha Bot")
            } else if request.path.contains("bot200:beta") {
                (200, "beta_bot", "Beta Bot")
            } else {
                return Err(TransportError::Failed("unknown Bot token".into()));
            };
            json!({
                "id": id,
                "is_bot": true,
                "first_name": first_name,
                "username": username
            })
        } else if request.path.ends_with("/sendMessage") {
            json!({
                "message_id": 91,
                "chat": {"id": -1001, "type": "group", "title": "Test Group"},
                "date": 1700000000,
                "text": "sent"
            })
        } else if request.path.ends_with("/getChat") {
            json!({
                "id":300,
                "type":"private",
                "first_name":"Profile",
                "last_name":"Human",
                "bio":"Profile signature"
            })
        } else {
            return Err(TransportError::Failed(
                "unexpected Telegram operation".into(),
            ));
        };
        self.requests
            .lock()
            .map_err(|_| TransportError::Failed("request journal poisoned".into()))?
            .push(request);
        Ok(lankide_provider_telegram::Response {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(&json!({"ok": true, "result": result}))
                .map_err(|error| TransportError::Failed(error.to_string()))?,
        })
    }
}

fn existing_agent_spec(name: &str, username: &str) -> AgentSpec {
    AgentSpec {
        name: name.into(),
        role: String::new(),
        bio: String::new(),
        avatar: None,
        soul: String::new(),
        computer_id: "fixture-computer".into(),
        harness_id: "pi".into(),
        model: String::new(),
        thinking_level: "low".into(),
        auto_reaction: None,
        cli_path: None,
        environment_variables: Vec::new(),
        provider_agent_config: json!({
            "mode": "existing",
            "bot_username": username,
            "skip_profile_sync": true,
        }),
    }
}

fn validate_run_id(run_id: &str) -> Result<(), &'static str> {
    if run_id.is_empty() || run_id.len() > 80 {
        return Err("run id must contain 1..80 characters");
    }
    if !run_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("run id contains an unsafe path character");
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct TelegramFixture {
    chat_id: i64,
    provider_id: String,
    agent_id: String,
    account_id: String,
    username: String,
    token: String,
    manager_account_id: String,
    manager_token: String,
}

fn load_fixture() -> Result<TelegramFixture, String> {
    let config = ProbeConfig::from_environment(GateSelection::TelegramLiveMutating, true);
    let fixture = lankide_testkit::read_telegram_fixture(&config.telegram_fixture)?;
    let chat_id = config
        .telegram_mutating_chat_id
        .as_deref()
        .unwrap_or(&config.authorized_telegram_chat_id)
        .parse::<i64>()
        .map_err(|_| "Telegram E2E chat ID is invalid".to_owned())?;
    if chat_id.to_string() != config.authorized_telegram_chat_id
        && config.telegram_mutating_chat_id.is_none()
    {
        return Err("Telegram E2E mutation is not explicitly authorized".to_owned());
    }

    if !fixture.chat_ids.contains(&chat_id.to_string()) {
        return Err("Telegram authorized chat is not present in the fixture".into());
    }
    Ok(TelegramFixture {
        chat_id,
        provider_id: fixture.provider_id,
        agent_id: fixture.agent_id,
        account_id: fixture.account_id,
        username: fixture.username,
        token: fixture.token,
        manager_account_id: fixture.manager_account_id,
        manager_token: fixture.manager_token,
    })
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
    offset_failures: AtomicI64,
    values: Mutex<BTreeMap<String, Vec<u8>>>,
    messages: Mutex<BTreeMap<i64, ProviderMessage>>,
    resources: Mutex<BTreeMap<i64, ProviderResource>>,
    people: Mutex<BTreeMap<String, ProviderPersonIdentity>>,
}

impl MemoryStorage {
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
            if key.ends_with("/poll_offset")
                && self
                    .offset_failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                        (remaining > 0).then(|| remaining - 1)
                    })
                    .is_ok()
            {
                return Err(lankide_provider::ProviderError::External {
                    operation: "offset_write",
                    retryable: true,
                });
            }
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
}

impl ResourceSaver for LiveResourceSaver {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, ResourceRef> {
        Box::pin(async move {
            let resource_id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(ResourceRef {
                resource_id,
                file_name: upload.file_name,
                media_type: upload.media_type,
                size_bytes: upload.bytes.len() as u64,
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

fn services_with_storage(
    values: BTreeMap<String, String>,
    logger: Arc<LiveLogger>,
    storage: Arc<MemoryStorage>,
) -> HostServices {
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

async fn run_telegram_scenario(
    run_id: &str,
    marker: &str,
    artifact: &Artifact,
) -> Result<(), String> {
    let fixture = load_fixture()?;
    let probe = LivePrerequisites {
        config: ProbeConfig::from_environment(GateSelection::TelegramLiveMutating, true),
    };
    probe.recheck()?;
    exercise_telegram_contract()?;

    let agent_client = TelegramClient::new(
        ReqwestTransport::new(),
        Credentials::new(fixture.account_id.clone(), fixture.token.clone()),
    )
    .map_err(|_| "Telegram Agent Bot client could not be created".to_owned())?;
    let agent = tokio::task::spawn_blocking(move || agent_client.verify_credentials())
        .await
        .map_err(|_| "Telegram Agent Bot verification task failed".to_owned())?
        .map_err(|error| format!("Telegram Agent Bot credential verification failed: {error}"))?;
    artifact.record("verify_agent_bot", &agent.id.to_string(), "ok");

    let manager_client = TelegramClient::new(
        ReqwestTransport::new(),
        Credentials::new(
            fixture.manager_account_id.clone(),
            fixture.manager_token.clone(),
        ),
    )
    .map_err(|_| "Telegram Manager Bot client could not be created".to_owned())?;
    let manager = tokio::task::spawn_blocking(move || manager_client.verify_credentials())
        .await
        .map_err(|_| "Telegram Manager Bot verification task failed".to_owned())?
        .map_err(|error| format!("Telegram Manager Bot credential verification failed: {error}"))?;
    artifact.record("verify_manager_bot", &manager.id.to_string(), "ok");

    // A zero-timeout, zero-offset read exercises the real update endpoint
    // without advancing the Bot's acknowledged cursor.
    let readonly_client = TelegramClient::new(
        ReqwestTransport::new(),
        Credentials::new(fixture.account_id.clone(), fixture.token.clone()),
    )
    .map_err(|_| "Telegram readonly client could not be created".to_owned())?;
    let updates = tokio::task::spawn_blocking(move || {
        readonly_client.get_updates(&PollRequest {
            offset: 0,
            timeout_seconds: 0,
            ..PollRequest::default()
        })
    })
    .await
    .map_err(|_| "Telegram update read task failed".to_owned())?
    .map_err(|_| "Telegram update endpoint failed".to_owned())?;
    artifact.record("read_updates", &updates.len().to_string(), "ok");

    let workspace_id = WorkspaceId::new("telegram_e2e")
        .map_err(|_| "invalid Telegram E2E workspace id".to_owned())?;
    let agent_id =
        AgentId::new(&fixture.agent_id).map_err(|_| "invalid Telegram E2E Agent id".to_owned())?;
    let channel_id =
        ChannelId::new("group_chat").map_err(|_| "invalid Telegram group channel id".to_owned())?;
    let logger = Arc::new(LiveLogger::default());
    let storage = Arc::new(MemoryStorage::default());
    let host = ProviderHost::new(
        TelegramProvider::new(),
        workspace_id,
        json!({
            "provider_id": fixture.provider_id,
            "account_id": fixture.account_id,
            "bot_token_ref": "bot_token",
            "polling": false,
        }),
        services_with_storage(
            BTreeMap::from([("bot_token".to_owned(), fixture.token.clone())]),
            logger.clone(),
            storage.clone(),
        ),
    )
    .map_err(|_| "Telegram ProviderHost could not be created".to_owned())?;
    host.init()
        .await
        .map_err(|_| "Telegram ProviderHost initialization failed".to_owned())?;
    host.create_agent(
        agent_id.clone(),
        AgentSpec {
            name: agent.display_name(),
            role: String::new(),
            bio: String::new(),
            avatar: None,
            soul: String::new(),
            computer_id: "fixture-computer".into(),
            harness_id: "pi".into(),
            model: String::new(),
            thinking_level: "low".into(),
            auto_reaction: None,
            cli_path: None,
            environment_variables: Vec::new(),
            provider_agent_config: json!({
                "mode": "existing",
                "bot_username": if agent.username.trim().is_empty() {
                    fixture.username.clone()
                } else {
                    agent.username.clone()
                },
                "skip_profile_sync": true,
            }),
        },
    )
    .await
    .map_err(|_| "Telegram Agent Bot initialization failed".to_owned())?;
    artifact.record("provider_init", &fixture.provider_id, "ok");

    let mut sent_ids = Vec::<String>::new();
    let scenario = tokio::time::timeout(Duration::from_secs(360), async {
        probe
            .recheck()
            .map_err(|error| format!("Telegram mutation recheck failed: {error}"))?;
        let first = host
            .send_message(
                agent_id.clone(),
                SendMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.to_string(),
                    message_id: MessageId::new(1)
                        .map_err(|_| "invalid Telegram internal message id".to_owned())?,
                    idempotency_key: format!("{run_id}-telegram-message"),
                    content: format!("{marker} provider message @Agent[{}]", fixture.agent_id),
                    reply_to: None,
                    mentions: vec![agent_id.clone()],
                    attachments: Vec::new(),
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .map_err(|_| "Telegram Provider message send failed".to_owned())?;
        if first.status != SendMessageStatus::Sent {
            return Err("Telegram Provider message send did not return Sent".to_owned());
        }
        let first_id = first
            .provider_message_id
            .ok_or_else(|| "Telegram Provider message returned no message ID".to_owned())?;
        sent_ids.push(first_id.clone());
        artifact.record("provider_send", &first_id, "ok");
        storage.insert_message(ProviderMessage {
            message_id: MessageId::new(1)
                .map_err(|_| "invalid Telegram internal message id".to_owned())?,
            provider_message_id: Some(first_id.clone()),
            channel_id: channel_id.clone(),
            chat_id: fixture.chat_id.to_string(),
        });

        host.edit_message(
            agent_id.clone(),
            EditMessageSpec {
                channel_id: channel_id.clone(),
                chat_id: fixture.chat_id.to_string(),
                message_id: MessageId::new(1)
                    .map_err(|_| "invalid Telegram edited message id".to_owned())?,
                provider_message_id: first_id.clone(),
                content: format!(
                    "{marker} provider message edited @Agent[{}]",
                    fixture.agent_id
                ),
                provider_agent_config: json!({}),
            },
        )
        .await
        .map_err(|_| "Telegram Provider message edit failed".to_owned())?;
        artifact.record("provider_edit", &first_id, "ok");

        probe
            .recheck()
            .map_err(|error| format!("Telegram mutation recheck failed: {error}"))?;
        let reply = host
            .send_message(
                agent_id.clone(),
                SendMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.to_string(),
                    message_id: MessageId::new(2)
                        .map_err(|_| "invalid Telegram reply message id".to_owned())?,
                    idempotency_key: format!("{run_id}-telegram-reply"),
                    content: format!("{marker} reply"),
                    reply_to: Some(
                        MessageId::new(1)
                            .map_err(|_| "invalid Telegram reply target id".to_owned())?,
                    ),
                    mentions: Vec::new(),
                    attachments: Vec::new(),
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .map_err(|_| "Telegram Provider reply failed".to_owned())?;
        let reply_id = reply
            .provider_message_id
            .ok_or_else(|| "Telegram Provider reply returned no message ID".to_owned())?;
        sent_ids.push(reply_id.clone());
        artifact.record("provider_reply", &reply_id, "ok");

        probe
            .recheck()
            .map_err(|error| format!("Telegram mutation recheck failed: {error}"))?;
        let attachment_bytes = format!("{marker} attachment").into_bytes();
        let attachment_resource = ResourceRef {
            resource_id: 1,
            file_name: format!("{marker}-attachment.txt"),
            media_type: "text/plain".into(),
            size_bytes: attachment_bytes.len() as u64,
            sha256: "0".repeat(64),
        };
        storage.insert_resource(ProviderResource {
            resource: attachment_resource.clone(),
            bytes: attachment_bytes,
        });
        let attachment = host
            .send_message(
                agent_id.clone(),
                SendMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.to_string(),
                    message_id: MessageId::new(3)
                        .map_err(|_| "invalid Telegram attachment message id".to_owned())?,
                    idempotency_key: format!("{run_id}-telegram-attachment"),
                    content: String::new(),
                    reply_to: None,
                    mentions: Vec::new(),
                    attachments: vec![MessageAttachment {
                        resource: attachment_resource,
                        caption: None,
                    }],
                    force: false,
                    provider_agent_config: json!({}),
                },
            )
            .await
            .map_err(|_| "Telegram Provider attachment send failed".to_owned())?;
        let attachment_id = attachment
            .provider_message_id
            .ok_or_else(|| "Telegram attachment returned no message ID".to_owned())?;
        sent_ids.push(attachment_id.clone());
        artifact.record("provider_attachment", &attachment_id, "ok");

        probe
            .recheck()
            .map_err(|error| format!("Telegram mutation recheck failed: {error}"))?;
        host.auto_reaction(
            agent_id.clone(),
            AutoReactionSpec {
                auto_reaction: "👍".to_owned(),
                channel_id: channel_id.clone(),
                chat_id: fixture.chat_id.to_string(),
                trigger_messages: vec![
                    MessageId::new(1)
                        .map_err(|_| "invalid Telegram reaction target id".to_owned())?,
                ],
                provider_agent_config: json!({}),
            },
        )
        .await
        .map_err(|_| "Telegram Provider reaction failed".to_owned())?;
        artifact.record("provider_reaction", &first_id, "ok");
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "telegram live scenario exceeded hard timeout".to_owned())
    .and_then(|result| result);

    if let Some(first_id) = sent_ids.first().and_then(|value| value.parse::<i64>().ok()) {
        let cleanup_client = TelegramClient::new(
            ReqwestTransport::new(),
            Credentials::new(fixture.account_id.clone(), fixture.token.clone()),
        );
        let chat_id = fixture.chat_id;
        let result = match cleanup_client {
            Ok(client) => tokio::task::spawn_blocking(move || {
                client.set_message_reaction(chat_id, first_id, None)
            })
            .await
            .ok()
            .and_then(Result::ok),
            Err(_) => None,
        };
        artifact.record_cleanup(
            "provider_reaction_remove",
            &first_id.to_string(),
            result.is_some(),
        );
    }
    for (index, provider_message_id) in sent_ids.iter().rev().enumerate() {
        let result = if probe.recheck().is_ok() {
            host.recall_message(
                agent_id.clone(),
                RecallMessageSpec {
                    channel_id: channel_id.clone(),
                    chat_id: fixture.chat_id.to_string(),
                    message_id: MessageId::new(100 + index as i64)
                        .map_err(|_| "invalid Telegram cleanup message id".to_owned())?,
                    provider_message_id: provider_message_id.clone(),
                    provider_agent_config: Value::Null,
                },
            )
            .await
        } else {
            Err(lankide_provider::ProviderError::External {
                operation: "telegram_cleanup_guard",
                retryable: false,
            })
        };
        artifact.record_cleanup("provider_recall", provider_message_id, result.is_ok());
    }
    let shutdown = tokio::time::timeout(Duration::from_secs(30), host.shutdown()).await;
    artifact.record_cleanup("provider_shutdown", "host", matches!(shutdown, Ok(Ok(()))));
    artifact.complete_cleanup();
    if scenario.is_err()
        && let Ok(messages) = logger.messages.lock()
    {
        for message in messages.iter() {
            artifact.record_diagnostic("provider_log", message);
        }
    }
    scenario
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn telegram_polling_recovers_transport_and_acceptance_failures() {
    #[derive(Clone, Default)]
    struct PollFailureTransport {
        identity: IdentityTransport,
        polls: Arc<AtomicI64>,
        offsets: Arc<Mutex<Vec<i64>>>,
    }
    impl Transport for PollFailureTransport {
        fn execute(
            &self,
            request: lankide_provider_telegram::Request,
        ) -> Result<lankide_provider_telegram::Response, TransportError> {
            if !request.path.ends_with("/getUpdates") {
                return self.identity.execute(request);
            }
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let offset = body["offset"].as_i64().unwrap_or_default();
            self.offsets.lock().unwrap().push(offset);
            if self.polls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(TransportError::Failed("transient test disconnect".into()));
            }
            let result = if offset < 42 {
                json!([{"update_id": 41, "message": {
                    "message_id": 8, "date": 1700000000,
                    "from": {"id": 300, "is_bot": false, "first_name": "Human"},
                    "chat": {"id": 300, "type": "private", "first_name": "Human"},
                    "text": "retry this message"
                }}])
            } else {
                std::thread::sleep(Duration::from_millis(20));
                json!([])
            };
            Ok(lankide_provider_telegram::Response {
                status: 200,
                headers: Vec::new(),
                body: serde_json::to_vec(&json!({"ok": true, "result": result})).unwrap(),
            })
        }
    }
    let transport = PollFailureTransport::default();
    let observed = transport.clone();
    let storage = Arc::new(MemoryStorage::default());
    storage.offset_failures.store(1, Ordering::SeqCst);
    let host = ProviderHost::new(
        TelegramProvider::new().with_transport(transport),
        WorkspaceId::new("telegram_recovery").unwrap(),
        json!({"poll_timeout_seconds": 0}),
        services_with_storage(
            BTreeMap::from([("agent:alpha:bot_token".into(), "100:alpha".into())]),
            Arc::new(LiveLogger::default()),
            storage.clone(),
        ),
    )
    .unwrap();
    host.init().await.unwrap();
    host.create_agent(
        AgentId::new("alpha").unwrap(),
        existing_agent_spec("Alpha", "alpha_bot"),
    )
    .await
    .unwrap();
    let mut person_id = None;
    for attempt in 0..3 {
        let delivery = tokio::time::timeout(Duration::from_secs(3), host.next_event())
            .await
            .unwrap()
            .unwrap();
        assert!(
            observed
                .offsets
                .lock()
                .unwrap()
                .iter()
                .all(|offset| *offset == 0),
            "remote offset advanced before durable acceptance and offset persistence"
        );
        let (event, receipt) = delivery.into_parts();
        assert!(PersonId::new(event.sender.id.clone()).is_ok());
        if let Some(expected) = &person_id {
            assert_eq!(&event.sender.id, expected);
        } else {
            person_id = Some(event.sender.id.clone());
        }
        if attempt == 0 {
            receipt.reject();
        } else {
            receipt.accept();
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if observed.offsets.lock().unwrap().contains(&42) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        storage
            .get("telegram/telegram_recovery/agent/alpha/poll_offset")
            .await
            .unwrap(),
        Some(b"42".to_vec())
    );
    host.shutdown().await.unwrap();
}
