//! Accounts for Harnesses that sign in themselves. The Runtime runs any
//! browser step; the Machine only stores the result and reports the state.

use super::connection::{publish_inventory, send_frame};
use super::*;
use meerkat_harness::HarnessCredential;
use meerkat_protocol::link::{SetHarnessCredential, SetHarnessCredentialResult};
use meerkat_protocol::machine_frame;

pub(super) async fn run(inner: Arc<Inner>, request: SetHarnessCredential) {
    let error = match inner.factories.get(&request.harness_id) {
        Some(factory) if factory.has_account() => {
            let credential = request.credential.map(|credential| match credential {
                meerkat_protocol::HarnessCredential::Chatgpt { auth_json } => {
                    HarnessCredential::Chatgpt { auth_json }
                }
                meerkat_protocol::HarnessCredential::ApiKey {
                    base_url,
                    api_key,
                    model,
                } => HarnessCredential::ApiKey {
                    base_url,
                    api_key,
                    model,
                },
            });
            match factory.set_credential(credential).await {
                Ok(()) => String::new(),
                Err(error) => {
                    log::warn!(
                        "machine={} harness={} account change failed: {}",
                        inner.config.machine_id,
                        request.harness_id,
                        error.failure.code
                    );
                    match error.failure.code.as_str() {
                        "invalid_key" | "unreachable" => error.failure.code,
                        _ => "account_not_saved".into(),
                    }
                }
            }
        }
        _ => "harness_unavailable".into(),
    };
    // Inventory first, so the Runtime already shows the new state when it
    // receives the acknowledgement.
    if error.is_empty() {
        publish_inventory(&inner).await;
    }
    send_frame(
        &inner,
        MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::SetHarnessCredentialResult(
                SetHarnessCredentialResult {
                    request_id: request.request_id,
                    error,
                },
            )),
        },
    )
    .await;
}
