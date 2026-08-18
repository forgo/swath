// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Raster vocabulary: what a source asset *is* and how to ask for pixels.
//!
//! These types are the nouns of the future `RasterSource` port
//! (ARCHITECTURE.md §6): [`RasterInfo`] is what `describe` returns,
//! [`WindowRequest`] is what `read_window` takes, and [`AssetRef`] names the
//! asset being read. No pixel data or I/O lives here — `WindowData` (the
//! pixels themselves) lands with the port and its first adapter.

use std::fmt;

use crate::crs::Crs;
use crate::error::Error;

/// An opaque reference to a source asset — a URI (`s3://…`, `file://…`,
/// `https://…`) the `RasterSource` adapter knows how to open.
///
/// Opaque by design: the core never parses it, so scheme support is purely an
/// adapter concern. Serializes as a bare string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct AssetRef(String);

impl AssetRef {
    /// Wraps a URI string.
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self(uri.into())
    }

    /// The URI as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Sample data type of a raster band.
///
/// The set is what HLS and VIIRS actually ship (HLS surface reflectance:
/// `Int16`; HLS Fmask: `UInt8`; VIIRS radiometric products: `UInt16`/
/// `Float32`) plus growth room; it widens as real sources demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DType {
    /// Unsigned 8-bit integer.
    UInt8,
    /// Signed 16-bit integer.
    Int16,
    /// Unsigned 16-bit integer.
    UInt16,
    /// Signed 32-bit integer.
    Int32,
    /// IEEE 754 single-precision float.
    Float32,
    /// IEEE 754 double-precision float.
    Float64,
}

impl DType {
    /// Size of one sample in bytes.
    #[must_use]
    pub const fn size_bytes(self) -> usize {
        match self {
            Self::UInt8 => 1,
            Self::Int16 | Self::UInt16 => 2,
            Self::Int32 | Self::Float32 => 4,
            Self::Float64 => 8,
        }
    }
}

/// Affine pixel↔CRS mapping, GDAL's six-parameter convention:
///
/// ```text
/// x = origin_x + col * pixel_width  + row * row_rotation
/// y = origin_y + col * col_rotation + row * pixel_height
/// ```
///
/// `(origin_x, origin_y)` is the CRS position of the **top-left corner of the
/// top-left pixel**; `(col, row)` are fractional pixel coordinates measured
/// from that corner (so integer `col`/`row` address pixel corners, and
/// `col + 0.5` / `row + 0.5` the pixel center). Rows are stored north-up in
/// the common case: `pixel_height` is **negative** (y decreases as `row`
/// grows southward) and both rotation terms are zero.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GeoTransform {
    /// CRS x of the top-left corner of pixel (0, 0).
    pub origin_x: f64,
    /// Column step in CRS x units (GDAL `GT(1)`); positive east-up.
    pub pixel_width: f64,
    /// Row step in CRS x units (GDAL `GT(2)`); zero for axis-aligned rasters.
    pub row_rotation: f64,
    /// CRS y of the top-left corner of pixel (0, 0).
    pub origin_y: f64,
    /// Column step in CRS y units (GDAL `GT(4)`); zero for axis-aligned rasters.
    pub col_rotation: f64,
    /// Row step in CRS y units (GDAL `GT(5)`); **negative** for north-up rasters.
    pub pixel_height: f64,
}

impl GeoTransform {
    /// An axis-aligned, north-up transform (both rotation terms zero).
    /// `pixel_height` should be negative per the north-up convention.
    #[must_use]
    pub const fn north_up(
        origin_x: f64,
        origin_y: f64,
        pixel_width: f64,
        pixel_height: f64,
    ) -> Self {
        Self {
            origin_x,
            pixel_width,
            row_rotation: 0.0,
            origin_y,
            col_rotation: 0.0,
            pixel_height,
        }
    }

    /// Maps fractional pixel coordinates `(col, row)` to CRS coordinates
    /// `(x, y)`.
    #[must_use]
    pub fn pixel_to_crs(&self, col: f64, row: f64) -> (f64, f64) {
        (
            self.row_rotation
                .mul_add(row, self.pixel_width.mul_add(col, self.origin_x)),
            self.pixel_height
                .mul_add(row, self.col_rotation.mul_add(col, self.origin_y)),
        )
    }

