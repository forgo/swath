// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Warp and resample kernels — the first stage of the tiler engine that
//! produces output pixels (ARCHITECTURE.md §5, ADR 0002: the tiler brain is
//! **BUILD**).
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
//! [`CoordTransform`]: swath_core::reproject::CoordTransform

mod error;
mod grid;
mod warp;
mod window;

pub use error::RenderError;
pub use grid::TargetGrid;
pub use warp::{NodataPolicy, Resampling, WarpedBuffer, warp};
pub use window::{BOUNDARY_SAMPLES_PER_EDGE, source_window};
