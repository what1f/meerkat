use crate::*;
use lankide_core::{AgentId, ProviderId, WorkspaceId, is_canonical_tool_name};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, RwLock, mpsc};

#[derive(Clone, Default)]
struct Registry(Arc<RwLock<BTreeMap<String, InterfaceDefinition>>>);
impl InterfaceRegistrar for Registry {
    fn register<'a>(&'a self, definition: InterfaceDefinition) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if definition.id.trim().is_empty() {
                return Err(ProviderError::InvalidInput {
                    field: "interface_id",
                });
            }
            let mut values = self.0.write().await;
            if values.contains_key(&definition.id) {
                return Err(ProviderError::DuplicateRegistration { id: definition.id });
            }
            values.insert(definition.id.clone(), definition);
            Ok(())
        })
    }
}

#[derive(Clone)]
struct AgentSecrets {
    agent_id: AgentId,
    workspace: Arc<dyn SecretReader>,
}

impl SecretReader for AgentSecrets {
    fn read<'a>(&'a self, name: &'a str) -> ProviderFuture<'a, SecretValue> {
        Box::pin(async move {
            let scoped = format!("agent:{}:{name}", self.agent_id.as_str());
            match self.workspace.read(&scoped).await {
                Ok(value) => Ok(value),
                Err(ProviderError::SecretAccessDenied) => self.workspace.read(name).await,
                Err(error) => Err(error),
            }
        })
    }
}

pub struct ProviderHost<P: Provider> {
    provider: Arc<P>,
    workspace_id: WorkspaceId,
    workspace_config: Value,
    services: HostServices,
    shutdown: ShutdownSignal,
    sink: EventSink,
    receiver: Mutex<mpsc::Receiver<EventDelivery>>,
    interfaces: Registry,
    initialized: AtomicBool,
    ready: AtomicBool,
}

mod sealed {
    pub trait Sealed {}
}

