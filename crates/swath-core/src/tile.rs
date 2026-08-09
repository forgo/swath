// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Tile addressing and `WebMercatorQuad` tile matrix set math.
//!
//! [`TileCoord`] is the TMS-independent quadtree address (`z/x/y`, XYZ row
//! order: `y` grows southward from the top-left origin). The projection math
//! that turns an address into ground coordinates lives on a TMS type —
//! [`WebMercatorQuad`] is the only one today, and keeping the math on the TMS
//! rather than on `TileCoord` itself means another tile matrix set (e.g.
//! `WorldCRS84Quad`) is an additive new type, not a breaking change.
//!
//! All math here is closed-form floating point — no I/O, no lookup tables.
//! Ground truth is pinned against morecantile in
//! `tests/data/tms_truth.json` (see `tests/tms_truth.rs`).

use std::fmt;

use crate::error::Error;

/// Largest supported zoom level.
///
/// At `z = 31` the full column/row range `0..2^31` still fits comfortably in
/// the `u32` tile coordinates; beyond that the addressing scheme itself would
/// need widening. (Web maps in practice stop well short of this — morecantile's
/// `WebMercatorQuad` definition tops out at 24.)
pub const MAX_ZOOM: u8 = 31;

/// Half the extent of the Web Mercator plane in meters: `π * 6378137`
/// (`2π * 6378137 / 2`), i.e. the coordinate of the top-right corner of tile
/// `0/0/0`. The plane spans `[-EXTENT, EXTENT]` on both axes.
pub const WEB_MERCATOR_EXTENT: f64 = 20_037_508.342_789_244;

/// WGS 84 semi-major axis in meters (the sphere radius of spherical Mercator).
const EARTH_RADIUS: f64 = 6_378_137.0;

/// A tile address in a power-of-two quadtree pyramid: zoom `z`, column `x`
/// (west→east), row `y` (north→south, XYZ/"slippy map" convention — row 0 is
/// the northernmost).
///
/// Fields are public for ergonomic construction in tests and pattern matching;
/// [`TileCoord::new`] is the validating constructor and the rest of the crate
/// assumes its invariant (`x, y < 2^z`, `z <=` [`MAX_ZOOM`]). The bounds math
/// on [`WebMercatorQuad`] is total either way — an out-of-range coordinate
/// simply extrapolates off the mercator plane rather than panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TileCoord {
    /// Zoom level (pyramid depth); level `z` is a `2^z × 2^z` grid.
    pub z: u8,
    /// Column, `0..2^z`, increasing eastward.
    pub x: u32,
    /// Row, `0..2^z`, increasing southward (top-left origin).
    pub y: u32,
}

impl TileCoord {
    /// Validating constructor: requires `z <= `[`MAX_ZOOM`] and `x, y < 2^z`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTileCoord`] when the coordinate violates either bound.
    pub const fn new(z: u8, x: u32, y: u32) -> Result<Self, Error> {
        let coord = Self { z, x, y };
        if coord.is_valid() {
            Ok(coord)
        } else {
            Err(Error::InvalidTileCoord { z, x, y })
        }
    }

    /// Whether this coordinate satisfies `z <= `[`MAX_ZOOM`] and `x, y < 2^z`.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        // 1u64 << z cannot overflow: z <= 31 has already been checked.
        self.z <= MAX_ZOOM
            && (self.x as u64) < (1u64 << self.z)
            && (self.y as u64) < (1u64 << self.z)
    }

    /// The tile one zoom level up that contains this tile, or `None` at the
    /// root (`z = 0`).
    #[must_use]
    pub const fn parent(self) -> Option<Self> {
        match self.z.checked_sub(1) {
            Some(z) => Some(Self {
                z,
                x: self.x / 2,
                y: self.y / 2,
            }),
            None => None,
        }
    }

    /// The four tiles one zoom level down that partition this tile, in
    /// row-major order (NW, NE, SW, SE), or `None` at [`MAX_ZOOM`] (where the
    /// child coordinates would leave the supported range).
    #[must_use]
    pub const fn children(self) -> Option<[Self; 4]> {
        if self.z >= MAX_ZOOM {
            return None;
        }
        let (z, x, y) = (self.z + 1, self.x * 2, self.y * 2);
        Some([
            Self { z, x, y },
            Self { z, x: x + 1, y },
            Self { z, x, y: y + 1 },
            Self {
                z,
                x: x + 1,
                y: y + 1,
            },
        ])
    }
}

