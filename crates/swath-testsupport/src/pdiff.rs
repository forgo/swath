// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `pdiff` — perceptual-diff utilities for validating Swath renders against the
//! GDAL/rio-tiler correctness oracle (ADR 0002: GDAL lives only in the test
//! suite; `tests/oracle/render_reference.py` produces the reference tiles).
//!
//! The comparison model is deliberately simple and explicit: two images match
//! under a [`DiffPolicy`] when, per pixel, every channel (alpha included —
//! nodata masking bugs surface there) differs by at most
//! `per_channel_tolerance`, and the fraction of pixels violating that bound is
//! at most `max_bad_pixel_fraction`. A dimension mismatch is always a hard
//! error, never a degraded score.

use std::fmt;
use std::path::Path;

pub use image::RgbaImage;

/// Number of distinct per-channel absolute-difference values (`u8` range).
const DIFF_BINS: usize = 256;

/// Errors from loading or comparing images.
#[derive(Debug)]
pub enum DiffError {
    /// The file could not be read or decoded as an image.
    Decode {
        /// Path of the offending file.
        path: String,
        /// Underlying decoder error.
        source: image::ImageError,
    },
    /// The two images have different pixel dimensions — a hard failure by
    /// policy: a resampling pipeline that changes tile geometry is wrong in a
    /// way no per-pixel tolerance should paper over.
    DimensionMismatch {
        /// Dimensions of the first image (width, height).
        a: (u32, u32),
        /// Dimensions of the second image (width, height).
        b: (u32, u32),
    },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode { path, source } => write!(f, "failed to load {path}: {source}"),
            Self::DimensionMismatch { a, b } => {
                write!(f, "dimension mismatch: {}x{} vs {}x{}", a.0, a.1, b.0, b.1)
            }
        }
    }
}

impl std::error::Error for DiffError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode { source, .. } => Some(source),
            Self::DimensionMismatch { .. } => None,
        }
    }
}

/// Pass/fail thresholds for a comparison.
///
/// The default is tuned for "same scene, independent render paths": Swath and
/// the GDAL/rio-tiler oracle agree on geometry and radiometry, but resampling
/// kernels and integer rounding may legitimately land neighbouring values a
/// step or two apart, concentrated along resample seams.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffPolicy {
    /// Maximum tolerated absolute difference per channel (out of 255) before a
    /// pixel counts as "bad".
    pub per_channel_tolerance: u8,
    /// Maximum tolerated fraction of bad pixels, in `0.0..=1.0`.
    pub max_bad_pixel_fraction: f64,
}

impl Default for DiffPolicy {
    /// Tolerance `2/255` per channel, at most `0.5%` bad pixels.
    ///
    /// Why: bilinear/cubic resampling and 8-bit rescale rounding differ by at
    /// most a couple of quantization steps between correct implementations, so
    /// `2` absorbs rounding jitter without hiding real value errors; seam
    /// pixels where kernels disagree more are a thin subset of a tile, so
    /// `0.5%` bounds them while still failing on any structured defect
    /// (wrong window, band swap, shifted geometry all blow far past it).
    fn default() -> Self {
        Self {
            per_channel_tolerance: 2,
            max_bad_pixel_fraction: 0.005,
        }
    }
}

/// The measured difference between two same-sized images.
///
/// The report is policy-independent: it carries a histogram of per-pixel
/// maximum channel differences, so any [`DiffPolicy`] can be evaluated against
/// it after the fact via [`DiffReport::passes`].
#[derive(Debug, Clone, PartialEq)]
pub struct DiffReport {
    /// Image width in pixels (identical for both inputs).
    pub width: u32,
    /// Image height in pixels (identical for both inputs).
    pub height: u32,
    /// Largest absolute difference observed on any channel of any pixel.
    pub max_abs_channel_diff: u8,
    /// Mean absolute difference over all channels of all pixels.
    pub mean_abs_diff: f64,
    /// `histogram[d]` = number of pixels whose maximum per-channel absolute
    /// difference is exactly `d`.
    pub pixel_max_diff_histogram: [u64; DIFF_BINS],
}

impl DiffReport {
    /// Total number of pixels compared.
    #[must_use]
    pub fn total_pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Number of pixels whose maximum per-channel difference exceeds
    /// `tolerance`.
    #[must_use]
    pub fn pixels_exceeding_tolerance(&self, tolerance: u8) -> u64 {
        self.pixel_max_diff_histogram[usize::from(tolerance) + 1..]
            .iter()
            .sum()
    }

    /// Fraction (`0.0..=1.0`) of pixels whose maximum per-channel difference
    /// exceeds `tolerance`. Zero-pixel images have no bad pixels.
    #[must_use]
    pub fn pct_pixels_exceeding_tolerance(&self, tolerance: u8) -> f64 {
        let total = self.total_pixels();
        if total == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)] // pixel counts are far below 2^52
        {
            self.pixels_exceeding_tolerance(tolerance) as f64 / total as f64
        }
    }

    /// Whether this comparison passes under `policy`.
    #[must_use]
    pub fn passes(&self, policy: &DiffPolicy) -> bool {
        self.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance)
            <= policy.max_bad_pixel_fraction
    }
}

