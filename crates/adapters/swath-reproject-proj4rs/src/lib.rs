// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `Reproject` adapter over pure-Rust [proj4rs](https://docs.rs/proj4rs).
//!
//! Projection math is **BIND, never build** (ADR 0002): this adapter wraps
//! proj4rs — a Rust port of proj.4 — behind the
//! [`Reproject`](swath_core::reproject::Reproject) port for the common CRSs
//! the HLS path needs. It never implements projection formulas itself.
//!
//! # Supported CRS set (hard boundary)
//!
//! | EPSG | CRS |
//! |---|---|
//! | 4326 | WGS 84 geographic (degrees) |
//! | 3857 | WGS 84 / Pseudo-Mercator (meters) |
//! | 32601–32660 | WGS 84 / UTM zone 1N–60N (meters) |
//! | 32701–32760 | WGS 84 / UTM zone 1S–60S (meters) |
//!
//! Anything else is [`ReprojectError::UnknownCrs`] — deliberately a hard
//! error, not a guess. The long tail of EPSG codes is the plan for a future
//! PROJ C-binding adapter (feature-gated, per ADR 0002); it will pass the
//! same accuracy suite this adapter does (`tests/common/`).
//!
//! # Units at the boundary (radians vs degrees)
//!
//! proj4rs speaks **radians** for geographic CRSs. The port contract
//! (swath-core `reproject` module docs) is **degrees** for geographic and
//! meters for projected, x=lon/easting y=lat/northing. This adapter owns
//! that conversion: degrees→radians on the way into proj4rs when the source
//! CRS is geographic, radians→degrees on the way out when the target is.
//! Projected CRSs are meters on both sides, passed through untouched.
//!
//! # Accuracy (measured, not aspirational)
//!
//! Agreement with PROJ (via pinned pyproj; truth table committed at
//! `tests/data/reproject_truth.json`) on the truth points, worst case per
//! pair — asserted in `tests/truth.rs` at just above measured + margin:
//!
//! | pair | measured max deviation | asserted tolerance |
//! |---|---|---|
//! | 4326 → 3857 | 3.8e-9 m | 1e-8 m |
//! | 3857 → 4326 | 2.9e-14 ° (~3 nm ground) | 1e-12 ° |
//! | UTM → 4326 | 1.5e-14 ° (~2 nm ground) | 1e-12 ° |
//! | 4326 → UTM | 2.5e-9 m | 1e-8 m |
//! | UTM ↔ 3857 | 1.4e-9 m | 1e-8 m |
//!
//! i.e. proj4rs matches PROJ at the nanometer level on this CRS set —
//! double-precision noise, six orders of magnitude inside the millimeter
//! bar the tiler needs.
//!
//! # Errors
//!
//! Out-of-domain input (|lat| > 90°, beyond Web Mercator's ±85.051129°
//! cutoff, or any input proj4rs rejects as out of range) returns
//! [`ReprojectError::OutOfDomain`]; it never panics. As a belt-and-braces
//! guard the adapter also rejects non-finite outputs, so NaN/∞ can never
//! leak through the port even if the underlying library produced one.

use proj4rs::Proj;
use proj4rs::transform::transform;
use swath_core::crs::Crs;
use swath_core::reproject::{CoordTransform, Reproject, ReprojectError};

/// How a supported CRS presents at the proj4rs boundary.
struct CrsDef {
    /// proj.4 initialization string.
    proj_string: String,
    /// Whether coordinates are angular (proj4rs radians ↔ port degrees).
    geographic: bool,
}

/// Resolves an EPSG code to its proj.4 definition, or `None` if outside the
/// supported set (see crate docs for the exact table).
fn crs_def(crs: Crs) -> Option<CrsDef> {
    let epsg = crs.epsg();
    match epsg {
        4326 => Some(CrsDef {
            proj_string: "+proj=longlat +datum=WGS84 +no_defs".to_owned(),
            geographic: true,
        }),
        // The canonical EPSG:3857 definition: spherical Mercator on the
        // WGS84 semi-major axis, with +nadgrids=@null suppressing any
        // datum shift (the sphere is *declared* to be WGS84).
        3857 => Some(CrsDef {
            proj_string: "+proj=merc +a=6378137 +b=6378137 +lat_ts=0 +lon_0=0 \
                          +x_0=0 +y_0=0 +k=1 +units=m +nadgrids=@null +no_defs"
                .to_owned(),
            geographic: false,
        }),
        // WGS 84 / UTM: zone number is EPSG-code arithmetic — 326xx north,
        // 327xx south.
        32601..=32660 => Some(CrsDef {
            proj_string: format!(
                "+proj=utm +zone={zone} +datum=WGS84 +units=m +no_defs",
                zone = epsg - 32600
            ),
            geographic: false,
        }),
        32701..=32760 => Some(CrsDef {
            proj_string: format!(
                "+proj=utm +zone={zone} +south +datum=WGS84 +units=m +no_defs",
                zone = epsg - 32700
            ),
            geographic: false,
        }),
        _ => None,
    }
}

/// The proj4rs-backed [`Reproject`] implementation.
///
/// Stateless and trivially cheap to construct; the per-pair work happens in
/// [`transformer`](Reproject::transformer), which compiles both proj.4
/// definitions once so the returned [`CoordTransform`] is reusable across
/// many points and threads.
#[derive(Debug, Default, Clone, Copy)]
pub struct Proj4rsReproject;

