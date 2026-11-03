use crate::MAX_WORKSPACE_IMAGE_BYTES;
use crate::Status;
use crate::wire::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};

pub fn validate_machine_hello(hello: &MachineHello, expected_version: u32) -> Result<(), Status> {
    if hello.protocol_version != expected_version {
        return Err(Status::failed_precondition(format!(
            "protocol_version_mismatch: expected {expected_version}, received {}",
            hello.protocol_version
        )));
    }
    require_text(&hello.machine_id, "machine_id")?;
    require_text(&hello.instance_id, "instance_id")?;
    require_text(&hello.machine_version, "machine_version")?;
    require_text(&hello.os, "os")?;
    require_text(&hello.arch, "arch")?;
    for harness in &hello.harnesses {
        require_text(&harness.harness_id, "harness_id")?;
        require_text(&harness.display_name, "harness_display_name")?;
        HarnessAvailability::try_from(harness.availability)
            .map_err(|_| Status::invalid_argument("unknown_harness_availability"))?;
    }
    Ok(())
}

pub fn validate_runtime_frame(frame: &RuntimeFrame) -> Result<(), Status> {
    match frame.frame.as_ref() {
        Some(runtime_frame::Frame::Hello(hello)) => {
            if hello.protocol_version == 0 {
                Err(Status::invalid_argument("invalid_protocol_version"))
            } else if hello.max_message_bytes == 0 {
                Err(Status::invalid_argument("invalid_max_message_bytes"))
            } else {
                require_text(&hello.runtime_instance_id, "runtime_instance_id")
            }
        }
        Some(runtime_frame::Frame::AssignExecution(assign)) => validate_assign_execution(assign),
        Some(runtime_frame::Frame::ExecutionEventAck(ack)) => {
            validate_execution_ref(ack.execution.as_ref())
        }
        Some(runtime_frame::Frame::ExecutionCommitted(committed)) => {
            validate_execution_ref(committed.execution.as_ref())
        }
        Some(runtime_frame::Frame::TestHarness(request)) => {
            validate_request_id(&request.request_id)?;
            require_text(&request.harness.harness_id, "harness_id")
        }
        Some(runtime_frame::Frame::DeleteAgentFiles(request)) => {
            validate_request_id(&request.request_id)?;
            validate_agent_ref(request.agent.as_ref())
        }
        Some(_) => Ok(()),
        None => Err(Status::invalid_argument("unknown_runtime_frame")),
    }
}

#[derive(Default)]
pub(super) struct MachineFrameValidator {
    last_sequences: HashMap<(i64, u32), u64>,
}

impl MachineFrameValidator {
    pub(super) fn validate(&mut self, frame: &MachineFrame) -> Result<(), Status> {
        match frame.frame.as_ref() {
            Some(machine_frame::Frame::Hello(_)) => {
                Err(Status::invalid_argument("duplicate_machine_hello"))
            }
            Some(machine_frame::Frame::ExecutionEvent(event)) => {
                let execution = event.execution.as_ref();
                validate_execution_ref(execution)?;
                if event.sequence == 0 {
                    return Err(Status::invalid_argument("event_sequence_must_be_positive"));
                }
                ExecutionEventKind::try_from(event.kind)
                    .map_err(|_| Status::invalid_argument("unknown_execution_event_kind"))?;
                let execution = execution.expect("validated execution ref");
                let key = (execution.execution_id, execution.attempt);
                if let Some(previous) = self.last_sequences.get(&key)
                    && event.sequence < *previous
                {
                    return Err(Status::out_of_range("event_sequence_regression"));
                }
                self.last_sequences.insert(key, event.sequence);
                Ok(())
            }
            Some(machine_frame::Frame::AssignmentResult(result)) => {
                validate_execution_ref(result.execution.as_ref())
            }
            Some(machine_frame::Frame::DeliveryResult(result)) => {
                validate_execution_ref(result.execution.as_ref())
            }
            Some(machine_frame::Frame::TestHarnessResult(result)) => {
                validate_request_id(&result.request_id)
            }
            Some(machine_frame::Frame::DeleteAgentFilesResult(result)) => {
                validate_request_id(&result.request_id)
            }
            Some(_) => Ok(()),
            None => Err(Status::invalid_argument("unknown_machine_frame")),
        }
    }
}

