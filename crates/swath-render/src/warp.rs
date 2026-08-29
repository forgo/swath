// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The workspace face of the published `swath-warp` kernel (ADR 0016):
//! the inverse-mapping warp and the source-window geometry, over this
//! workspace's own types. `swath-warp` is self-contained by rule — it
//! never depends on `swath-core` — so the boundary needs exactly this much
//! plumbing and no more (#352): field-for-field conversions of the core
//! raster/window types, a newtype that presents the core
//! [`CoordTransform`] port through the kernel's transform trait, the
//! `PixelBuffer` → `f64` widening, and the kernel's errors mapped onto
//! [`RenderError`]. The target grid is the kernel's own [`TargetGrid`];
//! [`tile_grid`] builds one for a `WebMercatorQuad` tile. Zero behaviour
//! change from the pre-extraction kernel: the goldens and property tests
//! in this crate's `tests/` hold this path to the GDAL oracle.

use swath_core::raster::{GeoTransform, RasterInfo, WindowRequest};
use swath_core::reproject::{CoordTransform, ReprojectError};
use swath_core::source::{PixelBuffer, WindowData};
use swath_core::tile::{TileCoord, WebMercatorQuad};

use crate::error::RenderError;

pub(crate) use swath_warp::SourceExtent;
pub use swath_warp::{GridBounds, NodataPolicy, Resampling, TargetGrid, WarpedBuffer};

/// The grid of one `WebMercatorQuad` tile at `tile_size` pixels per side
/// (256 for classic XYZ tiles, 512 for retina): row 0 at the northern
/// edge, like every raster in the system.
#[must_use]
pub fn tile_grid(tile: TileCoord, tile_size: u32) -> TargetGrid {
    let b = WebMercatorQuad::xy_bounds(tile);
    TargetGrid::new(
        GridBounds {
            min_x: b.min_x,
            min_y: b.min_y,
            max_x: b.max_x,
            max_y: b.max_y,
        },
        tile_size,
        tile_size,
    )
}

