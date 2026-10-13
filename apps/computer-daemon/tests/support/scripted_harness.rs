use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};

pub fn run() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(Ok(line)) = lines.next() else { return };
    let Ok(first) = serde_json::from_str::<Value>(&line) else {
        return;
    };
    record(first["op"].as_str().unwrap_or("unknown"));
    if first["op"] == "probe" {
        emit(json!({
            "type":"probe_result", "availability":"available", "version":"daemon-e2e",
            "capabilities":["system_prompt","streaming","resume","steer","cancel","tools"], "reason":null
        }));
        return;
    }
    let native = first["request"]["native_session_id"]
        .as_str()
        .unwrap_or("native-daemon-e2e")
        .to_owned();
    emit(json!({"type":"bound", "native_session_id":native}));
    let fault = std::env::var("COMPUTER_HARNESS_FAULT").unwrap_or_default();
    let mut tool_result_received = false;
    while let Some(Ok(line)) = lines.next() {
        let Ok(command) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        record(command["op"].as_str().unwrap_or("unknown"));
        match command["op"].as_str() {
            Some("execute") => {
                let input = command["request"]["input"].as_str().unwrap_or("");
                event(1, json!({"kind":"started"}));
                event(
                    2,
                    json!({"kind":"native_session_bound","native_session_id":native}),
                );
                event(
                    3,
                    json!({"kind":"assistant_delta","text":format!("delta:{input}")}),
                );
                if input.contains("hold") || input.contains("tool_wait") {
                    if input.contains("tool_wait") {
                        event(
                            4,
                            json!({"kind":"tool_started","tool_call_id":"call-1","tool_name":"echo","arguments":{"text":input}}),
                        );
                    }
                    continue;
                }
                event(
                    4,
                    json!({"kind":"completed","final_output":format!("final:{input}")}),
                );
            }
            Some("steer") => {
                if command["input"].as_str() == Some("finish-tool") {
                    if tool_result_received {
                        event(
                            5,
                            json!({"kind":"tool_finished","tool_call_id":"call-1","success":true,"output":{"from_runtime":true}}),
                        );
                    }
                    event(
                        6,
                        json!({"kind":"completed","final_output":"tool-result-complete"}),
                    );
                } else {
                    event(
                        4,
                        json!({"kind":"assistant_delta","text":format!("steer:{}", command["input"].as_str().unwrap_or(""))}),
                    );
                }
            }
            Some("tool_result") => tool_result_received = true,
            Some("cancel") if fault == "ignore_cancel" => {}
            Some("cancel") => event(
                5,
                json!({"kind":"cancelled","reason":command["reason"].as_str().unwrap_or("cancelled")}),
            ),
            Some("close") => return,
            _ => {}
        }
    }
}

fn record(operation: &str) {
    let Ok(path) = std::env::var("COMPUTER_HARNESS_LOG") else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{operation}\t{}", std::process::id());
    }
}

fn event(sequence: u64, body: Value) {
    let mut object = body.as_object().cloned().expect("scripted event object");
    object.insert("sequence".into(), json!(sequence));
    emit(Value::Object(object));
}

fn emit(value: Value) {
    println!(
        "{}",
        serde_json::to_string(&value).expect("serialize scripted response")
    );
    io::stdout().flush().expect("flush scripted response");
}