/// Type-erased Provider host interface consumed by Runtime.
///
/// Provider owns this interface because it owns the lifecycle, context
/// construction, capability validation, and invocation rules behind it.
pub trait ProviderHostInstance: sealed::Sealed + Send + Sync {
    fn provider_id(&self) -> ProviderId;
    fn reaction_catalog(&self) -> Vec<ReactionDefinition>;
    fn init(&self) -> ProviderFuture<'_, ()>;
    fn channels(&self) -> ProviderFuture<'_, Vec<ChannelDefinition>>;
    fn create_workspace(&self, spec: WorkspaceSpec) -> ProviderFuture<'_, ()>;
    fn update_workspace(
        &self,
        old_provider_config: Value,
        spec: WorkspaceSpec,
    ) -> ProviderFuture<'_, ()>;
    fn create_agent(&self, agent_id: AgentId, spec: AgentSpec) -> ProviderFuture<'_, ()>;
    fn agent_onboarding(
        &self,
        agent_id: AgentId,
        name: String,
        provider_agent_config: Value,
        action: String,
        opaque_payload: Value,
    ) -> ProviderFuture<'_, ProviderOnboarding>;
    fn update_agent(
        &self,
        agent_id: AgentId,
        old_provider_agent_config: Value,
        spec: AgentSpec,
    ) -> ProviderFuture<'_, ()>;
    fn delete_agent(&self, agent_id: AgentId, spec: AgentSpec) -> ProviderFuture<'_, ()>;
    fn next_event(&self) -> Pin<Box<dyn Future<Output = Option<EventDelivery>> + Send + '_>>;
    fn system_prompt(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, String>;
    fn context_index(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, String>;
    fn tools(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, Vec<ToolDefinition>>;
    fn cli_commands(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, Vec<CliCommandDefinition>>;
    fn send_message(
        &self,
        agent_id: AgentId,
        spec: SendMessageSpec,
    ) -> ProviderFuture<'_, SendMessageResult>;
    fn recall_message(&self, agent_id: AgentId, spec: RecallMessageSpec) -> ProviderFuture<'_, ()>;
    fn edit_message(&self, agent_id: AgentId, spec: EditMessageSpec) -> ProviderFuture<'_, ()>;
    fn invoke_tool(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
        id: String,
        input: Value,
    ) -> ProviderFuture<'_, Value>;
    fn invoke_cli(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
        id: String,
        arguments: Vec<String>,
    ) -> ProviderFuture<'_, Value>;
    fn invoke_interface(&self, id: String, input: Value) -> ProviderFuture<'_, Value>;
    fn list_interfaces(&self) -> ProviderFuture<'_, Vec<InterfaceCatalogEntry>>;
    fn auto_reaction(&self, agent_id: AgentId, spec: AutoReactionSpec) -> ProviderFuture<'_, ()>;
    fn shutdown(&self) -> ProviderFuture<'_, ()>;
}

impl<P: Provider> ProviderHost<P> {
    fn agent_secrets(&self, agent_id: &AgentId) -> Arc<dyn SecretReader> {
        Arc::new(AgentSecrets {
            agent_id: agent_id.clone(),
            workspace: self.services.secret_reader.clone(),
        })
    }
    pub fn new(
        provider: P,
        workspace_id: WorkspaceId,
        workspace_config: Value,
        services: HostServices,
    ) -> Result<Self, ProviderError> {
        if services.event_capacity == 0 {
            return Err(ProviderError::InvalidInput {
                field: "event_capacity",
            });
        }
        let shutdown = ShutdownSignal::new();
        let (sender, receiver) = mpsc::channel(services.event_capacity);
        let sink = EventSink {
            sender,
            shutdown: shutdown.clone(),
        };
        Ok(Self {
            provider: Arc::new(provider),
            workspace_id,
            workspace_config,
            services,
            shutdown,
            sink,
            receiver: Mutex::new(receiver),
            interfaces: Registry::default(),
            initialized: AtomicBool::new(false),
            ready: AtomicBool::new(false),
        })
    }
    pub fn reaction_catalog(&self) -> Vec<crate::ReactionDefinition> {
        self.provider.reaction_catalog()
    }
    pub fn info(&self) -> ProviderInfo {
        self.provider.info()
    }
    fn init_context(&self) -> InitContext {
        InitContext {
            workspace_id: self.workspace_id.clone(),
            workspace_config: self.workspace_config.clone(),
            secret_reader: self.services.secret_reader.clone(),
            secret_writer: self.services.secret_writer.clone(),
            storage: self.services.storage.clone(),
            logger: self.services.logger.clone(),
            shutdown: self.shutdown.clone(),
            sink: self.sink.clone(),
            registrar: Arc::new(self.interfaces.clone()),
            resource_saver: self.services.resource_saver.clone(),
        }
    }
    pub async fn channels(&self) -> Result<Vec<ChannelDefinition>, ProviderError> {
        self.provider.channels(self.init_context()).await
    }
    fn workspace_context(&self, old: Option<Value>, spec: WorkspaceSpec) -> WorkspaceContext {
        WorkspaceContext {
            workspace_id: self.workspace_id.clone(),
            name: spec.name,
            cover: spec.cover,
            rules: spec.rules,
            old_provider_config: old,
            new_provider_config: spec.provider_config,
            secrets: self.services.secret_reader.clone(),
            storage: self.services.storage.clone(),
            progress: self.services.operation_progress.clone(),
        }
    }
    pub async fn create_workspace(&self, spec: WorkspaceSpec) -> Result<(), ProviderError> {
        self.provider
            .create_workspace(self.workspace_context(None, spec))
            .await
    }
    pub async fn update_workspace(
        &self,
        old: Value,
        spec: WorkspaceSpec,
    ) -> Result<(), ProviderError> {
        self.provider
            .update_workspace(self.workspace_context(Some(old), spec))
            .await
    }
    fn agent_context(&self, id: AgentId, old: Option<Value>, spec: AgentSpec) -> AgentContext {
        let secrets = self.agent_secrets(&id);
        AgentContext {
            workspace_id: self.workspace_id.clone(),
            workspace_config: self.workspace_config.clone(),
            agent_id: id,
            name: spec.name,
            role: spec.role,
            bio: spec.bio,
            avatar: spec.avatar,
            soul: spec.soul,
            machine_id: spec.machine_id,
            harness_id: spec.harness_id,
            model: spec.model,
            thinking_level: spec.thinking_level,
            auto_reaction: spec.auto_reaction,
            cli_path: spec.cli_path,
            environment_variables: spec.environment_variables,
            old_provider_agent_config: old,
            new_provider_agent_config: spec.provider_agent_config,
            secrets,
            storage: self.services.storage.clone(),
            progress: self.services.operation_progress.clone(),
        }
    }
    pub async fn create_agent(&self, id: AgentId, spec: AgentSpec) -> Result<(), ProviderError> {
        self.provider
            .create_agent(self.agent_context(id, None, spec))
            .await
    }
    pub async fn agent_onboarding(
        &self,
        id: AgentId,
        name: String,
        provider_agent_config: Value,
        action: String,
        opaque_payload: Value,
    ) -> Result<ProviderOnboarding, ProviderError> {
        self.provider
            .agent_onboarding(
                AgentOnboardingContext::new(
                    self.workspace_id.clone(),
                    self.workspace_config.clone(),
                    id,
                    name,
                    provider_agent_config,
                    self.services.clone(),
                ),
                action,
                opaque_payload,
            )
            .await
    }
    pub async fn update_agent(
        &self,
        id: AgentId,
        old: Value,
        spec: AgentSpec,
    ) -> Result<(), ProviderError> {
        self.provider
            .update_agent(self.agent_context(id, Some(old), spec))
            .await
    }
    pub async fn delete_agent(&self, id: AgentId, spec: AgentSpec) -> Result<(), ProviderError> {
        self.provider
            .delete_agent(self.agent_context(id, None, spec))
            .await
    }
    pub async fn init(&self) -> Result<(), ProviderError> {
        if self.initialized.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if let Err(error) = self.provider.init(self.init_context()).await {
            self.initialized.store(false, Ordering::Release);
            return Err(error);
        }
        self.ready.store(true, Ordering::Release);
        Ok(())
    }
    pub async fn next_event(&self) -> Option<EventDelivery> {
        self.receiver.lock().await.recv().await
    }
    pub async fn invoke_interface(&self, id: &str, input: Value) -> Result<Value, ProviderError> {
        let definition = self
            .interfaces
            .0
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownRegistration { id: id.into() })?;
        invoke(
            "custom_interface",
            &definition.input_schema,
            &definition.output_schema,
            definition.handler.as_ref(),
            input,
            self.services.invocation_timeout,
        )
        .await
    }

    /// Lists stable, public metadata for interfaces registered during Provider init.
    pub async fn list_interfaces(&self) -> Result<Vec<InterfaceCatalogEntry>, ProviderError> {
        if !self.ready.load(Ordering::Acquire) || self.shutdown.is_shutdown() {
            return Err(ProviderError::NotFound {
                entity: "workspace",
            });
        }
        Ok(self
            .interfaces
            .0
            .read()
            .await
            .values()
            .map(|definition| InterfaceCatalogEntry {
                id: definition.id.clone(),
                description: None,
                input_schema: definition.input_schema.value().clone(),
                output_schema: definition.output_schema.value().clone(),
            })
            .collect())
    }
    fn runtime_context(&self, id: AgentId, spec: AgentRuntimeSpec) -> AgentRuntimeContext {
        let secrets = self.agent_secrets(&id);
        AgentRuntimeContext {
            workspace_id: self.workspace_id.clone(),
            workspace_config: self.workspace_config.clone(),
            agent_id: id,
            agent_config: spec.agent_config,
            provider_agent_config: spec.provider_agent_config,
            machine_id: spec.machine_id,
            harness_id: spec.harness_id,
            model: spec.model,
            thinking_level: spec.thinking_level,
            trigger_reason: spec.trigger_reason,
            chats: spec.chats,
            unread_snapshot: spec.unread_snapshot,
            secrets,
            storage: self.services.storage.clone(),
        }
    }
    pub async fn system_prompt(
        &self,
        id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> Result<String, ProviderError> {
        self.provider
            .system_prompt(self.runtime_context(id, spec))
            .await
    }
    pub async fn context_index(
        &self,
        id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> Result<String, ProviderError> {
        self.provider
            .context_index(self.runtime_context(id, spec))
            .await
    }
    pub async fn tools(
        &self,
        id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> Result<Vec<ToolDefinition>, ProviderError> {
        let definitions = self.provider.tools(self.runtime_context(id, spec)).await?;
        validate_unique(&definitions, |value| &value.id)?;
        if definitions
            .iter()
            .any(|definition| !is_canonical_tool_name(&definition.id))
        {
            return Err(ProviderError::InvalidInput {
                field: "capability_id",
            });
        }
        Ok(definitions)
    }

    /// Resolves and invokes a Tool from the same Agent execution context.
    ///
    /// Dynamic definitions are deliberately not cached on the Workspace host:
    /// two Agents in the same Workspace may have different permissions and
    /// handlers for the same Provider capability.
    pub async fn invoke_tool_in_context(
        &self,
        id: AgentId,
        spec: AgentRuntimeSpec,
        tool_id: &str,
        input: Value,
    ) -> Result<Value, ProviderError> {
        let definitions = self.provider.tools(self.runtime_context(id, spec)).await?;
        let definition = find_unique(&definitions, tool_id, |value| &value.id)?;
        invoke(
            "tool",
            &definition.input_schema,
            &definition.output_schema,
            definition.handler.as_ref(),
            input,
            self.services.invocation_timeout,
        )
        .await
    }

    pub async fn cli_commands(
        &self,
        id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> Result<Vec<CliCommandDefinition>, ProviderError> {
        let definitions = self
            .provider
            .cli_commands(self.runtime_context(id, spec))
            .await?;
        validate_unique(&definitions, |value| &value.id)?;
        Ok(definitions)
    }

    pub async fn invoke_cli_in_context(
        &self,
        id: AgentId,
        spec: AgentRuntimeSpec,
        command_id: &str,
        arguments: Vec<String>,
    ) -> Result<Value, ProviderError> {
        let definitions = self
            .provider
            .cli_commands(self.runtime_context(id, spec))
            .await?;
        let definition = find_unique(&definitions, command_id, |value| &value.id)?;
        let input = Value::Array(arguments.into_iter().map(Value::String).collect());
        match tokio::time::timeout(
            self.services.invocation_timeout,
            definition.handler.invoke(input),
        )
        .await
        {
            Ok(value) => value,
            Err(_) => Err(ProviderError::Timeout { operation: "cli" }),
        }
    }

    pub async fn send_message(
        &self,
        id: AgentId,
        spec: SendMessageSpec,
    ) -> Result<SendMessageResult, ProviderError> {
        let secrets = self.agent_secrets(&id);
        self.provider
            .send_message(SendMessageContext {
                workspace_id: self.workspace_id.clone(),
                agent_id: id,
                channel_id: spec.channel_id,
                chat_id: spec.chat_id,
                message_id: spec.message_id,
                idempotency_key: spec.idempotency_key,
                content: spec.content,
                reply_to: spec.reply_to,
                mentions: spec.mentions,
                attachments: spec.attachments,
                force: spec.force,
                provider_config: self.workspace_config.clone(),
                provider_agent_config: spec.provider_agent_config,
                secrets,
                storage: self.services.storage.clone(),
            })
            .await
    }
    pub async fn recall_message(
        &self,
        id: AgentId,
        spec: RecallMessageSpec,
    ) -> Result<(), ProviderError> {
        let secrets = self.agent_secrets(&id);
        self.provider
            .recall_message(RecallMessageContext {
                workspace_id: self.workspace_id.clone(),
                agent_id: id,
                channel_id: spec.channel_id,
                chat_id: spec.chat_id,
                message_id: spec.message_id,
                provider_message_id: spec.provider_message_id,
                provider_config: self.workspace_config.clone(),
                provider_agent_config: spec.provider_agent_config,
                secrets,
                storage: self.services.storage.clone(),
            })
            .await
    }
    pub async fn edit_message(
        &self,
        id: AgentId,
        spec: EditMessageSpec,
    ) -> Result<(), ProviderError> {
        let secrets = self.agent_secrets(&id);
        self.provider
            .edit_message(EditMessageContext {
                workspace_id: self.workspace_id.clone(),
                agent_id: id,
                channel_id: spec.channel_id,
                chat_id: spec.chat_id,
                message_id: spec.message_id,
                provider_message_id: spec.provider_message_id,
                content: spec.content,
                provider_config: self.workspace_config.clone(),
                provider_agent_config: spec.provider_agent_config,
                secrets,
                storage: self.services.storage.clone(),
            })
            .await
    }
    pub async fn auto_reaction(
        &self,
        id: AgentId,
        spec: AutoReactionSpec,
    ) -> Result<(), ProviderError> {
        let secrets = self.agent_secrets(&id);
        self.provider
            .auto_reaction(AutoReactionContext {
                workspace_id: self.workspace_id.clone(),
                agent_id: id,
                auto_reaction: spec.auto_reaction,
                channel_id: spec.channel_id,
                chat_id: spec.chat_id,
                trigger_messages: spec.trigger_messages,
                provider_config: self.workspace_config.clone(),
                provider_agent_config: spec.provider_agent_config,
                secrets,
                storage: self.services.storage.clone(),
            })
            .await
    }
    pub async fn shutdown(&self) -> Result<(), ProviderError> {
        if self.shutdown.is_shutdown() {
            return Ok(());
        }
        let result = self.provider.shutdown(self.init_context()).await;
        self.shutdown.shutdown();
        self.ready.store(false, Ordering::Release);
        self.receiver.lock().await.close();
        self.interfaces.0.write().await.clear();
        result
    }
}

impl<P: Provider> sealed::Sealed for ProviderHost<P> {}

impl<P: Provider> ProviderHostInstance for ProviderHost<P> {
    fn reaction_catalog(&self) -> Vec<ReactionDefinition> {
        ProviderHost::reaction_catalog(self)
    }

    fn provider_id(&self) -> ProviderId {
        self.info().id
    }

    fn init(&self) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::init(self))
    }

    fn channels(&self) -> ProviderFuture<'_, Vec<ChannelDefinition>> {
        Box::pin(ProviderHost::channels(self))
    }

    fn create_workspace(&self, spec: WorkspaceSpec) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::create_workspace(self, spec))
    }

    fn update_workspace(
        &self,
        old_provider_config: Value,
        spec: WorkspaceSpec,
    ) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::update_workspace(
            self,
            old_provider_config,
            spec,
        ))
    }

