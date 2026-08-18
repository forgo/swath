// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Property tests for the warp kernels: the invariants no kernel is
//! allowed to break, regardless of grid geometry.
//!
//! * nodata never fabricates data — an all-nodata source window yields an
//!   all-invalid output;
//! * nearest is value-preserving — the output value set is a subset of the
//!   source value set (no new values are ever invented);
//! * bilinear output is bounded by the min/max of the valid source values;
//! * warping a constant-valued raster is constant wherever valid.
//!
//! All cases run through an identity `CoordTransform` (target CRS ==
//! source CRS): the invariants are about *sampling*, and projection math is
//! BIND, never build (ADR 0002).

use proptest::prelude::*;
use swath_core::crs::Crs;
use swath_core::raster::{DType, GeoTransform, RasterInfo, WindowRequest};
use swath_core::reproject::{CoordTransform, ReprojectError};
use swath_core::source::{PixelBuffer, WindowData};
use swath_core::tile::MercatorBounds;
use swath_render::{NodataPolicy, Resampling, TargetGrid, warp};

const NODATA: i16 = -9999;

struct Identity;

impl CoordTransform for Identity {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        Ok((x, y))
    }
}

/// A `w × h` raster at origin (0, 0), 1 m pixels, y growing downward,
/// whose window is the whole raster.
fn raster(w: u64, h: u64) -> RasterInfo {
    RasterInfo {
        crs: Crs::WEB_MERCATOR,
        width: w,
        height: h,
        transform: GeoTransform::north_up(0.0, 0.0, 1.0, -1.0),
        band_count: 1,
        dtype: DType::Int16,
        nodata: Some(f64::from(NODATA)),
        overview_levels: vec![],
    }
}

fn source(w: u64, h: u64, pixels: Vec<i16>) -> WindowData {
    WindowData::new(
        WindowRequest {
            col_off: 0,
            row_off: 0,
            width: w,
            height: h,
        },
        raster(w, h),
        PixelBuffer::Int16(pixels),
        Some(f64::from(NODATA)),
        vec![],
    )
}

/// Strategy: a random source raster (1..=12 per side, values with nodata
/// mixed in) plus a random target grid overlapping it (upsampling and
/// downsampling both reachable).
fn cases() -> impl Strategy<Value = (u64, u64, Vec<i16>, TargetGrid, Resampling)> {
    (1_u64..=12, 1_u64..=12)
        .prop_flat_map(|(w, h)| {
            let len = usize::try_from(w * h).expect("small dims");
            (
                Just(w),
                Just(h),
                proptest::collection::vec(
                    prop_oneof![3 => -2000_i16..=8000, 1 => Just(NODATA)],
                    len,
                ),
                // Target grid bounds around (and beyond) the raster.
                (-4.0_f64..=16.0, 1.0_f64..=20.0),
                (-16.0_f64..=4.0, 1.0_f64..=20.0),
                (1_u32..=16, 1_u32..=16),
                prop_oneof![
                    Just(Resampling::Nearest),
                    Just(Resampling::Bilinear(NodataPolicy::ExcludeRenormalize)),
                    Just(Resampling::Bilinear(NodataPolicy::Propagate)),
                ],
            )
        })
        .prop_map(
            |(w, h, pixels, (min_x, dx), (min_y, dy), (gw, gh), resampling)| {
                let grid = TargetGrid::new(
                    MercatorBounds {
                        min_x,
                        min_y,
                        max_x: min_x + dx,
                        max_y: min_y + dy,
                    },
                    gw,
                    gh,
                );
                (w, h, pixels, grid, resampling)
            },
        )
}

proptest! {
    /// All-nodata source: every output pixel is invalid, whatever the
    /// kernel or geometry — nodata never fabricates data.
    #[test]
    fn all_nodata_source_yields_all_invalid(
        (w, h, _, grid, resampling) in cases()
    ) {
        let len = usize::try_from(w * h).expect("small dims");
        let src = source(w, h, vec![NODATA; len]);
        let out = warp(&src, &Identity, &grid, resampling).expect("warp");
        prop_assert_eq!(out.valid_count(), 0);
        prop_assert!(out.values.iter().all(|v| *v == 0.0));
    }

    /// Nearest never invents values: every valid output value is exactly
    /// one of the source's non-nodata values.
    #[test]
    fn nearest_preserves_the_source_value_set(
        (w, h, pixels, grid, _) in cases()
    ) {
        let src = source(w, h, pixels.clone());
        let out = warp(&src, &Identity, &grid, Resampling::Nearest)
            .expect("warp");
        for (v, valid) in out.values.iter().zip(&out.valid) {
            if *valid {
                #[allow(clippy::cast_possible_truncation, reason = "values are exact i16")]
                let as_i16 = *v as i16;
                prop_assert!(
                    (f64::from(as_i16) - *v).abs() < f64::EPSILON,
                    "non-integral nearest value {v}"
                );
                prop_assert!(
                    as_i16 != NODATA && pixels.contains(&as_i16),
                    "value {v} not in the source set"
                );
            }
        }
    }

    /// Bilinear output is a convex combination of valid source values, so
    /// it is bounded by their min and max.
    #[test]
    fn bilinear_is_bounded_by_valid_source_values(
        (w, h, pixels, grid, _) in cases()
    ) {
        let valid_vals: Vec<f64> = pixels
            .iter()
            .filter(|p| **p != NODATA)
            .map(|p| f64::from(*p))
            .collect();
        let src = source(w, h, pixels);
        for policy in [NodataPolicy::ExcludeRenormalize, NodataPolicy::Propagate] {
            let out = warp(&src, &Identity, &grid, Resampling::Bilinear(policy))
            .expect("warp");
            let (lo, hi) = valid_vals
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(*v), hi.max(*v))
                });
            // Margin: under the skip-the-divide shortcut (weight sum within
            // 1e-5 of 1.0, divide skipped) a convex combination scales by up
            // to 1e-5, so the bound can be exceeded by |bound| * 1e-5 (#240).
            let margin = 1e-9 + lo.abs().max(hi.abs()) * 1.1e-5;
            for (v, valid) in out.values.iter().zip(&out.valid) {
                if *valid {
                    prop_assert!(
                        *v >= lo - margin && *v <= hi + margin,
                        "value {v} outside valid source range [{lo}, {hi}]"
                    );
                }
            }
        }
    }

    /// Warping a constant raster returns that constant wherever valid,
    /// under every kernel and geometry.
    #[test]
    fn constant_raster_warps_to_the_constant(
        (w, h, _, grid, resampling) in cases(),
        value in -2000_i16..=8000,
    ) {
        let len = usize::try_from(w * h).expect("small dims");
        let src = source(w, h, vec![value; len]);
        let out = warp(&src, &Identity, &grid, resampling).expect("warp");
        for (v, valid) in out.values.iter().zip(&out.valid) {
            if *valid {
                // Tolerance: the kernel's GDAL-faithful skip-the-divide
                // shortcut returns `acc` undivided when the support weight
                // sum is within 1e-5 of 1.0, so a constant can come back as
                // `c * weight_sum` — relative error up to 1e-5 (issue #240).
                let tol = f64::from(value).abs() * 1.1e-5 + 1e-9;
                prop_assert!(
                    (*v - f64::from(value)).abs() <= tol,
                    "constant {value} warped to {v}"
                );
            }
        }
    }
}
