//! Machine-owned QuickJS workers. Native protocol decisions live in JavaScript.
use crate::*;
use rquickjs::{Context, Function, Runtime};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;

#[derive(Clone, Debug)]
pub struct JsHarnessFactory {
    info: HarnessInfo,
    source: Arc<str>,
    journal: Option<PathBuf>,
    defaults: HarnessDefaults,
}
impl JsHarnessFactory {
    pub fn new(id: &str, name: &str, source: impl Into<Arc<str>>) -> Result<Self, HarnessError> {
        let mut factory = Self {
            info: HarnessInfo::new(id, name)?,
            source: source.into(),
            journal: None,
            defaults: HarnessDefaults::default(),
        };
        // Validate declarations without probing or starting a native CLI.
        let worker = factory.worker(None)?;
        factory.defaults =
            serde_json::from_value(worker.shared.metadata.lock().unwrap()["defaults"].clone())
                .map_err(json_error)?;
        worker.stop();
        Ok(factory)
    }
    pub fn with_pid_journal(mut self, path: impl Into<PathBuf>) -> Self {
        self.journal = Some(path.into());
        self
    }
    fn worker(&self, request: Option<SessionRequest>) -> Result<Worker, HarnessError> {
        Worker::spawn(
            self.info.clone(),
            self.source.clone(),
            self.journal.clone(),
            request,
        )
    }
    async fn open(
        &self,
        request: SessionRequest,
        resume: bool,
        deadline: Option<Instant>,
    ) -> Result<Box<dyn HarnessSession>, HarnessError> {
        request.config.validate()?;
        if request.config.harness_id != self.info.harness_id {
            return Err(HarnessError::invalid_config("Harness identity mismatch"));
        }
        if resume
            && request
                .native_session_id
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(HarnessError::invalid_config(
                "resume requires native_session_id",
            ));
        }
        let worker = self.worker(Some(request.clone()))?;
        let operation = worker.call(if resume { "resume" } else { "start" }, json!(request));
        let result = if let Some(deadline) = deadline {
            match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), operation).await
            {
                Ok(result) => result,
                Err(_) => Err(HarnessError::new(
                    HarnessErrorCode::Timeout,
                    "Harness startup timed out",
                )),
            }
        } else {
            operation.await
        };
        match result {
            Ok(value) => {
                if let Some(id) = value
                    .get("native_session_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                {
                    worker.shared.entry.lock().unwrap().native_session_id = id.to_owned();
                }
            }
            Err(error) => {
                worker.stop();
                return Err(error);
            }
        }
        if worker
            .shared
            .entry
            .lock()
            .unwrap()
            .native_session_id
            .is_empty()
        {
            worker.stop();
            return Err(HarnessError::new(
                HarnessErrorCode::BindFailed,
                "Plugin returned no native session binding",
            ));
        }
        Ok(Box::new(PluginSession {
            worker,
            tool_lock: tokio::sync::Mutex::new(()),
        }))
    }
}
impl HarnessFactory for JsHarnessFactory {
    fn info(&self) -> &HarnessInfo {
        &self.info
    }
    fn defaults(&self) -> HarnessDefaults {
        self.defaults.clone()
    }
    fn test<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, HarnessTestPlan> {
        Box::pin(async move {
            let worker = self.worker(Some(request.clone()))?;
            let result = worker.call("test", json!(request)).await;
            worker.stop();
            match result? {
                Value::Null => Ok(HarnessTestPlan {
                    request,
                    input: "Hello! Please reply with a short greeting.".into(),
                }),
                value => serde_json::from_value(value).map_err(json_error),
            }
        })
    }
    fn probe<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        Box::pin(async move {
            config.validate_executable()?;
            let worker = self.worker(None)?;
            let result = worker.call("probe", json!(config)).await;
            worker.stop();
            serde_json::from_value(result?).map_err(json_error)
        })
    }
    fn models<'a>(&'a self, config: &'a HarnessConfig) -> HarnessFuture<'a, Vec<NativeModel>> {
        Box::pin(async move {
            let worker = self.worker(None)?;
            let result = worker.call("models", json!(config)).await;
            worker.stop();
            serde_json::from_value(result?).map_err(json_error)
        })
    }
    fn start<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(self.open(request, false, None))
    }
    fn start_before<'a>(
        &'a self,
        request: SessionRequest,
        deadline: Instant,
    ) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(self.open(request, false, Some(deadline)))
    }
    fn resume<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        Box::pin(self.open(request, true, None))
    }
}
struct Shared {
    metadata: Mutex<Value>,
    generation: std::sync::atomic::AtomicU64,
    stopped: AtomicBool,
    started: AtomicBool,
    last_binding: Mutex<Option<String>>,
    pid: AtomicU32,
    processes: Mutex<HashMap<u32, NativeProcess>>,
    entry: Mutex<SessionEntry>,
    active: Mutex<
        Option<(
            tokio::sync::mpsc::UnboundedSender<Result<AgentEvent, HarnessError>>,
            u64,
        )>,
    >,
    tool_results: Mutex<HashMap<String, Value>>,
}
struct NativeProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    journal: Option<PathBuf>,
    recorder: Arc<Mutex<Record>>,
}
impl NativeProcess {
    fn terminate(&mut self) {
        self.stdin.take();
        kill_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
        journal(self.journal.as_deref(), "reaped", self.child.id());
    }
}
impl Drop for NativeProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}
impl Shared {
    fn fail(&self, error: HarnessError) {
        self.entry.lock().unwrap().health = SessionHealth::Broken;
        if let Some((sender, _)) = self.active.lock().unwrap().take() {
            let _ = sender.send(Err(error));
        }
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.processes.lock().unwrap().clear();
        self.pid.store(0, Ordering::Release);
    }
    fn emit(&self, kind: AgentEventKind) {
        if matches!(kind, AgentEventKind::Started) && self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        if let AgentEventKind::NativeSessionBound { native_session_id } = &kind {
            self.entry.lock().unwrap().native_session_id = native_session_id.clone();
            let mut previous = self.last_binding.lock().unwrap();
            if previous.as_ref() == Some(native_session_id) {
                return;
            }
            *previous = Some(native_session_id.clone());
        }
        let kind_for_health = match &kind {
            AgentEventKind::Failed { .. } => 0,
            AgentEventKind::Cancelled { .. } => 1,
            _ => 2,
        };
        let terminal = matches!(
            kind,
            AgentEventKind::Completed { .. }
                | AgentEventKind::Failed { .. }
                | AgentEventKind::Cancelled { .. }
        );
        let mut active = self.active.lock().unwrap();
        if let Some((sender, seq)) = active.as_mut() {
            *seq += 1;
            let _ = sender.send(Ok(AgentEvent {
                sequence: *seq,
                kind,
            }));
        }
        if terminal {
            active.take();
            self.entry.lock().unwrap().health = match kind_for_health {
                0 => SessionHealth::Broken,
                1 => SessionHealth::Cancelled,
                _ => SessionHealth::Healthy,
            };
        }
    }
}
struct Invocation {
    hook: String,
    input: Value,
    reply: oneshot::Sender<Result<Value, HarnessError>>,
}
struct Worker {
    commands: mpsc::Sender<Invocation>,
    shared: Arc<Shared>,
}
impl Worker {
    fn spawn(
        info: HarnessInfo,
        source: Arc<str>,
        journal: Option<PathBuf>,
        request: Option<SessionRequest>,
    ) -> Result<Self, HarnessError> {
        let shared = Arc::new(Shared {
            metadata: Mutex::new(Value::Null),
            generation: std::sync::atomic::AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            started: AtomicBool::new(false),
            last_binding: Mutex::new(None),
            pid: AtomicU32::new(0),
            processes: Mutex::new(HashMap::new()),
            entry: Mutex::new(SessionEntry {
                harness_id: info.harness_id.clone(),
                native_session_id: String::new(),
                config_fingerprint: request
                    .as_ref()
                    .map(|r| r.config.config_fingerprint.clone())
                    .unwrap_or_default(),
                health: SessionHealth::Healthy,
            }),
            active: Mutex::new(None),
            tool_results: Mutex::new(HashMap::new()),
        });
        let (commands, rx) = mpsc::channel::<Invocation>();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let state = shared.clone();
        let thread_name = format!("harness-js-{}", info.harness_id);
        let runner = move || {
            let outcome = (|| -> Result<(), HarnessError> {
                let runtime = Runtime::new().map_err(js_error)?;
                runtime.set_memory_limit(64 * 1024 * 1024);
                runtime.set_max_stack_size(1024 * 1024);
                let interrupted = state.clone();
                runtime.set_interrupt_handler(Some(Box::new(move || {
                    interrupted.stopped.load(Ordering::Acquire)
                })));
                let context = Context::full(&runtime).map_err(js_error)?;
                let (tx, events) = mpsc::channel();
                let scratch = tempfile::Builder::new()
                    .prefix("meerkat-plugin-")
                    .tempdir()
                    .map_err(|e| HarnessError::new(HarnessErrorCode::Io, e.to_string()))?;
                let root = Some(
                    request
                        .as_ref()
                        .map(|r| r.working_directory.clone())
                        .unwrap_or_else(|| scratch.path().to_path_buf()),
                );
                let host = Arc::new(Host {
                    shared: state.clone(),
                    root,
                    scratch,
                    environment: request
                        .as_ref()
                        .map(|r| r.config.environment.clone())
                        .unwrap_or_default(),
                    events: Mutex::new(events),
                    tx,
                    journal,
                    mcp: Mutex::new(None),
                    mcp_results: Arc::new(Mutex::new(HashMap::new())),
                });
                context.with(|ctx|->Result<(),HarnessError>{
                    let bridge=host.clone();ctx.globals().set("__host",Function::new(ctx.clone(),move|op:String,input:String|->String{let result=serde_json::from_str(&input).map_err(json_error).and_then(|v|bridge.operation(&op,v));match result {Ok(value)=>json!({"ok":value}).to_string(),Err(e)=>json!({"error":e}).to_string()}}).map_err(js_error)?).map_err(js_error)?;
                    ctx.eval::<(),_>(include_str!("sdk.js")).map_err(js_error)?;
                    ctx.eval::<(),_>(source.as_ref()).map_err(|error|context_error(&ctx,error))?;
                    let metadata: String=ctx.eval("JSON.stringify({id:harness.id,apiVersion:harness.apiVersion,defaults:harness.defaults||{},reuseSession:harness.reuseSession===true})").map_err(js_error)?;
                    let metadata:Value=serde_json::from_str(&metadata).map_err(json_error)?;
                    *state.metadata.lock().unwrap()=metadata.clone();
                    if metadata["id"]!=info.harness_id||metadata["apiVersion"]!=1{return Err(HarnessError::invalid_config("Plugin API version or identity mismatch"));}
                    Ok(())
                })?;
                let _ = ready_tx.send(Ok(()));
                while !state.stopped.load(Ordering::Acquire) {
                    match rx.recv_timeout(Duration::from_millis(2)) {
                        Ok(command) => {
                            let result = invoke(&context, &command.hook, command.input);
                            let _ = command.reply.send(result);
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    for _ in 0..128 {
                        let event = host.events.lock().unwrap().try_recv().ok();
                        let Some(event) = event else { break };
                        let hook = if event.get("exit").is_some() {
                            "onExit"
                        } else if event.get("mcp").is_some() {
                            "onToolCall"
                        } else {
                            "onMessage"
                        };
                        if let Err(error) = invoke(&context, hook, event) {
                            state.fail(error);
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = outcome {
                let _ = ready_tx.send(Err(error.clone()));
                if let Some((sender, _)) = state.active.lock().unwrap().take() {
                    let _ = sender.send(Err(error));
                }
            }
            state.stop();
        };
        thread::Builder::new()
            .name(thread_name)
            .spawn(runner)
            .map_err(|e| HarnessError::new(HarnessErrorCode::SpawnFailed, e.to_string()))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(result) => result?,
            Err(_) => {
                shared.stop();
                return Err(HarnessError::new(
                    HarnessErrorCode::Timeout,
                    "Plugin initialization timed out",
                ));
            }
        }
        Ok(Self { commands, shared })
    }
    async fn call(&self, hook: &str, input: Value) -> Result<Value, HarnessError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Invocation {
                hook: hook.into(),
                input,
                reply: tx,
            })
            .map_err(|_| {
                HarnessError::new(HarnessErrorCode::InvalidState, "Plugin worker closed")
            })?;
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(reply) => reply.map_err(|_| {
                HarnessError::new(HarnessErrorCode::StreamInterrupted, "Plugin worker ended")
            })?,
            Err(_) => {
                self.stop();
                Err(HarnessError::new(
                    HarnessErrorCode::Timeout,
                    "Plugin hook timed out",
                ))
            }
        }
    }
    fn stop(&self) {
        self.shared.stop();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
fn invoke(context: &Context, hook: &str, input: Value) -> Result<Value, HarnessError> {
    context.with(|ctx| {
        let function: Function = ctx.globals().get("__invoke").map_err(js_error)?;
        let result: String = function
            .call((hook, input.to_string()))
            .map_err(|e| context_error(&ctx, e))?;
        let result: Value = serde_json::from_str(&result).map_err(json_error)?;
        if let Some(error) = result.get("error") {
            return Err(serde_json::from_value(error.clone()).unwrap_or_else(|_| {
                HarnessError::new(HarnessErrorCode::InvalidEvent, error.to_string())
            }));
        }
        Ok(result.get("ok").cloned().unwrap_or(Value::Null))
    })
}
fn context_error(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> HarnessError {
    let detail = if error.is_exception() {
        let e = ctx.catch();
        format!("{e:?}")
    } else {
        error.to_string()
    };
    HarnessError::new(HarnessErrorCode::InvalidEvent, detail)
}
fn js_error(error: rquickjs::Error) -> HarnessError {
    HarnessError::new(HarnessErrorCode::InvalidEvent, error.to_string())
}
fn json_error(error: serde_json::Error) -> HarnessError {
    HarnessError::new(HarnessErrorCode::InvalidEvent, error.to_string())
}
struct PluginSession {
    worker: Worker,
    tool_lock: tokio::sync::Mutex<()>,
}
impl HarnessSession for PluginSession {
    fn reusable(&self) -> bool {
        self.worker.shared.metadata.lock().unwrap()["reuseSession"] == true
    }
    fn rebind<'a>(&'a self, request: SessionRequest) -> HarnessFuture<'a, ()> {
        Box::pin(async move {
            if !self.reusable()
                || self.entry().health != SessionHealth::Healthy
                || self.worker.shared.active.lock().unwrap().is_some()
            {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "Session is not reusable",
                ));
            }
            self.worker.call("rebind", json!(request)).await?;
            Ok(())
        })
    }
    fn entry(&self) -> SessionEntry {
        self.worker.shared.entry.lock().unwrap().clone()
    }
    fn process_group_id(&self) -> Option<u32> {
        let id = self.worker.shared.pid.load(Ordering::Acquire);
        (id != 0).then_some(id)
    }
    fn execute<'a>(&'a self, request: ExecuteRequest) -> HarnessFuture<'a, AgentEventStream> {
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            {
                let mut active = self.worker.shared.active.lock().unwrap();
                if active.is_some() {
                    return Err(HarnessError::new(
                        HarnessErrorCode::InvalidState,
                        "Execution already active",
                    ));
                }
                *active = Some((tx, 0));
            }
            self.worker.shared.started.store(false, Ordering::Release);
            self.worker.shared.last_binding.lock().unwrap().take();
            self.worker.shared.emit(AgentEventKind::Started);
            self.worker.shared.emit(AgentEventKind::NativeSessionBound {
                native_session_id: self.entry().native_session_id,
            });
            self.worker.shared.generation.fetch_add(1, Ordering::AcqRel);
            self.worker.shared.tool_results.lock().unwrap().clear();
            if let Err(e) = self.worker.call("execute", json!(request)).await {
                self.worker.shared.active.lock().unwrap().take();
                return Err(e);
            }
            Ok(Box::pin(UnboundedReceiverStream::new(rx)) as AgentEventStream)
        })
    }
    fn steer<'a>(&'a self, delivery_id: String, input: String) -> HarnessFuture<'a, SteerReceipt> {
        Box::pin(async move {
            self.worker
                .call("steer", json!({"delivery_id":delivery_id,"input":input}))
                .await?;
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
            let _guard = self.tool_lock.lock().await;
            if self.worker.shared.active.lock().unwrap().is_none() {
                return Err(HarnessError::new(
                    HarnessErrorCode::InvalidState,
                    "Tool result requires active execution",
                ));
            }
            let value = json!(request);
            let previous = self
                .worker
                .shared
                .tool_results
                .lock()
                .unwrap()
                .get(&request.tool_call_id)
                .cloned();
            if let Some(previous) = previous {
                if previous != value {
                    return Err(HarnessError::new(
                        HarnessErrorCode::InvalidState,
                        "Conflicting tool result",
                    ));
                }
                return Ok(ToolResultReceipt {
                    tool_call_id: request.tool_call_id,
                    already_applied: true,
                });
            }
            self.worker.call("toolResult", value.clone()).await?;
            self.worker
                .shared
                .tool_results
                .lock()
                .unwrap()
                .insert(request.tool_call_id.clone(), value);
            Ok(ToolResultReceipt {
                tool_call_id: request.tool_call_id,
                already_applied: false,
            })
        })
    }
    fn cancel<'a>(&'a self, reason: String) -> HarnessFuture<'a, CancelReceipt> {
        Box::pin(async move {
            let value = self.worker.call("cancel", json!({"reason":reason})).await?;
            Ok(CancelReceipt {
                accepted: value["accepted"].as_bool().unwrap_or(true),
            })
        })
    }
    fn close<'a>(&'a self) -> HarnessFuture<'a, CloseReceipt> {
        Box::pin(async move {
            let closed = self.entry().health == SessionHealth::Closed;
            if !closed {
                let result = self.worker.call("close", json!({})).await;
                self.worker.stop();
                self.worker.shared.entry.lock().unwrap().health = SessionHealth::Closed;
                result?;
            }
            Ok(CloseReceipt {
                already_closed: closed,
            })
        })
    }
}
struct Host {
    shared: Arc<Shared>,
    root: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    scratch: tempfile::TempDir,
    events: Mutex<mpsc::Receiver<Value>>,
    tx: mpsc::Sender<Value>,
    journal: Option<PathBuf>,
    mcp: Mutex<Option<crate::plugin_mcp::McpStopHandle>>,
    mcp_results: Arc<Mutex<HashMap<String, mpsc::Sender<ToolResultRequest>>>>,
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Some(handle) = self.mcp.lock().unwrap().take() {
            handle.stop();
        }
        self.mcp_results.lock().unwrap().clear();
    }
}
impl Host {
    fn path(&self, value: &Value) -> Result<PathBuf, HarnessError> {
        let path = PathBuf::from(value.as_str().unwrap_or(""));
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| HarnessError::invalid_config("No session filesystem"))?;
        if !path.starts_with(root)
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(HarnessError::invalid_config(
                "Path is outside the managed Agent directory",
            ));
        }
        Ok(path)
    }
    fn writable_path(&self, value: &Value) -> Result<PathBuf, HarnessError> {
        let path = self.path(value)?;
        let root = fs::canonicalize(self.root.as_ref().unwrap())
            .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        let mut parent = path.as_path();
        while !parent.exists() {
            parent = parent
                .parent()
                .ok_or_else(|| HarnessError::invalid_config("Invalid managed path"))?;
        }
        let resolved =
            fs::canonicalize(parent).map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        if !resolved.starts_with(root) {
            return Err(HarnessError::invalid_config(
                "Managed writes cannot follow a link outside the Agent directory",
            ));
        }
        Ok(path)
    }
    fn operation(&self, op: &str, v: Value) -> Result<Value, HarnessError> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err(HarnessError::new(
                HarnessErrorCode::InvalidState,
                "Plugin worker closed",
            ));
        }
        let io = |e: std::io::Error| HarnessError::new(HarnessErrorCode::Io, e.to_string());
        match op {
            "cancelAfter" => {
                let state = self.shared.clone();
                let generation = state.generation.load(Ordering::Acquire);
                let pid_journal = self.journal.clone();
                let millis = v["millis"].as_u64().unwrap_or(3000);
                let reason = v["reason"].as_str().unwrap_or("Cancelled").to_string();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(millis));
                    if state.generation.load(Ordering::Acquire) == generation
                        && state.active.lock().unwrap().is_some()
                    {
                        let pid = state.pid.load(Ordering::Acquire);
                        journal(pid_journal.as_deref(), "forced", pid);
                        state.processes.lock().unwrap().clear();
                        state.emit(AgentEventKind::Cancelled { reason });
                    }
                });
                Ok(Value::Null)
            }
            "directory" => Ok(json!(self.root)),
            "uuid" => Ok(json!(uuid::Uuid::new_v4().to_string())),
            "env" => Ok(json!(
                self.environment
                    .get(v.as_str().unwrap_or(""))
                    .cloned()
                    .or_else(|| std::env::var(v.as_str().unwrap_or("")).ok())
            )),
            "fail" => {
                self.shared
                    .fail(serde_json::from_value(v).map_err(json_error)?);
                Ok(Value::Null)
            }
            "emit" => {
                let event: AgentEventKind = serde_json::from_value(v).map_err(json_error)?;
                self.shared.emit(event);
                Ok(Value::Null)
            }
            "bind" => {
                let id = v
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| HarnessError::invalid_config("Empty native binding"))?;
                self.shared.emit(AgentEventKind::NativeSessionBound {
                    native_session_id: id.into(),
                });
                Ok(Value::Null)
            }
            "write" => {
                let path = self.writable_path(&v["path"])?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(io)?;
                }
                private_write(&path, v["content"].as_str().unwrap_or("").as_bytes())?;
                Ok(Value::Null)
            }
            "canonical" => Ok(json!(fs::canonicalize(self.path(&v)?).map_err(io)?)),
            "read" => Ok(json!(fs::read_to_string(self.path(&v)?).map_err(io)?)),
            "readFirstLine" => {
                const MAX_BYTES: u64 = 64 * 1024;
                let file = fs::File::open(self.path(&v)?).map_err(io)?;
                let mut line = Vec::new();
                BufReader::new(file)
                    .take(MAX_BYTES + 1)
                    .read_until(b'\n', &mut line)
                    .map_err(io)?;
                if line.len() > MAX_BYTES as usize {
                    return Err(HarnessError::invalid_config(
                        "File first line exceeds 64 KiB",
                    ));
                }
                let line = String::from_utf8(line)
                    .map_err(|_| HarnessError::invalid_config("File first line must be UTF-8"))?;
                Ok(json!(line))
            }
            "exists" => Ok(json!(self.path(&v)?.exists())),
            "list" => {
                let path = self.path(&v)?;
                let entries = fs::read_dir(path)
                    .map_err(io)?
                    .filter_map(Result::ok)
                    .map(|entry| entry.path().display().to_string())
                    .collect::<Vec<_>>();
                Ok(json!(entries))
            }
            "prepare" => {
                let destination = self.writable_path(&v["directory"])?;
                let variable = v["sourceVariable"].as_str().unwrap_or("");
                let overrides: BTreeMap<String, String> =
                    serde_json::from_value(v["environment"].clone()).unwrap_or_default();
                let source = overrides
                    .get(variable)
                    .cloned()
                    .or_else(|| self.environment.get(variable).cloned())
                    .or_else(|| std::env::var(variable).ok())
                    .map(PathBuf::from)
                    .or_else(|| {
                        overrides
                            .get("HOME")
                            .cloned()
                            .or_else(|| self.environment.get("HOME").cloned())
                            .or_else(|| std::env::var("HOME").ok())
                            .map(|home| {
                                PathBuf::from(home).join(v["sourceRelative"].as_str().unwrap_or(""))
                            })
                    });
                let files = v["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                crate::config::refresh_config_overlay(&destination, source.as_deref(), &files)?;
                if let Some(source) = source.as_ref() {
                    let keys = v["pathKeys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>();
                    for name in files
                        .iter()
                        .filter(|name| !matches!(**name, "auth.json" | ".credentials.json"))
                    {
                        crate::config::rebase_native_config(
                            &destination.join(name),
                            source,
                            &keys,
                        )?;
                    }
                }

                for name in v["resources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if let Some(source) = source.as_ref() {
                        if Path::new(name).components().count() != 1
                            || !matches!(
                                Path::new(name).components().next(),
                                Some(std::path::Component::Normal(_))
                            )
                        {
                            return Err(HarnessError::invalid_config(
                                "Invalid native resource name",
                            ));
                        }
                        crate::config::link_native_resource(
                            &source.join(name),
                            &destination.join(name),
                        )?;
                    }
                }
                Ok(json!(destination))
            }
            "skills" => {
                crate::prepare_native_skills(
                    self.root
                        .as_ref()
                        .ok_or_else(|| HarnessError::invalid_config("No Agent directory"))?,
                    &self.path(&v)?,
                )?;
                Ok(Value::Null)
            }
            "spawn" => self.spawn(v),
            "send" => {
                let mut processes = self.shared.processes.lock().unwrap();
                let process = processes
                    .get_mut(&(v["process"].as_u64().unwrap_or(0) as u32))
                    .ok_or_else(|| {
                        HarnessError::new(
                            HarnessErrorCode::InvalidState,
                            "Native process unavailable",
                        )
                    })?;
                let stdin = process.stdin.as_mut().ok_or_else(|| {
                    HarnessError::new(HarnessErrorCode::InvalidState, "Native stdin closed")
                })?;
                writeln!(stdin, "{}", v["message"]).map_err(io)?;
                stdin.flush().map_err(io)?;
                Ok(Value::Null)
            }
            "record" => {
                let processes = self.shared.processes.lock().unwrap();
                let process = processes
                    .get(&(v["process"].as_u64().unwrap_or(0) as u32))
                    .ok_or_else(|| {
                        HarnessError::new(
                            HarnessErrorCode::InvalidState,
                            "Native process unavailable",
                        )
                    })?;
                *process.recorder.lock().unwrap() = Record::new(v["options"].clone());
                Ok(Value::Null)
            }
            "receive" => {
                let millis = v["timeout"].as_u64().unwrap_or(100).min(100);
                let event = self
                    .events
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_millis(millis))
                    .ok();
                Ok(event.unwrap_or(Value::Null))
            }
            "terminate" => {
                let pid = v.as_u64().unwrap_or(0) as u32;
                self.shared.processes.lock().unwrap().remove(&pid);
                Ok(Value::Null)
            }
            "closeStdin" => {
                if let Some(process) = self
                    .shared
                    .processes
                    .lock()
                    .unwrap()
                    .get_mut(&(v.as_u64().unwrap_or(0) as u32))
                {
                    process.stdin.take();
                }
                Ok(Value::Null)
            }
            "mcp" => {
                let directory = self.writable_path(&v["directory"])?;
                fs::create_dir_all(&directory).map_err(io)?;
                let tools = serde_json::from_value(v["tools"].clone()).map_err(json_error)?;
                let server = crate::plugin_mcp::McpServer::start(&directory, tools)?;
                let (path, calls, stop) = server.into_parts();
                *self.mcp.lock().unwrap() = Some(stop);
                let tx = self.tx.clone();
                let pending = self.mcp_results.clone();
                thread::spawn(move || {
                    while let Ok(call) = calls.recv() {
                        let id = call.call_id.clone();
                        pending.lock().unwrap().insert(id.clone(), call.response);
                        if tx.send(json!({"mcp":true,"tool_call_id":id,"tool_name":call.name,"arguments":call.arguments})).is_err(){break;}
                    }
                });
                Ok(json!(path))
            }
            "mcpResult" => {
                let request: ToolResultRequest = serde_json::from_value(v).map_err(json_error)?;
                let sender = self
                    .mcp_results
                    .lock()
                    .unwrap()
                    .remove(&request.tool_call_id)
                    .ok_or_else(|| {
                        HarnessError::new(HarnessErrorCode::InvalidState, "MCP call not observed")
                    })?;
                sender.send(request).map_err(|_| {
                    HarnessError::new(HarnessErrorCode::StreamInterrupted, "MCP connection closed")
                })?;
                Ok(Value::Null)
            }
            _ => Err(HarnessError::invalid_config(format!(
                "Unknown host operation: {op}"
            ))),
        }
    }
    fn spawn(&self, v: Value) -> Result<Value, HarnessError> {
        let executable = v["executable"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid_config("Missing executable"))?;
        let mut command = Command::new(executable);
        let args = v["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        command.args(args);
        if let Some(cwd) = v["cwd"].as_str() {
            command.current_dir(cwd);
        }
        if let Some(env) = v["env"].as_object() {
            for (key, value) in env {
                if let Some(value) = value.as_str() {
                    command.env(key, value);
                } else {
                    command.env_remove(key);
                }
            }
        }
        command.env_remove("MEERKAT_RUNTIME_RUN");
        let stderr = v["stderr"].as_str().map(PathBuf::from).unwrap_or_else(|| {
            self.scratch
                .path()
                .join(format!("{}.stderr", uuid::Uuid::new_v4()))
        });
        if let Some(parent) = stderr.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| HarnessError::new(HarnessErrorCode::Io, e.to_string()))?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr)
            .map_err(|e| HarnessError::new(HarnessErrorCode::Io, e.to_string()))?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(file);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|e| {
            let mut error = HarnessError::new(
                HarnessErrorCode::SpawnFailed,
                format!("Native CLI spawn failed: {e}"),
            );
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) {
                error.failure.category = FailureCategory::Configuration;
                error.failure.retryable = false;
            }
            error
        })?;
        let pid = child.id();
        journal(self.journal.as_deref(), "spawned", pid);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let recorder = Arc::new(Mutex::new(Record::new(
            v.get("record").cloned().unwrap_or(Value::Null),
        )));
        self.shared.processes.lock().unwrap().insert(
            pid,
            NativeProcess {
                child,
                stdin,
                journal: self.journal.clone(),
                recorder: recorder.clone(),
            },
        );
        self.shared.pid.store(pid, Ordering::Release);
        let tx = self.tx.clone();
        let process_state = self.shared.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader
                    .by_ref()
                    .take(8 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => break,
                    Ok(_) if line.len() > 8 * 1024 * 1024 => {
                        let _=tx.send(json!({"process":pid,"exit":true,"detail":"Native JSON line exceeds limit"}));
                        return;
                    }
                    Ok(_) => {
                        let raw = String::from_utf8_lossy(&line);
                        let message = serde_json::from_str::<Value>(&raw)
                            .unwrap_or_else(|_| json!({"invalid_output":raw}));
                        recorder.lock().unwrap().append(&message);
                        if tx.send(json!({"process":pid,"message":message})).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let success = process_state
                .processes
                .lock()
                .unwrap()
                .get_mut(&pid)
                .and_then(|process| process.child.try_wait().ok().flatten())
                .map(|status| status.success());
            let detail = read_stderr_tail(&stderr);
            let _=tx.send(json!({"process":pid,"exit":true,"success":success,"detail":if detail.is_empty(){"Native process exited".into()}else{detail}}));
        });
        Ok(json!(pid))
    }
}
fn read_stderr_tail(path: &Path) -> String {
    use std::io::{Seek, SeekFrom};
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(8192)));
    let mut bytes = Vec::new();
    let _ = file.take(8192).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<(), HarnessError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| HarnessError::new(HarnessErrorCode::Io, e.to_string()))?;
    file.write_all(bytes)
        .map_err(|e| HarnessError::new(HarnessErrorCode::Io, e.to_string()))
}
fn journal(path: Option<&Path>, action: &str, pid: u32) {
    if let Some(path) = path {
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(format!("{action}\t{pid}\n").as_bytes());
        }
    }
}
struct Record {
    path: Option<PathBuf>,
    limit: usize,
}
impl Record {
    fn new(v: Value) -> Self {
        Self {
            path: v["path"].as_str().map(PathBuf::from),
            limit: v["limit"].as_u64().unwrap_or(4 * 1024 * 1024) as usize,
        }
    }
    fn append(&mut self, v: &Value) {
        let Some(path) = &self.path else { return };
        if v["type"] == "tool_execution_update"
            || (v["type"] == "message_update"
                && v.pointer("/assistantMessageEvent/type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.ends_with("_delta")))
        {
            return;
        }
        let mut bytes = serde_json::to_vec(v).unwrap_or_default();
        if bytes.len() + 1 > self.limit {
            bytes = b"{\"type\":\"oversized_output\",\"record_truncated\":true}".to_vec();
        }
        bytes.push(b'\n');
        if fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0) + bytes.len() > self.limit {
            let old = path.with_extension("jsonl.1");
            let _ = fs::remove_file(&old);
            let _ = fs::rename(path, old);
        }
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(&bytes);
        }
    }
}

