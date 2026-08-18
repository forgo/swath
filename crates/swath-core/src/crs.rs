// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Coordinate reference system identity.
//!
//! [`Crs`] is vocabulary only: a CRS *name* the rest of the domain passes
//! around (raster metadata, the [`Trace`](crate::trace::Trace) `crs_from` /
//! `crs_to` fields, the [`Reproject`](crate::reproject::Reproject) port).
//! Projection **math** is
//! deliberately absent — per ARCHITECTURE.md §6 that lives behind the
//! `Reproject` port, implemented by adapter crates (proj4rs first).
//!
//! # Two vocabularies (issue #39)
//!
//! Most CRSs carry an EPSG code ([`Crs::Epsg`]). Some real grids do not:
//! VIIRS/MODIS gridded products live on the MODIS-heritage **sinusoidal**
//! grid, which has no EPSG registration and is conventionally named by its
//! proj string (`+proj=sinu +R=6371007.181 …`). [`Crs::Proj4`] carries that
//! identity losslessly — mirroring the manifest vocabulary
//! ([`GeorefCrs`](crate::manifest::GeorefCrs)) and the input language of the
//! proj4rs `Reproject` adapter. Which CRSs an adapter actually *resolves*
//! remains the adapter's documented contract; this type never validates.
//!
//! # Serialized form (a pinned contract)
//!
//! [`Crs`] serializes as a bare JSON **number** for [`Crs::Epsg`] — exactly
//! the pre-#39 wire form of the EPSG-only newtype, so every existing Trace
//! consumer keeps parsing unchanged — and as a bare JSON **string** (the raw
//! proj string) for [`Crs::Proj4`]. The two are unambiguous on the wire
//! (JSON numbers vs strings), keep the common case compact, and avoid an
//! object wrapper that would have broken the pinned Trace contract for
//! every historical value. Deserialization accepts exactly those two forms.

use std::fmt;

/// A coordinate reference system: an EPSG code, or — for grids with no EPSG
/// registration (MODIS/VIIRS sinusoidal) — a proj-string definition.
///
/// `Display` renders `"EPSG:3857"` / `"PROJ4:<string>"`; the serde form is
/// a bare number / bare string (see the [module docs](self) for why that
/// shape is pinned).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Crs {
    /// An EPSG-registered CRS, by code.
    Epsg(u32),
    /// A proj-string-defined CRS (e.g. `+proj=sinu +R=6371007.181 …`).
    ///
    /// The string is an opaque identity here: equality/hashing are textual,
    /// and resolution to actual math happens in `Reproject` adapters.
    Proj4(String),
}

impl Crs {
    /// WGS 84 geographic (longitude/latitude degrees), EPSG:4326.
    pub const WGS84: Self = Self::Epsg(4326);

    /// WGS 84 / Pseudo-Mercator ("Web Mercator", meters), EPSG:3857 — the
    /// output CRS of the `WebMercatorQuad` tile matrix set.
    pub const WEB_MERCATOR: Self = Self::Epsg(3857);

    /// Wraps an EPSG code.
    ///
    /// No registry validation happens here (that would require I/O or an
    /// embedded database); adapters that resolve codes report unknown ones
    /// through their own errors.
    #[must_use]
    pub const fn from_epsg(code: u32) -> Self {
        Self::Epsg(code)
    }

    /// Wraps a proj-string definition. As with [`Crs::from_epsg`], no
    /// validation happens here; adapters reject strings they cannot
    /// resolve.
    #[must_use]
    pub fn from_proj4(definition: impl Into<String>) -> Self {
        Self::Proj4(definition.into())
    }

    /// The EPSG code, when this CRS is EPSG-identified.
    #[must_use]
    pub const fn epsg(&self) -> Option<u32> {
        match self {
            Self::Epsg(code) => Some(*code),
            Self::Proj4(_) => None,
        }
    }

    /// The proj-string definition, when this CRS is proj-string-identified.
    #[must_use]
    pub fn proj4(&self) -> Option<&str> {
        match self {
            Self::Epsg(_) => None,
            Self::Proj4(definition) => Some(definition),
        }
    }
}

