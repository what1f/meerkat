use super::*;
use meerkat_provider::ResourceUpload;
use meerkat_storage::ResourceMetadata;
use std::{future::Future, path::PathBuf, pin::Pin};

pub type ResourceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

/// Runtime-owned byte storage. Persistence owns only resource identity and
/// metadata; Provider and Machine boundaries never receive its storage path.
pub trait ResourceStore: Send + Sync {
    fn save<'a>(
        &'a self,
        workspace_id: &'a WorkspaceId,
        upload: ResourceUpload,
    ) -> ResourceFuture<'a, ResourceRef>;

    fn remove<'a>(
        &'a self,
        workspace_id: &'a WorkspaceId,
        resource_id: ResourceId,
    ) -> ResourceFuture<'a, ()>;

    fn read<'a>(
        &'a self,
        workspace_id: &'a WorkspaceId,
        resource_id: ResourceId,
    ) -> ResourceFuture<'a, ResourceBytes>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceBytes {
    pub resource: ResourceRef,
    pub bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct LocalResourceStore {
    storage: Database,
    root: PathBuf,
}

impl LocalResourceStore {
    pub fn new(storage: Arc<Storage>, root: impl Into<PathBuf>) -> Self {
        Self::with_database(Database::new(storage), root)
    }

    pub(crate) fn with_database(storage: Database, root: impl Into<PathBuf>) -> Self {
        Self {
            storage,
            root: root.into(),
        }
    }

    fn path(&self, workspace_id: &WorkspaceId, resource_id: ResourceId) -> PathBuf {
        self.root
            .join(workspace_id.as_str())
            .join(resource_id.get().to_string())
    }
}

impl ResourceStore for LocalResourceStore {
    fn save<'a>(
        &'a self,
        workspace_id: &'a WorkspaceId,
        upload: ResourceUpload,
    ) -> ResourceFuture<'a, ResourceRef> {
        Box::pin(async move {
            let resource_id = self
                .storage
                .allocate_resource_id(workspace_id, now_millis())
                .await?;
            let path = self.path(workspace_id, resource_id);
            let parent = path
                .parent()
                .ok_or_else(|| RuntimeError::Application("invalid resource path".into()))?;
            std::fs::create_dir_all(parent).map_err(resource_io_error)?;
            if let Err(error) = std::fs::write(&path, &upload.bytes) {
                let _ = std::fs::remove_file(&path);
                return Err(resource_io_error(error));
            }
            let sha256 = Sha256::digest(&upload.bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if let Err(error) = self
                .storage
                .upsert_resource_metadata(ResourceMetadata {
                    workspace_id: workspace_id.clone(),
                    resource_id,
                    file_name: upload.file_name.clone(),
                    media_type: upload.media_type.clone(),
                    size_bytes: upload.bytes.len() as u64,
                    sha256: sha256.clone(),
                    storage_ref: path.to_string_lossy().into_owned(),
                    created_at: now_millis(),
                })
                .await
            {
                let _ = std::fs::remove_file(&path);
                return Err(error.into());
            }
            Ok(ResourceRef {
                resource_id: resource_id.get(),
                file_name: upload.file_name,
                media_type: upload.media_type,
                size_bytes: upload.bytes.len() as u64,
                sha256,
            })
        })
    }

    fn remove<'a>(
        &'a self,
        workspace_id: &'a WorkspaceId,
        resource_id: ResourceId,
    ) -> ResourceFuture<'a, ()> {
        Box::pin(async move {
            let file = std::fs::remove_file(self.path(workspace_id, resource_id));
            let metadata = self
                .storage
                .remove_resource_metadata(workspace_id, resource_id)
                .await;
            file.map_err(resource_io_error)?;
            metadata?;
            Ok(())
        })
    }

    fn read<'a>(
        &'a self,
        workspace_id: &'a WorkspaceId,
        resource_id: ResourceId,
    ) -> ResourceFuture<'a, ResourceBytes> {
        Box::pin(async move {
            let metadata = self
                .storage
                .resource_metadata(workspace_id, resource_id)
                .await?
                .ok_or_else(|| RuntimeError::Application("resource not found".into()))?;
            let expected = self.path(workspace_id, resource_id);
            if PathBuf::from(&metadata.storage_ref) != expected {
                return Err(RuntimeError::Application(
                    "resource storage reference mismatch".into(),
                ));
            }
            let bytes = std::fs::read(&expected).map_err(resource_io_error)?;
            let sha256 = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if bytes.len() as u64 != metadata.size_bytes || sha256 != metadata.sha256 {
                return Err(RuntimeError::Application(
                    "resource integrity check failed".into(),
                ));
            }
            Ok(ResourceBytes {
                resource: ResourceRef {
                    resource_id: resource_id.get(),
                    file_name: metadata.file_name,
                    media_type: metadata.media_type,
                    size_bytes: metadata.size_bytes,
                    sha256: metadata.sha256,
                },
                bytes,
            })
        })
    }
}

fn resource_io_error(error: std::io::Error) -> RuntimeError {
    RuntimeError::Application(format!("resource io: {error}"))
}