#[derive(Default)]
pub(super) struct ResourceUploadValidator {
    transfer_id: Option<String>,
    resource_id: Option<i64>,
    descriptor: Option<ResourceDescriptor>,
    next_offset: u64,
    digest: Sha256,
    saw_eof: bool,
}

impl ResourceUploadValidator {
    pub(super) fn expecting(transfer_id: String, resource_id: i64) -> Self {
        Self {
            transfer_id: Some(transfer_id),
            resource_id: Some(resource_id),
            ..Self::default()
        }
    }

    pub(super) fn validate(&mut self, chunk: &ResourceChunk) -> Result<(), Status> {
        if self.saw_eof {
            return Err(Status::invalid_argument("resource_chunk_after_eof"));
        }
        require_text(&chunk.transfer_id, "transfer_id")?;
        if chunk.offset != self.next_offset {
            return Err(Status::invalid_argument(format!(
                "resource_offset_mismatch: expected {}, received {}",
                self.next_offset, chunk.offset
            )));
        }
        if let Some(transfer_id) = &self.transfer_id
            && transfer_id != &chunk.transfer_id
        {
            return Err(Status::invalid_argument("transfer_id_changed"));
        }
        if let Some(resource_id) = self.resource_id
            && resource_id != chunk.resource_id
        {
            return Err(Status::invalid_argument("resource_id_changed"));
        }
        match (&self.descriptor, &chunk.descriptor) {
            (None, Some(descriptor)) => {
                validate_resource_descriptor(descriptor)?;
                validate_agent_run_context(descriptor.context.as_ref())?;
                self.transfer_id.get_or_insert(chunk.transfer_id.clone());
                self.resource_id.get_or_insert(chunk.resource_id);
                self.descriptor = Some(descriptor.clone());
            }
            (None, None) => return Err(Status::invalid_argument("first_chunk_missing_descriptor")),
            (Some(_), Some(_)) => {
                return Err(Status::invalid_argument(
                    "descriptor_only_allowed_on_first_chunk",
                ));
            }
            (Some(_), None) => {}
        }
        self.digest.update(&chunk.bytes);
        self.next_offset = self
            .next_offset
            .checked_add(chunk.bytes.len() as u64)
            .ok_or_else(|| Status::out_of_range("resource_size_overflow"))?;
        if chunk.eof {
            let descriptor = self.descriptor.as_ref().expect("first chunk validated");
            if self.next_offset != descriptor.size_bytes {
                return Err(Status::invalid_argument("resource_size_mismatch"));
            }
            let actual = lowercase_hex(&self.digest.clone().finalize());
            if actual != descriptor.sha256 {
                return Err(Status::data_loss("resource_sha256_mismatch"));
            }
            self.saw_eof = true;
        }
        Ok(())
    }

    pub(super) const fn complete(&self) -> bool {
        self.saw_eof
    }
}

