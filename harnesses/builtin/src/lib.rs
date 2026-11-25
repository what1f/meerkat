#![forbid(unsafe_code)]

//! Built-in Agent: in-process Rust execution powered by `jimu-*` crates.
//! Unlike the external CLI Harnesses it keeps its own account, so this crate
//! also owns the Machine-shared home where that account lives.

mod session;

use base64::Engine;
use jimu_ai::oauth::{CodexCredentials, fetch_codex_models, refresh_codex_credentials};
use jimu_ai::{CancelToken, TransportError};
use jimu_http::ReqwestTransport;
use meerkat_harness::{
    FailureCategory, HarnessAccount, HarnessAccountMethod, HarnessCapability, HarnessConfig,
    HarnessCredential, HarnessDefaults, HarnessError, HarnessErrorCode, HarnessFactory,
    HarnessFailure, HarnessFuture, HarnessInfo, HarnessSession, HarnessTestPlan, NativeModel,
    ProbeResult, SessionRequest,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const HARNESS_ID: &str = "builtin";
/// Default model after a ChatGPT sign-in.
pub const CHATGPT_MODEL: &str = "gpt-6-luna";

const SETTINGS_FILE: &str = "settings.json";
const AUTH_FILE: &str = "chatgpt-auth.json";
const API_KEY_FILE: &str = "api-key";
const API_KEY_ENV: &str = "MEERKAT_AGENT_API_KEY";
const CHATGPT_PROVIDER: &str = "codex";
const API_KEY_PROVIDER: &str = "custom";
/// Context window assumed for every model; the Machine may override it.
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;
/// Output room declared for API-key models.
pub(crate) const MAX_OUTPUT_TOKENS: u32 = 128_000;
/// Room kept below the window when compaction starts.
pub(crate) const COMPACTION_RESERVE: u32 = 33_000;
/// Refresh OAuth access tokens when within 60 seconds of expiry.
const REFRESH_MARGIN_MS: u64 = 60_000;
/// Thinking levels offered on API-key connections; "disable" is sent as "none".
const API_KEY_LEVELS: [&str; 6] = ["disable", "low", "medium", "high", "xhigh", "max"];

pub(crate) enum ActiveConnection {
    ApiKey { base_url: String, api_key: String },
    Chatgpt { credentials: CodexCredentials },
}

pub(crate) struct ConnectionSnapshot {
    pub connection: ActiveConnection,
    pub auth_revision: u64,
}

#[derive(Clone, Debug)]
pub struct BuiltinHarness {
    info: HarnessInfo,
    home: PathBuf,
    context_window: u32,
    writes: Arc<Mutex<()>>,
    refresh_gate: Arc<tokio::sync::Mutex<()>>,
    auth_revision: Arc<AtomicU64>,
}

impl BuiltinHarness {
    /// `home` is the Machine-shared home, `<machine root>/.machine/builtin-agent`.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            info: HarnessInfo::new(HARNESS_ID, "Meerkat").expect("valid builtin harness info"),
            home: home.into(),
            context_window: DEFAULT_CONTEXT_WINDOW,
            writes: Arc::default(),
            refresh_gate: Arc::default(),
            auth_revision: Arc::new(AtomicU64::new(1)),
        }
    }

    pub(crate) fn auth_revision(&self) -> u64 {
        self.auth_revision.load(Ordering::SeqCst)
    }

    /// The context window every model is budgeted for. Zero is ignored.
    pub fn with_context_window(mut self, tokens: u32) -> Self {
        if tokens > 0 {
            self.context_window = tokens;
        }
        self
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub(crate) fn context_window(&self) -> u32 {
        self.context_window
    }

    fn settings(&self) -> Map<String, Value> {
        fs::read(self.home.join(SETTINGS_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| match value {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn api_key(&self) -> Option<String> {
        let key = fs::read_to_string(self.home.join(API_KEY_FILE)).ok()?;
        let key = key.trim();
        (!key.is_empty()).then(|| key.to_owned())
    }

    fn configured_model(&self) -> Option<String> {
        let settings = self.settings();
        let provider = settings.get("provider")?.as_str()?;
        settings
            .get("models")?
            .get(provider)?
            .as_str()
            .map(str::to_owned)
    }

    pub(crate) fn active_connection(&self) -> Result<ActiveConnection, HarnessError> {
        self.active_connection_snapshot()
            .map(|snapshot| snapshot.connection)
    }

    /// Atomically snapshot active connection settings, credentials, and `auth_revision`
    /// under `writes` so concurrent credential updates cannot mix an old `base_url` with a new key.
    pub(crate) fn active_connection_snapshot(&self) -> Result<ConnectionSnapshot, HarnessError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let auth_revision = self.auth_revision();
        let settings = self.settings();
        let connection = match settings.get("provider").and_then(Value::as_str) {
            Some(API_KEY_PROVIDER) => {
                let api_key = self.api_key().ok_or_else(signed_out)?;
                let base_url = settings
                    .get("providers")
                    .and_then(|providers| providers.get(API_KEY_PROVIDER))
                    .and_then(|provider| provider.get("base_url"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(signed_out)?;
                ActiveConnection::ApiKey { base_url, api_key }
            }
            Some(CHATGPT_PROVIDER) => {
                let credentials = self.load_codex_credentials()?;
                ActiveConnection::Chatgpt { credentials }
            }
            _ => return Err(signed_out()),
        };
        Ok(ConnectionSnapshot {
            connection,
            auth_revision,
        })
    }

    pub(crate) fn load_codex_credentials(&self) -> Result<CodexCredentials, HarnessError> {
        let text = fs::read_to_string(self.home.join(AUTH_FILE)).map_err(|_| signed_out())?;
        let raw: Value = serde_json::from_str(&text).map_err(|_| signed_out())?;
        let access_token = raw
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(signed_out)?
            .to_owned();
        let refresh_token = raw
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let account_id = raw
            .get("account_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id_token = raw
            .get("id_token")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let expires_at_ms = raw
            .get("expires_at_ms")
            .and_then(Value::as_u64)
            .or_else(|| {
                raw.get("expires_at")
                    .and_then(Value::as_u64)
                    .map(|secs| secs.saturating_mul(1000))
            });
        CodexCredentials {
            access_token,
            refresh_token,
            account_id,
            expires_at_ms,
            id_token,
        }
        .normalized()
        .map_err(|_| signed_out())
    }

    fn save_refreshed_codex_credentials(
        &self,
        expected_revision: u64,
        expected_account_id: Option<&str>,
        expected_refresh_token: Option<&str>,
        credentials: &CodexCredentials,
    ) -> Result<bool, HarnessError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.auth_revision() != expected_revision {
            return Ok(false);
        }
        let Ok(current_disk) = self.load_codex_credentials() else {
            return Ok(false);
        };
        if current_disk.account_id.as_deref() != expected_account_id
            || current_disk.refresh_token.as_deref() != expected_refresh_token
        {
            return Ok(false);
        }
        create_private_dir(&self.home).map_err(io_error)?;
        let bytes = serde_json::to_vec_pretty(credentials)
            .map_err(|error| HarnessError::new(HarnessErrorCode::Io, error.to_string()))?;
        write_private(&self.home.join(AUTH_FILE), &bytes)?;
        Ok(true)
    }

    pub(crate) async fn fresh_codex_credentials(
        &self,
        transport: &ReqwestTransport,
        force_refresh: bool,
    ) -> Result<CodexCredentials, HarnessError> {
        let _refresh_guard = self.refresh_gate.lock().await;
        let revision_before = self.auth_revision();
        let creds = self.load_codex_credentials()?;
        let needs_refresh = force_refresh || creds.is_expiring_within(now_ms(), REFRESH_MARGIN_MS);
        if needs_refresh && creds.refresh_token.is_some() {
            let refreshed = refresh_codex_credentials(transport, &creds, CancelToken::new())
                .await
                .map_err(transport_error)?;
            let saved = self.save_refreshed_codex_credentials(
                revision_before,
                creds.account_id.as_deref(),
                creds.refresh_token.as_deref(),
                &refreshed,
            )?;
            if saved {
                return Ok(refreshed);
            }
            // Account changed or signed out concurrently while refresh was in flight; re-read active state.
            return self.load_codex_credentials();
        }
        Ok(creds)
    }

    fn model_metadata(&self) -> Value {
        json!({
            "context_window": self.context_window,
            "max_output_tokens": MAX_OUTPUT_TOKENS,
            "supports_tool_use": true,
        })
    }

    /// Keep `model_metadata` of an API-key connection current for the model a
    /// member starts with (members may pick other models than the sign-in's)
    /// and for the window in force. ChatGPT models carry their own metadata.
    pub(crate) fn describe_model(&self, model: &str) -> Result<(), HarnessError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut settings = self.settings();
        if settings.get("provider").and_then(Value::as_str) != Some(API_KEY_PROVIDER) {
            return Ok(());
        }
        let before = settings.clone();
        let default_model = settings
            .get("models")
            .and_then(|models| models.get(API_KEY_PROVIDER))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(provider) = settings
            .get_mut("providers")
            .and_then(|providers| providers.get_mut(API_KEY_PROVIDER))
            .and_then(Value::as_object_mut)
        else {
            return Ok(());
        };
        let metadata = object_entry(provider, "model_metadata");
        let mut models: BTreeSet<String> = metadata.keys().cloned().collect();
        models.extend(default_model);
        if !model.is_empty() {
            models.insert(model.to_owned());
        }
        for model in models {
            metadata.insert(model, self.model_metadata());
        }
        if settings == before {
            return Ok(());
        }
        write_settings(&self.home, settings)?;
        Ok(())
    }

    fn store(&self, credential: Option<HarnessCredential>) -> Result<(), HarnessError> {
        let _guard = self
            .writes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        create_private_dir(&self.home).map_err(io_error)?;
        let mut settings = self.settings();
        let providers = object_entry(&mut settings, "providers");
        providers.remove(API_KEY_PROVIDER);
        let empty_providers = providers.is_empty();
        if empty_providers {
            settings.remove("providers");
        }
        match credential {
            Some(HarnessCredential::Chatgpt { auth_json }) => {
                let parsed: Value = serde_json::from_str(&auth_json)
                    .map_err(|_| HarnessError::invalid_config("Sign-in result is not valid"))?;
                if parsed
                    .get("access_token")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err(HarnessError::invalid_config("Sign-in result is not valid"));
                }
                write_private(&self.home.join(AUTH_FILE), auth_json.as_bytes())?;
                remove_file(&self.home.join(API_KEY_FILE))?;
                settings.insert("provider".into(), json!(CHATGPT_PROVIDER));
                let models = object_entry(&mut settings, "models");
                models.remove(API_KEY_PROVIDER);
                models
                    .entry(CHATGPT_PROVIDER)
                    .or_insert_with(|| json!(CHATGPT_MODEL));
            }
            Some(HarnessCredential::ApiKey {
                base_url,
                api_key,
                model,
            }) => {
                let model = model.ok_or_else(|| {
                    HarnessError::invalid_config("No model is available for this connection")
                })?;
                write_private(&self.home.join(API_KEY_FILE), api_key.trim().as_bytes())?;
                remove_file(&self.home.join(AUTH_FILE))?;
                settings.insert("provider".into(), json!(API_KEY_PROVIDER));
                object_entry(&mut settings, "providers").insert(
                    API_KEY_PROVIDER.into(),
                    json!({
                        "protocol": "openai-chat-completions",
                        "base_url": base_url,
                        "auth": {"type": "bearer", "env": API_KEY_ENV},
                        "model_metadata": {model.as_str(): self.model_metadata()},
                    }),
                );
                object_entry(&mut settings, "models").insert(API_KEY_PROVIDER.into(), json!(model));
            }
            None => {
                remove_file(&self.home.join(AUTH_FILE))?;
                remove_file(&self.home.join(API_KEY_FILE))?;
                settings.remove("provider");
                let models = object_entry(&mut settings, "models");
                models.remove(API_KEY_PROVIDER);
                models.remove(CHATGPT_PROVIDER);
                if models.is_empty() {
                    settings.remove("models");
                }
            }
        }
        write_settings(&self.home, settings)?;
        self.auth_revision.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn all_capabilities() -> BTreeSet<HarnessCapability> {
    BTreeSet::from([
        HarnessCapability::SystemPrompt,
        HarnessCapability::Streaming,
        HarnessCapability::Resume,
        HarnessCapability::Steer,
        HarnessCapability::Cancel,
        HarnessCapability::Tools,
    ])
}

fn write_settings(home: &Path, settings: Map<String, Value>) -> Result<(), HarnessError> {
    let bytes = serde_json::to_vec_pretty(&Value::Object(settings))
        .map_err(|error| HarnessError::new(HarnessErrorCode::Io, error.to_string()))?;
    write_private(&home.join(SETTINGS_FILE), &bytes)
}

impl HarnessFactory for BuiltinHarness {
    fn info(&self) -> &HarnessInfo {
        &self.info
    }

    fn defaults(&self) -> HarnessDefaults {
        HarnessDefaults::default()
    }

    fn test<'a>(&'a self, mut request: SessionRequest) -> HarnessFuture<'a, HarnessTestPlan> {
        Box::pin(async move {
            if self.account().is_none() {
                return Err(signed_out());
            }
            self.describe_model(&request.config.model.model_id)?;
            // Test with the model the sign-in configured, not a member choice.
            if let Some(model) = self.configured_model() {
                request.config.model.model_id = model;
            }
            Ok(HarnessTestPlan {
                request,
                input: "Reply with one short greeting. Do not use tools.".into(),
            })
        })
    }

    fn probe<'a>(&'a self, _config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            Ok(ProbeResult {
                info: self.info.clone(),
                availability: meerkat_harness::HarnessAvailability::Available,
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                capabilities: all_capabilities(),
                reason: None,
            })
        })
    }

    fn capabilities(&self, _config: &HarnessConfig) -> BTreeSet<HarnessCapability> {
        all_capabilities()
    }

    fn models<'a>(&'a self, _config: &'a HarnessConfig) -> HarnessFuture<'a, Vec<NativeModel>> {
        Box::pin(async move {
            match self.active_connection()? {
                ActiveConnection::ApiKey { base_url, api_key } => {
                    Ok(list_models(&base_url, &api_key)
                        .await?
                        .into_iter()
                        .map(|(id, name)| NativeModel {
                            id,
                            name,
                            description: String::new(),
                            thinking_levels: API_KEY_LEVELS
                                .iter()
                                .map(|l| (*l).to_owned())
                                .collect(),
                        })
                        .collect())
                }
                ActiveConnection::Chatgpt { .. } => {
                    let transport = ReqwestTransport::new().map_err(|error| {
                        HarnessError::new(HarnessErrorCode::Io, error.to_string())
                    })?;
                    let creds = self.fresh_codex_credentials(&transport, false).await?;
                    let catalog = match fetch_codex_models(
                        &transport,
                        None,
                        &creds,
                        env!("CARGO_PKG_VERSION"),
                        CancelToken::new(),
                    )
                    .await
                    {
                        Ok(models) => models,
                        Err(TransportError::HttpStatus { status: 401, .. })
                            if creds.refresh_token.is_some() =>
                        {
                            let refreshed = self.fresh_codex_credentials(&transport, true).await?;
                            fetch_codex_models(
                                &transport,
                                None,
                                &refreshed,
                                env!("CARGO_PKG_VERSION"),
                                CancelToken::new(),
                            )
                            .await
                            .map_err(transport_error)?
                        }
                        Err(err) => return Err(transport_error(err)),
                    };
                    Ok(catalog
                        .into_iter()
                        .map(|model| {
                            let name = model.display_name.unwrap_or_else(|| model.id.clone());
                            let thinking_levels = model
                                .reasoning_levels
                                .into_iter()
                                .filter(|level| level != "auto")
                                .collect();
                            NativeModel {
                                id: model.id,
                                name,
                                description: String::new(),
                                thinking_levels,
                            }
                        })
                        .collect())
                }
            }
        })
    }

    fn account(&self) -> Option<HarnessAccount> {
        let settings = self.settings();
        match settings.get("provider").and_then(Value::as_str) {
            Some(CHATGPT_PROVIDER) => {
                let text = fs::read_to_string(self.home.join(AUTH_FILE)).ok()?;
                let auth: Value = serde_json::from_str(&text).ok()?;
                let token = auth.get("access_token")?.as_str()?;
                if token.is_empty() {
                    return None;
                }
                let label = ["id_token", "access_token"]
                    .iter()
                    .filter_map(|key| auth.get(*key).and_then(Value::as_str))
                    .find_map(token_email);
                Some(HarnessAccount {
                    method: HarnessAccountMethod::Chatgpt,
                    label,
                })
            }
            Some(API_KEY_PROVIDER) => {
                self.api_key()?;
                let base_url = settings
                    .get("providers")?
                    .get(API_KEY_PROVIDER)?
                    .get("base_url")?
                    .as_str()?;
                Some(HarnessAccount {
                    method: HarnessAccountMethod::ApiKey,
                    label: url_host(base_url),
                })
            }
            _ => None,
        }
    }

    fn has_account(&self) -> bool {
        true
    }

    fn set_credential<'a>(
        &'a self,
        credential: Option<HarnessCredential>,
    ) -> HarnessFuture<'a, ()> {
        Box::pin(async move {
            let credential = match credential {
                Some(HarnessCredential::ApiKey {
                    base_url,
                    api_key,
                    model,
                }) => {
                    // Check the key before saving; default to the first listed model.
                    let listed = list_models(&base_url, api_key.trim()).await?;
                    let model = model.or_else(|| listed.first().map(|(id, _)| id.clone()));
                    Some(HarnessCredential::ApiKey {
                        base_url,
                        api_key,
                        model,
                    })
                }
                other => other,
            };
            self.store(credential)
        })
    }

    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { session::open_session(self.clone(), request, false).await })
    }

    fn start_before<'a>(
        &'a self,
        request: SessionRequest,
        deadline: std::time::Instant,
    ) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move {
            if std::time::Instant::now() >= deadline {
                return Err(HarnessError::new(
                    HarnessErrorCode::Timeout,
                    "Harness startup timed out",
                ));
            }
            tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                session::open_session(self.clone(), request, false),
            )
            .await
            .map_err(|_| {
                HarnessError::new(HarnessErrorCode::Timeout, "Harness startup timed out")
            })?
        })
    }

    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { session::open_session(self.clone(), request, true).await })
    }
}

