// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The persistent pyramid layout: content-derived roots, Zarr v2 metadata
//! documents, and the pure level/ladder geometry.
//!
//! # Layout v1 (the storage contract)
//!
//! One pyramid per source asset, under a content-derived root (the same
//! sharding scheme as the tile cache):
//!
//! ```text
//! pyramids/<hh>/<hh>/<sha256(asset uri)>/
//!   .zgroup            {"zarr_format": 2}
//!   .zattrs            multiscales + the swath:pyramid identity document
//!   <factor>/.zarray   Zarr v2 array metadata (shape, chunks, dtype, fill)
//!   <factor>/<r>.<c>   raw little-endian C-order chunk, padded to full size
//! ```
//!
//! Every document is **plain Zarr v2** — `compressor: null`, C order,
//! little-endian dtype, `dimension_separator: "."` — so any Zarr reader
//! (zarr-python, `zarrs`) opens the group as a standard multiscale store;
//! nothing about the layout is private to Swath. The group `.zattrs`
//! carries an OME-style `multiscales` entry naming each level array by its
//! decimation factor, plus the `swath:pyramid` document that makes the
//! pyramid self-describing and *verifiable*: the source asset URI, its
//! full-resolution grid, CRS/transform, nodata, the resampling used, and
//! the list of **completed** factors — a level is served only once every
//! one of its chunks has been written and its factor recorded there.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use swath_core::crs::Crs;
use swath_core::raster::{DType, GeoTransform, RasterInfo, WindowRequest};

/// Domain-separation tag mixed into every pyramid root digest (the
/// `TileKey` discipline: version the scheme so a future v2 layout can
/// never collide with v1 objects in the same store).
pub const PYRAMID_KEY_DOMAIN: &str = "swath pyramid-key v1";

/// Prefix all pyramids live under, so they can share a bucket with source
/// data and the tile cache (and a future GC sweep knows what is its own).
pub const PREFIX: &str = "pyramids";

/// Chunk side length of every level array, in pixels. Matches the tile
/// cache's world: one chunk read serves one 256-px tile at matched zoom.
pub const CHUNK: u32 = 256;

/// Default coarsest-level bound: the ladder stops at the first level whose
/// larger axis fits `MIN_DIM` pixels (GDAL's own overview-build default),
/// so the coarsest level serves any lower zoom whole from one read.
pub const MIN_DIM: u32 = 256;

/// How level pixels are aggregated from the finer grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PyramidResampling {
    /// Nodata-aware mean of each block — continuous data.
    Average,
    /// Top-left sample of each block — categorical/QA data.
    Nearest,
}

/// The content-derived pyramid root for `asset_uri`:
/// `pyramids/<hh>/<hh>/<sha256 hex>` (module docs). Same inputs ⇒ same
/// root, on any machine, forever.
#[must_use]
pub fn pyramid_root(asset_uri: &str) -> String {
    let mut hasher = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    field(PYRAMID_KEY_DOMAIN.as_bytes());
    field(asset_uri.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use core::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing hex to a String is infallible");
    }
    format!("{PREFIX}/{}/{}/{hex}", &hex[..2], &hex[2..4])
}

/// The Zarr v2 dtype string for `dtype` (little-endian; `|u1` for bytes).
#[must_use]
pub fn zarr_dtype(dtype: DType) -> &'static str {
    match dtype {
        DType::UInt8 => "|u1",
        DType::Int16 => "<i2",
        DType::UInt16 => "<u2",
        DType::Int32 => "<i4",
        DType::Float32 => "<f4",
        DType::Float64 => "<f8",
        // DType is non_exhaustive; widens in lockstep with PixelBuffer.
        _ => unreachable!("dtype not produced by any swath source adapter"),
    }
}

/// The `DType` a Zarr v2 dtype string names, if this layout supports it.
#[must_use]
pub fn dtype_from_zarr(spec: &str) -> Option<DType> {
    match spec {
        "|u1" => Some(DType::UInt8),
        "<i2" => Some(DType::Int16),
        "<u2" => Some(DType::UInt16),
        "<i4" => Some(DType::Int32),
        "<f4" => Some(DType::Float32),
        "<f8" => Some(DType::Float64),
        _ => None,
    }
}