    /// Determinant of the 2×2 linear part; zero means the transform collapses
    /// the plane and cannot be inverted.
    #[must_use]
    pub fn determinant(&self) -> f64 {
        self.pixel_width
            .mul_add(self.pixel_height, -(self.row_rotation * self.col_rotation))
    }

    /// Maps CRS coordinates `(x, y)` back to fractional pixel coordinates
    /// `(col, row)` — the inverse of [`Self::pixel_to_crs`].
    ///
    /// # Errors
    ///
    /// [`Error::NonInvertibleTransform`] when the linear part is singular
    /// (determinant exactly zero, e.g. a zero-sized pixel).
    pub fn crs_to_pixel(&self, x: f64, y: f64) -> Result<(f64, f64), Error> {
        let det = self.determinant();
        if det == 0.0 {
            return Err(Error::NonInvertibleTransform { determinant: det });
        }
        let dx = x - self.origin_x;
        let dy = y - self.origin_y;
        Ok((
            self.pixel_height.mul_add(dx, -(self.row_rotation * dy)) / det,
            self.pixel_width.mul_add(dy, -(self.col_rotation * dx)) / det,
        ))
    }
}

/// The manifest records the same six numbers as pure data
/// (`swath_manifest::GeoTransform`, ADR 0016); the core owns the geometry.
/// The two convert field-for-field, both directions — the extraction shim.
impl From<swath_manifest::GeoTransform> for GeoTransform {
    fn from(t: swath_manifest::GeoTransform) -> Self {
        Self {
            origin_x: t.origin_x,
            pixel_width: t.pixel_width,
            row_rotation: t.row_rotation,
            origin_y: t.origin_y,
            col_rotation: t.col_rotation,
            pixel_height: t.pixel_height,
        }
    }
}

/// The core→manifest half of the shim: generators compute with the core
/// geometry and record the result in the schema's own vocabulary.
impl From<GeoTransform> for swath_manifest::GeoTransform {
    fn from(t: GeoTransform) -> Self {
        Self {
            origin_x: t.origin_x,
            pixel_width: t.pixel_width,
            row_rotation: t.row_rotation,
            origin_y: t.origin_y,
            col_rotation: t.col_rotation,
            pixel_height: t.pixel_height,
        }
    }
}

/// A rectangular pixel window into a raster grid: `width × height` pixels
/// starting at `(col_off, row_off)` from the top-left.
///
/// This is the request shape of the future `RasterSource::read_window`
/// (ARCHITECTURE.md §6). A zero-area window is valid and means "nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct WindowRequest {
    /// Leftmost column of the window.
    pub col_off: u64,
    /// Topmost row of the window.
    pub row_off: u64,
    /// Width in pixels (columns).
    pub width: u64,
    /// Height in pixels (rows).
    pub height: u64,
}

impl WindowRequest {
    /// One-past-the-end column (`col_off + width`), saturating at `u64::MAX`.
    #[must_use]
    pub const fn end_col(&self) -> u64 {
        self.col_off.saturating_add(self.width)
    }

    /// One-past-the-end row (`row_off + height`), saturating at `u64::MAX`.
    #[must_use]
    pub const fn end_row(&self) -> u64 {
        self.row_off.saturating_add(self.height)
    }

    /// Whether the window covers zero pixels.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Whether `other` lies entirely within `self`. Every window (including
    /// `self`) contains an empty window.
    #[must_use]
    pub const fn contains(&self, other: &Self) -> bool {
        other.is_empty()
            || (self.col_off <= other.col_off
                && self.row_off <= other.row_off
                && other.end_col() <= self.end_col()
                && other.end_row() <= self.end_row())
    }

