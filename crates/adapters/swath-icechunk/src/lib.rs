// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Icechunk interop adapter: the write half commits a [`VirtualManifest`]'s
//! byte-range references to
//! an **Icechunk repository** (ADR 0016's interop half, #191; spec target
//! recorded in ADR 0017) — the granule joins the VirtualiZarr→Icechunk
//! ecosystem instead of living only in Swath's private manifest format.
//!
//! [`commit_manifest`] maps each manifest array to a Zarr v3 array whose
//! chunks are **virtual chunk references** (`file://` byte ranges into the
//! original granule) and commits the whole tree in one Icechunk snapshot:
//!
//! - array names become the Zarr hierarchy (`HDFEOS/GRIDS/…/nir`), with
//!   explicit group metadata for every ancestor;
//! - the manifest codec chain (HDF5 filter-pipeline order) becomes the
//!   Zarr v3 codec chain `bytes → numcodecs.shuffle → numcodecs.zlib` —
//!   the same numcodecs vocabulary VirtualiZarr/kerchunk write, so
//!   icechunk-python/zarr-python decode the chunks without Swath;
//! - dimension names are `phony_dim_<n>` allocated per distinct size
//!   (h5py's `phony_dims="sort"` convention), keeping xarray's
//!   equal-size-per-name rule satisfied by construction.
//!
//! **Skips are loud, never silent:** arrays whose dtype or codec chain has
//! no Zarr v3 mapping (byte-string metadata blobs, GRIB2 packing) are
//! returned in [`CommitOutcome::skipped`] with a reason each; everything
//! committed is listed in [`CommitOutcome::committed`]. The conformance
//! gate (`just test-referencer`) opens the result with icechunk-python +
//! xarray and compares pixel values against the HDF5 source.
//!
//! Local-filesystem repositories only for now (the `object-store-fs`
//! feature is the only Icechunk backend compiled in); an object-store
//! deployment target arrives with a real deployment, feature-flagged then.
//!
//! The read half — [`IcechunkSource`], serving tiles **from a commit**,
//! byte-identical to the manifest path (#193) — lives in [`source`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use icechunk::config::Credentials;
use icechunk::format::ChunkIndices;
use icechunk::format::manifest::{VirtualChunkLocation, VirtualChunkRef};
use icechunk::store::Store;
use icechunk::virtual_chunks::VirtualChunkContainer;
use icechunk::{ObjectStoreConfig, Repository, RepositoryConfig, new_local_filesystem_storage};
use swath_manifest::{VirtualArray, VirtualManifest};

mod source;
pub use source::IcechunkSource;

/// What can go wrong committing a manifest to an Icechunk repository.
///
/// Per-array representability problems are **not** errors — they come back
/// as [`CommitOutcome::skipped`] entries (a metadata blob that cannot be a
/// Zarr array must not fail the granule). Only structural problems surface
/// here.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommitError {
    /// The manifest carries something structurally unusable (a chunk key
    /// that does not parse, an array name Icechunk rejects).
    #[error("manifest not representable: {detail}")]
    Manifest {
        /// What was wrong, naming the offending array/chunk.
        detail: String,
    },

    /// A chunk ref's `path` could not be resolved under the source root
    /// into a `file://` URL.
    #[error("chunk path `{path}` does not resolve under `{root}`")]
    SourcePath {
        /// The manifest's chunk path.
        path: String,
        /// The source root it was resolved against.
        root: String,
    },

    /// The Icechunk machinery failed (storage, session, commit).
    #[error("icechunk failure: {detail}")]
    Icechunk {
        /// What was being attempted.
        detail: String,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Nothing in the manifest was representable — an empty commit would
    /// claim interop that does not exist.
    #[error("no representable arrays in the manifest ({skipped} skipped)")]
    NothingToCommit {
        /// How many arrays were skipped.
        skipped: usize,
    },
}

/// One array left out of the commit, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedArray {
    /// The manifest array name.
    pub name: String,
    /// The honest reason (unmappable dtype, unmappable codec, …).
    pub reason: String,
}

/// The result of a successful commit.
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// The Icechunk snapshot id of the commit.
    pub snapshot_id: String,
    /// Arrays committed, in manifest order.
    pub committed: Vec<String>,
    /// Arrays skipped (with reasons), in manifest order.
    pub skipped: Vec<SkippedArray>,
}

/// Wraps an underlying Icechunk error with what was being attempted.
fn ice<E: std::error::Error + Send + Sync + 'static>(
    detail: &str,
) -> impl FnOnce(E) -> CommitError {
    let detail = detail.to_owned();
    move |source| CommitError::Icechunk {
        detail,
        source: Box::new(source),
    }
}

