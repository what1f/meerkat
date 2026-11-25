//! A local OpenAI-compatible endpoint: `GET /v1/models` and streaming
//! `POST /v1/chat/completions`, scripted by the last user message.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Recorded {
    pub authorization: Option<String>,
    pub body: Value,
}

pub struct FakeModel {
    port: u16,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl FakeModel {
    pub fn start(key: &'static str) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let recorded = recorded.clone();
                std::thread::spawn(move || {
                    let _ = handle(stream, key, &recorded);
                });
            }
        });
        Ok(Self { port, requests })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Chat Completions requests received so far.
    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

fn handle(stream: TcpStream, key: &str, recorded: &Mutex<Vec<Recorded>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut length = 0usize;
    let mut authorization = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => authorization = Some(value.trim().to_owned()),
                _ => {}
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let mut stream = stream;
    let authorized = authorization.as_deref() == Some(&*format!("Bearer {key}"));
    if !authorized {
        return respond(
            &mut stream,
            "401 Unauthorized",
            "application/json",
            b"{\"error\":{}}",
        );
    }
    if request_line.starts_with("GET ") && request_line.contains("/models") {
        let list = json!({"object":"list","data":[
            {"id":"fake-model","object":"model","name":"Fake Model"},
            {"id":"fake-mini","object":"model"}
        ]});
        return respond(
            &mut stream,
            "200 OK",
            "application/json",
            list.to_string().as_bytes(),
        );
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    recorded.lock().unwrap().push(Recorded {
        authorization,
        body: body.clone(),
    });
    let model = body["model"].as_str().unwrap_or("fake-model").to_owned();
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let text = |message: &Value| message["content"].to_string();
    let last = messages.last().cloned().unwrap_or(Value::Null);
    let user = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .map(text)
        .unwrap_or_default();
    let any_user = |needle: &str| {
        messages
            .iter()
            .any(|message| message["role"] == "user" && text(message).contains(needle))
    };
    let trailing_tool_text = messages
        .iter()
        .rev()
        .take_while(|message| message["role"] == "tool")
        .map(text)
        .collect::<Vec<_>>()
        .join("\n");

    let mut deltas = Vec::new();
    let mut finish = "stop";
    if last["role"] == "tool" {
        if user.contains("DEFERRED_UPDATE") {
            if trailing_tool_text.contains("reported-ok") && trailing_tool_text.contains("audit-ok")
            {
                deltas.push(json!({"role":"assistant","content":"deferred-updated-ok"}));
            } else if trailing_tool_text.contains("tag") && trailing_tool_text.contains("audit") {
                let call_report = json!({
                    "name": "report",
                    "arguments": {"value": "v2", "tag": "updated"}
                })
                .to_string();
                let call_audit = json!({
                    "name": "audit",
                    "arguments": {"note": "checked"}
                })
                .to_string();
                deltas.push(json!({"role":"assistant","tool_calls":[
                    {"index":0,"id":"call_report_v2","type":"function","function":{"name":"call_tool","arguments":call_report}},
                    {"index":1,"id":"call_audit_v1","type":"function","function":{"name":"call_tool","arguments":call_audit}}
                ]}));
                finish = "tool_calls";
            } else {
                deltas
                    .push(json!({"role":"assistant","content":"missing updated deferred schema"}));
            }
        } else if trailing_tool_text.contains("reported-ok") {
            deltas.push(json!({"role":"assistant","content":"tool said: "}));
            deltas.push(json!({"content": text(&last)}));
        } else if user.contains("TOOL") && trailing_tool_text.contains("report") {
            let call_report = json!({
                "name": "report",
                "arguments": {"value": "roundtrip"}
            })
            .to_string();
            deltas.push(json!({"role":"assistant","tool_calls":[
                {"index":0,"id":"call_report_1","type":"function","function":{"name":"call_tool","arguments":call_report}}
            ]}));
            finish = "tool_calls";
        } else {
            deltas.push(json!({"role":"assistant","content":"tool said: "}));
            deltas.push(json!({"content": text(&last)}));
        }
    } else if user.contains("DEFERRED_UPDATE") {
        let search_args = json!({"query": "select:report,audit"}).to_string();
        deltas.push(json!({"role":"assistant","tool_calls":[
            {"index":0,"id":"call_search_2","type":"function","function":{"name":"tool_search","arguments":search_args}}
        ]}));
        finish = "tool_calls";
    } else if user.contains("TOOL") {
        let search_args = json!({"query": "select:report"}).to_string();
        deltas.push(json!({"role":"assistant","tool_calls":[
            {"index":0,"id":"call_search_1","type":"function","function":{"name":"tool_search","arguments":search_args}}
        ]}));
        finish = "tool_calls";
    } else if user.contains("RECALL") {
        let word = if any_user("kiwi") { "kiwi" } else { "unknown" };
        deltas.push(json!({"role":"assistant","content":word}));
    } else if user.contains("STEERED") {
        deltas.push(json!({"role":"assistant","content":"steered"}));
    } else if user.contains("WAIT") {
        std::thread::sleep(Duration::from_secs(3));
        deltas.push(json!({"role":"assistant","content":"waited"}));
    } else if user.contains("HANG") {
        std::thread::sleep(Duration::from_secs(60));
        deltas.push(json!({"role":"assistant","content":"late"}));
    } else {
        deltas.push(json!({"role":"assistant","reasoning_content":"thinking"}));
        deltas.push(json!({"content":"Hello "}));
        deltas.push(json!({"content":"world"}));
    }
    let chunk = |delta: Value, finish: Option<&str>| {
        json!({"id":"chunk-1","object":"chat.completion.chunk","created":1,"model":model,
            "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    };
    let mut events = deltas
        .into_iter()
        .map(|delta| chunk(delta, None))
        .collect::<Vec<_>>();
    events.push(chunk(json!({}), Some(finish)));
    events.push(
        json!({"id":"chunk-1","object":"chat.completion.chunk","created":1,"model":model,
        "choices":[],"usage":{"prompt_tokens":11,"completion_tokens":3,"total_tokens":14}}),
    );
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n"
    )?;
    for event in events {
        write!(stream, "data: {event}\n\n")?;
        stream.flush()?;
    }
    write!(stream, "data: [DONE]\n\n")?;
    stream.flush()
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}