/// The decimation-factor ladder to materialize for a `width × height`
/// grid: powers of two, coarsest first level whose larger axis fits
/// `min_dim`, **excluding** factors the asset already embeds (those are
/// served from the asset itself — the pyramid only stores what is
/// missing). Ascending. Empty when the grid already fits `min_dim` or
/// every needed factor is embedded.
#[must_use]
pub fn ladder(width: u64, height: u64, embedded: &[u32], min_dim: u32) -> Vec<u32> {
    let mut factors = Vec::new();
    if width == 0 || height == 0 || width.max(height) <= u64::from(min_dim) {
        return factors;
    }
    let mut factor: u32 = 2;
    loop {
        let (w, h) = level_dims(width, height, factor);
        if !embedded.contains(&factor) {
            factors.push(factor);
        }
        if w.max(h) <= u64::from(min_dim) {
            break;
        }
        let Some(next) = factor.checked_mul(2) else {
            break;
        };
        factor = next;
    }
    factors
}

/// Level dimensions at decimation `factor`: `ceil(full / factor)` per axis
/// (the COG/GDAL overview convention).
#[must_use]
pub fn level_dims(width: u64, height: u64, factor: u32) -> (u64, u64) {
    let f = u64::from(factor.max(1));
    (width.div_ceil(f), height.div_ceil(f))
}

/// The grid a level stores, derived from the full-resolution `info`: real
/// ceil-divided dimensions, and the full-res geotransform with its pixel
/// scale multiplied by the **exact** per-axis ratio `full_dim / level_dim`
/// (identical to the COG adapter's embedded-overview convention, so
/// consumers see one grid contract regardless of where a level lives).
/// `overview_levels` is empty: a level grid is not itself overviewed.
#[must_use]
pub fn level_info(full: &RasterInfo, factor: u32) -> RasterInfo {
    let (width, height) = level_dims(full.width, full.height, factor);
    #[allow(
        clippy::cast_precision_loss,
        reason = "raster dims far below 2^52; the ratio is exact for them"
    )]
    let (rx, ry) = (
        full.width as f64 / width as f64,
        full.height as f64 / height as f64,
    );
    let mut transform = full.transform;
    transform.pixel_width *= rx;
    transform.pixel_height *= ry;
    RasterInfo {
        width,
        height,
        transform,
        overview_levels: vec![],
        ..full.clone()
    }
}

/// Maps a request in **full-resolution** pixel coordinates onto `grid`,
/// covering it: start offsets round down, end offsets round up, against
/// the exact per-axis ratio `full_dim / grid_dim` (the `ReadLevel`
/// rounding contract). Not yet clipped — the caller intersects with the
/// grid.
#[must_use]
pub fn to_grid(request: &WindowRequest, full: &RasterInfo, grid: &RasterInfo) -> WindowRequest {
    if grid.width == full.width && grid.height == full.height {
        return *request;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "raster and window dims far below 2^52"
    )]
    let (rx, ry) = (
        full.width as f64 / grid.width as f64,
        full.height as f64 / grid.height as f64,
    );
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "offsets clamped non-negative; dims far below 2^52"
    )]
    let scale = |v: u64, r: f64, round_up: bool| -> u64 {
        let scaled = v as f64 / r;
        (if round_up {
            scaled.ceil()
        } else {
            scaled.floor()
        })
        .max(0.0) as u64
    };
    let col_off = scale(request.col_off, rx, false);
    let row_off = scale(request.row_off, ry, false);
    let end_col = scale(request.end_col(), rx, true);
    let end_row = scale(request.end_row(), ry, true);
    WindowRequest {
        col_off,
        row_off,
        width: end_col.saturating_sub(col_off),
        height: end_row.saturating_sub(row_off),
    }
}

/// Version of the `swath:pyramid` identity document this adapter writes
/// and accepts.
pub const LAYOUT_VERSION: u32 = 1;

/// The group `.zattrs` document: the OME-style multiscale listing plus the
/// pyramid's identity (module docs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupAttrs {
    /// OME-style multiscale listing of the level arrays present.
    pub multiscales: Vec<Multiscale>,
    /// The identity/completion document.
    #[serde(rename = "swath:pyramid")]
    pub pyramid: PyramidMeta,
}

/// One multiscale entry: the level arrays of this group, finest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Multiscale {
    /// Multiscales convention version tag.
    pub version: String,
    /// The source asset URI this pyramid decimates.
    pub name: String,
    /// The level arrays, ascending by factor.
    pub datasets: Vec<MultiscaleLevel>,
    /// The aggregation used (`average` | `nearest`).
    #[serde(rename = "type")]
    pub resampling: PyramidResampling,
}

