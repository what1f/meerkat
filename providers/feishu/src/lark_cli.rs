//! `lark-cli` for Feishu Agents. Each call returns short-lived credentials as
//! lark-cli environment variables; `meerkat exec` hands them to the user's own
//! lark-cli. User tokens stay in Agent secrets and never reach the Machine
//! except inside that one process.

use crate::client::{Auth, Client as FeishuClient, DeviceTokenPoll, SendMessage, UserTokenGrant};
use crate::http::{ClientError, Transport};
use meerkat_core::PersonId;
use meerkat_provider::{
    AgentRuntimeContext, CliCommandDefinition, Clock, InitContext, InvocationHandler,
    ProviderError, ProviderFuture, SecretValue, TeamRole,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type SharedClient = FeishuClient<Arc<dyn Transport>>;

const REFRESH_AHEAD_MS: i64 = 5 * 60 * 1000;

const DESCRIPTION: &str = "飞书命令行。默认以你自己的机器人身份执行；要以某位成员的身份操作，加 `--as user`。群聊里有多位成员发言时，再加 `--person <成员ID>` 指明是谁。首次以成员身份使用，或提示缺少权限时，运行 `lark-cli auth login --scope \"<权限>\"`，对方会收到授权链接。";

/// Provider-wide coordination: single-use refresh tokens are refreshed by one
/// call at a time, and only the latest authorization per person keeps polling.
#[derive(Default)]
pub(crate) struct LarkCliState {
    refresh_locks: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pending: Mutex<BTreeMap<String, u64>>,
    generation: AtomicU64,
}

impl LarkCliState {
    fn refresh_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .refresh_locks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        locks.entry(key.to_owned()).or_default().clone()
    }

    fn start_authorization(&self, key: &str) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(key.to_owned(), generation);
        generation
    }

    fn authorization_current(&self, key: &str, generation: u64) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            == Some(&generation)
    }

    fn finish_authorization(&self, key: &str, generation: u64) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending.get(key) == Some(&generation) {
            pending.remove(key);
        }
    }
}

pub(crate) fn command(
    context: AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
    init: InitContext,
    state: Arc<LarkCliState>,
    clock: Arc<dyn Clock>,
) -> CliCommandDefinition {
    CliCommandDefinition {
        id: "feishu lark-cli".to_owned(),
        description: DESCRIPTION.to_owned(),
        wraps: "lark-cli".to_owned(),
        private_directories: vec![
            "LARKSUITE_CLI_CONFIG_DIR".to_owned(),
            "LARKSUITE_CLI_DATA_DIR".to_owned(),
            "LARKSUITE_CLI_LOG_DIR".to_owned(),
        ],
        handler: Arc::new(Handler {
            context,
            client,
            auth,
            init,
            state,
            clock,
        }),
    }
}

struct Handler {
    context: AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
    init: InitContext,
    state: Arc<LarkCliState>,
    clock: Arc<dyn Clock>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Identity {
    Bot,
    User,
}

#[derive(Debug, Eq, PartialEq)]
enum Action {
    Run,
    Login { scopes: Vec<String> },
    Refuse(String),
}

#[derive(Debug)]
struct Call {
    arguments: Vec<String>,
    person: Option<String>,
    identity: Identity,
    action: Action,
}

fn parse(input: Vec<String>) -> Call {
    let mut arguments = Vec::with_capacity(input.len());
    let mut person = None;
    let mut identity = Identity::Bot;
    let mut positional = Vec::new();
    let mut scopes = Vec::new();
    let mut unsupported = None;
    let mut values = input.into_iter();
    while let Some(argument) = values.next() {
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (argument.clone(), None),
        };
        match flag.as_str() {
            "--person" => {
                person = inline.or_else(|| values.next());
                continue;
            }
            "--as" | "--scope" => {
                let value = inline.clone().or_else(|| values.next()).unwrap_or_default();
                if flag == "--as" {
                    identity = if value == "user" {
                        Identity::User
                    } else {
                        Identity::Bot
                    };
                } else {
                    scopes.extend(
                        value
                            .split([' ', ','])
                            .filter(|scope| !scope.is_empty())
                            .map(str::to_owned),
                    );
                }
                arguments.push(argument);
                if inline.is_none() {
                    arguments.push(value);
                }
                continue;
            }
            "--domain" | "--recommend" | "--exclude" => unsupported = Some(flag.clone()),
            _ => {}
        }
        if !argument.starts_with('-') && positional.len() < 2 {
            positional.push(argument.clone());
        }
        arguments.push(argument);
    }
    let action = match positional
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["auth", "login", ..] => match unsupported {
            Some(flag) => Action::Refuse(format!(
                "不支持 {flag}，请用 `lark-cli auth login --scope \"<权限>\"` 指明需要的权限。"
            )),
            None if scopes.is_empty() => Action::Refuse(
                "请用 `lark-cli auth login --scope \"<权限>\"` 指明需要的权限。".to_owned(),
            ),
            None => Action::Login { scopes },
        },
        ["auth", "logout", ..] | ["config", ..] | ["profile", ..] => {
            Action::Refuse("这个命令会改动本机的 lark-cli 设置，不能在这里使用。".to_owned())
        }
        _ => Action::Run,
    };
    Call {
        arguments,
        person,
        identity,
        action,
    }
}