    fn create_agent(&self, agent_id: AgentId, spec: AgentSpec) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::create_agent(self, agent_id, spec))
    }

    fn agent_onboarding(
        &self,
        agent_id: AgentId,
        name: String,
        provider_agent_config: Value,
        action: String,
        opaque_payload: Value,
    ) -> ProviderFuture<'_, ProviderOnboarding> {
        Box::pin(ProviderHost::agent_onboarding(
            self,
            agent_id,
            name,
            provider_agent_config,
            action,
            opaque_payload,
        ))
    }

    fn update_agent(
        &self,
        agent_id: AgentId,
        old_provider_agent_config: Value,
        spec: AgentSpec,
    ) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::update_agent(
            self,
            agent_id,
            old_provider_agent_config,
            spec,
        ))
    }

    fn delete_agent(&self, agent_id: AgentId, spec: AgentSpec) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::delete_agent(self, agent_id, spec))
    }

    fn next_event(&self) -> Pin<Box<dyn Future<Output = Option<EventDelivery>> + Send + '_>> {
        Box::pin(ProviderHost::next_event(self))
    }

    fn system_prompt(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, String> {
        Box::pin(ProviderHost::system_prompt(self, agent_id, spec))
    }

    fn context_index(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, String> {
        Box::pin(ProviderHost::context_index(self, agent_id, spec))
    }
    fn tools(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, Vec<ToolDefinition>> {
        Box::pin(ProviderHost::tools(self, agent_id, spec))
    }

    fn cli_commands(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
    ) -> ProviderFuture<'_, Vec<CliCommandDefinition>> {
        Box::pin(ProviderHost::cli_commands(self, agent_id, spec))
    }

    fn send_message(
        &self,
        agent_id: AgentId,
        spec: SendMessageSpec,
    ) -> ProviderFuture<'_, SendMessageResult> {
        Box::pin(ProviderHost::send_message(self, agent_id, spec))
    }

    fn recall_message(&self, agent_id: AgentId, spec: RecallMessageSpec) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::recall_message(self, agent_id, spec))
    }

    fn edit_message(&self, agent_id: AgentId, spec: EditMessageSpec) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::edit_message(self, agent_id, spec))
    }

    fn invoke_tool(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
        id: String,
        input: Value,
    ) -> ProviderFuture<'_, Value> {
        Box::pin(async move {
            ProviderHost::invoke_tool_in_context(self, agent_id, spec, &id, input).await
        })
    }

    fn invoke_cli(
        &self,
        agent_id: AgentId,
        spec: AgentRuntimeSpec,
        id: String,
        arguments: Vec<String>,
    ) -> ProviderFuture<'_, Value> {
        Box::pin(async move {
            ProviderHost::invoke_cli_in_context(self, agent_id, spec, &id, arguments).await
        })
    }

    fn invoke_interface(&self, id: String, input: Value) -> ProviderFuture<'_, Value> {
        Box::pin(async move { ProviderHost::invoke_interface(self, &id, input).await })
    }

    fn list_interfaces(&self) -> ProviderFuture<'_, Vec<InterfaceCatalogEntry>> {
        Box::pin(ProviderHost::list_interfaces(self))
    }

    fn auto_reaction(&self, agent_id: AgentId, spec: AutoReactionSpec) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::auto_reaction(self, agent_id, spec))
    }

    fn shutdown(&self) -> ProviderFuture<'_, ()> {
        Box::pin(ProviderHost::shutdown(self))
    }
}

