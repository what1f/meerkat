//! Telegram long-polling lifecycle with explicit acknowledgement semantics.

use crate::client::{Client, PollRequest};
use crate::dto::Update;
use crate::http::{ClientError, Transport};
use crate::normalize::{NormalizationConfig, NormalizeError, NormalizedEvent, normalize_update};
use std::fmt;

#[derive(Clone, Debug)]
pub struct PollBatch {
    pub offset: i64,
    pub updates: Vec<Update>,
    pub events: Vec<NormalizedEvent>,
    pub next_offset: i64,
}

impl PollBatch {
    pub fn is_empty(&self) -> bool {
        self.updates.is_empty()
    }
}

/// `poll_once` never advances the offset.  Call `acknowledge` only after the
/// Runtime has durably accepted every normalized event in the batch.  A
/// process restart or failed ingestion therefore replays the update instead
/// of silently dropping it.
pub struct Poller<T> {
    client: Client<T>,
    request: PollRequest,
    normalization: NormalizationConfig,
    stopped: bool,
}

impl<T: Transport> Poller<T> {
    pub fn new(client: Client<T>, normalization: NormalizationConfig) -> Self {
        Self {
            client,
            request: PollRequest::default(),
            normalization,
            stopped: false,
        }
    }
    pub fn with_request(mut self, request: PollRequest) -> Self {
        self.request = request;
        self
    }
    pub fn offset(&self) -> i64 {
        self.request.offset
    }
    pub fn stop(&mut self) {
        self.stopped = true;
    }
    pub fn poll_once(&mut self) -> Result<Option<PollBatch>, PollError> {
        if self.stopped {
            return Ok(None);
        }
        let updates = self
            .client
            .get_updates(&self.request)
            .map_err(PollError::Client)?;
        if updates.is_empty() {
            return Ok(Some(PollBatch {
                offset: self.request.offset,
                updates,
                events: Vec::new(),
                next_offset: self.request.offset,
            }));
        }
        let mut events = Vec::new();
        let mut next_offset = self.request.offset;
        for update in &updates {
            let normalized =
                normalize_update(update, &self.normalization).map_err(PollError::Normalize)?;
            events.extend(normalized);
            next_offset = next_offset.max(update.update_id.saturating_add(1));
        }
        Ok(Some(PollBatch {
            offset: self.request.offset,
            updates,
            events,
            next_offset,
        }))
    }
    pub fn acknowledge(&mut self, batch: &PollBatch) -> Result<(), PollError> {
        if batch.offset != self.request.offset {
            return Err(PollError::BatchOffsetMismatch {
                current: self.request.offset,
                batch: batch.offset,
            });
        }
        if batch.next_offset < self.request.offset {
            return Err(PollError::OffsetRegression {
                current: self.request.offset,
                next: batch.next_offset,
            });
        }
        self.request.offset = batch.next_offset;
        Ok(())
    }
    pub fn client(&self) -> &Client<T> {
        &self.client
    }
    pub fn client_mut(&mut self) -> &mut Client<T> {
        &mut self.client
    }
}

#[derive(Debug)]
pub enum PollError {
    Client(ClientError),
    Normalize(NormalizeError),
    OffsetRegression { current: i64, next: i64 },
    BatchOffsetMismatch { current: i64, batch: i64 },
}

impl fmt::Display for PollError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(error) => error.fmt(formatter),
            Self::Normalize(error) => error.fmt(formatter),
            Self::OffsetRegression { current, next } => write!(
                formatter,
                "Telegram polling offset regressed from {current} to {next}"
            ),
            Self::BatchOffsetMismatch { current, batch } => write!(
                formatter,
                "Telegram polling batch offset {batch} does not match current offset {current}"
            ),
        }
    }
}
impl std::error::Error for PollError {}
