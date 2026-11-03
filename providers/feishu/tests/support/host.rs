//! Host services backed by the real SQLite and Runtime resource stores.
#[path = "../../../../tests/support/registered_machine.rs"]
mod registered_machine;
use meerkat_core::{
    AgentId, ChannelId, MessageId, PersonId, ProviderId, ResourceId, ResourceRef, WorkspaceId,
};
use meerkat_provider::*;
use meerkat_runtime::{LocalResourceStore, ResourceStore};
use meerkat_storage::{DatabaseConfig, Storage};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub struct Services {
    pub db: Arc<Storage>,
    pub workspace: WorkspaceId,
    pub logs: std::sync::Mutex<Vec<String>>,
    provider: ProviderId,
    files: LocalResourceStore,
    _directory: tempfile::TempDir,
}

fn error(_: impl std::fmt::Display) -> ProviderError {
    ProviderError::External {
        operation: "scenario_storage",
        retryable: false,
    }
}
fn person(value: meerkat_storage::PersonRecord) -> ProviderPersonIdentity {
    ProviderPersonIdentity {
        person_id: value.person_id,
        external_id: value.external_id,
        team_role: value.team_role,
        provider_state: value.provider_identity_state,
        state_revision: value.provider_state_revision,
    }
}
impl Services {
    pub fn new(provider: &str) -> Arc<Self> {
        let directory = tempfile::tempdir().unwrap();
        let db = Arc::new(
            Storage::open(DatabaseConfig::new(directory.path().join("runtime.sqlite"))).unwrap(),
        );
        registered_machine::register(&db, &["fixture-machine"]).unwrap();
        let workspace = WorkspaceId::new("im_scenarios").unwrap();
        let provider = ProviderId::new(provider).unwrap();
        db.create_workspace(meerkat_storage::WorkspaceSpec {
            id: workspace.clone(),
            name: "IM scenarios".into(),
            provider_id: provider.clone(),
            provider_config: json!({}),
            provider_secret: json!({}),
            active: true,
            now: 1,
        })
        .unwrap();
        Arc::new(Self {
            files: LocalResourceStore::new(db.clone(), directory.path().join("resources")),
            db,
            workspace,
            logs: std::sync::Mutex::new(Vec::new()),
            provider,
            _directory: directory,
        })
    }
    pub fn runtime(&self) -> meerkat_runtime::Runtime {
        meerkat_runtime::Runtime::new(meerkat_runtime::RuntimeConfig {
            database: DatabaseConfig::new(self._directory.path().join("runtime.sqlite")),
            retry: Default::default(),
        })
        .unwrap()
    }
    pub fn agent(&self, id: &AgentId, secret_name: &str, secret: &str) {
        self.db
            .create_agent(meerkat_storage::AgentSpec {
                machine_id: "fixture-machine".into(),
                workspace_id: self.workspace.clone(),
                id: id.clone(),
                name: id.to_string(),
                now: 1,
            })
            .unwrap();
        self.db
            .put_agent_secret(
                &self.workspace,
                id,
                secret_name,
                meerkat_storage::SecretValue::new(secret),
                1,
            )
            .unwrap();
    }
    pub fn host(self: &Arc<Self>) -> HostServices {
        HostServices {
            secret_reader: self.clone(),
            secret_writer: self.clone(),
            storage: self.clone(),
            logger: self.clone(),
            resource_saver: self.clone(),
            operation_progress: self.clone(),
            event_capacity: 16,
            invocation_timeout: Duration::from_secs(30),
        }
    }
    pub fn remember(&self, chat: &str, external: &str) -> MessageId {
        let result = self
            .db
            .receive_message(meerkat_storage::IncomingMessage {
                workspace_id: self.workspace.clone(),
                channel_id: ChannelId::new("group_chat").unwrap(),
                channel_name: "Group".into(),
                provider_chat_id: chat.into(),
                chat_name: "Group".into(),
                provider_message_id: external.into(),
                sender_id: "alice".into(),
                sender_name: "Alice".into(),
                content: "Hello".into(),
                occurred_at: 1,
                now: 1,
            })
            .unwrap();
        MessageId::new(result.message_id).unwrap()
    }
}
impl SecretReader for Services {
    fn read<'a>(&'a self, name: &'a str) -> ProviderFuture<'a, SecretValue> {
        Box::pin(async move {
            self.db
                .read_secret(&self.workspace, name)
                .map_err(error)?
                .map(|value| SecretValue::new(value.expose_secret()))
                .ok_or(ProviderError::SecretAccessDenied)
        })
    }
}
impl SecretWriter for Services {
    fn write<'a>(&'a self, name: &'a str, value: SecretValue) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            let (agent_id, secret_name) = name
                .strip_prefix("agent:")
                .and_then(|scoped| scoped.split_once(':'))
                .ok_or(ProviderError::SecretAccessDenied)?;
            self.db
                .put_agent_secret(
                    &self.workspace,
                    &AgentId::new(agent_id).unwrap(),
                    secret_name,
                    meerkat_storage::SecretValue::new(value.expose_secret()),
                    1,
                )
                .map_err(error)
        })
    }
}
impl ProviderStorage for Services {
    fn get<'a>(&'a self, key: &'a str) -> ProviderFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move {
            self.db
                .provider_kv_get(&self.workspace, self.provider.as_str(), key)
                .map_err(error)
        })
    }
    fn put<'a>(&'a self, key: &'a str, value: Vec<u8>) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.db
                .provider_kv_put(&self.workspace, self.provider.as_str(), key, &value, 1)
                .map_err(error)
        })
    }
    fn workspace_state(&self) -> ProviderFuture<'_, Value> {
        Box::pin(async {
            self.db
                .workspace_provider_state(&self.workspace)
                .map_err(error)
        })
    }
    fn put_workspace_state(&self, value: Value) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            self.db
                .put_workspace_provider_state(&self.workspace, &value)
                .map_err(error)
        })
    }
    fn agent_state<'a>(&'a self, id: &'a AgentId) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            self.db
                .agent_provider_state(&self.workspace, id)
                .map_err(error)
        })
    }
    fn put_agent_state<'a>(&'a self, id: &'a AgentId, value: Value) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            self.db
                .put_agent_provider_state(&self.workspace, id, &value)
                .map_err(error)
        })
    }
    fn message<'a>(&'a self, id: MessageId) -> ProviderFuture<'a, ProviderMessage> {
        Box::pin(async move {
            let value = self
                .db
                .provider_message(&self.workspace, id.get())
                .map_err(error)?
                .ok_or(ProviderError::NotFound { entity: "message" })?;
            Ok(ProviderMessage {
                message_id: id,
                provider_message_id: value.provider_message_id,
                channel_id: value.channel_id,
                chat_id: value.provider_chat_id,
            })
        })
    }
    fn resource<'a>(&'a self, resource: &'a ResourceRef) -> ProviderFuture<'a, ProviderResource> {
        Box::pin(async move {
            let stored = self
                .files
                .read(
                    &self.workspace,
                    ResourceId::new(resource.resource_id).unwrap(),
                )
                .await
                .map_err(error)?;
            assert_eq!(ResourceRef::from(stored.resource.clone()), *resource);
            Ok(ProviderResource {
                resource: stored.resource.into(),
                bytes: stored.bytes,
            })
        })
    }
    fn team_members(&self) -> ProviderFuture<'_, Vec<ProviderTeamMember>> {
        Box::pin(async move {
            let mut members = self
                .db
                .list_human_team_members(&self.workspace)
                .map_err(error)?
                .into_iter()
                .map(|record| ProviderTeamMember::Human(person(record)))
                .collect::<Vec<_>>();
            members.extend(
                self.db
                    .list_agents(&self.workspace)
                    .map_err(error)?
                    .into_iter()
                    .map(|agent| ProviderTeamMember::Agent { agent_id: agent.id }),
            );
            Ok(members)
        })
    }
    fn get_or_create_person<'a>(
        &'a self,
        external: &'a str,
    ) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.db
                .get_or_create_person(&self.workspace, &self.provider, external, 1)
                .map(person)
                .map_err(error)
        })
    }
    fn person<'a>(&'a self, id: &'a PersonId) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.db
                .person(&self.workspace, id)
                .map_err(error)?
                .map(person)
                .ok_or(ProviderError::NotFound { entity: "person" })
        })
    }
    fn compare_and_set_person_state<'a>(
        &'a self,
        id: &'a PersonId,
        revision: i64,
        value: Value,
    ) -> ProviderFuture<'a, ProviderPersonIdentity> {
        Box::pin(async move {
            self.db
                .compare_and_set_person_state(&self.workspace, id, revision, &value, 1)
                .map(person)
                .map_err(error)
        })
    }
}
impl ResourceSaver for Services {
    fn save<'a>(&'a self, upload: ResourceUpload) -> ProviderFuture<'a, ResourceRef> {
        Box::pin(async move {
            self.files
                .save(&self.workspace, upload)
                .await
                .map(Into::into)
                .map_err(error)
        })
    }
}
impl ProviderLogger for Services {
    fn log(&self, level: LogLevel, message: &str) {
        self.logs.lock().unwrap().push(message.to_owned());
        eprintln!("{level:?}: {message}");
    }
}
impl OperationProgress for Services {
    fn report<'a>(&'a self, _: OperationUpdate) -> ProviderFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
pub fn agent_spec(config: Value) -> AgentSpec {
    AgentSpec {
        name: "Bot B".into(),
        role: String::new(),
        bio: String::new(),
        avatar: None,
        soul: String::new(),
        machine_id: "machine".into(),
        harness_id: "pi".into(),
        model: "model".into(),
        thinking_level: "low".into(),
        auto_reaction: None,
        cli_path: None,
        environment_variables: vec![],
        provider_agent_config: config,
    }
}
