// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Error taxonomy for the warp/resample kernels.

/// What can go wrong computing a source window or warping a buffer.
///
/// Per-point reprojection failures are **not** errors: a boundary or pixel
/// whose coordinates fall outside the transform's domain is simply excluded
/// (window computation) or left invalid (warp), matching how GDAL's warper
/// treats untransformable points. Only structural problems — inputs that can
/// never produce a correct result — surface here.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The source geotransform's linear part is singular (zero determinant),
    /// so source CRS coordinates cannot be mapped back to pixels.
    #[error("source geotransform is not invertible (determinant {determinant})")]
    NonInvertibleTransform {
        /// The offending determinant (always zero today; carried for the
        /// error message).
        determinant: f64,
    },

    /// The source pixel buffer is a variant these kernels do not know
    /// (`PixelBuffer` is `#[non_exhaustive]`; a new dtype must be adopted
    /// here explicitly, never silently sampled as garbage).
    #[error("unsupported source dtype {dtype:?}")]
    UnsupportedDtype {
        /// The unrecognized sample type.
        dtype: swath_core::raster::DType,
    },

    /// The source buffer's sample count disagrees with its declared window
    /// (`window.width * window.height`), so indexing into it would be
    /// meaningless.
    #[error("source buffer holds {actual} samples but its window declares {expected}")]
    SourceShape {
        /// `window.width * window.height` of the declared window.
        expected: u64,
        /// Samples actually present in the pixel buffer.
        actual: u64,
    },
}
