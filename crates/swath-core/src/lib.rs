// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Swath domain core.
//!
//! The pure-logic center of Swath (ADR 0001, ADR 0002): domain types, port
//! traits, the materialization planner, the process-graph compiler + Render IR,
//! and the [`Trace`](trace::Trace) model. This crate performs **no I/O** — no
//! filesystem, no network, no clocks, nothing async. Everything external
//! enters through port traits implemented by adapter crates; the port traits
//! themselves land alongside their first adapters, so today this crate is the
//! shared *vocabulary* those traits will speak
//! ([ARCHITECTURE.md §5–6, §9](https://github.com/forgo/swath/blob/main/docs/ARCHITECTURE.md)).
//!
//! - [`tile`] — quadtree tile addressing + `WebMercatorQuad` TMS math
//! - [`crs`] — CRS identity (EPSG codes; projection math is an adapter concern)
//! - [`raster`] — raster metadata, pixel windows, asset references
//! - [`trace`] — the per-render x-ray record (REQUIREMENTS.md R4)
//! - [`error`] — the crate's small invariant-violation taxonomy

pub mod crs;
pub mod error;
pub mod raster;
pub mod tile;
pub mod trace;
