//! Long-connection/event-source boundary for Feishu.
//!
//! The Feishu SDK owns websocket framing while the Provider adapter owns the
//! source-factory retry loop.  This module owns only registration, dispatch
//! and readiness semantics so a host can plug the SDK (or a deterministic
//! peer) in without leaking SDK types into the Provider contract.

use crate::normalize::{NormalizationConfig, NormalizedEvent, normalize_event};
use std::collections::HashMap;
use std::fmt;

pub trait EventSource: Send {
    fn next(&mut self) -> Result<Option<RawEvent>, SourceError>;
    /// Confirm the last event only after the consumer durably accepts it.
    fn acknowledge(&mut self) -> Result<(), SourceError>;
    fn close(&mut self);
}

impl<T: EventSource + ?Sized> EventSource for Box<T> {
    fn next(&mut self) -> Result<Option<RawEvent>, SourceError> {
        self.as_mut().next()
    }

    fn acknowledge(&mut self) -> Result<(), SourceError> {
        self.as_mut().acknowledge()
    }

    fn close(&mut self) {
        self.as_mut().close();
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

pub trait EventSink {
    fn emit(&mut self, event: NormalizedEvent) -> Result<(), SinkError>;
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
pub struct SinkError(pub String);

impl fmt::Display for SinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
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

    pub fn poll_once<E: EventSink>(&mut self, sink: &mut E) -> Result<bool, DispatchError> {
        if self.stopped {
            return Ok(false);
        }
        if self.pending.is_none() {
            self.pending = self.source.next().map_err(DispatchError::Source)?;
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
            self.source.acknowledge().map_err(DispatchError::Source)?;
            self.pending = None;
            return Ok(false);
        }
        match normalize_event(&raw.body, &self.config) {
            Ok(event) => sink.emit(event).map_err(DispatchError::Sink)?,
            Err(crate::normalize::NormalizeError::SelfReaction) => {
                // Deliberately ignored feedback still needs a transport receipt.
            }
            Err(error) => return Err(DispatchError::Normalize(error)),
        }
        self.source.acknowledge().map_err(DispatchError::Source)?;
        self.pending = None;
        Ok(true)
    }

    pub fn stop(&mut self) {
        self.stopped = true;
        self.source.close();
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
