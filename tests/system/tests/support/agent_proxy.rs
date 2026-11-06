use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub fn cli(binary: &str, args: Vec<String>) -> Value {
    use std::io::Write as _;
    let home = std::env::var("MEERKAT_AGENT_HOME").expect("Agent home");
    let mut journal = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(&home).join("proxy-cli.log"))
        .expect("CLI scenario journal");
    writeln!(journal, "starting {args:?}").unwrap();
    let output = Command::new(binary)
        .args(&args)
        .output()
        .expect("run real meerkat CLI");
    writeln!(journal, "finished {args:?}: {}", output.status).unwrap();
    assert!(
        output.status.success(),
        "CLI {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}
pub fn run(binary: &str, input: &str) {
    let home = std::env::var("MEERKAT_AGENT_HOME").expect("Agent home");
    let home = Path::new(&home);
    let env = std::env::vars()
        .filter(|(key, _)| key.starts_with("MEERKAT_"))
        .collect::<std::collections::BTreeMap<_, _>>();
    let chat_input = input.split("\n\n### ").nth(1).expect("chat snapshot");
    let channel = chat_input
        .split("channel_id=")
        .nth(1)
        .unwrap()
        .split('，')
        .next()
        .unwrap();
    let chat = chat_input
        .split("chat_id=")
        .nth(1)
        .unwrap()
        .split('）')
        .next()
        .unwrap();
    let message = chat_input
        .rsplit("[message_id=")
        .next()
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    let capabilities = cli(binary, vec!["capabilities".into()]);
    let profile = cli(binary, vec!["profile".into()]);
    let avatar = home.join("portrait.png");
    let avatar_bytes = include_bytes!("../../../../crates/runtime/tests/assets/profile.png");
    std::fs::write(&avatar, avatar_bytes).unwrap();
    let updated = cli(
        binary,
        vec![
            "profile".into(),
            "update".into(),
            "--name".into(),
            "Bruce Lee".into(),
            "--bio".into(),
            "Ready".into(),
            "--avatar".into(),
            avatar.display().to_string(),
        ],
    );
    assert_eq!(
        std::fs::read(updated["avatar_path"].as_str().expect("local avatar path")).unwrap(),
        avatar_bytes
    );
    let arguments = home.join("arguments.json");
    std::fs::write(&arguments, "{}").unwrap();
    let invoked = cli(
        binary,
        vec![
            "invoke".into(),
            "unread_current".into(),
            "--arguments-file".into(),
            arguments.display().to_string(),
        ],
    );
    let content = cli(
        binary,
        vec![
            "content".into(),
            "--kind".into(),
            "message".into(),
            "--id".into(),
            message.into(),
        ],
    );
    let attachment = home.join("attachment.txt");
    // The JSON representation would exceed one WS frame without chunking.
    let bytes = vec![251_u8; 1_200_000];
    std::fs::write(&attachment, &bytes).unwrap();
    let mut invalid_attachments = Vec::new();
    for key in [
        "path",
        "file",
        "local_path",
        "id",
        "resourceId",
        "resource_id",
    ] {
        let result = cli(binary,vec!["invoke".into(),"message_send".into(),"--arguments-json".into(),json!({"channel_id":channel,"chat_id":chat,"content":"Report","attachments":[{key:attachment.display().to_string()}]}).to_string()]);
        invalid_attachments.push(result);
    }
    let local_markdown = cli(
        binary,
        vec![
            "invoke".into(),
            "message_send".into(),
            "--arguments-json".into(),
            json!({"channel_id":channel,"chat_id":chat,"content":"[![预览](./portrait.png)](./attachment.txt)"})
                .to_string(),
        ],
    );
    let content_file = home.join("message.md");
    std::fs::write(&content_file, "Sent with an attachment\n[Report](./attachment.txt)\n`[example](./missing-inline.txt)`\n```md\n![example](./missing-fenced.png)\n```").unwrap();
    let sent = cli(
        binary,
        vec![
            "send".into(),
            "--channel".into(),
            channel.into(),
            "--chat".into(),
            chat.into(),
            "--content-file".into(),
            content_file.display().to_string(),
            "--reply-to".into(),
            message.into(),
            "--force".into(),
        ],
    );
    let report = json!({"env":env,"capabilities":capabilities,"profile":profile,"updated":updated,"invoked":invoked,"content":content,"sent":sent,"invalid_attachments":invalid_attachments,"local_markdown":local_markdown});
    let path = home.join("proxy-report.json");
    std::fs::write(
        path.with_extension("tmp"),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    std::fs::rename(path.with_extension("tmp"), &path).unwrap();
    let release = std::env::var("MEERKAT_SYSTEM_RELEASE").expect("release gate");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !Path::new(&release).exists() {
        assert!(
            Instant::now() < deadline,
            "proxy scenario release timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
