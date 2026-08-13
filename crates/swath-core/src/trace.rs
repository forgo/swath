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
use crate::planner::{CandidateTrace, PlannedStrategy};
use crate::raster::AssetRef;

/// The materialization strategy the planner chose for a tile
/// (ARCHITECTURE.md §5/§10): serve from cache, from a pre-computed overview,
/// or render live from full-resolution source reads.
///
/// `CacheHit` carries the [`TileKey`](crate::cache::TileKey) in string
/// form, exactly as #21 planned ("`CacheHit` grows its key then" — "then"
/// is #36, the `TileCache` port landing). This is the one deliberate
/// change to the pinned JSON contract in #36: the wire form moves from the
/// bare string `"cache_hit"` to `{"cache_hit": {"key": "…"}}` (externally
/// tagged, like `overview`). The x-ray overlay's decision handling was
/// updated in lockstep to read both shapes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// The encoded tile was served straight from the tile cache.
    CacheHit {
        /// The content-derived cache key the tile was found under
        /// (lowercase hex — `TileKey::as_str`).
        key: String,
    },
    /// Pixels came from a pre-computed overview pyramid level.
    Overview {
        /// Overview level read, recorded as the **decimation factor** of
        /// the overview grid relative to full resolution (2 = half
        /// resolution, 4 = quarter, …) — exactly the naming
        /// `RasterInfo::overview_levels` and `ReadLevel::Overview` use,
        /// so the x-ray shows the same number the port speaks. (Widened
        /// from `u8` to `u32` when overview reads landed in #38; the JSON
        /// wire shape — a plain number — is unchanged.)
        level: u32,
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

/// The planner's reasoning for one tile (issue #37,
/// `docs/design/materialization-planner.md`): the chosen strategy and
/// **every** candidate weighed, each with its cost estimate,
/// admissibility, and reason — the x-ray answer to "why did it decide
/// that?" (CHARTER.md §6). [`Trace::decision`] stays the *executed*
/// strategy; this field explains the choice against its alternatives.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanTrace {
    /// The strategy the planner chose (candidate shape — the executed
    /// form, key included for cache hits, is [`Trace::decision`]).
    pub chosen: PlannedStrategy,
    /// Every candidate, in the fixed evaluation order `cache_hit`,
    /// `overview`, `live`.
    pub considered: Vec<CandidateTrace>,
}

/// The rule that selected the granule backing a time-parameterized frame
/// (ADR 0015): how the request's `datetime` (or its absence) was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TemporalRule {
    /// No `datetime` was requested: the fully open interval, resolving to
    /// the latest granule — the pre-ADR-0015 behavior, unchanged.
    Latest,
    /// An instant `t`: the latest granule with acquisition datetime ≤ `t`
    /// (the granule that was current at `t`).
    LatestAtOrBefore,
    /// An interval: the latest granule whose datetime falls within the
    /// inclusive (optionally open-ended) interval.
    LatestInInterval,
}

