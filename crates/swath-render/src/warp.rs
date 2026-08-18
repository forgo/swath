// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The inverse-mapping warp: the workspace face of the extracted
//! `swath-warp` kernel (ADR 0016, #186).
//!
//! The GDAL-exact kernel itself — nearest and bilinear resampling with
//! GDAL 3.12 `GDALWarpKernel` semantics — lives in the published
//! `swath-warp` crate; this module re-exports its output vocabulary
//! ([`NodataPolicy`], [`Resampling`], [`WarpedBuffer`]) and wraps [`warp`]
//! in the workspace's own types ([`WindowData`], the core
//! [`CoordTransform`] port, this crate's [`TargetGrid`] and
//! [`RenderError`]) through the field-for-field conversions in
//! `crate::shim`. Zero behavior change: the goldens and property tests in
//! this crate's `tests/` keep holding this path to the GDAL oracle.

use swath_core::reproject::CoordTransform;
use swath_core::source::{PixelBuffer, WindowData};

use crate::error::RenderError;
use crate::grid::TargetGrid;
use crate::shim;

pub use swath_warp::{NodataPolicy, Resampling, WarpedBuffer};

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
        grid: shim::source_grid(&source.grid),
        window: shim::pixel_window(source.window),
        samples: &samples,
        nodata: source.nodata,
    };
    swath_warp::warp(
        &buffer,
        &shim::WarpTransform(transform),
        &shim::target_grid(target),
        resampling,
    )
    .map_err(|err| shim::render_error(&err))
}

#[cfg(test)]
mod tests {
    use swath_core::crs::Crs;
    use swath_core::raster::{DType, GeoTransform, RasterInfo, WindowRequest};
    use swath_core::reproject::{CoordTransform, ReprojectError};
    use swath_core::source::{PixelBuffer, WindowData};
    use swath_core::tile::MercatorBounds;

    use super::{NodataPolicy, Resampling, WarpedBuffer, warp};
    use crate::error::RenderError;
    use crate::grid::TargetGrid;

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
            MercatorBounds {
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
            MercatorBounds {
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
            MercatorBounds {
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
            MercatorBounds {
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
            MercatorBounds {
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