/// Commits `manifest`'s virtual references to the Icechunk repository at
/// `repo_dir` (created if absent, committed to `main`), resolving each
/// chunk ref's `path` against `source_root` into a `file://` location.
///
/// The repository is configured with one virtual-chunk container covering
/// `source_root`, so readers must (and Swath's own reader does) authorize
/// exactly that prefix — Icechunk's safe-by-default posture for foreign
/// byte ranges.
///
/// # Errors
///
/// [`CommitError`]; see its variants. Per-array representability gaps are
/// reported in [`CommitOutcome::skipped`], not errors.
pub async fn commit_manifest(
    repo_dir: &Path,
    manifest: &VirtualManifest,
    source_root: &Path,
    message: &str,
) -> Result<CommitOutcome, CommitError> {
    let root_url = file_url(source_root)?;
    let container_prefix = format!("{root_url}/");

    let mut config = RepositoryConfig::default();
    let container = VirtualChunkContainer::new(
        container_prefix.clone(),
        ObjectStoreConfig::LocalFileSystem(source_root.to_path_buf()),
    )
    .map_err(|detail| CommitError::Manifest {
        detail: format!("virtual chunk container rejected: {detail}"),
    })?;
    config
        .set_virtual_chunk_container(container)
        .map_err(|detail| CommitError::Manifest {
            detail: format!("virtual chunk container rejected: {detail}"),
        })?;

    let storage = new_local_filesystem_storage(repo_dir)
        .await
        .map_err(ice("opening local filesystem storage"))?;
    let authorized: HashMap<String, Option<Credentials>> = HashMap::from([(
        container_prefix.clone(),
        Some(Credentials::LocalFileSystemAccess),
    )]);

    let repo = if Repository::exists(Arc::clone(&storage), None)
        .await
        .map_err(ice("probing repository"))?
    {
        Repository::open(Some(config), Arc::clone(&storage), authorized)
            .await
            .map_err(ice("opening repository"))?
    } else {
        Repository::create(Some(config), Arc::clone(&storage), authorized, None, true)
            .await
            .map_err(ice("creating repository"))?
    };

    let session = repo
        .writable_session("main")
        .await
        .map_err(ice("opening writable session"))?;
    let store = Store::from_session(Arc::new(tokio::sync::RwLock::new(session))).await;

    // Pass 1 — classify every array (skips are honest, never silent).
    let mut committable: Vec<(&VirtualArray, Mapping)> = Vec::new();
    let mut skipped = Vec::new();
    for array in &manifest.arrays {
        match representable(array) {
            Err(reason) => skipped.push(SkippedArray {
                name: array.name.clone(),
                reason,
            }),
            Ok(mapping) => committable.push((array, mapping)),
        }
    }
    if committable.is_empty() {
        return Err(CommitError::NothingToCommit {
            skipped: skipped.len(),
        });
    }

    // Pass 2 — group metadata first (parents before children: BTreeMap
    // order gives that for slash-separated paths), so every array lands
    // under an explicit Zarr v3 group.
    let mut groups: BTreeSet<String> = BTreeSet::new();
    groups.insert(String::new());
    for (array, _) in &committable {
        groups.extend(ancestors(&array.name));
    }
    for group in &groups {
        let key = if group.is_empty() {
            "zarr.json".to_owned()
        } else {
            format!("{group}/zarr.json")
        };
        let doc = serde_json::json!({"zarr_format": 3, "node_type": "group"});
        store
            .set(&key, serde_json::to_vec(&doc).expect("static json").into())
            .await
            .map_err(ice(&format!("writing group metadata `{key}`")))?;
    }

    // Pass 3 — arrays: metadata, then their virtual refs.
    let mut committed = Vec::new();
    let mut dims = PhonyDims::default();
    for (array, mapping) in &committable {
        write_array(&store, array, mapping, &mut dims, source_root, &root_url).await?;
        committed.push(array.name.clone());
    }

    let session = store.session();
    let mut session = session.write().await;
    let snapshot = session
        .commit(message)
        .execute()
        .await
        .map_err(ice("committing snapshot"))?;

    Ok(CommitOutcome {
        snapshot_id: snapshot.to_string(),
        committed,
        skipped,
    })
}

/// How one manifest array maps onto Zarr v3: dtype name, sample size,
/// fill value, codec chain.
struct Mapping {
    dtype: &'static str,
    codecs: Vec<serde_json::Value>,
}

