//! Agent home archives for moving an Agent between Machines.
//!
//! The source packs the home once and serves it in chunks; the target
//! collects chunks, checks the whole archive, then swaps it in as the home.
use super::connection::send_frame;
use super::{Inner, MachineError};
use crate::resources::agent_directory;
use meerkat_protocol::{AgentFilesResult, AgentRef, MachineFrame, machine_frame};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Rebuilt or downloaded again on the target, so never moved.
const LEFT_BEHIND: [&str; 2] = ["AGENTS.md", "resources"];
const MAX_READ: u64 = 1024 * 1024;

#[derive(Default)]
struct Answer {
    size: u64,
    sha256: String,
    bytes: Vec<u8>,
}

pub(super) async fn handle_export(inner: Arc<Inner>, request: meerkat_protocol::ExportAgentFiles) {
    let result = async {
        let agent = agent(request.agent.as_ref())?;
        super::sessions::close(
            &inner,
            &format!("{}/{}", agent.workspace_id, agent.agent_id),
        )
        .await;
        let home = agent_directory(&inner.root, &agent.workspace_id, &agent.agent_id)?;
        let archive = archive_path(&inner.root, "out", agent)?;
        let (size, sha256) = blocking(move || pack(&home, &archive)).await?;
        Ok(Answer {
            size,
            sha256,
            ..Answer::default()
        })
    }
    .await;
    reply(&inner, request.request_id, result).await;
}

pub(super) async fn handle_read(inner: Arc<Inner>, request: meerkat_protocol::ReadAgentFiles) {
    let result = async {
        let archive = archive_path(&inner.root, "out", agent(request.agent.as_ref())?)?;
        let (offset, length) = (request.offset, request.length.min(MAX_READ));
        let bytes = blocking(move || {
            let mut file = File::open(&archive)?;
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = Vec::new();
            file.take(length).read_to_end(&mut bytes)?;
            Ok(bytes)
        })
        .await?;
        Ok(Answer {
            bytes,
            ..Answer::default()
        })
    }
    .await;
    reply(&inner, request.request_id, result).await;
}

pub(super) async fn handle_write(inner: Arc<Inner>, request: meerkat_protocol::WriteAgentFiles) {
    let result = async {
        let agent = agent(request.agent.as_ref())?.clone();
        let incoming = archive_path(&inner.root, "in", &agent)?;
        let home = agent_directory(&inner.root, &agent.workspace_id, &agent.agent_id)?;
        let key = format!("{}/{}", agent.workspace_id, agent.agent_id);
        if request.eof {
            // Nothing of this Agent may run here while its home is replaced.
            super::sessions::close(&inner, &key).await;
            if inner.starting.lock().await.contains_key(&key)
                || inner
                    .active
                    .lock()
                    .await
                    .values()
                    .any(|active| active.agent_key == key)
            {
                return Err(MachineError::InvalidAssignment("agent_busy".into()));
            }
        }
        blocking(move || {
            append(&incoming, request.offset, &request.bytes)?;
            if request.eof {
                verify(&incoming, request.size, &request.sha256)?;
                install(&incoming, &home)?;
                fs::remove_file(&incoming)?;
            }
            Ok(())
        })
        .await?;
        Ok(Answer::default())
    }
    .await;
    reply(&inner, request.request_id, result).await;
}

pub(super) async fn handle_release(
    inner: Arc<Inner>,
    request: meerkat_protocol::ReleaseAgentFiles,
) {
    let result = match agent(request.agent.as_ref()) {
        Ok(agent) => release(&inner.root, agent).map(|()| Answer::default()),
        Err(error) => Err(error),
    };
    reply(&inner, request.request_id, result).await;
}

/// Removes every archive kept for the Agent, sent or received.
pub(super) fn release(root: &Path, agent: &AgentRef) -> Result<(), MachineError> {
    for kind in ["out", "in"] {
        let path = archive_path(root, kind, agent)?;
        for file in [path.clone(), path.with_extension("part")] {
            match fs::remove_file(file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
                _ => {}
            }
        }
    }
    Ok(())
}

fn agent(agent: Option<&AgentRef>) -> Result<&AgentRef, MachineError> {
    agent.ok_or_else(|| MachineError::InvalidAssignment("missing_agent".into()))
}

fn archive_path(root: &Path, kind: &str, agent: &AgentRef) -> Result<PathBuf, MachineError> {
    // Validates both IDs the same way the Agent home does.
    agent_directory(root, &agent.workspace_id, &agent.agent_id)?;
    Ok(root
        .join(".machine/relocations")
        .join(kind)
        .join(&agent.workspace_id)
        .join(format!("{}.zip", agent.agent_id)))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, MachineError> + Send + 'static,
) -> Result<T, MachineError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| MachineError::PersistenceFailed)?
}

async fn reply(inner: &Inner, request_id: String, result: Result<Answer, MachineError>) {
    let result = match result {
        Ok(answer) => AgentFilesResult {
            request_id,
            success: true,
            size: answer.size,
            sha256: answer.sha256,
            bytes: answer.bytes,
            ..AgentFilesResult::default()
        },
        Err(error) => {
            log::error!("[machine] Agent files request failed: {error}");
            AgentFilesResult {
                request_id,
                success: false,
                error_code: super::assignment::error_code(&error).into(),
                error_message: super::assignment::public_error(&error),
                ..AgentFilesResult::default()
            }
        }
    };
    send_frame(
        inner,
        MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::AgentFilesResult(result)),
        },
    )
    .await;
}

