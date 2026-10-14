//! Pair a remote private Computer home through the shipped command.
use lankide_protocol::{CatalogRequest, DEFAULT_MAX_MESSAGE_BYTES};
use lankide_runtime_daemon::{RuntimePaths, ShutdownReason, run_daemon};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{sync::oneshot, time::timeout};

#[tokio::test]
async fn pair_command_registers_a_device_and_stores_private_credentials()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime_home = tempfile::tempdir()?;
    let computer_home = tempfile::tempdir()?;
    let paths = RuntimePaths::for_home(runtime_home.path())?;
    let (stop, stopped) = oneshot::channel();
    let server_paths = paths.clone();
    let server = tokio::spawn(run_daemon(
        server_paths,
        lankide_runtime::ProviderFactoryRegistry::default(),
        async move {
            let _ = stopped.await;
            Ok(ShutdownReason::Internal)
        },
    ));
    let scenario = async {
        let management = timeout(Duration::from_secs(5), async {
            loop {
                let client = lankide_client::http::HttpManagementTransport::local(
                    &paths.socket,
                    DEFAULT_MAX_MESSAGE_BYTES,
                );
                if client
                    .list_workspaces(lankide_protocol::ListWorkspacesRequest {
                        request_id: "pair-ready".into(),
                    })
                    .await
                    .is_ok()
                {
                    break client;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        let code = management
            .create_pairing_code(CatalogRequest {
                request_id: "cli-pair".into(),
                workspace: None,
            })
            .await?
            .into_inner();
        let (url, _) = lankide_protocol::link::discover(&paths.socket)?;
        let http_url = url.replacen("ws://", "http://", 1);
        let output = timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_lankide-computer-daemon"))
                .arg("pair")
                .arg(&http_url)
                .arg(&code.code)
                .arg("--home")
                .arg(computer_home.path())
                .env("HOME", computer_home.path())
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let file = computer_home.path().join("computer-credentials.json");
        assert_eq!(
            std::fs::metadata(&file)?.permissions().mode() & 0o777,
            0o600
        );
        let credentials = lankide_computer_daemon::read_credentials(computer_home.path())?.unwrap();
        assert_eq!(credentials.runtime_url, url);
        assert_ne!(credentials.computer_id, "local");
        let catalog = management
            .get_computer_catalog(CatalogRequest {
                request_id: "cli-catalog".into(),
                workspace: None,
            })
            .await?
            .into_inner();
        assert_eq!(catalog.computers.len(), 1);
        assert_eq!(catalog.computers[0].computer_id, credentials.computer_id);
        assert!(catalog.computers[0].registered);
        assert_eq!(
            catalog.computers[0].observed_state,
            lankide_protocol::ObservedState::Stopped as i32
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let _ = stop.send(());
    timeout(Duration::from_secs(5), server).await???;
    scenario
}
