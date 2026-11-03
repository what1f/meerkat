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
        std::os::windows::fs::symlink_dir(source, destination)
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
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if !metadata.file_type().is_symlink() {
            return Ok(());
        }
        let target = source.canonicalize().ok();
        if target.is_some() && fs::read_link(destination).ok() == target {
            return Ok(());
        }
        fs::remove_file(destination).map_err(|e| HarnessError::invalid_config(e.to_string()))?;
    }
    if !source.exists() {
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
            std::os::windows::fs::symlink_dir(source, destination)
                .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        } else {
            std::os::windows::fs::symlink_file(source, destination)
                .map_err(|e| HarnessError::invalid_config(e.to_string()))?;
        }
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