    /// The overlapping region of two windows, or `None` when they are
    /// disjoint (or the overlap has zero area). Commutative.
    #[must_use]
    pub const fn intersection(&self, other: &Self) -> Option<Self> {
        let col_off = if self.col_off > other.col_off {
            self.col_off
        } else {
            other.col_off
        };
        let row_off = if self.row_off > other.row_off {
            self.row_off
        } else {
            other.row_off
        };
        let end_col = if self.end_col() < other.end_col() {
            self.end_col()
        } else {
            other.end_col()
        };
        let end_row = if self.end_row() < other.end_row() {
            self.end_row()
        } else {
            other.end_row()
        };
        if col_off < end_col && row_off < end_row {
            Some(Self {
                col_off,
                row_off,
                width: end_col - col_off,
                height: end_row - row_off,
            })
        } else {
            None
        }
    }
}

/// Static description of a raster asset — what `RasterSource::describe`
/// returns (ARCHITECTURE.md §6): everything the planner and tiler need to
/// reason about a source without reading pixels.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RasterInfo {
    /// Native CRS of the pixel grid.
    pub crs: Crs,
    /// Full-resolution grid width in pixels.
    pub width: u64,
    /// Full-resolution grid height in pixels.
    pub height: u64,
    /// Pixel↔CRS mapping of the full-resolution grid.
    pub transform: GeoTransform,
    /// Number of bands.
    pub band_count: u32,
    /// Sample type shared by all bands. (Mixed-dtype sources, if they ever
    /// appear, become per-band metadata then — not speculatively now.)
    pub dtype: DType,
    /// Nodata sentinel, widened to `f64` (GDAL convention), if declared.
    pub nodata: Option<f64>,
    /// Decimation factors of embedded overviews, ascending (e.g. `[2, 4, 8]`);
    /// empty when the asset has none.
    pub overview_levels: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::{AssetRef, DType, GeoTransform, WindowRequest};

    #[test]
    fn asset_ref_displays_its_uri() {
        let asset = AssetRef::new("s3://bucket/granule/B04.tif");
        assert_eq!(asset.to_string(), "s3://bucket/granule/B04.tif");
        assert_eq!(asset.as_str(), "s3://bucket/granule/B04.tif");
    }

    #[test]
    fn dtype_sizes() {
        assert_eq!(DType::UInt8.size_bytes(), 1);
        assert_eq!(DType::Int16.size_bytes(), 2);
        assert_eq!(DType::UInt16.size_bytes(), 2);
        assert_eq!(DType::Int32.size_bytes(), 4);
        assert_eq!(DType::Float32.size_bytes(), 4);
        assert_eq!(DType::Float64.size_bytes(), 8);
    }

    #[test]
    fn hls_fixture_transform_round_trips_a_corner() {
        // The committed HLS fixture window (tests/fixtures/README.md):
        // EPSG:32613, 30 m pixels, top-left corner at (453720, 4353960).
        let gt = GeoTransform::north_up(453_720.0, 4_353_960.0, 30.0, -30.0);
        assert_eq!(gt.pixel_to_crs(0.0, 0.0), (453_720.0, 4_353_960.0));
        // 512 pixels east and south lands on the window's far corner.
        assert_eq!(gt.pixel_to_crs(512.0, 512.0), (469_080.0, 4_338_600.0));
        let (col, row) = gt.crs_to_pixel(469_080.0, 4_338_600.0).unwrap();
        assert!((col - 512.0).abs() < 1e-9);
        assert!((row - 512.0).abs() < 1e-9);
    }

    #[test]
    fn singular_transform_refuses_inversion() {
        let gt = GeoTransform::north_up(0.0, 0.0, 30.0, 0.0);
        assert!(gt.crs_to_pixel(1.0, 1.0).is_err());
    }

    #[test]
    fn window_edges() {
        let a = WindowRequest {
            col_off: 0,
            row_off: 0,
            width: 4,
            height: 4,
        };
        let b = WindowRequest {
            col_off: 4,
            row_off: 0,
            width: 4,
            height: 4,
        };
        // Windows that only touch do not intersect.
        assert_eq!(a.intersection(&b), None);
        let empty = WindowRequest {
            col_off: 2,
            row_off: 2,
            width: 0,
            height: 3,
        };
        assert!(empty.is_empty());
        assert!(a.contains(&empty));
        assert_eq!(a.intersection(&empty), None);
        assert!(a.contains(&a));
    }
}
