use crate::client::{Auth, Client as FeishuClient};
use crate::http::Transport;
use crate::normalize::{
    NormalizationConfig, NormalizedMention, NormalizedMessage, NormalizedSender, SenderKind,
};
use lankide_core::AgentId;
use lankide_provider::{ProviderError, ProviderPersonIdentity, ProviderStorage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

const STATE_KEY: &str = "identity_directory";

/// Feishu identities are scoped by both tenant and observing App. The stable
/// Lankide Agent ID remains the only identity that leaves this Provider.
#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct IdentityDirectory {
    pub agents: BTreeMap<String, AgentIdentity>,
    unresolved_bots: BTreeMap<String, BotObservation>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct AgentIdentity {
    pub app_id: String,
    pub tenant_key: String,
    pub union_id: String,
    pub user_id: String,
    pub open_ids_by_app: BTreeMap<String, String>,
    pub display_name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
struct BotObservation {
    #[serde(default)]
    bot_app_id: String,
    observer_app_id: String,
    tenant_key: String,
    open_id: String,
    union_id: String,
    user_id: String,
    display_name: String,
}

impl IdentityDirectory {
    pub fn from_workspace_state(state: &Value) -> Result<Self, ProviderError> {
        state
            .get(STATE_KEY)
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| ProviderError::InvalidInput {
                field: "workspace_provider_state.identity_directory",
            })
            .map(|value| value.unwrap_or_default())
    }

    pub fn write_to_workspace_state(&self, state: &mut Value) -> Result<(), ProviderError> {
        let object = state.as_object_mut().ok_or(ProviderError::InvalidInput {
            field: "workspace_provider_state",
        })?;
        object.insert(
            STATE_KEY.to_owned(),
            serde_json::to_value(self).map_err(|_| ProviderError::InvalidInput {
                field: "workspace_provider_state.identity_directory",
            })?,
        );
        Ok(())
    }

    pub fn register_agent(
        &mut self,
        agent_id: &AgentId,
        app_id: &str,
        own_open_id: Option<&str>,
        display_name: &str,
    ) {
        let agent = self.agents.entry(agent_id.to_string()).or_default();
        agent.app_id = app_id.trim().to_owned();
        if !display_name.trim().is_empty() {
            agent.display_name = display_name.trim().to_owned();
        }
        if let Some(open_id) = non_empty(own_open_id.unwrap_or_default()) {
            agent
                .open_ids_by_app
                .insert(app_id.trim().to_owned(), open_id.to_owned());
        }
        self.reconcile_observations();
    }

    pub fn remove_agent(&mut self, agent_id: &AgentId) {
        self.agents.remove(agent_id.as_str());
    }

    /// Learn typed identities from one callback before the event is exposed to
    /// Runtime. Resolve bots by their stable App ID or by previously observed
    /// typed aliases; never infer an identity from a display name.
    pub fn learn_message(&mut self, message: &NormalizedMessage) {
        self.learn_sender(&message.sender, &message.app_id, &message.tenant_key);
        for mention in &message.mentions {
            if !matches!(mention.kind, SenderKind::Agent | SenderKind::Bot) {
                continue;
            }
            let aliases = AliasSet::from_mention(mention);
            let resolved = mention.agent_id.clone().or_else(|| {
                self.resolve_agent(
                    &message.app_id,
                    &message.tenant_key,
                    &aliases.open_id,
                    &aliases.union_id,
                    &aliases.user_id,
                )
            });
            if let Some(agent_id) = resolved {
                self.bind_observation(
                    &agent_id,
                    &message.app_id,
                    &message.tenant_key,
                    &aliases,
                    &mention.display_name,
                );
            } else {
                let observation = BotObservation {
                    bot_app_id: aliases.bot_app_id,
                    observer_app_id: message.app_id.trim().to_owned(),
                    tenant_key: message.tenant_key.trim().to_owned(),
                    open_id: aliases.open_id,
                    union_id: aliases.union_id,
                    user_id: aliases.user_id,
                    display_name: mention.display_name.trim().to_owned(),
                };
                if let Some(key) = observation_key(&observation) {
                    self.unresolved_bots.insert(key, observation);
                }
            }
        }
        self.reconcile_observations();
    }

    fn learn_sender(&mut self, sender: &NormalizedSender, app_id: &str, tenant_key: &str) {
        if !matches!(sender.kind, SenderKind::Agent | SenderKind::Bot) {
            return;
        }
        let aliases = AliasSet::from_aliases(&sender.aliases);
        let resolved = matches!(sender.kind, SenderKind::Agent)
            .then(|| sender.sender_id.clone())
            .or_else(|| {
                self.agents.iter().find_map(|(id, agent)| {
                    (!aliases.bot_app_id.is_empty()
                        && agent.app_id == aliases.bot_app_id
                        && same_tenant(&agent.tenant_key, tenant_key))
                    .then(|| id.clone())
                })
            })
            .or_else(|| {
                self.resolve_agent(
                    app_id,
                    tenant_key,
                    &aliases.open_id,
                    &aliases.union_id,
                    &aliases.user_id,
                )
            });
        if let Some(agent_id) = resolved {
            self.bind_observation(
                &agent_id,
                app_id,
                tenant_key,
                &aliases,
                &sender.display_name,
            );
            return;
        }
        let observation = BotObservation {
            bot_app_id: aliases.bot_app_id,
            observer_app_id: app_id.trim().to_owned(),
            tenant_key: tenant_key.trim().to_owned(),
            open_id: aliases.open_id,
            union_id: aliases.union_id,
            user_id: aliases.user_id,
            display_name: sender.display_name.trim().to_owned(),
        };
        if let Some(key) = observation_key(&observation) {
            self.unresolved_bots.insert(key, observation);
        }
    }

    pub fn normalization_config(&self, app_id: &str, tenant_key: &str) -> NormalizationConfig {
        let mut config = NormalizationConfig::new(app_id);
        for (agent_id, identity) in &self.agents {
            if !same_tenant(&identity.tenant_key, tenant_key) {
                continue;
            }
            config = config.with_agent(&identity.app_id, agent_id);
            if let Some(open_id) = identity.open_ids_by_app.get(app_id) {
                config = config.with_agent(open_id, agent_id);
            }
            for value in [&identity.union_id, &identity.user_id] {
                if !value.trim().is_empty() {
                    config = config.with_agent(value, agent_id);
                }
            }
        }
        config
    }

    pub fn mention_target(
        &self,
        sender_app_id: &str,
        agent_id: &AgentId,
    ) -> Option<(String, String)> {
        let identity = self.agents.get(agent_id.as_str())?;
        let open_id = identity.open_ids_by_app.get(sender_app_id)?.trim();
        if open_id.is_empty() {
            return None;
        }
        let display_name = if identity.display_name.trim().is_empty() {
            "Agent".to_owned()
        } else {
            identity.display_name.clone()
        };
        Some((open_id.to_owned(), display_name))
    }

    pub fn agent_app_id(&self, agent_id: &AgentId) -> Option<&str> {
        self.agents
            .get(agent_id.as_str())
            .and_then(|identity| non_empty(&identity.app_id))
    }

    fn resolve_agent(
        &self,
        observer_app_id: &str,
        tenant_key: &str,
        open_id: &str,
        union_id: &str,
        user_id: &str,
    ) -> Option<String> {
        self.agents.iter().find_map(|(agent_id, identity)| {
            if !same_tenant(&identity.tenant_key, tenant_key) {
                return None;
            }
            let exact_open = non_empty(open_id).is_some_and(|open_id| {
                identity
                    .open_ids_by_app
                    .get(observer_app_id)
                    .is_some_and(|known| known == open_id)
            });
            let same_union = non_empty(union_id).is_some_and(|value| identity.union_id == value);
            let same_user = non_empty(user_id).is_some_and(|value| identity.user_id == value);
            (exact_open || same_union || same_user).then(|| agent_id.clone())
        })
    }

    fn bind_observation(
        &mut self,
        agent_id: &str,
        observer_app_id: &str,
        tenant_key: &str,
        aliases: &AliasSet,
        display_name: &str,
    ) {
        let Some(identity) = self.agents.get_mut(agent_id) else {
            return;
        };
        if identity.tenant_key.trim().is_empty() {
            identity.tenant_key = tenant_key.trim().to_owned();
        }
        if !same_tenant(&identity.tenant_key, tenant_key) {
            return;
        }
        if !aliases.open_id.trim().is_empty() {
            identity
                .open_ids_by_app
                .insert(observer_app_id.trim().to_owned(), aliases.open_id.clone());
        }
        if identity.union_id.trim().is_empty() && !aliases.union_id.trim().is_empty() {
            identity.union_id = aliases.union_id.clone();
        }
        if identity.user_id.trim().is_empty() && !aliases.user_id.trim().is_empty() {
            identity.user_id = aliases.user_id.clone();
        }
        if identity.display_name.trim().is_empty() && !display_name.trim().is_empty() {
            identity.display_name = display_name.trim().to_owned();
        }
    }

    fn reconcile_observations(&mut self) {
        let matches = self
            .unresolved_bots
            .iter()
            .filter_map(|(key, observation)| {
                let agent_id = self
                    .agents
                    .iter()
                    .find_map(|(id, agent)| {
                        (!observation.bot_app_id.is_empty()
                            && agent.app_id == observation.bot_app_id
                            && same_tenant(&agent.tenant_key, &observation.tenant_key))
                        .then(|| id.clone())
                    })
                    .or_else(|| {
                        self.resolve_agent(
                            &observation.observer_app_id,
                            &observation.tenant_key,
                            &observation.open_id,
                            &observation.union_id,
                            &observation.user_id,
                        )
                    })?;
                Some((key.clone(), agent_id, observation.clone()))
            })
            .collect::<Vec<_>>();
        for (key, agent_id, observation) in matches {
            self.bind_observation(
                &agent_id,
                &observation.observer_app_id,
                &observation.tenant_key,
                &AliasSet {
                    bot_app_id: observation.bot_app_id,
                    open_id: observation.open_id,
                    union_id: observation.union_id,
                    user_id: observation.user_id,
                },
                &observation.display_name,
            );
            self.unresolved_bots.remove(&key);
        }
    }
}

pub(crate) async fn load(
    storage: &dyn ProviderStorage,
) -> Result<(Value, IdentityDirectory), ProviderError> {
    let state = storage.workspace_state().await?;
    let directory = IdentityDirectory::from_workspace_state(&state)?;
    Ok((state, directory))
}

pub(crate) async fn save(
    storage: &dyn ProviderStorage,
    mut state: Value,
    directory: &IdentityDirectory,
) -> Result<(), ProviderError> {
    directory.write_to_workspace_state(&mut state)?;
    storage.put_workspace_state(state).await
}

pub(crate) async fn person_open_id(
    storage: &dyn ProviderStorage,
    mut person: ProviderPersonIdentity,
    app_id: &str,
    client: Arc<FeishuClient<Arc<dyn Transport>>>,
    auth: Auth,
) -> Result<String, ProviderError> {
    for _ in 0..3 {
        if let Some(open_id) = person
            .provider_state
            .get("open_ids_by_app")
            .and_then(Value::as_object)
            .and_then(|values| values.get(app_id))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(open_id.to_owned());
        }
        let id_type = person
            .provider_state
            .get("external_id_type")
            .and_then(Value::as_str)
            .unwrap_or("union_id")
            .to_owned();
        let external_id = person.external_id.clone();
        let lookup_client = client.clone();
        let user = lookup_client
            .get_user_by_id(&external_id, &id_type, auth)
            .await
            .map_err(|_| ProviderError::External {
                operation: "resolve_feishu_person_identity",
                retryable: true,
            })?;
        if user.open_id.trim().is_empty() {
            return Err(ProviderError::NotFound {
                entity: "person_identity",
            });
        }
        let mut state = person
            .provider_state
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut open_ids = state
            .remove("open_ids_by_app")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        open_ids.insert(app_id.to_owned(), serde_json::json!(user.open_id));
        state.insert("open_ids_by_app".to_owned(), Value::Object(open_ids));
        match storage
            .compare_and_set_person_state(
                &person.person_id,
                person.state_revision,
                Value::Object(state),
            )
            .await
        {
            Ok(updated) => person = updated,
            Err(_) => person = storage.person(&person.person_id).await?,
        }
    }
    Err(ProviderError::NotFound {
        entity: "person_identity",
    })
}

