use super::*;
use meerkat_protocol::{FigureFrames, FigurePack, ImportFigurePackRequest, ListFigurePacksRequest};
use meerkat_provider::ResourceUpload;
use std::io::{Cursor, Read};

// Bound both the uploaded archive and all expanded entries, including unused files.
const MAX_PACK_BYTES: usize = meerkat_protocol::DEFAULT_MAX_MESSAGE_BYTES;
const MAX_EXPANDED_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Deserialize)]
struct Manifest {
    name: String,
    author: String,
    scale: f64,
}

fn invalid_pack() -> RuntimeError {
    RuntimeError::InvalidCore("figure_pack_invalid".into())
}

fn unpack(bytes: Vec<u8>) -> Result<(Manifest, Vec<Vec<u8>>), RuntimeError> {
    if bytes.len() > MAX_PACK_BYTES {
        return Err(invalid_pack());
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid_pack())?;
    let mut files = BTreeMap::new();
    let mut total = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| invalid_pack())?;
        let name = entry.name().replace('\\', "/");
        if name.starts_with('/')
            || name.split('/').any(|part| part == "..")
            || name.as_bytes().get(1) == Some(&b':')
        {
            return Err(invalid_pack());
        }
        let mut contents = Vec::new();
        entry
            .take(MAX_EXPANDED_BYTES.saturating_sub(total) + 1)
            .read_to_end(&mut contents)
            .map_err(|_| invalid_pack())?;
        total += contents.len() as u64;
        if total > MAX_EXPANDED_BYTES {
            return Err(invalid_pack());
        }
        files.insert(name, contents);
    }
    let manifest = serde_json::from_slice(files.get("manifest.json").ok_or_else(invalid_pack)?)
        .map_err(|_| invalid_pack())?;
    let mut frames = Vec::new();
    for name in ["work.png", "sleep.png", "carry.png", "stand.png"] {
        let bytes = files.remove(name).ok_or_else(invalid_pack)?;
        if bytes.len() > meerkat_protocol::MAX_WORKSPACE_IMAGE_BYTES {
            return Err(invalid_pack());
        }
        let reader = image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png);
        reader.decode().map_err(|_| invalid_pack())?;
        frames.push(bytes);
    }
    Ok((manifest, frames))
}

impl RuntimeApplication {
    pub(crate) async fn validate_figure(
        &self,
        workspace: &WorkspaceId,
        id: &str,
    ) -> Result<(), RuntimeError> {
        if id == "default" || id.starts_with("builtin:") {
            return Ok(());
        }
        if self
            .runtime()?
            .database()
            .list_figure_packs(workspace)
            .await?
            .iter()
            .any(|pack| pack["id"] == id)
        {
            return Ok(());
        }
        Err(RuntimeError::InvalidCore("figure_not_found".into()))
    }

    pub async fn list_figure_packs(
        &self,
        request: ListFigurePacksRequest,
    ) -> Result<Vec<FigurePack>, RuntimeError> {
        let workspace = management::workspace_id(request.workspace)?;
        self.runtime()?
            .database()
            .workspace_record(&workspace)
            .await?;
        self.runtime()?
            .database()
            .list_figure_packs(&workspace)
            .await?
            .into_iter()
            .map(|value| {
                serde_json::from_value(value)
                    .map_err(|error| RuntimeError::Application(error.to_string()))
            })
            .collect()
    }

    pub async fn import_figure_pack(
        &self,
        request: ImportFigurePackRequest,
    ) -> Result<FigurePack, RuntimeError> {
        let workspace = management::workspace_id(request.workspace)?;
        let database = self.runtime()?.database();
        database.workspace_record(&workspace).await?;
        let (manifest, frames) = tokio::task::spawn_blocking(move || unpack(request.bytes))
            .await
            .map_err(|error| RuntimeError::Application(error.to_string()))??;
        let resources = self.resource_store()?;
        let mut saved = Vec::new();
        let result = async {
            for (name, bytes) in ["work.png", "sleep.png", "carry.png", "stand.png"]
                .into_iter()
                .zip(frames)
            {
                saved.push(
                    resources
                        .save(
                            &workspace,
                            ResourceUpload {
                                file_name: name.into(),
                                media_type: "image/png".into(),
                                bytes,
                            },
                        )
                        .await?,
                );
            }
            let pack = FigurePack {
                id: uuid::Uuid::new_v4().to_string(),
                name: manifest.name,
                author: manifest.author,
                scale: manifest.scale,
                frames: FigureFrames {
                    work: saved[0].clone(),
                    sleep: saved[1].clone(),
                    carry: saved[2].clone(),
                    stand: saved[3].clone(),
                },
            };
            database
                .insert_figure_pack(
                    &workspace,
                    &pack.id,
                    serde_json::to_value(&pack).map_err(|_| invalid_pack())?,
                )
                .await?;
            Ok::<_, RuntimeError>(pack)
        }
        .await;
        if result.is_err() {
            for resource in saved {
                if let Err(error) = resources
                    .remove(
                        &workspace,
                        ResourceId::new(resource.resource_id).map_err(|_| invalid_pack())?,
                    )
                    .await
                {
                    log::error!("figure import rollback failed: {error}");
                }
            }
        } else {
            self.emit_management(
                meerkat_protocol::ManagementEventKind::CatalogChanged,
                &workspace,
                None,
                now_millis(),
            );
        }
        result
    }
}
