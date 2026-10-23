use lankide_protocol::{
    link::{self, MachineLink},
    *,
};
use lankide_system_e2e::{Result, RigConfig, Script, SystemRig};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio::time::timeout;

#[tokio::test]
async fn s15_cli_capabilities_and_attachments_use_attempt_proxy_without_runtime_address()
-> Result<()> {
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_system-cli"));
    let mut rig = SystemRig::start(RigConfig {
        harness_executable: env!("CARGO_BIN_EXE_system-harness").into(),
        machine_ws_only: true,
        network_proxy: true,
        link: link::LinkSettings {
            heartbeat_interval: Duration::from_millis(200),
            handshake_timeout: Duration::from_millis(800),
        },
        timeout: Duration::from_secs(30),
        ..Default::default()
    })
    .await?;
    assert!(rig.homes_are_separate());
    assert!(!rig.machine_has_runtime_socket());
    let team = rig.workspace("team").await?;
    let bruce = team
        .agent("bruce")
        .harness(Script::ProxyScenario {
            binary: binary.display().to_string(),
        })
        .create()
        .await?;
    team.mention(&bruce, "team-chat", "alice", "Please read this message")
        .await?;
    let home = bruce.resource_directory()?.parent().unwrap().to_path_buf();
    let report = timeout(Duration::from_secs(20), async {
        loop {
            if let Ok(bytes) = std::fs::read(home.join("proxy-report.json")) {
                break serde_json::from_slice(&bytes);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let report: Value = match report {
        Ok(report) => report?,
        Err(error) => {
            return Err(format!(
                "{error}; executions: {:?}; Machine diagnostics: {:?}; CLI journal: {:?}",
                bruce.executions().await?,
                rig.machine_failure_log(),
                std::fs::read_to_string(home.join("proxy-cli.log")),
            )
            .into());
        }
    };
    for result in report["invalid_attachments"].as_array().unwrap() {
        assert_eq!(
            result["error"]["code"], "invalid_input",
            "wrong attachment shape must be actionable: {result}"
        );
    }
    assert_eq!(report["local_markdown"]["success"], true);
    assert_eq!(
        report["content"]["content_markdown"],
        "Please read this message"
    );
    assert_eq!(report["updated"]["name"], "Bruce Lee");
    assert_eq!(report["invoked"]["success"], true);
    assert_eq!(report["sent"]["accepted"], true);
    assert!(
        report["capabilities"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "message_send")
    );
    let env = report["env"].as_object().unwrap();
    assert!(
        !env.keys().any(|key| key.starts_with("LANKIDE_RUNTIME_")),
        "{env:?}"
    );
    let socket = PathBuf::from(env["LANKIDE_AGENT_SOCKET"].as_str().unwrap());
    assert!(socket.starts_with(rig.machine_home()));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&socket)?.permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(socket.parent().unwrap())?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let context_path = PathBuf::from(env["LANKIDE_RUN_CONTEXT"].as_str().unwrap());
    assert_eq!(
        std::fs::metadata(&context_path)?.permissions().mode() & 0o777,
        0o600
    );
    let context: serde_json::Value = serde_json::from_slice(&std::fs::read(&context_path)?)?;
    assert_eq!(context["socket"], socket.to_string_lossy().as_ref());
    assert_eq!(context["execution_token"], env["LANKIDE_EXECUTION_TOKEN"]);
    let old_context = AgentRunContext {
        execution: Some(ExecutionRef {
            execution_id: env["LANKIDE_EXECUTION_ID"].as_str().unwrap().parse()?,
            attempt: env["LANKIDE_EXECUTION_ATTEMPT"].as_str().unwrap().parse()?,
        }),
        execution_token: env["LANKIDE_EXECUTION_TOKEN"].as_str().unwrap().into(),
    };
    let mut local = connect_uds(&socket, DEFAULT_MAX_MESSAGE_BYTES).await?;
    let mut second = local.clone();
    let (capabilities, profile) = tokio::join!(
        local.list_capabilities(old_context.clone()),
        second.get_self_profile(old_context.clone()),
    );
    assert!(!capabilities?.into_inner().capabilities.is_empty());
    assert_eq!(profile?.into_inner().name, "Bruce Lee");
    let proxy_management =
        lankide_client::http::HttpManagementTransport::local(&socket, DEFAULT_MAX_MESSAGE_BYTES);
    assert_eq!(
        proxy_management
            .list_workspaces(ListWorkspacesRequest {
                request_id: "no-management".into(),
            })
            .await
            .unwrap_err()
            .code(),
        lankide_protocol::Code::Unavailable
    );
    let mut wrong = old_context.clone();
    wrong.execution_token = "wrong-token".into();
    assert_eq!(
        local.list_capabilities(wrong).await.unwrap_err().code(),
        lankide_protocol::Code::Unauthenticated
    );
    assert!(
        !rig.runtime_run().join("runtime.sock").exists(),
        "Runtime uses HTTP discovery"
    );
    // Stall replies at the network boundary: the call must fail as retryable
    // Unavailable when the Machine detects the silent link.
    rig.proxy.as_ref().unwrap().pause();
    let failure = timeout(
        Duration::from_secs(3),
        local.list_capabilities(old_context.clone()),
    )
    .await?
    .unwrap_err();
    assert_eq!(failure.code(), lankide_protocol::Code::Unavailable);
    rig.proxy.as_ref().unwrap().cut();
    let failure = timeout(
        Duration::from_secs(1),
        local.list_capabilities(old_context.clone()),
    )
    .await?
    .unwrap_err();
    assert_eq!(failure.code(), lankide_protocol::Code::Unavailable);
    rig.proxy.as_ref().unwrap().restore();
    rig.wait_machine_online().await?;
    timeout(Duration::from_secs(3), async {
        loop {
            if local.list_capabilities(old_context.clone()).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    bruce.release_run().await?;
    rig.settle().await?;
    assert!(!socket.exists(), "attempt socket was not removed");
    assert!(
        !context_path.exists(),
        "attempt credentials were not removed"
    );
    assert!(
        local.list_capabilities(old_context.clone()).await.is_err(),
        "open connections must stop working"
    );
    let transcript = team.transcript(&bruce, "team-chat");
    assert_eq!(transcript.len(), 3);
    assert_eq!(transcript[1], "attachment.txt");
    assert!(transcript[2].contains("Sent with an attachment"));
    assert!(transcript[2].contains("`[example](./missing-inline.txt)`"));
    assert!(transcript[2].contains("![example](./missing-fenced.png)"));
    let sends = team.outbound(&bruce, "team-chat");
    assert_eq!(sends.len(), 2);
    assert!(sends[1].reply_to.is_some());
    assert_eq!(sends[0].resources.len(), 2);
    assert_eq!(sends[0].resources[1].resource.media_type, "image/png");
    for (index, send) in sends.into_iter().enumerate() {
        assert_eq!(send.resources.len(), if index == 0 { 2 } else { 1 });
        assert_eq!(send.resources[0].resource.size_bytes, 1_200_000);
        assert_eq!(send.resources[0].resource.media_type, "text/plain");
    }
    rig.stop_machine()?;
    // S15.3: replay the credential on an authenticated WS after the attempt finishes.
    let (url, _) = rig.machine_endpoint()?;
    let credentials: Value = serde_json::from_slice(&std::fs::read(
        rig.machine_home().join("machine-credentials.json"),
    )?)?;
    let secret = credentials["device_credential"].as_str().unwrap();
    let machine_id = credentials["machine_id"].as_str().unwrap();
    let mut link = link::connect(&url, secret, DEFAULT_MAX_MESSAGE_BYTES).await?;
    link.send(link::MachineFrame {
        protocol_version: PROTOCOL_VERSION,
        frame: Some(link::MachinePayload::Hello(link::MachineHello {
            device_credential: secret.into(),
            machine_id: machine_id.into(),
            instance_id: "credential-replay".into(),
            machine_version: "1".into(),
            os: "test".into(),
            arch: "test".into(),
            protocol_version: PROTOCOL_VERSION,
            harnesses: vec![],
        })),
    })
    .await?;

    link.send(link::MachineFrame {
        protocol_version: PROTOCOL_VERSION,
        frame: Some(link::MachinePayload::CapabilityRequest(
            link::CapabilityRequest {
                request_id: "finished-token".into(),
                method: "list_capabilities".into(),
                context: Some(old_context.clone().into()),
                payload: lankide_protocol::agent::encode(&old_context).expect("capability JSON"),
                offset: 0,
                eof: true,
            },
        )),
    })
    .await?;
    let response = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(frame) = link.receive().await? {
                if let Some(link::RuntimePayload::CapabilityResponse(response)) = frame.frame {
                    break Ok::<_, link::LinkError>(response);
                }
            } else {
                panic!("link ended before token rejection");
            }
        }
    })
    .await??;
    assert_eq!(
        response.error_code,
        lankide_protocol::Code::PermissionDenied as i32,
        "finished token accepted: {}",
        json!(response)
    );
    rig.close().await
}
