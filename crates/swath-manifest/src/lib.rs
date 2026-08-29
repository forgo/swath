// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
//!
//! # Georeferencing vocabulary
//!
//! Serving needs each array's spatial identity, so v1 adds an optional
//! per-array [`Georef`]: CRS + geotransform + nodata + band semantics. The
//! CRS is the manifest's **own** vocabulary ([`GeorefCrs`]): VNP09GA's grid
//! is MODIS-heritage sinusoidal with **no EPSG code**, so alongside
//! `{"epsg": N}` the manifest can carry `{"proj4": "+proj=sinu …"}` (a proj
//! string). How a consumer resolves a manifest CRS into projection math is
//! the consumer's decision, made where its reprojection wire-up lives — the
//! manifest just records the identity losslessly.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The schema version this module reads and writes.
pub const MANIFEST_VERSION: u32 = 1;

/// The `manifest_version` field: serializes as its number, deserialization
/// rejects any value other than [`MANIFEST_VERSION`] — an unknown schema
/// version is a loud error at the boundary, never a half-parsed manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(into = "u32")]
pub struct ManifestVersion;

impl From<ManifestVersion> for u32 {
    fn from(_: ManifestVersion) -> Self {
        MANIFEST_VERSION
    }
}

impl<'de> Deserialize<'de> for ManifestVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = u32::deserialize(deserializer)?;
        if version == MANIFEST_VERSION {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported manifest_version {version} (this reader understands {MANIFEST_VERSION})"
            )))
        }
    }
}

/// A legacy granule described as virtual references: schema v1.
///
/// Unknown fields are rejected (`deny_unknown_fields`): a manifest carrying
/// fields this version does not understand is version skew, not noise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualManifest {
    /// Schema version — always [`MANIFEST_VERSION`]; other values fail
    /// deserialization.
    pub manifest_version: ManifestVersion,
    /// Which generator produced this manifest (e.g. `swath-referencer`,
    /// `virtualizarr`). Informational: excluded from [`compare`].
    pub generator: String,
    /// The granule this manifest references (path or URI, as given to the
    /// generator).
    pub source: String,
    /// The arrays, in the generator's traversal order (depth-first datasets
    /// then subgroups for HDF5 — both generators agree on order, prototype
    /// 0001 §7).
    pub arrays: Vec<VirtualArray>,
}

/// One array of the virtual cube: chunk grid, dtype, codec chain, optional
/// georeferencing, and the chunk byte ranges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualArray {
    /// The array's name: the HDF5 path without the leading slash
    /// (`HDFEOS/GRIDS/…/SurfReflect_M1_1`), or the cfgrib-style variable
    /// name for GRIB2 messages.
    pub name: String,
    /// Dimension sizes.
    pub shape: Vec<u64>,
    /// Chunk shape (= `shape` for contiguous storage).
    pub chunks: Vec<u64>,
    /// Numpy-style dtype string (`int16`, `float64`, `|S32000`, …) — the
    /// vocabulary both generators derive independently (prototype 0001 §3).
    pub dtype: String,
    /// Codec chain, in **filter-pipeline (encode) order** — the order the
    /// HDF5 pipeline lists its filters (e.g. `["shuffle", "zlib:4"]`), which
    /// both generators emit identically. Readers decoding a chunk apply the
    /// chain in *reverse* (inflate, then unshuffle). Vocabulary: the shared
    /// codec strings (`zlib:8`, `shuffle`, `grib2:complex-spatial-diff`, …).
    /// (Earlier docs said "decode order"; the committed fixtures and both
    /// generators have always emitted pipeline order — the wording was
    /// corrected in #39, the bytes never changed.)
    pub codecs: Vec<String>,
    /// Spatial identity, when the generator could derive one (module docs).
    /// Absent for non-spatial arrays (coordinate vectors, metadata blobs)
    /// and excluded from [`compare`] — georef correctness is asserted by the
    /// generator's own known-answer tests, not cross-generator equivalence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub georef: Option<Georef>,
    /// The chunk byte ranges. Unallocated chunks are absent; an array with
    /// no allocated storage has an empty list.
    pub refs: Vec<ChunkRef>,
}

