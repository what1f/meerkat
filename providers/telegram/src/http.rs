//! Transport boundary for the Telegram Bot API client.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for Request {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization") {
                    (name.clone(), "<redacted>".to_owned())
                } else {
                    (name.clone(), value.clone())
                }
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("Request")
            .field("method", &self.method)
            // Telegram puts the bot token in the endpoint path rather than an
            // Authorization header.  Never let a request snapshot leak it.
            .field("path", &redact_bot_token(&self.path))
            .field("query", &self.query)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

impl Request {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
    pub fn json<T: Serialize>(mut self, value: &T) -> Result<Self, ClientError> {
        self.body =
            serde_json::to_vec(value).map_err(|error| ClientError::Encode(error.to_string()))?;
        self.headers
            .push(("Content-Type".to_owned(), "application/json".to_owned()));
        Ok(self)
    }
    pub fn body(mut self, body: impl Into<Vec<u8>>, content_type: impl Into<String>) -> Self {
        self.body = body.into();
        self.headers
            .push(("Content-Type".to_owned(), content_type.into()));
        self
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization")
                    || name.eq_ignore_ascii_case("cookie")
                    || name.eq_ignore_ascii_case("set-cookie")
                {
                    (name.clone(), "<redacted>".to_owned())
                } else {
                    (name.clone(), value.clone())
                }
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

impl Response {
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, ClientError> {
        serde_json::from_slice(&self.body).map_err(|error| ClientError::Decode(error.to_string()))
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub trait Transport: Send + Sync {
    fn execute(&self, request: Request) -> Result<Response, TransportError>;
}

impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        self.as_ref().execute(request)
    }
}

/// Pooled connections belong to the runtime that opened them, so every
/// request runs on one process-wide runtime that is never dropped.
static HTTP_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lankide-telegram-http")
            .enable_all()
            .build()
            .expect("HTTP runtime starts")
    });

/// Production HTTP transport used by the Provider adapter.  The low-level
/// Bot API client remains synchronous, so each request is bridged through a
/// shared Tokio runtime.
#[derive(Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl fmt::Debug for ReqwestTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReqwestTransport([configured])")
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::with_timeout(Duration::from_secs(30))
    }
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("reqwest client configuration is valid"),
        }
    }
}

impl Transport for ReqwestTransport {
    fn execute(&self, request: Request) -> Result<Response, TransportError> {
        let client = self.client.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        HTTP_RUNTIME.spawn(async move {
            let result = async move {
                let method = match request.method {
                    Method::Get => reqwest::Method::GET,
                    Method::Post => reqwest::Method::POST,
                    Method::Put => reqwest::Method::PUT,
                    Method::Patch => reqwest::Method::PATCH,
                    Method::Delete => reqwest::Method::DELETE,
                };
                let mut builder = client.request(method, &request.path);
                if !request.query.is_empty() {
                    builder = builder.query(&request.query);
                }
                for (name, value) in &request.headers {
                    builder = builder.header(name, value);
                }
                if !request.body.is_empty() {
                    builder = builder.body(request.body);
                }
                let response = builder.send().await.map_err(|error| {
                    TransportError::Failed(format!("HTTP request failed: {}", error.without_url()))
                })?;
                let status = response.status().as_u16();
                let headers = response
                    .headers()
                    .iter()
                    .filter_map(|(name, value)| {
                        value
                            .to_str()
                            .ok()
                            .map(|value| (name.to_string(), value.to_owned()))
                    })
                    .collect();
                let body = response
                    .bytes()
                    .await
                    .map_err(|error| {
                        TransportError::Failed(format!(
                            "HTTP response body read failed: {}",
                            error.without_url()
                        ))
                    })?
                    .to_vec();
                Ok(Response {
                    status,
                    headers,
                    body,
                })
            }
            .await;
            let _ = sender.send(result);
        });
        receiver
            .recv()
            .map_err(|_| TransportError::Failed("HTTP transport stopped".to_owned()))?
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportError {
    Failed(String),
    Cancelled,
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(message) => formatter.write_str(message),
            Self::Cancelled => formatter.write_str("transport was cancelled"),
        }
    }
}
impl Error for TransportError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError {
    InvalidInput(String),
    Encode(String),
    Decode(String),
    Transport(TransportError),
    HttpStatus {
        status: u16,
        operation: String,
    },
    Api {
        code: i64,
        message: String,
        operation: String,
        retry_after: Option<u64>,
    },
    MissingField {
        operation: String,
        field: String,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::Encode(message) => write!(formatter, "encode request: {message}"),
            Self::Decode(message) => write!(formatter, "decode response: {message}"),
            Self::Transport(error) => write!(formatter, "transport request failed: {error}"),
            Self::HttpStatus { status, operation } => {
                write!(formatter, "{operation} returned HTTP status {status}")
            }
            Self::Api {
                code,
                message,
                operation,
                retry_after,
            } => match retry_after {
                Some(seconds) => write!(
                    formatter,
                    "{operation} failed: code={code} message={message} (retry after {seconds}s)"
                ),
                None => write!(
                    formatter,
                    "{operation} failed: code={code} message={message}"
                ),
            },
            Self::MissingField { operation, field } => {
                write!(formatter, "{operation} response has no {field}")
            }
        }
    }
}
impl Error for ClientError {}
impl From<TransportError> for ClientError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

pub fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b':') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex(byte >> 4));
            encoded.push(hex(byte & 0x0f));
        }
    }
    encoded
}

pub fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex(byte >> 4));
            encoded.push(hex(byte & 0x0f));
        }
    }
    encoded
}

fn hex(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'A' + value - 10) as char,
    }
}

fn redact_bot_token(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut cursor = 0;
    while let Some(relative) = path[cursor..].find("/bot") {
        let start = cursor + relative;
        let token_start = start + "/bot".len();
        let Some(token_end) = path[token_start..]
            .find(|character: char| matches!(character, '/' | '?' | '#'))
            .map(|offset| token_start + offset)
        else {
            result.push_str(&path[cursor..start]);
            result.push_str("/bot<redacted>");
            return result;
        };
        result.push_str(&path[cursor..start]);
        result.push_str("/bot<redacted>");
        cursor = token_end;
    }
    result.push_str(&path[cursor..]);
    result
}
