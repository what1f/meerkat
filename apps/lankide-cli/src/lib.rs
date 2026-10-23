#![forbid(unsafe_code)]

//! Agent-run-scoped Lankide capability CLI.

use lankide_protocol as wire;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const CHUNK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RunConfig {
    pub socket: PathBuf,
    pub agent_home: PathBuf,
    pub execution_id: i64,
    pub attempt: u32,
    pub execution_token: String,
}

impl RunConfig {
    pub fn from_environment() -> Result<Self, CliError> {
        if let Some(path) = std::env::var_os("LANKIDE_RUN_CONTEXT") {
            let path = PathBuf::from(path);
            let home = PathBuf::from(required_env("LANKIDE_AGENT_HOME")?);
            if !path.is_absolute() || path != home.join(".lankide/run-context.json") {
                return Err(CliError::Config("invalid execution context path"));
            }
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(CliError::Config("invalid execution context file"));
            }
            let config: Self = serde_json::from_slice(&std::fs::read(path)?)
                .map_err(|_| CliError::Config("execution context is unavailable"))?;
            if config.agent_home != home {
                return Err(CliError::Config(
                    "execution context has a different Agent home",
                ));
            }
            config.context()?;
            return Ok(config);
        }
        let socket = required_env("LANKIDE_AGENT_SOCKET")?.into();
        let agent_home = required_env("LANKIDE_AGENT_HOME")?.into();
        let execution_id = required_env("LANKIDE_EXECUTION_ID")?
            .parse()
            .map_err(|_| CliError::Config("LANKIDE_EXECUTION_ID must be a positive integer"))?;
        let attempt = required_env("LANKIDE_EXECUTION_ATTEMPT")?
            .parse()
            .map_err(|_| {
                CliError::Config("LANKIDE_EXECUTION_ATTEMPT must be a positive integer")
            })?;
        let execution_token = required_env("LANKIDE_EXECUTION_TOKEN")?;
        let config = Self {
            socket,
            agent_home,
            execution_id,
            attempt,
            execution_token,
        };
        config.context()?;
        Ok(config)
    }

    fn context(&self) -> Result<wire::AgentRunContext, CliError> {
        if !self.socket.is_absolute() {
            return Err(CliError::Config("LANKIDE_AGENT_SOCKET must be absolute"));
        }
        if !self.agent_home.is_absolute() {
            return Err(CliError::Config("LANKIDE_AGENT_HOME must be absolute"));
        }
        if self.execution_id <= 0 || self.attempt == 0 || self.execution_token.trim().is_empty() {
            return Err(CliError::Config("execution context is incomplete"));
        }
        Ok(wire::AgentRunContext {
            execution: Some(wire::ExecutionRef {
                execution_id: self.execution_id,
                attempt: self.attempt,
            }),
            execution_token: self.execution_token.clone(),
        })
    }
}

#[derive(Debug)]
pub enum CliError {
    Config(&'static str),
    Usage(String),
    Io(std::io::Error),
    Transport(String),
    Runtime(lankide_protocol::Status),
    Decode(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(message) => formatter.write_str(message),
            Self::Usage(message) => write!(formatter, "usage: {message}"),
            Self::Io(error) => write!(formatter, "file operation failed: {error}"),
            Self::Transport(message) => write!(formatter, "Runtime is unavailable: {message}"),
            Self::Runtime(error) => write!(
                formatter,
                "request failed: {}: {}",
                error.code(),
                error.message()
            ),
            Self::Decode(message) => write!(formatter, "response is invalid: {message}"),
        }
    }
}

impl std::error::Error for CliError {}
impl From<std::io::Error> for CliError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub async fn execute_from_environment(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Value, CliError> {
    execute(RunConfig::from_environment()?, arguments).await
}

pub async fn execute(
    config: RunConfig,
    arguments: impl IntoIterator<Item = String>,
) -> Result<Value, CliError> {
    let args = arguments.into_iter().collect::<Vec<_>>();
    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError::Usage(help()));
    };
    let mut client = wire::connect_uds(&config.socket, wire::DEFAULT_MAX_MESSAGE_BYTES)
        .await
        .map_err(|error| CliError::Transport(error.to_string()))?;
    match command {
        "capabilities" => {
            require_no_extra(&args, 1)?;
            let response = client
                .list_capabilities(config.context()?)
                .await
                .map_err(status)?
                .into_inner();
            let mut capabilities = response
                .capabilities
                .into_iter()
                .map(|item| {
                    json!({
                        "name": item.name,
                        "display_name": item.display_name,
                        "description": item.description,
                        "kind": item.kind,
                        "input_schema": parse_json_or_string(&item.input_schema_json),
                        "output_schema": parse_json_or_string(&item.output_schema_json),
                    })
                })
                .collect::<Vec<_>>();
            capabilities.push(json!({
                "name": "profile",
                "display_name": "My profile",
                "description": "Run `lankide profile` to view your profile, or `lankide profile update --name NAME --role ROLE --bio BIO --avatar PATH` to update it.",
                "kind": wire::CapabilityKind::CliCommand as i32,
                "input_schema": Value::Null,
                "output_schema": Value::Null,
            }));
            Ok(json!({"capabilities": capabilities}))
        }
        "invoke" => invoke(&mut client, &config, &args[1..]).await,
        "send" => send(&mut client, &config, &args[1..]).await,
        "content" => content(&mut client, &config, &args[1..]).await,
        "profile" => profile(&mut client, &config, &args[1..]).await,
        "help" | "--help" | "-h" => Ok(json!({"usage": help()})),
        _ => Err(CliError::Usage(help())),
    }
}

