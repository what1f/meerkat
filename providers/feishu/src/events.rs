//! Long-connection/event-source boundary for Feishu.
//!
//! The Feishu SDK owns websocket framing while the Provider adapter owns the
//! source-factory retry loop.  This module owns only registration, dispatch
//! and readiness semantics so a host can plug the SDK (or a deterministic
//! peer) in without leaking SDK types into the Provider contract.

use crate::normalize::{NormalizationConfig, NormalizedEvent, normalize_event};
use std::collections::HashMap;
use std::fmt;

pub type EventFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub trait EventSource: Send {
    fn next(&mut self) -> EventFuture<'_, Result<Option<RawEvent>, SourceError>>;
    /// Confirm the last event only after durable acceptance or a deliberate skip.
    fn acknowledge(&mut self) -> EventFuture<'_, Result<(), SourceError>>;
    fn close(&mut self) -> EventFuture<'_, ()>;
}
impl<T: EventSource + ?Sized> EventSource for Box<T> {
    fn next(&mut self) -> EventFuture<'_, Result<Option<RawEvent>, SourceError>> {
        self.as_mut().next()
    }
    fn acknowledge(&mut self) -> EventFuture<'_, Result<(), SourceError>> {
        self.as_mut().acknowledge()
    }
    fn close(&mut self) -> EventFuture<'_, ()> {
        self.as_mut().close()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawEvent {
    pub event_type: String,
    pub body: Vec<u8>,
}

impl RawEvent {
    pub fn new(event_type: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            event_type: event_type.into(),
            body: body.into(),
        }
    }
}

pub trait EventSink: Send {
    fn emit(&mut self, event: NormalizedEvent) -> EventFuture<'_, Result<(), SinkError>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceError {
    Disconnected(String),
    Malformed(String),
    Failed(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected(message) => {
                write!(formatter, "Feishu event source disconnected: {message}")
            }
            Self::Malformed(message) => write!(
                formatter,
                "malformed Feishu event source payload: {message}"
            ),
            Self::Failed(message) => write!(formatter, "Feishu event source failed: {message}"),
        }
    }
}

impl std::error::Error for SourceError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SinkError {
    pub reason: String,
    pub retryable: bool,
}
impl SinkError {
    pub fn permanent(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            retryable: false,
        }
    }
}
impl From<crate::http::ClientError> for SinkError {
    fn from(error: crate::http::ClientError) -> Self {
        Self {
            retryable: error.is_retryable(),
            reason: error.to_string(),
        }
    }
}
impl From<lankide_provider::ProviderError> for SinkError {
    fn from(error: lankide_provider::ProviderError) -> Self {
        let retryable = matches!(
            error,
            lankide_provider::ProviderError::External {
                retryable: true,
                ..
            } | lankide_provider::ProviderError::Timeout { .. }
                | lankide_provider::ProviderError::EventSinkClosed
        );
        Self {
            retryable,
            reason: format!("{error:?}"),
        }
    }
}
impl fmt::Display for SinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}
impl From<crate::normalize::NormalizeError> for SinkError {
    fn from(error: crate::normalize::NormalizeError) -> Self {
        Self::permanent(error.to_string())
    }
}
impl From<serde_json::Error> for SinkError {
    fn from(error: serde_json::Error) -> Self {
        Self::permanent(error.to_string())
    }
}
impl std::error::Error for SinkError {}

/// Registers handlers for a single App's event types and routes each event to
/// the shared normalizer.  One App should own one source even when IM and Task
/// channels are consumed by different higher-level adapters.
pub struct Dispatcher<S> {
    source: S,
    handlers: HashMap<String, bool>,
    config: NormalizationConfig,
    ready: bool,
    stopped: bool,
    pending: Option<RawEvent>,
    accepted: bool,
}

impl<S: EventSource> Dispatcher<S> {
    pub fn new(source: S, config: NormalizationConfig) -> Self {
        Self {
            source,
            handlers: HashMap::new(),
            config,
            ready: false,
            stopped: false,
            pending: None,
            accepted: false,
        }
    }

    pub fn register(&mut self, event_type: impl Into<String>) -> Result<(), SourceError> {
        let event_type = event_type.into();
        if event_type.trim().is_empty() {
            return Err(SourceError::Malformed(
                "Feishu event type is required".to_owned(),
            ));
        }
        if self.handlers.insert(event_type.clone(), true).is_some() {
            return Err(SourceError::Malformed(format!(
                "Feishu event type {event_type} is already registered"
            )));
        }
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub async fn poll_once<E: EventSink>(&mut self, sink: &mut E) -> Result<bool, DispatchError> {
        if self.stopped {
            return Ok(false);
        }
        if self.accepted {
            self.skip().await?;
            return Ok(false);
        }
        if self.pending.is_none() {
            self.pending = self.source.next().await.map_err(DispatchError::Source)?;
        }
        let Some(raw) = self.pending.as_ref() else {
            self.ready = true;
            return Ok(false);
        };
        self.ready = true;
        let event_type = raw.event_type.trim();
        let registered = self.handlers.contains_key(event_type)
            || (event_type.starts_with("task.") && self.handlers.contains_key("task.*"));
        if !self.handlers.is_empty() && !registered {
            // Apps can retain subscriptions for disabled channels. They must
            // not permanently stop delivery of the channels enabled here.
            self.source
                .acknowledge()
                .await
                .map_err(DispatchError::Source)?;
            self.pending = None;
            return Ok(false);
        }
        if !self.accepted {
            match normalize_event(&raw.body, &self.config) {
                Ok(event) => sink.emit(event).await.map_err(DispatchError::Sink)?,
                Err(crate::normalize::NormalizeError::SelfReaction) => {
                    // Deliberately ignored feedback still needs a transport receipt.
                }
                Err(error) => return Err(DispatchError::Normalize(error)),
            }
            self.accepted = true;
        }
        self.skip().await?;
        Ok(true)
    }

    pub fn discard(&mut self) {
        self.accepted = true;
    }

    pub async fn skip(&mut self) -> Result<(), DispatchError> {
        self.source
            .acknowledge()
            .await
            .map_err(DispatchError::Source)?;
        self.pending = None;
        self.accepted = false;
        Ok(())
    }

    pub async fn stop(&mut self) {
        self.stopped = true;
        self.source.close().await;
    }
}

#[derive(Debug)]
pub enum DispatchError {
    Source(SourceError),
    Normalize(crate::normalize::NormalizeError),
    Sink(SinkError),
    Unregistered(String),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => error.fmt(formatter),
            Self::Normalize(error) => error.fmt(formatter),
            Self::Sink(error) => error.fmt(formatter),
            Self::Unregistered(event) => {
                write!(formatter, "no Feishu handler is registered for {event}")
            }
        }
    }
}

impl std::error::Error for DispatchError {}
