// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The read-back half (#193): a [`RasterSource`] serving tiles **from an
//! Icechunk commit** — the loop ADR 0016 closes: what
//! [`commit_manifest`](crate::commit_manifest) wrote, Swath serves back,
//! byte-identical to the manifest path and trace-visible.
//!
//! # How equivalence is engineered, not hoped for
//!
//! [`IcechunkSource`] reconstructs, per asset, exactly the two pieces the
//! manifest path serves from — a [`VirtualArray`] (shape/chunk grid/dtype/
//! codec chain/byte-range refs, read back from the commit's Zarr metadata
//! and virtual-ref table) and a [`Georef`] (the lossless `swath:georef`
//! attribute the writer embeds) — and then calls the **same serving core**
//! (`swath_source_virtual::read_array_window`): same chunk intersection,
//! same ranged fetches from the *original* granule, same codec decode,
//! same fill semantics, same [`Provenance`] pointing at the `.h5` file.
//! Byte-identical serving is a consequence of shared code over identical
//! inputs, with the equivalence tests pinning it.
//!
//! # Asset addressing: `<version>#<array-name>`
//!
//! The repository is fixed at construction; an asset names a version —
//! a branch (`main#…`, served from its tip) or a snapshot id
//! (`MNDH3ZWMN8QNVKH5M5TG#…`, the pinned form) — and the array path
//! within it, mirroring the manifest adapter's fragment convention.
//!
//! Reads are stateless per call (repository reopened, like the manifest
//! adapter's per-call manifest fetch); caching is a later, behavior-
//! preserving optimization.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use icechunk::format::manifest::ChunkPayload;
use icechunk::format::{ByteRange as IceByteRange, ChunkIndices};
use icechunk::repository::VersionInfo;
use icechunk::store::Store;
use icechunk::{Repository, new_local_filesystem_storage};
use object_store::ObjectStore;
use object_store::local::LocalFileSystem;
use swath_core::raster::{AssetRef, RasterInfo, WindowRequest};
use swath_core::source::{BandSelection, RasterSource, ReadLevel, SourceError, WindowData};
use swath_manifest::{ChunkRef, Georef, VirtualArray};
use swath_source_virtual::{array_raster_info, read_array_window};

/// A [`RasterSource`] serving arrays from a local Icechunk repository
/// (module docs; written by [`commit_manifest`](crate::commit_manifest)).
#[derive(Debug, Clone)]
pub struct IcechunkSource {
    repo_dir: PathBuf,
    /// Chunk fetches go straight to the referenced files (absolute-path
    /// keys over the filesystem root) — the same bytes, and the same
    /// provenance shape, as the manifest path.
    files: Arc<dyn ObjectStore>,
}

impl IcechunkSource {
    /// A source over the Icechunk repository at `repo_dir`.
    #[must_use]
    pub fn new(repo_dir: impl Into<PathBuf>) -> Self {
        Self {
            repo_dir: repo_dir.into(),
            files: Arc::new(LocalFileSystem::new()),
        }
    }

