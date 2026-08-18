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
//! source CRS): the invariants are about *sampling*, and projection math
//! never lives in this crate.

use proptest::prelude::*;
use swath_warp::{
    CoordTransform, GeoTransform, GridBounds, NodataPolicy, PixelWindow, Resampling, SourceBuffer,
    SourceGrid, TargetGrid, TransformError, warp,
};

const NODATA: i16 = -9999;

struct Identity;

impl CoordTransform for Identity {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), TransformError> {
        Ok((x, y))
    }
}

/// A `w × h` raster at origin (0, 0), 1 m pixels, y growing downward,
/// whose window is the whole raster.
fn source(w: u64, h: u64, samples: &[f64]) -> SourceBuffer<'_> {
    SourceBuffer {
        grid: SourceGrid {
            width: w,
            height: h,
            transform: GeoTransform::north_up(0.0, 0.0, 1.0, -1.0),
        },
        window: PixelWindow {
            col_off: 0,
            row_off: 0,
            width: w,
            height: h,
        },
        samples,
        nodata: Some(f64::from(NODATA)),
    }
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
                    GridBounds {
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

/// The exact widening the samples contract requires: `i16` → `f64`.
fn widen(pixels: &[i16]) -> Vec<f64> {
    pixels.iter().copied().map(f64::from).collect()
}

proptest! {
    /// All-nodata source: every output pixel is invalid, whatever the
    /// kernel or geometry — nodata never fabricates data.
    #[test]
    fn all_nodata_source_yields_all_invalid(
        (w, h, _, grid, resampling) in cases()
    ) {
        let len = usize::try_from(w * h).expect("small dims");
        let samples = widen(&vec![NODATA; len]);
        let src = source(w, h, &samples);
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
        let samples = widen(&pixels);
        let src = source(w, h, &samples);
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
        let samples = widen(&pixels);
        let src = source(w, h, &samples);
        for policy in [NodataPolicy::ExcludeRenormalize, NodataPolicy::Propagate] {
            let out = warp(&src, &Identity, &grid, Resampling::Bilinear(policy))
            .expect("warp");
            let (lo, hi) = valid_vals
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                    (lo.min(*v), hi.max(*v))
                });
            for (v, valid) in out.values.iter().zip(&out.valid) {
                if *valid {
                    prop_assert!(
                        *v >= lo - 1e-9 && *v <= hi + 1e-9,
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
        let samples = widen(&vec![value; len]);
        let src = source(w, h, &samples);
        let out = warp(&src, &Identity, &grid, resampling).expect("warp");
        for (v, valid) in out.values.iter().zip(&out.valid) {
            if *valid {
                prop_assert!(
                    (*v - f64::from(value)).abs() < 1e-9,
                    "constant {value} warped to {v}"
                );
            }
        }
    }
}
