#[path = "../../../../crates/harness/tests/support/process.rs"]
mod process_support;
use process_support::{ProcessHarnessFactory, ProcessHarnessSpec};

use lankide_harness::{HarnessConfig, HarnessInfo, ModelRef};
use lankide_machine::{AppError, DaemonConfig, run_with_harnesses};
use lankide_machine_core::RegisteredHarness;
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

pub async fn run_scripted(config: DaemonConfig, pid_journal: PathBuf) -> Result<(), AppError> {
    let executable = std::env::current_exe()?;
    let mut registrations = Vec::new();
    for harness_id in ["pi", "codex", "claude_code"] {
        let factory = ProcessHarnessFactory::new(
            HarnessInfo::new(harness_id, harness_id).map_err(AppError::MachineHarness)?,
            ProcessHarnessSpec {
                arguments: vec!["--scripted-harness".into()],
                bind_timeout: Duration::from_secs(4),
                cancel_grace: Duration::from_millis(100),
                close_grace: Duration::from_millis(100),
                pid_journal: Some(pid_journal.clone()),
            },
        );
        registrations.push(RegisteredHarness::new(
            factory,
            harness_config(harness_id, "fixture", "low", executable.clone()),
        ));
    }
    run_with_harnesses(config, registrations).await
}

fn harness_config(
    harness_id: &str,
    model: &str,
    thinking_level: &str,
    cli_path: PathBuf,
) -> HarnessConfig {
    HarnessConfig {
        launch_arguments: Default::default(),
        harness_id: harness_id.into(),
        model: ModelRef::new(model, model),
        thinking_level: thinking_level.into(),
        cli_path,
        environment: BTreeMap::new(),
        config_fingerprint: format!("local-{harness_id}"),
    }
}