fn validate_assign_execution(assign: &AssignExecution) -> Result<(), Status> {
    validate_execution_ref(assign.execution.as_ref())?;
    let workspace = assign
        .workspace
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("missing_workspace_ref"))?;
    meerkat_core::WorkspaceId::new(workspace.workspace_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    let agent = assign
        .agent
        .as_ref()
        .and_then(|profile| profile.agent.as_ref())
        .ok_or_else(|| Status::invalid_argument("missing_agent_ref"))?;
    if agent.workspace_id != workspace.workspace_id {
        return Err(Status::invalid_argument("agent_workspace_mismatch"));
    }
    meerkat_core::AgentId::new(agent.agent_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    if assign.harness.is_none() {
        return Err(Status::invalid_argument("missing_harness_config"));
    }
    if assign.session.is_none() {
        return Err(Status::invalid_argument("missing_session_directive"));
    }
    if assign.initial_delivery.is_none() {
        return Err(Status::invalid_argument("missing_initial_delivery"));
    }
    let mut tool_names = BTreeSet::new();
    for tool in &assign.tools {
        if !meerkat_core::is_canonical_tool_name(&tool.name) {
            return Err(Status::invalid_argument("invalid_tool_name"));
        }
        if !tool_names.insert(&tool.name) {
            return Err(Status::invalid_argument("duplicate_tool_name"));
        }
        require_text(&tool.description, "tool_description")?;
        validate_json(&tool.input_schema_json, "tool_input_schema_json")?;
        validate_json(&tool.output_schema_json, "tool_output_schema_json")?;
    }
    require_text(&assign.execution_token, "execution_token")
}

fn validate_execution_ref(execution: Option<&ExecutionRef>) -> Result<(), Status> {
    let execution = execution.ok_or_else(|| Status::invalid_argument("missing_execution_ref"))?;
    meerkat_core::ExecutionId::new(execution.execution_id)
        .map_err(|error| Status::invalid_argument(error.code()))?;
    if execution.attempt == 0 {
        return Err(Status::invalid_argument("attempt_must_be_positive"));
    }
    Ok(())
}

pub(super) fn validate_resource_ref(resource: &ResourceRef) -> Result<(), Status> {
    meerkat_core::ResourceId::new(resource.resource_id)
        .map_err(|error| Status::invalid_argument(error.code()))?;
    validate_resource_descriptor(&ResourceDescriptor {
        file_name: resource.file_name.clone(),
        media_type: resource.media_type.clone(),
        size_bytes: resource.size_bytes,
        sha256: resource.sha256.clone(),
        context: None,
    })
}

fn validate_resource_descriptor(descriptor: &ResourceDescriptor) -> Result<(), Status> {
    require_text(&descriptor.file_name, "file_name")?;
    require_text(&descriptor.media_type, "media_type")?;
    if descriptor.sha256.len() != 64
        || !descriptor
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Status::invalid_argument("invalid_resource_sha256"));
    }
    Ok(())
}

pub(super) fn validate_resource_request(request: &ResourceRequest) -> Result<(), Status> {
    meerkat_core::ResourceId::new(request.resource_id)
        .map_err(|error| Status::invalid_argument(error.code()))?;
    require_text(&request.transfer_id, "transfer_id")?;
    validate_agent_run_context(request.context.as_ref())
}

pub(super) fn validate_request_id(request_id: &str) -> Result<(), Status> {
    require_text(request_id, "request_id")
}

