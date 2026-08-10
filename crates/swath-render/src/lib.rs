// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The tiler engine's pixel stages (ARCHITECTURE.md §5, ADR 0002: the
//! tiler brain is **BUILD**): warp/resample kernels, the Render IR pixel
//! ops that consume their output, and the tile encoder.
//!
//! Given a target tile grid ([`TargetGrid`]), a coordinate transform from the
//! target CRS to the source CRS (the [`CoordTransform`] handed out by the
//! `Reproject` port), and a source pixel window (the `WindowData` a
//! `RasterSource` adapter read), this crate:
//!
//! 1. computes the minimal source window covering a tile
//!    ([`source_window`]) — the read request the tiler sends the source;
//! 2. warps the window into the tile grid by **inverse mapping**
//!    ([`warp`]): every target pixel center is projected back into source
//!    pixel space and sampled with a nodata-aware kernel
//!    ([`Resampling::Nearest`] for categorical data,
//!    [`Resampling::Bilinear`] for continuous data).
//!
//! Where warp lives (in the core, calling the minimal `Reproject` port) is
//! ARCHITECTURE.md's current proposal; §16.2 remains an open question and
//! this crate does not close it — the kernels only require a
//! `&dyn CoordTransform`, so a richer `Warp` port could later wrap them
//! without rework.
//!
//! Correctness is defined by the GDAL/rio-tiler oracle (ADR 0002): the
//! golden tests in `tests/golden.rs` render real HLS fixture tiles through
//! these kernels and perceptually diff them against committed
//! oracle-rendered tiles under the default `swath-testkit` policy. The
//! nodata semantics of [`Resampling::Bilinear`] deliberately mirror GDAL's
//! warper (see [`NodataPolicy`]).
//!
//! Downstream of the warp, the **Render IR** (the [`ir`] module) turns
//! warped `f64` planes into an 8-bit RGBA tile — band math, rescale,
//! composite, colormap — and the [`encode`] module serializes it (PNG in
//! Phase 1). The IR is the typed target the **process compiler** (the
//! [`process`] module) lowers openEO graphs into; [`eval`] executes it.
//!
//! [`render_tile`] stitches all of the stages into one motion — describe,
//! window, read, warp, eval, encode — and returns the encoded tile
//! together with the fully populated [`Trace`](swath_core::trace::Trace)
//! that explains it (REQUIREMENTS.md R4).
//!
//! [`CoordTransform`]: swath_core::reproject::CoordTransform

pub mod colormaps;
mod encode;
mod error;
mod grid;
pub mod ir;
pub mod plan;
pub mod process;
mod tiler;
mod warp;
mod window;

pub use encode::{EncodeError, encode_png};
pub use error::RenderError;
pub use grid::TargetGrid;
pub use ir::{RenderPlan, RgbaTile, eval};
pub use plan::{PlanMetadata, PlanSpec, ndvi_expr, plan_for};
pub use process::{CompileContext, CompileError, CompiledProduct, compile};
pub use tiler::{EncodedTile, TileError, TileRequest, render_tile, render_tile_cached};
pub use warp::{NodataPolicy, Resampling, WarpedBuffer, warp};
pub use window::{BOUNDARY_SAMPLES_PER_EDGE, source_window};