async fn profile(
    client: &mut wire::AgentClient,
    config: &RunConfig,
    args: &[String],
) -> Result<Value, CliError> {
    let response = match args.first().map(String::as_str) {
        None => client
            .get_self_profile(config.context()?)
            .await
            .map_err(status)?
            .into_inner(),
        Some("update") => {
            let options =
                Options::parse(&args[1..], &["--name", "--role", "--bio", "--avatar"], &[])?;
            if options.values.is_empty() {
                return Err(CliError::Usage(
                    "lankide profile update requires --name, --role, --bio, or --avatar".to_owned(),
                ));
            }
            let avatar = options
                .value("--avatar")
                .map(profile_avatar_upload)
                .transpose()?;
            client
                .update_self_profile(wire::UpdateSelfProfileRequest {
                    context: Some(config.context()?),
                    name: options.value("--name").map(str::to_owned),
                    role: options.value("--role").map(str::to_owned),
                    bio: options.value("--bio").map(str::to_owned),
                    avatar,
                })
                .await
                .map_err(status)?
                .into_inner()
        }
        Some(_) => return Err(CliError::Usage(help())),
    };
    profile_json(config, response)
}

fn profile_avatar_upload(value: &str) -> Result<wire::AgentAvatarUpload, CliError> {
    let path = Path::new(value);
    let media_type = media_type_for(path)
        .filter(|value| value.starts_with("image/"))
        .ok_or_else(|| {
            CliError::Usage("--avatar must be a PNG, JPEG, WebP, or GIF image".to_owned())
        })?;
    Ok(wire::AgentAvatarUpload {
        media_type: media_type.to_owned(),
        bytes: std::fs::read(path)?,
    })
}

fn profile_json(config: &RunConfig, profile: wire::AgentProfile) -> Result<Value, CliError> {
    let agent = profile
        .agent
        .ok_or_else(|| CliError::Decode("profile agent is missing".to_owned()))?;
    if agent.agent_id.trim().is_empty() || agent.workspace_id.trim().is_empty() {
        return Err(CliError::Decode("profile agent is invalid".to_owned()));
    }
    let avatar_path = profile
        .avatar
        .map(|avatar| materialize_profile_avatar(config, &agent.agent_id, avatar))
        .transpose()?;
    if avatar_path.is_none() {
        remove_local_avatars(&config.agent_home, &agent.agent_id, None)?;
    }
    Ok(json!({
        "workspace_id": agent.workspace_id,
        "agent_id": agent.agent_id,
        "name": profile.name,
        "role": profile.role,
        "bio": profile.bio,
        "avatar_path": avatar_path,
    }))
}

fn materialize_profile_avatar(
    config: &RunConfig,
    agent_id: &str,
    avatar: wire::AgentAvatarFile,
) -> Result<PathBuf, CliError> {
    let extension = match avatar.media_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => {
            return Err(CliError::Decode(
                "profile avatar format is invalid".to_owned(),
            ));
        }
    };
    let expected_name = format!("{agent_id}_avatar.{extension}");
    if Path::new(&avatar.path)
        .file_name()
        .and_then(|value| value.to_str())
        != Some(expected_name.as_str())
    {
        return Err(CliError::Decode(
            "profile avatar path is invalid".to_owned(),
        ));
    }
    std::fs::create_dir_all(&config.agent_home)?;
    let target = config.agent_home.join(expected_name);
    let temporary = target.with_extension(format!("{extension}.part"));
    std::fs::write(&temporary, avatar.bytes)?;
    std::fs::rename(&temporary, &target)?;
    remove_local_avatars(&config.agent_home, agent_id, Some(&target))?;
    Ok(target)
}