/// Compose an embedded adapter without a second native protocol implementation.
#[macro_export]
macro_rules! harness_plugin {
    ($name:ident, $id:literal, $display:literal, $source:expr) => {
        #[derive(Clone, Debug)]
        pub struct $name($crate::JsHarnessFactory);
        impl $name {
            pub fn new() -> Self {
                Self(
                    $crate::JsHarnessFactory::new($id, $display, $source)
                        .expect("valid embedded Harness plugin"),
                )
            }
            pub fn with_pid_journal(mut self, path: impl Into<std::path::PathBuf>) -> Self {
                self.0 = self.0.with_pid_journal(path);
                self
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl $crate::HarnessFactory for $name {
            fn info(&self) -> &$crate::HarnessInfo {
                self.0.info()
            }
            fn defaults(&self) -> $crate::HarnessDefaults {
                self.0.defaults()
            }
            fn test<'a>(
                &'a self,
                request: $crate::SessionRequest,
            ) -> $crate::HarnessFuture<'a, $crate::HarnessTestPlan> {
                self.0.test(request)
            }
            fn probe<'a>(
                &'a self,
                config: &'a $crate::HarnessConfig,
            ) -> $crate::HarnessFuture<'a, $crate::ProbeResult> {
                self.0.probe(config)
            }
            fn models<'a>(
                &'a self,
                config: &'a $crate::HarnessConfig,
            ) -> $crate::HarnessFuture<'a, Vec<$crate::NativeModel>> {
                self.0.models(config)
            }
            fn start<'a>(
                &'a self,
                request: $crate::SessionRequest,
            ) -> $crate::HarnessFuture<'a, Box<dyn $crate::HarnessSession>> {
                self.0.start(request)
            }
            fn start_before<'a>(
                &'a self,
                request: $crate::SessionRequest,
                deadline: std::time::Instant,
            ) -> $crate::HarnessFuture<'a, Box<dyn $crate::HarnessSession>> {
                self.0.start_before(request, deadline)
            }
            fn resume<'a>(
                &'a self,
                request: $crate::SessionRequest,
            ) -> $crate::HarnessFuture<'a, Box<dyn $crate::HarnessSession>> {
                self.0.resume(request)
            }
        }
    };
}
