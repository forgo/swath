// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Source-window computation: which source pixels does a tile need?

use swath_core::raster::{RasterInfo, WindowRequest};
use swath_core::reproject::CoordTransform;

use crate::error::RenderError;
use crate::grid::TargetGrid;

/// Number of sample points per tile edge (corners included) when tracing the
/// tile boundary through the CRS transform.
///
/// **Why densify at all:** the transform from Web Mercator to a source CRS
/// (UTM, say) is smooth but not affine, so a straight tile edge maps to a
/// *curve* in the source CRS. Sampling only the 4 corners bounds the chord,
/// not the curve — where the curve bulges outside the corner-spanned box
/// (meridian convergence and scale variation guarantee it does somewhere),
/// a corner-only window under-reads and the warp fabricates invalid edge
/// pixels. The chord-to-curve deviation (sagitta) shrinks quadratically with
/// the number of intervals, so a modest density makes the residual
/// negligible: at 21 points (20 intervals) it is 1/400 of the single-chord
/// error, far below one source pixel for any tile a single raster can cover,
/// and the caller's pixel `margin` absorbs what remains. 21 points per edge
/// is also the density GDAL itself uses when sampling transform extents
/// (`GDALSuggestedWarpOutput`'s 21×2 edge points), so the oracle and the
/// kernel agree on curvature handling.
///
/// Only the boundary is sampled, not the interior: CRS transforms are
/// continuous and injective on their domain, so the image of the tile's
/// boundary encloses the image of its interior — interior points can never
/// extend the bounding box.
pub const BOUNDARY_SAMPLES_PER_EDGE: usize = 21;

/// Computes the minimal source-pixel window covering `grid`, with `margin`
/// extra pixels on every side for resampling-kernel support (bilinear needs
/// 1; pass more to keep headroom for future kernels), clipped to the raster.
///
/// `to_source` transforms **target CRS → source CRS** (e.g. EPSG:3857 →
/// the raster's UTM zone). Boundary points the transform rejects (out of
/// domain) are skipped, mirroring GDAL's treatment of untransformable edge
/// samples; if *every* boundary point is rejected, or the covered region
/// misses the raster entirely, the tile has no source data and `Ok(None)`
/// is returned.
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
    let Some(extent) = source_extent(grid, info, to_source)? else {
        return Ok(None);
    };
    Ok(clip_to_raster(&extent, margin, info))
}

/// The fractional source-pixel bounding box of a target grid's boundary —
/// shared by [`source_window`] (which clips it to a read request) and the
/// warp kernel (which derives GDAL's resampling scales from it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SourceExtent {
    /// Westmost fractional column.
    pub(crate) min_col: f64,
    /// Eastmost fractional column.
    pub(crate) max_col: f64,
    /// Northmost fractional row.
    pub(crate) min_row: f64,
    /// Southmost fractional row.
    pub(crate) max_row: f64,
}

/// Traces the densified boundary of `grid` through `to_source` into
/// fractional source pixel coordinates. `Ok(None)` when every boundary
/// point is outside the transform's domain.
pub(crate) fn source_extent(
    grid: &TargetGrid,
    info: &RasterInfo,
    to_source: &dyn CoordTransform,
) -> Result<Option<SourceExtent>, RenderError> {
    let det = info.transform.determinant();
    if det == 0.0 {
        return Err(RenderError::NonInvertibleTransform { determinant: det });
    }

    let bounds = grid.bounds();
    let n = BOUNDARY_SAMPLES_PER_EDGE;
    let mut ext = SourceExtent {
        min_col: f64::INFINITY,
        max_col: f64::NEG_INFINITY,
        min_row: f64::INFINITY,
        max_row: f64::NEG_INFINITY,
    };
    let mut any = false;

    #[allow(clippy::cast_precision_loss, reason = "n is a small constant")]
    let step = 1.0 / (n - 1) as f64;
    for i in 0..n {
        #[allow(clippy::cast_precision_loss, reason = "n is a small constant")]
        let t = i as f64 * step;
        let x = bounds.min_x + t * (bounds.max_x - bounds.min_x);
        let y = bounds.min_y + t * (bounds.max_y - bounds.min_y);
        // The four edges: north, south, west, east.
        for (edge_x, edge_y) in [
            (x, bounds.max_y),
            (x, bounds.min_y),
            (bounds.min_x, y),
            (bounds.max_x, y),
        ] {
            let Ok((sx, sy)) = to_source.transform(edge_x, edge_y) else {
                continue; // out of the transform's domain: excluded
            };
            // Determinant checked above: crs_to_pixel cannot fail.
            let Ok((col, row)) = info.transform.crs_to_pixel(sx, sy) else {
                continue;
            };
            ext.min_col = ext.min_col.min(col);
            ext.max_col = ext.max_col.max(col);
            ext.min_row = ext.min_row.min(row);
            ext.max_row = ext.max_row.max(row);
            any = true;
        }
    }

    Ok(any.then_some(ext))
}

// NOTE: the overview selection rule (#38's `select_overview` and its 1.2
// oversampling threshold) was re-homed to `swath_core::planner` by #37:
// eligibility is now the planner's overview-candidate rule, with the
// threshold promoted to the `Budget::overview_oversample` knob. This
// module keeps the pure window geometry the tiler feeds the planner.

/// Expands the fractional pixel bounds by `margin`, snaps outward to whole
/// pixels, and intersects with the raster grid.
pub(crate) fn clip_to_raster(
    ext: &SourceExtent,
    margin: u32,
    info: &RasterInfo,
) -> Option<WindowRequest> {
    let m = f64::from(margin);
    let lo_col = (ext.min_col - m).floor();
    let hi_col = (ext.max_col + m).ceil();
    let lo_row = (ext.min_row - m).floor();
    let hi_row = (ext.max_row + m).ceil();

    #[allow(
        clippy::cast_precision_loss,
        reason = "raster dims far below 2^52 in practice; comparison only"
    )]
    let (w, h) = (info.width as f64, info.height as f64);
    if hi_col <= 0.0 || hi_row <= 0.0 || lo_col >= w || lo_row >= h {
        return None;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "values clamped into [0, raster dimension] before the cast"
    )]
    let (col_off, row_off, end_col, end_row) = (
        lo_col.clamp(0.0, w) as u64,
        lo_row.clamp(0.0, h) as u64,
        hi_col.clamp(0.0, w) as u64,
        hi_row.clamp(0.0, h) as u64,
    );
    if end_col <= col_off || end_row <= row_off {
        return None;
    }
    Some(WindowRequest {
        col_off,
        row_off,
        width: end_col - col_off,
        height: end_row - row_off,
    })
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
