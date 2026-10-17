use crate::{Credentials, TelegramClient, Transport};
use lankide_provider::{InitContext, InvocationHandler, ProviderError, ProviderFuture};
use serde_json::{Value, json};
use std::sync::Arc;
pub(crate) struct OwnerHandler {
    pub context: InitContext,
    pub transport: Arc<dyn Transport>,
}
impl InvocationHandler for OwnerHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        Box::pin(async move {
            let mut state = self.context.storage().workspace_state().await?;
            let mut owner = state
                .get("owner")
                .cloned()
                .unwrap_or_else(|| json!({"state":"unbound"}));
            match input.get("action").and_then(Value::as_str) {
                Some("get") => return Ok(owner),
                Some("refresh") if owner.get("person_id").is_none() => return Ok(owner),
                Some("refresh") => {}
                _ => return Err(ProviderError::InvalidInput { field: "action" }),
            }
            let token = self
                .context
                .secret_reader()
                .read("manager_bot_token")
                .await?;
            let user = state.get("owner_user_id").and_then(Value::as_i64).ok_or(
                ProviderError::InvalidInput {
                    field: "owner_user_id",
                },
            )?;
            let client = TelegramClient::new(
                self.transport.clone(),
                Credentials::new("manager", token.expose_secret()),
            )
            .map_err(external)?
            .with_domain(crate::adapter::api_endpoint(
                self.context.workspace_config(),
            )?);
            let result = async {
                // A name refresh uses the accessible private chat; the initial name
                // always comes from ManagedBotUpdated.user.
                let chat = client.get_chat(user).await?;
                let photo = client.user_profile_photo(user).await?;
                Ok::<_, crate::ClientError>((chat, photo))
            }
            .await;
            match result {
                Ok((chat, photo)) => {
                    owner["state"] = json!("ready");
                    owner["name"] = json!(chat.display_name());
                    owner.as_object_mut().unwrap().remove("avatar");
                    if let Some(photo) = photo {
                        owner["avatar"] = resource_json(
                            self.context
                                .save_resource(lankide_provider::ResourceUpload {
                                    file_name: "owner.jpg".into(),
                                    media_type: photo.content_type,
                                    bytes: photo.data,
                                })
                                .await?,
                        );
                    }
                }
                Err(crate::ClientError::Api {
                    code: 401 | 403, ..
                }) => owner["state"] = json!("reauth_required"),
                Err(e) => return Err(external(e)),
            }
            state["owner"] = owner.clone();
            self.context.storage().put_workspace_state(state).await?;
            self.context.storage().management_changed().await?;
            Ok(owner)
        })
    }
}
fn external(e: crate::ClientError) -> ProviderError {
    ProviderError::External {
        operation: "telegram_workspace_owner",
        retryable: e.is_retryable(),
    }
}

fn resource_json(r: lankide_core::ResourceRef) -> Value {
    json!({"resource_id":r.resource_id,"file_name":r.file_name,"media_type":r.media_type,"size_bytes":r.size_bytes,"sha256":r.sha256})
}