impl InvocationHandler for Handler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let input = input
                .as_array()
                .ok_or(ProviderError::InvalidInput { field: "arguments" })?
                .iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or(ProviderError::InvalidInput { field: "arguments" })?;
            let call = parse(input);
            match call.action {
                Action::Refuse(message) => Ok(json!({ "message": message })),
                Action::Login { ref scopes } => {
                    let person = match self.person(call.person.as_deref()).await? {
                        Ok(person) => person,
                        Err(message) => return Ok(json!({ "message": message })),
                    };
                    self.authorize(person, scopes).await
                }
                Action::Run if call.identity == Identity::Bot => {
                    let token = self
                        .client
                        .tenant_token()
                        .await
                        .map_err(|error| error.provider_error(false))?;
                    Ok(self.run(
                        call.arguments,
                        "bot",
                        "LARKSUITE_CLI_TENANT_ACCESS_TOKEN",
                        token,
                    ))
                }
                Action::Run => {
                    let person = match self.person(call.person.as_deref()).await? {
                        Ok(person) => person,
                        Err(message) => return Ok(json!({ "message": message })),
                    };
                    match self.user_token(&person).await? {
                        Some(token) => Ok(self.run(
                            call.arguments,
                            "user",
                            "LARKSUITE_CLI_USER_ACCESS_TOKEN",
                            token,
                        )),
                        None => Ok(json!({ "message": format!(
                            "{} 还没有给你授权。运行 `lark-cli auth login --scope \"<权限>\"`{} 向对方发送授权链接。",
                            person.mention(),
                            person.flag(),
                        ) })),
                    }
                }
            }
        })
    }
}

#[derive(Clone, Debug)]
struct Person {
    id: PersonId,
    name: String,
    explicit: bool,
}

impl Person {
    fn mention(&self) -> String {
        format!("@{}[{}]", self.name, self.id.as_str())
    }

    fn flag(&self) -> String {
        if self.explicit {
            format!(" --person {}", self.id.as_str())
        } else {
            String::new()
        }
    }

