// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Swath domain core.
//!
//! The pure-logic center of Swath (ADR 0001, ADR 0002): domain types, port
//! traits, the materialization planner, and the [`Trace`](trace::Trace) model.
//! (The process-graph compiler and the Render IR live in `swath-render`, the
//! rendering engine crate — not here.) This crate performs **no I/O** — no
//! filesystem, no network, no clocks, no async runtime. Everything external
//! enters through port traits implemented by adapter crates. Port traits may
//! be `async` (they describe I/O an adapter will perform — see
//! [`source::RasterSource`]) but the core itself only *defines* the futures'
//! signatures; it depends on no executor
//! ([ARCHITECTURE.md §5–6, §9](https://github.com/forgo/swath/blob/main/docs/ARCHITECTURE.md)).
//!
//! - [`cache`] — the `TileCache` port and the content-derived `TileKey`
//!   (ARCHITECTURE.md §10)
//! - [`catalog`] — Dataset/Granule/Layer domain, the `Catalog` port, and the
//!   lossless STAC converters (REQUIREMENTS.md R2/R5)
//! - [`events`] — the `EventSource` port: granule-arrival announcements
//! - [`ingest`] — the ingest orchestrator's registration step (REQUIREMENTS.md R1)
//!   and the `IngestReferencer` port (ADR 0006)
//! - [`manifest`] — virtual-reference manifest schema v1, the port contract
//!   (ADR 0006), plus the generator-equivalence check — re-exported from the
//!   extracted `swath-manifest` crate (ADR 0016)
//! - [`planner`] — the cost-aware materialization planner (issue #37):
//!   `plan()` chooses `CacheHit | Overview | Live` under a per-layer `Budget`
//!   and records every candidate's estimate for the Trace
//! - [`tile`] — quadtree tile addressing + `WebMercatorQuad` TMS math
//! - [`crs`] — CRS identity (EPSG codes; projection math is an adapter concern)
//! - [`raster`] — raster metadata, pixel windows, asset references
//! - [`source`] — the `RasterSource` port: async windowed reads + provenance
//! - [`reproject`] — the `Reproject` port: sync CRS-to-CRS point transforms
//! - [`trace`] — the per-render x-ray record (REQUIREMENTS.md R4)
//! - [`udf`] — the `ModuleStore` / `ModuleFetcher` ports and the content
//!   hash naming a `run_udf` module (ADR 0018)
//! - [`error`] — the crate's small invariant-violation taxonomy

pub mod cache;
pub mod catalog;
pub mod crs;
pub mod error;
pub mod events;
pub mod ingest;
/// Manifest v1 — the schema crate re-exported under its pre-extraction
/// path (ADR 0016): `swath_core::manifest::*` keeps resolving unchanged.
pub use swath_manifest as manifest;
pub mod planner;
pub mod raster;
pub mod reproject;
pub mod source;
pub mod tile;
pub mod trace;
pub mod udf;
