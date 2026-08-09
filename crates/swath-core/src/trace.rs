// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The Trace model — the x-ray keystone (ARCHITECTURE.md §9, REQUIREMENTS.md
//! R4: glass-box).
//!
//! Every `render_tile` will return a [`Trace`] alongside the encoded tile and
//! stream it over SSE; the test suite asserts against the *same* struct. The
//! JSON shape is therefore a contract shared by the UI and the tests:
//!
//! - **Field naming is `snake_case`** — Rust field names serialize verbatim
//!   (no rename attributes), so the Rust struct definition *is* the schema.
//! - **Enum representation is externally tagged with `snake_case` variant
//!   names**: `"decision": "live"`, `"decision": "cache_hit"`,
//!   `"decision": {"overview": {"level": 2}}`.
//! - The round-trip test in this module pins the exact serialized form;
//!   changing it is a deliberate, reviewed act.

use crate::crs::Crs;
use crate::raster::AssetRef;

/// The materialization strategy the planner chose for a tile
/// (ARCHITECTURE.md §5/§10): serve from cache, from a pre-computed overview,
/// or render live from full-resolution source reads.
///
/// The ARCHITECTURE.md sketch carries a `TileKey` inside `CacheHit`; the key
/// type is defined by the cache model (hash of `layer_version` + `render_spec` +
/// `tile_coord` + tms, §10) and lands with the `TileCache` port — `CacheHit`
/// grows its key then.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// The encoded tile was served straight from the tile cache.
    CacheHit,
    /// Pixels came from a pre-computed overview pyramid level.
    Overview {
        /// Overview level read. The numbering convention (which level is
        /// coarsest) is source-defined; the trace records the level as the
        /// adapter reported it.
        level: u8,
    },
    /// Pixels came from full-resolution source reads.
    Live,
}

/// One contiguous byte range read from a source, as the storage layer saw it.
///
/// Deliberately the same shape as prototype 0001's `ChunkRef`: a COG tile
/// read, a Zarr chunk fetch, and an HTTP range request all reduce to
/// *path + offset + length*, so one type covers every source kind the
/// `RasterSource` adapters will report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Provenance {
    /// Path or key of the object the bytes came from.
    pub path: String,
    /// Byte offset of the range within that object.
    pub offset: u64,
    /// Length of the range in bytes.
    pub length: u64,
}

/// Wall-clock milliseconds spent in each stage of a render.
///
/// `total_ms` is recorded, not derived: stages can overlap under concurrency,
/// so the parts need not sum to the whole.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Timings {
    /// Reading source bytes (I/O wait included).
    pub read_ms: u64,
    /// Reprojection / warping.
    pub warp_ms: u64,
    /// Band math, compositing, colormapping (the render IR).
    pub pixel_ops_ms: u64,
    /// Encoding the output tile (PNG/WebP).
    pub encode_ms: u64,
    /// End-to-end render duration.
    pub total_ms: u64,
}

/// The structured explanation of one rendered tile — what happened, from
/// where, and how long it took.
///
/// Differences from the ARCHITECTURE.md §9 sketch (which is explicitly
/// illustrative), all documented here so the sketch can be reconciled:
///
/// - `chunks_or_ranges` is named [`provenance`](Self::provenance) — the field
///   holds [`Provenance`] records and the shorter name is the one tests and
///   the UI will speak.
/// - `timings_ms` is named [`timings`](Self::timings) — the `_ms` unit suffix
///   lives on each [`Timings`] field, where it is adjacent to the value.
/// - `cache_key` is absent for now: `TileKey` is defined by the cache model
///   (§10) and lands with the `TileCache` port; it will be added here then.
/// - `sources` is an addition the sketch's single `source` could not
///   express: a composite render (true color: three reflectance bands from
///   three COGs) reads *several* assets, and R4 demands every one be
///   accounted for. Issue #21 deliberately deferred the multi-asset
///   question; `render_tile` (#26) answered it by keeping `source` as the
///   primary asset (the first declared band input's) for one-line summaries
///   and adding `sources` as the full list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    /// The planner's materialization choice for this tile.
    pub decision: Strategy,
    /// The primary source asset — the first declared band input's asset.
    /// The one-line answer to "where did this tile come from?"; the full
    /// accounting is [`sources`](Self::sources).
    pub source: AssetRef,
    /// Every distinct source asset read (or consulted) for this tile, in
    /// declared band-input order. Always contains
    /// [`source`](Self::source) first; single-asset renders have exactly
    /// one entry.
    pub sources: Vec<AssetRef>,
    /// CRS of the source pixels.
    pub crs_from: Crs,
    /// CRS of the rendered tile.
    pub crs_to: Crs,
    /// Total source bytes read for this render.
    pub bytes_read: u64,
    /// Every byte range / chunk touched, in read order.
    pub provenance: Vec<Provenance>,
    /// Per-stage wall-clock timings.
    pub timings: Timings,
    /// Ingest-to-pixel latency, present when this tile is the first render
    /// reflecting a just-ingested granule (the north-star timer).
    pub ingest_to_pixel_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{Provenance, Strategy, Timings, Trace};
    use crate::crs::Crs;
    use crate::raster::AssetRef;

    fn sample() -> Trace {
        Trace {
            decision: Strategy::Overview { level: 2 },
            source: AssetRef::new("s3://hls/granule/B04.tif"),
            sources: vec![
                AssetRef::new("s3://hls/granule/B04.tif"),
                AssetRef::new("s3://hls/granule/B03.tif"),
            ],
            crs_from: Crs::from_epsg(32613),
            crs_to: Crs::WEB_MERCATOR,
            bytes_read: 131_072,
            provenance: vec![Provenance {
                path: "granule/B04.tif".to_owned(),
                offset: 4096,
                length: 131_072,
            }],
            timings: Timings {
                read_ms: 12,
                warp_ms: 3,
                pixel_ops_ms: 1,
                encode_ms: 2,
                total_ms: 18,
            },
            ingest_to_pixel_ms: Some(950),
        }
    }

    /// The serialized field names and enum tags are a contract (SSE stream +
    /// tests); this pins the exact JSON so drift is a visible diff.
    #[test]
    fn json_shape_is_pinned() {
        let json = serde_json::to_value(sample()).unwrap();
        let expected = serde_json::json!({
            "decision": {"overview": {"level": 2}},
            "source": "s3://hls/granule/B04.tif",
            "sources": ["s3://hls/granule/B04.tif", "s3://hls/granule/B03.tif"],
            "crs_from": 32613,
            "crs_to": 3857,
            "bytes_read": 131_072,
            "provenance": [{"path": "granule/B04.tif", "offset": 4096, "length": 131_072}],
            "timings": {"read_ms": 12, "warp_ms": 3, "pixel_ops_ms": 1, "encode_ms": 2, "total_ms": 18},
            "ingest_to_pixel_ms": 950,
        });
        assert_eq!(json, expected);
    }

    #[test]
    fn unit_strategies_serialize_as_strings() {
        assert_eq!(
            serde_json::to_value(Strategy::CacheHit).unwrap(),
            "cache_hit"
        );
        assert_eq!(serde_json::to_value(Strategy::Live).unwrap(), "live");
    }

    #[test]
    fn round_trips_through_json() {
        let trace = sample();
        let json = serde_json::to_string(&trace).unwrap();
        let back: Trace = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trace);
    }
}