    fn secret(&self) -> String {
        format!("lark_user:{}", self.id.as_str())
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct UserToken {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    refresh_expires_at: i64,
    scopes: Vec<String>,
}

impl UserToken {
    fn from_grant(grant: UserTokenGrant, now: i64, requested: &[String]) -> Self {
        let mut scopes = grant
            .scope
            .split([' ', ','])
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if scopes.is_empty() {
            scopes = requested.to_vec();
        }
        Self {
            access_token: grant.access_token,
            refresh_token: grant.refresh_token,
            expires_at: now.saturating_add(grant.expires_in.saturating_mul(1000)),
            refresh_expires_at: now.saturating_add(grant.refresh_expires_in.saturating_mul(1000)),
            scopes,
        }
    }
}

impl Handler {
    fn run(&self, arguments: Vec<String>, identity: &str, key: &str, token: String) -> Value {
        let mut environment = Map::new();
        for (name, value) in [
            (
                "LARKSUITE_CLI_APP_ID",
                self.client.credentials().app_id.trim().to_owned(),
            ),
            (key, token),
            (
                "LARKSUITE_CLI_BRAND",
                self.client.domain().brand().to_owned(),
            ),
            ("LARKSUITE_CLI_DEFAULT_AS", identity.to_owned()),
            ("LARKSUITE_CLI_STRICT_MODE", identity.to_owned()),
            ("LARKSUITE_CLI_NO_UPDATE_NOTIFIER", "1".to_owned()),
            ("LARKSUITE_CLI_NO_SKILLS_NOTIFIER", "1".to_owned()),
        ] {
            environment.insert(name.to_owned(), Value::String(value));
        }
        json!({ "environment": environment, "arguments": arguments })
    }

    /// Only a team member who wrote in this execution can be acted for.
    async fn person(
        &self,
        requested: Option<&str>,
    ) -> Result<Result<Person, String>, ProviderError> {
        let mut candidates = Vec::<Person>::new();
        for (id, name) in human_senders(&self.context.unread_snapshot) {
            let Ok(id) = PersonId::new(id) else { continue };
            if candidates.iter().any(|person| person.id == id) {
                continue;
            }
            let person = self.context.storage().person(&id).await?;
            if person.team_role != TeamRole::External {
                candidates.push(Person {
                    id,
                    name,
                    explicit: false,
                });
            }
        }
        if let Some(requested) = requested {
            return Ok(candidates
                .into_iter()
                .find(|person| person.id.as_str() == requested)
                .map(|person| Person {
                    explicit: true,
                    ..person
                })
                .ok_or_else(|| "只能代表这次给你发消息的团队成员操作。".to_owned()));
        }
        match candidates.len() {
            1 => Ok(Ok(candidates.remove(0))),
            0 => Ok(Err(
                "这次没有团队成员给你发消息，不能以成员身份操作。".to_owned()
            )),
            _ => Ok(Err(format!(
                "有多位成员在和你对话，请加 `--person <成员ID>` 指明代表谁：{}",
                candidates
                    .iter()
                    .map(Person::mention)
                    .collect::<Vec<_>>()
                    .join("、")
            ))),
        }
    }

    async fn stored(&self, person: &Person) -> Result<Option<UserToken>, ProviderError> {
        match self.context.provider_secrets().read(&person.secret()).await {
            Ok(value) => Ok(serde_json::from_str(value.expose_secret()).ok()),
            Err(ProviderError::SecretAccessDenied) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn user_token(&self, person: &Person) -> Result<Option<String>, ProviderError> {
        let now = self.clock.now_millis();
        if let Some(token) = self.stored(person).await?
            && token.expires_at - now > REFRESH_AHEAD_MS
        {
            return Ok(Some(token.access_token));
        }
        let lock = self.state.refresh_lock(&format!(
            "{}/{}/{}",
            self.context.workspace_id.as_str(),
            self.context.agent_id.as_str(),
            person.id.as_str()
        ));
        let _guard = lock.lock().await;
        // Another call may have refreshed while this one waited.
        let Some(token) = self.stored(person).await? else {
            return Ok(None);
        };
        let now = self.clock.now_millis();
        if token.expires_at - now > REFRESH_AHEAD_MS {
            return Ok(Some(token.access_token));
        }
        if token.refresh_token.is_empty() || token.refresh_expires_at <= now {
            return Ok(None);
        }
        let grant = match self.client.refresh_user_token(&token.refresh_token).await {
            Ok(grant) => grant,
            Err(ClientError::Api { .. }) => return Ok(None),
            Err(error) => return Err(error.provider_error(false)),
        };
        let refreshed = UserToken::from_grant(grant, now, &token.scopes);
        let access_token = refreshed.access_token.clone();
        write_token(&self.init, &self.context, person, &refreshed).await?;
        Ok(Some(access_token))
    }

    async fn authorize(
        &self,
        person: Person,
        requested: &[String],
    ) -> Result<Value, ProviderError> {
        let mut scopes = self
            .stored(&person)
            .await?
            .map(|token| token.scopes)
            .unwrap_or_default()
            .into_iter()
            .chain(requested.iter().cloned())
            .chain(["offline_access".to_owned()])
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        scopes.sort();
        let authorization = match self.client.device_authorization(&scopes.join(" ")).await {
            Ok(value) => value,
            Err(ClientError::Api { message, .. }) => {
                return Ok(json!({ "message": format!(
                    "无法发起授权{}。如果是应用还没开通这些权限，请负责人在飞书开放平台开通后再试。",
                    if message.is_empty() { String::new() } else { format!("：{message}") }
                ) }));
            }
            Err(error) => return Err(error.provider_error(false)),
        };
        let identity = self.context.storage().person(&person.id).await?;
        let open_id = crate::identity::person_open_id(
            self.context.storage(),
            identity,
            self.client.credentials().app_id.trim(),
            self.client.clone(),
            self.auth,
        )
        .await?;
        self.client
            .send_message(
                &SendMessage {
                    receive_id: open_id,
                    receive_id_type: "open_id".to_owned(),
                    message_type: "text".to_owned(),
                    content: json!({
                        "text": format!("我需要以你的身份使用飞书，请点这里授权：{}", authorization.verification_url)
                    })
                    .to_string(),
                    idempotency_key: Some(uuid::Uuid::new_v4().to_string()),
                },
                self.auth,
            )
            .await
            .map_err(|error| error.provider_error(false))?;
        let key = format!(
            "{}/{}/{}",
            self.context.workspace_id.as_str(),
            self.context.agent_id.as_str(),
            person.id.as_str()
        );
        let generation = self.state.start_authorization(&key);
        let poll = Poll {
            client: self.client.clone(),
            init: self.init.clone(),
            context: self.context.clone(),
            state: self.state.clone(),
            clock: self.clock.clone(),
            person: person.clone(),
            key,
            generation,
            scopes,
        };
        tokio::spawn(poll.run(
            authorization.device_code,
            authorization.interval,
            authorization.expires_in,
        ));
        Ok(
            json!({ "message": format!("已向 {} 发送授权链接，对方授权后再试。", person.mention()) }),
        )
    }
}

struct Poll {
    client: Arc<SharedClient>,
    init: InitContext,
    context: AgentRuntimeContext,
    state: Arc<LarkCliState>,
    clock: Arc<dyn Clock>,
    person: Person,
    key: String,
    generation: u64,
    scopes: Vec<String>,
}

impl Poll {
    async fn run(self, device_code: String, interval: u64, expires_in: u64) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
        let mut interval = interval;
        while tokio::time::Instant::now() < deadline {
            tokio::select! {
                _ = self.init.shutdown_signal().cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
            }
            if !self.state.authorization_current(&self.key, self.generation) {
                return;
            }
            match self.client.device_token(&device_code).await {
                Ok(DeviceTokenPoll::Pending) | Err(_) => {}
                Ok(DeviceTokenPoll::SlowDown) => interval = (interval + 5).min(60),
                Ok(DeviceTokenPoll::Finished) => break,
                Ok(DeviceTokenPoll::Granted(grant)) => {
                    let token = UserToken::from_grant(grant, self.clock.now_millis(), &self.scopes);
                    if write_token(&self.init, &self.context, &self.person, &token)
                        .await
                        .is_err()
                    {
                        self.init.logger().log(
                            meerkat_provider::LogLevel::Warn,
                            &format!(
                                "Feishu lark-cli authorization could not be saved agent={}",
                                self.context.agent_id
                            ),
                        );
                    }
                    break;
                }
            }
        }
        self.state.finish_authorization(&self.key, self.generation);
    }
}

async fn write_token(
    init: &InitContext,
    context: &AgentRuntimeContext,
    person: &Person,
    token: &UserToken,
) -> Result<(), ProviderError> {
    let value = serde_json::to_string(token).map_err(|_| ProviderError::InvalidInput {
        field: "lark_user_token",
    })?;
    init.secret_writer()
        .write(
            &format!("agent:{}:{}", context.agent_id.as_str(), person.secret()),
            SecretValue::new(value),
        )
        .await
}

/// Human senders of the messages that started this execution.
fn human_senders(snapshot: &Value) -> Vec<(String, String)> {
    let mut senders = Vec::new();
    for chat in snapshot["chats"].as_array().into_iter().flatten() {
        let messages = chat["trigger_messages"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(
                chat.get("mentioned_message")
                    .filter(|value| value.is_object()),
            );
        for message in messages {
            let sender = &message["sender"];
            if sender["kind"] == "human"
                && let Some(id) = sender["id"].as_str()
            {
                let name = sender["name"].as_str().unwrap_or_default().to_owned();
                senders.push((id.to_owned(), name));
            }
        }
    }
    senders
}
