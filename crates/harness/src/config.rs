use crate::HarnessError;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

fn create_native_directory(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Make Agent-authored skills available to a native project skill scanner.
/// Link the directory so skills written during a run are visible on the next
/// scan. Preserve an existing native directory and its entries when merging.
pub fn prepare_native_skills(
    agent_home: &Path,
    native_directory: &Path,
) -> Result<(), HarnessError> {
    let prepare = || -> io::Result<()> {
        let skills = agent_home.join("skills");
        fs::create_dir_all(&skills)?;
        let skills = skills.canonicalize()?;
        fs::create_dir_all(native_directory.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid native skills directory",
            )
        })?)?;
        match fs::symlink_metadata(native_directory) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                link_directory(&skills, native_directory)?;
            }
            Ok(_) if native_directory.canonicalize()? == skills => {}
            Ok(_) => {
                for entry in fs::read_dir(&skills)? {
                    let entry = entry?;
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let destination = native_directory.join(entry.file_name());
                    match fs::symlink_metadata(&destination) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            link_directory(&entry.path(), &destination)?;
                        }
                        Err(error) => return Err(error),
                        Ok(_) => {}
                    }
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    };
    prepare().map_err(|error| {
        HarnessError::invalid_config(format!("Could not prepare native skills: {error}"))
    })
}

fn link_directory(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination)
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(source, destination) {
            Ok(()) => Ok(()),
            Err(_) => {
                let raw = source.to_string_lossy();
                if raw.starts_with(r"\\?\UNC\")
                    || (raw.starts_with(r"\\") && !raw.starts_with(r"\\?\"))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "directory junction does not support remote UNC paths",
                    ));
                }
                let target = raw.strip_prefix(r"\\?\").map(Path::new).unwrap_or(source);
                junction::create(target, destination)
            }
        }
    }
}

/// Refresh inherited configuration while keeping native sessions and local state.
/// Credentials remain native shared files, so token refresh cannot be rolled back
/// by copying an old snapshot over a freshly renewed token.
pub(crate) fn refresh_config_overlay(
    destination: &Path,
    source: Option<&Path>,
    files: &[&str],
) -> Result<(), HarnessError> {
    create_native_directory(destination)
        .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    let Some(source) = source else { return Ok(()) };
    if source == destination {
        return Ok(());
    }
    let manifest = destination.join(".meerkat-inherited.json");
    let previous: Vec<String> = fs::read(&manifest)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut inherited = Vec::new();
    for name in files {
        if Path::new(name).components().count() != 1 {
            return Err(HarnessError::invalid_config(
                "Invalid native configuration filename",
            ));
        }
        let origin = source.join(name);
        let target = destination.join(name);
        let apply = || -> io::Result<bool> {
            if !origin.exists() {
                if previous.iter().any(|v| v == name) {
                    let _ = fs::remove_file(&target);
                }
                return Ok(false);
            }
            let credentials = matches!(*name, "auth.json" | ".credentials.json");
            if credentials {
                if target.exists() && !previous.iter().any(|v| v == name) {
                    return Ok(false);
                }
                if fs::symlink_metadata(&target).is_ok() {
                    fs::remove_file(&target)?;
                }
                #[cfg(unix)]
                std::os::unix::fs::symlink(origin.canonicalize()?, &target)?;
                #[cfg(not(unix))]
                fs::copy(&origin, &target)?;
            } else {
                let temporary = destination.join(format!(".{}-{}", name, uuid::Uuid::new_v4()));
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&temporary)?;
                file.write_all(&fs::read(&origin)?)?;
                file.sync_all()?;
                drop(file);
                fs::rename(temporary, &target)?;
            }
            Ok(true)
        };
        let applied = apply().map_err(|e| {
            HarnessError::invalid_config(format!("Could not refresh native configuration: {e}"))
        })?;
        if applied {
            inherited.push(name.to_string());
        }
    }
    fs::write(
        manifest,
        serde_json::to_vec(&inherited).expect("configuration names"),
    )
    .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    Ok(())
}

pub(crate) fn link_native_resource(source: &Path, destination: &Path) -> Result<(), HarnessError> {
    #[cfg(windows)]
    let is_junction = junction::exists(destination).unwrap_or(false);
    #[cfg(not(windows))]
    let is_junction = false;

    if let Ok(metadata) = fs::symlink_metadata(destination) {
        let is_link = metadata.file_type().is_symlink() || is_junction;
        if !is_link {
            #[cfg(windows)]
            return sync_copied_windows_resource(source, destination);
            #[cfg(not(windows))]
            return Ok(());
        }
        let target = source.canonicalize().ok();
        let current = if is_junction {
            #[cfg(windows)]
            {
                junction::get_target(destination)
                    .ok()
                    .and_then(|path| path.canonicalize().ok())
            }
            #[cfg(not(windows))]
            {
                None
            }
        } else {
            fs::read_link(destination).ok()
        };
        if target.is_some() && current == target {
            return Ok(());
        }
        #[cfg(windows)]
        let remove = {
            use std::os::windows::fs::FileTypeExt;
            if is_junction || metadata.file_type().is_symlink_dir() {
                fs::remove_dir(destination)
            } else {
                fs::remove_file(destination)
            }
        };
        #[cfg(not(windows))]
        let remove = fs::remove_file(destination);
        remove.map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    }
    if !source.exists() {
        #[cfg(windows)]
        let _ = remove_copied_windows_record(destination);
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            source
                .canonicalize()
                .map_err(|e| HarnessError::invalid_config(e.to_string()))?,
            destination,
        )
        .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    }
    #[cfg(windows)]
    {
        if source.is_dir() {
            link_directory(source, destination)
                .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        } else if std::os::windows::fs::symlink_file(source, destination).is_err() {
            copy_tracked_windows_resource(source, destination)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn copied_manifest_path(destination: &Path) -> Option<std::path::PathBuf> {
    destination
        .parent()
        .map(|parent| parent.join(".meerkat-copied-resources.json"))
}

#[cfg(windows)]
fn file_sha256(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path)?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(windows)]
fn read_copied_manifest(path: &Path) -> std::collections::BTreeMap<String, String> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(windows)]
fn write_copied_manifest(
    path: &Path,
    entries: &std::collections::BTreeMap<String, String>,
) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(entries).expect("copied resource manifest"))?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary, path)
}

