// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
//!
//! # Warping one buffer
//!
//! Inverse mapping: every target pixel center is projected through the
//! transform into fractional source pixel coordinates and sampled there.
//! With an identity transform and a 1:1 grid, both kernels reproduce the
//! source exactly:
//!
//! ```
//! use swath_warp::{
//!     CoordTransform, GeoTransform, GridBounds, NodataPolicy, PixelWindow, Resampling,
//!     SourceBuffer, SourceGrid, TargetGrid, TransformError, warp,
//! };
//!
//! /// Identity transform: target CRS == source CRS.
//! struct Identity;
//!
//! impl CoordTransform for Identity {
//!     fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), TransformError> {
//!         Ok((x, y))
//!     }
//! }
//!
//! // A 4×4 source raster: 1 m pixels, origin (0, 0), y growing downward.
//! let samples: Vec<f64> = (0..16).map(f64::from).collect();
//! let source = SourceBuffer {
//!     grid: SourceGrid {
//!         width: 4,
//!         height: 4,
//!         transform: GeoTransform::north_up(0.0, 0.0, 1.0, -1.0),
//!     },
//!     window: PixelWindow { col_off: 0, row_off: 0, width: 4, height: 4 },
//!     samples: &samples,
//!     nodata: None,
//! };
//! // A 4×4 target grid aligned 1:1 with the source pixels.
//! let target = TargetGrid::new(
//!     GridBounds { min_x: 0.0, min_y: -4.0, max_x: 4.0, max_y: 0.0 },
//!     4,
//!     4,
//! );
//!
//! let out = warp(
//!     &source,
//!     &Identity,
//!     &target,
//!     Resampling::Bilinear(NodataPolicy::default()),
//! )?;
//! assert_eq!(out.valid_count(), 16);
//! assert_eq!(out.values, samples);
//! # Ok::<(), swath_warp::WarpError>(())
//! ```
//!
//! # Computing the read window first
//!
//! Real warps read a source window before resampling: [`source_window`]
//! computes the minimal window covering a target grid (plus a resampling
//! margin), exactly as GDAL's `ComputeSourceWindow` does — densified
//! boundary trace, out-of-domain points excluded, clipped to the raster.
//!

mod error;
mod geo;
mod grid;
mod source;
mod transform;
mod warp;
mod window;

pub use error::WarpError;
pub use geo::GeoTransform;
pub use grid::{GridBounds, TargetGrid};
pub use source::{PixelWindow, SourceBuffer, SourceGrid};
pub use transform::{CoordTransform, TransformError};
pub use warp::{NodataPolicy, Resampling, WarpedBuffer, warp};
pub use window::{
    BOUNDARY_SAMPLES_PER_EDGE, SourceExtent, clip_to_raster, source_extent, source_window,
};