/// One level array within a multiscale entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiscaleLevel {
    /// The array's path within the group (the factor as a string).
    pub path: String,
    /// The decimation factor relative to the full-resolution grid.
    pub factor: u32,
}

/// The `swath:pyramid` identity document: everything needed to verify the
/// pyramid still describes its source and to serve its levels without
/// consulting the source asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyramidMeta {
    /// Layout version ([`LAYOUT_VERSION`]).
    pub layout_version: u32,
    /// The source asset URI (as the serving path names it).
    pub source: String,
    /// Full-resolution grid width the pyramid was built from.
    pub width: u64,
    /// Full-resolution grid height the pyramid was built from.
    pub height: u64,
    /// Sample dtype, in Zarr v2 spelling (e.g. `<i2`).
    pub dtype: String,
    /// Nodata sentinel of the source, if declared.
    pub nodata: Option<f64>,
    /// CRS of the pixel grid.
    pub crs: Crs,
    /// Full-resolution pixel↔CRS transform (level transforms derive from
    /// it by the exact ratio — [`level_info`]).
    pub transform: GeoTransform,
    /// Chunk side length of every level array.
    pub chunk: u32,
    /// The aggregation used for every level.
    pub resampling: PyramidResampling,
    /// Factors whose every chunk is written — the servable set.
    pub completed: Vec<u32>,
}

impl PyramidMeta {
    /// Whether this pyramid still describes `info` for `asset_uri` — the
    /// staleness guard consulted before any level is served or resumed.
    #[must_use]
    pub fn matches(&self, asset_uri: &str, info: &RasterInfo) -> bool {
        self.layout_version == LAYOUT_VERSION
            && self.source == asset_uri
            && self.width == info.width
            && self.height == info.height
            && self.dtype == zarr_dtype(info.dtype)
    }

    /// The full-resolution grid this document records, as a `RasterInfo`
    /// (band 1, no embedded overviews — level grids derive via
    /// [`level_info`]).
    #[must_use]
    pub fn full_info(&self) -> Option<RasterInfo> {
        Some(RasterInfo {
            crs: self.crs.clone(),
            width: self.width,
            height: self.height,
            transform: self.transform,
            band_count: 1,
            dtype: dtype_from_zarr(&self.dtype)?,
            nodata: self.nodata,
            overview_levels: vec![],
        })
    }
}

/// The Zarr v2 `.zarray` document for one level, exactly as written
/// (field set fixed by the Zarr v2 spec).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZarrayMeta {
    /// Always 2.
    pub zarr_format: u32,
    /// `[height, width]` — row-major.
    pub shape: [u64; 2],
    /// `[chunk, chunk]`.
    pub chunks: [u64; 2],
    /// Little-endian dtype spelling.
    pub dtype: String,
    /// Always `null`: chunks are raw bytes.
    pub compressor: Option<serde_json::Value>,
    /// The nodata sentinel (or 0 when none is declared) — what padding in
    /// edge chunks holds.
    pub fill_value: serde_json::Value,
    /// Always `"C"`.
    pub order: String,
    /// Always `null`.
    pub filters: Option<serde_json::Value>,
    /// Always `"."` — chunk keys are `<row>.<col>`.
    pub dimension_separator: String,
}

impl ZarrayMeta {
    /// The `.zarray` document for a level of `(width, height)` pixels of
    /// `dtype` with `nodata` padding.
    #[must_use]
    pub fn new(width: u64, height: u64, dtype: DType, nodata: Option<f64>) -> Self {
        Self {
            zarr_format: 2,
            shape: [height, width],
            chunks: [u64::from(CHUNK), u64::from(CHUNK)],
            dtype: zarr_dtype(dtype).to_owned(),
            compressor: None,
            fill_value: fill_value(dtype, nodata),
            order: "C".to_owned(),
            filters: None,
            dimension_separator: ".".to_owned(),
        }
    }
}

/// The JSON `fill_value` for `dtype` with `nodata`: the nodata sentinel
/// when declared (written as an integer for integer dtypes so external
/// Zarr readers see a spec-typical fill), else 0.
fn fill_value(dtype: DType, nodata: Option<f64>) -> serde_json::Value {
    let value = nodata.unwrap_or(0.0);
    match dtype {
        DType::Float32 | DType::Float64 => serde_json::json!(value),
        #[allow(
            clippy::cast_possible_truncation,
            reason = "integer nodata sentinels are small integers by convention"
        )]
        _ => serde_json::json!(value as i64),
    }
}