/// One chunk's byte range into a source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkRef {
    /// Dotted chunk-grid position (`"0.0"`, `"1.2"`; `"0"` per rank for
    /// whole-array refs, `""` for scalars).
    pub key: String,
    /// The file holding the bytes (usually the manifest's `source`).
    pub path: String,
    /// Byte offset of the chunk within `path`.
    pub offset: u64,
    /// Stored (compressed) length in bytes.
    pub length: u64,
}

/// Affine pixel↔CRS mapping, GDAL's six-parameter convention:
///
/// ```text
/// x = origin_x + col * pixel_width  + row * row_rotation
/// y = origin_y + col * col_rotation + row * pixel_height
/// ```
///
/// `(origin_x, origin_y)` is the CRS position of the **top-left corner of the
/// top-left pixel**; `(col, row)` are fractional pixel coordinates measured
/// from that corner. Rows are stored north-up in the common case:
/// `pixel_height` is **negative** (y decreases as `row` grows southward) and
/// both rotation terms are zero.
///
/// This is the manifest's own record of the mapping — six named numbers,
/// data only. Geometry (inversion, windowing) lives with the consumer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoTransform {
    /// CRS x of the top-left corner of pixel (0, 0).
    pub origin_x: f64,
    /// Column step in CRS x units (GDAL `GT(1)`); positive east-up.
    pub pixel_width: f64,
    /// Row step in CRS x units (GDAL `GT(2)`); zero for axis-aligned rasters.
    pub row_rotation: f64,
    /// CRS y of the top-left corner of pixel (0, 0).
    pub origin_y: f64,
    /// Column step in CRS y units (GDAL `GT(4)`); zero for axis-aligned rasters.
    pub col_rotation: f64,
    /// Row step in CRS y units (GDAL `GT(5)`); **negative** for north-up rasters.
    pub pixel_height: f64,
}

impl GeoTransform {
    /// An axis-aligned, north-up transform (both rotation terms zero).
    /// `pixel_height` should be negative per the north-up convention.
    #[must_use]
    pub const fn north_up(
        origin_x: f64,
        origin_y: f64,
        pixel_width: f64,
        pixel_height: f64,
    ) -> Self {
        Self {
            origin_x,
            pixel_width,
            row_rotation: 0.0,
            origin_y,
            col_rotation: 0.0,
            pixel_height,
        }
    }
}

/// Per-array georeferencing: everything serving needs to place the array's
/// pixel grid on the planet (module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Georef {
    /// The grid's CRS, in the manifest's own vocabulary.
    pub crs: GeorefCrs,
    /// Pixel↔CRS affine mapping (GDAL convention, top-left anchored).
    pub transform: GeoTransform,
    /// Nodata sentinel, widened to `f64` (GDAL convention), when declared
    /// (HDF5 `_FillValue`, when numeric).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodata: Option<f64>,
    /// Band semantics: the science name of what the samples mean (the field
    /// name, e.g. `SurfReflect_M1_1`) — the hook dataset band vocabularies
    /// map onto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
}

/// A CRS as a manifest records it: an EPSG code when one exists, otherwise a
/// proj string (VNP09GA's MODIS-heritage sinusoidal grid has no EPSG code —
/// module docs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GeorefCrs {
    /// An EPSG-registered CRS: `{"epsg": 32613}`.
    Epsg(u32),
    /// A proj-string definition: `{"proj4": "+proj=sinu +R=6371007.181 …"}`.
    Proj4(String),
}

impl VirtualManifest {
    /// Parses a manifest from its JSON text, validating the schema version.
    ///
    /// # Errors
    ///
    /// [`ManifestError::Json`] naming what failed — including an unsupported
    /// `manifest_version`.
    pub fn from_json_str(text: &str) -> Result<Self, ManifestError> {
        serde_json::from_str(text).map_err(|e| ManifestError::Json {
            detail: e.to_string(),
        })
    }

    /// Serializes the manifest as pretty-printed JSON text.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        // Infallible: the manifest is a plain struct tree (string-keyed,
        // no fallible Serialize impls).
        let mut text =
            serde_json::to_string_pretty(self).expect("manifest serialization is infallible");
        text.push('\n');
        text
    }
}