fn remove_local_avatars(
    agent_home: &Path,
    agent_id: &str,
    keep: Option<&Path>,
) -> Result<(), CliError> {
    for extension in ["png", "jpg", "webp", "gif"] {
        let path = agent_home.join(format!("{agent_id}_avatar.{extension}"));
        if keep == Some(path.as_path()) {
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

async fn invoke(
    client: &mut wire::AgentClient,
    config: &RunConfig,
    args: &[String],
) -> Result<Value, CliError> {
    let name = args
        .first()
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| CliError::Usage("lankide invoke <capability> [--arguments-json JSON | --arguments-file PATH] [--invocation-id ID]".to_owned()))?
        .clone();
    let options = Options::parse(
        &args[1..],
        &["--arguments-json", "--arguments-file", "--invocation-id"],
        &[],
    )?;
    if options.has("--arguments-json") && options.has("--arguments-file") {
        return Err(CliError::Usage(
            "choose one of --arguments-json or --arguments-file".to_owned(),
        ));
    }
    let arguments_json = if let Some(path) = options.value("--arguments-file") {
        std::fs::read_to_string(path)?
    } else {
        options.value("--arguments-json").unwrap_or("{}").to_owned()
    };
    serde_json::from_str::<Value>(&arguments_json)
        .map_err(|error| CliError::Decode(error.to_string()))?;
    let invocation_id = options
        .value("--invocation-id")
        .map(str::to_owned)
        .unwrap_or_else(unique_id);
    let result = client
        .invoke_capability(wire::CapabilityInvokeRequest {
            context: Some(config.context()?),
            capability_name: name,
            arguments_json,
            invocation_id,
        })
        .await
        .map_err(status)?
        .into_inner();
    Ok(capability_result(result))
}

async fn send(
    client: &mut wire::AgentClient,
    config: &RunConfig,
    args: &[String],
) -> Result<Value, CliError> {
    let options = Options::parse(
        args,
        &[
            "--channel",
            "--to",
            "--chat",
            "--content",
            "--content-file",
            "--reply-to",
            "--attachment",
            "--idempotency-key",
        ],
        &["--force"],
    )?;
    let (channel_id, chat_id) = if let Some(person) = options.value("--to") {
        if options.has("--chat") || options.has("--channel") {
            return Err(CliError::Usage(
                "use --to PERSON_ID, or --channel CHANNEL --chat CHAT".into(),
            ));
        }
        (String::new(), format!("person:{person}"))
    } else {
        (
            options.required("--channel")?.to_owned(),
            options.required("--chat")?.to_owned(),
        )
    };
    if options.has("--content") && options.has("--content-file") {
        return Err(CliError::Usage(
            "choose one of --content or --content-file".to_owned(),
        ));
    }
    let content_markdown = if let Some(path) = options.value("--content-file") {
        std::fs::read_to_string(path)?
    } else {
        options.value("--content").unwrap_or_default().to_owned()
    };
    if content_markdown.trim().is_empty() && !options.has("--attachment") {
        return Err(CliError::Usage(
            "send requires --content, --content-file, or --attachment".to_owned(),
        ));
    }
    let mut resources = Vec::new();
    for path in options.values("--attachment") {
        resources.push(upload_file(client, config, Path::new(path), None).await?);
    }
    let reply_to_message_id = options
        .value("--reply-to")
        .map(|value| value.parse::<i64>())
        .transpose()
        .map_err(|_| CliError::Usage("--reply-to must be an integer Message ID".to_owned()))?;
    let idempotency_key = options
        .value("--idempotency-key")
        .map(str::to_owned)
        .unwrap_or_else(unique_id);
    let result = client
        .send_message(wire::SendMessageRequest {
            context: Some(config.context()?),
            channel_id,
            chat_id,
            content_markdown,
            reply_to_message_id,
            resources,
            idempotency_key,
            force: options.flag("--force"),
        })
        .await
        .map_err(status)?
        .into_inner();
    Ok(json!({
        "accepted": result.accepted,
        "message_id": result.message_id,
        "provider_message_id": result.provider_message_id,
        "error": result.error.map(tool_error),
    }))
}

async fn content(
    client: &mut wire::AgentClient,
    config: &RunConfig,
    args: &[String],
) -> Result<Value, CliError> {
    let options = Options::parse(args, &["--kind", "--id"], &[])?;
    let kind = match options.required("--kind")? {
        "message" => wire::ContentKind::Message,
        "task" => wire::ContentKind::Task,
        "provider_object" => wire::ContentKind::ProviderObject,
        _ => {
            return Err(CliError::Usage(
                "--kind must be message, task, or provider_object".to_owned(),
            ));
        }
    };
    let result = client
        .get_content(wire::ContentRequest {
            context: Some(config.context()?),
            kind: kind as i32,
            content_id: options.required("--id")?.to_owned(),
        })
        .await
        .map_err(status)?
        .into_inner();
    Ok(json!({
        "content_id": result.content_id,
        "content_markdown": result.content_markdown,
        "structured_content": parse_json_or_string(&result.structured_content_json),
        "resources": result.resources.into_iter().map(resource_json).collect::<Vec<_>>(),
    }))
}

fn media_type_for(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => return None,
    })
}

