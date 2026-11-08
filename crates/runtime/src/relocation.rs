//! Moves an Agent's home between Machines; see `docs/agent-relocation.md`.
//!
//! One driver runs per relocating Agent. It stops whenever a Machine it needs
//! is offline and starts again when that Machine reconnects.
use super::*;
use crate::application::agent_runtime_config;
use meerkat_protocol::{AgentFilesResult, AgentRef, ManagementEventKind, runtime_frame};
use meerkat_storage::{RelocationRecord, RelocationState};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::Duration;

const CHUNK: u64 = 1024 * 1024;
const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
/// Packing or unpacking a large home can take a while; there is no size cap.
const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DRAIN_POLL: Duration = Duration::from_secs(2);

enum Step {
    /// Look at the relocation row again.
    Again,
    /// A Machine is offline or did not answer; wait for it to reconnect.
    Wait,
    Fail(String),
    Done,
}

impl RuntimeApplication {
    /// Starts a driver for every relocation in progress.
    pub(crate) fn resume_relocations(&self) {
        let application = self.clone();
        let _ = self.spawn_background(async move {
            let Ok(runtime) = application.runtime() else {
                return;
            };
            match runtime.database().agent_relocations().await {
                Ok(relocations) => {
                    for relocation in relocations {
                        if relocation.state != RelocationState::Failed {
                            application
                                .drive_relocation(relocation.workspace_id, relocation.agent_id);
                        }
                    }
                }
                Err(error) => log::error!("[runtime] relocations could not be listed: {error}"),
            }
        });
    }

    pub(crate) fn drive_relocation(&self, workspace_id: WorkspaceId, agent_id: AgentId) {
        let key = (workspace_id.clone(), agent_id.clone());
        let Ok(mut running) = self.inner.relocations.lock() else {
            return;
        };
        if !running.insert(key.clone()) {
            return;
        }
        drop(running);
        let application = self.clone();
        let task_key = key.clone();
        let spawned = self.spawn_background(async move {
            let key = task_key;
            if let Err(error) = application.run_relocation(&workspace_id, &agent_id).await {
                log::error!(
                    "[runtime] workspace={workspace_id} agent={agent_id} relocation stopped: {error}"
                );
            }
            if let Ok(mut running) = application.inner.relocations.lock() {
                running.remove(&key);
            }
        });
        if spawned.is_err()
            && let Ok(mut running) = self.inner.relocations.lock()
        {
            running.remove(&key);
        }
    }