impl Proj4rsReproject {
    /// Creates the adapter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Reproject for Proj4rsReproject {
    fn transformer(&self, from: Crs, to: Crs) -> Result<Box<dyn CoordTransform>, ReprojectError> {
        let from_def = crs_def(from).ok_or(ReprojectError::UnknownCrs { crs: from })?;
        let to_def = crs_def(to).ok_or(ReprojectError::UnknownCrs { crs: to })?;
        if from == to {
            // Same-CRS pairs are exact pass-throughs: no proj4rs round trip,
            // no degree↔radian conversion noise.
            return Ok(Box::new(Identity));
        }
        let parse = |def: &CrsDef| {
            Proj::from_proj_string(&def.proj_string).map_err(|e| ReprojectError::Transform {
                detail: format!("proj4rs rejected {:?}: {e}", def.proj_string),
            })
        };
        Ok(Box::new(Proj4rsTransform {
            from: parse(&from_def)?,
            to: parse(&to_def)?,
            from_geographic: from_def.geographic,
            to_geographic: to_def.geographic,
        }))
    }
}

/// Exact pass-through for same-CRS pairs.
struct Identity;

impl CoordTransform for Identity {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        Ok((x, y))
    }
}

/// A compiled proj4rs transform pair, with degree↔radian conversion at the
/// boundary (see crate docs).
struct Proj4rsTransform {
    from: Proj,
    to: Proj,
    from_geographic: bool,
    to_geographic: bool,
}

impl Proj4rsTransform {
    /// Maps a proj4rs error for the input point `(x, y)` (port units).
    fn map_err(x: f64, y: f64, e: &proj4rs::errors::Error) -> ReprojectError {
        match e {
            proj4rs::errors::Error::CoordinateOutOfRange
            | proj4rs::errors::Error::LatitudeOutOfRange
            | proj4rs::errors::Error::LatOrLongExceedLimit
            | proj4rs::errors::Error::NanCoordinateValue
            | proj4rs::errors::Error::ForwardProjectionFailure
            | proj4rs::errors::Error::InverseProjectionFailure => {
                ReprojectError::OutOfDomain { x, y }
            }
            other => ReprojectError::Transform {
                detail: other.to_string(),
            },
        }
    }
}

impl CoordTransform for Proj4rsTransform {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        let (mut px, mut py) = (x, y);
        if self.from_geographic {
            px = px.to_radians();
            py = py.to_radians();
        }
        let mut point = (px, py, 0.0);
        transform(&self.from, &self.to, &mut point).map_err(|e| Self::map_err(x, y, &e))?;
        let (mut ox, mut oy) = (point.0, point.1);
        if self.to_geographic {
            ox = ox.to_degrees();
            oy = oy.to_degrees();
        }
        if !(ox.is_finite() && oy.is_finite()) {
            return Err(ReprojectError::OutOfDomain { x, y });
        }
        Ok((ox, oy))
    }

    fn transform_slice(&self, points: &mut [(f64, f64)]) -> Result<(), ReprojectError> {
        // Bulk path: proj4rs's Transform impl for &mut [(f64, f64, f64)]
        // iterates inside the library, amortizing per-call setup. Unit
        // conversion happens in the same passes that build/unpack the
        // triple buffer. Observable behavior is identical to the default
        // per-point loop (asserted by the property suite).
        let mut buf: Vec<(f64, f64, f64)> = points
            .iter()
            .map(|&(x, y)| {
                if self.from_geographic {
                    (x.to_radians(), y.to_radians(), 0.0)
                } else {
                    (x, y, 0.0)
                }
            })
            .collect();
        transform(&self.from, &self.to, buf.as_mut_slice()).map_err(|e| {
            // proj4rs stops at the first bad point but does not say which;
            // find it with the per-point path so the error names the point.
            for &(x, y) in points.iter() {
                if let Err(err) = self.transform(x, y) {
                    return err;
                }
            }
            Self::map_err(f64::NAN, f64::NAN, &e)
        })?;
        for (out, &(bx, by, _)) in points.iter_mut().zip(&buf) {
            let (ox, oy) = if self.to_geographic {
                (bx.to_degrees(), by.to_degrees())
            } else {
                (bx, by)
            };
            if !(ox.is_finite() && oy.is_finite()) {
                return Err(ReprojectError::OutOfDomain { x: out.0, y: out.1 });
            }
            *out = (ox, oy);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Proj4rsReproject, crs_def};
    use swath_core::crs::Crs;
    use swath_core::reproject::{Reproject, ReprojectError};

    #[test]
    fn utm_proj_strings_come_from_epsg_arithmetic() {
        let n = crs_def(Crs::from_epsg(32613)).unwrap();
        assert!(n.proj_string.contains("+zone=13"));
        assert!(!n.proj_string.contains("+south"));
        let s = crs_def(Crs::from_epsg(32755)).unwrap();
        assert!(s.proj_string.contains("+zone=55"));
        assert!(s.proj_string.contains("+south"));
    }

    #[test]
    fn unsupported_codes_are_rejected_at_resolution() {
        // Real EPSG codes outside the supported set must still hard-error:
        // NAD83 UTM (26913), a national grid (27700), and nonsense (0).
        for code in [26913_u32, 27700, 2154, 32600, 32661, 32700, 32761, 0] {
            let Err(err) = Proj4rsReproject::new().transformer(Crs::from_epsg(code), Crs::WGS84)
            else {
                panic!("EPSG:{code} unexpectedly resolved");
            };
            assert_eq!(
                err,
                ReprojectError::UnknownCrs {
                    crs: Crs::from_epsg(code)
                },
                "EPSG:{code} must be UnknownCrs"
            );
        }
    }

    #[test]
    fn same_crs_pair_is_exact_identity() {
        let t = Proj4rsReproject::new()
            .transformer(Crs::WGS84, Crs::WGS84)
            .unwrap();
        assert_eq!(t.transform(-105.123, 39.456).unwrap(), (-105.123, 39.456));
    }
}