    /// Splits `<version>#<array-name>`, refusing fragment-less URIs.
    fn split(asset: &AssetRef) -> Result<(&str, &str), SourceError> {
        let Some((version, array)) = asset.as_str().split_once('#') else {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: "icechunk assets are addressed as `<version>#<array-name>` \
                         (branch name or snapshot id, then the array path); no \
                         `#<array-name>` fragment present"
                    .to_owned(),
            });
        };
        if version.is_empty() || array.is_empty() {
            return Err(SourceError::Format {
                asset: asset.clone(),
                detail: "empty `<version>` or `#<array-name>` part".to_owned(),
            });
        }
        Ok((version, array))
    }

    /// Reconstructs the served pieces from the commit: the array (metadata
    /// + virtual refs) and its embedded georef.
    #[allow(
        clippy::too_many_lines,
        reason = "one linear reconstruction — open, resolve version, parse \
                  metadata, read refs; splitting would scatter the borrow of \
                  one session across helpers"
    )]
    async fn load_array(&self, asset: &AssetRef) -> Result<(VirtualArray, Georef), SourceError> {
        let (version, array_name) = Self::split(asset)?;
        let io = |detail: String| {
            let asset = asset.clone();
            move |source: Box<dyn std::error::Error + Send + Sync>| SourceError::Io {
                asset,
                source: Box::new(std::io::Error::other(format!("{detail}: {source}"))),
            }
        };

        let storage = new_local_filesystem_storage(&self.repo_dir)
            .await
            .map_err(|e| io("opening icechunk storage".to_owned())(Box::new(e)))?;
        // No virtual-chunk authorization: this adapter reads the ref TABLE
        // and fetches ranges itself; it never asks Icechunk to fetch.
        let repo = Repository::open(None, storage, HashMap::new())
            .await
            .map_err(|e| io("opening icechunk repository".to_owned())(Box::new(e)))?;

        // A branch name serves its tip; anything else must parse as a
        // snapshot id (the pinned form).
        let version_info = if repo
            .list_branches()
            .await
            .map_err(|e| io("listing branches".to_owned())(Box::new(e)))?
            .contains(version)
        {
            VersionInfo::BranchTipRef(version.to_owned())
        } else {
            match version.try_into() {
                Ok(id) => VersionInfo::SnapshotId(id),
                Err(_) => {
                    return Err(SourceError::Format {
                        asset: asset.clone(),
                        detail: format!(
                            "`{version}` is neither a branch of the repository nor a \
                             valid snapshot id"
                        ),
                    });
                }
            }
        };
        let session = repo
            .readonly_session(&version_info)
            .await
            .map_err(|e| io(format!("opening version `{version}`"))(Box::new(e)))?;
        let store = Store::from_session(Arc::new(tokio::sync::RwLock::new(session))).await;

        // Zarr metadata → shape / chunk grid / dtype / codecs / georef.
        let metadata = store
            .get(&format!("{array_name}/zarr.json"), &IceByteRange::ALL)
            .await
            .map_err(|_| SourceError::NotFound {
                asset: asset.clone(),
            })?;
        let metadata: serde_json::Value =
            serde_json::from_slice(&metadata).map_err(|e| SourceError::Format {
                asset: asset.clone(),
                detail: format!("array metadata is not JSON: {e}"),
            })?;
        let format = |detail: String| SourceError::Format {
            asset: asset.clone(),
            detail,
        };
        let shape = dims(&metadata["shape"])
            .ok_or_else(|| format("metadata `shape` is not a dimension list".into()))?;
        let chunks = dims(&metadata["chunk_grid"]["configuration"]["chunk_shape"])
            .ok_or_else(|| format("metadata `chunk_shape` is not a dimension list".into()))?;
        let dtype = metadata["data_type"]
            .as_str()
            .ok_or_else(|| format("metadata `data_type` is not a string".into()))?
            .to_owned();
        let codecs = manifest_codecs(&metadata["codecs"])
            .map_err(|detail| format(format!("codec chain not servable: {detail}")))?;
        let georef: Georef = serde_json::from_value(metadata["attributes"]["swath:georef"].clone())
            .map_err(|_| SourceError::Unsupported {
                asset: asset.clone(),
                detail: format!(
                    "array `{array_name}` carries no `swath:georef` attribute; only \
                 georeferenced arrays committed by swath are servable rasters"
                ),
            })?;

        // The virtual-ref table → manifest-shaped chunk refs whose paths
        // are absolute-file keys into this adapter's filesystem store.
        let session = store.session();
        let session = session.read().await;
        let array_path: icechunk::format::Path = format!("/{array_name}")
            .try_into()
            .map_err(|e| format(format!("array path rejected: {e:?}")))?;
        let mut refs = Vec::new();
        for (row, col) in grid_positions(&shape, &chunks) {
            let indices = ChunkIndices(vec![
                u32::try_from(row).map_err(|_| format("chunk row overflows u32".into()))?,
                u32::try_from(col).map_err(|_| format("chunk col overflows u32".into()))?,
            ]);
            let payload = session
                .get_chunk_ref(&array_path, &indices)
                .await
                .map_err(|e| io(format!("reading chunk ref {row}.{col}"))(Box::new(e)))?;
            match payload {
                None => {} // unallocated: fill semantics, like the manifest path
                Some(ChunkPayload::Virtual(virtual_ref)) => {
                    let path = file_key(virtual_ref.location.url()).ok_or_else(|| {
                        format(format!(
                            "chunk {row}.{col} location `{}` is not a file:// URL",
                            virtual_ref.location.url()
                        ))
                    })?;
                    refs.push(ChunkRef {
                        key: format!("{row}.{col}"),
                        path,
                        offset: virtual_ref.offset,
                        length: virtual_ref.length,
                    });
                }
                Some(other) => {
                    return Err(SourceError::Unsupported {
                        asset: asset.clone(),
                        detail: format!(
                            "chunk {row}.{col} is not a virtual reference ({other:?}); \
                             this adapter serves swath-committed virtual stores"
                        ),
                    });
                }
            }
        }

        let array = VirtualArray {
            name: array_name.to_owned(),
            shape,
            chunks,
            dtype,
            codecs,
            georef: Some(georef.clone()),
            refs,
        };
        Ok((array, georef))
    }
}

