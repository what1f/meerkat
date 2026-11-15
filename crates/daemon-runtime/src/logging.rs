#[cfg(unix)]
use super::PRIVATE_FILE_MODE;
use super::create_private_directory;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 4 * 1024 * 1024;

struct DaemonLog {
    path: PathBuf,
    writer: Mutex<File>,
}

fn open_log(path: &Path) -> io::Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && !metadata.is_file()
    {
        return Err(io::Error::other("unsafe daemon log"));
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(PRIVATE_FILE_MODE);
    let file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
    Ok(file)
}

/// Install the process logger. Diagnostics also remain visible on stderr.
/// Timestamps are Unix seconds with millisecond precision (UTC).
pub fn init_logging(home: &Path, daemon: &str) -> io::Result<()> {
    create_private_directory(home).map_err(io::Error::other)?;
    let directory = home.join("logs");
    create_private_directory(&directory).map_err(io::Error::other)?;
    let path = directory.join(format!("{daemon}.log"));
    let writer = Mutex::new(open_log(&path)?);
    log::set_boxed_logger(Box::new(DaemonLog { path, writer })).map_err(io::Error::other)?;
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}

impl log::Log for DaemonLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let result = (|| -> io::Result<()> {
            let mut writer = self
                .writer
                .lock()
                .map_err(|_| io::Error::other("daemon log lock"))?;
            for line in record.args().to_string().lines() {
                // Bound individual diagnostics as well as the total file size.
                let line: String = line.chars().take(16 * 1024).collect();
                let line = format!(
                    "{}.{:03}Z {} {line}\n",
                    timestamp.as_secs(),
                    timestamp.subsec_millis(),
                    record.level()
                );
                eprint!("{line}");
                if writer.metadata()?.len() + line.len() as u64 > MAX_BYTES {
                    fs::rename(&self.path, self.path.with_extension("log.1"))?;
                    *writer = open_log(&self.path)?;
                }
                writer.write_all(line.as_bytes())?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Could not write daemon log: {error}");
        }
    }

    fn flush(&self) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.flush();
        }
    }
}
