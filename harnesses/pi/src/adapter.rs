use lankide_harness::{
    AgentEvent, AgentEventKind, AgentEventStream, CancelReceipt, CloseReceipt, ExecuteRequest,
    HarnessAvailability, HarnessCapability, HarnessConfig, HarnessError, HarnessErrorCode,
    HarnessFactory, HarnessFuture, HarnessInfo, HarnessSession, ProbeResult, SessionEntry,
    SessionHealth, SessionRequest, SteerReceipt, ToolDefinition, ToolResultReceipt,
    ToolResultRequest,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc as std_mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub const HARNESS_ID: &str = "pi";

const BIND_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_GRACE: Duration = Duration::from_secs(3);
const CLOSE_GRACE: Duration = Duration::from_secs(2);
const TOOL_UI_TITLE: &str = "lankide-tool-result";

/// Pi RPC adapter. `HarnessConfig.environment` accepts
/// `LANKIDE_PI_RECORD_MAX_BYTES` (default 4 MiB, minimum 256 bytes),
/// `LANKIDE_PI_ARTIFACT_DIR`, and an optional `LANKIDE_PI_RECORD_PATH`.
/// Records keep one previous segment and bounded previews of oversized events.
#[derive(Clone, Debug)]
pub struct PiHarness {
    info: HarnessInfo,
}

impl PiHarness {
    pub fn new() -> Self {
        Self {
            info: HarnessInfo::new(HARNESS_ID, "Pi").expect("valid static Harness info"),
        }
    }

    fn validate(&self, config: &HarnessConfig) -> Result<(), HarnessError> {
        config.validate()?;
        if config.harness_id != HARNESS_ID {
            return Err(HarnessError::invalid_config("Pi requires harness_id=pi"));
        }
        if !matches!(
            config.thinking_level.as_str(),
            "" | "off" | "minimal" | "low" | "medium" | "high" | "xhigh"
        ) {
            return Err(HarnessError::invalid_config(
                "unsupported Pi thinking level",
            ));
        }
        Ok(())
    }

    fn open(
        &self,
        request: SessionRequest,
        resume: bool,
        startup_deadline: Option<Instant>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        self.validate(&request.config)?;
        match (resume, request.native_session_id.as_deref()) {
            (true, None | Some("")) => {
                return Err(HarnessError::invalid_config(
                    "resume requires native_session_id",
                ));
            }
            (false, Some(_)) => {
                return Err(HarnessError::invalid_config(
                    "start does not accept native_session_id",
                ));
            }
            _ => {}
        }

        let record_limit = request
            .config
            .environment
            .get("LANKIDE_PI_RECORD_MAX_BYTES")
            .map(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .filter(|limit| *limit >= 256)
                    .ok_or_else(|| HarnessError::invalid_config("invalid Pi record size limit"))
            })
            .transpose()?
            .unwrap_or(4 * 1024 * 1024);
        let paths = SessionPaths::prepare(&request)?;
        fs::create_dir_all(request.working_directory.join("skills"))
            .map_err(io_error("create Agent skills directory"))?;
        if let Some(session_id) = request.native_session_id.as_deref() {
            validate_session_file(&paths.session_dir, &request.working_directory, session_id)?;
        }
        let names = native_tool_names(&request.tools)?;
        write_extension(&paths.extension_path, &request.tools, &names)?;

        let mut command = Command::new(&request.config.cli_path);
        if !request.config.thinking_level.is_empty() {
            command.args(["--thinking", request.config.thinking_level.as_str()]);
        }
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.stderr_path)
            .map_err(io_error("open Pi stderr artifact"))?;
        command
            .args([
                "--mode",
                "rpc",
                "--model",
                request.config.model.model_id.as_str(),
                "--no-extensions",
                "--extension",
            ])
            .arg(&paths.extension_path)
            .args(["--no-themes", "--session-dir"])
            .arg(&paths.session_dir)
            .arg("--skill")
            .arg(request.working_directory.join("skills"))
            .args([
                "--system-prompt",
                request.system_prompt.as_str(),
                match request.permission_policy {
                    lankide_harness::PermissionPolicy::FullAccess => "--approve",
                },
            ])
            .current_dir(&request.working_directory)
            .envs(&request.config.environment)
            .env_remove("LANKIDE_RUNTIME_RUN")
            .env("PI_CODING_AGENT_DIR", &paths.agent_dir)
            .env("PI_CODING_AGENT_SESSION_DIR", &paths.session_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        if let Some(session_id) = request.native_session_id.as_deref() {
            command.args(["--session-id", session_id]);
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.args(&request.config.launch_arguments);
        let mut child = command.spawn().map_err(|error| {
            let mut failure = harness_error(
                HarnessErrorCode::SpawnFailed,
                format!("failed to spawn Pi CLI: {error}"),
            );
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) {
                failure.failure.category = lankide_harness::FailureCategory::Configuration;
                failure.failure.retryable = false;
            }
            failure
        })?;
        record_pid(paths.pid_journal.as_deref(), "spawned", child.id());
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| harness_error(HarnessErrorCode::SpawnFailed, "Pi stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| harness_error(HarnessErrorCode::SpawnFailed, "Pi stdout unavailable"))?;
        let (line_sender, line_receiver) = std_mpsc::channel();
        let rpc_artifact = paths.rpc_artifact.clone();
        thread::spawn(move || read_jsonl(stdout, line_sender, &rpc_artifact, record_limit));
        if let Err(error) = write_line(&mut stdin, &json!({"id":"lankide-bind","type":"get_state"}))
        {
            reap(&mut child, paths.pid_journal.as_deref(), true);
            return Err(error);
        }
        let state = match wait_for_state(&line_receiver, startup_deadline) {
            Ok(state) => state,
            Err(error) => {
                let detail = process_exit_detail(&paths.stderr_path, &mut child, &error.message);
                reap(&mut child, paths.pid_journal.as_deref(), true);
                return Err(harness_error(error.code, detail));
            }
        };
        let native_session_id = state
            .pointer("/data/sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                reap(&mut child, paths.pid_journal.as_deref(), true);
                harness_error(
                    HarnessErrorCode::BindFailed,
                    "Pi get_state omitted sessionId",
                )
            })?
            .to_owned();
        if resume && request.native_session_id.as_deref() != Some(native_session_id.as_str()) {
            reap(&mut child, paths.pid_journal.as_deref(), true);
            return Err(harness_error(
                HarnessErrorCode::BindFailed,
                "Pi resumed a different native session",
            ));
        }

        let inner = Arc::new(Inner {
            entry: Mutex::new(SessionEntry {
                harness_id: HARNESS_ID.to_owned(),
                native_session_id,
                config_fingerprint: request.config.config_fingerprint,
                health: SessionHealth::Healthy,
            }),
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(Some(child)),
            commands: Mutex::new(()),
            active: Mutex::new(None),
            responses: Mutex::new(HashMap::new()),
            command_id: AtomicU64::new(1),
            native_names: names,
            stderr_path: paths.stderr_path,
            pid_journal: paths.pid_journal,
        });
        dispatch(Arc::downgrade(&inner), line_receiver);
        Ok(Box::new(PiSession { inner }))
    }
}