pub(super) fn validate_workspace_ref(workspace: Option<&WorkspaceRef>) -> Result<(), Status> {
    let workspace = workspace.ok_or_else(|| Status::invalid_argument("missing_workspace_ref"))?;
    meerkat_core::WorkspaceId::new(workspace.workspace_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    Ok(())
}

pub(super) fn validate_agent_ref(agent: Option<&AgentRef>) -> Result<(), Status> {
    let agent = agent.ok_or_else(|| Status::invalid_argument("missing_agent_ref"))?;
    meerkat_core::WorkspaceId::new(agent.workspace_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    meerkat_core::AgentId::new(agent.agent_id.clone())
        .map_err(|error| Status::invalid_argument(error.code()))?;
    Ok(())
}

pub(super) fn validate_avatar_upload(avatar: &AgentAvatarUpload) -> Result<(), Status> {
    require_text(&avatar.media_type, "avatar_media_type")?;
    if avatar.bytes.is_empty() {
        return Err(Status::invalid_argument("avatar is empty"));
    }
    if avatar.bytes.len() > MAX_WORKSPACE_IMAGE_BYTES {
        return Err(Status::resource_exhausted("avatar exceeds size limit"));
    }
    if !matches!(
        avatar.media_type.as_str(),
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    ) {
        return Err(Status::invalid_argument("unsupported avatar format"));
    }
    Ok(())
}

pub(super) fn validate_avatar_file(avatar: &AgentAvatarFile) -> Result<(), Status> {
    require_text(&avatar.path, "avatar_path")?;
    require_text(&avatar.media_type, "avatar_media_type")?;
    if avatar.bytes.is_empty() || avatar.bytes.len() > MAX_WORKSPACE_IMAGE_BYTES {
        return Err(Status::data_loss("avatar bytes are invalid"));
    }
    Ok(())
}

pub(super) fn validate_agent_profile(profile: &AgentProfile) -> Result<(), Status> {
    validate_agent_ref(profile.agent.as_ref())?;
    require_text(&profile.name, "agent_name")?;
    if let Some(avatar) = profile.avatar.as_ref() {
        validate_avatar_file(avatar)?;
    }
    Ok(())
}

pub(super) fn validate_agent_write(
    request_id: &str,
    agent: Option<&AgentRef>,
    name: &str,
    harness_id: &str,
    machine_id: &str,
    attention: Option<&AgentAttention>,
) -> Result<(), Status> {
    validate_request_id(request_id)?;
    validate_agent_ref(agent)?;
    require_text(name, "agent_name")?;
    require_text(harness_id, "harness_id")?;
    require_text(machine_id, "machine_id")?;
    if let Some(attention) = attention {
        validate_attention(attention)?;
    }
    Ok(())
}

pub(super) fn validate_attention(attention: &AgentAttention) -> Result<(), Status> {
    let mut channels = BTreeSet::new();
    for value in &attention.channels {
        meerkat_core::ChannelId::new(value.channel_id.clone())
            .map_err(|error| Status::invalid_argument(error.code()))?;
        if !channels.insert(value.channel_id.as_str()) {
            return Err(Status::invalid_argument("Choose each channel only once."));
        }
        match AttentionMode::try_from(value.mode).unwrap_or(AttentionMode::Unspecified) {
            AttentionMode::Mention | AttentionMode::Always | AttentionMode::Mute
                if value.interval_minutes == 0 => {}
            AttentionMode::Schedule
                if matches!(
                    value.interval_minutes,
                    5 | 15 | 30 | 60 | 120 | 240 | 480 | 1440
                ) => {}
            AttentionMode::Unspecified => {
                return Err(Status::invalid_argument(
                    "Choose how the agent should respond in each channel.",
                ));
            }
            _ => {
                return Err(Status::invalid_argument(
                    "Choose a supported schedule interval.",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_catalog_request(request: &CatalogRequest) -> Result<(), Status> {
    validate_request_id(&request.request_id)?;
    if request.workspace.is_some() {
        validate_workspace_ref(request.workspace.as_ref())?;
    }
    Ok(())
}

pub(super) fn validate_agent_run_context(context: Option<&AgentRunContext>) -> Result<(), Status> {
    let context = context.ok_or_else(|| Status::unauthenticated("missing_agent_run_context"))?;
    validate_execution_ref(context.execution.as_ref())?;
    if context.execution_token.trim().is_empty() {
        return Err(Status::unauthenticated("missing_execution_token"));
    }
    Ok(())
}

pub(super) fn validate_send_message_content(
    content_markdown: &str,
    resources: &[ResourceRef],
) -> Result<(), Status> {
    if content_markdown.trim().is_empty() && resources.is_empty() {
        Err(Status::invalid_argument("empty_content_markdown"))
    } else {
        Ok(())
    }
}

pub(super) fn validate_json(value: &str, field: &'static str) -> Result<(), Status> {
    serde_json::from_str::<serde_json::Value>(value)
        .map(|_| ())
        .map_err(|_| Status::invalid_argument(format!("invalid_{field}")))
}

pub(super) fn require_text(value: &str, field: &'static str) -> Result<(), Status> {
    if value.trim().is_empty() {
        Err(Status::invalid_argument(format!("empty_{field}")))
    } else {
        Ok(())
    }
}

fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
