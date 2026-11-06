#![forbid(unsafe_code)]

//! Runtime-independent SDK and lifecycle host for messaging Providers.

mod context;
mod error;
mod host;
mod types;

pub use context::*;
pub use error::*;
pub use host::*;
pub use meerkat_core::PROVIDER_CAPABILITY_TASK_SYSTEM;
pub use types::*;

use std::{future::Future, pin::Pin};

pub type ProviderFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

/// Providers own external-platform behavior. Runtime owns scheduling and persistence.
pub trait Provider: Send + Sync + 'static {
    fn info(&self) -> ProviderInfo;
    fn capabilities() -> &'static [&'static str]
    where
        Self: Sized,
    {
        &[]
    }
    fn reaction_catalog(&self) -> Vec<ReactionDefinition> {
        Vec::new()
    }
    fn channels<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>>;
    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()>;
    fn send_message<'a>(
        &'a self,
        context: SendMessageContext,
    ) -> ProviderFuture<'a, SendMessageResult>;

    fn create_workspace<'a>(&'a self, _: WorkspaceContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn update_workspace<'a>(&'a self, _: WorkspaceContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn create_agent<'a>(&'a self, _: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn update_agent<'a>(&'a self, _: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn delete_agent<'a>(&'a self, _: AgentContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn agent_onboarding<'a>(
        &'a self,
        _: AgentOnboardingContext,
        _: String,
        opaque_payload: serde_json::Value,
    ) -> ProviderFuture<'a, ProviderOnboarding> {
        Box::pin(async move { Ok(ProviderOnboarding::ready(opaque_payload)) })
    }
    fn shutdown<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn recall_message<'a>(&'a self, _: RecallMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "recall_message",
            })
        })
    }
    fn edit_message<'a>(&'a self, _: EditMessageContext) -> ProviderFuture<'a, ()> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "edit_message",
            })
        })
    }
    fn auto_reaction<'a>(&'a self, _: AutoReactionContext) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn system_prompt<'a>(&'a self, _: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        Box::pin(async { Ok(String::new()) })
    }
    /// Bounded per-turn references. Never append these changing values to system_prompt.
    fn context_index<'a>(&'a self, _: AgentRuntimeContext) -> ProviderFuture<'a, String> {
        Box::pin(async { Ok(String::new()) })
    }
    fn tools<'a>(&'a self, _: AgentRuntimeContext) -> ProviderFuture<'a, Vec<ToolDefinition>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn cli_commands<'a>(
        &'a self,
        _: AgentRuntimeContext,
    ) -> ProviderFuture<'a, Vec<CliCommandDefinition>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
