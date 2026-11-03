//! Native CLI versions. The Machine detects them when it starts and keeps
//! them on disk, so an Agent starts with a known version. A background check
//! probes a CLI again only when its executable file changes.

use super::Inner;
use meerkat_harness::{HarnessConfig, HarnessError, HarnessFactory, ProbeResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, UNIX_EPOCH};

pub(super) const CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Identifies the executable file a CLI path currently resolves to.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Entry {
    harness_id: String,
    cli_path: PathBuf,
    /// PATH used to resolve a bare command name.
    search_path: Option<String>,
    /// `None` until a probe succeeds for the current file.
    stamp: Option<Stamp>,
    version: Option<String>,
}

pub(super) struct CliVersions {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, Entry>>,
}

impl CliVersions {
    pub fn load(path: PathBuf) -> Self {
        let entries = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            entries: Mutex::new(entries),
        }
    }

    /// The recorded version, probing first when the executable is new or changed.
    pub async fn version(
        &self,
        factory: &dyn HarnessFactory,
        config: &HarnessConfig,
    ) -> Option<String> {
        let key = key(config);
        let current = stamp(&config.cli_path, search_path(config).as_deref());
        if let Some(entry) = self.entries.lock().unwrap().get(&key)
            && current.is_some()
            && entry.stamp == current
        {
            return entry.version.clone();
        }
        match self.probe(factory, config).await {
            Ok(probe) => probe.version,
            Err(_) => self
                .entries
                .lock()
                .unwrap()
                .get(&key)
                .and_then(|entry| entry.version.clone()),
        }
    }

    /// Probe a CLI and record its version.
    pub async fn probe(
        &self,
        factory: &dyn HarnessFactory,
        config: &HarnessConfig,
    ) -> Result<ProbeResult, HarnessError> {
        let search_path = search_path(config);
        // Stamp before probing: a file replaced during the probe is checked again.
        let current = stamp(&config.cli_path, search_path.as_deref());
        let result = factory.probe(config).await;
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.entry(key(config)).or_insert_with(|| Entry {
            harness_id: config.harness_id.clone(),
            cli_path: config.cli_path.clone(),
            search_path: search_path.clone(),
            stamp: None,
            version: None,
        });
        entry.search_path = search_path;
        match &result {
            Ok(probe) => {
                entry.stamp = current;
                entry.version = probe.version.clone();
            }
            // Keep the last known version and try again on the next check.
            Err(_) => entry.stamp = None,
        }
        self.save(&entries);
        result
    }

    fn save(&self, entries: &BTreeMap<String, Entry>) {
        let result = (|| {
            let parent = self
                .path
                .parent()
                .ok_or(std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
            std::fs::create_dir_all(parent)?;
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            serde_json::to_writer(&mut file, entries)?;
            file.persist(&self.path)?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })();
        if let Err(error) = result {
            log::warn!("[machine] CLI versions were not saved: {error}");
        }
    }

    /// Configurations whose executable changed or was never probed successfully.
    fn changed(&self) -> Vec<(String, PathBuf)> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .filter(|entry| {
                entry.stamp.is_none()
                    || stamp(&entry.cli_path, entry.search_path.as_deref()) != entry.stamp
            })
            .map(|entry| (entry.harness_id.clone(), entry.cli_path.clone()))
            .collect()
    }
}

/// Re-probe CLIs whose executable changed, until the Machine stops.
pub(super) async fn watch(inner: Weak<Inner>) {
    loop {
        tokio::time::sleep(CHECK_INTERVAL).await;
        let Some(inner) = inner.upgrade() else { return };
        if inner.stop_tx.borrow().is_some() {
            return;
        }
        for (harness_id, cli_path) in inner.cli_versions.changed() {
            let (Some(factory), Some(defaults)) = (
                inner.factories.get(&harness_id).map(Arc::clone),
                inner.defaults.get(&harness_id),
            ) else {
                continue;
            };
            let config = HarnessConfig {
                cli_path,
                ..defaults.clone()
            };
            let _ = inner.cli_versions.probe(factory.as_ref(), &config).await;
        }
    }
}

fn key(config: &HarnessConfig) -> String {
    format!("{}:{}", config.harness_id, config.cli_path.display())
}

fn search_path(config: &HarnessConfig) -> Option<String> {
    config
        .environment
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
}

fn stamp(cli_path: &Path, search_path: Option<&str>) -> Option<Stamp> {
    let resolved = if cli_path.components().count() > 1 {
        cli_path.to_owned()
    } else {
        std::env::split_paths(search_path?)
            .map(|directory| directory.join(cli_path))
            .find(|candidate| candidate.is_file())?
    };
    let path = resolved.canonicalize().ok()?;
    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(Stamp {
        path,
        size: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}