impl RasterSource for IcechunkSource {
    async fn describe(&self, asset: &AssetRef) -> Result<RasterInfo, SourceError> {
        let (array, georef) = self.load_array(asset).await?;
        array_raster_info(asset, &array, &georef)
    }

    async fn read_window(
        &self,
        asset: &AssetRef,
        window: WindowRequest,
        band: BandSelection,
        level: ReadLevel,
    ) -> Result<WindowData, SourceError> {
        let (array, georef) = self.load_array(asset).await?;
        read_array_window(&self.files, asset, &array, &georef, window, band, level).await
    }
}

/// A JSON dimension list as `Vec<u64>`.
fn dims(value: &serde_json::Value) -> Option<Vec<u64>> {
    value
        .as_array()?
        .iter()
        .map(serde_json::Value::as_u64)
        .collect()
}

/// Zarr v3 codec chain (as the writer emits it) → the manifest codec
/// vocabulary the serving core decodes. The leading `bytes` codec is the
/// array→bytes stage (little-endian by the writer's construction); the
/// rest must be the numcodecs pair the manifests use.
fn manifest_codecs(value: &serde_json::Value) -> Result<Vec<String>, String> {
    let list = value.as_array().ok_or("codec list missing")?;
    let mut out = Vec::new();
    for codec in list {
        let name = codec["name"].as_str().ok_or("codec name missing")?;
        match name {
            "bytes" => {}
            "numcodecs.shuffle" => out.push("shuffle".to_owned()),
            "numcodecs.zlib" => {
                let level = codec["configuration"]["level"]
                    .as_u64()
                    .ok_or("zlib level missing")?;
                out.push(format!("zlib:{level}"));
            }
            other => return Err(format!("codec `{other}` is not servable")),
        }
    }
    Ok(out)
}

/// Every (row, col) of the chunk grid covering `shape`.
fn grid_positions(shape: &[u64], chunks: &[u64]) -> Vec<(u64, u64)> {
    let (&[rows, cols], &[chunk_rows, chunk_cols]) =
        (&shape[..2.min(shape.len())], &chunks[..2.min(chunks.len())])
    else {
        return Vec::new();
    };
    if chunk_rows == 0 || chunk_cols == 0 {
        return Vec::new();
    }
    let grid_rows = rows.div_ceil(chunk_rows);
    let grid_cols = cols.div_ceil(chunk_cols);
    let mut out = Vec::new();
    for row in 0..grid_rows {
        for col in 0..grid_cols {
            out.push((row, col));
        }
    }
    out
}

/// A `file://` URL → an absolute-path key into [`LocalFileSystem::new`]
/// (which roots keys at `/`).
fn file_key(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }
    let path = parsed.to_file_path().ok()?;
    let key = object_store::path::Path::from_absolute_path(path).ok()?;
    Some(key.to_string())
}