async fn upload_file(
    client: &mut wire::AgentClient,
    config: &RunConfig,
    path: &Path,
    media_type: Option<&str>,
) -> Result<wire::ResourceRef, CliError> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Err(CliError::Usage("attachment must not be empty".to_owned()));
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::Usage("attachment has no valid file name".to_owned()))?
        .to_owned();
    let sha256 = lowercase_hex(&Sha256::digest(&bytes));
    let transfer_id = unique_id();
    let descriptor = wire::ResourceDescriptor {
        file_name,
        media_type: media_type
            .or_else(|| media_type_for(path))
            .unwrap_or("application/octet-stream")
            .to_owned(),
        size_bytes: bytes.len() as u64,
        sha256,
        context: Some(config.context()?),
    };
    let len = bytes.len();
    let mut chunks = Vec::new();
    for (index, data) in bytes.chunks(CHUNK_BYTES).enumerate() {
        let offset = index * CHUNK_BYTES;
        chunks.push(wire::ResourceChunk {
            transfer_id: transfer_id.clone(),
            resource_id: 0,
            offset: offset as u64,
            bytes: data.to_vec(),
            eof: offset + data.len() == len,
            descriptor: (index == 0).then(|| descriptor.clone()),
        });
    }
    client
        .upload_resource(tokio_stream::iter(chunks))
        .await
        .map_err(status)
        .map(lankide_protocol::Response::into_inner)
}

fn capability_result(result: wire::CapabilityInvokeResult) -> Value {
    json!({
        "invocation_id": result.invocation_id,
        "success": result.success,
        "content_markdown": result.content_markdown,
        "structured_content": parse_json_or_string(&result.structured_content_json),
        "resources": result.resources.into_iter().map(resource_json).collect::<Vec<_>>(),
        "error": result.error.map(tool_error),
    })
}

fn resource_json(resource: wire::ResourceRef) -> Value {
    json!({
        "resource_id": resource.resource_id,
        "file_name": resource.file_name,
        "media_type": resource.media_type,
        "size_bytes": resource.size_bytes,
        "sha256": resource.sha256,
    })
}

fn tool_error(error: wire::ToolError) -> Value {
    json!({"code": error.code, "message": error.message, "retryable": error.retryable})
}

fn parse_json_or_string(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()))
    }
}

fn status(error: lankide_protocol::Status) -> CliError {
    CliError::Runtime(error)
}

fn required_env(name: &'static str) -> Result<String, CliError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CliError::Config(name))
}

fn unique_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut value, "{byte:02x}");
    }
    value
}

fn require_no_extra(args: &[String], expected: usize) -> Result<(), CliError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(CliError::Usage(help()))
    }
}

fn help() -> String {
    "lankide capabilities | profile [update --name NAME --role ROLE --bio BIO --avatar PATH] | invoke <capability> [--arguments-json JSON | --arguments-file PATH] | send (--channel ID --chat ID | --to PERSON_ID) (--content TEXT | --content-file PATH) [--reply-to MESSAGE_ID] [--attachment PATH] [--force] | content --kind KIND --id ID".to_owned()
}

struct Options {
    values: Vec<(String, String)>,
    flags: Vec<String>,
}

impl Options {
    fn parse(args: &[String], valued: &[&str], flags: &[&str]) -> Result<Self, CliError> {
        let mut result = Self {
            values: Vec::new(),
            flags: Vec::new(),
        };
        let mut index = 0;
        while index < args.len() {
            let option = args[index].as_str();
            if flags.contains(&option) {
                result.flags.push(option.to_owned());
                index += 1;
                continue;
            }
            if valued.contains(&option) {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::Usage(format!("{option} requires a value")))?;
                result.values.push((option.to_owned(), value.clone()));
                index += 2;
                continue;
            }
            return Err(CliError::Usage(format!("unknown option {option}")));
        }
        Ok(result)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.values
            .iter()
            .filter(move |(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn required(&self, name: &str) -> Result<&str, CliError> {
        self.value(name)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CliError::Usage(format!("{name} is required")))
    }

    fn has(&self, name: &str) -> bool {
        self.value(name).is_some() || self.flag(name)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|value| value == name)
    }
}

pub fn os_arguments(values: impl IntoIterator<Item = OsString>) -> Result<Vec<String>, CliError> {
    values
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| CliError::Usage("arguments must be UTF-8".to_owned()))
        })
        .collect()
}
