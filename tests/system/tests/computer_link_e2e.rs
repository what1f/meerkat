use lankide_protocol::link::{
    self, CloseReason, ComputerFrame, ComputerHello, ComputerLink, ComputerPayload, LinkError,
};
use lankide_protocol::{DEFAULT_MAX_MESSAGE_BYTES, PROTOCOL_VERSION};
use lankide_system_e2e::{OutboundKind, Result, RigConfig, Script, SystemRig};
use std::time::Duration;
use tokio::time::timeout;

fn config() -> RigConfig {
    RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        computer_ws_only: true,
        ..Default::default()
    }
}
fn hello(version: u32) -> ComputerFrame {
    ComputerFrame {
        protocol_version: version,
        frame: Some(ComputerPayload::Hello(ComputerHello {
            device_credential: String::new(),
            protocol_version: version,
            computer_id: "newer-computer".into(),
            instance_id: "first".into(),
            computer_version: "1".into(),
            os: "test-os".into(),
            arch: "test-arch".into(),
            harnesses: Vec::new(),
        })),
    }
}

#[tokio::test]
async fn s9_1_newer_computer_gets_a_typed_rejection_visible_in_management() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    use lankide_client::ManagementClient;
    let (url, _) = rig.computer_endpoint()?;
    let home = tempfile::tempdir()?;
    let code = rig.management().create_pairing_code().await?;
    let credentials = lankide_computer_daemon::pair(
        &lankide_computer_daemon::ComputerDaemonPaths::for_home(home.path())?,
        &url,
        &code.code,
    )
    .await?;
    let secret = credentials.device_credential.clone();
    let mut newer = hello(PROTOCOL_VERSION + 1);
    if let Some(ComputerPayload::Hello(hello)) = &mut newer.frame {
        hello.computer_id = credentials.computer_id.clone();
        hello.device_credential = secret.clone();
    }
    let mut link = link::connect(&url, &secret, DEFAULT_MAX_MESSAGE_BYTES).await?;
    link.send(newer.clone()).await?;
    let result = timeout(Duration::from_secs(3), link.receive())
        .await?
        .map_err(lankide_computer::ComputerError::from);
    assert!(
        matches!(
            result,
            Err(lankide_computer::ComputerError::ProtocolVersionUnsupported)
        ),
        "{result:?}"
    );
    let management = lankide_client::http::HttpManagementTransport::local(
        rig.runtime_socket(),
        DEFAULT_MAX_MESSAGE_BYTES,
    );
    let catalog = management
        .get_computer_catalog(lankide_protocol::CatalogRequest {
            request_id: "version-rejection".into(),
            workspace: None,
        })
        .await?
        .into_inner();
    let rejected = catalog
        .computers
        .iter()
        .find(|computer| computer.computer_id == credentials.computer_id)
        .expect("rejected Computer in catalog");
    assert_eq!(
        rejected.observed_state,
        lankide_protocol::ObservedState::Error as i32
    );
    assert_eq!(rejected.status_message, "protocol_version_unsupported");
    // A supported reconnect clears the rejection without changing storage.
    let mut compatible = link::connect(&url, &secret, DEFAULT_MAX_MESSAGE_BYTES).await?;
    newer.protocol_version = PROTOCOL_VERSION;
    if let Some(ComputerPayload::Hello(hello)) = &mut newer.frame {
        hello.protocol_version = PROTOCOL_VERSION;
    }
    compatible.send(newer).await?;
    assert!(matches!(
        timeout(Duration::from_secs(3), compatible.receive())
            .await??
            .and_then(|frame| frame.frame),
        Some(link::RuntimePayload::Hello(_))
    ));
    // Hold the active-attempt report at the link boundary. A hello alone must
    // not make the Computer available, even with an otherwise healthy socket.
    let before_report = management
        .get_computer_catalog(lankide_protocol::CatalogRequest {
            request_id: "before-reconciliation".into(),
            workspace: None,
        })
        .await?
        .into_inner();
    assert_eq!(
        before_report
            .computers
            .iter()
            .find(|computer| computer.computer_id == credentials.computer_id)
            .unwrap()
            .observed_state,
        lankide_protocol::ObservedState::Stopped as i32
    );
    // Availability requires the first active-attempt report, as on the real peer.
    compatible
        .send(ComputerFrame {
            protocol_version: PROTOCOL_VERSION,
            frame: Some(ComputerPayload::Heartbeat(link::ComputerHeartbeat {
                computer_id: credentials.computer_id.clone(),
                instance_id: "first".into(),
                sent_at: 0,
                active_attempts: Vec::new(),
            })),
        })
        .await?;
    timeout(Duration::from_secs(3), async {
        loop {
            let catalog = management
                .get_computer_catalog(lankide_protocol::CatalogRequest {
                    request_id: "compatible-reconnect".into(),
                    workspace: None,
                })
                .await?
                .into_inner();
            let computer = catalog
                .computers
                .iter()
                .find(|computer| computer.computer_id == credentials.computer_id)
                .unwrap();
            if computer.observed_state == lankide_protocol::ObservedState::Running as i32 {
                assert!(computer.status_message.is_empty());
                return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    compatible.close(CloseReason::Shutdown).await?;
    rig.close().await
}

#[tokio::test]
async fn s9_2_separate_homes_reply_to_a_mention_through_ws_only() -> Result<()> {
    let rig = SystemRig::start(config()).await?;
    assert!(rig.homes_are_separate());
    assert!(!rig.computer_has_runtime_socket());
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::LinkReply { text: "hi".into() })
        .create()
        .await?;
    let clark = team
        .agent("clark")
        .harness(Script::LinkReply { text: "hey".into() })
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "hello bruce")
        .await?;
    rig.settle().await?;
    assert_eq!(team.transcript(&bruce, "team-chat"), ["hello bruce", "hi"]);
    let sends = team.outbound(&bruce, "team-chat");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].kind, OutboundKind::Reply);
    let executions = bruce.executions().await?;
    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].state, "succeeded");
    assert!(clark.executions().await?.is_empty());
    assert!(team.outbound(&clark, "team-chat").is_empty());
    rig.close().await
}

