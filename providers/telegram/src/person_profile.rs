use crate::client::Client as TelegramClient;
use crate::http::Transport;
use lankide_core::PersonId;
use lankide_provider::{
    AgentRuntimeContext, InvocationHandler, JsonSchema, ProviderError, ProviderFuture,
    ResourceUpload, TeamRole, ToolDefinition,
};
use serde_json::{Value, json};
use std::sync::Arc;

type SharedClient = TelegramClient<Arc<dyn Transport>>;

pub(crate) fn tool(
    context: &AgentRuntimeContext,
    client: Arc<SharedClient>,
) -> Result<ToolDefinition, ProviderError> {
    Ok(ToolDefinition {
        id: "telegram_person_profile".to_owned(),
        description: "查看一个人的最新 Telegram 资料".to_owned(),
        input_schema: JsonSchema::new(json!({
            "type":"object",
            "required":["person_id"],
            "properties":{"person_id":{"type":"string","pattern":"^[0-9a-f]{6}$"}}
        }))?,
        output_schema: JsonSchema::new(json!({"type":"object"}))?,
        handler: Arc::new(ProfileHandler {
            context: context.clone(),
            client,
        }),
    })
}

struct ProfileHandler {
    context: AgentRuntimeContext,
    client: Arc<SharedClient>,
}

impl InvocationHandler for ProfileHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let context = self.context.clone();
        let client = self.client.clone();
        Box::pin(async move {
            let person_id = input
                .get("person_id")
                .and_then(Value::as_str)
                .and_then(|value| PersonId::new(value.to_owned()).ok())
                .ok_or(ProviderError::InvalidInput { field: "person_id" })?;
            let person = context.storage().person(&person_id).await?;
            let external_id =
                person
                    .external_id
                    .parse::<i64>()
                    .map_err(|_| ProviderError::InvalidInput {
                        field: "person_external_id",
                    })?;
            let (chat, avatar) = (async move {
                let chat = client.get_chat(external_id).await?;
                let file_id = chat
                    .photo
                    .as_ref()
                    .map(|photo| {
                        if photo.big_file_id.trim().is_empty() {
                            photo.small_file_id.as_str()
                        } else {
                            photo.big_file_id.as_str()
                        }
                    })
                    .filter(|value| !value.trim().is_empty());
                let avatar = if let Some(id) = file_id {
                    Some(client.download_file(id).await?)
                } else {
                    None
                };
                Ok::<_, crate::http::ClientError>((chat, avatar))
            })
            .await
            .map_err(|_| ProviderError::External {
                operation: "telegram_person_profile",
                retryable: true,
            })?;
            let avatar_path = if let Some(avatar) = avatar {
                Some(
                    context
                        .storage()
                        .save_agent_file(ResourceUpload {
                            file_name: format!("person-{}.jpg", person_id.as_str()),
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
                "name": chat.display_name(),
                "signature": chat.bio,
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