impl fmt::Display for TileCoord {
    /// Formats as `"z/x/y"` — the order tiles appear in XYZ URLs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.z, self.x, self.y)
    }
}

/// An axis-aligned bounding box in projected (Web Mercator, EPSG:3857) meters.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MercatorBounds {
    /// Western edge (meters).
    pub min_x: f64,
    /// Southern edge (meters).
    pub min_y: f64,
    /// Eastern edge (meters).
    pub max_x: f64,
    /// Northern edge (meters).
    pub max_y: f64,
}

impl MercatorBounds {
    /// Whether `other` lies entirely within `self` (edges may touch).
    #[must_use]
    pub fn contains(&self, other: &Self) -> bool {
        self.min_x <= other.min_x
            && self.min_y <= other.min_y
            && self.max_x >= other.max_x
            && self.max_y >= other.max_y
    }

    /// Center point `(x, y)` in meters.
    #[must_use]
    pub fn center(&self) -> (f64, f64) {
        (
            f64::midpoint(self.min_x, self.max_x),
            f64::midpoint(self.min_y, self.max_y),
        )
    }
}

/// An axis-aligned bounding box in geographic (WGS 84, EPSG:4326) degrees.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LonLatBounds {
    /// Western edge (degrees longitude).
    pub west: f64,
    /// Southern edge (degrees latitude).
    pub south: f64,
    /// Eastern edge (degrees longitude).
    pub east: f64,
    /// Northern edge (degrees latitude).
    pub north: f64,
}

/// The `WebMercatorQuad` tile matrix set (OGC 17-083r4): spherical Mercator
/// (EPSG:3857), square tiles, top-left origin at `(-EXTENT, EXTENT)`, each
/// zoom level doubling the grid.
///
/// A zero-sized marker type so a second TMS is an additive sibling type. All
/// methods are pure functions of their inputs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WebMercatorQuad;

impl WebMercatorQuad {
    /// Number of tiles along each axis at zoom `z` (`2^z`), or `None` beyond
    /// [`MAX_ZOOM`].
    #[must_use]
    pub const fn matrix_width(z: u8) -> Option<u32> {
        if z <= MAX_ZOOM { Some(1u32 << z) } else { None }
    }

    /// Side length in meters of one tile at zoom `z`
    /// (`2 * EXTENT / 2^z`).
    #[must_use]
    pub fn tile_span(z: u8) -> f64 {
        2.0 * WEB_MERCATOR_EXTENT / f64::from(z).exp2()
    }

    /// Web Mercator (EPSG:3857) bounds of `tile` in meters.
    #[must_use]
    pub fn xy_bounds(tile: TileCoord) -> MercatorBounds {
        let span = Self::tile_span(tile.z);
        let min_x = span.mul_add(f64::from(tile.x), -WEB_MERCATOR_EXTENT);
        let max_y = span.mul_add(-f64::from(tile.y), WEB_MERCATOR_EXTENT);
        MercatorBounds {
            min_x,
            min_y: max_y - span,
            max_x: min_x + span,
            max_y,
        }
    }

    /// Geographic (WGS 84) bounds of `tile` in degrees, from the closed-form
    /// inverse spherical Mercator (`lat = atan(sinh(y / R))`).
    #[must_use]
    pub fn lonlat_bounds(tile: TileCoord) -> LonLatBounds {
        let m = Self::xy_bounds(tile);
        LonLatBounds {
            west: Self::lon_of(m.min_x),
            south: Self::lat_of(m.min_y),
            east: Self::lon_of(m.max_x),
            north: Self::lat_of(m.max_y),
        }
    }

