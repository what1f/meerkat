use lankide_core::{
    AttentionLevel, MessageId, ScheduleInterval, UnixMillis, WorkspaceId, is_canonical_tool_name,
    markdown_mention, markdown_mentions,
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

    let now = UnixMillis::new(1_700_000_123_456)?;
    let follow = AttentionLevel::Follow;
    let schedule = ScheduleInterval::try_from(30)?;
    let mention_only = AttentionLevel::Mention;
    assert_eq!(follow.as_str(), "follow");
    assert_eq!(schedule.minutes(), 30);
    assert_eq!(schedule.next_boundary(now)?.get() % 1_800_000, 0);
    assert_eq!(mention_only.as_str(), "mention");

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
        ScheduleInterval::try_from(10).unwrap_err().code(),
        "invalid_schedule_interval"
    );

    // Chat notices retain one short sentence in the triggering language.
    let english = lankide_core::agent_failure_notice("Please help");
    let chinese = lankide_core::agent_failure_notice("请帮忙");
    assert_eq!(
        english,
        "I couldn’t handle this message; please ask my owner to check Lankide."
    );
    assert_eq!(chinese, "我未能处理这条消息，请负责人在 Lankide 中检查。");
    write_result(
        &artifact,
        &run_id,
        started,
        timer.elapsed().as_millis(),
        "pass",
    )?;
    Ok(())
}
