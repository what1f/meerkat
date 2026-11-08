//! Built-in Agent sign-in through management HTTP and a real Machine link,
//! with an API-key connection checked against a local endpoint.
use meerkat_system_e2e::{Result, RigConfig, SystemRig};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

const KEY: &str = "sk-system-secret";

/// `GET /v1/models`: the list for the right key, 401 otherwise.
fn endpoint() -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr()?.port());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut authorized = false;
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("authorization:") {
                    authorized = line.trim_end().ends_with(&format!("Bearer {KEY}"));
                }
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let _ = reader.read_exact(&mut vec![0; length]);
            let (status, body) = if authorized {
                (
                    "200 OK",
                    r#"{"data":[{"id":"first-model","name":"First"},{"id":"second-model"}]}"#,
                )
            } else {
                ("401 Unauthorized", r#"{"error":{"message":"bad key"}}"#)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Ok(url)
}

struct Http {
    client: reqwest::Client,
    url: String,
}
impl Http {
    async fn call(&self, op: &str, body: Value) -> Result<Value> {
        let response = self
            .client
            .post(format!("{}/api/v1/{op}", self.url))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        assert!(status.is_success(), "{op} {status}: {text}");
        Ok(serde_json::from_str(&text)?)
    }
    async fn builtin(&self, machine: &str) -> Result<Value> {
        let machines = self
            .call(
                "get_machine_catalog",
                json!({"request_id":"machines","workspace":null}),
            )
            .await?;
        machines["machines"]
            .as_array()
            .and_then(|machines| machines.iter().find(|m| m["machine_id"] == machine))
            .and_then(|m| m["harnesses"].as_array())
            .and_then(|harnesses| harnesses.iter().find(|h| h["harness_id"] == "builtin"))
            .cloned()
            .ok_or_else(|| "builtin harness missing from inventory".into())
    }
    async fn status(&self, sign_in_id: &Value) -> Result<Value> {
        self.call(
            "builtin_agent_sign_in_status",
            json!({"sign_in_id": sign_in_id}),
        )
        .await
    }
}

#[tokio::test]
async fn builtin_agent_api_key_sign_in_and_sign_out() -> Result<()> {
    let rig = SystemRig::start(RigConfig::default()).await?;
    let scenario = async {
        let _team = rig.workspace("team").await?;
        let remote = rig.add_machine_with_builtin_agent().await?;
        let machine = remote.id.as_str();
        let http = Http {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(35))
                .build()?,
            url: std::fs::read_to_string(rig.runtime_home().join("run/management.url"))?,
        };
        let base_url = endpoint()?;
        let home = meerkat_machine::MachineDaemonPaths::for_home(remote.home.path())?
            .workspaces
            .join(".machine/builtin-agent");

        let initial = http.builtin(machine).await?;
        assert_eq!(initial["display_name"], "Meerkat");
        assert_eq!(initial["account"], Value::Null);

        // A rejected key fails with a stable reason and saves nothing.
        let wrong = http
            .call(
                "builtin_agent_sign_in",
                json!({"workspace":null,"machine_id":machine,"method":"api_key",
                    "base_url":base_url,"api_key":"sk-wrong"}),
            )
            .await?;
        assert_eq!(wrong["authorize_url"], Value::Null);
        let status = http.status(&wrong["sign_in_id"]).await?;
        assert_eq!(status, json!({"state":"failed","reason":"invalid_key"}));
        assert_eq!(http.builtin(machine).await?["account"], Value::Null);

        // An unreachable endpoint has its own reason.
        let closed = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let offline = http
            .call(
                "builtin_agent_sign_in",
                json!({"workspace":null,"machine_id":machine,"method":"api_key",
                    "base_url":format!("http://127.0.0.1:{closed}/v1"),"api_key":KEY}),
            )
            .await?;
        let status = http.status(&offline["sign_in_id"]).await?;
        assert_eq!(status, json!({"state":"failed","reason":"unreachable"}));

        // A good key signs in; the first listed model becomes the default.
        let signed = http
            .call(
                "builtin_agent_sign_in",
                json!({"workspace":null,"machine_id":machine,"method":"api_key",
                    "base_url":base_url,"api_key":KEY}),
            )
            .await?;
        assert_eq!(signed["authorize_url"], Value::Null);
        let status = http.status(&signed["sign_in_id"]).await?;
        assert_eq!(status, json!({"state":"succeeded","reason":null}));
        let builtin = http.builtin(machine).await?;
        assert_eq!(
            builtin["account"],
            json!({"method":"api_key","label":"127.0.0.1"})
        );
        let settings = std::fs::read_to_string(home.join("settings.json"))?;
        assert!(!settings.contains(KEY), "the key stays out of settings");
        let settings: Value = serde_json::from_str(&settings)?;
        assert_eq!(settings["provider"], "custom");
        assert_eq!(settings["models"]["custom"], "first-model");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(home.join("api-key"))?.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let runtime_files = walk(rig.runtime_home())?;
        assert!(
            !runtime_files.iter().any(|text| text.contains(KEY)),
            "the Runtime keeps no copy of the key"
        );

        // The picker reads the connection's own model list.
        let models = http
            .call(
                "get_model_catalog",
                json!({"request_id":"models","workspace":null,"machine_id":machine,"harness_id":"builtin"}),
            )
            .await?;
        let text = models.to_string();
        assert!(text.contains("first-model") && text.contains("second-model"), "{text}");

        // Sign-out removes the credentials.
        http.call(
            "builtin_agent_sign_out",
            json!({"workspace":null,"machine_id":machine}),
        )
        .await?;
        assert_eq!(http.builtin(machine).await?["account"], Value::Null);
        assert!(!home.join("api-key").exists());

        // Unknown sign-ins read as expired.
        let unknown = json!({"sign_in_id":"signin_unknown"});
        assert_eq!(
            http.call("builtin_agent_sign_in_status", unknown.clone()).await?["state"],
            "expired"
        );
        assert_eq!(
            http.call("builtin_agent_sign_in_cancel", unknown).await?["state"],
            "expired"
        );
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    rig.close().await?;
    scenario
}

/// Text of every readable file under `root`.
fn walk(root: &std::path::Path) -> Result<Vec<String>> {
    let mut texts = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else if metadata.is_file() {
            let bytes = std::fs::read(&path).unwrap_or_default();
            texts.push(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    Ok(texts)
}
