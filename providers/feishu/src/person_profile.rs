use crate::client::{Auth, Client as FeishuClient};
use crate::http::Transport;
use meerkat_core::PersonId;
use meerkat_provider::{
    AgentRuntimeContext, InvocationHandler, JsonSchema, ProviderError, ProviderFuture,
    ResourceUpload, TeamRole, ToolDefinition,
};
use serde_json::{Value, json};
use std::sync::Arc;

type SharedClient = FeishuClient<Arc<dyn Transport>>;

pub(crate) fn tool(
    context: &AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
) -> Result<ToolDefinition, ProviderError> {
    Ok(ToolDefinition {
        id: "feishu_person_profile".to_owned(),
        description: "查看一个人的最新飞书资料".to_owned(),
        input_schema: JsonSchema::new(json!({
            "type":"object",
            "required":["person_id"],
            "properties":{"person_id":{"type":"string","pattern":"^[0-9a-f]{6}$"}}
        }))?,
        output_schema: JsonSchema::new(json!({"type":"object"}))?,
        handler: Arc::new(ProfileHandler {
            context: context.clone(),
            client,
            auth,
        }),
    })
}

struct ProfileHandler {
    context: AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
}

impl InvocationHandler for ProfileHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let context = self.context.clone();
        let client = self.client.clone();
        let auth = self.auth;
        Box::pin(async move {
            let person_id = input
                .get("person_id")
                .and_then(Value::as_str)
                .and_then(|value| PersonId::new(value.to_owned()).ok())
                .ok_or(ProviderError::InvalidInput { field: "person_id" })?;
            let person = context.storage().person(&person_id).await?;
            let id_type = person
                .provider_state
                .get("external_id_type")
                .and_then(Value::as_str)
                .unwrap_or("union_id")
                .to_owned();
            let external_id = person.external_id.clone();
            let (user, avatar) = (async move {
                let user = client.get_user_by_id(&external_id, &id_type, auth).await?;
                let url = [
                    user.avatar.avatar_origin.as_str(),
                    user.avatar.avatar_640.as_str(),
                    user.avatar.avatar_240.as_str(),
                    user.avatar.avatar_72.as_str(),
                ]
                .into_iter()
                .find(|value| !value.trim().is_empty());
                let avatar = if let Some(url) = url {
                    Some(client.download_public_url(url).await?)
                } else {
                    None
                };
                Ok::<_, crate::ClientError>((user, avatar))
            })
            .await
            .map_err(|_| ProviderError::External {
                operation: "feishu_person_profile",
                retryable: true,
            })?;
            let avatar_path = if let Some(avatar) = avatar {
                Some(
                    context
                        .storage()
                        .save_agent_file(ResourceUpload {
                            file_name: if avatar.file_name.trim().is_empty() {
                                format!("person-{}.jpg", person_id.as_str())
                            } else {
                                avatar.file_name
                            },
                            media_type: avatar.content_type,
                            bytes: avatar.data,
                        })
                        .await?
                        .path,
                )
            } else {
                None
            };
            let mut output = json!({
                "person_id": person.person_id,
                "name": if user.nickname.trim().is_empty() { user.name } else { user.nickname },
                "signature": user.description,
                "avatar_path": avatar_path,
            });
            if person.team_role != TeamRole::External {
                output.as_object_mut().unwrap().insert(
                    "team_role".to_owned(),
                    json!(match person.team_role {
                        TeamRole::Lead => "lead",
                        TeamRole::Member => "member",
                        TeamRole::External => unreachable!(),
                    }),
                );
            }
            Ok(output)
        })
    }
}
