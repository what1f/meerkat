use super::*;
use lankide_protocol::link::{CloseReason, PairMachineRequest, PairMachineResponse};
use lankide_storage::MachineRecord;
use std::time::Duration;
use subtle::ConstantTimeEq;

impl RuntimeApplication {
    pub fn set_pairing_validity(&self, duration: Duration) -> Result<(), RuntimeError> {
        let millis = i64::try_from(duration.as_millis())
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| RuntimeError::Application("invalid_pairing_validity".into()))?;
        self.inner
            .pairing_validity_ms
            .store(millis, Ordering::Relaxed);
        Ok(())
    }

    pub async fn create_pairing_code(&self) -> Result<lankide_protocol::PairingCode, RuntimeError> {
        use base64::Engine as _;
        let mut bytes = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|error| RuntimeError::Application(error.to_string()))?;
        let code = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let issued_at = application::now_millis();
        self.runtime()?
            .database()
            .issue_pairing_code(&credential_hash(&code), issued_at)
            .await?;
        Ok(lankide_protocol::PairingCode {
            code,
            issued_at,
            expires_at: issued_at
                .saturating_add(self.inner.pairing_validity_ms.load(Ordering::Relaxed)),
        })
    }

    pub async fn pair_machine(
        &self,
        request: PairMachineRequest,
        local_secret: &str,
    ) -> Result<PairMachineResponse, RuntimeError> {
        if request.code.is_empty()
            || request.display_name.trim().is_empty()
            || request.display_name.len() > 256
            || request.os.len() > 128
            || request.version.len() > 128
        {
            return Err(RuntimeError::Application("pairing_request_invalid".into()));
        }
        let local = !local_secret.is_empty()
            && bool::from(
                credential_hash(&request.code)
                    .as_bytes()
                    .ct_eq(credential_hash(local_secret).as_bytes()),
            );
        let device_credential = random_secret()?;
        let machine_id = format!("machine-{}", random_secret()?);
        let code_hash = credential_hash(&request.code);
        let machine_id = self
            .runtime()?
            .database()
            .pair_machine(
                &MachineRecord {
                    machine_id: machine_id.clone(),
                    is_local: local,
                    display_name: request.display_name,
                    credential_hash: credential_hash(&device_credential),
                    paired_at: application::now_millis(),
                    revoked_at: None,
                    last_seen_at: None,
                    os: request.os,
                    version: request.version,
                    inventory_json: "[]".into(),
                },
                if local { None } else { Some(&code_hash) },
                self.inner.pairing_validity_ms.load(Ordering::Relaxed),
            )
            .await?;
        self.emit_machine_catalog_changed();
        Ok(PairMachineResponse {
            machine_id,
            device_credential,
        })
    }

    pub(crate) async fn authenticate_machine(
        &self,
        machine_id: Option<&str>,
        credential: &str,
    ) -> Result<(), CloseReason> {
        let hash = credential_hash(credential);
        let runtime = self.runtime().map_err(|_| CloseReason::Unauthorized)?;
        let machines = runtime
            .database()
            .registered_machines()
            .await
            .map_err(|_| CloseReason::Unauthorized)?;
        for machine in machines {
            let matches = bool::from(machine.credential_hash.as_bytes().ct_eq(hash.as_bytes()));
            if matches && machine_id.is_none_or(|id| id == machine.machine_id) {
                return if machine.revoked_at.is_some() {
                    Err(CloseReason::MachineRevoked)
                } else {
                    Ok(())
                };
            }
        }
        Err(CloseReason::MachineUnknown)
    }

    pub async fn rename_machine(&self, machine_id: &str, name: &str) -> Result<(), RuntimeError> {
        self.runtime()?
            .database()
            .rename_machine(machine_id, name)
            .await
            .map_err(machine_management_error)?;
        self.emit_machine_catalog_changed();
        Ok(())
    }

    pub async fn revoke_machine(&self, machine_id: &str) -> Result<(), RuntimeError> {
        // Serialize revocation against authenticated hello registration.
        let _guard = self.inner.machine_registration.lock().await;
        self.runtime()?
            .database()
            .revoke_machine(machine_id, application::now_millis())
            .await
            .map_err(machine_management_error)?;
        self.machine_broker().revoke(machine_id)?;
        self.emit_machine_catalog_changed();
        Ok(())
    }
}

fn credential_hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn random_secret() -> Result<String, RuntimeError> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| RuntimeError::Application(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn machine_management_error(error: lankide_storage::StorageError) -> RuntimeError {
    match error {
        lankide_storage::StorageError::Invalid(code) if code == "machine_has_assigned_agents" => {
            RuntimeError::MachineHasAssignedAgents
        }
        lankide_storage::StorageError::Invalid(code)
            if code == "local_machine_cannot_be_removed" =>
        {
            RuntimeError::LocalMachineCannotBeRemoved
        }
        lankide_storage::StorageError::Invalid(code) if code == "machine_name_invalid" => {
            RuntimeError::MachineNameInvalid
        }
        error => error.into(),
    }
}
