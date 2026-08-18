// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The thin adapter between this workspace's vocabulary and `swath-warp`'s
//! self-contained input types (ADR 0016's standalone rule: the published
//! kernel never depends on `swath-core`; the workspace adapts here
//! instead). Pure type plumbing — every conversion is field-for-field, so
//! the kernel's behavior is untouched.

use swath_core::raster::{GeoTransform, RasterInfo, WindowRequest};
use swath_core::reproject::{CoordTransform, ReprojectError};

use crate::error::RenderError;
use crate::grid::TargetGrid;

/// Core [`GeoTransform`] → the kernel's own six-parameter affine.
pub(crate) fn geo_transform(gt: &GeoTransform) -> swath_warp::GeoTransform {
    swath_warp::GeoTransform {
        origin_x: gt.origin_x,
        pixel_width: gt.pixel_width,
        row_rotation: gt.row_rotation,
        origin_y: gt.origin_y,
        col_rotation: gt.col_rotation,
        pixel_height: gt.pixel_height,
    }
}

/// The slice of [`RasterInfo`] the kernel needs: dimensions + transform.
pub(crate) fn source_grid(info: &RasterInfo) -> swath_warp::SourceGrid {
    swath_warp::SourceGrid {
        width: info.width,
        height: info.height,
        transform: geo_transform(&info.transform),
    }
}

/// Render [`TargetGrid`] (Web Mercator by construction) → the kernel's
/// CRS-agnostic target grid.
pub(crate) fn target_grid(grid: &TargetGrid) -> swath_warp::TargetGrid {
    let b = grid.bounds();
    swath_warp::TargetGrid::new(
        swath_warp::GridBounds {
            min_x: b.min_x,
            min_y: b.min_y,
            max_x: b.max_x,
            max_y: b.max_y,
        },
        grid.width(),
        grid.height(),
    )
}

/// Core [`WindowRequest`] → the kernel's pixel window.
pub(crate) fn pixel_window(w: WindowRequest) -> swath_warp::PixelWindow {
    swath_warp::PixelWindow {
        col_off: w.col_off,
        row_off: w.row_off,
        width: w.width,
        height: w.height,
    }
}

/// The kernel's pixel window → core [`WindowRequest`].
pub(crate) fn window_request(w: swath_warp::PixelWindow) -> WindowRequest {
    WindowRequest {
        col_off: w.col_off,
        row_off: w.row_off,
        width: w.width,
        height: w.height,
    }
}

/// Kernel errors → this crate's [`RenderError`], variant for variant.
pub(crate) fn render_error(err: &swath_warp::WarpError) -> RenderError {
    match *err {
        swath_warp::WarpError::NonInvertibleTransform { determinant } => {
            RenderError::NonInvertibleTransform { determinant }
        }
        swath_warp::WarpError::SourceShape { expected, actual } => {
            RenderError::SourceShape { expected, actual }
        }
        // `WarpError` is `#[non_exhaustive]` in its home crate; a new
        // variant must be adopted here explicitly, never silently mapped.
        _ => unreachable!("unmapped swath-warp error variant: {err:?}"),
    }
}

/// A core [`CoordTransform`] viewed through the kernel's transform port.
/// Both methods forward 1:1 (the batch path stays the adapter's batch
/// path), so the kernel drives the same calls the in-tree kernel did.
pub(crate) struct WarpTransform<'a>(pub(crate) &'a dyn CoordTransform);

fn transform_error(err: &ReprojectError) -> swath_warp::TransformError {
    match err {
        ReprojectError::OutOfDomain { .. } => swath_warp::TransformError::OutOfDomain,
        _ => swath_warp::TransformError::Failed,
    }
}

impl swath_warp::CoordTransform for WarpTransform<'_> {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), swath_warp::TransformError> {
        self.0.transform(x, y).map_err(|e| transform_error(&e))
    }

    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), swath_warp::TransformError> {
        self.0
            .transform_slice(points)
            .map_err(|e| transform_error(&e))
    }
}
