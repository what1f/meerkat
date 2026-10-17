//! Feishu developer console, driven with the signed-in user's web session.
//!
//! Feishu has no public API that creates an App. The console's own web API
//! does, so a user who scans one sign-in QR code lets Lankide create, configure
//! and publish Agent Apps, and authorize as that user, without further
//! confirmations. Every call here is the same request the console page makes.

use crate::authorization::{STANDARD_BOT_EVENTS, STANDARD_BOT_TENANT_SCOPES};
use crate::http::{ClientError, Method, Request, Response, Transport, encode_query_component};
use lankide_provider::{ProviderError, SecretReader, SecretValue, SecretWriter};
use serde::{Deserialize, Serialize};
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
/// Never visited: Lankide reads the authorization code from the redirect.
pub(crate) const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1/lankide/feishu-oauth";
/// User scopes Agent Apps request so the team lead's own Tasklist can be
/// created, shared, and updated as the team lead.
pub(crate) const USER_TASKLIST_SCOPES: [&str; 4] = [
    "task:tasklist:read",
    "task:tasklist:write",
    "task:task:read",
    "task:task:write",
];
const APP_ICON: &[u8] = include_bytes!("../assets/app-icon.png");

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CookieJar {
    cookies: Vec<Cookie>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        if next.as_str().starts_with(OAUTH_REDIRECT_URI) {
            return Ok((response, next.into()));
        }
        request = Request::new(Method::Get, next.to_string()).header("User-Agent", USER_AGENT);
    }
    Err(ClientError::HttpStatus {
        status: 310,
        operation: operation.to_owned(),
    })
}

/// Sign-in that has shown a QR code and waits for the user's scan.
#[derive(Clone, Default, Serialize, Deserialize)]
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
#[derive(Clone, Default, Serialize, Deserialize)]
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

/// Team lead identity returned by the user's authorization.
pub(crate) struct UserGrant {
    access_token: String,
    pub(crate) info: Value,
    pub(crate) open_id: String,
    pub(crate) union_id: String,
    pub(crate) user_id: String,
}

impl fmt::Debug for UserGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserGrant")
            .field("open_id", &self.open_id)
            .finish_non_exhaustive()
    }
}

impl UserGrant {
    pub(crate) fn access_token(&self) -> &str {
        &self.access_token
    }
}

pub(crate) struct DeveloperConsole {
    transport: Arc<dyn Transport>,
    session: ConsoleSession,
}

impl DeveloperConsole {
    pub(crate) fn new(transport: Arc<dyn Transport>, session: ConsoleSession) -> Self {
        Self { transport, session }
    }

    pub(crate) fn into_session(self) -> ConsoleSession {
        self.session
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
        let (response, _) = fetch(
            self.transport.as_ref(),
            &mut self.session.jar,
            request,
            operation,
        )
        .await?;
        if (300..400).contains(&response.status) {
            // Console calls redirect to sign-in once the session has ended.
            return Err(ClientError::HttpStatus {
                status: 401,
                operation: operation.to_owned(),
            });
        }
        passport_data(&response, operation)
    }