impl Default for PiHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl HarnessFactory for PiHarness {
    fn info(&self) -> &HarnessInfo {
        &self.info
    }

    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            config.validate_executable()?;
            let output = Command::new(&config.cli_path)
                .arg("--version")
                .envs(&config.environment)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            let (availability, version, reason) = match output {
                Ok(output) if output.status.success() => {
                    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    (
                        HarnessAvailability::Available,
                        (!value.is_empty()).then_some(value),
                        None,
                    )
                }
                Ok(_) => (
                    HarnessAvailability::Unavailable,
                    None,
                    Some("Pi --version failed".to_owned()),
                ),
                Err(error) => (
                    HarnessAvailability::Unavailable,
                    None,
                    Some(format!("Pi CLI unavailable: {error}")),
                ),
            };
            Ok(ProbeResult {
                info: self.info.clone(),
                availability,
                version,
                capabilities: BTreeSet::from([
                    HarnessCapability::SystemPrompt,
                    HarnessCapability::Streaming,
                    HarnessCapability::Resume,
                    HarnessCapability::Steer,
                    HarnessCapability::Cancel,
                    HarnessCapability::Tools,
                ]),
                reason,
            })
        })
    }

    fn models<'a>(
        &'a self,
        config: &'a HarnessConfig,
    ) -> HarnessFuture<'a, Vec<lankide_harness::NativeModel>> {
        Box::pin(async move {
            let mut process = lankide_harness::NativeCatalogProcess::spawn(
                config,
                &[
                    "--mode",
                    "rpc",
                    "--no-session",
                    "--no-extensions",
                    "--no-themes",
                ],
            )?;
            let result =
                async {
                    process
                        .send(json!({"id":"catalog","type":"get_available_models"}))
                        .await?;
                    let response = process.receive(|v| v["id"] == "catalog").await?;
                    let models = response
                        .pointer("/data/models")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            harness_error(
                                HarnessErrorCode::InvalidEvent,
                                "Pi did not return a model directory",
                            )
                        })?;
                    models
                        .iter()
                        .map(|model| {
                            let id = model["id"].as_str().filter(|v| !v.is_empty()).ok_or_else(
                                || {
                                    harness_error(
                                        HarnessErrorCode::InvalidEvent,
                                        "Pi returned a model without an ID",
                                    )
                                },
                            )?;
                            let provider = model["provider"]
                                .as_str()
                                .filter(|v| !v.is_empty())
                                .ok_or_else(|| {
                                    harness_error(
                                        HarnessErrorCode::InvalidEvent,
                                        "Pi returned a model without a provider",
                                    )
                                })?;
                            Ok(lankide_harness::NativeModel {
                                id: format!("{provider}/{id}"),
                                name: model["name"].as_str().unwrap_or(id).into(),
                                description: String::new(),
                                thinking_levels: if model["reasoning"] == true {
                                    vec!["off", "minimal", "low", "medium", "high", "xhigh"]
                                } else {
                                    vec!["off"]
                                }
                                .into_iter()
                                .map(str::to_owned)
                                .collect(),
                            })
                        })
                        .collect()
                }
                .await;
            process.close().await;
            result
        })
    }

    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open(request, false, None) })
    }

    fn start_before<'a>(
        &'a self,
        request: SessionRequest,
        deadline: Instant,
    ) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open(request, false, Some(deadline)) })
    }

    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(async move { self.open(request, true, None) })
    }
}

struct SessionPaths {
    agent_dir: PathBuf,
    session_dir: PathBuf,
    extension_path: PathBuf,
    stderr_path: PathBuf,
    rpc_artifact: PathBuf,
    pid_journal: Option<PathBuf>,
}

