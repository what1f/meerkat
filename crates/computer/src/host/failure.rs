use super::Inner;
use lankide_harness::HarnessFailure;
use lankide_protocol::{AttemptFailure, ExecutionRef};
use sha2::{Digest, Sha256};
use std::io::Write;

pub(super) fn attempt_failure(failure: &HarnessFailure) -> AttemptFailure {
    let normalized = failure
        .detail
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut hash = Sha256::new();
    hash.update(failure.code.as_bytes());
    hash.update(b"\0");
    hash.update(normalized.as_bytes());
    AttemptFailure {
        code: failure.code.clone(),
        category: serde_json::to_value(failure.category)
            .expect("failure category")
            .as_str()
            .expect("category string")
            .to_owned(),
        retryable: failure.retryable,
        fingerprint: hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        detail: failure.detail.clone(),
    }
}

pub(super) fn log_failure(
    inner: &Inner,
    execution: &ExecutionRef,
    agent_id: &str,
    failure: &AttemptFailure,
) {
    let record = serde_json::json!({
        "event": "attempt_failure", "execution_id": execution.execution_id,
        "attempt": execution.attempt, "agent_id": agent_id,
        "computer_id": inner.config.computer_id, "failure": failure,
    });
    eprintln!("{record}");
    let result = (|| -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(inner.root.join("attempt-failures.jsonl"))?
            .write_all(&bytes)
    })();
    if let Err(error) = result {
        eprintln!("Computer failure log could not be saved: {error}");
    }
}
