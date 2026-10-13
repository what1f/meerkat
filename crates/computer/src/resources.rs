use lankide_protocol::{AgentRunContext, ResourceRef, ResourceRequest};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::{Component, Path, PathBuf};
use tokio_stream::StreamExt;

use crate::ComputerError;

pub fn workspace_directory(root: &Path, workspace_id: &str) -> Result<PathBuf, ComputerError> {
    checked_child(root, workspace_id, "workspace_id")
}

pub fn agent_directory(
    root: &Path,
    workspace_id: &str,
    agent_id: &str,
) -> Result<PathBuf, ComputerError> {
    checked_child(
        &workspace_directory(root, workspace_id)?.join("agents"),
        agent_id,
        "agent_id",
    )
}

fn checked_child(root: &Path, value: &str, field: &'static str) -> Result<PathBuf, ComputerError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ComputerError::InvalidAssignment(format!("invalid_{field}")));
    }
    Ok(root.join(path))
}

pub fn resolve_content_file(agent_home: &Path, value: &str) -> Result<PathBuf, ComputerError> {
    let requested = Path::new(value.trim());
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        agent_home.join(requested)
    };
    let agent_home = agent_home.canonicalize()?;
    let path = path.canonicalize()?;
    if !path.starts_with(&agent_home) {
        return Err(ComputerError::PathOutsideAgentHome);
    }
    Ok(path)
}

pub(crate) async fn download_resource(
    socket: &Path,
    resource: &ResourceRef,
    context: AgentRunContext,
    directory: &Path,
) -> Result<PathBuf, ComputerError> {
    let file_name = Path::new(&resource.file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ComputerError::Resource("invalid_resource_file_name".into()))?;
    if Path::new(&resource.file_name).is_absolute()
        || Path::new(&resource.file_name).components().count() != 1
        || file_name != resource.file_name
    {
        return Err(ComputerError::Resource(
            "resource_file_name_outside_target".into(),
        ));
    }
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!("{}-{file_name}", resource.resource_id));
    let temporary = path.with_extension("part");
    let mut file = std::fs::File::create(&temporary)?;
    let mut client =
        lankide_protocol::connect_uds(socket, lankide_protocol::DEFAULT_MAX_MESSAGE_BYTES).await?;
    let transfer_id = format!("download-{}-{}", resource.resource_id, std::process::id());
    let mut stream = client
        .download_resource(ResourceRequest {
            resource_id: resource.resource_id,
            transfer_id,
            context: Some(context),
        })
        .await?
        .into_inner();
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut saw_eof = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.offset != size {
            return Err(ComputerError::Resource("resource_offset_mismatch".into()));
        }
        digest.update(&chunk.bytes);
        std::io::Write::write_all(&mut file, &chunk.bytes)?;
        size += chunk.bytes.len() as u64;
        if chunk.eof {
            saw_eof = true;
            break;
        }
    }
    std::io::Write::flush(&mut file)?;
    let actual = lowercase_hex(&digest.finalize());
    if !saw_eof || size != resource.size_bytes || actual != resource.sha256.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&temporary);
        return Err(ComputerError::Resource(
            "resource_integrity_mismatch".into(),
        ));
    }
    std::fs::rename(&temporary, &path)?;
    Ok(path)
}

pub(crate) fn replace_resource_links(markdown: &str, paths: &[(i64, PathBuf)]) -> String {
    let mut output = markdown.to_owned();
    for (id, path) in paths {
        let local = path.display().to_string();
        output = output.replace(&format!("resource://{id}"), &local);
        output = output.replace(&format!("attachment://{id}"), &local);
    }
    output
}

fn lowercase_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
