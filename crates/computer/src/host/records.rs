use super::{ComputerError, OutputRecordPolicy};
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
) -> Result<(), ComputerError> {
    if config.harness_id != "pi" {
        return Ok(());
    }
    let directory = root
        .join(".computer/diagnostics")
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
        .map_err(|_| ComputerError::PersistenceFailed)?,
    )?;
    Ok(())
}

pub(super) fn expire(root: &Path, retention: Duration) -> Result<(), ComputerError> {
    let now = SystemTime::now();
    let root = root.canonicalize()?;
    let diagnostics = root.join(".computer/diagnostics");
    if contained_directory(&root, &diagnostics)? {
        expire_directory(&diagnostics, retention, now, true)?;
    }
    // Old adapters put output beside native session history. Remove only records.
    for workspace in directories(&root)? {
        for agent in directories(&workspace.join("agents"))? {
            for directory in [".lankide/pi/sessions", ".lankide/pi/records"] {
                let directory = agent.join(directory);
                if contained_directory(&root, &directory)? {
                    expire_directory(&directory, retention, now, false)?;
                }
            }
        }
    }
    Ok(())
}

fn contained_directory(root: &Path, path: &Path) -> std::io::Result<bool> {
    if !path.try_exists()? {
        return Ok(false);
    }
    Ok(path.canonicalize()?.starts_with(root))
}

fn directories(path: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    if !path.try_exists()? || fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(Vec::new());
    }
    fs::read_dir(path)?
        .filter_map(|entry| match entry {
            Ok(entry) => match entry.file_type() {
                Ok(kind) if kind.is_dir() => Some(Ok(entry.path())),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            },
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn expire_directory(
    path: &Path,
    retention: Duration,
    now: SystemTime,
    owned: bool,
) -> std::io::Result<()> {
    if !path.try_exists()? || fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() && owned {
            expire_directory(&entry.path(), retention, now, owned)?;
            if fs::read_dir(entry.path())?.next().is_none() {
                fs::remove_dir(entry.path())?;
            }
        } else if kind.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let record = owned
                || (name.starts_with("pi-")
                    && (name.ends_with(".rpc.jsonl")
                        || name.ends_with(".rpc.jsonl.1")
                        || name.ends_with(".stderr.log")));
            if record
                && now
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
