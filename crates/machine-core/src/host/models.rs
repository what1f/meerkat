//! Models and the CLI version are probed only when the Runtime asks, for
//! example when a person picks an AI for a member.

use super::connection::{make_hello, publish_inventory, send_frame};
use super::*;
use meerkat_protocol::{ModelCatalog, ModelCatalogItem, machine_frame};

const LIST_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) async fn run(inner: Arc<Inner>, request: meerkat_protocol::link::ListModels) {
    let before = make_hello(&inner).await.harnesses;
    let (catalog, error) = list(&inner, &request.harness_id).await;
    send_frame(
        &inner,
        MachineFrame {
            protocol_version: meerkat_protocol::PROTOCOL_VERSION,
            frame: Some(machine_frame::Frame::ListModelsResult(
                meerkat_protocol::link::ListModelsResult {
                    request_id: request.request_id,
                    catalog,
                    error,
                },
            )),
        },
    )
    .await;
    // The Runtime checks selections against the inventory, so it learns of a
    // new version or model list too. An unchanged answer sends nothing.
    if make_hello(&inner).await.harnesses != before {
        publish_inventory(&inner).await;
    }
}

async fn list(inner: &Inner, harness_id: &str) -> (Option<ModelCatalog>, String) {
    let (Some(factory), Some(config)) = (
        inner.factories.get(harness_id),
        inner.defaults.get(harness_id),
    ) else {
        return (None, "harness_unavailable".into());
    };
    let (probe, models) = tokio::join!(
        inner.cli_versions.probe(factory.as_ref(), config),
        tokio::time::timeout(LIST_TIMEOUT, factory.models(config)),
    );
    let probe = match probe {
        Ok(mut probe) => {
            probe.capabilities = factory.capabilities(config);
            probe.reason = probe
                .reason
                .map(|reason| redact_diagnostic(&reason, config));
            probe
        }
        Err(error) => ProbeResult {
            reason: Some(redact_diagnostic(&error.to_string(), config)),
            ..inner.cli_versions.snapshot(factory.as_ref(), config)
        },
    };
    inner
        .probes
        .lock()
        .unwrap()
        .insert(harness_id.to_owned(), probe);
    match models {
        Ok(Ok(models)) => {
            let catalog = ModelCatalog {
                models: models
                    .into_iter()
                    .map(|model| ModelCatalogItem {
                        available: true,
                        model_id: model.id,
                        display_name: model.name,
                        description: model.description,
                        supported_thinking_levels: model.thinking_levels,
                    })
                    .collect(),
            };
            inner.cli_versions.set_models(config, catalog.clone());
            (Some(catalog), String::new())
        }
        Ok(Err(error)) => (None, redact_diagnostic(&error.to_string(), config)),
        Err(_) => (None, "Model discovery timed out".into()),
    }
}
