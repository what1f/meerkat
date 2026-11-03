use super::*;
use image::ImageFormat;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

pub(crate) struct AgentProfileFiles {
    data_root: PathBuf,
}

pub(crate) struct StoredAgentAvatar {
    pub path: String,
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

pub(crate) struct AgentAvatarMutation {
    pub avatar: StoredAgentAvatar,
    pub changed: bool,
    target: PathBuf,
    previous_path: Option<PathBuf>,
    previous_target_bytes: Option<Vec<u8>>,
}

impl AgentProfileFiles {
    pub fn new(database_path: &Path) -> Self {
        Self {
            data_root: database_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
        }
    }

    pub fn read_avatar(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        stored_path: Option<&str>,
    ) -> Result<Option<StoredAgentAvatar>, RuntimeError> {
        let Some(stored_path) = stored_path.filter(|value| !value.trim().is_empty()) else {
            return Ok(None);
        };
        let relative = self.validate_relative_path(workspace_id, agent_id, stored_path)?;
        let absolute = self.data_root.join(&relative);
        let bytes = std::fs::read(&absolute).map_err(profile_io_error)?;
        let (extension, media_type) = detect_avatar_format(&bytes)?;
        if relative.extension().and_then(|value| value.to_str()) != Some(extension) {
            return Err(RuntimeError::Application(
                "agent avatar format does not match its path".into(),
            ));
        }
        let file_name = relative
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| RuntimeError::Application("invalid agent avatar path".into()))?
            .to_owned();
        Ok(Some(StoredAgentAvatar {
            path: stored_path.to_owned(),
            file_name,
            media_type: media_type.to_owned(),
            bytes,
        }))
    }

    pub fn replace_avatar(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        previous_path: Option<&str>,
        upload: &meerkat_protocol::AgentAvatarUpload,
    ) -> Result<AgentAvatarMutation, RuntimeError> {
        let (extension, media_type) = detect_avatar_format(&upload.bytes)?;
        if upload.media_type != media_type {
            return Err(RuntimeError::Application(
                "agent avatar media type does not match its contents".into(),
            ));
        }
        let relative = avatar_relative_path(workspace_id, agent_id, extension);
        let path = relative.to_string_lossy().into_owned();
        let target = self.data_root.join(&relative);
        let previous_path = previous_path
            .map(|value| {
                self.validate_relative_path(workspace_id, agent_id, value)
                    .map(|path| self.data_root.join(path))
            })
            .transpose()?;
        let previous_target_bytes = match std::fs::read(&target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(profile_io_error(error)),
        };
        let changed = previous_target_bytes.as_deref() != Some(upload.bytes.as_slice())
            || previous_path.as_ref() != Some(&target);
        if changed {
            atomic_write(&target, &upload.bytes)?;
        }
        Ok(AgentAvatarMutation {
            avatar: StoredAgentAvatar {
                path,
                file_name: format!("{}_avatar.{extension}", agent_id.as_str()),
                media_type: media_type.to_owned(),
                bytes: upload.bytes.clone(),
            },
            changed,
            target,
            previous_path,
            previous_target_bytes,
        })
    }

    pub fn remove_avatar(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        stored_path: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let Some(stored_path) = stored_path.filter(|value| !value.trim().is_empty()) else {
            return Ok(());
        };
        let relative = self.validate_relative_path(workspace_id, agent_id, stored_path)?;
        match std::fs::remove_file(self.data_root.join(relative)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(profile_io_error(error)),
        }
    }

    fn validate_relative_path(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
        value: &str,
    ) -> Result<PathBuf, RuntimeError> {
        let path = PathBuf::from(value);
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(RuntimeError::Application(
                "invalid agent avatar path".into(),
            ));
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| matches!(*value, "png" | "jpg" | "webp" | "gif"))
            .ok_or_else(|| RuntimeError::Application("invalid agent avatar format".into()))?;
        if path != avatar_relative_path(workspace_id, agent_id, extension) {
            return Err(RuntimeError::Application(
                "invalid agent avatar path".into(),
            ));
        }
        Ok(path)
    }
}

impl AgentAvatarMutation {
    pub fn rollback(self) -> Result<(), RuntimeError> {
        if !self.changed {
            return Ok(());
        }
        if let Some(bytes) = self.previous_target_bytes {
            atomic_write(&self.target, &bytes)
        } else {
            match std::fs::remove_file(&self.target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(profile_io_error(error)),
            }
        }
    }

    pub fn finish(self) {
        if self.changed
            && let Some(previous_path) = self.previous_path
            && previous_path != self.target
        {
            let _ = std::fs::remove_file(previous_path);
        }
    }
}

pub(crate) fn avatar_file(avatar: StoredAgentAvatar) -> meerkat_protocol::AgentAvatarFile {
    meerkat_protocol::AgentAvatarFile {
        path: avatar.path,
        media_type: avatar.media_type,
        bytes: avatar.bytes,
    }
}

fn avatar_relative_path(
    workspace_id: &WorkspaceId,
    agent_id: &AgentId,
    extension: &str,
) -> PathBuf {
    PathBuf::from("resources")
        .join(workspace_id.as_str())
        .join(format!("{}_avatar.{extension}", agent_id.as_str()))
}

fn detect_avatar_format(bytes: &[u8]) -> Result<(&'static str, &'static str), RuntimeError> {
    if bytes.is_empty() || bytes.len() > meerkat_protocol::MAX_WORKSPACE_IMAGE_BYTES {
        return Err(RuntimeError::Application(
            "Choose a PNG, JPEG, WebP or GIF image up to 3 MiB".into(),
        ));
    }
    let format = image::guess_format(bytes).map_err(|_| {
        RuntimeError::Application("Choose a PNG, JPEG, WebP or GIF image up to 3 MiB".into())
    })?;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| {
        RuntimeError::Application("Choose a PNG, JPEG, WebP or GIF image up to 3 MiB".into())
    })?;
    match format {
        ImageFormat::Png => Ok(("png", "image/png")),
        ImageFormat::Jpeg => Ok(("jpg", "image/jpeg")),
        ImageFormat::WebP => Ok(("webp", "image/webp")),
        ImageFormat::Gif => Ok(("gif", "image/gif")),
        _ => Err(RuntimeError::Application(
            "Choose a PNG, JPEG, WebP or GIF image up to 3 MiB".into(),
        )),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
    let parent = path
        .parent()
        .ok_or_else(|| RuntimeError::Application("invalid agent avatar path".into()))?;
    std::fs::create_dir_all(parent).map_err(profile_io_error)?;
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, bytes).map_err(profile_io_error)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(profile_io_error(error));
    }
    Ok(())
}

fn profile_io_error(_: std::io::Error) -> RuntimeError {
    RuntimeError::Application("Agent avatar is unavailable".into())
}
