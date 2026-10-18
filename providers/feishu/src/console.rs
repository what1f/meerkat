//! Feishu developer console, driven with the signed-in user's web session.
//!
//! Feishu has no public API that creates an App. The console's own web API
//! does, so a user who scans one sign-in QR code lets Lankide create, configure
//! and publish Agent Apps without further
//! confirmations. Every call here is the same request the console page makes.

use crate::authorization::{STANDARD_BOT_EVENTS, STANDARD_BOT_TENANT_SCOPES};
use crate::http::{ClientError, Method, Request, Response, Transport, encode_query_component};
use lankide_provider::ProviderError;
use serde_json::{Value, json};
use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const ACCOUNTS: &str = "https://accounts.feishu.cn";
const OPEN: &str = "https://open.feishu.cn";
const CONSOLE_HOME: &str = "https://open.feishu.cn/app";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36";
const DEVICE_INFO: &str = "device_id=0;device_name=Chrome;device_os=Mac;device_model=Chrome;lark_version=;channel=Release;package_name=feishu;tt_app_id=1658;is_dpop_support=false;is_iframe=false";
const MAX_HOPS: usize = 10;
/// A sign-in QR code stays valid for a few minutes; a fresh one replaces it
/// before the user can scan an expired code.
pub(crate) const LOGIN_QR_LIFETIME_MS: u64 = 150_000;
pub(crate) const LOGIN_POLL_INTERVAL_SECONDS: u32 = 2;
const APP_ICON: &[u8] = include_bytes!("../assets/app-icon.png");

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct CookieJar {
    cookies: Vec<Cookie>,
}

#[derive(Clone, PartialEq, Eq)]
struct Cookie {
    domain: String,
    name: String,
    value: String,
}

impl fmt::Debug for CookieJar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CookieJar")
            .field("cookies", &self.cookies.len())
            .finish()
    }
}

impl CookieJar {
    fn store(&mut self, url: &str, response: &Response) {
        let host = host_of(url);
        for header in response.header_values("set-cookie") {
            let mut parts = header.split(';');
            let Some((name, value)) = parts.next().and_then(|pair| pair.split_once('=')) else {
                continue;
            };
            let (name, value) = (name.trim().to_owned(), value.trim().to_owned());
            let mut domain = host.clone();
            let mut removed = value.is_empty();
            for attribute in parts {
                let Some((key, attribute_value)) = attribute.split_once('=') else {
                    continue;
                };
                let attribute_value = attribute_value.trim();
                if key.trim().eq_ignore_ascii_case("domain") {
                    domain = attribute_value.trim_start_matches('.').to_ascii_lowercase();
                } else if key.trim().eq_ignore_ascii_case("max-age") {
                    removed |= attribute_value.parse::<i64>().is_ok_and(|age| age <= 0);
                }
            }
            self.cookies
                .retain(|cookie| !(cookie.domain == domain && cookie.name == name));
            if !removed {
                self.cookies.push(Cookie {
                    domain,
                    name,
                    value,
                });
            }
        }
    }

    fn header_for(&self, url: &str) -> String {
        let host = host_of(url);
        self.cookies
            .iter()
            .filter(|cookie| {
                host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain))
            })
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|cookie| cookie.name == name)
            .map(|cookie| cookie.value.as_str())
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

/// Sends `request` with the jar's cookies and follows redirects one hop at a
/// time, because sign-in sets cookies on intermediate responses.
async fn fetch(
    transport: &dyn Transport,
    jar: &mut CookieJar,
    mut request: Request,
    operation: &str,
) -> Result<(Response, String), ClientError> {
    for _ in 0..MAX_HOPS {
        let url = request.path.clone();
        let cookie = jar.header_for(&url);
        request = request.without_redirects();
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("cookie"));
        if !cookie.is_empty() {
            request.headers.push(("Cookie".into(), cookie));
        }
        let response = transport.execute(request.clone()).await?;
        jar.store(&url, &response);
        let location = response
            .header("location")
            .filter(|_| (300..400).contains(&response.status))
            .and_then(|location| url::Url::parse(&url).ok()?.join(location).ok());
        let Some(next) = location else {
            return Ok((response, url));
        };
        request = Request::new(Method::Get, next.to_string()).header("User-Agent", USER_AGENT);
    }
    Err(ClientError::HttpStatus {
        status: 310,
        operation: operation.to_owned(),
    })
}