/// Object path of the group `.zgroup` under `root`.
#[must_use]
pub fn zgroup_path(root: &str) -> String {
    format!("{root}/.zgroup")
}

/// Object path of the group `.zattrs` under `root`.
#[must_use]
pub fn zattrs_path(root: &str) -> String {
    format!("{root}/.zattrs")
}

/// Object path of a level's `.zarray` under `root`.
#[must_use]
pub fn zarray_path(root: &str, factor: u32) -> String {
    format!("{root}/{factor}/.zarray")
}

/// Object path of one chunk of a level under `root` (`<row>.<col>` in
/// chunk-grid coordinates).
#[must_use]
pub fn chunk_path(root: &str, factor: u32, chunk_row: u64, chunk_col: u64) -> String {
    format!("{root}/{factor}/{chunk_row}.{chunk_col}")
}

#[cfg(test)]
mod tests {
    use swath_core::crs::Crs;
    use swath_core::raster::{DType, GeoTransform, RasterInfo, WindowRequest};

    use super::{
        CHUNK, GroupAttrs, LAYOUT_VERSION, Multiscale, MultiscaleLevel, PyramidMeta,
        PyramidResampling, ZarrayMeta, chunk_path, dtype_from_zarr, ladder, level_dims, level_info,
        pyramid_root, to_grid, zarr_dtype,
    };

    fn info(width: u64, height: u64) -> RasterInfo {
        RasterInfo {
            crs: Crs::from_epsg(32610),
            width,
            height,
            transform: GeoTransform::north_up(600_000.0, 4_500_000.0, 30.0, -30.0),
            band_count: 1,
            dtype: DType::Int16,
            nodata: Some(-9999.0),
            overview_levels: vec![],
        }
    }

    /// The root is pinned to a known answer: this exact path for this
    /// exact URI, forever (any change to the scheme must consciously
    /// rewrite this constant and bump `PYRAMID_KEY_DOMAIN`).
    #[test]
    fn root_is_pinned_to_a_known_answer() {
        let root = pyramid_root("granules/B04.tif");
        assert!(root.starts_with("pyramids/"), "got {root}");
        let hex = root.rsplit('/').next().unwrap();
        assert_eq!(hex.len(), 64);
        assert_eq!(
            root,
            format!("pyramids/{}/{}/{hex}", &hex[..2], &hex[2..4]),
            "sharded by the first two byte pairs"
        );
        // Determinism + sensitivity.
        assert_eq!(root, pyramid_root("granules/B04.tif"));
        assert_ne!(root, pyramid_root("granules/B05.tif"));
    }

    /// The HLS shape: a 3660² grid with embedded [2, 4, 8] materializes
    /// exactly the missing ×16 level; without embedded overviews the whole
    /// ladder is built; a grid already at or under `min_dim` needs nothing.
    #[test]
    fn ladder_builds_exactly_what_is_missing() {
        assert_eq!(ladder(3660, 3660, &[2, 4, 8], 256), vec![16]);
        assert_eq!(ladder(3660, 3660, &[], 256), vec![2, 4, 8, 16]);
        assert_eq!(ladder(512, 512, &[2], 256), Vec::<u32>::new());
        assert_eq!(ladder(512, 512, &[], 256), vec![2]);
        assert_eq!(ladder(256, 256, &[], 256), Vec::<u32>::new());
        assert_eq!(ladder(512, 512, &[], 64), vec![2, 4, 8]);
        // The larger axis governs.
        assert_eq!(ladder(3660, 100, &[], 256), vec![2, 4, 8, 16]);
        assert_eq!(ladder(0, 0, &[], 256), Vec::<u32>::new());
    }

    #[test]
    fn level_dims_ceil_divide() {
        assert_eq!(level_dims(3660, 3660, 16), (229, 229));
        assert_eq!(level_dims(512, 512, 2), (256, 256));
        assert_eq!(level_dims(513, 511, 2), (257, 256));
    }

