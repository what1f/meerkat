use super::{MachineError, OutputRecordPolicy};
use lankide_harness::HarnessConfig;
use lankide_protocol::ExecutionRef;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

pub(super) fn prepare(
    root: &Path,
    policy: &OutputRecordPolicy,
    execution: &ExecutionRef,
    config: &mut HarnessConfig,
) -> Result<(), MachineError> {
    if config.harness_id != "pi" {
        return Ok(());
    }
    let directory = root
        .join(".machine/diagnostics")
        .join(format!("{}-{}", execution.execution_id, execution.attempt));
    fs::create_dir_all(&directory)?;
    let record = directory.join("rpc.jsonl");
    config.environment.extend([
        (
            "LANKIDE_PI_ARTIFACT_DIR".into(),
            directory.display().to_string(),
        ),
        (
            "LANKIDE_PI_RECORD_PATH".into(),
            record.display().to_string(),
        ),
        (
            "LANKIDE_PI_RECORD_MAX_BYTES".into(),
            policy.max_bytes.to_string(),
        ),
    ]);
    fs::write(
        directory.join("diagnostics.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "execution_id": execution.execution_id,
            "attempt": execution.attempt,
            "output_record": record,
            "previous_output_record": record.with_extension("jsonl.1"),
        }))
        .map_err(|_| MachineError::PersistenceFailed)?,
    )?;
    Ok(())
}

pub(super) fn expire(root: &Path, retention: Duration) -> Result<(), MachineError> {
    let now = SystemTime::now();
    let root = root.canonicalize()?;
    let diagnostics = root.join(".machine/diagnostics");
    if contained_directory(&root, &diagnostics)? {
        expire_directory(&diagnostics, retention, now)?;
    }
    Ok(())
}

fn contained_directory(root: &Path, path: &Path) -> std::io::Result<bool> {
    if !path.try_exists()? {
        return Ok(false);
    }
    Ok(path.canonicalize()?.starts_with(root))
}

fn expire_directory(path: &Path, retention: Duration, now: SystemTime) -> std::io::Result<()> {
    if !path.try_exists()? || fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            expire_directory(&entry.path(), retention, now)?;
            if fs::read_dir(entry.path())?.next().is_none() {
                fs::remove_dir(entry.path())?;
            }
        } else if kind.is_file() {
            if now
                .duration_since(entry.metadata()?.modified()?)
                .unwrap_or_default()
                > retention
            {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}