impl fmt::Display for DiffReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "dimensions:            {}x{}", self.width, self.height)?;
        writeln!(f, "max |channel diff|:    {}", self.max_abs_channel_diff)?;
        write!(f, "mean |channel diff|:   {:.6}", self.mean_abs_diff)
    }
}

/// Load a PNG (or any decodable image) as RGBA8.
///
/// Everything is normalized to RGBA so grayscale/RGB oracle output and
/// RGBA Swath output compare channel-for-channel; missing alpha decodes as
/// fully opaque on both sides and therefore diffs as zero.
pub fn load_png(path: &Path) -> Result<RgbaImage, DiffError> {
    let img = image::open(path).map_err(|source| DiffError::Decode {
        path: path.display().to_string(),
        source,
    })?;
    Ok(img.into_rgba8())
}

/// Compare two images channel-by-channel (alpha included).
///
/// # Errors
///
/// Returns [`DiffError::DimensionMismatch`] when the images differ in size.
pub fn diff(a: &RgbaImage, b: &RgbaImage) -> Result<DiffReport, DiffError> {
    if a.dimensions() != b.dimensions() {
        return Err(DiffError::DimensionMismatch {
            a: a.dimensions(),
            b: b.dimensions(),
        });
    }
    let (width, height) = a.dimensions();
    let mut histogram = [0_u64; DIFF_BINS];
    let mut max_abs: u8 = 0;
    let mut sum_abs: u64 = 0;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let mut pixel_max: u8 = 0;
        for (ca, cb) in pa.0.iter().zip(pb.0.iter()) {
            let d = ca.abs_diff(*cb);
            pixel_max = pixel_max.max(d);
            sum_abs += u64::from(d);
        }
        max_abs = max_abs.max(pixel_max);
        histogram[usize::from(pixel_max)] += 1;
    }
    let channel_count = u64::from(width) * u64::from(height) * 4;
    #[allow(clippy::cast_precision_loss)] // channel counts are far below 2^52
    let mean_abs_diff = if channel_count == 0 {
        0.0
    } else {
        sum_abs as f64 / channel_count as f64
    };
    Ok(DiffReport {
        width,
        height,
        max_abs_channel_diff: max_abs,
        mean_abs_diff,
        pixel_max_diff_histogram: histogram,
    })
}

/// Asserts `ours` matches the committed golden PNG at `golden` under the
/// default policy, printing the diff metrics (the test's report) either
/// way. `label` names the case in the output. The tail every oracle and
/// served-tile golden test shared (#348).
#[allow(clippy::print_stdout, reason = "diff metrics are the test's report")]
pub fn assert_matches_golden(label: &str, ours: &RgbaImage, golden: &Path) {
    let reference =
        load_png(golden).unwrap_or_else(|err| panic!("golden {}: {err}", golden.display()));
    let report = diff(ours, &reference).expect("dimensions match");
    let policy = DiffPolicy::default();
    let bad_pct = report.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance) * 100.0;
    println!(
        "{label}: max |diff| {}, mean {:.4}, {bad_pct:.4}% pixels over tolerance {}",
        report.max_abs_channel_diff, report.mean_abs_diff, policy.per_channel_tolerance
    );
    assert!(
        report.passes(&policy),
        "{label}: fails default policy — max |diff| {}, {bad_pct:.4}% pixels over tolerance {}",
        report.max_abs_channel_diff,
        policy.per_channel_tolerance
    );
}

#[cfg(test)]
mod tests {
    use super::{DiffError, DiffPolicy, RgbaImage, diff};
    use image::Rgba;

