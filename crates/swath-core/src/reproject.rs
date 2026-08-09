// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `Reproject` port: coordinate transforms between CRSs.
//!
//! Kept deliberately minimal per ARCHITECTURE.md §6: this port hands out
//! point transforms and nothing more — warp/resample kernels live in the
//! core, not behind this boundary (where exactly warp lives is still open,
//! ARCHITECTURE.md §16.2; this port does not resolve that question).
//! Projection math is **BIND, never build** (ADR 0002): adapters wrap
//! pure-Rust `proj4rs` for the common CRSs, with PROJ C-bindings
//! feature-gated later for the long tail. Swath never reimplements
//! projection math.
//!
//! Unlike [`RasterSource`](crate::source::RasterSource), these traits are
//! synchronous: projection is pure math with no I/O to await. They are also
//! **dyn-compatible** by design — [`Reproject::transformer`] returns
//! `Box<dyn CoordTransform>`, so the tiler can hold transforms without being
//! generic over the adapter.
//!
//! # Units and axis order (READ THIS — the classic footgun)
//!
//! Every coordinate crossing this port is in the CRS's **native units**:
//!
//! * geographic CRSs (e.g. EPSG:4326): **degrees**;
//! * projected CRSs (e.g. EPSG:3857, UTM): **meters** (easting/northing).
//!
//! Axis order is always **x = longitude / easting, y = latitude /
//! northing** — the GIS-traditional order, **not** the EPSG-official axis
//! order (which puts latitude first for EPSG:4326). If an underlying
//! library speaks radians (proj4rs does, for geographic CRSs) or official
//! axis order (pyproj without `always_xy=True` does), the **adapter**
//! converts at its boundary; port callers never see anything but
//! degrees/meters in x=lon/easting, y=lat/northing order.
//!
//! # Errors
//!
//! Two failure classes, distinguished so callers can react differently:
//! a CRS the adapter cannot resolve ([`ReprojectError::UnknownCrs`] — fail
//! the layer, suggest the long-tail adapter) versus a point outside the
//! transform's mathematical domain ([`ReprojectError::OutOfDomain`] — e.g.
//! latitude beyond ±90°, or beyond Web Mercator's ±85.051129° cutoff; skip
//! or clamp the point). Adapters must return errors for out-of-domain
//! input, never panic and never silently produce NaN/∞.

use crate::crs::Crs;

/// What can go wrong resolving a CRS pair or transforming a point.
///
/// The port's error contract, defined in the core so consumers match on
/// semantics rather than adapter internals (same pattern as
/// [`SourceError`](crate::source::SourceError)).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ReprojectError {
    /// The adapter cannot resolve this CRS (outside its supported set, or
    /// not a CRS the underlying library knows). The set each adapter
    /// supports is documented on the adapter; codes beyond it are this
    /// error, not a guess.
    #[error("unknown or unsupported CRS {crs}")]
    UnknownCrs {
        /// The CRS that could not be resolved.
        crs: Crs,
    },

    /// The input point lies outside the mathematical domain of the
    /// transform (e.g. |lat| > 90°, or beyond a projection's validity
    /// cutoff). The coordinates echo the offending **input** point, in the
    /// source CRS's native units.
    #[error("point ({x}, {y}) is outside the transform's domain")]
    OutOfDomain {
        /// x (longitude or easting) of the rejected input point.
        x: f64,
        /// y (latitude or northing) of the rejected input point.
        y: f64,
    },

    /// Any other failure inside the projection library (numerical
    /// non-convergence, malformed internal definition, …).
    #[error("projection failure: {detail}")]
    Transform {
        /// Adapter-provided description of the underlying failure.
        detail: String,
    },
}

/// A compiled transform from one CRS to another.
///
/// Obtained from [`Reproject::transformer`]; immutable and safe to share
/// across threads (`Send + Sync`), so one transform can serve many
/// concurrent tile renders.
///
/// Units and axis order follow the [module contract](self): CRS-native
/// units, x = lon/easting, y = lat/northing.
pub trait CoordTransform: Send + Sync {
    /// Transforms a single point from the source CRS to the target CRS.
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError>;

