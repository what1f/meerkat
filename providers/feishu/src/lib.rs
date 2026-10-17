#![forbid(unsafe_code)]

//! Feishu-specific Provider implementation.

mod adapter;
mod agent_tasks;
mod authorization;
pub mod client;
mod console;
pub mod dto;
pub mod events;
pub mod http;
mod identity;
pub mod mention;
pub mod normalize;
mod onboarding;
mod person_profile;
mod reactions;
pub mod registration;
pub mod tasks;
pub mod ws;

pub use adapter::{CONSOLE_LOGIN_INTERFACE, EventSourceFactory, FeishuProvider};
pub use authorization::{
    STANDARD_BOT_EVENTS, STANDARD_BOT_TENANT_SCOPES, authorization_url,
    contains_standard_capabilities,
};
pub use client::{
    Auth, Client as FeishuClient, Credentials, Domain, Page, PublishedCapabilities, ReactionItem,
    ReplyMessage, SendMessage, SentMessage, UploadedResource,
};
pub use events::{
    DispatchError, Dispatcher as FeishuEventDispatcher, EventSink, EventSource, RawEvent,
    SinkError, SourceError,
};
pub use http::{
    ClientError, Method, Request, ReqwestTransport, Response, Transport, TransportError,
};
pub use mention::{
    MentionTarget, RenderedMention, extract_agent_ids, render_card_markdown, render_text,
};
pub use normalize::{
    NormalizationConfig, NormalizeError, NormalizedAttachment, NormalizedChatUpdate,
    NormalizedEvent, NormalizedMention, NormalizedMessage, NormalizedReaction, NormalizedSender,
    NormalizedTaskEvent, ReactionAction, SenderKind, normalize_chat_update, normalize_event,
    normalize_message, normalize_reaction, normalize_task,
};
pub use onboarding::agent_onboarding;
pub use registration::{
    Registrar as FeishuRegistrar, Registration, RegistrationCredential, RegistrationPoll,
};
pub use tasks::{
    Comment as TaskCommentInput, CreateTask, ListRequest as TaskListRequest, MemberIdentity,
    TaskEventReason, TaskMemberInput, TaskPatch, TasklistMember,
};
pub use ws::{FeishuWebSocketSource, native_event_source_factory};

mod workspace_owner;
mod workspace_tasks;
