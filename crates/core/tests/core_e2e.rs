use lankide_core::{
    Agent, AgentChatCursor, AgentId, Attachment, AttentionLevel, AttentionSetting, Channel,
    ChannelAbility, ChannelId, Chat, DomainError, ExecutionId, ExecutionInput, Mention, Message,
    MessageAuthor, MessageAuthorType, MessageId, MessageStatus, MessageTimeline, ProviderId,
    ResourceId, SnapshotId, TriggerReason, UnixMillis, UnreadChatInput, UnreadSnapshot,
    UnreadSnapshotInput, Workspace, WorkspaceId, is_canonical_tool_name, markdown_mention,
    markdown_mentions,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_result(
    artifact: &PathBuf,
    run_id: &str,
    started_at: u128,
    duration_millis: u128,
    status: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(
        artifact,
        serde_json::to_vec_pretty(&json!({
            "run_id": run_id,
            "module": "core",
            "classification": "hermetic",
            "started_at_unix_nanos": started_at,
            "duration_millis": duration_millis,
            "prerequisite_check": "pass",
            "scenarios": [{"name": "core_public_domain_scenario", "status": status}],
            "created_resources": [],
            "cleanup": "pass",
            "status": status,
        }))?,
    )?;
    Ok(())
}

fn workspace(id: &str) -> Result<Workspace, DomainError> {
    Workspace::new(
        WorkspaceId::new(id)?,
        format!("Workspace {id}"),
        ProviderId::new("fixture_im")?,
    )
}

fn author() -> Result<MessageAuthor, DomainError> {
    MessageAuthor::new(MessageAuthorType::Human, "human_1", "Human One")
}

fn message(
    chat: &Chat,
    id: i64,
    content: &str,
    mentions: Vec<Agent>,
    reply_to: Option<&Message>,
    attachments: Vec<Attachment>,
) -> Result<Message, DomainError> {
    Message::new(
        chat,
        MessageId::new(id)?,
        author()?,
        content,
        mentions,
        reply_to,
        attachments,
        MessageStatus::Sent,
        UnixMillis::new(1_700_000_000_000 + id)?,
    )
}

#[test]
fn core_public_domain_scenario() -> Result<(), Box<dyn std::error::Error>> {
    let timer = Instant::now();
    let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let run_id = format!("e2e_core_{started}_{}", std::process::id());
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("core crate is nested under the workspace")
        .to_path_buf();
    let artifact_dir = workspace_root
        .join("target/e2e-artifacts/core")
        .join(&run_id);
    fs::create_dir_all(&artifact_dir)?;
    let artifact = artifact_dir.join("result.json");
    write_result(&artifact, &run_id, started, 0, "running")?;
    println!("run_id={run_id}");
    println!("artifact={}", artifact.display());

    assert!(is_canonical_tool_name("feishu_reaction_catalog"));
    for invalid in ["feishu.reaction.catalog", "FeishuReaction", "_hidden", ""] {
        assert!(!is_canonical_tool_name(invalid), "accepted {invalid}");
    }

    let mention = markdown_mention("小林", "person_1").expect("valid mention");
    assert_eq!(mention, "@小林[person_1]");
    let parsed = markdown_mentions("请 @小林[person_1] 和 @助手[agent_alpha] 看一下");
    assert_eq!(
        parsed
            .iter()
            .map(|mention| (mention.name, mention.id))
            .collect::<Vec<_>>(),
        vec![("小林", "person_1"), ("助手", "agent_alpha")]
    );
    assert!(markdown_mentions("旧语法 [@agent_alpha]").is_empty());

    let workspace_a = workspace("alpha_team")?;
    let workspace_b = workspace("beta_team")?;
    let agent_a = Agent::new(&workspace_a, AgentId::new("helper")?, "Helper A")?;
    let agent_b = Agent::new(&workspace_b, AgentId::new("helper")?, "Helper B")?;
    assert_eq!(agent_a.id(), agent_b.id());
    assert_ne!(agent_a.workspace_id(), agent_b.workspace_id());

    let channel = Channel::new(
        &workspace_a,
        ChannelId::new("project_updates")?,
        "Project updates",
        [
            AttentionLevel::Follow,
            AttentionLevel::Schedule,
            AttentionLevel::Mention,
        ],
        [ChannelAbility::Reply, ChannelAbility::Attachment],
    )?;
    let chat_one = Chat::new(&workspace_a, &channel, 11, "external-chat-a", "Planning")?;
    let chat_two = Chat::new(&workspace_a, &channel, 12, "external-chat-b", "Delivery")?;
    assert_eq!(chat_one.id().as_str(), "fixture_im:project_updates:11");

    let first = message(&chat_one, 1, "initial", vec![], None, vec![])?;
    let attachment = Attachment::new(
        ResourceId::new(41)?,
        "brief.md",
        "text/markdown",
        128,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )?;
    let mentioned = message(
        &chat_one,
        2,
        "please review @Helper[helper]",
        vec![agent_a.clone()],
        Some(&first),
        vec![attachment],
    )?;
    let third = message(&chat_one, 3, "third", vec![], None, vec![])?;
    let fourth = message(&chat_one, 4, "fourth", vec![], None, vec![])?;
    let fifth = message(&chat_one, 5, "fifth", vec![], None, vec![])?;
    assert_eq!(mentioned.reply().unwrap().message_id(), first.id());
    assert_eq!(mentioned.mentions()[0].agent_id(), agent_a.id());
    assert_eq!(mentioned.attachments()[0].file_name(), "brief.md");

    let mut timeline_one = MessageTimeline::new(&chat_one);
    for item in [&first, &mentioned, &third, &fourth, &fifth] {
        timeline_one.append(item.clone())?;
    }
    let second_chat_message = message(&chat_two, 6, "delivery update", vec![], None, vec![])?;
    let mut timeline_two = MessageTimeline::new(&chat_two);
    timeline_two.append(second_chat_message.clone())?;

    let now = UnixMillis::new(1_700_000_123_456)?;
    let follow = AttentionSetting::new(
        &agent_a,
        &channel,
        &chat_one,
        AttentionLevel::Follow,
        None,
        now,
    )?;
    let schedule = AttentionSetting::new(
        &agent_a,
        &channel,
        &chat_one,
        AttentionLevel::Schedule,
        Some(30),
        now,
    )?;
    let mention_only = AttentionSetting::new(
        &agent_a,
        &channel,
        &chat_two,
        AttentionLevel::Mention,
        None,
        now,
    )?;
    assert_eq!(follow.level().as_str(), "follow");
    assert_eq!(schedule.schedule_interval().unwrap().minutes(), 30);
    assert_eq!(schedule.next_scheduled_at().unwrap().get() % 1_800_000, 0);
    assert_eq!(mention_only.level().as_str(), "mention");

    let mut cursor_one = AgentChatCursor::new(&agent_a, &chat_one)?;
    cursor_one.advance_to(&first)?;
    let cursor_two = AgentChatCursor::new(&agent_a, &chat_two)?;
    let unread_one = UnreadChatInput::new(
        &agent_a,
        &channel,
        &chat_one,
        &cursor_one,
        &timeline_one,
        Some(mentioned.id()),
    )?;
    let unread_two = UnreadChatInput::new(
        &agent_a,
        &channel,
        &chat_two,
        &cursor_two,
        &timeline_two,
        None,
    )?;
    let mut snapshot_input = UnreadSnapshotInput::new(&agent_a, now);
    snapshot_input.add_chat(unread_two)?;
    snapshot_input.add_chat(unread_one)?;

    let snapshot = UnreadSnapshot::build(SnapshotId::new(70)?, snapshot_input.clone())?;
    let repeated = UnreadSnapshot::build(SnapshotId::new(70)?, snapshot_input)?;
    assert_eq!(snapshot, repeated);
    assert_eq!(snapshot.chats()[0].chat_id(), chat_one.id());
    assert_eq!(snapshot.chats()[0].unread_count(), 4);
    assert_eq!(snapshot.chats()[0].read_to_message_id(), fifth.id());
    assert_eq!(
        snapshot.chats()[0]
            .recent_messages()
            .iter()
            .map(Message::id)
            .collect::<Vec<_>>(),
        vec![mentioned.id(), fourth.id(), fifth.id()]
    );

    let execution = ExecutionInput::new(
        ExecutionId::new(90)?,
        &snapshot,
        vec![TriggerReason::Mention {
            chat_id: chat_one.id().clone(),
            message_id: mentioned.id(),
        }],
    )?;
    timeline_one.append(message(
        &chat_one,
        7,
        "arrived later",
        vec![],
        None,
        vec![],
    )?)?;
    assert_eq!(
        execution.snapshot().chats()[0].read_to_message_id(),
        fifth.id()
    );
    assert_eq!(execution.snapshot(), &snapshot);

    assert_eq!(
        WorkspaceId::new("Invalid-ID").unwrap_err().code(),
        "invalid_identifier"
    );
    assert_eq!(
        serde_json::from_str::<WorkspaceId>(r#""Invalid-ID""#)
            .unwrap_err()
            .to_string(),
        "invalid_identifier"
    );
    assert!(serde_json::from_str::<MessageId>("0").is_err());
    assert_eq!(
        Mention::for_agent(&chat_one, &agent_b).unwrap_err().code(),
        "cross_workspace_reference"
    );
    let restricted_channel = Channel::new(
        &workspace_a,
        ChannelId::new("alerts")?,
        "Alerts",
        [AttentionLevel::Mention],
        [],
    )?;
    let restricted_chat = Chat::new(&workspace_a, &restricted_channel, 13, "alerts-1", "Alerts")?;
    assert_eq!(
        AttentionSetting::new(
            &agent_a,
            &restricted_channel,
            &restricted_chat,
            AttentionLevel::Follow,
            None,
            now,
        )
        .unwrap_err()
        .code(),
        "unsupported_attention"
    );
    assert_eq!(
        AttentionSetting::new(
            &agent_a,
            &channel,
            &chat_one,
            AttentionLevel::Schedule,
            Some(10),
            now,
        )
        .unwrap_err()
        .code(),
        "invalid_schedule_interval"
    );
    let mut regressing_cursor = AgentChatCursor::new(&agent_a, &chat_one)?;
    regressing_cursor.advance_to(&mentioned)?;
    assert_eq!(
        regressing_cursor.advance_to(&first).unwrap_err().code(),
        "cursor_regression"
    );

    write_result(
        &artifact,
        &run_id,
        started,
        timer.elapsed().as_millis(),
        "pass",
    )?;
    Ok(())
}
