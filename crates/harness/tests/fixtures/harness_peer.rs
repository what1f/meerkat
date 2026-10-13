use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

fn main() {
    let fault = std::env::var("HARNESS_PEER_FAULT").unwrap_or_else(|_| "none".to_owned());
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(Ok(first)) = lines.next() else {
        return;
    };
    let Ok(command) = serde_json::from_str::<Value>(&first) else {
        return;
    };
    match command["op"].as_str() {
        Some("probe") => {
            emit(json!({
                "type": "probe_result",
                "availability": "available",
                "version": "conformance-1.0.0",
                "capabilities": [
                    "system_prompt", "streaming", "resume", "steer", "cancel", "tools"
                ],
                "reason": null
            }));
        }
        Some("start") | Some("resume") => run_session(command, fault, lines),
        _ => {}
    }
}

fn run_session(
    command: Value,
    fault: String,
    mut lines: impl Iterator<Item = Result<String, io::Error>>,
) {
    if fault == "bind_exit" {
        return;
    }
    let native_session_id = command["request"]["native_session_id"]
        .as_str()
        .unwrap_or("native-conformance-session")
        .to_owned();
    emit(json!({
        "type": "bound",
        "native_session_id": native_session_id
    }));
    let mut sequence = 1_u64;
    let mut pending_tool_result = None;
    let mut tool_result_writes = 0_u64;
    while let Some(Ok(line)) = lines.next() {
        let Ok(command) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match command["op"].as_str() {
            Some("execute") if fault == "invalid_event" => {
                println!("not-json");
                io::stdout().flush().expect("flush invalid event");
            }
            Some("execute") if fault == "stream_cut" => {
                event(sequence, json!({"kind": "started"}));
                return;
            }
            Some("execute") => {
                sequence = 1;
                event(sequence, json!({"kind": "started"}));
                sequence += 1;
                event(
                    sequence,
                    json!({
                        "kind": "native_session_bound",
                        "native_session_id": native_session_id
                    }),
                );
                sequence += 1;
                let input = command["request"]["input"].as_str().unwrap_or("");
                event(
                    sequence,
                    json!({"kind": "assistant_delta", "text": format!("delta:{input}")}),
                );
                sequence += 1;
                if input != "long" {
                    event(
                        sequence,
                        json!({
                            "kind": "tool_started",
                            "tool_call_id": "call-1",
                            "tool_name": "echo",
                            "arguments": {"text": input}
                        }),
                    );
                    sequence += 1;
                    if input == "tool_roundtrip" {
                        continue;
                    }
                    event(
                        sequence,
                        json!({
                            "kind": "tool_finished",
                            "tool_call_id": "call-1",
                            "success": true,
                            "output": {"echoed": input}
                        }),
                    );
                    sequence += 1;
                    event(
                        sequence,
                        json!({"kind": "completed", "final_output": format!("final:{input}")}),
                    );
                }
            }
            Some("tool_result") => {
                tool_result_writes += 1;
                pending_tool_result = Some(command["request"].clone());
            }
            Some("steer") if command["input"].as_str() == Some("finish-tool") => {
                event(
                    sequence,
                    json!({
                        "kind": "tool_finished",
                        "tool_call_id": "call-1",
                        "success": false,
                        "output": {
                            "received": pending_tool_result,
                            "peer_write_count": tool_result_writes
                        }
                    }),
                );
                sequence += 1;
                event(
                    sequence,
                    json!({"kind": "completed", "final_output": "tool-result-applied"}),
                );
            }
            Some("steer") => {
                event(
                    sequence,
                    json!({
                        "kind": "assistant_delta",
                        "text": format!("steer:{}", command["input"].as_str().unwrap_or(""))
                    }),
                );
                sequence += 1;
            }
            Some("cancel") if fault == "ignore_cancel" => {}
            Some("cancel") => {
                event(
                    sequence,
                    json!({
                        "kind": "cancelled",
                        "reason": command["reason"].as_str().unwrap_or("cancelled")
                    }),
                );
            }
            Some("close") => return,
            _ => {}
        }
    }
}

fn event(sequence: u64, body: Value) {
    let mut object = body.as_object().cloned().expect("event object");
    object.insert("sequence".to_owned(), json!(sequence));
    emit(Value::Object(object));
}

fn emit(value: Value) {
    println!(
        "{}",
        serde_json::to_string(&value).expect("serialize peer event")
    );
    io::stdout().flush().expect("flush peer event");
}
