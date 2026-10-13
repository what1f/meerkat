use lankide_core::AgentId;
use lankide_provider::{ProviderError, ProviderStorage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const STATE_KEY: &str = "identity_directory";

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct IdentityDirectory {
    #[serde(default)]
    pub agents: BTreeMap<String, AgentIdentity>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
pub(crate) struct AgentIdentity {
    pub account_id: i64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub display_name: String,
}

impl IdentityDirectory {
    fn from_workspace_state(state: &Value) -> Result<Self, ProviderError> {
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

    fn write_to_workspace_state(&self, state: &mut Value) -> Result<(), ProviderError> {
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
        account_id: i64,
        username: &str,
        display_name: &str,
    ) {
        self.agents.insert(
            agent_id.to_string(),
            AgentIdentity {
                account_id,
                username: username.trim().trim_start_matches('@').to_owned(),
                display_name: display_name.trim().to_owned(),
            },
        );
    }

    pub fn remove_agent(&mut self, agent_id: &AgentId) {
        self.agents.remove(agent_id.as_str());
    }

    pub fn normalization_config(
        &self,
        bot: crate::dto::User,
    ) -> crate::normalize::NormalizationConfig {
        let mut config =
            crate::normalize::NormalizationConfig::new("telegram", bot.id.to_string(), bot);
        for (agent_id, identity) in &self.agents {
            config = config.with_agent(identity.account_id, agent_id);
        }
        config
    }

    pub fn mention_target(
        &self,
        agent_id: &AgentId,
    ) -> Option<(Option<i64>, Option<String>, String)> {
        let identity = self.agents.get(agent_id.as_str())?;
        let display_name = if identity.display_name.trim().is_empty() {
            agent_id.to_string()
        } else {
            identity.display_name.clone()
        };
        Some((
            (identity.account_id != 0).then_some(identity.account_id),
            (!identity.username.trim().is_empty()).then(|| identity.username.clone()),
            display_name,
        ))
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
