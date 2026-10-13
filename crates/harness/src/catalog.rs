use crate::{HarnessConfig, HarnessError, HarnessErrorCode};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// A short-lived native CLI connection used only for capability discovery.
pub struct NativeCatalogProcess {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl NativeCatalogProcess {
    pub fn spawn(config: &HarnessConfig, arguments: &[&str]) -> Result<Self, HarnessError> {
        config.validate_executable()?;
        let mut child = Command::new(&config.cli_path)
            .args(arguments)
            .envs(&config.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| {
                error(
                    HarnessErrorCode::SpawnFailed,
                    "Could not start model discovery",
                )
            })?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| error(HarnessErrorCode::Io, "Model discovery input unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| error(HarnessErrorCode::Io, "Model discovery output unavailable"))?;
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
        })
    }

    pub async fn send(&mut self, value: Value) -> Result<(), HarnessError> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| {
            error(
                HarnessErrorCode::Io,
                "Could not encode model discovery request",
            )
        })?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| error(HarnessErrorCode::Io, "Model discovery input closed"))
    }

    pub async fn receive(
        &mut self,
        matches: impl Fn(&Value) -> bool,
    ) -> Result<Value, HarnessError> {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                // A bounded line also bounds allocation before parsing native output.
                let mut line = Vec::new();
                loop {
                    let bytes = self.output.fill_buf().await.map_err(|_| {
                        error(
                            HarnessErrorCode::Io,
                            "Could not read model discovery response",
                        )
                    })?;
                    if bytes.is_empty() {
                        return Err(error(
                            HarnessErrorCode::StreamInterrupted,
                            "Model discovery ended before responding",
                        ));
                    }
                    let count = bytes
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(bytes.len(), |index| index + 1);
                    if line.len() + count > 1024 * 1024 {
                        return Err(error(
                            HarnessErrorCode::InvalidEvent,
                            "Model discovery response is too large",
                        ));
                    }
                    let complete = bytes[count - 1] == b'\n';
                    line.extend_from_slice(&bytes[..count]);
                    self.output.consume(count);
                    if complete {
                        break;
                    }
                }
                if let Ok(value) = serde_json::from_slice::<Value>(&line)
                    && matches(&value)
                {
                    return Ok(value);
                }
            }
        })
        .await
        .map_err(|_| error(HarnessErrorCode::Timeout, "Model discovery timed out"))?
    }

    pub async fn close(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

fn error(code: HarnessErrorCode, message: &str) -> HarnessError {
    HarnessError::new(code, message)
}
