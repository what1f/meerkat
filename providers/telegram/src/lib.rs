#![forbid(unsafe_code)]

//! Telegram-specific Provider implementation.

mod adapter;
pub mod client;
pub mod dto;
pub mod http;
mod identity;
pub mod mention;
pub mod normalize;
mod onboarding;
mod person_profile;
pub mod polling;
pub use adapter::TelegramProvider;
pub use client::{
    Client as TelegramClient, Credentials, MAX_DOWNLOAD_BYTES, ManagedBotCredential,
    ManagedBotRegistration, PollRequest, SentMessage,
};
pub use http::{
    ClientError, Method, Request, ReqwestTransport, Response, Transport, TransportError,
};
pub use mention::{
    MentionTarget, extract_agent_ids, mentioned_agent_ids, render_html_with_mentions,
};
pub use normalize::{
    NormalizationConfig, NormalizeError, NormalizedAttachment, NormalizedEvent, NormalizedMention,
    NormalizedMessage, NormalizedReaction, NormalizedSender, ReactionAction, SenderKind,
    normalize_message, normalize_reaction, normalize_update,
};
pub use onboarding::agent_onboarding;
pub use polling::{PollBatch, PollError, Poller};

mod workspace_owner;