impl From<&crate::manifest::GeorefCrs> for Crs {
    /// The manifest CRS vocabulary maps losslessly onto the core [`Crs`]
    /// (which grew its proj-string variant in #39 for exactly this): an
    /// EPSG code stays a code, a proj string stays a proj string. (Moved
    /// here from the manifest module when the schema was extracted to the
    /// `swath-manifest` crate — ADR 0016; `Crs` is the local type.)
    fn from(crs: &crate::manifest::GeorefCrs) -> Self {
        match crs {
            crate::manifest::GeorefCrs::Epsg(code) => Self::Epsg(*code),
            crate::manifest::GeorefCrs::Proj4(definition) => Self::Proj4(definition.clone()),
        }
    }
}

impl fmt::Display for Crs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Epsg(code) => write!(f, "EPSG:{code}"),
            Self::Proj4(definition) => write!(f, "PROJ4:{definition}"),
        }
    }
}

impl serde::Serialize for Crs {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Epsg(code) => serializer.serialize_u32(*code),
            Self::Proj4(definition) => serializer.serialize_str(definition),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Crs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = Crs;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an EPSG code (number) or a proj string")
            }

            fn visit_u64<E: serde::de::Error>(self, code: u64) -> Result<Crs, E> {
                u32::try_from(code)
                    .map(Crs::Epsg)
                    .map_err(|_| E::custom(format!("EPSG code {code} exceeds u32")))
            }

            fn visit_i64<E: serde::de::Error>(self, code: i64) -> Result<Crs, E> {
                u32::try_from(code)
                    .map(Crs::Epsg)
                    .map_err(|_| E::custom(format!("EPSG code {code} is not a valid u32")))
            }

            fn visit_str<E: serde::de::Error>(self, definition: &str) -> Result<Crs, E> {
                Ok(Crs::Proj4(definition.to_owned()))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::Crs;

    const SINU: &str = "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R=6371007.181 +units=m +no_defs";

    #[test]
    fn display_is_scheme_prefixed() {
        assert_eq!(Crs::WGS84.to_string(), "EPSG:4326");
        assert_eq!(Crs::WEB_MERCATOR.to_string(), "EPSG:3857");
        assert_eq!(Crs::from_epsg(32613).to_string(), "EPSG:32613");
        assert_eq!(Crs::from_proj4(SINU).to_string(), format!("PROJ4:{SINU}"));
    }

    #[test]
    fn constants_carry_the_wellknown_codes() {
        assert_eq!(Crs::WGS84.epsg(), Some(4326));
        assert_eq!(Crs::WEB_MERCATOR.epsg(), Some(3857));
        assert_eq!(Crs::WEB_MERCATOR.proj4(), None);
        assert_eq!(Crs::from_proj4(SINU).epsg(), None);
        assert_eq!(Crs::from_proj4(SINU).proj4(), Some(SINU));
    }

    /// The wire contract (module docs): EPSG is a bare number — the exact
    /// pre-#39 form — and proj4 is a bare string.
    #[test]
    fn serde_is_number_for_epsg_and_string_for_proj4() {
        assert_eq!(
            serde_json::to_value(Crs::from_epsg(32613)).unwrap(),
            serde_json::json!(32613)
        );
        assert_eq!(
            serde_json::to_value(Crs::from_proj4(SINU)).unwrap(),
            serde_json::json!(SINU)
        );
        let epsg: Crs = serde_json::from_str("4326").unwrap();
        assert_eq!(epsg, Crs::WGS84);
        let proj: Crs = serde_json::from_value(serde_json::json!(SINU)).unwrap();
        assert_eq!(proj, Crs::from_proj4(SINU));
        // Anything else is refused, never guessed.
        assert!(serde_json::from_str::<Crs>("{\"epsg\": 4326}").is_err());
        assert!(serde_json::from_str::<Crs>("-3").is_err());
        assert!(serde_json::from_str::<Crs>("4294967296").is_err());
    }
}
