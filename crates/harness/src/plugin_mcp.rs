use crate::{HarnessError, HarnessErrorCode, ToolDefinition, ToolResultRequest};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, serde::Deserialize)]
pub(crate) struct McpTool {
    #[serde(flatten)]
    definition: ToolDefinition,
    #[serde(default, rename = "_meta")]
    metadata: Value,
}

pub(crate) struct McpCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    pub response: mpsc::Sender<ToolResultRequest>,
}

pub(crate) struct McpServer {
    config_path: Option<PathBuf>,
    calls: Option<mpsc::Receiver<McpCall>>,
    shutdown: Arc<AtomicBool>,
    port: u16,
}

pub(crate) struct McpStopHandle {
    shutdown: Arc<AtomicBool>,
    port: u16,
}

impl McpStopHandle {
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

impl McpServer {
    pub fn start(artifact_dir: &Path, tools: Vec<McpTool>) -> Result<Self, HarnessError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(io_error("bind MCP server"))?;
        let port = listener
            .local_addr()
            .map_err(io_error("read MCP address"))?
            .port();
        let config_path = artifact_dir.join(format!("mcp-{}.json", unique()));
        let config = json!({"mcpServers":{"lankide":{
            "type":"http", "url":format!("http://127.0.0.1:{port}/mcp")
        }}});
        fs::write(
            &config_path,
            serde_json::to_vec(&config).expect("serialize MCP config"),
        )
        .map_err(io_error("write MCP config"))?;
        let (sender, calls) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&shutdown);
        thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::Acquire) {
                    return;
                }
                if let Ok(stream) = stream {
                    let tools = tools.clone();
                    let sender = sender.clone();
                    thread::spawn(move || handle_connection(stream, &tools, &sender));
                }
            }
        });
        Ok(Self {
            config_path: Some(config_path),
            calls: Some(calls),
            shutdown,
            port,
        })
    }

    pub fn into_parts(mut self) -> (PathBuf, mpsc::Receiver<McpCall>, McpStopHandle) {
        let handle = McpStopHandle {
            shutdown: Arc::clone(&self.shutdown),
            port: self.port,
        };
        (
            self.config_path.take().expect("MCP config path"),
            self.calls.take().expect("MCP call receiver"),
            handle,
        )
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        if self.calls.is_some() {
            self.shutdown.store(true, Ordering::Release);
            let _ = TcpStream::connect(("127.0.0.1", self.port));
        }
    }
}

fn handle_connection(mut stream: TcpStream, tools: &[McpTool], calls: &mpsc::Sender<McpCall>) {
    let Some(body) = read_http_body(&mut stream) else {
        let _ = write_http(&mut stream, 400, None);
        return;
    };
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        let _ = write_http(&mut stream, 400, None);
        return;
    };
    let Some(id) = request.get("id").cloned() else {
        let _ = write_http(&mut stream, 202, None);
        return;
    };
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": request.pointer("/params/protocolVersion").cloned()
                .unwrap_or_else(|| json!("2025-06-18")),
            "capabilities":{"tools":{"listChanged":false}},
            "serverInfo":{"name":"lankide","version":env!("CARGO_PKG_VERSION")}
        }),
        "tools/list" => json!({"tools":tools.iter().map(|tool| json!({
            "name":tool.definition.name,
            "description":tool.definition.description,
            "inputSchema":tool.definition.input_schema,
            "outputSchema":tool.definition.output_schema,
            "_meta":tool.metadata
        })).collect::<Vec<_>>() }),
        "tools/call" => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !tools.iter().any(|tool| tool.definition.name == name) {
                return write_rpc_error(&mut stream, id, -32602, "unknown tool");
            }
            let (result_sender, result_receiver) = mpsc::channel();
            let call = McpCall {
                call_id: format!("claude-mcp-{}", unique()),
                name: name.to_owned(),
                arguments: request
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                response: result_sender,
            };
            if calls.send(call).is_err() {
                return write_rpc_error(&mut stream, id, -32000, "tool bridge closed");
            }
            match result_receiver.recv() {
                Ok(result) => json!({
                    "content":[{"type":"text","text":result.content_markdown}],
                    "structuredContent":result.structured_content,
                    "isError":!result.success
                }),
                Err(_) => return write_rpc_error(&mut stream, id, -32000, "tool call cancelled"),
            }
        }
        _ => return write_rpc_error(&mut stream, id, -32601, "method not found"),
    };
    let response = json!({"jsonrpc":"2.0","id":id,"result":result});
    let _ = write_http(&mut stream, 200, Some(&response));
}

fn write_rpc_error(stream: &mut TcpStream, id: Value, code: i64, message: &str) {
    let response = json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}});
    let _ = write_http(stream, 200, Some(&response));
}

fn read_http_body(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > 64 * 1024 {
            return None;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())?;
    while bytes.len() < header_end + length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Some(bytes[header_end..header_end + length].to_vec())
}

fn write_http(stream: &mut TcpStream, status: u16, value: Option<&Value>) -> std::io::Result<()> {
    let body = value.map(Value::to_string).unwrap_or_default();
    let text = match status {
        200 => "OK",
        202 => "Accepted",
        _ => "Bad Request",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    stream.flush()
}

fn unique() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ NEXT.fetch_add(1, Ordering::Relaxed)
        ^ ((std::process::id() as u64) << 32)
}

fn io_error(context: &'static str) -> impl FnOnce(std::io::Error) -> HarnessError {
    move |error| HarnessError::new(HarnessErrorCode::Io, format!("{context}: {error}"))
}