/// Decides whether `array` is representable, and how. `Err` carries the
/// honest skip reason.
fn representable(array: &VirtualArray) -> Result<Mapping, String> {
    let (dtype, dtype_size) = match array.dtype.as_str() {
        "int8" => ("int8", 1),
        "uint8" => ("uint8", 1),
        "int16" => ("int16", 2),
        "uint16" => ("uint16", 2),
        "int32" => ("int32", 4),
        "uint32" => ("uint32", 4),
        "int64" => ("int64", 8),
        "uint64" => ("uint64", 8),
        "float32" => ("float32", 4),
        "float64" => ("float64", 8),
        other => {
            return Err(format!(
                "dtype `{other}` has no Zarr v3 mapping (metadata blob, not pixels)"
            ));
        }
    };

    // Zarr v3 encode order: array→bytes first, then bytes→bytes codecs in
    // the manifest's filter-pipeline (encode) order.
    let mut codecs = vec![serde_json::json!({
        "name": "bytes",
        "configuration": {"endian": "little"}
    })];
    for codec in &array.codecs {
        if codec == "shuffle" {
            codecs.push(serde_json::json!({
                "name": "numcodecs.shuffle",
                "configuration": {"elementsize": dtype_size}
            }));
        } else if let Some(level) = codec.strip_prefix("zlib:") {
            let level: u8 = level
                .parse()
                .map_err(|_| format!("zlib level `{level}` is not a number"))?;
            codecs.push(serde_json::json!({
                "name": "numcodecs.zlib",
                "configuration": {"level": level}
            }));
        } else {
            return Err(format!("codec `{codec}` has no Zarr v3 mapping"));
        }
    }

    Ok(Mapping { dtype, codecs })
}

/// Allocates `phony_dim_<n>` names per distinct `(size, occurrence)`:
/// every k-th dimension of a given size *within one array* shares a name
/// across arrays (so xarray's equal-size-per-name rule holds by
/// construction), while a square array's two axes get **distinct** names
/// (xarray rejects duplicate dims within one variable — the netCDF rule
/// h5py's phony dims follow too).
#[derive(Default)]
struct PhonyDims {
    by_key: BTreeMap<(u64, usize), usize>,
}

impl PhonyDims {
    /// Names every dimension of one array's `shape`.
    fn names(&mut self, shape: &[u64]) -> Vec<String> {
        let mut seen: BTreeMap<u64, usize> = BTreeMap::new();
        shape
            .iter()
            .map(|&size| {
                let occurrence = seen.entry(size).or_insert(0);
                let key = (size, *occurrence);
                *occurrence += 1;
                let next = self.by_key.len();
                let index = *self.by_key.entry(key).or_insert(next);
                format!("phony_dim_{index}")
            })
            .collect()
    }
}

/// Writes one array's Zarr metadata and virtual chunk refs.
async fn write_array(
    store: &Store,
    array: &VirtualArray,
    mapping: &Mapping,
    dims: &mut PhonyDims,
    source_root: &Path,
    root_url: &str,
) -> Result<(), CommitError> {
    let dimension_names = dims.names(&array.shape);
    let fill_value = fill_value(mapping.dtype, array);
    // Georeferencing travels in a namespaced attribute (the manifest's own
    // `Georef` shape, serde-identical): any consumer can read it, and the
    // read-back adapter (`IcechunkSource`, #193) reconstructs the exact
    // `RasterInfo` the manifest path serves from — the byte-identical
    // serving equivalence rests on this being lossless.
    let mut attributes = serde_json::Map::new();
    if let Some(georef) = &array.georef {
        attributes.insert(
            "swath:georef".to_owned(),
            serde_json::to_value(georef).expect("georef is a plain serde tree"),
        );
    }
    let metadata = serde_json::json!({
        "zarr_format": 3,
        "node_type": "array",
        "shape": array.shape,
        "data_type": mapping.dtype,
        "chunk_grid": {
            "name": "regular",
            "configuration": {"chunk_shape": array.chunks}
        },
        "chunk_key_encoding": {"name": "default"},
        "fill_value": fill_value,
        "codecs": mapping.codecs,
        "dimension_names": dimension_names,
        "attributes": attributes
    });
    let key = format!("{}/zarr.json", array.name);
    store
        .set(
            &key,
            serde_json::to_vec(&metadata)
                .expect("plain json tree")
                .into(),
        )
        .await
        .map_err(ice(&format!("writing array metadata `{key}`")))?;

    let array_path: icechunk::format::Path =
        format!("/{}", array.name)
            .try_into()
            .map_err(|err| CommitError::Manifest {
                detail: format!("array name `{}` rejected: {err:?}", array.name),
            })?;

    let mut refs = Vec::with_capacity(array.refs.len());
    for chunk in &array.refs {
        let indices =
            parse_key(&chunk.key, array.shape.len()).ok_or_else(|| CommitError::Manifest {
                detail: format!(
                    "chunk key `{}` of `{}` does not parse",
                    chunk.key, array.name
                ),
            })?;
        let location = chunk_location(&chunk.path, source_root, root_url)?;
        refs.push((
            ChunkIndices(indices),
            VirtualChunkRef {
                location,
                offset: chunk.offset,
                length: chunk.length,
                checksum: None,
            },
        ));
    }
    let result = store
        .set_virtual_refs(&array_path, true, refs)
        .await
        .map_err(ice(&format!("writing virtual refs for `{}`", array.name)))?;
    if let icechunk::store::SetVirtualRefsResult::FailedRefs(failed) = result {
        return Err(CommitError::Manifest {
            detail: format!(
                "{} virtual refs of `{}` rejected by the container (first: {:?})",
                failed.len(),
                array.name,
                failed.first()
            ),
        });
    }
    Ok(())
}