pub(crate) fn signed_out() -> HarnessError {
    let message = "Meerkat has no model connected";
    let mut error = HarnessError::new(HarnessErrorCode::Unavailable, message);
    error.failure = HarnessFailure {
        code: "harness_signed_out".into(),
        category: FailureCategory::Authentication,
        retryable: false,
        detail: message.into(),
    };
    error
}

pub(crate) fn classify_agent_failure(code: &str, message: &str) -> HarnessFailure {
    let lower = message.to_ascii_lowercase();
    let category = if (lower.contains("model")
        && (lower.contains("not found")
            || lower.contains("not available")
            || lower.contains("not supported")))
        || lower.contains("unknown model")
        || lower.contains("invalid model")
        || lower.contains("does not exist")
        || lower.contains("no such file")
        || lower.contains("permission denied")
    {
        FailureCategory::Configuration
    } else if lower.contains("unauthor")
        || lower.contains("authenticat")
        || lower.contains("invalid api key")
        || lower.contains("invalid key")
        || lower.contains("invalid token")
        || lower.contains("token expired")
        || lower.contains("no api key")
        || lower.contains("missing api key")
        || lower.contains("not logged in")
        || lower.contains("login required")
        || lower.contains("401")
        || lower.contains("forbidden")
        || lower.contains("403")
    {
        FailureCategory::Authentication
    } else if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("usage limit")
        || lower.contains("resource_exhausted")
        || lower.contains("quota")
        || lower.contains("too many requests")
        || lower.contains("429")
    {
        FailureCategory::RateLimit
    } else if lower.contains("cancelled") || lower.contains("canceled") || lower.contains("aborted")
    {
        FailureCategory::Cancelled
    } else if lower.contains("timeout") || lower.contains("timed out") {
        FailureCategory::Timeout
    } else if lower.contains("stream")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("503")
        || lower.contains("502")
        || lower.contains("temporarily unavailable")
    {
        FailureCategory::Transient
    } else {
        FailureCategory::Agent
    };
    HarnessFailure {
        code: format!("harness_{code}"),
        category,
        retryable: matches!(
            category,
            FailureCategory::RateLimit | FailureCategory::Transient | FailureCategory::Timeout
        ),
        detail: message.to_owned(),
    }
}

