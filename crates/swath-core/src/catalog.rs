// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The catalog domain: `Dataset` / `Granule` / `Layer`, and the `Catalog` port.
//!
//! This is the "make STAC disappear" module (REQUIREMENTS.md R2, ARCHITECTURE.md
//! §5, `docs/design/catalog-domain.md`): users and the rest of the core speak
//! [`Dataset`]s, [`Granule`]s, and [`Layer`]s; the STAC documents those persist
//! as exist only inside adapters, produced and consumed by the pure converters
//! in [`stac`]. No STAC type appears in any port signature — R2 by construction.
//!
//! - [`Dataset`] — a logical collection of granules sharing a band vocabulary
//!   (maps to a STAC Collection)
//! - [`Granule`] — one acquisition's band → asset map (maps to a STAC Item)
//! - [`Layer`] — a serving definition over a Dataset (stored as `swath:layers`
//!   on the Collection; see the design doc for the storage decision)
//! - [`Catalog`] — the port trait adapters implement (pgstac first)
//!
//! The port uses the same native async-in-trait pattern as
//! [`RasterSource`](crate::source::RasterSource): `-> impl Future<…> + Send`,
//! no runtime dependency in the core, deliberately not dyn-compatible (see the
//! [`crate::source`] module docs for the recorded trade-off).

pub mod stac;

use core::fmt;
use core::future::Future;
use std::collections::{BTreeMap, BTreeSet};

use crate::error::Error;
use crate::raster::AssetRef;

/// Identifies a [`Dataset`] — the STAC Collection id in storage.
///
/// Serializes as a bare string.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct DatasetId(String);

impl DatasetId {
    /// Wraps an id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DatasetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies a [`Granule`] within its dataset — the STAC Item id in storage.
///
/// Serializes as a bare string.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct GranuleId(String);

impl GranuleId {
    /// Wraps an id string.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GranuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A WGS84 (CRS84 axis order: longitude, latitude) bounding box.
///
/// Invariant, documented rather than constructed-checked: all four values are
/// finite degrees (non-finite floats have no JSON representation). An
/// antimeridian-crossing box has `west > east`, per the STAC/GeoJSON
/// convention; no ordering is enforced here.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bbox {
    /// Western (minimum-longitude) edge, degrees.
    pub west: f64,
    /// Southern (minimum-latitude) edge, degrees.
    pub south: f64,
    /// Eastern (maximum-longitude) edge, degrees.
    pub east: f64,
    /// Northern (maximum-latitude) edge, degrees.
    pub north: f64,
}

impl Bbox {
    /// The `[west, south, east, north]` array form STAC documents carry.
    #[must_use]
    pub const fn to_array(self) -> [f64; 4] {
        [self.west, self.south, self.east, self.north]
    }

    /// A box from the STAC `[west, south, east, north]` array form.
    #[must_use]
    pub const fn from_array([west, south, east, north]: [f64; 4]) -> Self {
        Self {
            west,
            south,
            east,
            north,
        }
    }
}

/// An RFC 3339 UTC timestamp, `Z`-suffixed — the only datetime form the
/// catalog speaks (STAC requires RFC 3339; Swath additionally normalizes to
/// UTC so stored values are comparable and unambiguous).
///
/// Validated at construction: `YYYY-MM-DDThh:mm:ss[.fraction]Z`, with
/// calendar-aware day-of-month bounds. Serde deserialization re-validates via
/// `try_from`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Datetime(String);

impl Datetime {
    /// Validates and wraps an RFC 3339 UTC (`Z`) timestamp.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidDatetime`] when the string is not of the form
    /// `YYYY-MM-DDThh:mm:ss[.fraction]Z` with in-range date/time components.
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if is_rfc3339_utc(&value) {
            Ok(Self(value))
        } else {
            Err(Error::InvalidDatetime { value })
        }
    }

    /// The timestamp as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Datetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Datetime {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Datetime> for String {
    fn from(value: Datetime) -> Self {
        value.0
    }
}

/// Whether `s` is `YYYY-MM-DDThh:mm:ss[.fraction]Z` with in-range components.
fn is_rfc3339_utc(s: &str) -> bool {
    let b = s.as_bytes();
    // Fixed prefix: "YYYY-MM-DDThh:mm:ss" is 19 bytes; then optional ".digits";
    // then the mandatory 'Z'.
    if b.len() < 20 || b[b.len() - 1] != b'Z' {
        return false;
    }
    let digits = |range: core::ops::Range<usize>| -> Option<u32> {
        let mut n: u32 = 0;
        for &c in &b[range] {
            if !c.is_ascii_digit() {
                return None;
            }
            n = n * 10 + u32::from(c - b'0');
        }
        Some(n)
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        digits(0..4),
        digits(5..7),
        digits(8..10),
        digits(11..13),
        digits(14..16),
        digits(17..19),
    ) else {
        return false;
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return false;
    }
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return false;
    }
    // Optional fractional seconds between the fixed prefix and the 'Z'.
    let fraction = &b[19..b.len() - 1];
    match fraction {
        [] => true,
        [b'.', rest @ ..] => !rest.is_empty() && rest.iter().all(u8::is_ascii_digit),
        _ => false,
    }
}