    /// Confirms the session still signs in to the console.
    pub(crate) async fn verify(&mut self) -> Result<(), ClientError> {
        self.call(
            Method::Get,
            "app/list?Count=1&Cursor=0",
            None,
            "console_app_list",
        )
        .await
        .map(|_| ())
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
        let catalog = self
            .call(
                Method::Get,
                &format!("scope/all/{app_id}"),
                None,
                "console_scope_catalog",
            )
            .await?;
        let tenant_scope_ids = scope_ids(&catalog, &STANDARD_BOT_TENANT_SCOPES)?;
        let user_scope_ids = scope_ids(&catalog, &USER_TASKLIST_SCOPES)?;
        self.call(
            Method::Post,
            &format!("scope/update/{app_id}"),
            Some(json!({
                "clientId": app_id,
                "appScopeIDs": tenant_scope_ids,
                "userScopeIDs": user_scope_ids,
                "scopeIds": [],
                "operation": "add",
            })),
            "console_scopes",
        )
        .await?;
        self.call(
            Method::Post,
            &format!("safe_setting/update/{app_id}"),
            Some(json!({"clientId": app_id, "redirectURL": [OAUTH_REDIRECT_URI]})),
            "console_redirect_url",
        )
        .await?;
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
        let version = self
            .call(
                Method::Post,
                &format!("app_version/create/{app_id}"),
                Some(json!({
                    "clientId": app_id,
                    "appVersion": "1.0.0",
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
        Ok(app_secret)
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

    /// Authorizes `app` as the signed-in user and returns the user's token.
    /// The console session already identifies the user, so the consent the
    /// authorization page would ask for is confirmed directly.
    pub(crate) async fn authorize_user(
        &mut self,
        app: &str,
        app_secret: &str,
        base_url: &str,
    ) -> Result<UserGrant, ClientError> {
        let scope = USER_TASKLIST_SCOPES.join(" ");
        let authorize = format!(
            "{ACCOUNTS}/open-apis/authen/v1/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}&state=lankide",
            encode_query_component(app),
            encode_query_component(OAUTH_REDIRECT_URI),
            encode_query_component(&scope),
        );
        let (response, final_url) = fetch(
            self.transport.as_ref(),
            &mut self.session.jar,
            Request::new(Method::Get, authorize.clone()).header("User-Agent", USER_AGENT),
            "user_authorize",
        )
        .await?;
        let redirect = if final_url.starts_with(OAUTH_REDIRECT_URI) {
            final_url
        } else {
            crate::http::validate_success(&response, "user_authorize")?;
            self.confirm_consent(app, &authorize, &response).await?
        };
        let code = url::Url::parse(&redirect)
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "code")
                    .map(|(_, value)| value.into_owned())
            })
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ClientError::MissingField {
                operation: "user_authorize".into(),
                field: "code".into(),
            })?;
        self.exchange_code(app, app_secret, &code, base_url).await
    }

    async fn confirm_consent(
        &mut self,
        app: &str,
        authorize_url: &str,
        page: &Response,
    ) -> Result<String, ClientError> {
        let html = String::from_utf8_lossy(&page.body);
        let marker = "window.serverInjectRes = ";
        let injected = html
            .find(marker)
            .map(|start| &html[start + marker.len()..])
            .and_then(|rest| {
                serde_json::Deserializer::from_str(rest)
                    .into_iter::<Value>()
                    .next()?
                    .ok()
            })
            .ok_or_else(|| ClientError::MissingField {
                operation: "user_consent".into(),
                field: "serverInjectRes".into(),
            })?;
        let user =
            injected
                .pointer("/data/current_user")
                .ok_or_else(|| ClientError::MissingField {
                    operation: "user_consent".into(),
                    field: "current_user".into(),
                })?;
        let user_id = text_field(user, "user_id", "user_consent")?;
        let scope = user
            .get("all_scope_key_list")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let flow_key = injected
            .get("X-Auth-Flow-Key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let csrf = self
            .session
            .jar
            .value("open_csrf_token")
            .unwrap_or_default()
            .to_owned();
        let request = Request::new(
            Method::Post,
            format!("{ACCOUNTS}/open-apis/authen/v1/confirm"),
        )
        .header("User-Agent", USER_AGENT)
        .header("Referer", authorize_url)
        .header("Origin", ACCOUNTS)
        .header("open-csrf-token", csrf)
        .header("X-Auth-Flow-Key", flow_key)
        .json(&json!({"app_id": app, "user_id": user_id, "scope": scope}))?;
        let confirmed = self.send(request, "user_consent").await?;
        text_field(&confirmed, "redirect_uri", "user_consent")
    }

    async fn exchange_code(
        &self,
        app: &str,
        app_secret: &str,
        code: &str,
        base_url: &str,
    ) -> Result<UserGrant, ClientError> {
        let request = Request::new(
            Method::Post,
            format!("{base_url}/open-apis/authen/v2/oauth/token"),
        )
        .json(&json!({
            "grant_type": "authorization_code",
            "client_id": app,
            "client_secret": app_secret,
            "code": code,
            "redirect_uri": OAUTH_REDIRECT_URI,
        }))?;
        let response = self.transport.execute(request).await?;
        crate::http::validate_success(&response, "user_token")?;
        let token: Value = response.json()?;
        if token.get("code").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Err(ClientError::Api {
                code: token.get("code").and_then(Value::as_i64).unwrap_or(-1),
                message: token
                    .get("error_description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                operation: "user_token".into(),
            });
        }
        let access_token = text_field(&token, "access_token", "user_token")?;
        let request = Request::new(
            Method::Get,
            format!("{base_url}/open-apis/authen/v1/user_info"),
        )
        .header("Authorization", format!("Bearer {access_token}"));
        let response = self.transport.execute(request).await?;
        let info = passport_data(&response, "user_info")?;
        Ok(UserGrant {
            info: info.clone(),
            open_id: text_field(&info, "open_id", "user_info")?,
            union_id: info
                .get("union_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            user_id: info
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            access_token,
        })
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

const SESSION_SECRET: &str = "feishu_console_session";
const PENDING_LOGIN_SECRET: &str = "feishu_console_login";

/// Developer console sign-in as the user sees it.
#[derive(Debug)]
pub(crate) enum Login {
    Ready(ConsoleSession),
    Required,
    Waiting { qr_code: String, scanned: bool },
}

async fn read_secret<T: serde::de::DeserializeOwned>(
    reader: &dyn SecretReader,
    name: &str,
) -> Result<Option<T>, ProviderError> {
    match reader.read(name).await {
        Ok(value) => Ok(serde_json::from_str(value.expose_secret()).ok()),
        Err(ProviderError::SecretAccessDenied) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn write_secret<T: Serialize>(
    writer: &dyn SecretWriter,
    name: &str,
    value: &T,
) -> Result<(), ProviderError> {
    let encoded = serde_json::to_string(value).map_err(|_| ProviderError::External {
        operation: "feishu_console_state",
        retryable: false,
    })?;
    writer.write(name, SecretValue::new(encoded)).await
}

fn console_error(operation: &'static str, error: &ClientError) -> ProviderError {
    ProviderError::External {
        operation,
        retryable: error.is_retryable(),
    }
}

/// The saved console session, when it still signs in.
pub(crate) async fn saved_session(
    transport: Arc<dyn Transport>,
    reader: &dyn SecretReader,
) -> Result<Option<ConsoleSession>, ProviderError> {
    let Some(session) = read_secret::<ConsoleSession>(reader, SESSION_SECRET).await? else {
        return Ok(None);
    };
    let mut console = DeveloperConsole::new(transport, session);
    match console.verify().await {
        Ok(()) => Ok(Some(console.session)),
        Err(error) if error.is_retryable() => Err(console_error("feishu_console_verify", &error)),
        Err(_) => Ok(None),
    }
}

pub(crate) async fn save_session(
    writer: &dyn SecretWriter,
    session: &ConsoleSession,
) -> Result<(), ProviderError> {
    write_secret(writer, SESSION_SECRET, session).await
}

/// Advances the sign-in: `start` shows a new QR code, `poll` checks the scan
/// and replaces a code before it expires, anything else only reports state.
pub(crate) async fn login(
    transport: Arc<dyn Transport>,
    reader: &dyn SecretReader,
    writer: &dyn SecretWriter,
    action: &str,
) -> Result<Login, ProviderError> {
    if let Some(session) = saved_session(transport.clone(), reader).await? {
        return Ok(Login::Ready(session));
    }
    let pending = read_secret::<PendingLogin>(reader, PENDING_LOGIN_SECRET)
        .await?
        .filter(|pending| !pending.expired());
    let mut pending = match (action, pending) {
        ("poll", Some(pending)) => pending,
        ("start" | "poll", _) => {
            let pending = start_login(transport.as_ref())
                .await
                .map_err(|error| console_error("feishu_console_login", &error))?;
            write_secret(writer, PENDING_LOGIN_SECRET, &pending).await?;
            return Ok(Login::Waiting {
                qr_code: pending.qr_code,
                scanned: false,
            });
        }
        _ => return Ok(Login::Required),
    };
    let polled = poll_login(transport.as_ref(), &mut pending)
        .await
        .map_err(|error| console_error("feishu_console_login", &error))?;
    let scanned = match polled {
        LoginPoll::Complete(session) => {
            save_session(writer, &session).await?;
            write_secret(writer, PENDING_LOGIN_SECRET, &PendingLogin::default()).await?;
            return Ok(Login::Ready(session));
        }
        LoginPoll::Expired => {
            let pending = start_login(transport.as_ref())
                .await
                .map_err(|error| console_error("feishu_console_login", &error))?;
            write_secret(writer, PENDING_LOGIN_SECRET, &pending).await?;
            return Ok(Login::Waiting {
                qr_code: pending.qr_code,
                scanned: false,
            });
        }
        LoginPoll::Waiting => false,
        LoginPoll::Scanned => true,
    };
    write_secret(writer, PENDING_LOGIN_SECRET, &pending).await?;
    Ok(Login::Waiting {
        qr_code: pending.qr_code,
        scanned,
    })
}

/// A candidate session never replaces the current binding while QR login is pending.
const SWITCH_CANDIDATE: &str = "feishu_console_switch_candidate";
const SWITCH_PENDING: &str = "feishu_console_switch_pending";

pub(crate) async fn switch_is_current(
    reader: &dyn SecretReader,
    candidate: &str,
) -> Result<bool, ProviderError> {
    Ok(!candidate.is_empty()
        && read_secret::<String>(reader, SWITCH_CANDIDATE)
            .await?
            .as_deref()
            == Some(candidate))
}
pub(crate) async fn finish_switch(writer: &dyn SecretWriter) -> Result<(), ProviderError> {
    write_secret(writer, SWITCH_CANDIDATE, &String::new()).await?;
    write_secret(writer, SWITCH_PENDING, &PendingLogin::default()).await
}

pub(crate) async fn switch_login(
    transport: Arc<dyn Transport>,
    reader: &dyn SecretReader,
    writer: &dyn SecretWriter,
    action: &str,
    lock: &tokio::sync::Mutex<()>,
) -> Result<(String, Login), ProviderError> {
    let guard = lock.lock().await;
    if action == "switch_cancel" {
        finish_switch(writer).await?;
        return Ok((String::new(), Login::Required));
    }
    if action == "switch_start" {
        let candidate = uuid::Uuid::new_v4().to_string();
        write_secret(writer, SWITCH_CANDIDATE, &candidate).await?;
        write_secret(writer, SWITCH_PENDING, &PendingLogin::default()).await?;
        drop(guard);
        let pending = start_login(transport.as_ref())
            .await
            .map_err(|e| console_error("feishu_console_login", &e))?;
        let _guard = lock.lock().await;
        if !switch_is_current(reader, &candidate).await? {
            return Ok((candidate, Login::Required));
        }
        write_secret(writer, SWITCH_PENDING, &pending).await?;
        return Ok((
            candidate,
            Login::Waiting {
                qr_code: pending.qr_code,
                scanned: false,
            },
        ));
    }
    let candidate = read_secret::<String>(reader, SWITCH_CANDIDATE)
        .await?
        .unwrap_or_default();
    let pending = read_secret::<PendingLogin>(reader, SWITCH_PENDING)
        .await?
        .filter(|p| !p.expired());
    let Some(mut pending) = pending.filter(|_| !candidate.is_empty()) else {
        return Ok((candidate, Login::Required));
    };
    drop(guard);
    let result = poll_login(transport.as_ref(), &mut pending)
        .await
        .map_err(|e| console_error("feishu_console_login", &e))?;
    let _guard = lock.lock().await;
    if !switch_is_current(reader, &candidate).await? {
        return Ok((candidate, Login::Required));
    }
    let login = match result {
        // Keep the candidate valid until OAuth and profile preparation finish.
        LoginPoll::Complete(session) => Login::Ready(session),
        LoginPoll::Expired => {
            finish_switch(writer).await?;
            Login::Required
        }
        LoginPoll::Waiting | LoginPoll::Scanned => {
            write_secret(writer, SWITCH_PENDING, &pending).await?;
            Login::Waiting {
                qr_code: pending.qr_code,
                scanned: matches!(result, LoginPoll::Scanned),
            }
        }
    };
    Ok((candidate, login))
}