    /// The tile at zoom `z` containing the Web Mercator point `(x, y)` in
    /// meters. Points outside the plane, and points exactly on the east/south
    /// edge, clamp to the nearest tile (morecantile's behavior).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTileCoord`] when `z` exceeds [`MAX_ZOOM`].
    pub fn tile_for_xy(x: f64, y: f64, z: u8) -> Result<TileCoord, Error> {
        let Some(width) = Self::matrix_width(z) else {
            return Err(Error::InvalidTileCoord { z, x: 0, y: 0 });
        };
        let span = Self::tile_span(z);
        let max_index = f64::from(width - 1);
        let col = ((x + WEB_MERCATOR_EXTENT) / span)
            .floor()
            .clamp(0.0, max_index);
        let row = ((WEB_MERCATOR_EXTENT - y) / span)
            .floor()
            .clamp(0.0, max_index);
        // Truncation/sign-loss is impossible: both values were clamped into
        // [0, 2^z - 1] with z <= 31.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(TileCoord {
            z,
            x: col as u32,
            y: row as u32,
        })
    }

    /// Longitude in degrees of a Web Mercator x coordinate.
    fn lon_of(x: f64) -> f64 {
        x / WEB_MERCATOR_EXTENT * 180.0
    }

    /// Latitude in degrees of a Web Mercator y coordinate.
    fn lat_of(y: f64) -> f64 {
        (y / EARTH_RADIUS).sinh().atan().to_degrees()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_ZOOM, MercatorBounds, TileCoord, WEB_MERCATOR_EXTENT, WebMercatorQuad};
    use crate::error::Error;

    #[test]
    fn display_is_z_x_y() {
        let tile = TileCoord::new(6, 10, 24).unwrap();
        assert_eq!(tile.to_string(), "6/10/24");
    }

    #[test]
    fn validity_edges() {
        assert!(TileCoord::new(0, 0, 0).is_ok());
        assert_eq!(
            TileCoord::new(0, 0, 1),
            Err(Error::InvalidTileCoord { z: 0, x: 0, y: 1 })
        );
        // Max valid coordinate at MAX_ZOOM…
        let max = (1u32 << MAX_ZOOM) - 1;
        assert!(TileCoord::new(MAX_ZOOM, max, max).is_ok());
        // …and one past it, in either direction.
        assert!(TileCoord::new(MAX_ZOOM, max, max.wrapping_add(1)).is_err());
        assert!(TileCoord::new(MAX_ZOOM + 1, 0, 0).is_err());
    }

    #[test]
    fn root_has_no_parent_and_max_zoom_has_no_children() {
        let root = TileCoord::new(0, 0, 0).unwrap();
        assert_eq!(root.parent(), None);
        let deep = TileCoord::new(MAX_ZOOM, 0, 0).unwrap();
        assert_eq!(deep.children(), None);
    }

    #[test]
    fn parent_child_round_trip() {
        let tile = TileCoord::new(12, 848, 1561).unwrap();
        for child in tile.children().unwrap() {
            assert!(child.is_valid());
            assert_eq!(child.parent(), Some(tile));
        }
    }

    #[test]
    fn root_bounds_are_the_full_plane() {
        let bounds = WebMercatorQuad::xy_bounds(TileCoord { z: 0, x: 0, y: 0 });
        assert_eq!(
            bounds,
            MercatorBounds {
                min_x: -WEB_MERCATOR_EXTENT,
                min_y: -WEB_MERCATOR_EXTENT,
                max_x: WEB_MERCATOR_EXTENT,
                max_y: WEB_MERCATOR_EXTENT,
            }
        );
    }

    #[test]
    fn root_lonlat_spans_the_mercator_world() {
        let ll = WebMercatorQuad::lonlat_bounds(TileCoord { z: 0, x: 0, y: 0 });
        assert!((ll.west - -180.0).abs() < 1e-9);
        assert!((ll.east - 180.0).abs() < 1e-9);
        // The Mercator latitude limit.
        assert!((ll.north - 85.051_128_779_806_59).abs() < 1e-9);
        assert!((ll.south - -85.051_128_779_806_59).abs() < 1e-9);
    }

    #[test]
    fn tile_for_xy_clamps_edges() {
        // The exact east/south corner of the plane belongs to the last tile.
        let tile =
            WebMercatorQuad::tile_for_xy(WEB_MERCATOR_EXTENT, -WEB_MERCATOR_EXTENT, 1).unwrap();
        assert_eq!(tile, TileCoord { z: 1, x: 1, y: 1 });
        assert!(WebMercatorQuad::tile_for_xy(0.0, 0.0, MAX_ZOOM + 1).is_err());
    }

    #[test]
    fn matrix_width_stops_at_max_zoom() {
        assert_eq!(WebMercatorQuad::matrix_width(0), Some(1));
        assert_eq!(WebMercatorQuad::matrix_width(MAX_ZOOM), Some(1u32 << 31));
        assert_eq!(WebMercatorQuad::matrix_width(MAX_ZOOM + 1), None);
    }
}
