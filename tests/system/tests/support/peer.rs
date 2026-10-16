mod agent_proxy;
use lankide_system_e2e::Script;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    time::Duration,
};

fn emit(value: Value) {
    println!("{value}");
    io::stdout().flush().expect("flush event");
}
fn event(sequence: &mut u64, mut value: Value) {
    *sequence += 1;
    value["sequence"] = json!(*sequence);
    emit(value);
}

fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let first: Value = serde_json::from_str(
        &lines
            .next()
            .expect("initial command")
            .expect("read command"),
    )
    .expect("initial JSON");
    if first["op"] == "probe" {
        emit(
            json!({"type":"probe_result","availability":"available","version":"system-rig","capabilities":serde_json::from_str::<Value>(&std::env::var("LANKIDE_SYSTEM_CAPABILITIES").expect("capabilities")).expect("capability JSON"),"reason":null}),
        );
        return;
    }
    let controls_path = std::env::var("LANKIDE_SYSTEM_CONTROLS").expect("control journal");
    let mut controls = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&controls_path)
        .expect("control journal");
    writeln!(controls, "{}", json!({"op": first["op"], "native_session_id": first["request"]["native_session_id"], "pid": std::process::id()})).expect("record session start");
    let script: Script =
        serde_json::from_str(&std::env::var("LANKIDE_SYSTEM_SCRIPT").expect("Agent script"))
            .expect("script JSON");
    emit(
        json!({"type":"bound","native_session_id":first["request"]["native_session_id"].as_str().unwrap_or("system-native")}),
    );
    drop(lines);
    let runtime = tokio::runtime::Runtime::new().expect("peer runtime");
    let (commands, received) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let command: Value =
                serde_json::from_str(&line.expect("command")).expect("command JSON");
            let mut controls = std::fs::OpenOptions::new()
                .append(true)
                .open(&controls_path)
                .expect("control journal");
            writeln!(controls, "{}", json!({"op": command["op"]})).expect("record native control");
            if command["op"] == "steer" {
                let path = std::env::var("LANKIDE_SYSTEM_STEERS").expect("steer journal");
                let mut journal = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("journal");
                writeln!(journal, "{command}").expect("record steer");
            }
            if commands.send(command).is_err() {
                break;
            }
        }
    });
    let mut sequence = 0;
    for command in &received {
        match command["op"].as_str() {
            Some("execute") => {
                let input = command["request"]["input"]
                    .as_str()
                    .expect("snapshot input");
                let mut journal = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(std::env::var("LANKIDE_SYSTEM_INPUTS").expect("input journal"))
                    .expect("open journal");
                writeln!(
                    journal,
                    "{}",
                    serde_json::to_string(input).expect("input JSON")
                )
                .expect("record input");
                sequence = 0;
                event(&mut sequence, json!({"kind":"started"}));
                let (text, delay) = match &script {
                    Script::Reply {
                        text,
                        delay_ms,
                        deltas,
                        held,
                    } => {
                        for delta in deltas {
                            event(
                                &mut sequence,
                                json!({"kind":"assistant_delta","text":delta}),
                            );
                        }
                        if *held {
                            let release =
                                std::env::var("LANKIDE_SYSTEM_RELEASE").expect("release gate");
                            while !std::path::Path::new(&release).exists() {
                                std::thread::sleep(Duration::from_millis(5));
                            }
                        }
                        (text, *delay_ms)
                    }
                    Script::OfflineEvents { count, bytes, text } => {
                        let release =
                            std::env::var("LANKIDE_SYSTEM_RELEASE").expect("release gate");
                        while !std::path::Path::new(&release).exists() {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        for _ in 0..*count {
                            event(
                                &mut sequence,
                                json!({"kind":"assistant_delta", "text":"x".repeat(*bytes)}),
                            );
                        }
                        (text, 0)
                    }
                    Script::LinkReply { text } | Script::ReplyBeforeTerminal { text } => (text, 0),
                    Script::BufferedReply { text } => {
                        let release =
                            std::env::var("LANKIDE_SYSTEM_RELEASE").expect("release gate");
                        while !std::path::Path::new(&release).exists() {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        (text, 0)
                    }
                    Script::FailSequence { texts, reply } => {
                        let counter = format!(
                            "{}.count",
                            std::env::var("LANKIDE_SYSTEM_INPUTS").expect("counter")
                        );
                        let n = std::fs::read_to_string(&counter)
                            .ok()
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        std::fs::write(counter, (n + 1).to_string()).expect("counter write");
                        if let Some(text) = texts.get(n) {
                            event(
                                &mut sequence,
                                json!({"kind":"failed","code":"scripted_failure","message":text,"failure":lankide_system_e2e::scripted_failure(text)}),
                            );
                            continue;
                        }
                        (reply, 0)
                    }
                    Script::FailMidTurn { text } | Script::FailSpawn { text } => {
                        event(
                            &mut sequence,
                            json!({"kind":"failed","code":"scripted_failure","message":text,"failure":lankide_system_e2e::scripted_failure(text)}),
                        );
                        continue;
                    }
                    Script::Hang => continue,
                    Script::HangOnce { text } => {
                        if !std::path::Path::new("system-hung-once").exists() {
                            std::fs::write("system-hung-once", b"hung").expect("hang marker");
                            continue;
                        }
                        (text, 0)
                    }
                    Script::Progress {
                        duration_ms,
                        interval_ms,
                    } => {
                        let deadline =
                            std::time::Instant::now() + Duration::from_millis(*duration_ms);
                        while std::time::Instant::now() < deadline {
                            event(
                                &mut sequence,
                                json!({"kind":"assistant_delta","text":"progress"}),
                            );
                            std::thread::sleep(Duration::from_millis((*interval_ms).max(1)));
                        }
                        event(
                            &mut sequence,
                            json!({"kind":"completed","final_output":"OK"}),
                        );
                        continue;
                    }
                    Script::SpawnChild { text, delay_ms } => {
                        let child = std::process::Command::new("sleep")
                            .arg("300")
                            .spawn()
                            .expect("spawn child");
                        std::fs::write("system-child.pid", child.id().to_string())
                            .expect("child journal");
                        let _ = (text, delay_ms);
                        continue;
                    }
                    Script::ProxyScenario { binary } => {
                        agent_proxy::run(binary, input);
                        event(
                            &mut sequence,
                            json!({"kind":"completed","final_output":"OK"}),
                        );
                        continue;
                    }
                    Script::FailedProviderTool {
                        via_cli,
                        rpc_error,
                        text,
                    } => {
                        if *via_cli {
                            let result = runtime
                                .block_on(lankide_cli::execute_from_environment(vec![
                                    "invoke".into(),
                                    "failing_reaction".into(),
                                    "--arguments-json".into(),
                                    "{}".into(),
                                ]))
                                .expect("failed Tool is a capability result");
                            assert_eq!(result["success"], false, "{result}");
                            assert_eq!(result["error"]["code"], "provider_tool_failed", "{result}");
                        } else {
                            event(
                                &mut sequence,
                                json!({"kind":"tool_started", "tool_call_id":"failed-im",
                                "tool_name":if *rpc_error {"message_recall"} else {"failing_reaction"}, "arguments":{}}),
                            );
                            let result = received.recv_timeout(Duration::from_secs(2)).expect(
                                "failed provider Tool result must arrive before idle timeout",
                            );
                            assert_eq!(result["op"], "tool_result");
                            assert_eq!(result["request"]["success"], false, "{result}");
                            assert_eq!(
                                result["request"]["error"]["code"],
                                if *rpc_error {
                                    "invalid_request"
                                } else {
                                    "provider_tool_failed"
                                },
                                "{result}"
                            );
                            if *rpc_error {
                                assert_eq!(
                                    result["request"]["error"]["retryable"], false,
                                    "{result}"
                                );
                            }
                            event(
                                &mut sequence,
                                json!({"kind":"tool_finished", "tool_call_id":"failed-im", "success":false,"output":null}),
                            );
                        }
                        (text, 0)
                    }
                    Script::Capability { arguments, text } => {
                        runtime
                            .block_on(lankide_cli::execute_from_environment(arguments.clone()))
                            .expect("Agent capability");
                        (text, 0)
                    }
                };
                std::thread::sleep(Duration::from_millis(delay));
                // Use the real Agent capability entry point. A terminal Harness
                // output is diagnostic text, not an implicit message to the IM.
                let input = command["request"]["input"]
                    .as_str()
                    .expect("snapshot input");
                for chat_input in input.split("\n\n### ").skip(1) {
                    let channel = chat_input
                        .split("channel_id=")
                        .nth(1)
                        .expect("snapshot channel")
                        .split('，')
                        .next()
                        .expect("channel");
                    let chat = chat_input
                        .split("chat_id=")
                        .nth(1)
                        .expect("snapshot chat")
                        .split('）')
                        .next()
                        .expect("chat");
                    let message = chat_input
                        .rsplit("[message_id=")
                        .next()
                        .expect("snapshot message")
                        .split(']')
                        .next()
                        .expect("message");
                    if matches!(
                        script,
                        Script::LinkReply { .. } | Script::BufferedReply { .. }
                    ) {
                        let call_id = format!("system-reply-{message}");
                        event(
                            &mut sequence,
                            json!({"kind":"tool_started", "tool_call_id":call_id,
                            "tool_name":"message_send", "arguments":{
                                "channel_id":channel, "chat_id":chat, "content":text,
                                "reply_to":message.parse::<i64>().expect("message ID"), "force":true
                            }}),
                        );
                        if matches!(script, Script::BufferedReply { .. }) {
                            continue;
                        }
                        let result = received
                            .recv_timeout(Duration::from_secs(10))
                            .expect("Tool result over Machine link");
                        assert_eq!(result["op"], "tool_result");
                        assert_eq!(result["request"]["tool_call_id"], call_id);
                        assert_eq!(result["request"]["success"], true, "Tool failed: {result}");
                        event(
                            &mut sequence,
                            json!({"kind":"tool_finished", "tool_call_id":call_id,
                            "success":true, "output":result["request"]["structured_content"]}),
                        );
                        continue;
                    }
                    let result = runtime
                        .block_on(retry_capability(
                            vec![
                                "send".into(),
                                "--channel".into(),
                                channel.into(),
                                "--chat".into(),
                                chat.into(),
                                "--content".into(),
                                text.clone(),
                                "--reply-to".into(),
                                message.into(),
                                "--force".into(),
                                "--idempotency-key".into(),
                                format!("system-reply-{message}"),
                            ],
                            if matches!(script, Script::OfflineEvents { .. }) {
                                Duration::from_secs(60)
                            } else {
                                Duration::from_secs(10)
                            },
                        ))
                        .expect("send through Agent capability");
                    assert_eq!(result["accepted"], true, "reply rejected: {result}");
                }
                if matches!(script, Script::ReplyBeforeTerminal { .. }) {
                    let mut journal = std::fs::OpenOptions::new()
                        .append(true)
                        .open(std::env::var("LANKIDE_SYSTEM_CONTROLS").expect("control journal"))
                        .expect("open control journal");
                    journal
                        .write_all(b"{\"op\":\"reply_sent\"}\n")
                        .expect("record capability reply");
                    let release = std::env::var("LANKIDE_SYSTEM_RELEASE").expect("terminal gate");
                    let deadline = std::time::Instant::now() + Duration::from_secs(20);
                    while !std::path::Path::new(&release).exists() {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "terminal gate timeout"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                event(
                    &mut sequence,
                    json!({"kind":"completed","final_output":"OK"}),
                );
            }
            Some("cancel")
                if matches!(
                    script,
                    Script::Hang | Script::HangOnce { .. } | Script::SpawnChild { .. }
                ) => {}
            Some("cancel") => event(
                &mut sequence,
                json!({"kind":"cancelled","reason":command["reason"]}),
            ),
            Some("close") => return,
            _ => {}
        }
    }
}

// A native Agent may retry typed transport failures. Keep retries at the outer
// Harness boundary and reuse the send idempotency key across an unknown reply.
async fn retry_capability(
    arguments: Vec<String>,
    budget: Duration,
) -> Result<Value, lankide_cli::CliError> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let result = lankide_cli::execute_from_environment(arguments.clone()).await;
        let retryable = match &result {
            Err(lankide_cli::CliError::Runtime(status)) => matches!(
                status.code(),
                lankide_protocol::Code::Unavailable | lankide_protocol::Code::DeadlineExceeded
            ),
            Err(lankide_cli::CliError::Transport(_)) => true,
            _ => false,
        };
        if !retryable || tokio::time::Instant::now() >= deadline {
            return result;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