fn validate_unique<T>(values: &[T], id: impl Fn(&T) -> &String) -> Result<(), ProviderError> {
    let mut ids = std::collections::BTreeSet::new();
    for value in values {
        let key = id(value).clone();
        if key.trim().is_empty() {
            return Err(ProviderError::InvalidInput {
                field: "capability_id",
            });
        }
        if !ids.insert(key.clone()) {
            return Err(ProviderError::DuplicateRegistration { id: key });
        }
    }
    Ok(())
}

fn find_unique<'a, T>(
    values: &'a [T],
    expected: &str,
    id: impl Fn(&T) -> &String,
) -> Result<&'a T, ProviderError> {
    validate_unique(values, &id)?;
    values
        .iter()
        .find(|value| id(value) == expected)
        .ok_or_else(|| ProviderError::UnknownRegistration {
            id: expected.to_owned(),
        })
}
async fn invoke(
    operation: &'static str,
    input_schema: &JsonSchema,
    output_schema: &JsonSchema,
    handler: &dyn InvocationHandler,
    input: Value,
    timeout: Duration,
) -> Result<Value, ProviderError> {
    input_schema.validate(&input)?;
    let output = match tokio::time::timeout(timeout, handler.invoke(input)).await {
        Ok(result) => result?,
        Err(_) => return Err(ProviderError::Timeout { operation }),
    };
    output_schema
        .validate(&output)
        .map_err(|_| ProviderError::SchemaMismatch { location: "output" })?;
    Ok(output)
}