    /// Transforms a batch of `(x, y)` points **in place**.
    ///
    /// The default implementation loops over [`transform`](Self::transform)
    /// point by point; adapters whose underlying library has a bulk path
    /// should override it. Overrides must be observably identical to the
    /// per-point loop on success (the adapter test suites assert this).
    ///
    /// On error, the slice's contents are **unspecified**: some prefix may
    /// already be transformed. Callers that need all-or-nothing semantics
    /// keep their own copy.
    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), ReprojectError> {
        for p in points.iter_mut() {
            *p = self.transform(p.0, p.1)?;
        }
        Ok(())
    }
}

/// Coordinate transforms between CRSs (ARCHITECTURE.md §6).
///
/// The factory half of the port: resolves a `(from, to)` CRS pair into a
/// reusable [`CoordTransform`]. Resolution is where "do I know this CRS?"
/// is answered — [`ReprojectError::UnknownCrs`] surfaces here, so per-point
/// calls on the returned transform only ever fail on the point itself.
///
/// Object-safe on purpose: the tiler can hold a `&dyn Reproject` (or
/// `Box<dyn Reproject>`) and the accuracy test suite runs identically
/// against any adapter behind the same `&dyn Reproject`.
pub trait Reproject: Send + Sync {
    /// Builds a transform from `from` to `to`.
    ///
    /// Returns [`ReprojectError::UnknownCrs`] if either CRS is outside the
    /// adapter's supported set. A same-CRS pair is valid and yields an
    /// identity transform.
    fn transformer(&self, from: &Crs, to: &Crs) -> Result<Box<dyn CoordTransform>, ReprojectError>;
}

#[cfg(test)]
mod tests {
    use super::{CoordTransform, Reproject, ReprojectError};
    use crate::crs::Crs;

    /// Minimal in-crate impl: axis swap, rejecting negative x. Exists to
    /// exercise the default batch impl and dyn-compatibility, not to do
    /// projection math (that is BIND — adapters only).
    struct Swap;

    impl CoordTransform for Swap {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            if x < 0.0 {
                return Err(ReprojectError::OutOfDomain { x, y });
            }
            Ok((y, x))
        }
    }

    struct SwapFactory;

    impl Reproject for SwapFactory {
        fn transformer(
            &self,
            from: &Crs,
            _to: &Crs,
        ) -> Result<Box<dyn CoordTransform>, ReprojectError> {
            if from.epsg() == Some(0) {
                return Err(ReprojectError::UnknownCrs { crs: from.clone() });
            }
            Ok(Box::new(Swap))
        }
    }

    #[test]
    fn traits_are_dyn_compatible() {
        let f: &dyn Reproject = &SwapFactory;
        let t = f.transformer(&Crs::WGS84, &Crs::WEB_MERCATOR).unwrap();
        assert_eq!(t.transform(1.0, 2.0).unwrap(), (2.0, 1.0));
    }

    #[test]
    fn default_batch_matches_per_point() {
        let t: Box<dyn CoordTransform> = Box::new(Swap);
        let mut batch = [(1.0, 2.0), (3.0, 4.0)];
        t.transform_slice(&mut batch).unwrap();
        assert_eq!(batch, [(2.0, 1.0), (4.0, 3.0)]);
    }

    #[test]
    fn default_batch_fails_fast_on_bad_point() {
        let t = Swap;
        let mut batch = [(1.0, 2.0), (-1.0, 4.0)];
        let err = t.transform_slice(&mut batch).unwrap_err();
        assert_eq!(err, ReprojectError::OutOfDomain { x: -1.0, y: 4.0 });
    }

    #[test]
    fn unknown_crs_surfaces_at_resolution() {
        let Err(err) = SwapFactory.transformer(&Crs::from_epsg(0), &Crs::WGS84) else {
            panic!("EPSG:0 unexpectedly resolved");
        };
        assert_eq!(
            err,
            ReprojectError::UnknownCrs {
                crs: Crs::from_epsg(0)
            }
        );
    }
}