    pub(crate) fn relocation_archive(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> PathBuf {
        self.inner
            .relocation_root
            .join(workspace_id.as_str())
            .join(format!("{}.zip", agent_id.as_str()))
    }

    async fn run_relocation(
        &self,
        workspace_id: &WorkspaceId,
        agent_id: &AgentId,
    ) -> Result<(), RuntimeError> {
        let database = self.runtime()?.database();
        loop {
            let Some(relocation) = database.agent_relocation(workspace_id, agent_id).await? else {
                // Cancelled.
                self.drop_relocation_archive(workspace_id, agent_id);
                return Ok(());
            };
            let step = match relocation.state {
                RelocationState::Failed => return Ok(()),
                RelocationState::Waiting => self.export_step(&database, &relocation).await?,
                RelocationState::Staged => self.import_step(&database, &relocation).await?,
            };
            match step {
                Step::Again => {}
                Step::Wait => return Ok(()),
                Step::Fail(code) => {
                    log::error!(
                        "[runtime] workspace={workspace_id} agent={agent_id} relocation failed: {code}"
                    );
                    self.drop_relocation_archive(workspace_id, agent_id);
                    database
                        .fail_agent_relocation(workspace_id, agent_id, &code, now_millis())
                        .await?;
                    database.wake();
                    self.emit_management(
                        ManagementEventKind::AgentUpdated,
                        workspace_id,
                        Some(agent_id),
                        now_millis(),
                    );
                    return Ok(());
                }
                Step::Done => {
                    return self.finish_relocation(&database, &relocation).await;
                }
            }
        }
    }

    /// Waits for the running execution to end, then pulls the home from the source.
    async fn export_step(
        &self,
        database: &Database,
        relocation: &RelocationRecord,
    ) -> Result<Step, RuntimeError> {
        let (w, a) = (&relocation.workspace_id, &relocation.agent_id);
        // Leaving the files behind skips the wait too: the old placement's run
        // is stopped by the placement change.
        if relocation.without_files {
            return Ok(Step::Done);
        }
        if !database
            .active_execution_ids_for_agent(w, a)
            .await?
            .is_empty()
        {
            tokio::time::sleep(DRAIN_POLL).await;
            return Ok(Step::Again);
        }
        let broker = self.machine_broker();
        let source = relocation.source_machine_id.as_str();
        if !broker.is_connected(source) {
            return Ok(Step::Wait);
        }
        let agent = agent_ref(w, a);
        let exported = match broker
            .agent_files_request(source, ARCHIVE_TIMEOUT, |request_id| {
                runtime_frame::Frame::ExportAgentFiles(meerkat_protocol::ExportAgentFiles {
                    request_id,
                    agent: Some(agent.clone()),
                })
            })
            .await
        {
            Ok(result) if result.success => result,
            Ok(result) => return Ok(Step::Fail(refusal(result))),
            Err(_) => return Ok(Step::Wait),
        };
        let archive = self.relocation_archive(w, a);
        let partial = archive.with_extension("part");
        if let Some(parent) = archive.parent() {
            std::fs::create_dir_all(parent).map_err(staging)?;
        }
        let mut file = std::fs::File::create(&partial).map_err(staging)?;
        let mut digest = Sha256::new();
        let mut offset = 0;
        while offset < exported.size {
            let chunk = match broker
                .agent_files_request(source, CHUNK_TIMEOUT, |request_id| {
                    runtime_frame::Frame::ReadAgentFiles(meerkat_protocol::ReadAgentFiles {
                        request_id,
                        agent: Some(agent.clone()),
                        offset,
                        length: CHUNK,
                    })
                })
                .await
            {
                Ok(result) if result.success && !result.bytes.is_empty() => result.bytes,
                Ok(result) if result.success => return Ok(Step::Fail("archive_truncated".into())),
                Ok(result) => return Ok(Step::Fail(refusal(result))),
                Err(_) => {
                    let _ = std::fs::remove_file(&partial);
                    return Ok(Step::Wait);
                }
            };
            file.write_all(&chunk).map_err(staging)?;
            digest.update(&chunk);
            offset += chunk.len() as u64;
        }
        file.sync_all().map_err(staging)?;
        drop(file);
        if hex(digest) != exported.sha256 {
            return Ok(Step::Fail("archive_integrity_mismatch".into()));
        }
        std::fs::rename(&partial, &archive).map_err(staging)?;
        self.release_archive(source, &agent).await;
        if !database
            .stage_agent_relocation(w, a, exported.size, &exported.sha256, now_millis())
            .await?
        {
            // Cancelled or switched to moving without files meanwhile.
            self.drop_relocation_archive(w, a);
        }
        Ok(Step::Again)
    }

    /// Pushes the staged home to the target, which swaps it in on the last chunk.
    async fn import_step(
        &self,
        database: &Database,
        relocation: &RelocationRecord,
    ) -> Result<Step, RuntimeError> {
        let (w, a) = (&relocation.workspace_id, &relocation.agent_id);
        let (Some(size), Some(sha256)) =
            (relocation.archive_size, relocation.archive_sha256.clone())
        else {
            return Ok(Step::Fail("archive_missing".into()));
        };
        let archive = self.relocation_archive(w, a);
        let Ok(mut file) = std::fs::File::open(&archive) else {
            return Ok(Step::Fail("archive_missing".into()));
        };
        let broker = self.machine_broker();
        let target = relocation.target_machine_id.as_str();
        if !broker.is_connected(target) {
            return Ok(Step::Wait);
        }
        let agent = agent_ref(w, a);
        let mut offset = 0;
        loop {
            let mut bytes = Vec::with_capacity(CHUNK as usize);
            file.seek(SeekFrom::Start(offset)).map_err(staging)?;
            (&mut file)
                .take(CHUNK)
                .read_to_end(&mut bytes)
                .map_err(staging)?;
            let eof = offset + bytes.len() as u64 >= size;
            let length = bytes.len() as u64;
            let sha256 = sha256.clone();
            let answer = broker
                .agent_files_request(
                    target,
                    if eof { ARCHIVE_TIMEOUT } else { CHUNK_TIMEOUT },
                    |request_id| {
                        runtime_frame::Frame::WriteAgentFiles(meerkat_protocol::WriteAgentFiles {
                            request_id,
                            agent: Some(agent.clone()),
                            offset,
                            bytes,
                            eof,
                            size,
                            sha256,
                        })
                    },
                )
                .await;
            match answer {
                Ok(result) if result.success => {}
                // The target is still running something of this Agent; try again shortly.
                Ok(result) if result.error_code == "agent_busy" => {
                    tokio::time::sleep(DRAIN_POLL).await;
                    return Ok(Step::Again);
                }
                Ok(result) => return Ok(Step::Fail(refusal(result))),
                Err(_) => return Ok(Step::Wait),
            }
            if eof {
                break;
            }
            if length == 0 {
                return Ok(Step::Fail("archive_truncated".into()));
            }
            offset += length;
        }
        // The move may have been pointed elsewhere while the files were on their way.
        let current = database.agent_relocation(w, a).await?;
        if current.is_some_and(|current| current.target_machine_id == relocation.target_machine_id)
        {
            Ok(Step::Done)
        } else {
            Ok(Step::Again)
        }
    }

    pub(crate) async fn finish_relocation(
        &self,
        database: &Database,
        relocation: &RelocationRecord,
    ) -> Result<(), RuntimeError> {
        let (w, a) = (&relocation.workspace_id, &relocation.agent_id);
        let Some(record) = database.commit_agent_relocation(w, a, now_millis()).await? else {
            return Ok(());
        };
        self.drop_relocation_archive(w, a);
        let runtime = self.runtime()?;
        let workspace = database.workspace_record(w).await?;
        runtime
            .configure_agent(w, agent_runtime_config(&workspace, record)?)
            .await?;
        self.emit_management(ManagementEventKind::AgentUpdated, w, Some(a), now_millis());
        database.wake();
        let broker = self.machine_broker();
        let source = relocation.source_machine_id.clone();
        let (cleanup_w, cleanup_a) = (w.clone(), a.clone());
        self.spawn_background(async move {
            // Queued in storage; retried when the source reconnects.
            if let Err(error) = broker
                .delete_agent_files(&source, &cleanup_w, &cleanup_a)
                .await
            {
                log::warn!(
                    "[runtime] workspace={cleanup_w} agent={cleanup_a} old home cleanup deferred: {error}"
                );
            }
        })?;
        Ok(())
    }

    async fn release_archive(&self, machine_id: &str, agent: &AgentRef) {
        let _ = self
            .machine_broker()
            .agent_files_request(machine_id, CHUNK_TIMEOUT, |request_id| {
                runtime_frame::Frame::ReleaseAgentFiles(meerkat_protocol::ReleaseAgentFiles {
                    request_id,
                    agent: Some(agent.clone()),
                })
            })
            .await;
    }

    pub(crate) fn drop_relocation_archive(&self, workspace_id: &WorkspaceId, agent_id: &AgentId) {
        let archive = self.relocation_archive(workspace_id, agent_id);
        let _ = std::fs::remove_file(archive.with_extension("part"));
        let _ = std::fs::remove_file(archive);
    }
}

fn agent_ref(workspace_id: &WorkspaceId, agent_id: &AgentId) -> AgentRef {
    AgentRef {
        workspace_id: workspace_id.to_string(),
        agent_id: agent_id.to_string(),
    }
}

fn refusal(result: AgentFilesResult) -> String {
    if result.error_code.is_empty() {
        "relocation_failed".into()
    } else {
        result.error_code
    }
}

fn hex(digest: Sha256) -> String {
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn staging(error: std::io::Error) -> RuntimeError {
    RuntimeError::Application(format!("relocation staging failed: {error}"))
}
