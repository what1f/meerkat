//! Small transport boundary shared by the Feishu REST client and its tests.
//!
//! The provider deliberately does not own a runtime or an HTTP implementation.
//! A host can inject its configured TLS client, while module scenarios can use
//! a local peer for deterministic failures.  Keeping requests as plain DTOs
//! also makes it possible to inspect the exact Open API contract without
//! exposing credentials through `Debug` output.

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
                if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("cookie")
                {
                    (name.clone(), "<redacted>".to_owned())
                } else {
                    (name.clone(), value.clone())
                }
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("Request")
            .field("method", &self.method)
            .field("path", &self.path)
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

pub type TransportFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Response, TransportError>> + Send + 'a>,
>;

/// Asynchronous HTTP boundary; all requests use the caller's Tokio runtime.
pub trait Transport: Send + Sync {
    fn execute(&self, request: Request) -> TransportFuture<'_>;
}
impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn execute(&self, request: Request) -> TransportFuture<'_> {
        self.as_ref().execute(request)
    }
}

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
    fn execute(&self, request: Request) -> TransportFuture<'_> {
        Box::pin(async move {
            let method = match request.method {
                Method::Get => reqwest::Method::GET,
                Method::Post => reqwest::Method::POST,
                Method::Put => reqwest::Method::PUT,
                Method::Patch => reqwest::Method::PATCH,
                Method::Delete => reqwest::Method::DELETE,
            };
            let mut builder = self.client.request(method, &request.path);
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
        })
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
    },
    MissingField {
        operation: String,
        field: String,
    },
}

impl ClientError {
    pub(crate) fn provider_error(&self, configuration: bool) -> lankide_provider::ProviderError {
        let (operation, upstream_code, status) = match self {
            Self::Api {
                operation, code, ..
            } => (operation.as_str(), Some(*code), None),
            Self::HttpStatus { operation, status } => (operation.as_str(), None, Some(*status)),
            Self::MissingField { operation, .. } => (operation.as_str(), None, None),
            Self::Transport(_) => ("transport", None, None),
            Self::Decode(_) => ("decode", None, None),
            Self::Encode(_) | Self::InvalidInput(_) => ("request", None, None),
        };
        // Do not forward arbitrary strings, URLs or upstream messages into diagnostics.
        let operation = match operation {
            "transport" => "transport",
            "decode" => "decode",
            "request" => "request",
            _ => "feishu_api",
        };
        if configuration {
            lankide_provider::ProviderError::AppConfigurationFailed {
                operation,
                upstream_code,
                status,
            }
        } else {
            lankide_provider::ProviderError::Upstream {
                operation,
                upstream_code,
                status,
                retryable: self.is_retryable(),
            }
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::HttpStatus { status, .. } => matches!(status, 408 | 429 | 500..=599),
            // Common platform rate limit and documented temporary server errors.
            // Other API codes (permissions, invalid tokens, missing targets) need
            // a changed request or configuration.
            Self::Api { code, .. } => matches!(
                code,
                99991400 | 230020 | 11232 | 11233 | 1500 | 1503 | 1551 | 99991644 | 99991645
            ),
            _ => false,
        }
    }
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
            } => write!(
                formatter,
                "{operation} failed: code={code} message={message}"
            ),
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

pub(crate) fn validate_success(response: &Response, operation: &str) -> Result<(), ClientError> {
    if (200..300).contains(&response.status) {
        Ok(())
    } else {
        if !matches!(response.status, 408 | 429 | 500..=599)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&response.body)
            && let Some(code) = value
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .filter(|code| *code != 0)
        {
            return Err(ClientError::Api {
                code,
                message: value
                    .get("msg")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Feishu API rejected the request")
                    .to_owned(),
                operation: operation.to_owned(),
            });
        }
        Err(ClientError::HttpStatus {
            status: response.status,
            operation: operation.to_owned(),
        })
    }
}

pub fn encode_path_segment(value: &str) -> String {
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