    /// A deterministic gradient test image (no fixture files needed).
    fn gradient(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            #[allow(clippy::cast_possible_truncation)] // values folded into u8 range
            Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
        })
    }

    #[test]
    fn errors_render_and_chain_usefully() {
        use std::error::Error as _;

        let decode = super::load_png(std::path::Path::new("/nonexistent/oracle.png"))
            .expect_err("missing file");
        assert!(
            decode
                .to_string()
                .contains("failed to load /nonexistent/oracle.png")
        );
        assert!(decode.source().is_some());

        let mismatch = DiffError::DimensionMismatch {
            a: (256, 256),
            b: (256, 128),
        };
        assert_eq!(
            mismatch.to_string(),
            "dimension mismatch: 256x256 vs 256x128"
        );
        assert!(mismatch.source().is_none());
    }

    #[test]
    fn report_display_summarizes_the_comparison() {
        let a = gradient(4, 4);
        let report = diff(&a, &a.clone()).expect("same dimensions");
        let rendered = report.to_string();
        assert!(rendered.contains("dimensions:            4x4"));
        assert!(rendered.contains("max |channel diff|:    0"));
        assert!(rendered.contains("mean |channel diff|:   0.000000"));
    }

    #[test]
    fn zero_pixel_images_compare_as_equal() {
        let a = RgbaImage::new(0, 0);
        let report = diff(&a, &RgbaImage::new(0, 0)).expect("same dimensions");
        assert_eq!(report.total_pixels(), 0);
        assert!((report.mean_abs_diff - 0.0).abs() < f64::EPSILON);
        assert!((report.pct_pixels_exceeding_tolerance(0) - 0.0).abs() < f64::EPSILON);
        assert!(report.passes(&DiffPolicy::default()));
    }

    #[test]
    fn identical_images_pass_with_zero_tolerance() {
        let a = gradient(64, 48);
        let report = diff(&a, &a.clone()).expect("same dimensions");
        assert_eq!(report.max_abs_channel_diff, 0);
        assert!((report.mean_abs_diff - 0.0).abs() < f64::EPSILON);
        assert_eq!(report.pixels_exceeding_tolerance(0), 0);
        assert!(report.passes(&DiffPolicy {
            per_channel_tolerance: 0,
            max_bad_pixel_fraction: 0.0,
        }));
        assert!(report.passes(&DiffPolicy::default()));
    }

    #[test]
    fn seeded_single_pixel_error_is_detected_and_reported_precisely() {
        let a = gradient(64, 48);
        let mut b = a.clone();
        // Seed a one-pixel, one-channel error of exactly +5.
        let px = b.get_pixel_mut(17, 23);
        px.0[1] = px.0[1].wrapping_add(5);
        let report = diff(&a, &b).expect("same dimensions");
        assert_eq!(report.max_abs_channel_diff, 5);
        assert_eq!(report.pixels_exceeding_tolerance(0), 1);
        assert_eq!(report.pixels_exceeding_tolerance(4), 1);
        assert_eq!(report.pixels_exceeding_tolerance(5), 0);
        // One bad channel among 64*48*4.
        let expected_mean = 5.0 / (64.0 * 48.0 * 4.0);
        assert!((report.mean_abs_diff - expected_mean).abs() < 1e-12);
        // Zero tolerance, zero bad pixels allowed: must fail.
        assert!(!report.passes(&DiffPolicy {
            per_channel_tolerance: 0,
            max_bad_pixel_fraction: 0.0,
        }));
    }

    #[test]
    fn dimension_mismatch_is_a_hard_error() {
        let a = gradient(64, 48);
        let b = gradient(64, 49);
        let err = diff(&a, &b).expect_err("dimensions differ");
        match err {
            DiffError::DimensionMismatch { a, b } => {
                assert_eq!(a, (64, 48));
                assert_eq!(b, (64, 49));
            }
            DiffError::Decode { .. } => panic!("expected DimensionMismatch"),
        }
    }

    #[test]
    fn default_policy_boundary_diff_of_two_is_tolerated() {
        // Every pixel off by exactly the default tolerance (2): zero bad
        // pixels, so the default policy passes.
        let a = gradient(32, 32);
        let mut b = a.clone();
        for px in b.pixels_mut() {
            px.0[0] = px.0[0].saturating_add(2).max(2);
        }
        let report = diff(&a, &b).expect("same dimensions");
        assert_eq!(report.max_abs_channel_diff, 2);
        assert_eq!(report.pixels_exceeding_tolerance(2), 0);
        assert!(report.passes(&DiffPolicy::default()));
    }

    #[test]
    fn default_policy_bad_pixel_fraction_boundary() {
        // 32*32 = 1024 pixels; 0.5% of 1024 = 5.12, so 5 bad pixels pass and
        // 6 fail under the default policy.
        let a = gradient(32, 32);
        let mut b = a.clone();
        for x in 0..5 {
            b.get_pixel_mut(x, 0).0[2] ^= 0x40;
        }
        let report = diff(&a, &b).expect("same dimensions");
        assert_eq!(report.pixels_exceeding_tolerance(2), 5);
        assert!(report.passes(&DiffPolicy::default()));

        b.get_pixel_mut(5, 0).0[2] ^= 0x40;
        let report = diff(&a, &b).expect("same dimensions");
        assert_eq!(report.pixels_exceeding_tolerance(2), 6);
        assert!(!report.passes(&DiffPolicy::default()));
    }

    #[test]
    fn alpha_is_compared_like_any_channel() {
        let a = gradient(8, 8);
        let mut b = a.clone();
        b.get_pixel_mut(3, 3).0[3] = 0; // punch a hole in alpha only
        let report = diff(&a, &b).expect("same dimensions");
        assert_eq!(report.max_abs_channel_diff, 255);
        assert_eq!(report.pixels_exceeding_tolerance(254), 1);
    }
}
