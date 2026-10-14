use crate::{HarnessError, HarnessErrorCode};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Seed a private native configuration directory without copying session history.
/// Existing local files remain authoritative, including refreshed credentials.
pub fn prepare_config_overlay(
    destination: &Path,
    source: Option<&Path>,
    files: &[&str],
) -> Result<(), HarnessError> {
    let prepare = || -> io::Result<()> {
        fs::create_dir_all(destination)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(destination, fs::Permissions::from_mode(0o700))?;
        }
        let Some(source) = source else { return Ok(()) };
        for name in files {
            let name = Path::new(name);
            if name.components().count() != 1
                || !matches!(
                    name.components().next(),
                    Some(std::path::Component::Normal(_))
                )
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid config file name",
                ));
            }
            let target = destination.join(name);
            if target.try_exists()? {
                continue;
            }
            let bytes = match fs::read(source.join(name)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&target) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
                        let _ = fs::remove_file(&target);
                        return Err(error);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    };
    prepare().map_err(|error| {
        HarnessError::new(
            HarnessErrorCode::InvalidConfig,
            format!("Could not prepare native configuration: {error}"),
        )
    })
}

pub fn user_config_directory(variable: &str, home_relative: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(home_relative)))
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