/// What can go wrong reading a manifest document.
///
/// Hand-implemented `Display`/`Error` (no derive dependency): the published
/// crate's tree stays exactly serde + `serde_json` (ADR 0016's zero-new-deps
/// rule, and the design note's recorded dep set).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManifestError {
    /// The text is not a valid v1 manifest document (malformed JSON, missing
    /// fields, unknown fields, or an unsupported `manifest_version`).
    Json {
        /// The underlying parse/validation failure.
        detail: String,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { detail } => write!(f, "invalid manifest document: {detail}"),
        }
    }
}

impl std::error::Error for ManifestError {}

// ---------- equivalence (the conformance check, from prototype 0001) ----------

/// The outcome of [`compare`]: array counts plus every observed mismatch,
/// phrased for the human reading a conformance-run report.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EquivalenceReport {
    /// Array count in manifest A.
    pub arrays_a: usize,
    /// Array count in manifest B.
    pub arrays_b: usize,
    /// Arrays of A found (by name) in B.
    pub matched_arrays: usize,
    /// Shape/chunk-grid/dtype/codec disagreements, one line each.
    pub grid_mismatches: Vec<String>,
    /// Per-chunk (offset, length) disagreements, one line each.
    pub chunk_mismatches: Vec<String>,
}

impl EquivalenceReport {
    /// Whether the two manifests are equivalent: same arrays, same grids,
    /// same per-chunk byte ranges.
    #[must_use]
    pub fn equivalent(&self) -> bool {
        self.arrays_a == self.arrays_b
            && self.matched_arrays == self.arrays_a
            && self.grid_mismatches.is_empty()
            && self.chunk_mismatches.is_empty()
    }
}