/// An optionally open-ended time range: `start`/`end` are inclusive bounds,
/// `None` means unbounded on that side.
///
/// Used both as a dataset's temporal extent (inside [`Extent`]) and as the
/// datetime filter of a [`GranuleQuery`].
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TimeRange {
    /// Inclusive lower bound; `None` = open.
    pub start: Option<Datetime>,
    /// Inclusive upper bound; `None` = open.
    pub end: Option<Datetime>,
}

/// A dataset's overall spatial + temporal extent (STAC Collection `extent`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Extent {
    /// Overall spatial coverage.
    pub bbox: Bbox,
    /// Overall temporal coverage; either side may be open.
    pub interval: TimeRange,
}

/// How a [`Layer`] turns its dataset's bands into gray/color planes.
///
/// This is the small, stable, *storage-facing* plan vocabulary — deliberately
/// not `swath-render`'s executable `RenderPlan`/`Expr` IR, which refactors
/// freely and lives above this crate's consumers (design doc §2). Lowering
/// `PlanKind` into a `RenderPlan` happens at serving wire-up.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum PlanKind {
    /// Three named bands as the R, G, B planes.
    Composite {
        /// Band for the red channel.
        r: String,
        /// Band for the green channel.
        g: String,
        /// Band for the blue channel.
        b: String,
    },
    /// An infix band-math expression producing gray planes (e.g.
    /// `"(b8a - b04) / (b8a + b04)"`). Stored opaquely; the process compiler
    /// (issue #34) owns parsing it.
    BandMath {
        /// The expression source text.
        expression: String,
    },
}

/// Linear mapping of a value range onto 0..=255, clamping outside values.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rescale {
    /// Value mapped to 0.
    pub min: f64,
    /// Value mapped to 255.
    pub max: f64,
}

/// A named colormap applied to gray planes. Mirrors the serving vocabulary
/// (`swath-render` grows real palettes; this enum follows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Colormap {
    /// The identity map: gray in, gray out.
    Grayscale,
}

/// Resampling kernel a layer's warps use. The nodata *policy* is a
/// serving-time default, not catalog state (design doc §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Resampling {
    /// Nearest neighbor (categorical bands, masks).
    Nearest,
    /// Bilinear (continuous reflectance/radiance).
    Bilinear,
}

/// A serving definition over a [`Dataset`]: the `TileRequest` template plus
/// the human-facing identity the OGC documents expose.
///
/// Persisted verbatim (via serde) as one entry of the Collection's
/// `swath:layers` array — this JSON shape is contractual and pinned by
/// snapshot test. Unknown fields are rejected on read (`deny_unknown_fields`):
/// a document only Swath should write that carries fields Swath doesn't know
/// is a loud error, not silent data loss.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// URL-safe identifier — the `{layerId}` path segment.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Short narrative description.
    pub description: String,
    /// How dataset bands become pixels.
    pub plan: PlanKind,
    /// Value range mapped onto 0..=255.
    pub rescale: Rescale,
    /// Colormap applied after the plan, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colormap: Option<Colormap>,
    /// Resampling kernel for the warps.
    pub resampling: Resampling,
    /// Tile side length in pixels.
    pub tile_size: u32,
}

/// A logical collection of granules sharing a band vocabulary, CRS family,
/// and cadence — maps to a STAC Collection (design doc §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Dataset {
    /// Identifier (the STAC Collection id).
    pub id: DatasetId,
    /// Human-readable title. Required here even though STAC makes it
    /// optional — Swath always writes it.
    pub title: String,
    /// Narrative description.
    pub description: String,
    /// Data license (SPDX id, or `other`), passed through to STAC.
    pub license: String,
    /// Overall spatial + temporal extent.
    pub extent: Extent,
    /// The band names granules of this dataset provide. A sorted set: the
    /// canonical order makes the STAC round trip structural.
    pub bands: BTreeSet<String>,
    /// Serving definitions, in presentation order.
    pub layers: Vec<Layer>,
}

/// One acquisition's assets: a band → asset-URI map plus footprint and
/// timestamp — maps to a STAC Item (design doc §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Granule {
    /// Identifier, unique within the dataset (the STAC Item id).
    pub id: GranuleId,
    /// The owning dataset (the STAC `collection`).
    pub dataset: DatasetId,
    /// WGS84 footprint. The STAC `geometry` is derived from this box, never
    /// stored independently.
    pub bbox: Bbox,
    /// Acquisition time.
    pub datetime: Datetime,
    /// Band name → asset URI, the map a layer's plan resolves bands against.
    pub assets: BTreeMap<String, AssetRef>,
}

