// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Coordinate reference system identity.
//!
//! [`Crs`] is vocabulary only: an EPSG code the rest of the domain passes
//! around (raster metadata, the [`Trace`](crate::trace::Trace) `crs_from` /
//! `crs_to` fields, the future `Reproject` port). Projection **math** is
//! deliberately absent — per ARCHITECTURE.md §6 that lives behind the
//! `Reproject` port, implemented by adapter crates (proj4rs first).

use std::fmt;

/// A coordinate reference system, identified by its EPSG code.
///
/// Serializes as the bare EPSG code (e.g. `3857`), matching the transparent
/// newtype representation; `Display` renders the conventional URN-ish form
/// `"EPSG:3857"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Crs(u32);

impl Crs {
    /// WGS 84 geographic (longitude/latitude degrees), EPSG:4326.
    pub const WGS84: Self = Self(4326);

    /// WGS 84 / Pseudo-Mercator ("Web Mercator", meters), EPSG:3857 — the
    /// output CRS of the `WebMercatorQuad` tile matrix set.
    pub const WEB_MERCATOR: Self = Self(3857);

    /// Wraps an EPSG code.
    ///
    /// No registry validation happens here (that would require I/O or an
    /// embedded database); adapters that resolve codes report unknown ones
    /// through their own errors.
    #[must_use]
    pub const fn from_epsg(code: u32) -> Self {
        Self(code)
    }

    /// The EPSG code.
    #[must_use]
    pub const fn epsg(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Crs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EPSG:{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Crs;

    #[test]
    fn display_is_epsg_prefixed() {
        assert_eq!(Crs::WGS84.to_string(), "EPSG:4326");
        assert_eq!(Crs::WEB_MERCATOR.to_string(), "EPSG:3857");
        assert_eq!(Crs::from_epsg(32613).to_string(), "EPSG:32613");
    }

    #[test]
    fn constants_carry_the_wellknown_codes() {
        assert_eq!(Crs::WGS84.epsg(), 4326);
        assert_eq!(Crs::WEB_MERCATOR.epsg(), 3857);
    }
}
