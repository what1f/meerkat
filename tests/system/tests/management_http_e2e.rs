//! Owner management uses the shared listener without granting Computer access.
use lankide_client::{ManagementClient, ManagementEventKind};
use lankide_protocol::{DEFAULT_MAX_MESSAGE_BYTES, management};
use lankide_system_e2e::{Result, RigConfig, SystemRig};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Error, client::IntoClientRequest};

#[tokio::test]
async fn owner_manages_over_http_and_events_while_other_credentials_are_rejected() -> Result<()> {
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let run = rig.runtime_home().join("run");
        let url = std::fs::read_to_string(run.join(management::URL_FILE))?;
        let owner_path = run.join(management::OWNER_FILE);
        let owner = std::fs::read_to_string(&owner_path)?;
        assert_eq!(
            std::fs::metadata(&owner_path)?.permissions().mode() & 0o777,
            0o600
        );
        let (_, device) = rig.computer_endpoint()?;
        let (_, pairing) = rig.local_pairing_endpoint()?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?;
        for credential in [
            None,
            Some("wrong-owner"),
            Some(device.as_str()),
            Some(pairing.as_str()),
        ] {
            let mut request = http.post(format!("{url}/api/v1/list_workspaces")).json(
                &management::ListWorkspacesRequest {
                    request_id: "unauthorized-owner".into(),
                },
            );
            if let Some(credential) = credential {
                request = request.bearer_auth(credential);
            }
            let response = request.send().await?;
            assert_eq!(response.status(), 401);
            let error: management::ErrorBody = response.json().await?;
            assert_eq!(error.code, management::ErrorCode::Unauthenticated);

            let mut request = format!(
                "{}{}",
                url.replacen("http://", "ws://", 1),
                management::EVENTS_PATH
            )
            .into_client_request()?;
            if let Some(credential) = credential {
                request
                    .headers_mut()
                    .insert("authorization", format!("Bearer {credential}").parse()?);
            }
            let result = timeout(
                Duration::from_secs(3),
                tokio_tungstenite::connect_async(request),
            )
            .await?;
            assert!(matches!(result, Err(Error::Http(response)) if response.status() == 401));
        }
        // The Desktop receives the same typed authorization error on either transport.
        let foreign_home = tempfile::tempdir()?;
        std::fs::write(foreign_home.path().join(management::URL_FILE), &url)?;
        std::fs::write(foreign_home.path().join(management::OWNER_FILE), &device)?;
        let unauthorized =
            lankide_client::RuntimeManagementClient::connect_local_lazy(foreign_home.path());
        assert!(matches!(
            unauthorized.list_workspaces().await,
            Err(lankide_client::ClientError::Provider(_))
        ));
        assert!(matches!(
            unauthorized.subscribe_management_events(None, 0).await,
            Err(lankide_client::ClientError::Provider(_))
        ));
        // The owner cannot cross into the Computer entry point.
        let (computer_url, _) = rig.local_pairing_endpoint()?;
        assert!(
            lankide_protocol::link::connect(&computer_url, &owner, DEFAULT_MAX_MESSAGE_BYTES)
                .await
                .is_err()
        );

        let mut events = rig
            .management()
            .subscribe_management_events(Some("owner_team"), 0)
            .await?;
        rig.workspace("other_team").await?;
        let team = rig.workspace("owner_team").await?;
        assert_eq!(
            rig.management().get_workspace("owner_team").await?.id,
            "owner_team"
        );
        let event = timeout(Duration::from_secs(3), events.next())
            .await?
            .ok_or("event stream ended")??;
        assert_eq!(event.workspace_id, "owner_team");
        assert_eq!(event.kind, ManagementEventKind::WorkspaceCreated);
        assert!(event.sequence > 0);
        drop(team);
        drop(events);
        rig.restart_runtime().await?;
        assert_eq!(std::fs::read_to_string(&owner_path)?, owner);
        assert_eq!(
            rig.management().get_workspace("owner_team").await?.id,
            "owner_team"
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn wildcard_listener_keeps_local_desktop_management_and_closes_events_on_shutdown()
-> Result<()> {
    use lankide_runtime_daemon::{RuntimePaths, ShutdownReason, run_daemon};
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("runtime.json"),
        br#"{"computer_link_listen":"0.0.0.0:0"}"#,
    )?;
    let paths = RuntimePaths::for_home(home.path())?;
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let daemon = tokio::spawn(run_daemon(paths.clone(), Default::default(), async move {
        let _ = stopped.await;
        Ok(ShutdownReason::Internal)
    }));
    let scenario = async {
        let client = timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(client) =
                    lankide_client::RuntimeManagementClient::connect_local(&paths.run).await
                {
                    return client;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        let url = std::fs::read_to_string(paths.run.join(management::URL_FILE))?;
        assert_eq!(reqwest::Url::parse(&url)?.host_str(), Some("127.0.0.1"));
        assert!(client.list_workspaces().await?.is_empty());
        let events = client.subscribe_management_events(None, 0).await?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(events)
    }
    .await;
    let _ = stop.send(());
    timeout(Duration::from_secs(5), daemon).await???;
    let mut events = scenario?;
    let ended = timeout(Duration::from_secs(3), events.next()).await?;
    assert!(ended.is_none() || ended.is_some_and(|event| event.is_err()));
    assert!(!paths.run.join(management::URL_FILE).exists());
    Ok(())
}

#[tokio::test]
async fn paused_workspace_agent_can_be_renamed_and_runs_after_resume() -> Result<()> {
    use lankide_client::WorkspaceStatus;
    use lankide_system_e2e::Script;
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let team = rig.workspace("team").await?;
    team.agent("bruce")
        .harness(Script::reply("done"))
        .create()
        .await?;
    rig.management()
        .set_workspace_state("team", WorkspaceStatus::Paused)
        .await?;
    // Paused workspaces have no live Provider entry after startup.
    rig.restart_runtime().await?;
    let mut agent = rig.management().get_agent("team", "bruce").await?;
    agent.name = "Bruce renamed".into();
    let saved = rig.management().update_agent(agent, None).await?;
    assert_eq!(saved.name, "Bruce renamed");
    rig.management()
        .set_workspace_state("team", WorkspaceStatus::Active)
        .await?;
    let team = rig.existing_workspace("team");
    let agent = team.existing_agent("bruce");
    team.direct_message(&agent, "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(agent.view().await?.name, "Bruce renamed");
    assert_eq!(team.dm_transcript(&agent, "alice"), ["hello", "done"]);
    rig.close().await
}