fn transport_error(error: TransportError) -> HarnessError {
    match error {
        TransportError::HttpStatus {
            status: 401 | 403, ..
        } => {
            let message = "Authentication failed for Meerkat";
            let mut err = HarnessError::new(HarnessErrorCode::Unavailable, message);
            err.failure = HarnessFailure {
                code: "harness_authentication".into(),
                category: FailureCategory::Authentication,
                retryable: false,
                detail: message.into(),
            };
            err
        }
        TransportError::Timeout => {
            HarnessError::new(HarnessErrorCode::Timeout, "Model service request timed out")
        }
        TransportError::Aborted => {
            HarnessError::new(HarnessErrorCode::StreamInterrupted, "Request cancelled")
        }
        other => HarnessError::new(
            HarnessErrorCode::StreamInterrupted,
            format!("Could not reach the model service: {other}"),
        ),
    }
}

/// `GET {base_url}/models` with the key. Failures carry a stable reason code
/// (`invalid_key`, `unreachable`) and never the key or the response.
async fn list_models(base_url: &str, key: &str) -> Result<Vec<(String, String)>, HarnessError> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| connection_error("unreachable", "The connection could not be reached"))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(connection_error("invalid_key", "The key was not accepted"));
    }
    if !status.is_success() {
        return Err(connection_error(
            "unreachable",
            "The connection could not be reached",
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| connection_error("unreachable", "The connection returned no model list"))?;
    let entries = body
        .get("data")
        .or_else(|| body.get("models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id").and_then(Value::as_str)?.to_owned();
            let name = ["display_name", "name"]
                .iter()
                .find_map(|key| entry.get(*key).and_then(Value::as_str))
                .filter(|name| !name.is_empty())
                .map_or_else(|| id.clone(), str::to_owned);
            Some((id, name))
        })
        .collect())
}

fn connection_error(code: &str, message: &str) -> HarnessError {
    let mut error = HarnessError::new(HarnessErrorCode::Unavailable, message);
    error.failure = HarnessFailure {
        code: code.into(),
        category: if code == "invalid_key" {
            FailureCategory::Authentication
        } else {
            FailureCategory::Transient
        },
        retryable: code != "invalid_key",
        detail: message.into(),
    };
    error
}

fn io_error(error: io::Error) -> HarnessError {
    HarnessError::new(HarnessErrorCode::Io, error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn object_entry<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let value = map.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("object")
}

fn token_email(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("https://api.openai.com/profile")
        .and_then(|profile| profile.get("email"))
        .or_else(|| claims.get("email"))
        .and_then(Value::as_str)
        .filter(|email| !email.is_empty())
        .map(str::to_owned)
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split_once(']').map(|(host, _)| host)?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_owned())
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        builder.create(path)
    }
}

/// Write a 0600 file through a temporary sibling and rename.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), HarnessError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temporary = path.with_file_name(format!(".{name}.{}", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(io_error)
}

fn remove_file(path: &Path) -> Result<(), HarnessError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}
