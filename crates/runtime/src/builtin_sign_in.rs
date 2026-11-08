//! Built-in Agent sign-in. The Runtime runs the browser step in a throwaway
//! home, hands the resulting credentials to the Machine over the link and
//! keeps no copy. API-key connections skip the browser step entirely.

use meerkat_protocol::{
    BuiltinAgentSignInMethod, BuiltinAgentSignInRequest, BuiltinAgentSignInResponse,
    BuiltinAgentSignInState, BuiltinAgentSignInStatus, HarnessCredential, Status,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::oneshot,
};

use crate::MachineBroker;

const HARNESS_ID: &str = "builtin";
const EXECUTABLE: &str = "meerkat-agent";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const URL_TIMEOUT: Duration = Duration::from_secs(30);
const RETAIN_FINISHED: Duration = Duration::from_secs(900);

#[derive(Default)]
pub(crate) struct BuiltinSignIns {
    entries: Mutex<BTreeMap<String, Entry>>,
    // The ChatGPT login listens on a fixed local port, so logins run one at a time.
    login: Arc<tokio::sync::Mutex<()>>,
}

struct Entry {
    status: BuiltinAgentSignInStatus,
    cancel: Option<oneshot::Sender<()>>,
    finished: Option<Instant>,
}

impl BuiltinSignIns {
    pub(crate) async fn sign_in(
        self: &Arc<Self>,
        broker: MachineBroker,
        request: BuiltinAgentSignInRequest,
    ) -> Result<BuiltinAgentSignInResponse, Status> {
        self.prune();
        let sign_in_id = format!("signin_{}", uuid::Uuid::new_v4().simple());
        match request.method {
            BuiltinAgentSignInMethod::ApiKey => {
                let credential = HarnessCredential::ApiKey {
                    base_url: request.base_url.unwrap_or_default(),
                    api_key: request.api_key.unwrap_or_default(),
                    model: request.model.filter(|model| !model.trim().is_empty()),
                };
                let status = match broker
                    .set_harness_credential(&request.machine_id, HARNESS_ID, Some(credential))
                    .await
                {
                    Ok(()) => finished(BuiltinAgentSignInState::Succeeded, None),
                    Err(reason) => finished(BuiltinAgentSignInState::Failed, Some(reason)),
                };
                self.insert(&sign_in_id, status, None, true);
                Ok(BuiltinAgentSignInResponse {
                    sign_in_id,
                    authorize_url: None,
                })
            }
            BuiltinAgentSignInMethod::Chatgpt => {
                let executable = discover()
                    .ok_or_else(|| Status::unavailable("Built-in Agent is not installed"))?;
                self.cancel_pending();
                let guard = self.login.clone().lock_owned().await;
                let home =
                    TempHome::create().map_err(|_| Status::internal("Sign-in could not start"))?;
                let mut child = Command::new(executable)
                    .args(["login", "codex"])
                    .env("FX_HOME", home.path())
                    .env("FX_AUTO_UPGRADE", "0")
                    .env("FX_DISABLE_KEYCHAIN", "1")
                    .env("FX_NO_OPEN_BROWSER", "1")
                    .env("FX_HERDR", "0")
                    .current_dir(home.path())
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|_| Status::unavailable("Sign-in could not start"))?;
                let stdout = child.stdout.take().expect("piped stdout");
                if let Some(mut stderr) = child.stderr.take() {
                    tokio::spawn(async move {
                        let mut sink = Vec::new();
                        let _ = stderr.read_to_end(&mut sink).await;
                    });
                }
                let mut lines = BufReader::new(stdout).lines();
                let authorize_url = tokio::time::timeout(URL_TIMEOUT, async {
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Some(url) = authorize_url(&line) {
                            return Some(url);
                        }
                    }
                    None
                })
                .await
                .ok()
                .flatten()
                .ok_or_else(|| Status::unavailable("Sign-in could not start"))?;

                let (cancel, cancelled) = oneshot::channel();
                self.insert(
                    &sign_in_id,
                    BuiltinAgentSignInStatus::default(),
                    Some(cancel),
                    false,
                );
                let this = self.clone();
                let id = sign_in_id.clone();
                let machine_id = request.machine_id;
                tokio::spawn(async move {
                    // Drain the rest of the output so the process never blocks on a full pipe.
                    let drain = async move { while let Ok(Some(_)) = lines.next_line().await {} };
                    tokio::spawn(drain);
                    let outcome = tokio::select! {
                        status = child.wait() => match status {
                            Ok(status) if status.success() => Outcome::Exited,
                            _ => Outcome::Failed,
                        },
                        _ = tokio::time::sleep(LOGIN_TIMEOUT) => Outcome::Expired,
                        _ = cancelled => Outcome::Cancelled,
                    };
                    let _ = child.kill().await;
                    let status = match outcome {
                        Outcome::Exited => match read_auth(home.path()) {
                            Some(auth_json) => match broker
                                .set_harness_credential(
                                    &machine_id,
                                    HARNESS_ID,
                                    Some(HarnessCredential::Chatgpt { auth_json }),
                                )
                                .await
                            {
                                Ok(()) => finished(BuiltinAgentSignInState::Succeeded, None),
                                Err(reason) => {
                                    finished(BuiltinAgentSignInState::Failed, Some(reason))
                                }
                            },
                            None => {
                                finished(BuiltinAgentSignInState::Failed, Some("sign_in_failed"))
                            }
                        },
                        Outcome::Failed => {
                            finished(BuiltinAgentSignInState::Failed, Some("sign_in_failed"))
                        }
                        Outcome::Expired => finished(BuiltinAgentSignInState::Expired, None),
                        Outcome::Cancelled => {
                            finished(BuiltinAgentSignInState::Failed, Some("cancelled"))
                        }
                    };
                    drop(home);
                    this.finish(&id, status);
                    drop(guard);
                });
                Ok(BuiltinAgentSignInResponse {
                    sign_in_id,
                    authorize_url: Some(authorize_url),
                })
            }
        }
    }

    pub(crate) fn status(&self, sign_in_id: &str) -> BuiltinAgentSignInStatus {
        self.prune();
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(sign_in_id).map(|entry| entry.status.clone()))
            .unwrap_or_else(|| finished(BuiltinAgentSignInState::Expired, None))
    }

    pub(crate) fn cancel(&self, sign_in_id: &str) -> BuiltinAgentSignInStatus {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.get_mut(sign_in_id)
            && entry.status.state == BuiltinAgentSignInState::Pending
        {
            if let Some(cancel) = entry.cancel.take() {
                let _ = cancel.send(());
            }
            entry.status = finished(BuiltinAgentSignInState::Failed, Some("cancelled"));
            entry.finished = Some(Instant::now());
        }
        self.status(sign_in_id)
    }

    fn cancel_pending(&self) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        for entry in entries.values_mut() {
            if entry.status.state == BuiltinAgentSignInState::Pending {
                if let Some(cancel) = entry.cancel.take() {
                    let _ = cancel.send(());
                }
                entry.status = finished(BuiltinAgentSignInState::Failed, Some("cancelled"));
                entry.finished = Some(Instant::now());
            }
        }
    }

    fn insert(
        &self,
        sign_in_id: &str,
        status: BuiltinAgentSignInStatus,
        cancel: Option<oneshot::Sender<()>>,
        done: bool,
    ) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                sign_in_id.to_owned(),
                Entry {
                    status,
                    cancel,
                    finished: done.then(Instant::now),
                },
            );
        }
    }

    fn finish(&self, sign_in_id: &str, status: BuiltinAgentSignInStatus) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.get_mut(sign_in_id)
            && entry.status.state == BuiltinAgentSignInState::Pending
        {
            entry.status = status;
            entry.cancel = None;
            entry.finished = Some(Instant::now());
        }
    }

    fn prune(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|_, entry| {
                entry
                    .finished
                    .is_none_or(|finished| finished.elapsed() < RETAIN_FINISHED)
            });
        }
    }
}

enum Outcome {
    Exited,
    Failed,
    Expired,
    Cancelled,
}

fn finished(state: BuiltinAgentSignInState, reason: Option<&str>) -> BuiltinAgentSignInStatus {
    BuiltinAgentSignInStatus {
        state,
        reason: reason.map(str::to_owned),
    }
}

fn authorize_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url = line[start..].split_whitespace().next()?;
    url.contains("/oauth/authorize").then(|| url.to_owned())
}

fn read_auth(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join("chatgpt-auth.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())?;
    Some(text)
}

/// Same lookup order as the Machine: next to this executable, then the app
/// bundle's `Resources/bin`, then PATH.
pub(crate) fn discover() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let mut candidates = vec![executable.with_file_name(EXECUTABLE)];
    if let Some(contents) = executable
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "Contents"))
    {
        candidates.push(contents.join("Resources/bin").join(EXECUTABLE));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join(EXECUTABLE)));
    }
    candidates.into_iter().find(|path| executable_file(path))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Private temporary home for one login; removed (with any credentials in it)
/// when dropped.
struct TempHome(PathBuf);

impl TempHome {
    fn create() -> std::io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("meerkat-sign-in-{}", uuid::Uuid::new_v4().simple()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