/// Sign-in that has shown a QR code and waits for the user's scan.
#[derive(Clone, Default)]
pub(crate) struct PendingLogin {
    jar: CookieJar,
    flow_key: String,
    pub(crate) qr_code: String,
    pub(crate) started_at_ms: u64,
}

impl fmt::Debug for PendingLogin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingLogin")
            .field("started_at_ms", &self.started_at_ms)
            .finish_non_exhaustive()
    }
}

impl PendingLogin {
    pub(crate) fn expired(&self) -> bool {
        self.qr_code.is_empty()
            || now_ms().saturating_sub(self.started_at_ms) >= LOGIN_QR_LIFETIME_MS
    }
}

#[derive(Debug)]
pub(crate) enum LoginPoll {
    Waiting,
    Scanned,
    Complete(ConsoleSession),
    Expired,
}

/// Signed-in developer console session.
#[derive(Clone, Default)]
pub(crate) struct ConsoleSession {
    jar: CookieJar,
    csrf: String,
}

impl fmt::Debug for ConsoleSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleSession")
            .field("jar", &self.jar)
            .finish_non_exhaustive()
    }
}

fn accounts_request(method: Method, path: &str) -> Request {
    Request::new(method, format!("{ACCOUNTS}{path}"))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json, text/plain, */*")
        .header("Referer", format!("{ACCOUNTS}/"))
        .header("X-Terminal-Type", "2")
        .header("X-Api-Version", "1.0.31")
        .header("X-App-Id", "7")
        .header("x-lgw-terminal-type", "2")
        .header("x-lsc-terminal", "web")
        .header("x-lgw-os-type", "3")
        .header("X-Device-Info", DEVICE_INFO)
        .header("X-Request-ID", uuid::Uuid::new_v4().to_string())
}

pub(crate) fn passport_data(response: &Response, operation: &str) -> Result<Value, ClientError> {
    crate::http::validate_success(response, operation)?;
    let value: Value = response.json()?;
    let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        return Err(ClientError::Api {
            code,
            message: value
                .get("message")
                .or_else(|| value.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            operation: operation.to_owned(),
        });
    }
    Ok(value.get("data").cloned().unwrap_or(Value::Null))
}

/// Starts a sign-in whose QR code content the user scans with Feishu.
pub(crate) async fn start_login(transport: &dyn Transport) -> Result<PendingLogin, ClientError> {
    let mut jar = CookieJar::default();
    let login_page = format!(
        "/accounts/page/login?app_id=7&redirect_uri={}",
        encode_query_component(CONSOLE_HOME)
    );
    fetch(
        transport,
        &mut jar,
        accounts_request(Method::Get, &login_page),
        "console_login_page",
    )
    .await?;
    let request = accounts_request(Method::Post, "/accounts/qrlogin/init")
        .json(&json!({"biz_type": null, "redirect_uri": CONSOLE_HOME}))?;
    let (response, _) = fetch(transport, &mut jar, request, "console_login_init").await?;
    let flow_key = response.header("x-flow-key").unwrap_or_default().to_owned();
    let data = passport_data(&response, "console_login_init")?;
    let token = data
        .pointer("/step_info/token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ClientError::MissingField {
            operation: "console_login_init".into(),
            field: "token".into(),
        })?;
    Ok(PendingLogin {
        jar,
        flow_key,
        qr_code: json!({"qrlogin": {"token": token}}).to_string(),
        started_at_ms: now_ms(),
    })
}

pub(crate) async fn poll_login(
    transport: &dyn Transport,
    pending: &mut PendingLogin,
) -> Result<LoginPoll, ClientError> {
    let mut request = accounts_request(Method::Post, "/accounts/qrlogin/polling")
        .json(&json!({"biz_type": null}))?;
    if !pending.flow_key.is_empty() {
        request = request.header("X-Flow-Key", pending.flow_key.clone());
    }
    let (response, _) = fetch(transport, &mut pending.jar, request, "console_login_poll").await?;
    let data = match passport_data(&response, "console_login_poll") {
        Ok(data) => data,
        Err(error) if error.is_retryable() => return Err(error),
        Err(_) => return Ok(LoginPoll::Expired),
    };
    match data.get("next_step").and_then(Value::as_str) {
        Some("qr_login_polling") => {
            match data.pointer("/step_info/status").and_then(Value::as_i64) {
                Some(0 | 1) => Ok(LoginPoll::Waiting),
                Some(2) => Ok(LoginPoll::Scanned),
                _ => Ok(LoginPoll::Expired),
            }
        }
        Some(step) if !step.is_empty() => {
            let mut jar = std::mem::take(&mut pending.jar);
            let session = open_console(transport, &mut jar).await?;
            Ok(LoginPoll::Complete(session))
        }
        _ => Ok(LoginPoll::Expired),
    }
}

/// Visits the console home with the sign-in cookies to obtain its CSRF token.
async fn open_console(
    transport: &dyn Transport,
    jar: &mut CookieJar,
) -> Result<ConsoleSession, ClientError> {
    let (response, _) = fetch(
        transport,
        jar,
        Request::new(Method::Get, CONSOLE_HOME).header("User-Agent", USER_AGENT),
        "console_home",
    )
    .await?;
    crate::http::validate_success(&response, "console_home")?;
    let html = String::from_utf8_lossy(&response.body);
    let csrf = embedded_csrf(&html)
        .or_else(|| jar.value("lark_oapi_csrf_token").map(str::to_owned))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ClientError::MissingField {
            operation: "console_home".into(),
            field: "csrf_token".into(),
        })?;
    Ok(ConsoleSession {
        jar: std::mem::take(jar),
        csrf,
    })
}

fn embedded_csrf(html: &str) -> Option<String> {
    let start = html.find("csrfToken")? + "csrfToken".len();
    let rest =
        html[start..].trim_start_matches(|c: char| c.is_whitespace() || c == '=' || c == ':');
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &rest[1..];
    Some(value[..value.find(quote)?].to_owned())
}

/// App created through the console, with its secret.
pub(crate) struct CreatedApp {
    pub(crate) app_id: String,
    app_secret: String,
}

impl fmt::Debug for CreatedApp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreatedApp")
            .field("app_id", &self.app_id)
            .finish_non_exhaustive()
    }
}

impl CreatedApp {
    pub(crate) fn app_secret(&self) -> &str {
        &self.app_secret
    }
}

/// Developer console operations through the saved web session.
pub(crate) struct DeveloperConsole {
    transport: Arc<dyn Transport>,
    session: ConsoleSession,
}

impl DeveloperConsole {
    pub(crate) fn new(transport: Arc<dyn Transport>, session: ConsoleSession) -> Self {
        Self { transport, session }
    }

    async fn call(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        operation: &str,
    ) -> Result<Value, ClientError> {
        let mut request = Request::new(method, format!("{OPEN}/developers/v1/{path}"))
            .header("User-Agent", USER_AGENT)
            .header("Accept", "*/*")
            .header("Referer", CONSOLE_HOME)
            .header("x-timezone-offset", "-480")
            .header("x-csrf-token", self.session.csrf.clone());
        if let Some(body) = body {
            request = request.json(&body)?;
        }
        self.send(request, operation).await
    }

    async fn send(&mut self, request: Request, operation: &str) -> Result<Value, ClientError> {
        let (response, final_url) = fetch(
            self.transport.as_ref(),
            &mut self.session.jar,
            request,
            operation,
        )
        .await?;
        if final_url.contains("accounts.feishu.cn") || (300..400).contains(&response.status) {
            // Console calls redirect to sign-in once the session has ended.
            return Err(ClientError::HttpStatus {
                status: 401,
                operation: operation.to_owned(),
            });
        }
        passport_data(&response, operation)
    }

    pub(crate) async fn tenant_id(&mut self) -> Result<String, ClientError> {
        self.send(
            Request::new(Method::Get, format!("{OPEN}/napi/check/login"))
                .header("User-Agent", USER_AGENT),
            "console_tenant",
        )
        .await?["tenantId"]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ClientError::MissingField {
                operation: "console_tenant".into(),
                field: "tenantId".into(),
            })
    }

    /// Creates an Agent App with the bot, permissions, events and icon Lankide
    /// needs, and submits its first version for release.
    pub(crate) async fn create_agent_app(&mut self, name: &str) -> Result<CreatedApp, ClientError> {
        let description = "Lankide Agent";
        let i18n = json!({"zh_cn": {"name": name, "description": description}});
        let created = self
            .call(
                Method::Post,
                "app/create",
                Some(json!({
                    "appSceneType": 0,
                    "name": name,
                    "desc": description,
                    "avatar": "",
                    "i18n": i18n,
                    "primaryLang": "zh_cn",
                })),
                "console_app_create",
            )
            .await?;
        let app_id = text_field(&created, "ClientID", "console_app_create")?;
        match self.configure_app(&app_id, name, description, &i18n).await {
            Ok(app_secret) => Ok(CreatedApp { app_id, app_secret }),
            Err(error) => {
                // A half-configured App would linger in the user's console.
                let _ = self
                    .call(
                        Method::Post,
                        &format!("app/delete/{app_id}"),
                        Some(json!({})),
                        "console_app_delete",
                    )
                    .await;
                Err(error)
            }
        }
    }

    async fn configure_app(
        &mut self,
        app_id: &str,
        name: &str,
        description: &str,
        i18n: &Value,
    ) -> Result<String, ClientError> {
        let secret = self
            .call(
                Method::Get,
                &format!("secret/{app_id}"),
                None,
                "console_app_secret",
            )
            .await?;
        let app_secret = text_field(&secret, "secret", "console_app_secret")?;
        self.call(
            Method::Post,
            &format!("robot/switch/{app_id}"),
            Some(json!({"enable": true})),
            "console_bot_enable",
        )
        .await?;
        self.call(
            Method::Post,
            &format!("event/switch/{app_id}"),
            Some(json!({"eventMode": 4})),
            "console_event_mode",
        )
        .await?;
        self.call(
            Method::Post,
            &format!("event/update/{app_id}"),
            Some(json!({"operation": "add", "events": STANDARD_BOT_EVENTS, "eventMode": 4})),
            "console_events",
        )
        .await?;
        self.configure_authorization(app_id).await?;
        let avatar = self.upload_icon().await?;
        self.call(
            Method::Post,
            &format!("base_info/{app_id}"),
            Some(json!({
                "clientId": app_id,
                "name": name,
                "desc": description,
                "avatar": avatar,
                "languages": ["zh_cn"],
                "i18n": i18n,
            })),
            "console_base_info",
        )
        .await?;
        self.publish_authorization(app_id, "1.0.0").await?;
        Ok(app_secret)
    }

    async fn configure_authorization(&mut self, app_id: &str) -> Result<(), ClientError> {
        let catalog = self
            .call(
                Method::Get,
                &format!("scope/all/{app_id}"),
                None,
                "console_scope_catalog",
            )
            .await?;
        let tenant_scope_ids = scope_ids(&catalog, &STANDARD_BOT_TENANT_SCOPES)?;
        self.call(
            Method::Post,
            &format!("scope/update/{app_id}"),
            Some(json!({
                "clientId": app_id,
                "appScopeIDs": tenant_scope_ids,
                "userScopeIDs": [],
                "scopeIds": [],
                "operation": "add",
            })),
            "console_scopes",
        )
        .await?;
        Ok(())
    }

    async fn publish_authorization(
        &mut self,
        app_id: &str,
        version_name: &str,
    ) -> Result<(), ClientError> {
        let version = self
            .call(
                Method::Post,
                &format!("app_version/create/{app_id}"),
                Some(json!({
                    "clientId": app_id,
                    "appVersion": version_name,
                    "changeLog": "Lankide",
                    "autoPublish": false,
                    "pcDefaultAbility": "bot",
                    "mobileDefaultAbility": "bot",
                })),
                "console_version_create",
            )
            .await?;
        let version_id = text_field(&version, "versionId", "console_version_create")?;
        let commit = self
            .call(
                Method::Post,
                &format!("publish/commit/{app_id}/{version_id}"),
                Some(json!({"clientId": app_id, "versionId": version_id})),
                "console_version_commit",
            )
            .await?;
        if commit.get("isOk").and_then(Value::as_bool) != Some(true) {
            return Err(ClientError::MissingField {
                operation: "console_version_commit".into(),
                field: "isOk".into(),
            });
        }
        Ok(())
    }

    async fn upload_icon(&mut self) -> Result<String, ClientError> {
        let boundary = format!("lankide{}", uuid::Uuid::new_v4().simple());
        let mut body = Vec::with_capacity(APP_ICON.len() + 512);
        for (name, value) in [
            ("uploadType", "4"),
            ("isIsv", "false"),
            ("scale", r#"{"width":512,"height":512}"#),
        ] {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"icon.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(APP_ICON);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let request = Request::new(
            Method::Post,
            format!("{OPEN}/developers/v1/app/upload/image"),
        )
        .header("User-Agent", USER_AGENT)
        .header("Referer", CONSOLE_HOME)
        .header("x-csrf-token", self.session.csrf.clone())
        .body(body, format!("multipart/form-data; boundary={boundary}"));
        let uploaded = self.send(request, "console_icon_upload").await?;
        text_field(&uploaded, "url", "console_icon_upload")
    }
}