#[cfg(windows)]
fn copy_tracked_windows_resource(source: &Path, destination: &Path) -> Result<(), HarnessError> {
    fs::copy(source, destination).map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    if let (Some(manifest_path), Some(name)) = (
        copied_manifest_path(destination),
        destination.file_name().and_then(|value| value.to_str()),
    ) {
        let hash =
            file_sha256(destination).map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        let mut entries = read_copied_manifest(&manifest_path);
        entries.insert(name.to_owned(), hash);
        write_copied_manifest(&manifest_path, &entries)
            .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    }
    Ok(())
}

#[cfg(windows)]
fn remove_copied_windows_record(destination: &Path) -> io::Result<()> {
    if let (Some(manifest_path), Some(name)) = (
        copied_manifest_path(destination),
        destination.file_name().and_then(|value| value.to_str()),
    ) {
        let mut entries = read_copied_manifest(&manifest_path);
        if entries.remove(name).is_some() {
            write_copied_manifest(&manifest_path, &entries)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn sync_copied_windows_resource(source: &Path, destination: &Path) -> Result<(), HarnessError> {
    let (Some(manifest_path), Some(name)) = (
        copied_manifest_path(destination),
        destination.file_name().and_then(|value| value.to_str()),
    ) else {
        return Ok(());
    };
    let mut entries = read_copied_manifest(&manifest_path);
    let Some(recorded_hash) = entries.get(name).cloned() else {
        // User-created file: never overwrite or delete.
        return Ok(());
    };
    let current_hash = match file_sha256(destination) {
        Ok(hash) => hash,
        Err(_) => return Ok(()),
    };
    if current_hash != recorded_hash {
        // User modified the copy: preserve local edits.
        return Ok(());
    }
    if !source.exists() {
        match fs::remove_file(destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Ok(()),
        }
        entries.remove(name);
        write_copied_manifest(&manifest_path, &entries)
            .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        return Ok(());
    }
    let source_hash =
        file_sha256(source).map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    if source_hash != recorded_hash {
        copy_tracked_windows_resource(source, destination)?;
    }
    Ok(())
}

/// Rebase only path fields named by the adapter; commands and other settings stay untouched.
pub(crate) fn rebase_native_config(
    path: &Path,
    source: &Path,
    keys: &[&str],
) -> Result<(), HarnessError> {
    if keys.is_empty() || !path.is_file() {
        return Ok(());
    }
    let apply = || -> Result<(), Box<dyn std::error::Error>> {
        let text = fs::read_to_string(path)?;
        if text.trim().is_empty() {
            return Ok(());
        }
        let is_toml = path.extension().is_some_and(|v| v == "toml");
        let mut value: serde_json::Value = if is_toml {
            serde_json::to_value(toml::from_str::<toml::Value>(&text)?)?
        } else if path.extension().is_some_and(|v| v == "json") {
            serde_json::from_str(&text)?
        } else {
            return Ok(());
        };
        fn rebase(
            value: &mut serde_json::Value,
            source: &Path,
            keys: &[&str],
            selected: bool,
        ) -> bool {
            match value {
                serde_json::Value::Object(entries) => {
                    entries.iter_mut().fold(false, |changed, (key, value)| {
                        rebase(value, source, keys, keys.contains(&key.as_str())) || changed
                    })
                }
                serde_json::Value::Array(entries) => {
                    entries.iter_mut().fold(false, |changed, value| {
                        rebase(value, source, keys, selected) || changed
                    })
                }
                serde_json::Value::String(text) if selected && !text.is_empty() => {
                    let (prefix, relative) = text
                        .strip_prefix('!')
                        .map(|s| ("!", s))
                        .unwrap_or(("", text.as_str()));
                    if relative.starts_with('~')
                        || relative.contains(':')
                        || Path::new(relative).is_absolute()
                    {
                        return false;
                    }
                    *text = format!("{prefix}{}", source.join(relative).display());
                    true
                }
                _ => false,
            }
        }
        if rebase(&mut value, source, keys, false) {
            let text = if is_toml {
                toml::to_string(&value)?
            } else {
                serde_json::to_string_pretty(&value)?
            };
            fs::write(path, text)?;
        }
        Ok(())
    };
    apply().map_err(|error| {
        HarnessError::invalid_config(format!(
            "Could not preserve native configuration paths: {error}"
        ))
    })
}
