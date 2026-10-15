use lankide_protocol::{AgentRunContext, ResourceRef};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::{Component, Path, PathBuf};

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
    runtime_run: &Path,
    endpoint: Option<&str>,
    resource: &ResourceRef,
    context: AgentRunContext,
    directory: &Path,
) -> Result<PathBuf, ComputerError> {
    let (path, temporary) = download_paths(resource, directory)?;
    let runtime_url = match endpoint {
        Some(url) => url.to_owned(),
        None => lankide_protocol::link::discover(runtime_run)?.0,
    };
    let mut url = reqwest::Url::parse(&runtime_url)
        .map_err(|_| ComputerError::Resource("invalid_runtime_url".into()))?;
    let scheme = match url.scheme() {
        "ws" | "http" => "http",
        "wss" | "https" => "https",
        _ => return Err(ComputerError::Resource("invalid_runtime_url".into())),
    };
    url.set_scheme(scheme)
        .map_err(|_| ComputerError::Resource("invalid_runtime_url".into()))?;
    url.set_path(&format!("/resources/{}", resource.resource_id));
    url.set_query(None);
    url.set_fragment(None);
    let execution = context
        .execution
        .as_ref()
        .ok_or_else(|| ComputerError::Resource("missing_execution".into()))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| ComputerError::Resource(e.to_string()))?;
    // Keep an interrupted file for the next bounded retry, including retries
    // after a Computer restart. Never publish bytes before integrity checks.
    for retry in 0..3 {
        let offset = std::fs::metadata(&temporary).map(|m| m.len()).unwrap_or(0);
        if offset > resource.size_bytes {
            let _ = std::fs::remove_file(&temporary);
            return Err(ComputerError::ResourceIntegrityMismatch);
        }
        if offset == resource.size_bytes && temporary.exists() {
            break;
        }
        let mut request = client
            .get(url.clone())
            .bearer_auth(&context.execution_token)
            .header("x-lankide-execution", execution.execution_id)
            .header("x-lankide-attempt", execution.attempt);
        if offset > 0 {
            request = request.header("range", format!("bytes={offset}-"));
        }
        let response = request.send().await;
        let mut response = match response {
            Ok(response) => response,
            Err(_) if retry < 2 => continue,
            Err(error) => return Err(ComputerError::Resource(error.to_string())),
        };
        if offset > 0 {
            let expected = format!(
                "bytes {offset}-{}/{}",
                resource.size_bytes.saturating_sub(1),
                resource.size_bytes
            );
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                || response
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    != Some(expected.as_str())
            {
                let _ = std::fs::remove_file(&temporary);
                return Err(ComputerError::Resource("resource_resume_rejected".into()));
            }
        } else if response.status() != reqwest::StatusCode::OK {
            return Err(ComputerError::Resource(format!(
                "resource_http_{}",
                response.status().as_u16()
            )));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temporary)?;
        let mut size = offset;
        let mut interrupted = false;
        loop {
            match response.chunk().await {
                Ok(Some(bytes)) => {
                    size += bytes.len() as u64;
                    if size > resource.size_bytes {
                        drop(file);
                        let _ = std::fs::remove_file(&temporary);
                        return Err(ComputerError::ResourceIntegrityMismatch);
                    }
                    std::io::Write::write_all(&mut file, &bytes)?;
                }
                Ok(None) => break,
                Err(_) => {
                    interrupted = true;
                    break;
                }
            }
        }
        file.sync_all()?;
        if !interrupted {
            break;
        }
        if retry == 2 {
            return Err(ComputerError::Resource(
                "resource_transfer_interrupted".into(),
            ));
        }
    }
    finish_download(resource, &temporary, &path)
}

pub(crate) fn download_paths(
    resource: &ResourceRef,
    directory: &Path,
) -> Result<(PathBuf, PathBuf), ComputerError> {
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
    Ok((path, temporary))
}

pub(crate) fn finish_download(
    resource: &ResourceRef,
    temporary: &Path,
    path: &Path,
) -> Result<PathBuf, ComputerError> {
    let mut file = std::fs::File::open(&temporary)?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut file, &mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        size += count as u64;
    }
    let actual = lowercase_hex(&digest.finalize());
    if size != resource.size_bytes || actual != resource.sha256.to_ascii_lowercase() {
        drop(file);
        let _ = std::fs::remove_file(&temporary);
        return Err(ComputerError::ResourceIntegrityMismatch);
    }
    drop(file);
    std::fs::rename(&temporary, &path)?;
    Ok(path.to_owned())
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
