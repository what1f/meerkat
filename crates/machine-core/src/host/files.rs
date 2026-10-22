//! Resolve local Markdown attachments at the Machine boundary for both CLI and tools.
use super::proxy::LocalTransport;
use lankide_protocol::{agent, *};
use pulldown_cmark::{Event, Parser, Tag};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

pub(super) async fn prepare(
    transport: &LocalTransport,
    method: &str,
    context: &AgentRunContext,
    home: &Path,
    payload: Vec<u8>,
) -> Result<Vec<u8>, Status> {
    match method {
        "send_message" => {
            let mut request: SendMessageRequest = agent::decode(&payload)?;
            request.content_markdown = attachments(
                transport,
                context,
                home,
                &request.content_markdown,
                &mut request.resources,
            )
            .await?;
            agent::encode(&request)
        }
        "invoke_capability" => {
            let mut request: CapabilityInvokeRequest = agent::decode(&payload)?;
            if request.capability_name != "message_send" {
                return Ok(payload);
            }
            let arguments: serde_json::Value = serde_json::from_str(&request.arguments_json)
                .map_err(|_| Status::invalid_argument("invalid_tool_arguments"))?;
            let mut arguments =
                super::control::prepare_tool_arguments("message_send", &arguments, home)
                    .map_err(|(code, _)| Status::invalid_argument(code))?;
            let content = arguments
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let mut resources = Vec::new();
            let content = attachments(transport, context, home, content, &mut resources).await?;
            if !resources.is_empty() {
                let object = arguments
                    .as_object_mut()
                    .ok_or_else(|| Status::invalid_argument("invalid_tool_arguments"))?;
                object.insert("content".into(), content.into());
                let existing = object
                    .entry("attachments")
                    .or_insert_with(|| serde_json::json!([]));
                let existing = existing
                    .as_array_mut()
                    .ok_or_else(|| Status::invalid_argument("invalid_attachments"))?;
                for resource in resources {
                    existing.push(
                        serde_json::to_value(resource)
                            .map_err(|_| Status::internal("encode_resource"))?,
                    );
                }
            }
            request.arguments_json = arguments.to_string();
            agent::encode(&request)
        }
        _ => Ok(payload),
    }
}

async fn attachments(
    transport: &LocalTransport,
    context: &AgentRunContext,
    home: &Path,
    content: &str,
    resources: &mut Vec<ResourceRef>,
) -> Result<String, Status> {
    let _guard = transport.attachment_lock.lock().await;
    let mut links = Vec::new();
    for (event, range) in Parser::new(content).into_offset_iter() {
        if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event {
            let target = dest_url.as_ref();
            if target.starts_with('#')
                || target.contains("://") && !target.starts_with("file://")
                || target.starts_with("mailto:")
            {
                continue;
            }
            if target.is_empty() {
                continue;
            }
            links.push((range, target.to_owned()));
        }
    }
    let mut uploaded = BTreeMap::new();
    let mut replacements = Vec::new();
    for (range, target) in links {
        let target = target.strip_prefix("file://").unwrap_or(&target);
        let path = crate::resolve_content_file(home, target).map_err(|_| {
            Status::invalid_argument("attachment_path_outside_agent_home_or_missing")
        })?;
        let resource = if let Some(resource) = uploaded.get(&path) {
            resource
        } else {
            let metadata = std::fs::metadata(&path)
                .map_err(|_| Status::invalid_argument("attachment_unreadable"))?;
            // The existing capability transport bounds serialized uploads to 8 MiB.
            if !metadata.is_file()
                || metadata.len() == 0
                || metadata.len() > agent::MAX_CALL_BYTES as u64
            {
                return Err(Status::resource_exhausted("attachment_too_large_or_empty"));
            }
            let bytes = std::fs::read(&path)
                .map_err(|_| Status::invalid_argument("attachment_unreadable"))?;
            let descriptor = ResourceDescriptor {
                file_name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                media_type: mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .to_string(),
                size_bytes: bytes.len() as u64,
                sha256: Sha256::digest(&bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                context: Some(context.clone()),
            };
            let transfer_id = uuid::Uuid::new_v4().to_string();
            let chunks: Vec<ResourceChunk> = bytes
                .chunks(agent::CHUNK_BYTES)
                .enumerate()
                .map(|(index, data)| ResourceChunk {
                    transfer_id: transfer_id.clone(),
                    resource_id: 0,
                    offset: (index * agent::CHUNK_BYTES) as u64,
                    bytes: data.to_vec(),
                    eof: (index + 1) * agent::CHUNK_BYTES >= bytes.len(),
                    descriptor: (index == 0).then(|| descriptor.clone()),
                })
                .collect();
            let cache_key = serde_json::to_string(&(
                &descriptor.file_name,
                &descriptor.media_type,
                &descriptor.sha256,
            ))
            .map_err(|_| Status::internal("encode_resource"))?;
            let resource: ResourceRef = match transport.cached_upload(&cache_key).await {
                Some(resource) => resource,
                None => {
                    let result = transport
                        .forward("upload_resource", context.clone(), agent::encode(&chunks)?)
                        .await?;
                    let resource = agent::decode::<ResourceRef>(&result)?;
                    transport
                        .remember_upload(cache_key, resource.clone())
                        .await?;
                    resource
                }
            };
            resources.push(resource.clone());
            uploaded.insert(path.clone(), resource);
            uploaded.get(&path).expect("inserted resource")
        };
        replacements.push((range, resource.file_name.clone()));
    }
    let mut result = content.to_owned();
    // Parser ranges can nest (an image inside a link). Upload every resource,
    // but replace only outermost spans so offsets always refer to the original text.
    let mut outermost: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for (range, label) in replacements {
        if outermost
            .last()
            .is_some_and(|(outer, _)| outer.end > range.start)
        {
            continue;
        }
        outermost.push((range, label));
    }
    for (range, label) in outermost.into_iter().rev() {
        result.replace_range(range, &label);
    }
    Ok(result)
}
