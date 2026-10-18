//! Local browser access control on the shared management listener.
use futures_util::StreamExt;
use lankide_client::{ManagementClient, ManagementEventKind};
use lankide_protocol::management;
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::{Error, Message, client::IntoClientRequest};

// A2.1: the management client creates business data and subscribes without a secret.
#[tokio::test]
async fn a2_1_local_client_creates_workspace_agent_and_subscribes_without_credentials() -> Result<()>
{
    let mut rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let mut events = rig
            .management()
            .subscribe_management_events(Some("local_team"), 0)
            .await?;
        let team = rig.workspace("local_team").await?;
        team.agent("bruce")
            .harness(Script::reply("done"))
            .create()
            .await?;
        assert_eq!(rig.management().list_agents("local_team").await?.len(), 1);
        let event = timeout(Duration::from_secs(3), events.next())
            .await?
            .ok_or("closed")??;
        assert_eq!(event.workspace_id, "local_team");
        assert_eq!(event.kind, ManagementEventKind::WorkspaceCreated);
        rig.restart_runtime().await?;
        assert_eq!(
            rig.management().get_workspace("local_team").await?.id,
            "local_team"
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a2_2_cross_site_requests_cannot_change_business_data_on_any_local_route() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let url = std::fs::read_to_string(rig.runtime_run().join(management::URL_FILE))?;
        let http = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build()?;
        let port = reqwest::Url::parse(&url)?.port().ok_or("missing port")?;
        // Valid mutation input: a missing check would create a Workspace.
        let body = serde_json::json!({"request_id":"blocked", "workspace_id":"blocked", "name":"Blocked", "rules":"", "provider_id":"system_im", "provider_config_json":"{}", "provider_secrets_json":"{}"});
        for (header, value, status) in [
            ("host", format!("evil.test:{port}"), 403),
            ("origin", "https://evil.test".into(), 403),
            ("origin", "null".into(), 403),
            ("content-type", "text/plain".into(), 415),
        ] {
            for path in ["/api/v1/create_workspace", "/machines/pair", "/resources/1", "/", "/unknown"] {
                let response = http.post(format!("{url}{path}")).json(&body).header(header, &value).send().await?;
                assert_eq!(response.status(), status, "{path} {header}");
                let error: management::ErrorBody = response.json().await?;
                assert_eq!(error.code, if status == 403 {management::ErrorCode::PermissionDenied} else {management::ErrorCode::InvalidArgument});
            }
        }
        for path in [management::EVENTS_PATH, "/"] {
            for (header, value) in [("origin", "https://evil.test".to_owned()), ("host", format!("evil.test:{port}"))] {
                let mut request = format!("{}{path}", url.replacen("http://", "ws://", 1)).into_client_request()?;
                request.headers_mut().insert(header, value.parse()?);
                let result = timeout(Duration::from_secs(3), tokio_tungstenite::connect_async(request)).await?;
                match result {
                    Err(Error::Http(response)) => {
                        assert_eq!(response.status(), 403);
                        let error: management::ErrorBody = serde_json::from_slice(response.body().as_ref().ok_or("missing error")?)?;
                        assert_eq!(error.code, management::ErrorCode::PermissionDenied);
                    }
                    _ => return Err("cross-site WS was not rejected".into()),
                }
            }
        }
        assert!(rig.management().list_workspaces().await?.is_empty());
        assert_eq!(rig.management().get_machine_catalog(None).await?.len(), 1);
        // Positive control: the same mutation succeeds after valid local checks.
        let response = http.post(format!("{url}/api/v1/create_workspace"))
            .header("origin", &url).json(&body).send().await?;
        assert_eq!(response.status(), 200);
        assert_eq!(rig.management().get_workspace("blocked").await?.name, "Blocked");
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a2_3_same_origin_post_and_query_events_work_for_both_local_hosts() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let url = std::fs::read_to_string(rig.runtime_run().join(management::URL_FILE))?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?;
        let port = reqwest::Url::parse(&url)?.port().ok_or("missing port")?;
        for (index, host) in ["127.0.0.1", "localhost"].iter().enumerate() {
            let origin = format!("http://{host}:{port}");
            let response = http
                .post(format!("{url}/api/v1/list_workspaces"))
                .header("host", format!("{host}:{port}"))
                .header("origin", &origin)
                .header("content-type", "application/json; charset=utf-8")
                .body(r#"{"request_id":"same-origin"}"#)
                .send()
                .await?;
            assert_eq!(response.status(), 200);
            let workspace = format!("query_{index}");
            let mut request = format!(
                "{}{}?unused=value&workspace={workspace}&after=0&extra=ignored&unused=again",
                url.replacen("http://", "ws://", 1),
                management::EVENTS_PATH
            )
            .into_client_request()?;
            request
                .headers_mut()
                .insert("host", format!("{host}:{port}").parse()?);
            request.headers_mut().insert("origin", origin.parse()?);
            let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
            rig.workspace(&format!("other_{index}")).await?;
            rig.workspace(&workspace).await?;
            let message = timeout(Duration::from_secs(3), socket.next())
                .await?
                .ok_or("closed")??;
            let Message::Text(text) = message else {
                return Err("expected event".into());
            };
            let management::EventFrame::Event(event) = serde_json::from_str(&text)? else {
                return Err("expected event frame".into());
            };
            assert_eq!(
                event.workspace.as_ref().map(|w| w.workspace_id.as_str()),
                Some(workspace.as_str())
            );
            assert!(event.sequence > 0);
            socket.close(None).await?;
        }
        // Optional parameters and a nonzero cursor use real Workspace events.
        for query in ["", "?workspace=query_0", "?after=0", "?unused=value"] {
            let (mut socket, _) = tokio_tungstenite::connect_async(format!(
                "{}{}{query}",
                url.replacen("http://", "ws://", 1),
                management::EVENTS_PATH
            ))
            .await?;
            socket.close(None).await?;
        }
        let (mut future_events, _) = tokio_tungstenite::connect_async(format!(
            "{}{}?workspace=query_0&after={}",
            url.replacen("http://", "ws://", 1),
            management::EVENTS_PATH,
            u64::MAX
        ))
        .await?;
        let mut workspace = rig.management().get_workspace("query_0").await?;
        workspace.name = "Updated query workspace".into();
        rig.management().update_workspace(workspace).await?;
        assert!(
            timeout(Duration::from_millis(150), future_events.next())
                .await
                .is_err(),
            "after must filter older event sequences"
        );
        future_events.close(None).await?;
        for query in [
            "?after=invalid",
            "?after=1&after=2",
            "?workspace=a&workspace=b",
        ] {
            let result = tokio_tungstenite::connect_async(format!(
                "{}{}{query}",
                url.replacen("http://", "ws://", 1),
                management::EVENTS_PATH
            ))
            .await;
            assert!(matches!(result, Err(Error::Http(response)) if response.status() == 400));
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

// A2.4 is exercised through the shipped daemon entrypoint.
#[tokio::test]
async fn a2_4_occupied_port_exits_nonzero_without_random_port_fallback() -> Result<()> {
    occupied_port_scenario(false).await
}

// Run separately: hermetic gates must never bind product port 7440.
#[tokio::test]
#[ignore = "binds product port 7440; run separately from the hermetic gate"]
async fn a2_4_default_7440_conflict_exits_nonzero() -> Result<()> {
    occupied_port_scenario(true).await
}

async fn occupied_port_scenario(default_port: bool) -> Result<()> {
    let occupied = std::net::TcpListener::bind(if default_port {
        "127.0.0.1:7440"
    } else {
        "127.0.0.1:0"
    })?;
    let address = occupied.local_addr()?;
    let home = tempfile::tempdir()?;
    let mut config = serde_json::json!({"remote_access_port":0});
    if !default_port {
        config["machine_link_listen"] = serde_json::json!(address.to_string());
    }
    std::fs::write(
        home.path().join("runtime.json"),
        serde_json::to_vec(&config)?,
    )?;
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_system-runtime"));
    let output = timeout(
        Duration::from_secs(5),
        command
            .arg("--home")
            .arg(home.path())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    assert!(!output.status.success());
    let stderr = format!(
        "{}\n{}",
        String::from_utf8(output.stderr)?,
        std::fs::read_to_string(home.path().join("logs/lankided.log")).unwrap_or_default()
    );
    assert!(stderr.contains(&address.port().to_string()), "{stderr}");
    assert!(!home.path().join("run").join(management::URL_FILE).exists());
    Ok(())
}

#[tokio::test]
async fn loopback_management_closes_events_on_shutdown() -> Result<()> {
    use lankided::{RuntimePaths, ShutdownReason, run_daemon};
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("runtime.json"),
        br#"{"machine_link_listen":"127.0.0.1:0","remote_access_port":0}"#,
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

#[tokio::test]
async fn management_cannot_be_configured_on_a_public_plaintext_listener() -> Result<()> {
    use lankided::{RuntimePaths, run_daemon};
    let home = tempfile::tempdir()?;
    std::fs::write(
        home.path().join("runtime.json"),
        br#"{"machine_link_listen":"0.0.0.0:0"}"#,
    )?;
    let paths = RuntimePaths::for_home(home.path())?;
    let result = run_daemon(paths.clone(), Default::default(), std::future::pending()).await;
    assert!(result.is_err());
    assert!(!paths.run.join(management::URL_FILE).exists());
    Ok(())
}

#[tokio::test]
async fn local_client_sees_machine_pairing_presence_and_revocation_from_events() -> Result<()> {
    use lankide_machine::{MachineDaemonPaths, pair};
    let mut rig = SystemRig::start(RigConfig {
        auto_start_machine: false,
        ..Default::default()
    })
    .await?;
    let scenario = async {
        let client = rig.management().clone();
        let mut events = client.subscribe_management_events(None, 0).await?;
        let code = client.create_pairing_code().await?;
        let (url, _) = rig.local_pairing_endpoint()?;
        let device = pair(
            &MachineDaemonPaths::for_home(rig.machine_home())?,
            &url,
            &code.code,
        )
        .await?;
        let event = timeout(Duration::from_secs(3), events.next())
            .await?
            .ok_or("pairing event stream ended")??;
        assert_eq!(event.kind, ManagementEventKind::MachineCatalogChanged);
        assert!(event.workspace_id.is_empty());
        let catalog = client.get_machine_catalog(None).await?;
        assert!(
            catalog
                .iter()
                .any(|c| c.id == device.machine_id && c.registered && !c.available)
        );

        rig.start_paired_machine()?;
        loop {
            timeout(Duration::from_secs(3), events.next())
                .await?
                .ok_or("online event stream ended")??;
            if client
                .get_machine_catalog(None)
                .await?
                .iter()
                .any(|c| c.id == device.machine_id && c.available)
            {
                break;
            }
        }
        // Start the offline observer after online reconciliation so an earlier
        // registration or inventory event cannot satisfy this transition.
        events = client.subscribe_management_events(None, 0).await?;
        rig.stop_machine()?;
        loop {
            timeout(Duration::from_secs(3), events.next())
                .await?
                .ok_or("offline event stream ended")??;
            if client
                .get_machine_catalog(None)
                .await?
                .iter()
                .any(|c| c.id == device.machine_id && !c.available)
            {
                break;
            }
        }
        drop(events);
        rig.restart_runtime().await?;
        // The same client discovers the restarted Runtime and subscribes again.
        let mut events = client
            .subscribe_management_events(Some("owner_team"), 0)
            .await?;
        client.revoke_machine(&device.machine_id).await?;
        timeout(Duration::from_secs(3), events.next())
            .await?
            .ok_or("revocation event stream ended")??;
        assert!(
            client
                .get_machine_catalog(None)
                .await?
                .iter()
                .any(|c| c.id == device.machine_id && c.revoked_at.is_some() && !c.available)
        );
        assert!(
            rig.await_terminal_machine()
                .await?
                .contains("machine_revoked")
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}

#[tokio::test]
async fn a4e_missing_agent_returns_not_found() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let team = rig.workspace("team").await?;
        team.agent("existing")
            .harness(Script::reply("done"))
            .create()
            .await?;
        let url = std::fs::read_to_string(rig.runtime_run().join(management::URL_FILE))?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?;
        for operation in ["get_agent", "update_agent", "delete_agent"] {
            let response = http
                .post(format!("{url}/api/v1/{operation}"))
                .json(&serde_json::json!({
                    "request_id": "missing-agent",
                    "agent": {"workspace_id": "team", "agent_id": "missing"}
                }))
                .send()
                .await?;
            assert_eq!(response.status(), 404, "{operation}");
            let error: management::ErrorBody = response.json().await?;
            assert_eq!(error.code, management::ErrorCode::NotFound, "{operation}");
        }
        assert_eq!(
            rig.management().get_agent("team", "existing").await?.id,
            "existing"
        );
        assert_eq!(rig.management().list_agents("team").await?.len(), 1);
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let cleanup = rig.close().await;
    scenario?;
    cleanup
}
