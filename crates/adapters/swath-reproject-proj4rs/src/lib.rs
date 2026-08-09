// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `Reproject` adapter over pure-Rust [proj4rs](https://docs.rs/proj4rs).
//!
//! Projection math is **BIND, never build** (ADR 0002): this adapter wraps
//! proj4rs — a Rust port of proj.4 — behind the
//! [`Reproject`](swath_core::reproject::Reproject) port for the common CRSs
//! the HLS path needs, with one narrow, documented exception ([`sinu`],
//! below). It never implements general projection machinery itself.
//!
//! # Supported CRS set (hard boundary)
//!
//! EPSG-identified CRSs ([`Crs::Epsg`]):
//!
//! | EPSG | CRS |
//! |---|---|
//! | 4326 | WGS 84 geographic (degrees) |
//! | 3857 | WGS 84 / Pseudo-Mercator (meters) |
//! | 32601–32660 | WGS 84 / UTM zone 1N–60N (meters) |
//! | 32701–32760 | WGS 84 / UTM zone 1S–60S (meters) |
//!
//! Proj-string CRSs ([`Crs::Proj4`], new in #39 for the virtual-reference
//! serve path): the definition is handed to proj4rs verbatim; whatever
//! proj4rs resolves, this adapter serves (angular units converted at the
//! boundary via the compiled projection's own geographic-ness). One family
//! proj4rs 0.1.10 cannot resolve is implemented here under a measured
//! exception: **spherical sinusoidal** (`+proj=sinu +R=…`), the
//! MODIS/VIIRS grid of VNP09GA — see the [`sinu`] module docs for the full
//! justification and scope fence. Everything else — an EPSG code outside
//! the table, a proj string neither proj4rs nor [`sinu`] resolves — is
//! [`ReprojectError::UnknownCrs`], deliberately a hard error, not a guess.
//! The long tail remains the plan for a future PROJ C-binding adapter
//! (feature-gated, per ADR 0002); it will pass the same accuracy suite
//! this adapter does (`tests/common/`).
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
//! | sinusoidal → 4326 | 2.9e-14 ° (~3 nm ground) | 1e-12 ° |
//! | sinusoidal → 3857 | 3.8e-9 m | 1e-8 m |
//! | 4326/3857 → sinusoidal | exact (0 measured) | 1e-8 m |
//!
//! i.e. this adapter matches PROJ at the nanometer level on this CRS set —
//! double-precision noise, six orders of magnitude inside the millimeter
//! bar the tiler needs. (Sinusoidal numbers measured 2026-08 vs PROJ 9.5.1
//! at the VNP09GA h33v12 1-km grid corners/center — including the wrapped
//! antimeridian corners; see `tests/truth.rs`.)
//!
//! # Errors
//!
//! Out-of-domain input (|lat| > 90°, beyond Web Mercator's ±85.051129°
//! cutoff, or any input proj4rs rejects as out of range) returns
//! [`ReprojectError::OutOfDomain`]; it never panics. As a belt-and-braces
//! guard the adapter also rejects non-finite outputs, so NaN/∞ can never
//! leak through the port even if the underlying library produced one.

mod sinu;

use proj4rs::Proj;
use proj4rs::transform::transform;
use swath_core::crs::Crs;
use swath_core::reproject::{CoordTransform, Reproject, ReprojectError};

use crate::sinu::Sinu;

/// How a supported EPSG code presents at the proj4rs boundary.
struct CrsDef {
    /// proj.4 initialization string.
    proj_string: String,
    /// Whether coordinates are angular (proj4rs radians ↔ port degrees).
    geographic: bool,
}

/// The WGS 84 geographic definition — the hub CRS of sinusoidal bridging.
const LONGLAT_WGS84: &str = "+proj=longlat +datum=WGS84 +no_defs";

