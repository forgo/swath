// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Error taxonomy for the warp/resample kernels.

use std::fmt;

/// What can go wrong computing a source window or warping a buffer.
///
/// Per-point reprojection failures are **not** errors: a boundary or pixel
/// whose coordinates fall outside the transform's domain is simply excluded
/// (window computation) or left invalid (warp), matching how GDAL's warper
/// treats untransformable points. Only structural problems — inputs that can
/// never produce a correct result — surface here.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WarpError {
    /// The source geotransform's linear part is singular (zero determinant),
    /// so source CRS coordinates cannot be mapped back to pixels.
    NonInvertibleTransform {
        /// The offending determinant (always zero today; carried for the
        /// error message).
        determinant: f64,
    },

    /// The source buffer's sample count disagrees with its declared window
    /// (`window.width * window.height`), so indexing into it would be
    /// meaningless.
    SourceShape {
        /// `window.width * window.height` of the declared window.
        expected: u64,
        /// Samples actually present in the buffer.
        actual: u64,
    },
}

impl fmt::Display for WarpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonInvertibleTransform { determinant } => write!(
                f,
                "source geotransform is not invertible (determinant {determinant})"
            ),
            Self::SourceShape { expected, actual } => write!(
                f,
                "source buffer holds {actual} samples but its window declares {expected}"
            ),
        }
    }
}

impl std::error::Error for WarpError {}
