//! Pair and reconnect a real second Machine through the shipped command.
use meerkat_client::{ManagementClient, RuntimeManagementClient};
use meerkatd::{RuntimePaths, ShutdownReason, run_daemon};
use std::{os::unix::fs::PermissionsExt, path::Path, process::Stdio, time::Duration};
use tokio::{sync::oneshot, time::timeout};
type Error = Box<dyn std::error::Error + Send + Sync>;

struct Server {
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), meerkatd::AppError>>,
}
impl Server {
    async fn start(paths: &RuntimePaths) -> Result<Self, Error> {
        if !paths.home.join("runtime.json").exists() {
            std::fs::write(
                paths.home.join("runtime.json"),
                r#"{"machine_link_listen":"127.0.0.1:0","remote_access_port":0}"#,
            )?;
        }
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(run_daemon(
            paths.clone(),
            meerkat_runtime::ProviderFactoryRegistry::default(),
            async move {
                let _ = stopped.await;
                Ok(ShutdownReason::Internal)
            },
        ));
        timeout(Duration::from_secs(10), async {
            loop {
                if RuntimeManagementClient::connect_local(&paths.run)
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?;
        Ok(Self {
            stop: Some(stop),
            task,
        })
    }
    async fn stop(mut self) -> Result<(), Error> {
        let _ = self.stop.take().unwrap().send(());
        timeout(Duration::from_secs(10), &mut self.task).await???;
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
fn free_address() -> Result<String, Error> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.to_string())
}
fn command(
    home: &Path,
    url: Option<&str>,
    code: Option<&str>,
    pin: Option<&str>,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_meerkat-machine"));
    if let (Some(url), Some(code)) = (url, code) {
        command.args(["pair", url, code]);
    }
    if let Some(pin) = pin {
        command.args(["--cert-sha256", pin]);
    }
    command
        .arg("--home")
        .arg(home)
        .env("HOME", home)
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}
async fn rejected(url: &str, code: &str, pin: Option<&str>, expected: &str) -> Result<(), Error> {
    let home = tempfile::tempdir()?;
    let output = timeout(
        Duration::from_secs(15),
        command(home.path(), Some(url), Some(code), pin).output(),
    )
    .await??;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!home.path().join("machine-credentials.json").exists());
    let path = home.path().join("logs/meerkat-machine.log");
    let log = std::fs::read_to_string(&path)?;
    assert!(log.contains(expected), "{log}");
    assert!(log.lines().all(|line| {
        line.split_whitespace()
            .next()
            .unwrap()
            .trim_end_matches('Z')
            .parse::<f64>()
            .is_ok()
    }));
    assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
    Ok(())
}
async fn online(client: &RuntimeManagementClient, id: &str) -> Result<(), Error> {
    timeout(Duration::from_secs(20), async {
        loop {
            if client
                .get_machine_catalog(None)
                .await
                .is_ok_and(|catalog| catalog.iter().any(|c| c.id == id && c.available))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pinned_pair_command_reconnects_and_remote_address_changes_live() -> Result<(), Error> {
    let runtime_home = tempfile::tempdir()?;
    let second_home = tempfile::tempdir()?;
    std::fs::write(
        runtime_home.path().join("runtime.json"),
        r#"{"machine_link_listen":"127.0.0.1:0","remote_access_port":0,"pairing_validity_ms":1000}"#,
    )?;
    let paths = RuntimePaths::for_home(runtime_home.path())?;
    let server = Server::start(&paths).await?;
    let client = RuntimeManagementClient::connect_local(&paths.run).await?;
    assert!(client.get_remote_access().await?.address.is_none());
    let address = free_address()?;
    let access = client.set_remote_access(Some(address.clone())).await?;
    assert_eq!(access.address.as_deref(), Some(address.as_str()));
    for invalid in [
        "host",
        "host:0",
        "host:99999",
        "::1:7443",
        "host[]:7443",
        "host:7443;echo",
    ] {
        assert!(
            client
                .set_remote_access(Some(invalid.into()))
                .await
                .is_err()
        );
        assert_eq!(client.get_remote_access().await?, access);
    }
    let pin = access.cert_sha256.unwrap();
    let url = format!("wss://{address}");
    assert_eq!(
        std::fs::metadata(runtime_home.path().join("machine-link-identity.json"))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let identity = std::fs::read(runtime_home.path().join("machine-link-identity.json"))?;

    // Local discovery is still plaintext and works independently of remote access.
    let mut local = command(runtime_home.path(), None, None, None);
    local.env("MEERKAT_RUNTIME_RUN", &paths.run);
    let mut local = local.spawn()?;
    let local_id = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(credentials) =
                meerkat_machine::read_credentials(runtime_home.path()).unwrap()
            {
                break credentials.machine_id;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    online(&client, &local_id).await?;
    let code = client.create_pairing_code().await?;
    assert_eq!(code.code.len(), 43); // All 256 random bits, encoded without shell metacharacters.
    rejected(&url, &code.code, Some(&"00".repeat(32)), "pin mismatch").await?;
    rejected(&url, &code.code, None, "Could not reach").await?;
    rejected("ws://192.0.2.1:7443", &code.code, None, "secure address").await?;
    let code = client.create_pairing_code().await?;
    let mut second =
        command(second_home.path(), Some(&url), Some(&code.code), Some(&pin)).spawn()?;
    let credentials = timeout(Duration::from_secs(10), async {
        loop {
            if let Some(credentials) =
                meerkat_machine::read_credentials(second_home.path()).unwrap()
            {
                break credentials;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(credentials.cert_sha256.as_deref(), Some(pin.as_str()));
    assert_eq!(credentials.runtime_url, url);
    assert_eq!(
        std::fs::metadata(second_home.path().join("machine-credentials.json"))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    online(&client, &credentials.machine_id).await?;
    assert!(second.try_wait()?.is_none(), "pair must keep running");
    use tokio::io::AsyncBufReadExt as _;
    let mut output = tokio::io::BufReader::new(second.stdout.take().expect("pair stdout"));
    let mut line = String::new();
    timeout(Duration::from_secs(5), output.read_line(&mut line)).await??;
    assert_eq!(
        line.trim(),
        format!("Connected as {}", meerkat_machine::machine_name())
    );
    let catalog = client.get_machine_catalog(None).await?;
    assert_eq!(catalog.len(), 2);
    assert_eq!(
        catalog
            .iter()
            .find(|c| c.id == credentials.machine_id)
            .unwrap()
            .name,
        meerkat_machine::machine_name()
    );
    rejected(&url, &code.code, Some(&pin), "rejected or expired").await?;
    let expired = client.create_pairing_code().await?;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    rejected(&url, &expired.code, Some(&pin), "rejected or expired").await?;
    // The TLS entry point exposes neither management nor resource routes.
    let http = reqwest::Client::builder()
        .no_proxy()
        .use_preconfigured_tls(meerkat_protocol::tls::client_config(&pin)?)
        .build()?;
    for route in ["/api/v1/list_workspaces", "/resources/1"] {
        assert_eq!(
            http.post(format!("https://{address}{route}"))
                .send()
                .await?
                .status(),
            404
        );
    }
    server.stop().await?;
    let server = Server::start(&paths).await?;
    assert_eq!(
        client.get_remote_access().await?.cert_sha256.as_deref(),
        Some(pin.as_str())
    );
    assert_eq!(
        std::fs::read(runtime_home.path().join("machine-link-identity.json"))?,
        identity
    );
    online(&client, &credentials.machine_id)
        .await
        .map_err(|e| {
            format!(
                "remote reconnect after restart: {e}; process={:?}",
                second.try_wait()
            )
        })?;
    if local.try_wait()?.is_some() {
        local = command(runtime_home.path(), None, None, None).spawn()?;
    }
    online(&client, &local_id).await?;
    // Restart the second process too: its stored URL and pin are sufficient.
    second.kill().await?;
    let _ = second.wait().await?;
    timeout(Duration::from_secs(5), async {
        while client
            .get_machine_catalog(None)
            .await
            .unwrap()
            .iter()
            .any(|c| c.id == credentials.machine_id && c.available)
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    let credentials_path = second_home.path().join("machine-credentials.json");
    let original = std::fs::read(&credentials_path)?;
    let mut wrong = credentials.clone();
    wrong.cert_sha256 = Some("00".repeat(32));
    std::fs::write(&credentials_path, serde_json::to_vec(&wrong)?)?;
    let mut rejected_reconnect = command(second_home.path(), None, None, None).spawn()?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !client
            .get_machine_catalog(None)
            .await?
            .iter()
            .any(|c| c.id == credentials.machine_id && c.available)
    );
    rejected_reconnect.kill().await?;
    let _ = rejected_reconnect.wait().await?;
    std::fs::write(&credentials_path, original)?;
    let mut second = command(second_home.path(), None, None, None).spawn()?;
    online(&client, &credentials.machine_id).await?;
    let next_address = free_address()?;
    let next = client.set_remote_access(Some(next_address.clone())).await?;
    assert_eq!(next.cert_sha256.as_deref(), Some(pin.as_str()));
    assert!(tokio::net::TcpStream::connect(&address).await.is_err());
    online(&client, &local_id).await?;
    let next_port = next_address.parse::<std::net::SocketAddr>()?.port();
    let next_url = format!("wss://[::1]:{next_port}");
    let new_code = client.create_pairing_code().await?;
    let third_home = tempfile::tempdir()?;
    let _ = meerkat_machine::pair_with_pin(
        &meerkat_machine::MachineDaemonPaths::for_home(third_home.path())?,
        &next_url,
        &new_code.code,
        Some(&pin),
    )
    .await?;
    client.set_remote_access(None).await?;
    assert!(client.get_remote_access().await?.address.is_none());
    assert!(tokio::net::TcpStream::connect(&next_address).await.is_err());
    online(&client, &local_id).await?;
    second.kill().await?;
    let _ = second.wait().await?;
    local.kill().await?;
    let _ = local.wait().await?;
    // Registered devices remain in the catalog after disconnecting.
    assert!(
        client
            .get_machine_catalog(None)
            .await?
            .iter()
            .any(|c| c.id == credentials.machine_id && c.registered)
    );
    server.stop().await?;
    Ok(())
}
