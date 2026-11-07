//! What the Machine knows about each native CLI. Starting the Machine only
//! checks that an executable resolves; versions and model lists are probed
//! when a person asks for them or an Agent starts, and kept on disk.

use meerkat_harness::{
    HarnessAvailability, HarnessConfig, HarnessError, HarnessFactory, ProbeResult,
};
use meerkat_protocol::ModelCatalog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

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
    /// The last model list; it outlives CLI updates so names keep showing.
    #[serde(default)]
    models: Option<ModelCatalog>,
}

impl Entry {
    fn new(config: &HarnessConfig) -> Self {
        Self {
            harness_id: config.harness_id.clone(),
            cli_path: config.cli_path.clone(),
            search_path: search_path(config),
            stamp: None,
            version: None,
            models: None,
        }
    }
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

    /// Inventory for a CLI without starting it: present or not, plus what the
    /// last probe of this same executable found.
    pub fn snapshot(&self, factory: &dyn HarnessFactory, config: &HarnessConfig) -> ProbeResult {
        let current = stamp(&config.cli_path, search_path(config).as_deref());
        let entries = self.entries.lock().unwrap();
        let known = entries
            .get(&key(config))
            .filter(|entry| current.is_some() && entry.stamp == current);
        ProbeResult {
            info: factory.info().clone(),
            availability: if current.is_some() {
                HarnessAvailability::Available
            } else {
                HarnessAvailability::Unavailable
            },
            version: known.and_then(|entry| entry.version.clone()),
            capabilities: factory.capabilities(config),
            reason: current.is_none().then(|| "not installed".into()),
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

    pub fn models(&self, config: &HarnessConfig) -> Option<ModelCatalog> {
        self.entries
            .lock()
            .unwrap()
            .get(&key(config))
            .and_then(|entry| entry.models.clone())
    }

    pub fn set_models(&self, config: &HarnessConfig, models: ModelCatalog) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(key(config))
            .or_insert_with(|| Entry::new(config));
        entry.models = Some(models);
        self.save(&entries);
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
        let entry = entries
            .entry(key(config))
            .or_insert_with(|| Entry::new(config));
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
