// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Spherical sinusoidal projection — the narrow, measured exception to
//! "projection math is BIND, never build" (ADR 0002).
//!
//! # Why this module exists (documented deviation)
//!
//! The MODIS/VIIRS gridded products (VNP09GA, ADR 0008) live on the
//! MODIS-heritage sinusoidal grid: spherical sinusoidal on a sphere of
//! radius 6 371 007.181 m, **no EPSG code**, conventionally named by its
//! proj string (`+proj=sinu +R=6371007.181 …`). proj4rs 0.1.10 does **not**
//! implement `sinu` (`Proj::from_proj_string` returns "Projection not
//! found" — verified, not assumed), and ADR 0002's designated fallback for
//! the long tail — PROJ C-bindings — is a deliberately separate adapter
//! with its own build/supply-chain consequences, not something to smuggle
//! in as a side effect of #39.
//!
//! The spherical sinusoidal mapping is two lines of arithmetic
//! (`x = R·λ·cosφ`, `y = R·φ` — PROJ's own `PJ_sinu.c` spherical branch),
//! and this module implements exactly that case and nothing more: sphere
//! only (`+R`, or `+a`=`+b`), unit meters, and it is validated
//! point-for-point against real PROJ ground truth at the VNP09GA grid
//! corners/centers (`tests/data/reproject_truth.json`, same
//! measured-accuracy regime as every other pair this adapter serves).
//! Anything outside that envelope — an ellipsoidal sinusoidal, exotic
//! parameters — is refused as `UnknownCrs`, never approximated. If/when
//! proj4rs grows `sinu` or the PROJ C-binding adapter lands, this module
//! is deleted and the truth table keeps the replacement honest.

use crate::ReprojectError;

/// Half pi, the latitude domain bound.
const HALF_PI: f64 = core::f64::consts::FRAC_PI_2;

/// A compiled spherical sinusoidal projection (parameters in meters /
/// radians).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sinu {
    /// Sphere radius, meters.
    r: f64,
    /// Central meridian, radians.
    lon_0: f64,
    /// False easting, meters.
    x_0: f64,
    /// False northing, meters.
    y_0: f64,
}