fn text_field(value: &Value, field: &str, operation: &str) -> Result<String, ClientError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ClientError::MissingField {
            operation: operation.to_owned(),
            field: field.to_owned(),
        })
}

fn scope_ids(catalog: &Value, names: &[&str]) -> Result<Vec<String>, ClientError> {
    let scopes = catalog
        .get("scopes")
        .and_then(Value::as_array)
        .ok_or_else(|| ClientError::MissingField {
            operation: "console_scope_catalog".into(),
            field: "scopes".into(),
        })?;
    names
        .iter()
        .map(|name| {
            scopes
                .iter()
                .find(|scope| scope.get("name").and_then(Value::as_str) == Some(*name))
                .and_then(|scope| match scope.get("id") {
                    Some(Value::String(id)) => Some(id.clone()),
                    Some(Value::Number(id)) => Some(id.to_string()),
                    _ => None,
                })
                .ok_or_else(|| ClientError::MissingField {
                    operation: "console_scope_catalog".into(),
                    field: (*name).to_owned(),
                })
        })
        .collect()
}

/// QR cookies and the one-use onboarding handoff exist only in memory.
#[derive(Default)]
pub(crate) struct LoginFlow {
    pending: Option<PendingLogin>,
    ready: Option<ConsoleSession>,
}
#[derive(Debug)]
pub(crate) enum Login {
    Ready(ConsoleSession),
    Waiting { qr_code: String, scanned: bool },
}
impl LoginFlow {
    pub(crate) async fn advance(
        &mut self,
        transport: &dyn Transport,
        action: &str,
    ) -> Result<Login, ProviderError> {
        if action != "start"
            && let Some(session) = self.ready.take()
        {
            return Ok(Login::Ready(session));
        }
        if action == "start" || self.pending.as_ref().is_none_or(PendingLogin::expired) {
            self.ready = None;
            self.pending = Some(
                start_login(transport)
                    .await
                    .map_err(|e| e.provider_error(false))?,
            );
            return Ok(Login::Waiting {
                qr_code: self.pending.as_ref().unwrap().qr_code.clone(),
                scanned: false,
            });
        }
        let pending = self.pending.as_mut().unwrap();
        match poll_login(transport, pending)
            .await
            .map_err(|e| e.provider_error(false))?
        {
            LoginPoll::Complete(session) => {
                self.pending = None;
                Ok(Login::Ready(session))
            }
            LoginPoll::Waiting => Ok(Login::Waiting {
                qr_code: pending.qr_code.clone(),
                scanned: false,
            }),
            LoginPoll::Scanned => Ok(Login::Waiting {
                qr_code: pending.qr_code.clone(),
                scanned: true,
            }),
            LoginPoll::Expired => {
                self.pending = None;
                Ok(Login::Waiting {
                    qr_code: String::new(),
                    scanned: false,
                })
            }
        }
    }
    pub(crate) fn hold(&mut self, session: ConsoleSession) {
        self.ready = Some(session);
    }
    pub(crate) fn take(&mut self) -> Option<ConsoleSession> {
        self.ready.take()
    }
}
