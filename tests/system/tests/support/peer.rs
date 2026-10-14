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
            json!({"type":"probe_result","availability":"available","version":"system-rig","capabilities":["system_prompt","streaming","resume","cancel","tools"],"reason":null}),
        );
        return;
    }
    let script: Script =
        serde_json::from_str(&std::env::var("LANKIDE_SYSTEM_SCRIPT").expect("Agent script"))
            .expect("script JSON");
    emit(
        json!({"type":"bound","native_session_id":first["request"]["native_session_id"].as_str().unwrap_or("system-native")}),
    );
    let runtime = tokio::runtime::Runtime::new().expect("peer runtime");
    let mut sequence = 0;
    for line in lines {
        let command: Value = serde_json::from_str(&line.expect("command")).expect("command JSON");
        match command["op"].as_str() {
            Some("execute") => {
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
                    let result = runtime
                        .block_on(lankide_cli::execute_from_environment([
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
                        ]))
                        .expect("send through Agent capability");
                    assert_eq!(result["accepted"], true, "reply rejected: {result}");
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
