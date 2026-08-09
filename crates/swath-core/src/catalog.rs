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

    /// Milliseconds since the Unix epoch (1970-01-01T00:00:00Z), fractional
    /// seconds truncated to millisecond precision.
    ///
    /// Pure calendar arithmetic (proleptic Gregorian, days-from-civil) — no
    /// clock is read, so this stays within the core's no-I/O contract. The
    /// ingest-to-pixel timer subtracts two of these; the truncation makes
    /// the metric's resolution 1 ms, which is far below its noise floor.
    #[must_use]
    pub fn to_unix_millis(&self) -> i64 {
        let b = self.0.as_bytes();
        // Validated at construction: fixed "YYYY-MM-DDThh:mm:ss" prefix.
        let num = |range: core::ops::Range<usize>| -> i64 {
            b[range]
                .iter()
                .fold(0, |n, &c| n * 10 + i64::from(c - b'0'))
        };
        let days = days_from_civil(num(0..4), num(5..7), num(8..10));
        let seconds = num(11..13) * 3600 + num(14..16) * 60 + num(17..19);
        // Optional fraction between the fixed prefix and the trailing 'Z':
        // take the first three digits (zero-padded) as milliseconds.
        let fraction = &b[19..b.len() - 1];
        let mut millis = 0;
        for i in 0..3 {
            let digit = fraction.get(1 + i).map_or(0, |&c| i64::from(c - b'0'));
            millis = millis * 10 + digit;
        }
        (days * 86_400 + seconds) * 1000 + millis
    }

    /// The timestamp `millis` milliseconds after the Unix epoch, rendered
    /// RFC 3339 UTC (`.mmm` fraction only when non-zero).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidDatetime`] when the instant falls outside the
    /// four-digit-year range (0000..=9999) this format can express.
    pub fn from_unix_millis(millis: i64) -> Result<Self, Error> {
        let days = millis.div_euclid(86_400_000);
        let of_day = millis.rem_euclid(86_400_000);
        let (year, month, day) = civil_from_days(days);
        if !(0..=9999).contains(&year) {
            return Err(Error::InvalidDatetime {
                value: format!("{millis} ms since epoch (year out of 0000..=9999)"),
            });
        }
        let (second_of_day, ms) = (of_day / 1000, of_day % 1000);
        let (hour, minute, second) = (
            second_of_day / 3600,
            second_of_day % 3600 / 60,
            second_of_day % 60,
        );
        let fraction = if ms == 0 {
            String::new()
        } else {
            format!(".{ms:03}")
        };
        Self::new(format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{fraction}Z"
        ))
    }
}

/// Days since 1970-01-01 of a proleptic-Gregorian civil date (Howard
/// Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date of a days-since-epoch count — the exact inverse of
/// [`days_from_civil`] (Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { y + 1 } else { y }, month, day)
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

/// What a granule asset *is*, so serving can dispatch how to read it: a
/// plain raster file (COG — the default) versus a **virtual cube manifest**
/// (a `VirtualManifest` JSON generated by the ingest referencer, ADR 0006 —
/// byte-range references into a legacy granule, read by the virtual source
/// path, #39).
///
/// Persisted on the STAC asset as `swath:kind` (omitted for the default,
/// so plain-raster documents are byte-identical to pre-#40 ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AssetKind {
    /// A directly readable raster (COG today).
    #[default]
    Raster,
    /// A virtual-reference manifest describing a legacy granule as a cube.
    VirtualCube,
}

/// One granule asset: where the bytes live plus what kind of thing they
/// are (the minimal extension #40 needed — everything else about assets
/// stays an opaque [`AssetRef`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GranuleAsset {
    /// The asset URI/key, as the serving path will read it.
    pub href: AssetRef,
    /// What the URI points at; defaults to a plain raster.
    #[serde(default)]
    pub kind: AssetKind,
}

impl GranuleAsset {
    /// A plain raster asset (the overwhelmingly common case).
    #[must_use]
    pub fn raster(uri: impl Into<String>) -> Self {
        Self {
            href: AssetRef::new(uri),
            kind: AssetKind::Raster,
        }
    }

    /// A virtual-cube manifest asset.
    #[must_use]
    pub fn virtual_cube(uri: impl Into<String>) -> Self {
        Self {
            href: AssetRef::new(uri),
            kind: AssetKind::VirtualCube,
        }
    }
}

/// One acquisition's assets: a band → asset map plus footprint and
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
    /// Band name → asset, the map a layer's plan resolves bands against.
    pub assets: BTreeMap<String, GranuleAsset>,
    /// When Swath ingested this granule — the zero point of the
    /// ingest-to-pixel metric (REQUIREMENTS.md §3), stamped by the ingest
    /// orchestrator from the event's arrival time and persisted as
    /// `properties."swath:ingested_at"` on the STAC Item. `None` for
    /// granules registered outside the event path.
    pub ingested_at: Option<Datetime>,
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
    fn unix_millis_known_vectors() {
        let ms = |s: &str| Datetime::new(s).unwrap().to_unix_millis();
        assert_eq!(ms("1970-01-01T00:00:00Z"), 0);
        assert_eq!(ms("2024-06-06T17:54:00Z"), 1_717_696_440_000);
        // Leap day, and pre-epoch negativity.
        assert_eq!(ms("2024-02-29T00:00:00Z"), 1_709_164_800_000);
        assert_eq!(ms("1969-12-31T23:59:59Z"), -1000);
        // Fractions: padded, truncated beyond milliseconds.
        assert_eq!(ms("1970-01-01T00:00:00.5Z"), 500);
        assert_eq!(ms("1970-01-01T00:00:00.123456Z"), 123);
    }

    #[test]
    fn unix_millis_round_trips_through_from() {
        for ms in [
            0,
            1,
            999,
            1_717_696_440_000,
            1_709_164_800_000,
            -1000,
            -62_167_219_200_000, // 0000-01-01T00:00:00Z
            253_402_300_799_999, // 9999-12-31T23:59:59.999Z
        ] {
            let dt = Datetime::from_unix_millis(ms).expect("in range");
            assert_eq!(dt.to_unix_millis(), ms, "via {dt}");
        }
        assert!(Datetime::from_unix_millis(-62_167_219_200_001).is_err());
        assert!(Datetime::from_unix_millis(253_402_300_800_000).is_err());
    }

    #[test]
    fn from_unix_millis_renders_canonical_text() {
        let dt = |ms: i64| Datetime::from_unix_millis(ms).unwrap();
        assert_eq!(dt(0).as_str(), "1970-01-01T00:00:00Z");
        assert_eq!(dt(1_717_696_440_000).as_str(), "2024-06-06T17:54:00Z");
        assert_eq!(dt(500).as_str(), "1970-01-01T00:00:00.500Z");
        assert_eq!(dt(7).as_str(), "1970-01-01T00:00:00.007Z");
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