impl Sinu {
    /// Parses a proj string if — and only if — it is a spherical
    /// sinusoidal definition this module fully understands (module docs).
    /// Returns `None` for anything else, including non-`sinu` strings.
    #[allow(
        clippy::many_single_char_names,
        reason = "r/a/b are the proj parameter names themselves"
    )]
    pub(crate) fn parse(definition: &str) -> Option<Self> {
        let mut proj = None;
        let (mut r, mut a, mut b) = (None, None, None);
        let (mut lon_0, mut x_0, mut y_0) = (0.0, 0.0, 0.0);
        for token in definition.split_whitespace() {
            let token = token.strip_prefix('+')?;
            let (key, value) = match token.split_once('=') {
                Some((k, v)) => (k, Some(v)),
                None => (token, None),
            };
            let parsed = |v: Option<&str>| v.and_then(|v| v.parse::<f64>().ok());
            match key {
                "proj" => proj = value.map(str::to_owned),
                "R" => r = parsed(value),
                "a" => a = parsed(value),
                "b" => b = parsed(value),
                "lon_0" => lon_0 = parsed(value)?.to_radians(),
                "x_0" => x_0 = parsed(value)?,
                "y_0" => y_0 = parsed(value)?,
                // Inert decorations the MODIS/VIIRS strings carry.
                "units" => {
                    if value != Some("m") {
                        return None;
                    }
                }
                "no_defs" | "wktext" | "type" => {}
                "nadgrids" => {
                    if value != Some("@null") {
                        return None;
                    }
                }
                // Any other parameter means this is not the narrow case.
                _ => return None,
            }
        }
        if proj.as_deref() != Some("sinu") {
            return None;
        }
        // Sphere only: +R, or +a with +b textually equal to it (exact
        // float equality is the point: any a≠b is an ellipsoid, refused).
        #[allow(clippy::float_cmp, reason = "a sphere means bit-equal semi-axes")]
        let radius = match (r, a, b) {
            (Some(r), None, None) => r,
            (None, Some(a), Some(b)) if a == b => a,
            _ => return None,
        };
        (radius.is_finite() && radius > 0.0).then_some(Self {
            r: radius,
            lon_0,
            x_0,
            y_0,
        })
    }

    /// Geographic (radians) → projected (meters).
    pub(crate) fn forward(&self, lam: f64, phi: f64) -> Result<(f64, f64), ReprojectError> {
        if !(lam.is_finite() && phi.is_finite()) || phi.abs() > HALF_PI {
            return Err(ReprojectError::OutOfDomain {
                x: lam.to_degrees(),
                y: phi.to_degrees(),
            });
        }
        Ok((
            self.r.mul_add((lam - self.lon_0) * phi.cos(), self.x_0),
            self.r.mul_add(phi, self.y_0),
        ))
    }

    /// Projected (meters) → geographic (radians).
    pub(crate) fn inverse(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
        if !(x.is_finite() && y.is_finite()) {
            return Err(ReprojectError::OutOfDomain { x, y });
        }
        let phi = (y - self.y_0) / self.r;
        if phi.abs() > HALF_PI {
            return Err(ReprojectError::OutOfDomain { x, y });
        }
        // At the poles the parallel degenerates to a point: λ is defined
        // as the central meridian (PROJ's spherical inverse convention).
        let cos_phi = phi.cos();
        let mut lam = if cos_phi == 0.0 {
            self.lon_0
        } else {
            self.lon_0 + (x - self.x_0) / (self.r * cos_phi)
        };
        if !lam.is_finite() {
            return Err(ReprojectError::OutOfDomain { x, y });
        }
        // Wrap into [-π, π] exactly as PROJ's adjlon does: the MODIS/VIIRS
        // grid runs past the antimeridian (h33v12's east edge inverts to
        // ~-175°), and the truth table pins PROJ's wrapped answers.
        let two_pi = 2.0 * core::f64::consts::PI;
        if lam.abs() > core::f64::consts::PI {
            lam -= two_pi * ((lam + core::f64::consts::PI) / two_pi).floor();
        }
        Ok((lam, phi))
    }
}

#[cfg(test)]
mod tests {
    use super::Sinu;

    const VNP09GA: &str = "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R=6371007.181 +units=m +no_defs";

    #[test]
    #[allow(clippy::float_cmp, reason = "parsed constants compare exactly")]
    fn parses_the_modis_viirs_family_only() {
        let sinu = Sinu::parse(VNP09GA).expect("the VNP09GA string parses");
        assert_eq!(sinu.r, 6_371_007.181);
        assert_eq!((sinu.lon_0, sinu.x_0, sinu.y_0), (0.0, 0.0, 0.0));
        // +a/+b sphere spelling is the same case.
        assert!(Sinu::parse("+proj=sinu +a=6371007.181 +b=6371007.181").is_some());
        // Not sinu, ellipsoidal, or exotic: refused, never approximated.
        assert!(Sinu::parse("+proj=moll +R=6371007.181").is_none());
        assert!(Sinu::parse("+proj=sinu +ellps=WGS84").is_none());
        assert!(Sinu::parse("+proj=sinu +a=6378137 +b=6356752.3").is_none());
        assert!(Sinu::parse("+proj=sinu +R=6371007.181 +units=us-ft").is_none());
    }

    #[test]
    fn forward_and_inverse_round_trip() {
        let sinu = Sinu::parse(VNP09GA).unwrap();
        let (lam, phi) = (2.5_f64.to_radians(), -31.2_f64.to_radians());
        let (x, y) = sinu.forward(lam, phi).unwrap();
        let (lam2, phi2) = sinu.inverse(x, y).unwrap();
        assert!((lam - lam2).abs() < 1e-15 && (phi - phi2).abs() < 1e-15);
    }

    #[test]
    fn out_of_domain_is_an_error_never_nan() {
        let sinu = Sinu::parse(VNP09GA).unwrap();
        assert!(sinu.forward(0.0, 2.0).is_err()); // |φ| > π/2
        assert!(sinu.inverse(0.0, 2.0e13).is_err()); // beyond the pole
        assert!(sinu.forward(f64::NAN, 0.0).is_err());
    }
}
