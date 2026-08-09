// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The target pixel grid a warp renders into.

use swath_core::tile::{MercatorBounds, TileCoord, WebMercatorQuad};

/// A regular pixel grid over a Web Mercator bounding box — the destination
/// of a warp: `width × height` pixels spanning `bounds`, row 0 at the
/// northern edge (top-left origin, like every raster in the system).
///
/// Constructed per tile via [`TargetGrid::for_tile`] (256 or 512 px tiles);
/// [`TargetGrid::new`] exists for non-tile targets (tests, previews).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetGrid {
    bounds: MercatorBounds,
    width: u32,
    height: u32,
}

impl TargetGrid {
    /// A grid of `width × height` pixels over `bounds`.
    #[must_use]
    pub const fn new(bounds: MercatorBounds, width: u32, height: u32) -> Self {
        Self {
            bounds,
            width,
            height,
        }
    }

    /// The grid of one `WebMercatorQuad` tile at `tile_size` pixels per side
    /// (256 for classic XYZ tiles, 512 for retina).
    #[must_use]
    pub fn for_tile(tile: TileCoord, tile_size: u32) -> Self {
        Self::new(WebMercatorQuad::xy_bounds(tile), tile_size, tile_size)
    }

    /// The grid's Web Mercator bounds.
    #[must_use]
    pub const fn bounds(&self) -> MercatorBounds {
        self.bounds
    }

    /// Grid width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Grid height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Ground size of one pixel in meters `(dx, dy)`; both positive.
    #[must_use]
    pub fn pixel_size(&self) -> (f64, f64) {
        (
            (self.bounds.max_x - self.bounds.min_x) / f64::from(self.width),
            (self.bounds.max_y - self.bounds.min_y) / f64::from(self.height),
        )
    }

    /// Web Mercator coordinates of the **center** of pixel `(col, row)` —
    /// the point the inverse-mapping warp projects into the source.
    #[must_use]
    pub fn pixel_center(&self, col: u32, row: u32) -> (f64, f64) {
        let (dx, dy) = self.pixel_size();
        (
            dx.mul_add(f64::from(col) + 0.5, self.bounds.min_x),
            dy.mul_add(-(f64::from(row) + 0.5), self.bounds.max_y),
        )
    }
}

#[cfg(test)]
mod tests {
    use swath_core::tile::TileCoord;

    use super::TargetGrid;

    #[test]
    fn tile_grid_pixel_centers_span_the_tile() {
        let tile = TileCoord::new(12, 848, 1561).unwrap();
        let grid = TargetGrid::for_tile(tile, 256);
        let (dx, dy) = grid.pixel_size();
        assert!(dx > 0.0 && dy > 0.0);
        assert!((dx - dy).abs() < 1e-9, "web mercator tiles are square");
        let b = grid.bounds();
        // First center is half a pixel inside the top-left corner…
        let (x0, y0) = grid.pixel_center(0, 0);
        assert!((x0 - (b.min_x + dx / 2.0)).abs() < 1e-6);
        assert!((y0 - (b.max_y - dy / 2.0)).abs() < 1e-6);
        // …and the last is half a pixel inside the bottom-right corner.
        let (x1, y1) = grid.pixel_center(255, 255);
        assert!((x1 - (b.max_x - dx / 2.0)).abs() < 1e-6);
        assert!((y1 - (b.min_y + dy / 2.0)).abs() < 1e-6);
    }

    #[test]
    fn tile_size_parameter_scales_resolution() {
        let tile = TileCoord::new(12, 848, 1561).unwrap();
        let g256 = TargetGrid::for_tile(tile, 256);
        let g512 = TargetGrid::for_tile(tile, 512);
        assert_eq!(g256.bounds(), g512.bounds());
        assert!((g256.pixel_size().0 - 2.0 * g512.pixel_size().0).abs() < 1e-9);
    }
}
