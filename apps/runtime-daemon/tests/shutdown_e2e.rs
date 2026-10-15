//! Public Runtime lifecycle acceptance, without the OS socket boundary.
use lankide_core::{ProviderId, WorkspaceId};
use lankide_protocol::{ComputerHello, ComputerPeer, PROTOCOL_VERSION, RuntimePeer};
use lankide_provider::{
    ChannelDefinition, InitContext, Provider, ProviderFuture, ProviderHost, ProviderInfo,
};
use lankide_runtime::{
    ProviderFactoryRegistry, RuntimeApplication, RuntimeConfig, RuntimePeerAdapter,
};
use lankide_storage::{DatabaseConfig, Storage, WorkspaceSpec};
use sha2::Digest;
use std::time::Duration;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

struct ReceiverProvider;
impl Provider for ReceiverProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: ProviderId::new("receiver").unwrap(),
            name: "Receiver".into(),
            description: String::new(),
            version: "1".into(),
            icon: String::new(),
        }
    }
    fn send_message<'a>(
        &'a self,
        _: lankide_provider::SendMessageContext,
    ) -> ProviderFuture<'a, lankide_provider::SendMessageResult> {
        Box::pin(async {
            Err(lankide_provider::ProviderError::External {
                operation: "unexpected_send",
                retryable: false,
            })
        })
    }
    fn channels<'a>(&'a self, _: InitContext) -> ProviderFuture<'a, Vec<ChannelDefinition>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn init<'a>(&'a self, context: InitContext) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            tokio::spawn(async move {
                context.shutdown_signal().cancelled().await;
                // The outer receiver needs time to finish after cancellation.
                tokio::time::sleep(Duration::from_millis(150)).await;
                drop(context);
            });
            Ok(())
        })
    }
}

#[tokio::test]
async fn shutdown_releases_connected_computer_and_provider_before_database_reopen() {
    let home = tempfile::tempdir().unwrap();
    let database = DatabaseConfig::new(home.path().join("runtime.db"));
    let storage = Storage::open(database.clone()).unwrap();
    storage
        .create_workspace(WorkspaceSpec {
            id: WorkspaceId::new("team").unwrap(),
            name: "Team".into(),
            provider_id: ReceiverProvider.info().id,
            provider_config: serde_json::json!({}),
            provider_secret: serde_json::json!({}),
            active: true,
            now: 1,
        })
        .unwrap();
    storage.close().unwrap();
    let factories = ProviderFactoryRegistry::default();
    factories
        .register_host_factory::<ReceiverProvider, _>(
            ReceiverProvider.info(),
            |workspace, services| {
                ProviderHost::new(
                    ReceiverProvider,
                    workspace.id.clone(),
                    workspace.provider_config.clone(),
                    services,
                )
            },
        )
        .unwrap();
    let application = RuntimeApplication::open_with_shutdown_timeout(
        RuntimeConfig {
            database: database.clone(),
            retry: Default::default(),
        },
        factories.clone(),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    application.start().await.unwrap();
    lankide_storage::Storage::open(database.clone())
        .unwrap()
        .pair_computer(
            &lankide_storage::ComputerRecord {
                is_local: false,
                computer_id: "computer".into(),
                display_name: "computer".into(),
                credential_hash: sha2::Sha256::digest(b"fixture-computer")
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
                paired_at: 1,
                revoked_at: None,
                last_seen_at: None,
                os: "test".into(),
                version: "1".into(),
                inventory_json: "[]".into(),
            },
            None,
            600_000,
        )
        .unwrap();
    let peer = RuntimePeerAdapter::new(application.clone());
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    let mut frames = peer
        .connect_computer(
            ComputerHello {
                device_credential: "fixture-computer".into(),
                protocol_version: PROTOCOL_VERSION,
                computer_id: "computer".into(),
                instance_id: "instance".into(),
                computer_version: "1".into(),
                os: "test".into(),
                arch: "test".into(),
                harnesses: Vec::new(),
            },
            Box::pin(ReceiverStream::new(receiver)),
        )
        .await
        .unwrap();
    assert!(frames.next().await.unwrap().is_ok());
    tokio::time::timeout(Duration::from_secs(3), application.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(application.readiness().state, "stopped");
    drop(sender);
    drop(frames);
    let reopened = RuntimeApplication::open(
        RuntimeConfig {
            database,
            retry: Default::default(),
        },
        factories,
    )
    .await
    .unwrap();
    let peer = RuntimePeerAdapter::new(reopened.clone());
    assert_eq!(
        peer.list_workspaces(lankide_protocol::ListWorkspacesRequest {
            request_id: "reopened".into(),
        })
        .await
        .unwrap()
        .workspaces
        .len(),
        1
    );
    reopened.shutdown().await.unwrap();
}
