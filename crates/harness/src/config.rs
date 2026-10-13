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
