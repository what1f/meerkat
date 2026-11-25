//! Built-in Agent sign-in. The Runtime runs the browser OAuth PKCE callback
//! listener in-process, exchanges the authorization code, hands the resulting
//! credentials to the Machine over the link, and keeps no copy. API-key
//! connections skip the browser step entirely.

use jimu_ai::CancelToken;
use jimu_ai::oauth::{exchange_codex_code, parse_authorization_input, start_codex_authorization};
use jimu_http::ReqwestTransport;
use meerkat_protocol::{
    BuiltinAgentSignInMethod, BuiltinAgentSignInRequest, BuiltinAgentSignInResponse,
    BuiltinAgentSignInState, BuiltinAgentSignInStatus, HarnessCredential, Status,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

use crate::MachineBroker;

const HARNESS_ID: &str = "builtin";
const CALLBACK_ADDR: &str = "127.0.0.1:1455";
const CALLBACK_PATH: &str = "/auth/callback";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST_LINE_BYTES: usize = 8192;
const MAX_HEADER_LINES: usize = 32;
const RETAIN_FINISHED: Duration = Duration::from_secs(900);

const SUCCESS_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"theme-color\" content=\"#000000\"><title>Meerkat</title><style>\
:root{color-scheme:dark;font-family:\"Geist Mono\",ui-monospace,SFMono-Regular,Menlo,monospace;\
font-size:15px;line-height:1.75}*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:flex;flex-direction:column;background:#000;color:#ededed}\
main{flex:1;display:flex;flex-direction:column;justify-content:center;padding:48px 32px;max-width:560px;margin:0 auto}\
h1{margin:0 0 12px;font-size:20px;font-weight:500}p{margin:0;color:#a1a1a1}\
</style></head><body><main><h1>Authorization complete</h1>\
<p>You can close this tab and return to Meerkat.</p></main></body></html>";

const FAILED_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"theme-color\" content=\"#000000\"><title>Meerkat</title><style>\
:root{color-scheme:dark;font-family:\"Geist Mono\",ui-monospace,SFMono-Regular,Menlo,monospace;\
font-size:15px;line-height:1.75}*{box-sizing:border-box}\
body{margin:0;min-height:100vh;display:flex;flex-direction:column;background:#000;color:#ededed}\
main{flex:1;display:flex;flex-direction:column;justify-content:center;padding:48px 32px;max-width:560px;margin:0 auto}\
h1{margin:0 0 12px;font-size:20px;font-weight:500}p{margin:0;color:#a1a1a1}\
</style></head><body><main><h1>Authorization failed</h1>\
<p>Return to Meerkat for details.</p></main></body></html>";

#[derive(Default)]
pub(crate) struct BuiltinSignIns {
    entries: Mutex<BTreeMap<String, Entry>>,
    // The ChatGPT login listens on a fixed local port, so logins run one at a time.
    login: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SignInPhase {
    Pending,
    Committing,
    Done,
}

struct Entry {
    phase: SignInPhase,
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
                self.insert(&sign_in_id, SignInPhase::Done, status, None);
                Ok(BuiltinAgentSignInResponse {
                    sign_in_id,
                    authorize_url: None,
                })
            }
            BuiltinAgentSignInMethod::Chatgpt => {
                self.cancel_pending();
                let guard = self.login.clone().lock_owned().await;
                let listener = TcpListener::bind(CALLBACK_ADDR)
                    .await
                    .map_err(|_| Status::unavailable("Sign-in could not start"))?;
                let auth = start_codex_authorization(None)
                    .map_err(|_| Status::internal("Sign-in could not start"))?;
                let authorize_url = auth.url.clone();

                let (cancel, cancelled) = oneshot::channel();
                self.insert(
                    &sign_in_id,
                    SignInPhase::Pending,
                    BuiltinAgentSignInStatus::default(),
                    Some(cancel),
                );
                let this = self.clone();
                let id = sign_in_id.clone();
                let machine_id = request.machine_id;
                tokio::spawn(async move {
                    let cancel_token = CancelToken::new();
                    let flow_cancel = cancel_token.clone();
                    let flow = async move {
                        let callback_target = match wait_for_callback(&listener, &auth.state).await
                        {
                            Some(path) => path,
                            None => return Outcome::Failed,
                        };
                        drop(listener);
                        let Ok(transport) = ReqwestTransport::new() else {
                            return Outcome::Failed;
                        };
                        let Ok(creds) =
                            exchange_codex_code(&transport, &auth, &callback_target, flow_cancel)
                                .await
                        else {
                            return Outcome::Failed;
                        };
                        let Ok(auth_json) = serde_json::to_string(&creds) else {
                            return Outcome::Failed;
                        };
                        Outcome::Ready(auth_json)
                    };

                    let outcome = tokio::select! {
                        res = flow => res,
                        _ = tokio::time::sleep(LOGIN_TIMEOUT) => {
                            cancel_token.cancel();
                            Outcome::Expired
                        }
                        _ = cancelled => {
                            cancel_token.cancel();
                            Outcome::Cancelled
                        }
                    };

                    match outcome {
                        Outcome::Ready(auth_json) => {
                            // Atomically transition Pending -> Committing under the state lock.
                            // If a concurrent cancel already won, skip writing to the Machine.
                            if this.begin_commit(&id) {
                                let status = match broker
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
                                };
                                this.complete_commit(&id, status);
                            }
                        }
                        Outcome::Failed => {
                            this.finish(
                                &id,
                                finished(BuiltinAgentSignInState::Failed, Some("sign_in_failed")),
                            );
                        }
                        Outcome::Expired => {
                            this.finish(&id, finished(BuiltinAgentSignInState::Expired, None));
                        }
                        Outcome::Cancelled => {
                            this.finish(
                                &id,
                                finished(BuiltinAgentSignInState::Failed, Some("cancelled")),
                            );
                        }
                    }
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
            && entry.phase == SignInPhase::Pending
        {
            if let Some(cancel) = entry.cancel.take() {
                let _ = cancel.send(());
            }
            entry.phase = SignInPhase::Done;
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
            if entry.phase == SignInPhase::Pending {
                if let Some(cancel) = entry.cancel.take() {
                    let _ = cancel.send(());
                }
                entry.phase = SignInPhase::Done;
                entry.status = finished(BuiltinAgentSignInState::Failed, Some("cancelled"));
                entry.finished = Some(Instant::now());
            }
        }
    }

    fn insert(
        &self,
        sign_in_id: &str,
        phase: SignInPhase,
        status: BuiltinAgentSignInStatus,
        cancel: Option<oneshot::Sender<()>>,
    ) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                sign_in_id.to_owned(),
                Entry {
                    phase,
                    status,
                    cancel,
                    finished: (phase == SignInPhase::Done).then(Instant::now),
                },
            );
        }
    }

    fn begin_commit(&self, sign_in_id: &str) -> bool {
        let Ok(mut entries) = self.entries.lock() else {
            return false;
        };
        let Some(entry) = entries.get_mut(sign_in_id) else {
            return false;
        };
        if entry.phase != SignInPhase::Pending {
            return false;
        }
        entry.phase = SignInPhase::Committing;
        entry.cancel = None;
        true
    }

    fn complete_commit(&self, sign_in_id: &str, status: BuiltinAgentSignInStatus) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.get_mut(sign_in_id)
            && entry.phase == SignInPhase::Committing
        {
            entry.phase = SignInPhase::Done;
            entry.status = status;
            entry.finished = Some(Instant::now());
        }
    }

    fn finish(&self, sign_in_id: &str, status: BuiltinAgentSignInStatus) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.get_mut(sign_in_id)
            && entry.phase == SignInPhase::Pending
        {
            entry.phase = SignInPhase::Done;
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
    Ready(String),
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

async fn wait_for_callback(listener: &TcpListener, expected_state: &str) -> Option<String> {
    loop {
        let (stream, _) = listener.accept().await.ok()?;
        match handle_callback_connection(stream, expected_state).await {
            CallbackAction::Complete(target) => return Some(target),
            CallbackAction::Failed => return None,
            CallbackAction::Ignore => continue,
        }
    }
}

enum CallbackAction {
    Complete(String),
    Failed,
    Ignore,
}

async fn read_bounded_http_request(stream: &mut TcpStream) -> Option<String> {
    tokio::time::timeout(CONNECTION_READ_TIMEOUT, async {
        let mut reader = BufReader::new(stream);
        let mut request_line = Vec::new();
        loop {
            let available = reader.fill_buf().await.ok()?;
            if available.is_empty() {
                return None;
            }
            if let Some(pos) = available.iter().position(|&b| b == b'\n') {
                let take = pos + 1;
                if request_line.len().saturating_add(take) > MAX_REQUEST_LINE_BYTES {
                    return None;
                }
                request_line.extend_from_slice(&available[..take]);
                reader.consume(take);
                break;
            }
            let len = available.len();
            if request_line.len().saturating_add(len) > MAX_REQUEST_LINE_BYTES {
                return None;
            }
            request_line.extend_from_slice(available);
            reader.consume(len);
        }

        for _ in 0..MAX_HEADER_LINES {
            let mut header = Vec::new();
            loop {
                let available = reader.fill_buf().await.ok()?;
                if available.is_empty() {
                    return String::from_utf8(request_line).ok();
                }
                if let Some(pos) = available.iter().position(|&b| b == b'\n') {
                    let take = pos + 1;
                    if header.len().saturating_add(take) <= MAX_REQUEST_LINE_BYTES {
                        header.extend_from_slice(&available[..take]);
                    }
                    reader.consume(take);
                    break;
                }
                let len = available.len();
                if header.len().saturating_add(len) > MAX_REQUEST_LINE_BYTES {
                    return None;
                }
                header.extend_from_slice(available);
                reader.consume(len);
            }
            if header.iter().all(|b| b.is_ascii_whitespace()) {
                break;
            }
        }
        String::from_utf8(request_line).ok()
    })
    .await
    .ok()
    .flatten()
}

async fn handle_callback_connection(mut stream: TcpStream, expected_state: &str) -> CallbackAction {
    let Some(request_line) = read_bounded_http_request(&mut stream).await else {
        return CallbackAction::Ignore;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        let _ = write_http_response(&mut stream, "404 Not Found", "text/plain", "Not found").await;
        return CallbackAction::Ignore;
    }
    let path = target.split('?').next().unwrap_or(target);
    if path != CALLBACK_PATH {
        let _ = write_http_response(&mut stream, "404 Not Found", "text/plain", "Not found").await;
        return CallbackAction::Ignore;
    }
    let parsed = parse_authorization_input(target);
    if parsed.state.as_deref() != Some(expected_state) {
        // Stale or unrelated browser tab: reply 400 and keep listening for the active sign-in's state.
        let _ = write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/html; charset=utf-8",
            FAILED_PAGE,
        )
        .await;
        return CallbackAction::Ignore;
    }
    if parsed.code.as_deref().is_some_and(|c| !c.is_empty()) {
        let _ = write_http_response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            SUCCESS_PAGE,
        )
        .await;
        CallbackAction::Complete(target.to_owned())
    } else {
        let _ = write_http_response(
            &mut stream,
            "400 Bad Request",
            "text/html; charset=utf-8",
            FAILED_PAGE,
        )
        .await;
        CallbackAction::Failed
    }
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline';\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}