    /// The level grid follows the COG embedded-overview convention: ceil
    /// dims, transform scaled by the exact per-axis ratio.
    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the origin must be bit-identical (shared, never scaled)"
    )]
    fn level_info_scales_the_transform_exactly() {
        let full = info(3660, 3660);
        let level = level_info(&full, 16);
        assert_eq!((level.width, level.height), (229, 229));
        let ratio = 3660.0 / 229.0;
        assert!((level.transform.pixel_width - 30.0 * ratio).abs() < 1e-9);
        assert!((level.transform.pixel_height + 30.0 * ratio).abs() < 1e-9);
        assert_eq!(level.transform.origin_x, full.transform.origin_x);
        assert!(level.overview_levels.is_empty());
    }

    /// Covering semantics: start floors, end ceils, identity on the
    /// full-res grid.
    #[test]
    fn to_grid_covers() {
        let full = info(3660, 3660);
        let level = level_info(&full, 16);
        let request = WindowRequest {
            col_off: 100,
            row_off: 3659,
            width: 50,
            height: 1,
        };
        let mapped = to_grid(&request, &full, &level);
        // 100 / (3660/229) = 6.25 → 6; 150 / ratio = 9.38 → 10.
        assert_eq!(mapped.col_off, 6);
        assert_eq!(mapped.end_col(), 10);
        // Row 3659..3660 → 228..229 (the last level row).
        assert_eq!(mapped.row_off, 228);
        assert_eq!(mapped.end_row(), 229);
        assert_eq!(to_grid(&request, &full, &full), request);
    }

    /// The `.zarray` document is byte-pinned: this is the persistent
    /// format external Zarr readers parse.
    #[test]
    fn zarray_document_is_pinned() {
        let meta = ZarrayMeta::new(229, 229, DType::Int16, Some(-9999.0));
        assert_eq!(
            serde_json::to_value(&meta).unwrap(),
            serde_json::json!({
                "zarr_format": 2,
                "shape": [229, 229],
                "chunks": [256, 256],
                "dtype": "<i2",
                "compressor": null,
                "fill_value": -9999,
                "order": "C",
                "filters": null,
                "dimension_separator": ".",
            })
        );
        let float = ZarrayMeta::new(4, 4, DType::Float32, None);
        assert_eq!(float.fill_value, serde_json::json!(0.0));
    }

    #[test]
    fn dtype_spellings_round_trip() {
        for dtype in [
            DType::UInt8,
            DType::Int16,
            DType::UInt16,
            DType::Int32,
            DType::Float32,
            DType::Float64,
        ] {
            assert_eq!(dtype_from_zarr(zarr_dtype(dtype)), Some(dtype));
        }
        assert_eq!(dtype_from_zarr(">i2"), None, "big-endian is not written");
    }

    /// The group attrs round-trip through JSON, and the staleness guard
    /// verifies source identity.
    #[test]
    fn group_attrs_round_trip_and_guard() {
        let full = info(3660, 3660);
        let meta = PyramidMeta {
            layout_version: LAYOUT_VERSION,
            source: "granules/B04.tif".to_owned(),
            width: 3660,
            height: 3660,
            dtype: "<i2".to_owned(),
            nodata: Some(-9999.0),
            crs: full.crs.clone(),
            transform: full.transform,
            chunk: CHUNK,
            resampling: PyramidResampling::Average,
            completed: vec![16],
        };
        let attrs = GroupAttrs {
            multiscales: vec![Multiscale {
                version: "0.1".to_owned(),
                name: "granules/B04.tif".to_owned(),
                datasets: vec![MultiscaleLevel {
                    path: "16".to_owned(),
                    factor: 16,
                }],
                resampling: PyramidResampling::Average,
            }],
            pyramid: meta.clone(),
        };
        let json = serde_json::to_string(&attrs).unwrap();
        assert!(json.contains(r#""swath:pyramid""#));
        assert!(json.contains(r#""type":"average""#));
        let back: GroupAttrs = serde_json::from_str(&json).unwrap();
        assert_eq!(back, attrs);

        assert!(meta.matches("granules/B04.tif", &full));
        assert!(!meta.matches("granules/B05.tif", &full), "other asset");
        assert!(!meta.matches("granules/B04.tif", &info(3661, 3660)), "grid");
        let stored = meta.full_info().expect("dtype known");
        assert_eq!(stored.width, 3660);
        assert_eq!(stored.dtype, DType::Int16);
        assert_eq!(stored.nodata, Some(-9999.0));
    }

    #[test]
    fn chunk_paths_are_row_dot_col() {
        assert_eq!(
            chunk_path("pyramids/ab/cd/xyz", 16, 0, 3),
            "pyramids/ab/cd/xyz/16/0.3"
        );
    }
}