impl SessionPaths {
    fn prepare(request: &SessionRequest) -> Result<Self, HarnessError> {
        let base = request.working_directory.join(".lankide/pi");
        let agent_dir = request
            .config
            .environment
            .get("PI_CODING_AGENT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join("agent"));
        let session_dir = request
            .config
            .environment
            .get("PI_CODING_AGENT_SESSION_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join("sessions"));
        lankide_harness::prepare_config_overlay(
            &agent_dir,
            lankide_harness::user_config_directory("PI_CODING_AGENT_DIR", ".pi/agent").as_deref(),
            &["auth.json", "models.json", "settings.json"],
        )?;
        fs::create_dir_all(&session_dir).map_err(io_error("create Pi session dir"))?;
        let pid_journal = request
            .config
            .environment
            .get("LANKIDE_PI_PID_JOURNAL")
            .map(PathBuf::from);
        let artifact_dir = request
            .config
            .environment
            .get("LANKIDE_PI_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join("records"));
        fs::create_dir_all(&artifact_dir).map_err(io_error("create Pi artifact dir"))?;
        let suffix = unique();
        Ok(Self {
            agent_dir,
            extension_path: session_dir.join(format!("lankide-extension-{suffix}.ts")),
            stderr_path: artifact_dir.join(format!("pi-{suffix}.stderr.log")),
            rpc_artifact: request
                .config
                .environment
                .get("LANKIDE_PI_RECORD_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|| artifact_dir.join(format!("pi-{suffix}.rpc.jsonl"))),
            session_dir,
            pid_journal,
        })
    }
}

fn unique() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn native_tool_names(tools: &[ToolDefinition]) -> Result<BTreeMap<String, String>, HarnessError> {
    let mut names = BTreeMap::new();
    for (index, tool) in tools.iter().enumerate() {
        if tool.name.trim().is_empty() {
            return Err(HarnessError::invalid_config("tool name is empty"));
        }
        let stem = tool
            .name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        names.insert(format!("lankide_{index}_{stem}"), tool.name.clone());
    }
    Ok(names)
}

fn write_extension(
    path: &Path,
    tools: &[ToolDefinition],
    names: &BTreeMap<String, String>,
) -> Result<(), HarnessError> {
    let definitions = tools
        .iter()
        .map(|tool| {
            let native_name = names
                .iter()
                .find_map(|(native, public)| (public == &tool.name).then_some(native))
                .expect("tool name mapping exists");
            json!({
                "nativeName": native_name,
                "publicName": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    let definitions = serde_json::to_string(&definitions).map_err(|error| {
        harness_error(
            HarnessErrorCode::InvalidConfig,
            format!("encode Pi tools: {error}"),
        )
    })?;
    let source = format!(
        r#"const tools = {definitions};
export default function (pi) {{
  for (const tool of tools) {{
    pi.registerTool({{
      name: tool.nativeName,
      label: tool.publicName,
      description: tool.description,
      parameters: tool.parameters,
      async execute(toolCallId, params, signal, _onUpdate, ctx) {{
        const request = JSON.stringify({{ tool_call_id: toolCallId, tool_name: tool.publicName, arguments: params }});
        const raw = await ctx.ui.input("{TOOL_UI_TITLE}", request, {{ signal }});
        if (raw === undefined) throw new Error("Lankide tool result was cancelled");
        const result = JSON.parse(raw);
        const resources = Array.isArray(result.resources) ? result.resources : [];
        const resourceText = resources.map((item) => `${{item.file_name}}: ${{item.local_path}}`).join("\n");
        const text = [result.content_markdown || "", resourceText].filter(Boolean).join("\n\n") || JSON.stringify(result.structured_content);
        if (!result.success) throw new Error(result.error?.message || text || "Tool failed");
        return {{ content: [{{ type: "text", text }}], details: result.structured_content }};
      }}
    }});
  }}
}}
"#,
    );
    fs::write(path, source).map_err(io_error("write Pi extension"))
}

fn validate_session_file(
    session_dir: &Path,
    working_directory: &Path,
    session_id: &str,
) -> Result<(), HarnessError> {
    let suffix = format!("_{session_id}.jsonl");
    let matches = fs::read_dir(session_dir)
        .map_err(io_error("read Pi session dir"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(harness_error(
            HarnessErrorCode::BindFailed,
            format!("Pi session artifact count is {}, expected 1", matches.len()),
        ));
    }
    let mut lines =
        BufReader::new(fs::File::open(&matches[0]).map_err(io_error("open Pi session artifact"))?)
            .lines();
    let header: Value = lines
        .next()
        .transpose()
        .map_err(io_error("read Pi session header"))?
        .and_then(|line| serde_json::from_str(&line).ok())
        .ok_or_else(|| harness_error(HarnessErrorCode::BindFailed, "invalid Pi session header"))?;
    if header.get("type").and_then(Value::as_str) != Some("session")
        || header.get("id").and_then(Value::as_str) != Some(session_id)
    {
        return Err(harness_error(
            HarnessErrorCode::BindFailed,
            "Pi session header does not match native session id",
        ));
    }
    if let Some(cwd) = header.get("cwd").and_then(Value::as_str) {
        let stored = fs::canonicalize(cwd).map_err(io_error("resolve stored Pi cwd"))?;
        let requested =
            fs::canonicalize(working_directory).map_err(io_error("resolve requested Pi cwd"))?;
        if stored != requested {
            return Err(harness_error(
                HarnessErrorCode::BindFailed,
                "Pi session belongs to another working directory",
            ));
        }
    }
    Ok(())
}

fn wait_for_state(
    receiver: &std_mpsc::Receiver<Result<String, std::io::Error>>,
    startup_deadline: Option<Instant>,
) -> Result<Value, HarnessError> {
    let deadline = Instant::now() + BIND_TIMEOUT;
    let deadline = startup_deadline.map_or(deadline, |limit| deadline.min(limit));
    loop {
        if startup_deadline.is_some() && Instant::now() >= deadline {
            return Err(harness_error(
                HarnessErrorCode::Timeout,
                "Pi get_state timeout",
            ));
        }
        let line = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| match error {
                std_mpsc::RecvTimeoutError::Timeout => {
                    harness_error(HarnessErrorCode::Timeout, "Pi get_state timeout")
                }
                std_mpsc::RecvTimeoutError::Disconnected => {
                    harness_error(HarnessErrorCode::BindFailed, "Pi exited before get_state")
                }
            })?
            .map_err(io_error("read Pi bind event"))?;
        let value: Value = serde_json::from_str(&line).map_err(|error| {
            harness_error(
                HarnessErrorCode::BindFailed,
                format!("invalid Pi bind event: {error}"),
            )
        })?;
        if value.get("type").and_then(Value::as_str) == Some("response")
            && value.get("id").and_then(Value::as_str) == Some("lankide-bind")
        {
            if value.get("success").and_then(Value::as_bool) == Some(true) {
                return Ok(value);
            }
            return Err(harness_error(
                HarnessErrorCode::BindFailed,
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Pi get_state failed"),
            ));
        }
    }
}

struct PiSession {
    inner: Arc<Inner>,
}

struct Inner {
    stderr_path: PathBuf,
    entry: Mutex<SessionEntry>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    commands: Mutex<()>,
    active: Mutex<Option<Active>>,
    responses: Mutex<HashMap<String, std_mpsc::Sender<Result<(), HarnessError>>>>,
    command_id: AtomicU64,
    native_names: BTreeMap<String, String>,
    pid_journal: Option<PathBuf>,
}

struct Active {
    sender: mpsc::Sender<Result<AgentEvent, HarnessError>>,
    next_sequence: u64,
    final_output: String,
    observed_tools: BTreeSet<String>,
    ui_requests: HashMap<String, String>,
    results: BTreeMap<String, ToolResultRequest>,
    cancelling: Option<String>,
}

impl Inner {
    fn health(&self) -> SessionHealth {
        self.entry.lock().expect("Pi entry mutex poisoned").health
    }

    fn set_health(&self, health: SessionHealth) {
        self.entry.lock().expect("Pi entry mutex poisoned").health = health;
    }

    fn write(&self, value: &Value) -> Result<(), HarnessError> {
        let mut guard = self.stdin.lock().expect("Pi stdin mutex poisoned");
        let stdin = guard
            .as_mut()
            .ok_or_else(|| harness_error(HarnessErrorCode::InvalidState, "Pi stdin is closed"))?;
        write_line(stdin, value)
    }

    fn id(&self, operation: &str) -> String {
        format!(
            "lankide-{operation}-{}",
            self.command_id.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn request(&self, id: String, value: Value) -> Result<(), HarnessError> {
        let (sender, receiver) = std_mpsc::channel();
        self.responses
            .lock()
            .expect("Pi response mutex poisoned")
            .insert(id.clone(), sender);
        if let Err(error) = self.write(&value) {
            self.responses
                .lock()
                .expect("Pi response mutex poisoned")
                .remove(&id);
            return Err(error);
        }
        receiver.recv_timeout(COMMAND_TIMEOUT).map_err(|error| {
            self.responses
                .lock()
                .expect("Pi response mutex poisoned")
                .remove(&id);
            match error {
                std_mpsc::RecvTimeoutError::Timeout => {
                    harness_error(HarnessErrorCode::Timeout, "Pi command response timeout")
                }
                std_mpsc::RecvTimeoutError::Disconnected => harness_error(
                    HarnessErrorCode::StreamInterrupted,
                    "Pi command response channel closed",
                ),
            }
        })?
    }

    fn kill(&self, action: &str) {
        self.stdin.lock().expect("Pi stdin mutex poisoned").take();
        if let Some(mut child) = self.child.lock().expect("Pi child mutex poisoned").take() {
            let pid = child.id();
            lankide_harness::kill_process_group(child.id());
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
            record_pid(self.pid_journal.as_deref(), action, pid);
            record_pid(self.pid_journal.as_deref(), "reaped", pid);
        }
    }

    fn force_cancel(&self, reason: String) {
        let active = self.active.lock().expect("Pi active mutex poisoned").take();
        if let Some(active) = active {
            let _ = active.sender.blocking_send(Ok(AgentEvent {
                sequence: active.next_sequence,
                kind: AgentEventKind::Cancelled { reason },
            }));
            self.set_health(SessionHealth::Cancelled);
            self.kill("forced");
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(stdin) = self.stdin.get_mut() {
            stdin.take();
        }
        if let Ok(child) = self.child.get_mut()
            && let Some(mut child) = child.take()
        {
            let pid = child.id();
            lankide_harness::kill_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            record_pid(self.pid_journal.as_deref(), "reaped", pid);
        }
    }
}

impl HarnessSession for PiSession {
    fn process_group_id(&self) -> Option<u32> {
        self.inner
            .child
            .lock()
            .expect("child lock")
            .as_ref()
            .map(Child::id)
    }

    fn entry(&self) -> SessionEntry {
        self.inner
            .entry
            .lock()
            .expect("Pi entry mutex poisoned")
            .clone()
    }

    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async move {
            let _guard = self
                .inner
                .commands
                .lock()
                .expect("Pi command mutex poisoned");
            if self.inner.health() != SessionHealth::Healthy {
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "Pi session is not healthy",
                ));
            }
            let (sender, receiver) = mpsc::channel(64);
            {
                let mut active = self.inner.active.lock().expect("Pi active mutex poisoned");
                if active.is_some() {
                    return Err(harness_error(
                        HarnessErrorCode::InvalidState,
                        "Pi execution is already active",
                    ));
                }
                *active = Some(Active {
                    sender: sender.clone(),
                    next_sequence: 3,
                    final_output: String::new(),
                    observed_tools: BTreeSet::new(),
                    ui_requests: HashMap::new(),
                    results: BTreeMap::new(),
                    cancelling: None,
                });
            }
            sender
                .try_send(Ok(AgentEvent {
                    sequence: 1,
                    kind: AgentEventKind::Started,
                }))
                .map_err(|_| stream_dropped())?;
            sender
                .try_send(Ok(AgentEvent {
                    sequence: 2,
                    kind: AgentEventKind::NativeSessionBound {
                        native_session_id: self.entry().native_session_id,
                    },
                }))
                .map_err(|_| stream_dropped())?;
            let id = self.inner.id("prompt");
            if let Err(error) = self.inner.write(&json!({
                "id":id,
                "type":"prompt",
                "message":request.input,
            })) {
                self.inner
                    .active
                    .lock()
                    .expect("Pi active mutex poisoned")
                    .take();
                self.inner.set_health(SessionHealth::Broken);
                return Err(error);
            }
            Ok(Box::pin(ReceiverStream::new(receiver)) as AgentEventStream)
        })
    }

    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async move {
            let _guard = self
                .inner
                .commands
                .lock()
                .expect("Pi command mutex poisoned");
            let active_guard = self.inner.active.lock().expect("Pi active mutex poisoned");
            let active = active_guard.as_ref().ok_or_else(|| {
                harness_error(
                    HarnessErrorCode::InvalidState,
                    "steer requires an active Pi execution",
                )
            })?;
            if active.cancelling.is_some() {
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "Pi cancellation is in progress",
                ));
            }
            drop(active_guard);
            let id = self.inner.id("steer");
            self.inner
                .request(id.clone(), json!({"id":id,"type":"steer","message":input}))?;
            Ok(SteerReceipt {
                delivery_id,
                accepted: true,
            })
        })
    }

    fn tool_result<'a>(
        &'a self,
        request: ToolResultRequest,
    ) -> HarnessFuture<'a, ToolResultReceipt> {
        Box::pin(async move {
            let _guard = self
                .inner
                .commands
                .lock()
                .expect("Pi command mutex poisoned");
            let mut active_guard = self.inner.active.lock().expect("Pi active mutex poisoned");
            let active = active_guard.as_mut().ok_or_else(|| {
                harness_error(
                    HarnessErrorCode::InvalidState,
                    "tool result requires an active Pi execution",
                )
            })?;
            if !active.observed_tools.contains(&request.tool_call_id) {
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "Pi tool call was not observed",
                ));
            }
            if let Some(applied) = active.results.get(&request.tool_call_id) {
                if applied == &request {
                    return Ok(ToolResultReceipt {
                        tool_call_id: request.tool_call_id,
                        already_applied: true,
                    });
                }
                return Err(harness_error(
                    HarnessErrorCode::InvalidState,
                    "conflicting Pi tool result",
                ));
            }
            let ui_id = active.ui_requests.get(&request.tool_call_id).cloned();
            active
                .results
                .insert(request.tool_call_id.clone(), request.clone());
            drop(active_guard);
            if let Some(ui_id) = ui_id {
                inject_tool_result(&self.inner, &ui_id, &request)?;
            }
            Ok(ToolResultReceipt {
                tool_call_id: request.tool_call_id,
                already_applied: false,
            })
        })
    }

    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async move {
            let _guard = self
                .inner
                .commands
                .lock()
                .expect("Pi command mutex poisoned");
            if self.inner.health() == SessionHealth::Cancelled {
                return Ok(CancelReceipt { accepted: false });
            }
            let mut active_guard = self.inner.active.lock().expect("Pi active mutex poisoned");
            let active = active_guard.as_mut().ok_or_else(|| {
                harness_error(
                    HarnessErrorCode::InvalidState,
                    "cancel requires an active Pi execution",
                )
            })?;
            if active.cancelling.is_some() {
                return Ok(CancelReceipt { accepted: false });
            }
            active.cancelling = Some(reason.clone());
            drop(active_guard);
            let id = self.inner.id("abort");
            // Pi 0.83 can settle an already-steered turn without emitting the
            // correlated abort response. Successful JSONL delivery is the
            // acceptance boundary; agent_end or the grace timeout supplies the
            // authoritative terminal state.
            self.inner.write(&json!({"id":id,"type":"abort"}))?;
            let weak = Arc::downgrade(&self.inner);
            thread::spawn(move || {
                thread::sleep(CANCEL_GRACE);
                if let Some(inner) = weak.upgrade() {
                    let pending = inner
                        .active
                        .lock()
                        .expect("Pi active mutex poisoned")
                        .as_ref()
                        .is_some_and(|active| active.cancelling.is_some());
                    if pending {
                        inner.force_cancel(reason);
                    }
                }
            });
            Ok(CancelReceipt { accepted: true })
        })
    }

    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            let _guard = self
                .inner
                .commands
                .lock()
                .expect("Pi command mutex poisoned");
            if self.inner.health() == SessionHealth::Closed {
                return Ok(CloseReceipt {
                    already_closed: true,
                });
            }
            if self
                .inner
                .active
                .lock()
                .expect("Pi active mutex poisoned")
                .is_some()
            {
                self.inner.force_cancel("session closed".to_owned());
            } else {
                self.inner
                    .stdin
                    .lock()
                    .expect("Pi stdin mutex poisoned")
                    .take();
                let deadline = Instant::now() + CLOSE_GRACE;
                while Instant::now() < deadline {
                    let exited = self
                        .inner
                        .child
                        .lock()
                        .expect("Pi child mutex poisoned")
                        .as_mut()
                        .is_none_or(|child| child.try_wait().ok().flatten().is_some());
                    if exited {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                self.inner.kill("closed");
            }
            self.inner.set_health(SessionHealth::Closed);
            Ok(CloseReceipt {
                already_closed: false,
            })
        })
    }
}

