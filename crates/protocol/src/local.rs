//! Owner-only local endpoints for Agent capability calls.
//!
//! An endpoint is named by a path inside a private run directory. On Unix the
//! path is the socket itself. On Windows it names a local named pipe that only
//! the owning user can open; the path stays the identity callers pass around.

use interprocess::local_socket::{
    ListenerOptions, Name,
    tokio::{Listener, Stream, prelude::*},
};
use std::{io, path::Path};

pub use interprocess::local_socket::tokio::Stream as LocalStream;

pub struct LocalListener {
    inner: Listener,
}

impl LocalListener {
    /// Binds a fresh endpoint and creates its parent directory owner-only.
    /// A leftover endpoint at the same path is replaced.
    pub fn bind(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("socket_path_has_no_parent"))?;
        private_directory(parent)?;
        // The explicit remove after shutdown owns cleanup. Automatic reclaim
        // would let a late-dropped listener unlink a newer one at the same path.
        let options = ListenerOptions::new().name(name(path)?).reclaim_name(false);
        #[cfg(unix)]
        let inner = {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::symlink_metadata(path).is_ok() {
                std::fs::remove_file(path)?;
            }
            let inner = options.create_tokio()?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            inner
        };
        #[cfg(windows)]
        let inner = {
            use interprocess::os::windows::{
                local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
            };
            // Only SYSTEM and the current user may open it. The user SID is
            // named explicitly: an elevated token's default owner can be the
            // Administrators group rather than the user.
            let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", current_user_sid()?);
            let sddl = widestring::U16CString::from_str(sddl).map_err(io::Error::other)?;
            let owner_only = SecurityDescriptor::deserialize(&sddl)?;
            options.security_descriptor(owner_only).create_tokio()?
        };
        Ok(Self { inner })
    }

    pub async fn accept(&self) -> io::Result<LocalStream> {
        self.inner.accept().await
    }
}

pub async fn connect(path: &Path) -> io::Result<LocalStream> {
    Stream::connect(name(path)?).await
}

/// Removes what a listener leaves on disk once it has stopped.
pub fn remove(path: &Path) {
    #[cfg(unix)]
    let _ = std::fs::remove_file(path);
    #[cfg(windows)]
    let _ = path;
}

#[cfg(unix)]
fn name(path: &Path) -> io::Result<Name<'_>> {
    use interprocess::local_socket::{GenericFilePath, ToFsName};
    path.to_fs_name::<GenericFilePath>()
}

#[cfg(windows)]
fn name(path: &Path) -> io::Result<Name<'static>> {
    use interprocess::local_socket::{GenericNamespaced, ToNsName};
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("meerkat-{hex}").to_ns_name::<GenericNamespaced>()
}

#[cfg(windows)]
fn current_user_sid() -> io::Result<&'static str> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    static SID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SID.get_or_init(|| {
        let output = std::process::Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let listing = String::from_utf8_lossy(&output.stdout);
        let sid = listing.trim().rsplit(',').next()?.trim_matches('"');
        (output.status.success() && sid.starts_with("S-1-")).then(|| sid.to_owned())
    })
    .as_deref()
    .ok_or_else(|| io::Error::other("current_user_sid_unavailable"))
}

fn private_directory(directory: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
    }
    // The run directory sits inside the Machine home, which is restricted to
    // the current user when it is created.
    #[cfg(windows)]
    std::fs::create_dir_all(directory)
}