/// The Zarr v3 fill value for an array: the georef nodata when it is
/// exactly representable in the dtype, otherwise 0 (fill only matters for
/// unallocated chunks; comparisons run on stored chunks).
fn fill_value(dtype: &str, array: &VirtualArray) -> serde_json::Value {
    let nodata = array.georef.as_ref().and_then(|g| g.nodata);
    match (dtype, nodata) {
        ("float32" | "float64", Some(nd)) if nd.is_nan() => serde_json::json!("NaN"),
        ("float32" | "float64", Some(nd)) => serde_json::json!(nd),
        #[allow(
            clippy::cast_possible_truncation,
            reason = "integrality and range checked before the cast"
        )]
        (_, Some(nd)) if nd.fract() == 0.0 && i64::from(i32::MIN) <= nd as i64 => {
            serde_json::json!(nd as i64)
        }
        _ => serde_json::json!(0),
    }
}

/// `"0.1"` → `[0, 1]`; `""` (scalar) → `[]`, checked against the rank.
fn parse_key(key: &str, rank: usize) -> Option<Vec<u32>> {
    if key.is_empty() {
        return (rank == 0).then(Vec::new);
    }
    let parts: Vec<u32> = key
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (parts.len() == rank).then_some(parts)
}

/// Every proper ancestor path of a slash-separated name (excluding the
/// name itself and the root).
fn ancestors(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for part in name
        .split('/')
        .take(name.split('/').count().saturating_sub(1))
    {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        out.push(acc.clone());
    }
    out
}

/// A filesystem path as a `file://` URL (percent-encoded correctly).
fn file_url(path: &Path) -> Result<String, CommitError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| CommitError::SourcePath {
        path: path.display().to_string(),
        root: String::from("(canonicalize failed)"),
    })?;
    let url = url::Url::from_file_path(&canonical).map_err(|()| CommitError::SourcePath {
        path: path.display().to_string(),
        root: String::from("(not an absolute path)"),
    })?;
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

/// A chunk ref's location: its `path` resolved to a real file — as given
/// (absolute, or relative to the working directory: manifests carry the
/// granule path exactly as the generator received it), else under
/// `source_root` (store-relative manifests).
fn chunk_location(
    chunk_path: &str,
    source_root: &Path,
    root_url: &str,
) -> Result<VirtualChunkLocation, CommitError> {
    let as_given = Path::new(chunk_path);
    let joined = source_root.join(chunk_path);
    let resolved = if as_given.is_file() {
        as_given
    } else if joined.is_file() {
        joined.as_path()
    } else {
        return Err(CommitError::SourcePath {
            path: chunk_path.to_owned(),
            root: source_root.display().to_string(),
        });
    };
    let url = file_url(resolved)?;
    // Container safety: the resolved file must sit under the container
    // prefix (file_url canonicalizes, so `..` cannot escape it silently).
    if !url.starts_with(root_url) {
        return Err(CommitError::SourcePath {
            path: chunk_path.to_owned(),
            root: source_root.display().to_string(),
        });
    }
    VirtualChunkLocation::from_url(&url).map_err(|err| CommitError::Manifest {
        detail: format!("chunk location `{url}` rejected: {err}"),
    })
}