fn pack(home: &Path, archive: &Path) -> Result<(u64, String), MachineError> {
    fs::create_dir_all(archive.parent().expect("archive has a parent"))?;
    let partial = archive.with_extension("part");
    let mut writer = zip::ZipWriter::new(File::create(&partial)?);
    if home.is_dir() {
        add_directory(&mut writer, home, home)?;
    }
    let file = writer.finish().map_err(zip_error)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, archive)?;
    digest(archive)
}

fn add_directory(
    writer: &mut zip::ZipWriter<File>,
    home: &Path,
    directory: &Path,
) -> Result<(), MachineError> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(home).expect("entry inside home");
        if directory == home && LEFT_BEHIND.iter().any(|name| entry.file_name() == *name) {
            continue;
        }
        let Some(name) = archive_name(relative) else {
            continue;
        };
        let metadata = fs::symlink_metadata(&path)?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(permissions(&metadata));
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            // A link that leads outside the home would point somewhere else on the target.
            if stays_inside(relative, &target) {
                writer
                    .add_symlink(name, target.to_string_lossy(), options)
                    .map_err(zip_error)?;
            }
        } else if metadata.is_dir() {
            writer
                .add_directory(format!("{name}/"), options)
                .map_err(zip_error)?;
            add_directory(writer, home, &path)?;
        } else if metadata.is_file() {
            writer
                .start_file(name, options.large_file(metadata.len() >= u32::MAX as u64))
                .map_err(zip_error)?;
            io::copy(&mut File::open(&path)?, writer)?;
        }
    }
    Ok(())
}

fn archive_name(relative: &Path) -> Option<String> {
    relative
        .components()
        .map(|part| match part {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

/// True when a link at `link` (relative to the home) resolves inside the home.
fn stays_inside(link: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth = link.components().count() as i64 - 1;
    for part in target.components() {
        match part {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            _ => return false,
        }
        if depth < 0 {
            return false;
        }
    }
    true
}

#[cfg(unix)]
fn permissions(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn permissions(metadata: &fs::Metadata) -> u32 {
    if metadata.is_dir() { 0o755 } else { 0o644 }
}

fn append(path: &Path, offset: u64, bytes: &[u8]) -> Result<(), MachineError> {
    fs::create_dir_all(path.parent().expect("archive has a parent"))?;
    let mut file = if offset == 0 {
        File::create(path)?
    } else {
        let file = OpenOptions::new().append(true).open(path)?;
        if file.metadata()?.len() != offset {
            return Err(MachineError::InvalidAssignment(
                "archive_offset_mismatch".into(),
            ));
        }
        file
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn verify(path: &Path, size: u64, sha256: &str) -> Result<(), MachineError> {
    let (actual_size, actual) = digest(path)?;
    if actual_size != size || !actual.eq_ignore_ascii_case(sha256) {
        let _ = fs::remove_file(path);
        return Err(MachineError::ResourceIntegrityMismatch);
    }
    Ok(())
}

fn digest(path: &Path) -> Result<(u64, String), MachineError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        size += read as u64;
    }
    let hash = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((size, hash))
}

/// Unpacks next to the home, then swaps it in so a failed unpack leaves the
/// previous home untouched.
fn install(archive: &Path, home: &Path) -> Result<(), MachineError> {
    let parent = home.parent().expect("Agent home has a parent");
    fs::create_dir_all(parent)?;
    let name = home
        .file_name()
        .and_then(|name| name.to_str())
        .expect("Agent home has a name");
    let staged = parent.join(format!(".{name}.incoming"));
    let replaced = parent.join(format!(".{name}.replaced"));
    for leftover in [&staged, &replaced] {
        if leftover.exists() {
            fs::remove_dir_all(leftover)?;
        }
    }
    fs::create_dir_all(&staged)?;
    if let Err(error) = unpack(archive, &staged) {
        let _ = fs::remove_dir_all(&staged);
        return Err(error);
    }
    if home.exists() {
        fs::rename(home, &replaced)?;
    }
    fs::rename(&staged, home)?;
    if replaced.exists() {
        fs::remove_dir_all(&replaced)?;
    }
    Ok(())
}

fn unpack(archive: &Path, target: &Path) -> Result<(), MachineError> {
    let mut archive = zip::ZipArchive::new(File::open(archive)?).map_err(zip_error)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| MachineError::InvalidAssignment("archive_path_outside_home".into()))?;
        let path = target.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            set_permissions(&path, entry.unix_mode())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if entry.is_symlink() {
            let mut link = String::new();
            entry.read_to_string(&mut link)?;
            if stays_inside(&relative, Path::new(&link)) {
                symlink(Path::new(&link), &path)?;
            }
            continue;
        }
        io::copy(&mut entry, &mut File::create(&path)?)?;
        set_permissions(&path, entry.unix_mode())?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: Option<u32>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match mode {
        Some(mode) => fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777)),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
fn set_permissions(_: &Path, _: Option<u32>) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> io::Result<()> {
    Ok(())
}

fn zip_error(error: zip::result::ZipError) -> MachineError {
    MachineError::Io(io::Error::other(error))
}
