use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio_stream::Stream;

pub type HarnessFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HarnessError>> + Send + 'a>>;
pub type AgentEventStream =
    Pin<Box<dyn Stream<Item = Result<AgentEvent, HarnessError>> + Send + 'static>>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HarnessInfo {
    pub harness_id: String,
    pub display_name: String,
}

impl HarnessInfo {
    pub fn new(
        harness_id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Result<Self, HarnessError> {
        let value = Self {
            harness_id: harness_id.into(),
            display_name: display_name.into(),
        };
        validate_harness_id(&value.harness_id)?;
        if value.display_name.trim().is_empty() {
            return Err(HarnessError::invalid_config("display_name is empty"));
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelRef {
    pub model_id: String,
    pub display_name: String,
}

impl ModelRef {
    pub fn new(model_id: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            display_name: display_name.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct NativeModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub thinking_levels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum HarnessCapability {
    SystemPrompt,
    Streaming,
    Resume,
    Steer,
    Cancel,
    Tools,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessAvailability {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProbeResult {
    pub info: HarnessInfo,
    pub availability: HarnessAvailability,
    pub version: Option<String>,
    pub capabilities: BTreeSet<HarnessCapability>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HarnessConfig {
    pub harness_id: String,
    pub model: ModelRef,
    pub thinking_level: String,
    pub cli_path: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub config_fingerprint: String,
}

impl HarnessConfig {
    pub fn validate(&self) -> Result<(), HarnessError> {
        self.validate_executable()?;
        if self.model.model_id.trim().is_empty() {
            return Err(HarnessError::invalid_config("model_id is empty"));
        }
        if self.thinking_level.trim().is_empty() {
            return Err(HarnessError::invalid_config("thinking_level is empty"));
        }
        if self.config_fingerprint.trim().is_empty() {
            return Err(HarnessError::invalid_config("config_fingerprint is empty"));
        }
        Ok(())
    }
    pub fn validate_executable(&self) -> Result<(), HarnessError> {
        validate_harness_id(&self.harness_id)?;
        if self.cli_path.as_os_str().is_empty() {
            return Err(HarnessError::invalid_config("cli_path is empty"));
        }
        if self.environment.keys().any(|key| key.is_empty()) {
            return Err(HarnessError::invalid_config(
                "environment variable name is empty",
            ));
        }
        Ok(())
    }
}

fn validate_harness_id(value: &str) -> Result<(), HarnessError> {
    let valid = !value.is_empty()
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if !valid {
        return Err(HarnessError::invalid_config("invalid harness_id"));
    }
    if value == "dcc" {
        return Err(HarnessError::invalid_config(
            "dcc is a CLI path choice, not a harness_id",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SessionRequest {
    pub config: HarnessConfig,
    pub working_directory: PathBuf,
    pub system_prompt: String,
    pub tools: Vec<ToolDefinition>,
    pub native_session_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionHealth {
    Starting,
    Healthy,
    Broken,
    Cancelled,
    Closed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SessionEntry {
    pub harness_id: String,
    pub native_session_id: String,
    pub config_fingerprint: String,
    pub health: SessionHealth,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecuteRequest {
    pub input: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AgentEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub kind: AgentEventKind,
}

impl AgentEvent {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.kind,
            AgentEventKind::Completed { .. }
                | AgentEventKind::Failed { .. }
                | AgentEventKind::Cancelled { .. }
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEventKind {
    Started,
    NativeSessionBound {
        native_session_id: String,
    },
    AssistantDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolStarted {
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolFinished {
        tool_call_id: String,
        success: bool,
        output: Value,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    Warning {
        message: String,
    },
    Completed {
        final_output: String,
    },
    Failed {
        code: String,
        message: String,
    },
    Cancelled {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SteerReceipt {
    pub delivery_id: String,
    pub accepted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolResultResource {
    pub local_path: PathBuf,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolResultError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ToolResultRequest {
    pub tool_call_id: String,
    pub success: bool,
    pub content_markdown: String,
    pub structured_content: Value,
    pub resources: Vec<ToolResultResource>,
    pub error: Option<ToolResultError>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolResultReceipt {
    pub tool_call_id: String,
    pub already_applied: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CancelReceipt {
    pub accepted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CloseReceipt {
    pub already_closed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessErrorCode {
    InvalidConfig,
    Unavailable,
    SpawnFailed,
    BindFailed,
    InvalidEvent,
    StreamInterrupted,
    InvalidState,
    Timeout,
    Io,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HarnessError {
    pub code: HarnessErrorCode,
    pub message: String,
}

impl HarnessError {
    pub fn new(code: HarnessErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::new(HarnessErrorCode::InvalidConfig, message)
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for HarnessError {}

pub trait HarnessFactory: Send + Sync {
    fn info(&self) -> &HarnessInfo;
    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult>;
    fn models<'a>(&'a self, _config: &'a HarnessConfig) -> HarnessFuture<'a, Vec<NativeModel>> {
        Box::pin(async {
            Err(HarnessError::new(
                HarnessErrorCode::Unavailable,
                "This Harness does not provide a model directory",
            ))
        })
    }
    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>>;
    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>>;
}

pub trait HarnessSession: Send + Sync {
    fn entry(&self) -> SessionEntry;
    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream>;
    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt>;
    fn tool_result<'a>(
        &'a self,
        request: ToolResultRequest,
    ) -> HarnessFuture<'a, ToolResultReceipt>;
    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt>;
    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt>;
}
