// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! What the kernel knows about the source: grid, window, samples.

use crate::geo::GeoTransform;

/// The source raster grid a window was read from: its pixel dimensions and
/// its pixel↔CRS mapping. For an overview read, this is the **overview
/// grid** (its dimensions and scaled geotransform) — every coordinate the
/// kernel computes is in this grid's pixel space, so overview warps work
/// by construction, exactly as GDAL warps from an overview dataset's grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceGrid {
    /// Grid width in pixels.
    pub width: u64,
    /// Grid height in pixels.
    pub height: u64,
    /// Pixel↔CRS mapping of this grid.
    pub transform: GeoTransform,
}

/// A rectangular pixel window into a source grid: `width × height` pixels
/// starting at `(col_off, row_off)` from the top-left. A zero-area window
/// is valid and means "nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelWindow {
    /// Leftmost column of the window.
    pub col_off: u64,
    /// Topmost row of the window.
    pub row_off: u64,
    /// Width in pixels (columns).
    pub width: u64,
    /// Height in pixels (rows).
    pub height: u64,
}

/// One read source window, ready to warp: the grid it came from, its
/// placement within that grid, its samples, and the nodata sentinel.
///
/// # Samples are `f64` (exact by contract)
///
/// Samples are the source pixels widened to `f64`, row-major over
/// `window`. Every integer dtype up to 32 bits and `f32` is exactly
/// representable in `f64`, so the widening loses nothing; callers with
/// native `u8`/`i16`/… buffers convert with `f64::from` (or an equivalent
/// exact cast) before warping. The nodata sentinel is compared **exactly**
/// (GDAL semantics — a sentinel, never a range), so it must be widened the
/// same way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceBuffer<'a> {
    /// The grid the window was read from (full-resolution or overview).
    pub grid: SourceGrid,
    /// Where the buffer sits within [`grid`](Self::grid).
    pub window: PixelWindow,
    /// Sample values, row-major over [`window`](Self::window), widened to
    /// `f64`.
    pub samples: &'a [f64],
    /// Nodata sentinel, widened to `f64` (GDAL convention), if declared.
    /// NaN is a valid sentinel and matches NaN samples.
    pub nodata: Option<f64>,
}
