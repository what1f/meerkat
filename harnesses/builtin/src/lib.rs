#![forbid(unsafe_code)]

//! Built-in Agent: the bundled `meerkat-agent` binary driven over ACP by the
//! embedded JavaScript adapter. Unlike the other Harnesses it keeps its own
//! account, so this crate also owns the Machine-shared home where that
//! account lives.

use base64::Engine;
use meerkat_harness::{
    FailureCategory, HarnessAccount, HarnessAccountMethod, HarnessCapability, HarnessConfig,
    HarnessCredential, HarnessDefaults, HarnessError, HarnessErrorCode, HarnessFactory,
    HarnessFailure, HarnessFuture, HarnessInfo, HarnessSession, HarnessTestPlan, JsHarnessFactory,
    NativeModel, ProbeResult, SessionRequest,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const HARNESS_ID: &str = "builtin";
pub const EXECUTABLE: &str = "meerkat-agent";
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
const MAX_OUTPUT_TOKENS: u32 = 128_000;
/// Room kept below the window when compaction starts.
const COMPACTION_RESERVE: u32 = 33_000;
/// Thinking levels offered on API-key connections; "disable" is sent as "none".
const API_KEY_LEVELS: [&str; 6] = ["disable", "low", "medium", "high", "xhigh", "max"];

/// Locate the binary: next to this executable, then the app bundle's
/// `Resources/bin`, then PATH (same order as the Machine's capability CLI).
pub fn discover() -> Option<PathBuf> {
    let name = format!("{EXECUTABLE}{}", std::env::consts::EXE_SUFFIX);
    let executable = std::env::current_exe().ok()?;
    let mut candidates = vec![executable.with_file_name(&name)];
    if let Some(contents) = executable
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "Contents"))
    {
        candidates.push(contents.join("Resources/bin").join(&name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join(&name)));
    }
    candidates.into_iter().find(|path| executable_file(path))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[derive(Clone, Debug)]
pub struct BuiltinHarness {
    inner: JsHarnessFactory,
    home: PathBuf,
    context_window: u32,
    writes: Arc<Mutex<()>>,
}

impl BuiltinHarness {
    /// `home` is the Machine-shared home, `<machine root>/.machine/builtin-agent`.
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            inner: JsHarnessFactory::new(HARNESS_ID, "Meerkat", include_str!("../plugin.js"))
                .expect("valid embedded Harness plugin"),
            home: home.into(),
            context_window: DEFAULT_CONTEXT_WINDOW,
            writes: Arc::default(),
        }
    }

    /// The context window every model is budgeted for. Zero is ignored.
    pub fn with_context_window(mut self, tokens: u32) -> Self {
        if tokens > 0 {
            self.context_window = tokens;
        }
        self
    }

    pub fn with_pid_journal(mut self, path: impl Into<PathBuf>) -> Self {
        self.inner = self.inner.with_pid_journal(path);
        self
    }

    pub fn home(&self) -> &Path {
        &self.home
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

    /// Add the private environment every native process needs.
    fn prepared(&self, mut config: HarnessConfig) -> Result<HarnessConfig, HarnessError> {
        create_private_dir(&self.home).map_err(io_error)?;
        let environment = &mut config.environment;
        environment.insert("FX_HOME".into(), self.home.display().to_string());
        environment.insert("FX_AUTO_UPGRADE".into(), "0".into());
        environment.insert("FX_DISABLE_KEYCHAIN".into(), "1".into());
        environment.insert("FX_NO_OPEN_BROWSER".into(), "1".into());
        environment.insert("FX_HERDR".into(), "0".into());
        environment.insert(
            "FX_AUTO_COMPACT_TOKENS".into(),
            self.compact_at_tokens().to_string(),
        );
        environment.remove(API_KEY_ENV);
        if let Some(key) = self.api_key() {
            environment.insert(API_KEY_ENV.into(), key);
        }
        Ok(config)
    }

    fn compact_at_tokens(&self) -> u32 {
        let window = self.context_window;
        window.saturating_sub(COMPACTION_RESERVE).max(window / 2)
    }

    fn model_metadata(&self) -> Value {
        json!({
            "context_window": self.context_window,
            "max_output_tokens": MAX_OUTPUT_TOKENS,
            "supports_tool_use": true,
        })
    }

    fn signed_in_request(
        &self,
        mut request: SessionRequest,
    ) -> Result<SessionRequest, HarnessError> {
        if self.account().is_none() {
            return Err(signed_out());
        }
        self.describe_model(&request.config.model.model_id)?;
        request.config = self.prepared(request.config)?;
        Ok(request)
    }

    /// Keep `model_metadata` of an API-key connection current for the model a
    /// member starts with (members may pick other models than the sign-in's)
    /// and for the window in force. ChatGPT models carry their own metadata.
    fn describe_model(&self, model: &str) -> Result<(), HarnessError> {
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
        write_settings(&self.home, settings)
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
        write_settings(&self.home, settings)
    }
}

fn write_settings(home: &Path, settings: Map<String, Value>) -> Result<(), HarnessError> {
    let bytes = serde_json::to_vec_pretty(&Value::Object(settings))
        .map_err(|error| HarnessError::new(HarnessErrorCode::Io, error.to_string()))?;
    write_private(&home.join(SETTINGS_FILE), &bytes)
}

impl HarnessFactory for BuiltinHarness {
    fn info(&self) -> &HarnessInfo {
        self.inner.info()
    }
    fn defaults(&self) -> HarnessDefaults {
        self.inner.defaults()
    }
    fn test<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, HarnessTestPlan> {
        Box::pin(async move {
            let mut request = self.signed_in_request(request)?;
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
    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            let config = self.prepared(config.clone())?;
            self.inner.probe(&config).await
        })
    }
    fn capabilities(&self, config: &HarnessConfig) -> BTreeSet<HarnessCapability> {
        self.inner.capabilities(config)
    }
    fn models<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, Vec<NativeModel>> {
        Box::pin(async move {
            let Some(account) = self.account() else {
                return Err(signed_out());
            };
            if account.method == HarnessAccountMethod::ApiKey {
                // The connection's own model list, read only when asked.
                let base_url = self
                    .settings()
                    .get("providers")
                    .and_then(|providers| providers.get(API_KEY_PROVIDER))
                    .and_then(|provider| provider.get("base_url"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(signed_out)?;
                let key = self.api_key().ok_or_else(signed_out)?;
                return Ok(list_models(&base_url, &key)
                    .await?
                    .into_iter()
                    .map(|(id, name)| NativeModel {
                        id,
                        name,
                        description: String::new(),
                        thinking_levels: API_KEY_LEVELS.iter().map(|l| (*l).to_owned()).collect(),
                    })
                    .collect());
            }
            let config = self.prepared(config.clone())?;
            self.inner.models(&config).await
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
        Box::pin(async move {
            let request = self.signed_in_request(request)?;
            self.inner.start(request).await
        })
    }
    fn start_before<'a>(
        &'a self,
        request: SessionRequest,
        deadline: std::time::Instant,
    ) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move {
            let request = self.signed_in_request(request)?;
            self.inner.start_before(request, deadline).await
        })
    }
    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move {
            let request = self.signed_in_request(request)?;
            self.inner.resume(request).await
        })
    }
}

fn signed_out() -> HarnessError {
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