/// Compares two manifests for reference equivalence: same arrays (by name),
/// same shape/chunk grid/dtype/codecs, same per-chunk (offset, length).
///
/// `generator`, `source`, ref `path`s, and `georef` are deliberately outside
/// the comparison: the first three legitimately differ between generators
/// naming the same granule; georef is single-generator territory (its truth
/// comes from known-answer tests, not cross-generator agreement).
#[must_use]
pub fn compare(a: &VirtualManifest, b: &VirtualManifest) -> EquivalenceReport {
    let mut report = EquivalenceReport {
        arrays_a: a.arrays.len(),
        arrays_b: b.arrays.len(),
        ..Default::default()
    };
    for arr_a in &a.arrays {
        let Some(arr_b) = b.arrays.iter().find(|x| x.name == arr_a.name) else {
            report
                .grid_mismatches
                .push(format!("array '{}' missing in B", arr_a.name));
            continue;
        };
        report.matched_arrays += 1;
        if arr_a.shape != arr_b.shape
            || arr_a.chunks != arr_b.chunks
            || arr_a.dtype != arr_b.dtype
            || arr_a.codecs != arr_b.codecs
        {
            report.grid_mismatches.push(format!(
                "array '{}' grid/dtype/codecs differ: \
                 A(shape={:?},chunks={:?},dtype={},codecs={:?}) vs \
                 B(shape={:?},chunks={:?},dtype={},codecs={:?})",
                arr_a.name,
                arr_a.shape,
                arr_a.chunks,
                arr_a.dtype,
                arr_a.codecs,
                arr_b.shape,
                arr_b.chunks,
                arr_b.dtype,
                arr_b.codecs
            ));
        }
        let by_key: BTreeMap<&str, &ChunkRef> =
            arr_b.refs.iter().map(|c| (c.key.as_str(), c)).collect();
        if arr_a.refs.len() != arr_b.refs.len() {
            report.chunk_mismatches.push(format!(
                "array '{}' ref count differs: A={} vs B={}",
                arr_a.name,
                arr_a.refs.len(),
                arr_b.refs.len()
            ));
        }
        for chunk_a in &arr_a.refs {
            match by_key.get(chunk_a.key.as_str()) {
                None => report
                    .chunk_mismatches
                    .push(format!("{}::{} missing in B", arr_a.name, chunk_a.key)),
                Some(chunk_b) => {
                    if chunk_a.offset != chunk_b.offset || chunk_a.length != chunk_b.length {
                        report.chunk_mismatches.push(format!(
                            "{}::{} offset/length differs: A({},{}) vs B({},{})",
                            arr_a.name,
                            chunk_a.key,
                            chunk_a.offset,
                            chunk_a.length,
                            chunk_b.offset,
                            chunk_b.length
                        ));
                    }
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::{ChunkRef, GeorefCrs, ManifestVersion, VirtualManifest, compare};

    fn manifest(arrays: Vec<super::VirtualArray>) -> VirtualManifest {
        VirtualManifest {
            manifest_version: ManifestVersion,
            generator: "test".to_owned(),
            source: "granule.h5".to_owned(),
            arrays,
        }
    }

    fn array(name: &str, offset: u64) -> super::VirtualArray {
        super::VirtualArray {
            name: name.to_owned(),
            shape: vec![4, 4],
            chunks: vec![2, 4],
            dtype: "int16".to_owned(),
            codecs: vec!["zlib:8".to_owned()],
            georef: None,
            refs: vec![ChunkRef {
                key: "0.0".to_owned(),
                path: "granule.h5".to_owned(),
                offset,
                length: 64,
            }],
        }
    }

    #[test]
    fn version_field_round_trips_and_rejects_others() {
        let m = manifest(vec![]);
        let text = m.to_json_string();
        assert!(text.contains("\"manifest_version\": 1"));
        assert_eq!(VirtualManifest::from_json_str(&text).unwrap(), m);

        let bumped = text.replace("\"manifest_version\": 1", "\"manifest_version\": 2");
        let err = VirtualManifest::from_json_str(&bumped).unwrap_err();
        assert!(err.to_string().contains("unsupported manifest_version 2"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut text = manifest(vec![]).to_json_string();
        text = text.replacen("\"generator\"", "\"surprise\": true,\n  \"generator\"", 1);
        assert!(VirtualManifest::from_json_str(&text).is_err());
    }

    #[test]
    fn equivalent_manifests_report_equivalent() {
        let a = manifest(vec![array("x", 100)]);
        let mut b = a.clone();
        b.generator = "other".to_owned();
        b.arrays[0].refs[0].path = "elsewhere/granule.h5".to_owned();
        let report = compare(&a, &b);
        assert!(report.equivalent(), "{report:?}");
        assert_eq!(report.matched_arrays, 1);
    }

    #[test]
    fn compare_flags_grid_chunk_and_count_mismatches() {
        let a = manifest(vec![array("x", 100), array("y", 200)]);

        // Missing array.
        let b = manifest(vec![array("x", 100)]);
        let report = compare(&a, &b);
        assert!(!report.equivalent());
        assert_eq!(report.grid_mismatches.len(), 1);

        // Byte-range drift.
        let mut b = a.clone();
        b.arrays[1].refs[0].offset = 999;
        let report = compare(&a, &b);
        assert!(!report.equivalent());
        assert_eq!(report.chunk_mismatches.len(), 1);

        // Codec drift is a grid mismatch.
        let mut b = a.clone();
        b.arrays[0].codecs = vec![];
        assert!(!compare(&a, &b).equivalent());

        // Extra refs in B are caught by the count check even when every
        // A-side key matches.
        let mut b = a.clone();
        b.arrays[0].refs.push(ChunkRef {
            key: "1.0".to_owned(),
            path: "granule.h5".to_owned(),
            offset: 500,
            length: 64,
        });
        assert!(!compare(&a, &b).equivalent());
    }

    #[test]
    fn georef_crs_serializes_in_both_vocabularies() {
        let epsg = serde_json::to_value(GeorefCrs::Epsg(32613)).unwrap();
        assert_eq!(epsg, serde_json::json!({"epsg": 32613}));
        let proj =
            serde_json::to_value(GeorefCrs::Proj4("+proj=sinu +R=6371007.181".into())).unwrap();
        assert_eq!(
            proj,
            serde_json::json!({"proj4": "+proj=sinu +R=6371007.181"})
        );
    }
}
