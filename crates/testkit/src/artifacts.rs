//! Durable evidence for test scenarios, separate from all product dependencies.

use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct ScenarioArtifact {
    pub marker: String,
    pub dir: PathBuf,
    journal: Mutex<File>,
    journal_failed: AtomicBool,
    cleanup_failed: AtomicBool,
    cleanup_complete: AtomicBool,
}

impl ScenarioArtifact {
    pub fn new(root: &Path, suite: &str, run_id: &str) -> Result<Self, String> {
        for value in [suite, run_id] {
            if !value
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
                || value.len() > 80
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            {
                return Err("invalid scenario artifact identifier".into());
            }
        }
        let parent = root.join(suite);
        fs::create_dir_all(&parent).map_err(|_| "cannot create artifact parent")?;
        let dir = parent.join(run_id);
        fs::create_dir(&dir).map_err(|_| "cannot reserve a fresh artifact directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
                .map_err(|_| "cannot protect artifact directory")?;
        }
        let journal = private_file(&dir.join("cleanup.jsonl"))?;
        let artifact = Self {
            marker: format!("[meerkat-e2e:{run_id}]"),
            dir,
            journal: Mutex::new(journal),
            journal_failed: AtomicBool::new(false),
            cleanup_failed: AtomicBool::new(false),
            cleanup_complete: AtomicBool::new(false),
        };
        artifact.write_result("running", "incomplete", None)?;
        Ok(artifact)
    }

    pub fn record(&self, operation: &str, id: &str, status: &str) {
        let chars: Vec<_> = id.trim().chars().collect();
        let id = if chars.len() > 10 {
            format!(
                "{}…{}",
                chars[..6].iter().collect::<String>(),
                chars[chars.len() - 4..].iter().collect::<String>()
            )
        } else {
            id.trim().to_owned()
        };
        self.append(operation, &id, status);
    }

    /// The caller must supply only secret-free diagnostics.
    pub fn record_diagnostic(&self, operation: &str, message: &str) {
        self.append(operation, message, "diagnostic");
    }

    pub fn record_cleanup(&self, operation: &str, id: &str, succeeded: bool) {
        if !succeeded {
            self.cleanup_failed.store(true, Ordering::SeqCst);
        }
        self.record(operation, id, if succeeded { "ok" } else { "failed" });
    }

    /// Call only after every owned cleanup operation has been attempted.
    pub fn complete_cleanup(&self) {
        self.cleanup_complete.store(true, Ordering::SeqCst);
    }

    fn append(&self, operation: &str, id: &str, status: &str) {
        let result = (|| -> Result<(), ()> {
            let mut bytes = serde_json::to_vec(
                &json!({"marker":self.marker,"operation":operation,"id":id,"status":status}),
            )
            .map_err(|_| ())?;
            bytes.push(b'\n');
            let mut journal = self.journal.lock().map_err(|_| ())?;
            journal.write_all(&bytes).map_err(|_| ())?;
            journal.sync_all().map_err(|_| ())
        })();
        if result.is_err() {
            self.journal_failed.store(true, Ordering::SeqCst);
        }
    }

    pub fn finish(&self, scenario: Result<(), String>) -> Result<(), String> {
        let cleanup = if !self.cleanup_complete.load(Ordering::SeqCst) {
            "incomplete"
        } else if self.cleanup_failed.load(Ordering::SeqCst) {
            "failed"
        } else {
            "passed"
        };
        let mut errors = Vec::new();
        if let Err(error) = scenario {
            errors.push(error);
        }
        if cleanup != "passed" {
            errors.push(format!("scenario cleanup {cleanup}"));
        }
        if self.journal_failed.load(Ordering::SeqCst) {
            errors.push("scenario journal write failed".into());
        }
        let error = (!errors.is_empty()).then(|| errors.join("; "));
        self.write_result(
            if error.is_none() { "passed" } else { "failed" },
            cleanup,
            error.as_deref(),
        )?;
        error.map_or(Ok(()), Err)
    }

    fn write_result(&self, status: &str, cleanup: &str, error: Option<&str>) -> Result<(), String> {
        let temporary = self.dir.join("result.json.next");
        let mut file = private_file(&temporary)?;
        let bytes = serde_json::to_vec_pretty(
            &json!({"marker":self.marker,"status":status,"cleanup":cleanup,"error":error}),
        )
        .map_err(|_| "cannot encode scenario result")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "cannot persist scenario result")?;
        fs::rename(&temporary, self.dir.join("result.json"))
            .map_err(|_| "cannot publish scenario result")?;
        Ok(())
    }
}

fn private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| "cannot create private scenario evidence".into())
}