fn inject_tool_result(
    inner: &Inner,
    ui_id: &str,
    request: &ToolResultRequest,
) -> Result<(), HarnessError> {
    let encoded = serde_json::to_string(request).map_err(|error| {
        harness_error(
            HarnessErrorCode::Io,
            format!("encode Pi tool result: {error}"),
        )
    })?;
    inner.write(&json!({
        "type":"extension_ui_response",
        "id":ui_id,
        "value":encoded,
    }))
}

fn dispatch(weak: Weak<Inner>, lines: std_mpsc::Receiver<Result<String, std::io::Error>>) {
    thread::spawn(move || {
        while let Ok(line) = lines.recv() {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            match line {
                Ok(line) => dispatch_line(&inner, &line),
                Err(error) => fail(
                    &inner,
                    harness_error(
                        HarnessErrorCode::StreamInterrupted,
                        format!("Pi stdout failed: {error}"),
                    ),
                ),
            }
        }
        if let Some(inner) = weak.upgrade()
            && !matches!(
                inner.health(),
                SessionHealth::Closed | SessionHealth::Cancelled
            )
        {
            fail(
                &inner,
                harness_error(
                    HarnessErrorCode::StreamInterrupted,
                    inner
                        .child
                        .lock()
                        .expect("child lock")
                        .as_mut()
                        .map(|child| {
                            process_exit_detail(
                                &inner.stderr_path,
                                child,
                                "Pi RPC ended before a terminal event",
                            )
                        })
                        .unwrap_or_else(|| "Pi RPC ended before a terminal event".into()),
                ),
            );
        }
    });
}

