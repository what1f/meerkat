//! A1: browser-facing DTOs through the real Runtime HTTP boundary.
use futures_util::StreamExt;
use lankide_protocol::management;
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

struct Http {
    client: reqwest::Client,
    url: String,
    owner: String,
}
impl Http {
    fn for_rig(rig: &SystemRig) -> Result<Self> {
        let run = rig.runtime_home().join("run");
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?,
            url: std::fs::read_to_string(run.join(management::URL_FILE))?,
            owner: std::fs::read_to_string(run.join(management::OWNER_FILE))?,
        })
    }
    async fn post(&self, op: &str, body: Value) -> Result<Value> {
        Ok(self
            .client
            .post(format!("{}/api/v1/{op}", self.url))
            .bearer_auth(&self.owner)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}

#[tokio::test]
async fn a1_1_string_states_round_trip_and_agent_still_replies() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let team = rig.workspace("team").await?;
        let agent = team.agent("bruce").harness(Script::reply("done")).create().await?;
        agent.group_attention(lankide_protocol::AttentionMode::Mention).await?;
        let http = Http::for_rig(&rig)?;
        let workspace = json!({"workspace_id": "team"});
        let agent_ref = json!({"workspace_id": "team", "agent_id": "bruce"});
        let agents = http.post("list_agents", json!({"request_id":"list", "workspace":workspace})).await?;
        let attention = &agents["agents"][0]["attention"];
        assert_eq!(attention["channels"][0]["mode"], "mention");
        assert!(agents["agents"][0]["created_at"].is_number());
        assert!(agents["agents"][0]["pause_reason"].is_null());
        let update = http.post("update_agent", json!({"request_id":"update", "agent":agent_ref, "attention":attention})).await?;
        assert_eq!(&update["attention"], attention);

        let catalog = http.post("get_machine_catalog", json!({"request_id":"machines", "workspace":null})).await?;
        let machine = &catalog["machines"][0];
        assert_eq!(machine["observed_state"], "running");
        assert_eq!(machine["harnesses"][0]["availability"], "available");
        let harnesses = http.post("get_harness_catalog", json!({"request_id":"harnesses", "workspace":null})).await?;
        assert_eq!(harnesses["harnesses"][0]["availability"], "available");
        let onboarding = http.post("get_agent_onboarding", json!({"request_id":"onboarding", "agent":agent_ref})).await?;
        assert_eq!(onboarding["status"], "ready");
        let workspaces = http.post("list_workspaces", json!({"request_id":"workspaces"})).await?;
        let active = &workspaces["workspaces"][0];
        assert_eq!(active["desired_state"], "running");
        assert_eq!(active["observed_state"], "running");
        let mut handshake = format!("{}/api/v1/events", http.url.replacen("http://", "ws://", 1)).into_client_request()?;
        handshake.headers_mut().insert("authorization", format!("Bearer {}", http.owner).parse()?);
        handshake.headers_mut().insert("x-lankide-subscription", json!({"request_id":"events", "workspace":workspace, "after_sequence":0}).to_string().parse()?);
        let (mut events, _) = tokio_tungstenite::connect_async(handshake).await?;
        let paused = http.post("set_workspace_state", json!({"request_id":"pause", "workspace":workspace, "desired_state":"paused"})).await?;
        let frame = tokio::time::timeout(Duration::from_secs(5), events.next()).await?.ok_or("events ended")??;
        let event: Value = serde_json::from_str(frame.to_text()?)?;
        assert_eq!(event["type"], "event");
        assert_eq!(event["data"]["kind"], "workspace_state_changed");
        drop(events);
        assert_eq!(paused["desired_state"], "paused");
        assert_eq!(paused["observed_state"], "paused");
        let resumed = http.post("set_workspace_state", json!({"request_id":"resume", "workspace":workspace, "desired_state":active["desired_state"]})).await?;
        assert_eq!(resumed["desired_state"], active["desired_state"]);
        // Numeric and unknown enum inputs are rejected before changing state.
        for state in [json!(2), json!("invalid")] {
            let response = http.client.post(format!("{}/api/v1/set_workspace_state", http.url))
                .bearer_auth(&http.owner).json(&json!({"request_id":"invalid", "workspace":workspace, "desired_state":state})).send().await?;
            assert_eq!(response.status(), 400);
        }
        team.mention(&agent, "group", "alice", "hello").await?;
        rig.settle().await?;
        assert_eq!(team.transcript(&agent, "group"), ["hello", "done"]);
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a1_2_local_and_remote_identity_survives_runtime_restart() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig {
        local_discovery: true,
        ..Default::default()
    })
    .await?;
    let scenario = async {
        let remote = rig.add_machine().await?;
        let original = lankide_machine::read_credentials(rig.machine_home())?
            .ok_or("missing local credentials")?;
        let local_id = original.machine_id.clone();
        {
            let team = rig.workspace("local_team").await?;
            team.agent("bruce")
                .machine(&local_id)
                .harness(Script::reply("done"))
                .create()
                .await?;
        }
        use lankide_client::ManagementClient;
        rig.management().rename_machine(&local_id, "My Mac").await?;
        // Lost local credentials must recover the same identity, including Agent placement.
        std::fs::remove_file(rig.machine_home().join("machine-credentials.json"))?;
        rig.restart_machine().await?;
        // The catalog can briefly retain the old connection after stop; wait for
        // the daemon to publish its renewed credentials before observing recovery.
        let renewed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(credentials) = lankide_machine::read_credentials(rig.machine_home())? {
                    break Ok::<_, Box<dyn std::error::Error + Send + Sync>>(credentials);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await??;
        assert_eq!(renewed.machine_id, local_id);
        assert_ne!(renewed.device_credential, original.device_credential);
        let (url, _) = rig.local_pairing_endpoint()?;
        assert!(
            lankide_protocol::link::connect(
                &url,
                &original.device_credential,
                lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES
            )
            .await
            .is_err()
        );
        for restart in [false, true] {
            if restart {
                rig.restart_runtime().await?;
            }
            let http = Http::for_rig(&rig)?;
            let catalog = http
                .post(
                    "get_machine_catalog",
                    json!({"request_id":"identity", "workspace":null}),
                )
                .await?;
            let machines = catalog["machines"].as_array().ok_or("missing catalog")?;
            assert_eq!(machines.iter().filter(|m| m["is_local"] == true).count(), 1);
            assert_eq!(
                machines
                    .iter()
                    .find(|m| m["machine_id"] == local_id)
                    .ok_or("missing local")?["is_local"],
                true
            );
            assert_eq!(machines.len(), 2, "re-pairing must not create another row");
            let local = machines
                .iter()
                .find(|m| m["machine_id"] == local_id)
                .ok_or("missing local")?;
            assert_eq!(local["display_name"], "My Mac");
            let team = rig.existing_workspace("local_team");
            let agent = team.existing_agent("bruce");
            assert_eq!(agent.view().await?.machine_id, local_id);
            team.direct_message(
                &agent,
                "alice",
                if restart {
                    "after restart"
                } else {
                    "after re-pair"
                },
            )
            .await?;
            rig.settle().await?;
            let expected = if restart {
                vec!["after re-pair", "done", "after restart", "done"]
            } else {
                vec!["after re-pair", "done"]
            };
            assert_eq!(team.dm_transcript(&agent, "alice"), expected);
            assert_eq!(
                machines
                    .iter()
                    .find(|m| m["machine_id"] == remote.id)
                    .ok_or("missing remote")?["is_local"],
                false
            );
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
