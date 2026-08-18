// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The point-transform port the kernel consumes.
//!
//! Projection math never lives in this crate: implement [`CoordTransform`]
//! over proj4rs, PROJ bindings, or any other projection library, and the
//! kernel drives it. Every coordinate crossing the trait is in the CRS's
//! **native units** (degrees for geographic CRSs, meters for projected
//! ones), axis order **x = longitude / easting, y = latitude / northing** —
//! the GIS-traditional order, not the EPSG-official one.

use std::fmt;

/// Why a point could not be transformed.
///
/// The kernel never inspects the reason — a failed point is excluded
/// (window computation) or invalid (warp), exactly as GDAL's warper treats
/// untransformable points — but implementors report it honestly so the
/// distinction stays visible to other consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransformError {
    /// The input point lies outside the mathematical domain of the
    /// transform (e.g. |lat| > 90°, or beyond a projection's validity
    /// cutoff).
    OutOfDomain,
    /// Any other failure inside the projection library (numerical
    /// non-convergence, malformed definition, …).
    Failed,
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfDomain => f.write_str("point is outside the transform's domain"),
            Self::Failed => f.write_str("projection failure"),
        }
    }
}

impl std::error::Error for TransformError {}

/// A compiled point transform from the target CRS to the source CRS.
///
/// Implementations must return errors for untransformable input — never
/// panic, and never silently produce NaN/∞.
pub trait CoordTransform {
    /// Transforms a single point from the target CRS to the source CRS.
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), TransformError>;

    /// Transforms a batch of `(x, y)` points **in place**.
    ///
    /// The default implementation loops over [`transform`](Self::transform)
    /// point by point; implementations whose underlying library has a bulk
    /// path should override it. Overrides must be observably identical to
    /// the per-point loop on success.
    ///
    /// On error, the slice's contents are **unspecified**: some prefix may
    /// already be transformed. The warp kernel handles this by redoing a
    /// failed batch per point; other callers that need all-or-nothing
    /// semantics keep their own copy.
    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), TransformError> {
        for p in points.iter_mut() {
            *p = self.transform(p.0, p.1)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CoordTransform, TransformError};

    /// Axis swap, rejecting negative x — exercises the default batch impl
    /// and dyn-compatibility, not projection math.
    struct Swap;

    impl CoordTransform for Swap {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), TransformError> {
            if x < 0.0 {
                return Err(TransformError::OutOfDomain);
            }
            Ok((y, x))
        }
    }

    #[test]
    fn default_batch_matches_per_point() {
        let t: &dyn CoordTransform = &Swap;
        let mut batch = [(1.0, 2.0), (3.0, 4.0)];
        t.transform_slice(&mut batch).unwrap();
        assert_eq!(batch, [(2.0, 1.0), (4.0, 3.0)]);
    }

    #[test]
    fn default_batch_fails_fast_on_bad_point() {
        let mut batch = [(1.0, 2.0), (-1.0, 4.0)];
        let err = Swap.transform_slice(&mut batch).unwrap_err();
        assert_eq!(err, TransformError::OutOfDomain);
    }
}