#[tokio::test]
async fn ws_rejects_unauthorized_and_invalid_connections_without_disrupting_a_computer()
-> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use std::os::unix::fs::PermissionsExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let rig = SystemRig::start(config()).await?;
    let (url, secret) = rig.computer_endpoint()?;
    let run = rig.runtime_socket().parent().unwrap();
    for name in ["computer-link.url", "computer-link.secret"] {
        assert_eq!(
            std::fs::metadata(run.join(name))?.permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(matches!(
        link::connect(&url, "wrong-secret", DEFAULT_MAX_MESSAGE_BYTES).await,
        Err(LinkError::Closed(CloseReason::ComputerUnknown))
    ));
    let rejected = tokio_tungstenite::connect_async(&url).await;
    assert!(
        matches!(rejected, Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status() == 401)
    );
    for (json, expected) in [
        ("{".into(), "malformed_json"),
        (
            format!(
                r#"{{"protocol_version":{PROTOCOL_VERSION},"type":"future_frame","payload":{{}}}}"#
            ),
            "unknown_frame_type",
        ),
    ] {
        let mut request = url.clone().into_client_request()?;
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {secret}").parse()?);
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
        socket.send(Message::Text(json.into())).await?;
        let response = timeout(Duration::from_secs(3), socket.next())
            .await?
            .expect("close frame")?;
        assert!(
            matches!(response, Message::Close(Some(frame)) if frame.reason == expected),
            "expected {expected}"
        );
    }
    // A valid hello above the byte limit must never reach the broker. TCP may
    // reset a rejected large frame before its sender has finished writing it.
    let mut oversized = hello(PROTOCOL_VERSION);
    if let Some(ComputerPayload::Hello(hello)) = &mut oversized.frame {
        hello.computer_version = "x".repeat(DEFAULT_MAX_MESSAGE_BYTES);
    }
    let mut request = url.clone().into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {secret}").parse()?);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    if timeout(
        Duration::from_secs(3),
        socket.send(Message::Text(serde_json::to_string(&oversized)?.into())),
    )
    .await?
    .is_ok()
    {
        let response = timeout(Duration::from_secs(3), socket.next()).await?;
        match response {
            Some(Ok(Message::Close(Some(frame)))) => assert_eq!(frame.reason, "message_too_large"),
            Some(Err(_)) | None => {}
            other => panic!("oversized hello was not rejected: {other:?}"),
        }
    }
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::LinkReply {
            text: "still here".into(),
        })
        .create()
        .await?;
    team.mention(&bruce, "chat", "alice", "hello").await?;
    rig.settle().await?;
    assert_eq!(team.transcript(&bruce, "chat"), ["hello", "still here"]);
    rig.close().await
}