fn dispatch_line(inner: &Inner, line: &str) {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            fail(
                inner,
                harness_error(
                    HarnessErrorCode::InvalidEvent,
                    format!("invalid Pi RPC event: {error}"),
                ),
            );
            return;
        }
    };
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    if event_type == "extension_ui_request" {
        tool_ui_request(inner, &value);
        return;
    }
    if event_type == "response" {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            let sender = inner
                .responses
                .lock()
                .expect("Pi response mutex poisoned")
                .remove(id);
            if let Some(sender) = sender {
                let result = if value.get("success").and_then(Value::as_bool) == Some(true) {
                    Ok(())
                } else {
                    Err(harness_error(
                        HarnessErrorCode::InvalidState,
                        value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Pi RPC command failed"),
                    ))
                };
                let _ = sender.send(result);
                return;
            }
        }
        if value.get("command").and_then(Value::as_str) == Some("prompt")
            && value.get("success").and_then(Value::as_bool) == Some(false)
        {
            fail(
                inner,
                harness_error(
                    HarnessErrorCode::InvalidState,
                    value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Pi rejected prompt"),
                ),
            );
        }
        return;
    }

    let mut active_guard = inner.active.lock().expect("Pi active mutex poisoned");
    let Some(active) = active_guard.as_mut() else {
        return;
    };
    let event = match event_type {
        "message_update" => message_delta(active, &value),
        "message_end" => {
            if let Some(message) = value.get("message")
                && let Some(text) = assistant_text(message)
            {
                active.final_output = text;
            }
            None
        }
        "tool_execution_start" => {
            let native_name = value.get("toolName").and_then(Value::as_str).unwrap_or("");
            let Some(public_name) = inner.native_names.get(native_name) else {
                // Native tools execute inside Pi. Only injected tools need a
                // Runtime result; forwarding bash would execute it twice.
                return;
            };
            let call_id = value
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if call_id.is_empty() {
                drop(active_guard);
                fail(
                    inner,
                    harness_error(HarnessErrorCode::InvalidEvent, "Pi tool call id is empty"),
                );
                return;
            }
            active.observed_tools.insert(call_id.clone());
            Some(AgentEventKind::ToolStarted {
                tool_call_id: call_id,
                tool_name: public_name.clone(),
                arguments: value.get("args").cloned().unwrap_or(Value::Null),
            })
        }
        "tool_execution_end" => {
            let call_id = value
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if !active.observed_tools.contains(&call_id) {
                return;
            }
            let output = active
                .results
                .get(&call_id)
                .map(|result| {
                    json!({
                        "content_markdown":result.content_markdown,
                        "structured_content":result.structured_content,
                        "resources":result.resources,
                        "error":result.error,
                    })
                })
                .unwrap_or_else(|| value.get("result").cloned().unwrap_or(Value::Null));
            Some(AgentEventKind::ToolFinished {
                tool_call_id: call_id,
                success: value.get("isError").and_then(Value::as_bool) != Some(true),
                output,
            })
        }
        "agent_end" => {
            let output = value
                .get("messages")
                .and_then(Value::as_array)
                .and_then(|messages| messages.iter().rev().find_map(assistant_text))
                .unwrap_or_else(|| active.final_output.clone());
            let cancelled = active.cancelling.take();
            let failed = last_stop_reason(&value);
            let kind = if let Some(reason) = cancelled {
                AgentEventKind::Cancelled { reason }
            } else if matches!(failed, Some("error" | "aborted")) {
                AgentEventKind::Failed {
                    failure: None,
                    code: failed.unwrap_or("error").to_owned(),
                    message: last_error(&value)
                        .unwrap_or("Pi execution failed")
                        .to_owned(),
                }
            } else {
                AgentEventKind::Completed {
                    final_output: output,
                }
            };
            let kind = classified_event(kind);
            let sequence = active.next_sequence;
            let sender = active.sender.clone();
            let was_cancelled = matches!(kind, AgentEventKind::Cancelled { .. });
            active_guard.take();
            drop(active_guard);
            let _ = sender.blocking_send(Ok(AgentEvent { sequence, kind }));
            inner.set_health(if was_cancelled {
                SessionHealth::Cancelled
            } else {
                SessionHealth::Healthy
            });
            return;
        }
        _ => None,
    };
    let Some(kind) = event else {
        return;
    };
    let sequence = active.next_sequence;
    active.next_sequence += 1;
    let sender = active.sender.clone();
    drop(active_guard);
    let _ = sender.blocking_send(Ok(AgentEvent { sequence, kind }));
}

