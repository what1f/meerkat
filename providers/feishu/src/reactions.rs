use crate::client::{Auth, Client as FeishuClient};
use crate::http::Transport;
use lankide_core::MessageId;
use lankide_provider::{
    AgentRuntimeContext, InvocationHandler, JsonSchema, ProviderError, ProviderFuture,
    ToolDefinition,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

type SharedClient = FeishuClient<Arc<dyn Transport>>;

pub(crate) fn management_catalog() -> Vec<lankide_provider::ReactionDefinition> {
    bundled_catalog()
        .into_iter()
        .map(|entry| lankide_provider::ReactionDefinition {
            id: entry.emoji_type,
            name: entry.name,
            emoji: String::new(),
        })
        .collect()
}

pub(crate) async fn tools(
    context: &AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
) -> Result<Vec<ToolDefinition>, ProviderError> {
    [
        ReactionOperation::Catalog,
        ReactionOperation::Add,
        ReactionOperation::List,
        ReactionOperation::Remove,
    ]
    .into_iter()
    .map(|operation| definition(operation, context.clone(), client.clone(), auth))
    .collect()
}

#[derive(Clone, Copy)]
enum ReactionOperation {
    Catalog,
    Add,
    List,
    Remove,
}

impl ReactionOperation {
    fn id(self) -> &'static str {
        match self {
            Self::Catalog => "feishu_reaction_catalog",
            Self::Add => "feishu_reaction_add",
            Self::List => "feishu_reaction_list",
            Self::Remove => "feishu_reaction_remove",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Catalog => "查询飞书原生表情的含义和使用场景",
            Self::Add => "给一条飞书消息添加原生表情回复",
            Self::List => "查看一条飞书消息已有的表情回复",
            Self::Remove => "撤回一个飞书表情回复",
        }
    }

    fn input_schema(self) -> Value {
        match self {
            Self::Catalog => json!({
                "type":"object",
                "properties":{"query":{"type":"string"},"cursor":{"type":"integer"},"limit":{"type":"integer"}}
            }),
            Self::Add => json!({
                "type":"object","required":["message_id","emoji_type"],
                "properties":{"message_id":{"type":"integer"},"emoji_type":{"type":"string"}}
            }),
            Self::List => json!({
                "type":"object","required":["message_id"],
                "properties":{"message_id":{"type":"integer"},"page_token":{"type":"string"},"page_size":{"type":"integer"}}
            }),
            Self::Remove => json!({
                "type":"object","required":["message_id","reaction_id"],
                "properties":{"message_id":{"type":"integer"},"reaction_id":{"type":"string"}}
            }),
        }
    }
}

fn definition(
    operation: ReactionOperation,
    context: AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
) -> Result<ToolDefinition, ProviderError> {
    Ok(ToolDefinition {
        id: operation.id().to_owned(),
        description: operation.description().to_owned(),
        input_schema: JsonSchema::new(operation.input_schema())?,
        output_schema: JsonSchema::new(json!({"type":"object"}))?,
        handler: Arc::new(ReactionHandler {
            operation,
            context,
            client,
            auth,
        }),
    })
}

struct ReactionHandler {
    operation: ReactionOperation,
    context: AgentRuntimeContext,
    client: Arc<SharedClient>,
    auth: Auth,
}

impl InvocationHandler for ReactionHandler {
    fn invoke<'a>(&'a self, input: Value) -> ProviderFuture<'a, Value> {
        let operation = self.operation;
        let context = self.context.clone();
        let client = self.client.clone();
        let auth = self.auth;
        Box::pin(async move {
            if matches!(operation, ReactionOperation::Catalog) {
                return Ok(catalog(&input));
            }
            let message_id = input
                .get("message_id")
                .and_then(Value::as_i64)
                .and_then(|value| MessageId::new(value).ok())
                .ok_or(ProviderError::InvalidInput {
                    field: "message_id",
                })?;
            let message = context.storage().message(message_id).await?;
            let provider_message_id =
                message
                    .provider_message_id
                    .ok_or(ProviderError::InvalidInput {
                        field: "message_id",
                    })?;
            (async move {match operation {
                ReactionOperation::Add => {
                    let emoji = required_text(&input, "emoji_type")?;
                    let reaction_id = client
                        .add_reaction(&provider_message_id, &emoji, auth).await
                        .map_err(|_| reaction_error("feishu_reaction_add"))?;
                    Ok(json!({"ok":true,"reaction_id":reaction_id,"emoji_type":emoji}))
                }
                ReactionOperation::List => {
                    let page = client
                        .list_reactions(
                            &provider_message_id,
                            input.get("page_token").and_then(Value::as_str),
                            input.get("page_size").and_then(Value::as_u64).unwrap_or(50) as usize,
                            auth,
                        ).await
                        .map_err(|_| reaction_error("feishu_reaction_list"))?;
                    Ok(json!({"ok":true,"items":page.items.iter().map(|item|json!({
                        "reaction_id":item.reaction_id,
                        "emoji_type":item.emoji,
                        "operator_id":item.operator_id,
                        "operator_type":item.operator_type,
                    })).collect::<Vec<_>>(),"next_page_token":page.next_page_token,"has_more":page.has_more}))
                }
                ReactionOperation::Remove => {
                    let reaction_id = required_text(&input, "reaction_id")?;
                    client
                        .remove_reaction(&provider_message_id, &reaction_id, auth).await
                        .map_err(|_| reaction_error("feishu_reaction_remove"))?;
                    Ok(json!({"ok":true}))
                }
                ReactionOperation::Catalog => unreachable!(),
            }})
            .await
        })
    }
}

fn catalog(input: &Value) -> Value {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let cursor = input.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 100) as usize;
    let catalog = bundled_catalog();
    let filtered = catalog
        .iter()
        .filter(|item| {
            query.is_empty()
                || item.emoji_type.to_ascii_lowercase().contains(&query)
                || item.name.to_ascii_lowercase().contains(&query)
                || item.description.to_ascii_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    let items = filtered
        .iter()
        .skip(cursor)
        .take(limit)
        .map(|item| {
            json!({
                "emoji_type": item.emoji_type,
                "description": item.agent_description(),
            })
        })
        .collect::<Vec<_>>();
    let next_cursor = (cursor + items.len() < filtered.len()).then_some(cursor + items.len());
    json!({"ok":true,"items":items,"next_cursor":next_cursor})
}

#[derive(Clone, Deserialize)]
struct CatalogEntry {
    emoji_type: String,
    name: String,
    description: String,
}

impl CatalogEntry {
    fn agent_description(&self) -> String {
        if self.description.is_empty() {
            return self.name.clone();
        }
        if self
            .name
            .ends_with(['。', '？', '！', '.', '?', '!', '：', ':'])
        {
            format!("{}{}", self.name, self.description)
        } else {
            format!("{}。{}", self.name, self.description)
        }
    }
}

fn bundled_catalog() -> Vec<CatalogEntry> {
    serde_json::from_str(include_str!("../assets/reactions.json"))
        .expect("bundled Feishu reaction catalog must be valid")
}

fn required_text(input: &Value, field: &'static str) -> Result<String, ProviderError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(ProviderError::InvalidInput { field })
}

fn reaction_error(operation: &'static str) -> ProviderError {
    ProviderError::External {
        operation,
        retryable: false,
    }
}