/// The temporal decision behind one rendered frame (ADR 0015): which
/// granule the tile's `datetime` parameter resolved to, and by what rule
/// — the x-ray answer to "which acquisition am I looking at?". Present on
/// catalog-backed renders only; static/fixture layers have no time
/// dimension and no granule, so their traces carry nothing here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TemporalTrace {
    /// The granule the frame resolved to.
    pub granule_id: String,
    /// That granule's acquisition datetime (RFC 3339 UTC).
    pub granule_datetime: String,
    /// The request's raw `datetime` parameter, verbatim; `None` when the
    /// request carried none (absent = latest).
    pub requested: Option<String>,
    /// The resolution rule that applied.
    pub rule: TemporalRule,
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
    /// Total **source** bytes read for this render. Deliberately `0` on a
    /// cache hit (#36, documented decision): no source ranges were
    /// touched, so `bytes_read` stays honest to its definition and
    /// [`provenance`](Self::provenance) stays empty — the cached payload's
    /// size is already on the wire as `Content-Length`, and a hit is
    /// unmistakable from [`decision`](Self::decision) alone, so no
    /// cached-bytes field is added to the contract.
    pub bytes_read: u64,
    /// Every byte range / chunk touched, in read order.
    pub provenance: Vec<Provenance>,
    /// Per-stage wall-clock timings.
    pub timings: Timings,
    /// Ingest-to-pixel latency, present when this tile is the first render
    /// reflecting a just-ingested granule (the north-star timer).
    pub ingest_to_pixel_ms: Option<u64>,
    /// The planner's reasoning (#37): chosen strategy + every candidate
    /// with estimates. Present on every planned render; `None` only for
    /// traces predating the planner (or synthetic ones).
    pub plan: Option<PlanTrace>,
    /// The temporal decision (ADR 0015): the granule this frame resolved
    /// to and the rule that chose it. Present on catalog-backed renders;
    /// `None` (and omitted from the JSON — the deliberate, additive
    /// contract change of #180) for static layers and older traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temporal: Option<TemporalTrace>,
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{PlanTrace, Provenance, Strategy, Timings, Trace};
    use crate::crs::Crs;
    use crate::planner::{CandidateTrace, PlannedStrategy};
    use crate::raster::AssetRef;

    fn sample_plan() -> PlanTrace {
        PlanTrace {
            chosen: PlannedStrategy::Overview { factor: 2 },
            considered: vec![
                CandidateTrace {
                    strategy: PlannedStrategy::CacheHit,
                    estimated_cost_bytes: 0,
                    admissible: false,
                    reason: Cow::Borrowed("cache miss"),
                },
                CandidateTrace {
                    strategy: PlannedStrategy::Overview { factor: 2 },
                    estimated_cost_bytes: 128_018,
                    admissible: true,
                    reason: Cow::Borrowed("coarsest overview within the oversample threshold"),
                },
                CandidateTrace {
                    strategy: PlannedStrategy::Live,
                    estimated_cost_bytes: 510_050,
                    admissible: true,
                    reason: Cow::Borrowed("full-resolution read"),
                },
            ],
        }
    }

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
            plan: Some(sample_plan()),
            temporal: None,
        }
    }

    /// The serialized field names and enum tags are a contract (SSE stream +
    /// tests); this pins the exact JSON so drift is a visible diff.
    ///
    /// The one deliberate change in #37: the Trace gains `plan` — the
    /// planner's chosen strategy plus every weighed candidate (estimate,
    /// admissibility, reason). This is the x-ray "why did it decide
    /// that?" payload the charter promises (§6); candidates reuse the
    /// `Strategy` tag vocabulary (`cache_hit`/`overview`/`live`, factor
    /// instead of key) so the overlay parses one vocabulary. `plan` is
    /// `null` only on traces that never went through the planner.
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
            "plan": {
                "chosen": {"overview": {"factor": 2}},
                "considered": [
                    {
                        "strategy": "cache_hit",
                        "estimated_cost_bytes": 0,
                        "admissible": false,
                        "reason": "cache miss",
                    },
                    {
                        "strategy": {"overview": {"factor": 2}},
                        "estimated_cost_bytes": 128_018,
                        "admissible": true,
                        "reason": "coarsest overview within the oversample threshold",
                    },
                    {
                        "strategy": "live",
                        "estimated_cost_bytes": 510_050,
                        "admissible": true,
                        "reason": "full-resolution read",
                    },
                ],
            },
        });
        assert_eq!(json, expected);
    }

    /// `live` stays a bare string; `cache_hit` carries its key (externally
    /// tagged, like `overview`) since #36 — the deliberate, documented
    /// contract change that landed with the `TileCache` port (the enum
    /// docs carry the justification; the x-ray overlay reads both shapes).
    #[test]
    fn strategy_wire_shapes_are_pinned() {
        assert_eq!(serde_json::to_value(Strategy::Live).unwrap(), "live");
        assert_eq!(
            serde_json::to_value(Strategy::CacheHit {
                key: "0123abcd".to_owned()
            })
            .unwrap(),
            serde_json::json!({"cache_hit": {"key": "0123abcd"}}),
        );
    }

    /// The temporal decision's wire shape (ADR 0015, #180): pinned like
    /// the rest of the contract. `temporal` is omitted entirely when
    /// `None` — pre-#180 traces (and static-layer renders) keep their
    /// exact bytes, which is what makes the field additive.
    #[test]
    fn temporal_wire_shape_is_pinned_and_absent_when_none() {
        let json = serde_json::to_value(sample()).unwrap();
        assert!(
            json.as_object().unwrap().get("temporal").is_none(),
            "a None temporal must be omitted, not null"
        );

        let mut trace = sample();
        trace.temporal = Some(super::TemporalTrace {
            granule_id: "hlss30-t10tfk-2024204".to_owned(),
            granule_datetime: "2024-07-22T19:03:00Z".to_owned(),
            requested: Some("2024-08-01T00:00:00Z".to_owned()),
            rule: super::TemporalRule::LatestAtOrBefore,
        });
        let json = serde_json::to_value(&trace).unwrap();
        assert_eq!(
            json["temporal"],
            serde_json::json!({
                "granule_id": "hlss30-t10tfk-2024204",
                "granule_datetime": "2024-07-22T19:03:00Z",
                "requested": "2024-08-01T00:00:00Z",
                "rule": "latest_at_or_before",
            }),
        );
        let back: Trace = serde_json::from_value(json).unwrap();
        assert_eq!(back, trace);

        // Older serialized traces (no `temporal` key) still deserialize.
        let mut old = serde_json::to_value(sample()).unwrap();
        old.as_object_mut().unwrap().remove("temporal");
        let back: Trace = serde_json::from_value(old).unwrap();
        assert_eq!(back.temporal, None);
    }

    #[test]
    fn round_trips_through_json() {
        let trace = sample();
        let json = serde_json::to_string(&trace).unwrap();
        let back: Trace = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trace);
    }
}