#[derive(Default)]
struct AliasSet {
    bot_app_id: String,
    open_id: String,
    union_id: String,
    user_id: String,
}

impl AliasSet {
    fn from_mention(mention: &NormalizedMention) -> Self {
        Self::from_aliases(&mention.aliases)
    }

    fn from_aliases(values: &[(String, String)]) -> Self {
        let mut aliases = Self::default();
        for (kind, value) in values {
            match kind.as_str() {
                "app_id" => aliases.bot_app_id = value.trim().to_owned(),
                "open_id" => aliases.open_id = value.trim().to_owned(),
                "union_id" => aliases.union_id = value.trim().to_owned(),
                "user_id" => aliases.user_id = value.trim().to_owned(),
                _ => {}
            }
        }
        aliases
    }
}

fn observation_key(observation: &BotObservation) -> Option<String> {
    let correlation = non_empty(&observation.union_id)
        .or_else(|| non_empty(&observation.open_id))
        .or_else(|| non_empty(&observation.user_id))?;
    Some(format!(
        "{}:{}:{}:{}",
        observation.tenant_key, observation.observer_app_id, observation.open_id, correlation
    ))
}

fn same_tenant(known: &str, observed: &str) -> bool {
    known.trim().is_empty() || observed.trim().is_empty() || known.trim() == observed.trim()
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then(|| value.trim())
}
