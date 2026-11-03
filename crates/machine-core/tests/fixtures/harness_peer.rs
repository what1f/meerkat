use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let Some(Ok(line)) = lines.next() else { return };
    let Ok(first) = serde_json::from_str::<Value>(&line) else {
        return;
    };
    record(first["op"].as_str().unwrap_or("unknown"));
    if first["op"] == "probe" {
        emit(json!({
            "type":"probe_result", "availability":"available", "version":"machine-e2e",
            "capabilities":["system_prompt","streaming","resume","steer","cancel","tools"], "reason":null
        }));
        return;
    }
    for name in [
        "MEERKAT_AGENT_SOCKET",
        "MEERKAT_EXECUTION_ID",
        "MEERKAT_EXECUTION_ATTEMPT",
        "MEERKAT_EXECUTION_TOKEN",
        "MEERKAT_AGENT_HOME",
    ] {
        assert!(
            !std::env::var(name).unwrap_or_default().is_empty(),
            "Machine must inject {name}"
        );
    }
    assert!(std::path::Path::new(&std::env::var("MEERKAT_AGENT_HOME").unwrap()).is_absolute());
    let cli = std::process::Command::new("meerkat")
        .arg("capabilities")
        .output()
        .expect("Machine must provide capability CLI on PATH");
    assert!(cli.status.success());
    assert_eq!(
        String::from_utf8(cli.stdout).unwrap(),
        std::env::var("MEERKAT_EXECUTION_ID").unwrap()
    );
    assert!(std::path::Path::new(&std::env::var("MEERKAT_CLI").unwrap()).is_file());
    assert!(
        first["request"]["system_prompt"]
            .as_str()
            .unwrap()
            .contains("meerkat invoke 'cli: inspect'"),
        "Provider CLI commands must be described to the native Agent"
    );
    assert!(
        first["request"]["system_prompt"]
            .as_str()
            .unwrap()
            .contains("meerkat profile update --name NAME --role ROLE --bio BIO --avatar PATH"),
        "Agent profile commands must be described to the native Agent"
    );
    let prompt = first["request"]["system_prompt"].as_str().unwrap();
    let path = std::env::var("PATH").unwrap();
    let user_bin = path.split(':').find(|directory| {
        std::path::Path::new(directory).join("lark-cli").is_file()
            && !directory.ends_with(".machine/bin")
    });
    if let Some(user_bin) = user_bin {
        assert!(prompt.contains("- `lark-cli`：Feishu command line"));
        assert!(
            !prompt.contains("feishu lark-cli'"),
            "wrapped commands are not invoked directly"
        );
        let output = std::process::Command::new("lark-cli")
            .args(["docs", "+fetch", "--as", "user"])
            .output()
            .expect("the shim is on PATH");
        assert!(
            output.status.success(),
            "the shim runs instead of the user's lark-cli"
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("exec: feishu lark-cli\n--\n{user_bin}/lark-cli\ndocs\n+fetch\n--as\nuser\n")
        );
        record("lark_cli_shim");
    } else {
        assert!(
            !prompt.contains("lark-cli"),
            "an absent lark-cli is not mentioned"
        );
    }
    let native = first["request"]["native_session_id"]
        .as_str()
        .unwrap_or("native-machine-e2e")
        .to_owned();
    emit(json!({"type":"bound", "native_session_id":native}));
    let fault = std::env::var("MACHINE_HARNESS_FAULT").unwrap_or_default();
    let mut tool_result_received = false;
    let mut completion_fault = None;
    while let Some(Ok(line)) = lines.next() {
        let Ok(command) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        record(command["op"].as_str().unwrap_or("unknown"));
        match command["op"].as_str() {
            Some("execute") if fault == "bad_sequence" => {
                event(1, json!({"kind":"started"}));
                event(3, json!({"kind":"assistant_delta","text":"bad"}));
            }
            Some("execute") if fault == "stream_cut" => {
                event(1, json!({"kind":"started"}));
                return;
            }
            Some("execute") => {
                let input = command["request"]["input"].as_str().unwrap_or("");
                let correction = input.contains("上一条回复不符合结束协议");
                if input.contains("completion_retry") {
                    completion_fault = Some(false);
                } else if input.contains("completion_fail") {
                    completion_fault = Some(true);
                }
                event(1, json!({"kind":"started"}));
                event(
                    2,
                    json!({"kind":"native_session_bound","native_session_id":native}),
                );
                event(
                    3,
                    json!({"kind":"assistant_delta","text":format!("delta:{input}")}),
                );
                if correction {
                    record("completion_correction");
                    let final_output = if completion_fault == Some(false) {
                        completion_fault = None;
                        "OK"
                    } else {
                        "not OK"
                    };
                    event(4, json!({"kind":"completed","final_output":final_output}));
                    continue;
                }
                if input.contains("hold") {
                    continue;
                }
                let (tool_name, arguments) = if input.contains("message_send_file") {
                    (
                        "message_send",
                        json!({"channel_id":"chat","chat_id":"chat-a","content_file":"tool-message.md"}),
                    )
                } else if input.contains("message_send_missing") {
                    (
                        "message_send",
                        json!({"channel_id":"chat","chat_id":"chat-a","content_file":"missing.md"}),
                    )
                } else if input.contains("message_send_large") {
                    (
                        "message_send",
                        json!({"channel_id":"chat","chat_id":"chat-a","content_file":"large.md"}),
                    )
                } else if input.contains("message_send_utf8") {
                    (
                        "message_send",
                        json!({"channel_id":"chat","chat_id":"chat-a","content_file":"invalid.md"}),
                    )
                } else {
                    ("echo", json!({"text":input}))
                };
                event(
                    4,
                    json!({"kind":"tool_started","tool_call_id":"call-1","tool_name":tool_name,"arguments":arguments}),
                );
                if input.contains("tool_wait") {
                    continue;
                }
                event(
                    5,
                    json!({"kind":"tool_finished","tool_call_id":"call-1","success":true,"output":{"echoed":input}}),
                );
                let final_output = if completion_fault.is_some() {
                    "not OK"
                } else {
                    "OK"
                };
                event(6, json!({"kind":"completed","final_output":final_output}));
            }
            Some("steer") => {
                if command["input"].as_str() == Some("finish-tool") {
                    if tool_result_received {
                        event(
                            5,
                            json!({
                                "kind":"tool_finished",
                                "tool_call_id":"call-1",
                                "success":true,
                                "output":{"from_runtime":true}
                            }),
                        );
                    }
                    event(6, json!({"kind":"completed","final_output":"OK"}));
                } else {
                    event(
                        4,
                        json!({"kind":"assistant_delta","text":format!("steer:{}", command["input"].as_str().unwrap_or(""))}),
                    );
                }
            }
            Some("tool_result") => {
                tool_result_received = true;
                if let Some(code) = command["request"]["error"]["code"].as_str() {
                    record(&format!("tool_result_error:{code}"));
                }
            }
            Some("cancel") if fault == "ignore_cancel" => {}
            Some("cancel") => {
                event(
                    5,
                    json!({"kind":"cancelled","reason":command["reason"].as_str().unwrap_or("cancelled")}),
                );
            }
            Some("close") => return,
            _ => {}
        }
    }
}

fn record(operation: &str) {
    let Ok(path) = std::env::var("MACHINE_HARNESS_LOG") else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{operation}\t{}", std::process::id());
    }
}

fn event(sequence: u64, body: Value) {
    let mut object = body.as_object().cloned().expect("object");
    object.insert("sequence".into(), json!(sequence));
    emit(Value::Object(object));
}

fn emit(value: Value) {
    println!("{}", serde_json::to_string(&value).expect("serialize"));
    io::stdout().flush().expect("flush");
}