/// Resolves an EPSG code to its proj.4 definition, or `None` if outside the
/// supported set (see crate docs for the exact table).
fn epsg_def(epsg: u32) -> Option<CrsDef> {
    match epsg {
        4326 => Some(CrsDef {
            proj_string: LONGLAT_WGS84.to_owned(),
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

/// One resolved side of a transform: a compiled proj4rs projection, or the
/// in-crate spherical sinusoidal (the [`sinu`] exception).
#[allow(
    clippy::large_enum_variant,
    reason = "Ends are built once per transformer, never stored in bulk"
)]
enum End {
    /// proj4rs handles this CRS.
    Proj {
        /// The compiled projection.
        proj: Proj,
        /// Whether the port-facing coordinates are angular degrees.
        geographic: bool,
    },
    /// The spherical sinusoidal family proj4rs lacks.
    Sinu(Sinu),
}

/// Resolves a port CRS into an [`End`], or [`ReprojectError::UnknownCrs`].
fn resolve(crs: &Crs) -> Result<End, ReprojectError> {
    let unknown = || ReprojectError::UnknownCrs { crs: crs.clone() };
    match crs {
        Crs::Epsg(code) => {
            let def = epsg_def(*code).ok_or_else(unknown)?;
            // The table's strings are known-good; a parse failure here is a
            // library-level surprise, reported as such.
            let proj = Proj::from_proj_string(&def.proj_string).map_err(|e| {
                ReprojectError::Transform {
                    detail: format!("proj4rs rejected {:?}: {e}", def.proj_string),
                }
            })?;
            Ok(End::Proj {
                proj,
                geographic: def.geographic,
            })
        }
        Crs::Proj4(definition) => {
            if let Some(sinu) = Sinu::parse(definition) {
                return Ok(End::Sinu(sinu));
            }
            // Anything proj4rs itself resolves is served; a definition it
            // rejects is an unknown CRS (the honest boundary), not a
            // transform failure.
            let proj = Proj::from_proj_string(definition).map_err(|_| unknown())?;
            let geographic = proj.is_latlong();
            Ok(End::Proj { proj, geographic })
        }
    }
}

/// The proj4rs-backed [`Reproject`] implementation.
///
/// Stateless and trivially cheap to construct; the per-pair work happens in
/// [`transformer`](Reproject::transformer), which compiles both definitions
/// once so the returned [`CoordTransform`] is reusable across many points
/// and threads.
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
    fn transformer(&self, from: &Crs, to: &Crs) -> Result<Box<dyn CoordTransform>, ReprojectError> {
        let from_end = resolve(from)?;
        let to_end = resolve(to)?;
        if from == to {
            // Same-CRS pairs are exact pass-throughs: no library round
            // trip, no degree↔radian conversion noise.
            return Ok(Box::new(Identity));
        }
        match (from_end, to_end) {
            (
                End::Proj {
                    proj: from,
                    geographic: from_geographic,
                },
                End::Proj {
                    proj: to,
                    geographic: to_geographic,
                },
            ) => Ok(Box::new(Proj4rsTransform {
                from,
                to,
                from_geographic,
                to_geographic,
            })),
            // Any pair involving sinusoidal bridges through geographic
            // radians (see SinuBridge docs).
            (from_end, to_end) => {
                let latlong = Proj::from_proj_string(LONGLAT_WGS84).map_err(|e| {
                    ReprojectError::Transform {
                        detail: format!("proj4rs rejected {LONGLAT_WGS84:?}: {e}"),
                    }
                })?;
                Ok(Box::new(SinuBridge {
                    from: from_end,
                    to: to_end,
                    latlong,
                }))
            }
        }
    }
}

/// Exact pass-through for same-CRS pairs.
struct Identity;

impl CoordTransform for Identity {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        Ok((x, y))
    }
}

