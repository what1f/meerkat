use super::*;
use lankide_protocol::link::{CloseReason, PairComputerRequest, PairComputerResponse};
use lankide_storage::ComputerRecord;
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

    pub async fn pair_computer(
        &self,
        request: PairComputerRequest,
        local_secret: &str,
    ) -> Result<PairComputerResponse, RuntimeError> {
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
        let computer_id = format!("computer-{}", random_secret()?);
        let code_hash = credential_hash(&request.code);
        self.runtime()?
            .database()
            .pair_computer(
                &ComputerRecord {
                    computer_id: computer_id.clone(),
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
        self.emit_computer_catalog_changed();
        Ok(PairComputerResponse {
            computer_id,
            device_credential,
        })
    }

    pub(crate) async fn authenticate_computer(
        &self,
        computer_id: Option<&str>,
        credential: &str,
    ) -> Result<(), CloseReason> {
        let hash = credential_hash(credential);
        let runtime = self.runtime().map_err(|_| CloseReason::Unauthorized)?;
        let computers = runtime
            .database()
            .registered_computers()
            .await
            .map_err(|_| CloseReason::Unauthorized)?;
        for computer in computers {
            let matches = bool::from(computer.credential_hash.as_bytes().ct_eq(hash.as_bytes()));
            if matches && computer_id.is_none_or(|id| id == computer.computer_id) {
                return if computer.revoked_at.is_some() {
                    Err(CloseReason::ComputerRevoked)
                } else {
                    Ok(())
                };
            }
        }
        Err(CloseReason::ComputerUnknown)
    }

    pub async fn revoke_computer(&self, computer_id: &str) -> Result<(), RuntimeError> {
        // Serialize revocation against authenticated hello registration.
        let _guard = self.inner.computer_registration.lock().await;
        self.runtime()?
            .database()
            .revoke_computer(computer_id, application::now_millis())
            .await?;
        self.computer_broker().revoke(computer_id)?;
        self.emit_computer_catalog_changed();
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