fn message_delta(active: &mut Active, value: &Value) -> Option<AgentEventKind> {
    let kind = value
        .pointer("/assistantMessageEvent/type")
        .and_then(Value::as_str)?;
    let text = value
        .pointer("/assistantMessageEvent/delta")
        .and_then(Value::as_str)?;
    if text.is_empty() {
        return None;
    }
    match kind {
        "text_delta" => {
            active.final_output.push_str(text);
            Some(AgentEventKind::AssistantDelta {
                text: text.to_owned(),
            })
        }
        "thinking_delta" => Some(AgentEventKind::ReasoningDelta {
            text: text.to_owned(),
        }),
        _ => None,
    }
}

fn tool_ui_request(inner: &Inner, value: &Value) {
    if value.get("method").and_then(Value::as_str) != Some("input")
        || value.get("title").and_then(Value::as_str) != Some(TOOL_UI_TITLE)
    {
        return;
    }
    let Some(ui_id) = value.get("id").and_then(Value::as_str).map(str::to_owned) else {
        return;
    };
    let Some(call_id) = value
        .get("placeholder")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|request| {
            request
                .get("tool_call_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    else {
        fail(
            inner,
            harness_error(HarnessErrorCode::InvalidEvent, "invalid Pi tool UI request"),
        );
        return;
    };
    let mut active_guard = inner.active.lock().expect("Pi active mutex poisoned");
    let Some(active) = active_guard.as_mut() else {
        return;
    };
    if !active.observed_tools.contains(&call_id) {
        drop(active_guard);
        fail(
            inner,
            harness_error(
                HarnessErrorCode::InvalidEvent,
                "Pi tool UI request preceded ToolStarted",
            ),
        );
        return;
    }
    active.ui_requests.insert(call_id.clone(), ui_id.clone());
    let result = active.results.get(&call_id).cloned();
    drop(active_guard);
    if let Some(result) = result {
        let _ = inject_tool_result(inner, &ui_id, &result);
    }
}

fn assistant_text(message: &Value) -> Option<String> {
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    Some(
        message
            .get("content")?
            .as_array()?
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
    )
}

fn last_stop_reason(value: &Value) -> Option<&str> {
    value
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))?
        .get("stopReason")?
        .as_str()
}