/// Maps a proj4rs error for the input point `(x, y)` (port units).
fn map_proj4rs_err(x: f64, y: f64, e: &proj4rs::errors::Error) -> ReprojectError {
    match e {
        proj4rs::errors::Error::CoordinateOutOfRange
        | proj4rs::errors::Error::LatitudeOutOfRange
        | proj4rs::errors::Error::LatOrLongExceedLimit
        | proj4rs::errors::Error::NanCoordinateValue
        | proj4rs::errors::Error::ForwardProjectionFailure
        | proj4rs::errors::Error::InverseProjectionFailure => ReprojectError::OutOfDomain { x, y },
        other => ReprojectError::Transform {
            detail: other.to_string(),
        },
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

impl CoordTransform for Proj4rsTransform {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        let (mut px, mut py) = (x, y);
        if self.from_geographic {
            px = px.to_radians();
            py = py.to_radians();
        }
        let mut point = (px, py, 0.0);
        transform(&self.from, &self.to, &mut point).map_err(|e| map_proj4rs_err(x, y, &e))?;
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
            map_proj4rs_err(f64::NAN, f64::NAN, &e)
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

/// A transform where at least one side is the sinusoidal exception:
/// bridges through geographic **radians** — sinusoidal sides use the
/// in-crate [`Sinu`] math, proj4rs sides run through
/// `transform(…, latlong)` / `transform(latlong, …)` exactly as a
/// two-proj4rs pair would.
///
/// Datum note, made explicit: the MODIS/VIIRS sinusoidal sphere carries no
/// datum shift (its graticule is treated as WGS84 longitude/latitude —
/// PROJ's own behavior for these proj-string pipelines), so the bridge's
/// geographic radians pass between the sides unshifted. The truth table
/// pins this agreement against real PROJ.
struct SinuBridge {
    from: End,
    to: End,
    latlong: Proj,
}

impl CoordTransform for SinuBridge {
    fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        // Input → geographic radians.
        let (lam, phi) = match &self.from {
            End::Sinu(sinu) => sinu.inverse(x, y)?,
            End::Proj { proj, geographic } => {
                let (px, py) = if *geographic {
                    (x.to_radians(), y.to_radians())
                } else {
                    (x, y)
                };
                let mut point = (px, py, 0.0);
                transform(proj, &self.latlong, &mut point)
                    .map_err(|e| map_proj4rs_err(x, y, &e))?;
                (point.0, point.1)
            }
        };
        // Geographic radians → output.
        let (ox, oy) = match &self.to {
            End::Sinu(sinu) => sinu.forward(lam, phi)?,
            End::Proj { proj, geographic } => {
                let mut point = (lam, phi, 0.0);
                transform(&self.latlong, proj, &mut point)
                    .map_err(|e| map_proj4rs_err(x, y, &e))?;
                if *geographic {
                    (point.0.to_degrees(), point.1.to_degrees())
                } else {
                    (point.0, point.1)
                }
            }
        };
        if !(ox.is_finite() && oy.is_finite()) {
            return Err(ReprojectError::OutOfDomain { x, y });
        }
        Ok((ox, oy))
    }
}

#[cfg(test)]
mod tests {
    use super::{Proj4rsReproject, epsg_def};
    use swath_core::crs::Crs;
    use swath_core::reproject::{Reproject, ReprojectError};

    const SINU: &str = "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R=6371007.181 +units=m +no_defs";

    #[test]
    fn utm_proj_strings_come_from_epsg_arithmetic() {
        let n = epsg_def(32613).unwrap();
        assert!(n.proj_string.contains("+zone=13"));
        assert!(!n.proj_string.contains("+south"));
        let s = epsg_def(32755).unwrap();
        assert!(s.proj_string.contains("+zone=55"));
        assert!(s.proj_string.contains("+south"));
    }

    #[test]
    fn unsupported_codes_are_rejected_at_resolution() {
        // Real EPSG codes outside the supported set must still hard-error:
        // NAD83 UTM (26913), a national grid (27700), and nonsense (0).
        for code in [26913_u32, 27700, 2154, 32600, 32661, 32700, 32761, 0] {
            let Err(err) = Proj4rsReproject::new().transformer(&Crs::from_epsg(code), &Crs::WGS84)
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
    fn unresolvable_proj_strings_are_unknown_crs() {
        for definition in [
            "+proj=nonexistent +R=1",
            "not a proj string at all",
            // Ellipsoidal sinusoidal: outside the sinu module's fence and
            // outside proj4rs — must refuse, never approximate.
            "+proj=sinu +ellps=WGS84 +units=m +no_defs",
        ] {
            let crs = Crs::from_proj4(definition);
            let Err(err) = Proj4rsReproject::new().transformer(&crs, &Crs::WGS84) else {
                panic!("{definition:?} unexpectedly resolved");
            };
            assert_eq!(err, ReprojectError::UnknownCrs { crs });
        }
    }

    #[test]
    fn proj_strings_proj4rs_resolves_are_served() {
        // Mollweide is in proj4rs's catalog: a proj-string CRS needing no
        // sinu exception must work end to end.
        let t = Proj4rsReproject::new()
            .transformer(&Crs::from_proj4("+proj=moll +R=6371007.181"), &Crs::WGS84)
            .expect("moll resolves via proj4rs");
        let (lon, lat) = t.transform(0.0, 0.0).unwrap();
        assert!(lon.abs() < 1e-9 && lat.abs() < 1e-9);
    }

    #[test]
    fn same_crs_pair_is_exact_identity() {
        let t = Proj4rsReproject::new()
            .transformer(&Crs::WGS84, &Crs::WGS84)
            .unwrap();
        assert_eq!(t.transform(-105.123, 39.456).unwrap(), (-105.123, 39.456));

        // Same proj-string pair too — including sinusoidal.
        let sinu = Crs::from_proj4(SINU);
        let t = Proj4rsReproject::new().transformer(&sinu, &sinu).unwrap();
        assert_eq!(
            t.transform(16_679_257.795, -3_335_851.559).unwrap(),
            (16_679_257.795, -3_335_851.559)
        );
    }
}