/// The granule filter [`Catalog::find_granules`] takes: optional bbox
/// intersection, optional datetime range. Deliberately minimal — this is what
/// serving and ingest consume, not a STAC search façade (design doc §4).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct GranuleQuery {
    /// Keep granules whose footprint intersects this box.
    pub bbox: Option<Bbox>,
    /// Keep granules whose datetime falls in this (inclusive, optionally
    /// open-ended) range.
    pub datetime: Option<TimeRange>,
}

/// What can go wrong at the catalog boundary.
///
/// The port's error contract, defined in the core so consumers match on
/// semantics, not adapter internals (same pattern as
/// [`SourceError`](crate::source::SourceError)).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CatalogError {
    /// An operation referenced a dataset the catalog does not contain.
    #[error("dataset not found: {id}")]
    DatasetNotFound {
        /// The dataset that was referenced.
        id: DatasetId,
    },

    /// A stored document failed to map back to the domain — the signal that
    /// something other than Swath wrote to the catalog's backing store.
    #[error("stored document is not a valid swath catalog document")]
    Stac(#[from] stac::StacError),

    /// Connection, transport, or backend-database failure.
    #[error("catalog backend failure: {detail}")]
    Backend {
        /// What was being attempted.
        detail: String,
        /// The underlying driver/transport error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The catalog port (ARCHITECTURE.md §6, refined domain-shaped by the design
/// doc): persist and query datasets and granules. Implemented by adapter
/// crates (pgstac first); consumed generically by ingest and serving.
///
/// See the [module docs](self) for the async-in-trait pattern (native AFIT,
/// `Send` futures, not dyn-compatible).
pub trait Catalog: Send + Sync {
    /// Creates or replaces a dataset (and its layers, atomically).
    fn upsert_dataset(
        &self,
        dataset: &Dataset,
    ) -> impl Future<Output = Result<(), CatalogError>> + Send;

    /// Creates or replaces granules. Every granule's `dataset` must already
    /// exist ([`CatalogError::DatasetNotFound`] otherwise).
    fn upsert_granules(
        &self,
        granules: &[Granule],
    ) -> impl Future<Output = Result<(), CatalogError>> + Send;

    /// The dataset with this id, or `None`.
    fn get_dataset(
        &self,
        id: &DatasetId,
    ) -> impl Future<Output = Result<Option<Dataset>, CatalogError>> + Send;

    /// All datasets, in id order.
    fn list_datasets(&self) -> impl Future<Output = Result<Vec<Dataset>, CatalogError>> + Send;

    /// The granules of `dataset` matching `query`, exhaustively (adapters
    /// page internally; callers see the full result set).
    fn find_granules(
        &self,
        dataset: &DatasetId,
        query: &GranuleQuery,
    ) -> impl Future<Output = Result<Vec<Granule>, CatalogError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::{Bbox, Datetime};

    #[test]
    fn datetime_accepts_rfc3339_utc_forms() {
        for ok in [
            "2024-06-06T17:54:00Z",
            "2024-02-29T00:00:00Z", // leap day
            "2024-06-06T17:54:00.123456Z",
            "1999-12-31T23:59:59.9Z",
        ] {
            assert!(Datetime::new(ok).is_ok(), "should accept {ok}");
        }
    }

    #[test]
    fn datetime_rejects_non_utc_and_malformed_forms() {
        for bad in [
            "2024-06-06T17:54:00",       // no zone
            "2024-06-06T17:54:00+00:00", // offset form, not Z
            "2024-06-06 17:54:00Z",      // space separator
            "2023-02-29T00:00:00Z",      // not a leap year
            "2024-13-01T00:00:00Z",      // month 13
            "2024-06-31T00:00:00Z",      // June has 30 days
            "2024-06-06T24:00:00Z",      // hour 24
            "2024-06-06T17:54:00.Z",     // empty fraction
            "2024-06-06T17:54:00.12aZ",  // non-digit fraction
            "not a date",
            "",
        ] {
            assert!(Datetime::new(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn datetime_serde_revalidates() {
        let ok: Result<Datetime, _> = serde_json::from_str(r#""2024-06-06T17:54:00Z""#);
        assert_eq!(ok.unwrap().as_str(), "2024-06-06T17:54:00Z");
        let bad: Result<Datetime, _> = serde_json::from_str(r#""yesterday""#);
        assert!(bad.is_err());
    }

    #[test]
    fn bbox_array_round_trip() {
        let b = Bbox {
            west: -106.1,
            south: 39.2,
            east: -105.9,
            north: 39.4,
        };
        assert_eq!(Bbox::from_array(b.to_array()), b);
    }
}