fn last_error(value: &Value) -> Option<&str> {
    value
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find_map(|message| message.get("errorMessage").and_then(Value::as_str))
}

fn fail(inner: &Inner, error: HarnessError) {
    let active = inner
        .active
        .lock()
        .expect("Pi active mutex poisoned")
        .take();
    if let Some(active) = active {
        let _ = active.sender.blocking_send(Err(error));
    }
    inner.set_health(SessionHealth::Broken);
}

fn read_jsonl(
    stdout: impl std::io::Read,
    sender: std_mpsc::Sender<Result<String, std::io::Error>>,
    artifact: &Path,
    record_limit: u64,
) {
    let mut reader = BufReader::new(stdout);
    let mut record = OutputRecord {
        path: artifact.to_owned(),
        limit: record_limit,
    };
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {
                let value = serde_json::from_str::<Value>(&line)
                    .unwrap_or_else(|_| json!({"type": "invalid_output", "line": line}));
                let streaming = (value.get("type").and_then(Value::as_str)
                    == Some("message_update")
                    && value
                        .pointer("/assistantMessageEvent/type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.ends_with("_delta")))
                    || value.get("type").and_then(Value::as_str) == Some("tool_execution_update");
                if !streaming {
                    let _ = record.append(value);
                }
                if line.ends_with('\n') {
                    line.pop();
                }
                if line.ends_with('\r') {
                    line.pop();
                }
                if sender.send(Ok(line)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        }
    }
}

// Keep one previous segment. Oversized events retain bounded string previews.
struct OutputRecord {
    path: PathBuf,
    limit: u64,
}

impl OutputRecord {
    fn append(&mut self, mut value: Value) -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        if bytes.len() as u64 + 1 > self.limit {
            if let Value::Object(fields) = &mut value {
                fields.insert("record_truncated".into(), Value::Bool(true));
            }
        }
        let mut preview = self.limit as usize / 4;
        while bytes.len() as u64 + 1 > self.limit && preview > 0 {
            shorten_strings(&mut value, preview);
            bytes = serde_json::to_vec(&value)?;
            preview /= 2;
        }
        if bytes.len() as u64 + 1 > self.limit {
            bytes = serde_json::to_vec(&json!({
                "type": "oversized_output", "record_truncated": true
            }))?;
        }
        bytes.push(b'\n');
        let size = fs::metadata(&self.path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if size + bytes.len() as u64 > self.limit {
            let previous = self.path.with_extension("jsonl.1");
            if previous.exists() {
                fs::remove_file(&previous)?;
            }
            if self.path.exists() {
                fs::rename(&self.path, previous)?;
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&bytes)?;
        file.flush()
    }
}

fn shorten_strings(value: &mut Value, limit: usize) {
    match value {
        Value::String(text) if text.len() > limit => {
            let mut end = limit;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("…");
        }
        Value::Array(values) => {
            for value in values {
                shorten_strings(value, limit);
            }
        }
        Value::Object(values) => {
            for (key, value) in values.iter_mut() {
                if !matches!(key.as_str(), "type" | "role" | "stopReason") {
                    shorten_strings(value, limit);
                }
            }
        }
        _ => {}
    }
}

fn write_line(stdin: &mut ChildStdin, value: &Value) -> Result<(), HarnessError> {
    serde_json::to_writer(&mut *stdin, value).map_err(|error| {
        harness_error(
            HarnessErrorCode::Io,
            format!("encode Pi RPC command: {error}"),
        )
    })?;
    stdin
        .write_all(b"\n")
        .and_then(|()| stdin.flush())
        .map_err(io_error("write Pi RPC command"))
}

fn reap(child: &mut Child, journal: Option<&Path>, force: bool) {
    let pid = child.id();
    lankide_harness::kill_process_group(child.id());
    if force || child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    record_pid(journal, "reaped", pid);
}

fn record_pid(path: Option<&Path>, action: &str, pid: u32) {
    let Some(path) = path else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let line = format!("{action}\t{pid}\n");
        let _ = file.write_all(line.as_bytes());
    }
}

fn io_error(context: &'static str) -> impl FnOnce(std::io::Error) -> HarnessError {
    move |error| harness_error(HarnessErrorCode::Io, format!("{context}: {error}"))
}

fn stream_dropped() -> HarnessError {
    harness_error(
        HarnessErrorCode::InvalidState,
        "Pi event stream was dropped",
    )
}

fn classify_failure(code: String, detail: String) -> lankide_harness::HarnessFailure {
    use lankide_harness::FailureCategory;
    let text = detail.to_ascii_lowercase();
    let category = if (text.contains("model")
        && (text.contains("not found")
            || text.contains("not available")
            || text.contains("not supported")))
        || [
            "model not found",
            "unknown model",
            "invalid model",
            "does not exist",
            "no such file",
            "not found (os error 2)",
            "command not found",
            "enoent",
        ]
        .iter()
        .any(|value| text.contains(value))
    {
        FailureCategory::Configuration
    } else if [
        "unauthorized",
        "authentication",
        "authenticate",
        "invalid bearer token",
        "token expired",
        "no api key",
        "missing api key",
        "invalid api key",
        "invalid_api_key",
        "api key not",
        "not logged in",
        "login required",
        "401",
        "forbidden",
        "403",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        FailureCategory::Authentication
    } else if [
        "rate limit",
        "rate_limit",
        "rate-limit",
        "usage limit",
        "usage_limit",
        "resource_exhausted",
        "exceeded your current quota",
        "too many requests",
        "429",
        "quota exceeded",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        FailureCategory::RateLimit
    } else if text.contains("cancelled") || text.contains("canceled") || text.contains("aborted") {
        FailureCategory::Cancelled
    } else if text.contains("timeout") || text.contains("timed out") {
        FailureCategory::Timeout
    } else if [
        "exited",
        "exit status",
        "process exit",
        "stream",
        "connection",
        "503",
        "502",
        "temporarily unavailable",
    ]
    .iter()
    .any(|value| text.contains(value))
    {
        FailureCategory::Transient
    } else {
        FailureCategory::Agent
    };
    lankide_harness::HarnessFailure {
        code,
        category,
        retryable: matches!(
            category,
            FailureCategory::RateLimit | FailureCategory::Transient | FailureCategory::Timeout
        ),
        detail,
    }
}

fn harness_error(code: HarnessErrorCode, message: impl Into<String>) -> HarnessError {
    let mut error = HarnessError::new(code, message);
    error.failure = classify_failure(error.failure.code.clone(), error.message.clone());
    // Unknown transport and configuration failures keep the contract default.
    if error.failure.category == lankide_harness::FailureCategory::Agent {
        let default = HarnessError::new(code, error.message.clone());
        error.failure.category = default.failure.category;
        error.failure.retryable = default.failure.retryable;
    }
    error
}

fn classified_event(mut kind: AgentEventKind) -> AgentEventKind {
    if let AgentEventKind::Failed {
        code,
        message,
        failure,
    } = &mut kind
    {
        if failure.is_none() {
            *failure = Some(classify_failure(code.clone(), message.clone()));
        }
    }
    kind
}

fn process_exit_detail(stderr_path: &Path, child: &mut Child, fallback: &str) -> String {
    let status = child
        .try_wait()
        .ok()
        .flatten()
        .map(|status| status.to_string());
    let stderr = std::fs::read_to_string(stderr_path).unwrap_or_default();
    if stderr.trim().is_empty() {
        status
            .map(|status| format!("{fallback}; process exit: {status}"))
            .unwrap_or_else(|| fallback.to_owned())
    } else {
        match status {
            Some(status) => format!("{stderr}\nprocess exit: {status}"),
            None => stderr,
        }
    }
}
