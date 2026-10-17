use lankide_client::ManagementClient;
use lankide_system_e2e::{Result, RigConfig, SystemRig};
use serde_json::{Value, json};
use std::{path::PathBuf, process::Child, time::Duration};

pub struct Machines {
    pub children: Vec<Child>,
    pub homes: Vec<PathBuf>,
}
impl Machines {
    pub fn new() -> Self {
        Self {
            children: vec![],
            homes: vec![],
        }
    }
    pub async fn pair(&mut self, rig: &SystemRig, command: &str) -> Result<Value> {
        // The product emits plain arguments, never a shell program. Do not execute it.
        let args: Vec<_> = command.split_whitespace().collect();
        let (url, code, pin) = match args.as_slice() {
            ["lankide-machine", "pair", url, code, "--cert-sha256", pin] => (*url, *code, *pin),
            _ => return Err("expected the displayed pinned Machine pairing command".into()),
        };
        // Only this rig's remote listener is allowed; never contact a real Runtime.
        let access = rig.management().get_remote_access().await?;
        let expected = format!("wss://{}", access.address.ok_or("remote access address")?);
        if url != expected || Some(pin) != access.cert_sha256.as_deref() {
            return Err("pairing command does not belong to this rig".into());
        }
        let home = rig
            .runtime_home()
            .join("web-machines")
            .join(self.homes.len().to_string());
        std::fs::create_dir_all(&home)?;
        self.homes.push(home.clone());
        let bin = std::env::current_exe()?
            .parent()
            .ok_or("binary directory")?
            .to_path_buf();
        let child = std::process::Command::new(bin.join("system-machine"))
            .env(
                "LANKIDE_SYSTEM_CONTROL_ROOT",
                rig.runtime_home().join("scripted-members"),
            )
            .arg("pair")
            .arg(&home)
            .arg(url)
            .arg(code)
            .arg(pin)
            .arg(bin.join("system-harness"))
            .arg(serde_json::to_string(
                &RigConfig::default().harness_capabilities,
            )?)
            .spawn()?;
        self.children.push(child);
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if self.children.last_mut().unwrap().try_wait()?.is_some() {
                    return Err("paired Machine exited before registration".into());
                }
                if let Some(credentials) = lankide_machine::read_credentials(&home)? {
                    if rig
                        .management()
                        .get_machine_catalog(None)
                        .await?
                        .iter()
                        .any(|m| m.id == credentials.machine_id && m.available)
                    {
                        return Ok(json!({"machine_id":credentials.machine_id}));
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?
    }
    pub fn stop(&mut self) -> Result<()> {
        for child in &mut self.children {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
        }
        Ok(())
    }
}
impl Drop for Machines {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
