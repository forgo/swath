// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Source-window computation: which source pixels does a tile need?
//!
//! The geometry itself — densified boundary trace, fractional extent,
//! raster clipping — lives in the extracted `swath-warp` crate
//! (ADR 0016, #186); this module wraps it in the workspace's core types
//! through `crate::shim`, field for field, with zero behavior change.

use swath_core::raster::{RasterInfo, WindowRequest};
use swath_core::reproject::CoordTransform;

use crate::error::RenderError;
use crate::grid::TargetGrid;
use crate::shim;

pub use swath_warp::BOUNDARY_SAMPLES_PER_EDGE;
pub(crate) use swath_warp::SourceExtent;

/// Computes the minimal source-pixel window covering `grid`, with `margin`
/// extra pixels on every side for resampling-kernel support (bilinear needs
/// 1; pass more to keep headroom for future kernels), clipped to the raster.
///
/// `to_source` transforms **target CRS → source CRS** (e.g. EPSG:3857 →
/// the raster's UTM zone). Boundary points the transform rejects (out of
/// domain) are skipped, mirroring GDAL's treatment of untransformable edge
/// samples; if *every* boundary point is rejected, or the covered region
/// misses the raster entirely, the tile has no source data and `Ok(None)`
/// is returned. See [`swath_warp::source_window`] for the full contract.
///
/// # Errors
///
/// [`RenderError::NonInvertibleTransform`] when the raster's geotransform
/// cannot map CRS coordinates back to pixels.
pub fn source_window(
    grid: &TargetGrid,
    info: &RasterInfo,
    to_source: &dyn CoordTransform,
    margin: u32,
) -> Result<Option<WindowRequest>, RenderError> {
    Ok(swath_warp::source_window(
        &shim::target_grid(grid),
        &shim::source_grid(info),
        &shim::WarpTransform(to_source),
        margin,
    )
    .map_err(|err| shim::render_error(&err))?
    .map(shim::window_request))
}

/// Traces the densified boundary of `grid` through `to_source` into
/// fractional source pixel coordinates ([`swath_warp::source_extent`]).
/// `Ok(None)` when every boundary point is outside the transform's domain.
pub(crate) fn source_extent(
    grid: &TargetGrid,
    info: &RasterInfo,
    to_source: &dyn CoordTransform,
) -> Result<Option<SourceExtent>, RenderError> {
    swath_warp::source_extent(
        &shim::target_grid(grid),
        &shim::source_grid(info),
        &shim::WarpTransform(to_source),
    )
    .map_err(|err| shim::render_error(&err))
}

// NOTE: the overview selection rule (#38's `select_overview` and its 1.2
// oversampling threshold) was re-homed to `swath_core::planner` by #37:
// eligibility is now the planner's overview-candidate rule, with the
// threshold promoted to the `Budget::overview_oversample` knob. This
// module keeps the pure window geometry the tiler feeds the planner.

/// Expands the fractional pixel bounds by `margin`, snaps outward to whole
/// pixels, and intersects with the raster grid
/// ([`swath_warp::clip_to_raster`]).
pub(crate) fn clip_to_raster(
    ext: &SourceExtent,
    margin: u32,
    info: &RasterInfo,
) -> Option<WindowRequest> {
    swath_warp::clip_to_raster(ext, margin, &shim::source_grid(info)).map(shim::window_request)
}

#[cfg(test)]
mod tests {
    use swath_core::crs::Crs;
    use swath_core::raster::{DType, GeoTransform, RasterInfo};
    use swath_core::reproject::{CoordTransform, ReprojectError};
    use swath_core::tile::MercatorBounds;

    use super::source_window;
    use crate::grid::TargetGrid;

    /// Identity "transform" — target CRS == source CRS. Projection math is
    /// BIND (ADR 0002); this is plumbing, not projection.
    struct Identity;

    impl CoordTransform for Identity {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            Ok((x, y))
        }
    }

    /// A transform whose domain excludes everything (every point rejected).
    struct NoDomain;

    impl CoordTransform for NoDomain {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            Err(ReprojectError::OutOfDomain { x, y })
        }
    }

    /// A 100×80 raster at origin (0, 100), 1 m pixels, in the "same" CRS as
    /// the target grid.
    fn info() -> RasterInfo {
        RasterInfo {
            crs: Crs::WEB_MERCATOR,
            width: 100,
            height: 80,
            transform: GeoTransform::north_up(0.0, 100.0, 1.0, -1.0),
            band_count: 1,
            dtype: DType::UInt8,
            nodata: None,
            overview_levels: vec![],
        }
    }

    fn grid(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> TargetGrid {
        TargetGrid::new(
            MercatorBounds {
                min_x,
                min_y,
                max_x,
                max_y,
            },
            16,
            16,
        )
    }

    #[test]
    fn interior_tile_covers_exactly_with_margin() {
        // Grid spanning x 10..20, y 60..70 → pixels cols 10..20, rows 30..40.
        let g = grid(10.0, 60.0, 20.0, 70.0);
        let w = source_window(&g, &info(), &Identity, 0).unwrap().unwrap();
        assert_eq!((w.col_off, w.row_off, w.width, w.height), (10, 30, 10, 10));
        let w = source_window(&g, &info(), &Identity, 2).unwrap().unwrap();
        assert_eq!((w.col_off, w.row_off, w.width, w.height), (8, 28, 14, 14));
    }

    #[test]
    fn window_clips_to_the_raster() {
        // Overhangs the west and north edges.
        let g = grid(-5.0, 95.0, 5.0, 105.0);
        let w = source_window(&g, &info(), &Identity, 1).unwrap().unwrap();
        assert_eq!((w.col_off, w.row_off), (0, 0));
        assert_eq!((w.width, w.height), (6, 6));
    }

    #[test]
    fn tile_fully_outside_is_none() {
        let g = grid(200.0, 0.0, 210.0, 10.0); // east of the raster
        assert_eq!(source_window(&g, &info(), &Identity, 4).unwrap(), None);
        let g = grid(0.0, 150.0, 10.0, 160.0); // north of the raster
        assert_eq!(source_window(&g, &info(), &Identity, 4).unwrap(), None);
    }

    #[test]
    fn all_points_out_of_domain_is_none() {
        let g = grid(10.0, 60.0, 20.0, 70.0);
        assert_eq!(source_window(&g, &info(), &NoDomain, 1).unwrap(), None);
    }

    // The overview selection truth table moved to `swath_core::planner`
    // with the rule itself (#37) — see
    // `planner::tests::overview_selection_follows_the_gdal_rule`.

    #[test]
    fn singular_geotransform_is_an_error() {
        let mut bad = info();
        bad.transform = GeoTransform::north_up(0.0, 100.0, 1.0, 0.0);
        let g = grid(10.0, 60.0, 20.0, 70.0);
        assert!(source_window(&g, &bad, &Identity, 0).is_err());
    }
}
