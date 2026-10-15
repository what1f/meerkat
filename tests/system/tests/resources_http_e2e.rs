#[path = "support/cutter.rs"]
mod cutter;
use cutter::{Cutter, Download};
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use sha2::{Digest, Sha256};
use std::time::Duration;

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        computer_ws_only: true,
        ..Default::default()
    }
}
fn image(size: usize) -> Vec<u8> {
    // A one-pixel PNG, padded for a deterministic large transfer.
    let mut bytes = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    bytes.resize(size.max(bytes.len()), 0);
    bytes
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn s11_1_image_reaches_the_remote_agent_snapshot_with_the_right_hash() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::LinkReply {
            text: "seen".into(),
        })
        .create()
        .await?;
    let bytes = image(0);
    let resource = team.image_message(&agent, "images", bytes.clone()).await?;
    rig.settle().await?;
    assert!(rig.homes_are_separate());
    assert!(!rig.computer_has_runtime_socket());
    let path = agent
        .resource_directory()?
        .join(format!("{}-photo.png", resource.resource_id));
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(hash(&std::fs::read(&path)?), resource.sha256);
    let inputs = agent.harness_inputs()?;
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].contains(path.to_str().unwrap()), "{}", inputs[0]);
    assert!(!inputs[0].contains("resource://"));
    assert!(!path.with_extension("part").exists());
    assert_eq!(team.outbound(&agent, "images").len(), 1);
    rig.close().await
}

#[tokio::test]
async fn captioned_image_keeps_caption_and_local_path_together() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::LinkReply {
            text: "seen".into(),
        })
        .create()
        .await?;
    let bytes = image(0);
    let resource = team
        .captioned_image_message(&agent, "images", "Check this photo", bytes.clone())
        .await?;
    rig.settle().await?;
    let path = agent
        .resource_directory()?
        .join(format!("{}-photo.png", resource.resource_id));
    assert_eq!(std::fs::read(&path)?, bytes);
    let inputs = agent.harness_inputs()?;
    let line = inputs[0]
        .lines()
        .find(|line| line.contains("Check this photo"))
        .ok_or("caption missing")?;
    assert!(
        line.contains(path.to_str().unwrap()),
        "caption has no attachment: {line}"
    );
    rig.close().await
}

#[tokio::test]
async fn s11_2_cut_download_resumes_from_the_partial_file() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let (url, _) = rig.computer_endpoint()?;
    let cut_after = 1024 * 1024;
    let cutter = Cutter::start(&url, Some(cut_after), false).await?;
    rig.route_computer_through(&cutter.url).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::LinkReply {
            text: "seen".into(),
        })
        .create()
        .await?;
    let bytes = image(2 * cut_after);
    let resource = team.image_message(&agent, "images", bytes.clone()).await?;
    rig.settle().await?;
    let downloads = cutter.downloads();
    assert_eq!(downloads.len(), 2, "{downloads:?}");
    assert_eq!(downloads[0].range, None);
    assert_eq!(
        downloads[1].range.as_deref(),
        Some(format!("bytes={cut_after}-").as_str())
    );
    let path = agent
        .resource_directory()?
        .join(format!("{}-photo.png", resource.resource_id));
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(hash(&std::fs::read(&path)?), resource.sha256);
    assert!(!path.with_extension("part").exists());
    assert_eq!(agent.harness_inputs()?.len(), 1);
    assert_eq!(team.outbound(&agent, "images").len(), 1);
    rig.close().await
}

fn request(
    client: &reqwest::Client,
    url: &str,
    resource: i64,
    auth: &Download,
) -> reqwest::RequestBuilder {
    client
        .get(format!(
            "{}/resources/{resource}",
            url.replacen("ws://", "http://", 1)
        ))
        .bearer_auth(&auth.token)
        .header("x-lankide-execution", &auth.execution)
        .header("x-lankide-attempt", &auth.attempt)
}

#[tokio::test]
async fn s11_3_foreign_and_expired_attempt_tokens_are_rejected() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        execution_token_lifetime: Duration::from_secs(5),
        ..config()
    })
    .await?;
    let (url, secret) = rig.computer_endpoint()?;
    let cutter = Cutter::start(&url, None, false).await?;
    rig.route_computer_through(&cutter.url).await?;
    let team = rig.workspace("team").await?;
    let agent = team.agent("bruce").harness(Script::Hang).create().await?;
    let resource = team.image_message(&agent, "images", image(0)).await?;
    agent.wait_until_running().await?;
    let other = team.agent("clark").harness(Script::Hang).create().await?;
    team.image_message(&other, "other-images", image(0)).await?;
    other.wait_until_running().await?;
    let records = cutter.downloads();
    assert_eq!(records.len(), 2);
    let auth = &records[0];
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    assert_eq!(
        request(&client, &url, resource.resource_id, auth)
            .send()
            .await?
            .status(),
        200
    );
    let mut foreign = auth.clone();
    foreign.token = records[1].token.clone();
    assert_eq!(
        request(&client, &url, resource.resource_id, &foreign)
            .send()
            .await?
            .status(),
        403
    );
    foreign.token = secret;
    assert_eq!(
        request(&client, &url, resource.resource_id, &foreign)
            .send()
            .await?
            .status(),
        403
    );
    let other_team = rig.workspace("otherteam").await?;
    let stranger = other_team
        .agent("diana")
        .harness(Script::Hang)
        .create()
        .await?;
    // Resource identities are workspace-local. Choose an ID absent from team.
    other_team.save_image(image(0)).await?;
    other_team.save_image(image(0)).await?;
    let private_resource = other_team
        .image_message(&stranger, "private", image(0))
        .await?;
    stranger.wait_until_running().await?;
    assert_eq!(
        request(&client, &url, private_resource.resource_id, auth)
            .send()
            .await?
            .status(),
        403
    );
    let response = request(&client, &url, resource.resource_id, auth)
        .header("range", "bytes=2-8")
        .send()
        .await?;
    assert_eq!(response.status(), 206);
    assert_eq!(response.bytes().await?.as_ref(), &image(0)[2..=8]);
    let response = request(&client, &url, resource.resource_id, auth)
        .header("range", "bytes=999999-")
        .send()
        .await?;
    assert_eq!(response.status(), 416);
    // Observe actual wall-clock expiration while the attempt is still running.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = request(&client, &url, resource.resource_id, auth)
                .send()
                .await?
                .status();
            if status == 403 {
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            assert_eq!(status, 200);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    assert_eq!(agent.executions().await?[0].state, "running");
    rig.close().await
}

#[tokio::test]
async fn corrupted_download_is_discarded_before_the_harness_starts() -> Result<()> {
    let mut rig = SystemRig::start(config()).await?;
    let (url, _) = rig.computer_endpoint()?;
    let cutter = Cutter::start(&url, None, true).await?;
    rig.route_computer_through(&cutter.url).await?;
    let team = rig.workspace("team").await?;
    let agent = team
        .agent("bruce")
        .harness(Script::LinkReply {
            text: "seen".into(),
        })
        .create()
        .await?;
    let resource = team.image_message(&agent, "images", image(0)).await?;
    agent
        .wait_for(|executions, _| {
            executions.iter().any(|execution| {
                execution.error_code.as_deref() == Some("resource_integrity_mismatch")
            })
        })
        .await?;
    let path = agent
        .resource_directory()?
        .join(format!("{}-photo.png", resource.resource_id));
    assert!(!path.exists());
    assert!(!path.with_extension("part").exists());
    assert!(agent.harness_inputs()?.is_empty());
    rig.close().await
}