/// Core [`GeoTransform`] → the kernel's own six-parameter affine.
fn geo_transform(gt: &GeoTransform) -> swath_warp::GeoTransform {
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
fn source_grid(info: &RasterInfo) -> swath_warp::SourceGrid {
    swath_warp::SourceGrid {
        width: info.width,
        height: info.height,
        transform: geo_transform(&info.transform),
    }
}

/// Core [`WindowRequest`] → the kernel's pixel window.
fn pixel_window(w: WindowRequest) -> swath_warp::PixelWindow {
    swath_warp::PixelWindow {
        col_off: w.col_off,
        row_off: w.row_off,
        width: w.width,
        height: w.height,
    }
}

/// The kernel's pixel window → core [`WindowRequest`].
fn window_request(w: swath_warp::PixelWindow) -> WindowRequest {
    WindowRequest {
        col_off: w.col_off,
        row_off: w.row_off,
        width: w.width,
        height: w.height,
    }
}

/// Kernel errors → this crate's [`RenderError`], variant for variant.
fn render_error(err: &swath_warp::WarpError) -> RenderError {
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
struct WarpTransform<'a>(&'a dyn CoordTransform);

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

/// Widens a buffer to `f64` samples (exact for every supported variant),
/// or `None` for a variant the kernel does not know yet.
fn widen(pixels: &PixelBuffer) -> Option<Vec<f64>> {
    match pixels {
        PixelBuffer::UInt8(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Int16(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::UInt16(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Int32(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Float32(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Float64(v) => Some(v.clone()),
        _ => None,
    }
}

/// Warps `source` into `target` by inverse mapping: each target pixel
/// center is mapped through `transform` (**target CRS → source CRS**) and
/// `source.grid.transform` (the geotransform of the grid the window was
/// read from — `source.window` places the buffer within that grid) and
/// sampled with `resampling`. See [`swath_warp::warp`] for the kernel's
/// full contract (GDAL-equivalent resampling geometry, validity rules,
/// batch-transform behavior).
///
/// The grid comes from the [`WindowData`] itself (never passed
/// separately), so overview reads warp correctly by construction: an
/// overview window carries the overview grid, and every coordinate —
/// window offsets, kernel window, raster bounds — is in that grid's pixel
/// space (#38).
///
/// The source window must cover the resampling support: bilinear needs the
/// tile's source extent plus 1 pixel at scale ≥ 1, growing to
/// `ceil(1/scale) + 1` when the warp decimates (pass that as the `margin`
/// of [`crate::source_window`]).
///
/// # Errors
///
/// * [`RenderError::SourceShape`] — buffer length disagrees with its window.
/// * [`RenderError::NonInvertibleTransform`] — singular geotransform.
/// * [`RenderError::UnsupportedDtype`] — pixel-buffer variant unknown to
///   the kernel.
pub fn warp(
    source: &WindowData,
    transform: &dyn CoordTransform,
    target: &TargetGrid,
    resampling: Resampling,
) -> Result<WarpedBuffer, RenderError> {
    let samples = widen(&source.pixels).ok_or(RenderError::UnsupportedDtype {
        dtype: source.pixels.dtype(),
    })?;
    let buffer = swath_warp::SourceBuffer {
        grid: source_grid(&source.grid),
        window: pixel_window(source.window),
        samples: &samples,
        nodata: source.nodata,
    };
    swath_warp::warp(&buffer, &WarpTransform(transform), target, resampling)
        .map_err(|err| render_error(&err))
}

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
    Ok(
        swath_warp::source_window(grid, &source_grid(info), &WarpTransform(to_source), margin)
            .map_err(|err| render_error(&err))?
            .map(window_request),
    )
}

/// Traces the densified boundary of `grid` through `to_source` into
/// fractional source pixel coordinates ([`swath_warp::source_extent`]).
/// `Ok(None)` when every boundary point is outside the transform's domain.
pub(crate) fn source_extent(
    grid: &TargetGrid,
    info: &RasterInfo,
    to_source: &dyn CoordTransform,
) -> Result<Option<SourceExtent>, RenderError> {
    swath_warp::source_extent(grid, &source_grid(info), &WarpTransform(to_source))
        .map_err(|err| render_error(&err))
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
    swath_warp::clip_to_raster(ext, margin, &source_grid(info)).map(window_request)
}

#[cfg(test)]
mod warp_tests {
    use swath_core::crs::Crs;
    use swath_core::raster::{DType, GeoTransform, RasterInfo, WindowRequest};
    use swath_core::reproject::{CoordTransform, ReprojectError};
    use swath_core::source::{PixelBuffer, WindowData};
    use swath_warp::GridBounds;

    use super::{NodataPolicy, Resampling, TargetGrid, WarpedBuffer, warp};
    use crate::error::RenderError;

    struct Identity;

    impl CoordTransform for Identity {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            Ok((x, y))
        }
    }

    /// Rejects x < 0 to exercise the per-point fallback path.
    struct RejectWest;

    impl CoordTransform for RejectWest {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            if x < 0.0 {
                Err(ReprojectError::OutOfDomain { x, y })
            } else {
                Ok((x, y))
            }
        }
    }

    /// A window at full-res offset (0, 0): w×h pixels, 1 m grid, origin (0, 0)
    /// with y growing downward from 0 (`north_up` with `origin_y` = 0).
    fn window(w: u64, h: u64, pixels: Vec<i16>, nodata: Option<f64>) -> WindowData {
        WindowData::new(
            WindowRequest {
                col_off: 0,
                row_off: 0,
                width: w,
                height: h,
            },
            info(w, h),
            PixelBuffer::Int16(pixels),
            nodata,
            vec![],
        )
    }

    fn gt() -> GeoTransform {
        GeoTransform::north_up(0.0, 0.0, 1.0, -1.0)
    }

    /// Raster metadata matching [`window`]/[`gt`]: a `w × h` raster whose
    /// window is the whole raster.
    fn info(w: u64, h: u64) -> RasterInfo {
        RasterInfo {
            crs: Crs::WEB_MERCATOR,
            width: w,
            height: h,
            transform: gt(),
            band_count: 1,
            dtype: DType::Int16,
            nodata: None,
            overview_levels: vec![],
        }
    }

    /// A target grid aligned 1:1 with source pixels (identity warp).
    fn aligned_grid(w: u32, h: u32) -> TargetGrid {
        TargetGrid::new(
            GridBounds {
                min_x: 0.0,
                min_y: -f64::from(h),
                max_x: f64::from(w),
                max_y: 0.0,
            },
            w,
            h,
        )
    }

    #[test]
    fn identity_nearest_reproduces_the_source() {
        let src = window(4, 4, (0..16).collect(), None);
        let out = warp(&src, &Identity, &aligned_grid(4, 4), Resampling::Nearest).unwrap();
        assert_eq!(out.valid_count(), 16);
        let got: Vec<f64> = out.values.clone();
        assert_eq!(got, (0..16).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn identity_bilinear_at_pixel_centers_reproduces_the_source() {
        let src = window(4, 4, (0..16).collect(), None);
        let out = warp(
            &src,
            &Identity,
            &aligned_grid(4, 4),
            Resampling::Bilinear(NodataPolicy::default()),
        )
        .unwrap();
        assert_eq!(out.valid_count(), 16);
        assert_eq!(out.values, (0..16).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn bilinear_interpolates_between_centers() {
        // 2×1 source [10, 30]; a 2× upsampled grid puts target centers at
        // source-local x = 0.25, 0.75, 1.25, 1.75 → u = -0.25, 0.25, 0.75, 1.25.
        let src = window(2, 1, vec![10, 30], None);
        let target = TargetGrid::new(
            GridBounds {
                min_x: 0.0,
                min_y: -1.0,
                max_x: 2.0,
                max_y: 0.0,
            },
            4,
            1,
        );
        let out = warp(
            &src,
            &Identity,
            &target,
            Resampling::Bilinear(NodataPolicy::default()),
        )
        .unwrap();
        // Edge pixels renormalize to their single in-bounds neighbour.
        assert_eq!(out.values, vec![10.0, 15.0, 25.0, 30.0]);
        assert_eq!(out.valid_count(), 4);
    }

    #[test]
    fn nearest_rejects_nodata_and_out_of_window() {
        let src = window(2, 2, vec![7, -9999, 7, 7], Some(-9999.0));
        let out = warp(&src, &Identity, &aligned_grid(2, 2), Resampling::Nearest).unwrap();
        assert_eq!(out.valid, vec![true, false, true, true]);
        assert!(out.values[1].abs() < f64::EPSILON);
    }

    #[test]
    fn bilinear_renormalizes_around_nodata_but_propagate_invalidates() {
        // 2×2 with one nodata corner; sample at (0.9, 0.9) — the containing
        // pixel (0, 0) is valid (so GDAL's containing-pixel gate passes) but
        // the 2×2 support includes the nodata corner with weight 0.16.
        let src = window(2, 2, vec![100, 100, 100, -9999], Some(-9999.0));
        let center = TargetGrid::new(
            GridBounds {
                min_x: 0.4,
                min_y: -1.4,
                max_x: 1.4,
                max_y: -0.4,
            },
            1,
            1,
        );
        let out = warp(
            &src,
            &Identity,
            &center,
            Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
        )
        .unwrap();
        assert_eq!(out.valid, vec![true]);
        assert!((out.values[0] - 100.0).abs() < 1e-12);

        let out = warp(
            &src,
            &Identity,
            &center,
            Resampling::Bilinear(NodataPolicy::Propagate),
        )
        .unwrap();
        assert_eq!(out.valid, vec![false]);
    }

    #[test]
    fn containing_pixel_nodata_gates_bilinear_output() {
        // GDAL invalidates the destination pixel when the source pixel
        // *containing* the mapped point is nodata, even though the bilinear
        // support holds three valid samples. Sample dead-center of the
        // nodata pixel (1.5, 1.5): support weights would renormalize to the
        // three valid neighbours, but the gate wins.
        let src = window(2, 2, vec![100, 100, 100, -9999], Some(-9999.0));
        let over_nodata = TargetGrid::new(
            GridBounds {
                min_x: 1.0,
                min_y: -2.0,
                max_x: 2.0,
                max_y: -1.0,
            },
            1,
            1,
        );
        for policy in [NodataPolicy::ExcludeRenormalize, NodataPolicy::Propagate] {
            let out = warp(&src, &Identity, &over_nodata, Resampling::Bilinear(policy)).unwrap();
            assert_eq!(out.valid, vec![false], "policy {policy:?}");
        }
    }

    #[test]
    fn all_nodata_support_is_invalid_under_both_policies() {
        let src = window(2, 2, vec![-9999; 4], Some(-9999.0));
        for policy in [NodataPolicy::ExcludeRenormalize, NodataPolicy::Propagate] {
            let out = warp(
                &src,
                &Identity,
                &aligned_grid(2, 2),
                Resampling::Bilinear(policy),
            )
            .unwrap();
            assert_eq!(out.valid_count(), 0, "policy {policy:?}");
        }
    }

    #[test]
    fn out_of_domain_pixels_are_invalid_not_errors() {
        // Grid straddling x = 0; RejectWest fails the batch, and the
        // per-point fallback marks the western half invalid.
        let src = window(4, 1, vec![1, 2, 3, 4], None);
        let target = TargetGrid::new(
            GridBounds {
                min_x: -2.0,
                min_y: -1.0,
                max_x: 2.0,
                max_y: 0.0,
            },
            4,
            1,
        );
        let out = warp(&src, &RejectWest, &target, Resampling::Nearest).unwrap();
        assert_eq!(out.valid, vec![false, false, true, true]);
    }

    #[test]
    fn shape_mismatch_and_singular_transform_are_errors() {
        let src = window(4, 4, vec![0; 15], None);
        let err = warp(&src, &Identity, &aligned_grid(4, 4), Resampling::Nearest)
            .expect_err("shape mismatch");
        assert_eq!(
            err,
            RenderError::SourceShape {
                expected: 16,
                actual: 15
            }
        );

        let mut src = window(2, 2, vec![0; 4], None);
        src.grid.transform = GeoTransform::north_up(0.0, 0.0, 1.0, 0.0);
        assert!(matches!(
            warp(&src, &Identity, &aligned_grid(2, 2), Resampling::Nearest),
            Err(RenderError::NonInvertibleTransform { .. })
        ));
    }

    #[test]
    fn empty_source_window_yields_all_invalid() {
        let src = window(0, 0, vec![], Some(-9999.0));
        let out = warp(&src, &Identity, &aligned_grid(2, 2), Resampling::Nearest).unwrap();
        assert_eq!(out.valid_count(), 0);
        let len = 4;
        assert_eq!(
            out,
            WarpedBuffer {
                width: 2,
                height: 2,
                values: vec![0.0; len],
                valid: vec![false; len],
            }
        );
    }
}

#[cfg(test)]
mod window_tests {
    use swath_core::crs::Crs;
    use swath_core::raster::{DType, GeoTransform, RasterInfo};
    use swath_core::reproject::{CoordTransform, ReprojectError};
    use swath_warp::GridBounds;

    use super::{TargetGrid, source_window};

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
            GridBounds {
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

#[cfg(test)]
mod grid_tests {
    use swath_core::tile::TileCoord;

    use super::tile_grid;

    #[test]
    fn tile_grid_pixel_centers_span_the_tile() {
        let tile = TileCoord::new(12, 848, 1561).unwrap();
        let grid = tile_grid(tile, 256);
        let (dx, dy) = grid.pixel_size();
        assert!(dx > 0.0 && dy > 0.0);
        assert!((dx - dy).abs() < 1e-9, "web mercator tiles are square");
        let b = grid.bounds();
        // First center is half a pixel inside the top-left corner…
        let (x0, y0) = grid.pixel_center(0, 0);
        assert!((x0 - (b.min_x + dx / 2.0)).abs() < 1e-6);
        assert!((y0 - (b.max_y - dy / 2.0)).abs() < 1e-6);
        // …and the last is half a pixel inside the bottom-right corner.
        let (x1, y1) = grid.pixel_center(255, 255);
        assert!((x1 - (b.max_x - dx / 2.0)).abs() < 1e-6);
        assert!((y1 - (b.min_y + dy / 2.0)).abs() < 1e-6);
    }

    #[test]
    fn tile_size_parameter_scales_resolution() {
        let tile = TileCoord::new(12, 848, 1561).unwrap();
        let g256 = tile_grid(tile, 256);
        let g512 = tile_grid(tile, 512);
        assert_eq!(g256.bounds(), g512.bounds());
        assert!((g256.pixel_size().0 - 2.0 * g512.pixel_size().0).abs() < 1e-9);
    }
}
