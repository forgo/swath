// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The target pixel grid a warp renders into.

/// An axis-aligned bounding box in the target CRS's native units,
/// y growing northward (up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridBounds {
    /// Western edge.
    pub min_x: f64,
    /// Southern edge.
    pub min_y: f64,
    /// Eastern edge.
    pub max_x: f64,
    /// Northern edge.
    pub max_y: f64,
}

/// A regular pixel grid over a target-CRS bounding box — the destination
/// of a warp: `width × height` pixels spanning `bounds`, row 0 at the
/// northern edge (top-left origin, the raster convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetGrid {
    bounds: GridBounds,
    width: u32,
    height: u32,
}

impl TargetGrid {
    /// A grid of `width × height` pixels over `bounds`.
    #[must_use]
    pub const fn new(bounds: GridBounds, width: u32, height: u32) -> Self {
        Self {
            bounds,
            width,
            height,
        }
    }

    /// The grid's bounds in the target CRS.
    #[must_use]
    pub const fn bounds(&self) -> GridBounds {
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

    /// Ground size of one pixel in CRS units `(dx, dy)`; both positive.
    #[must_use]
    pub fn pixel_size(&self) -> (f64, f64) {
        (
            (self.bounds.max_x - self.bounds.min_x) / f64::from(self.width),
            (self.bounds.max_y - self.bounds.min_y) / f64::from(self.height),
        )
    }

    /// Target-CRS coordinates of the **center** of pixel `(col, row)` —
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
    use super::{GridBounds, TargetGrid};

    #[test]
    fn pixel_centers_span_the_grid() {
        let b = GridBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 256.0,
            max_y: 256.0,
        };
        let grid = TargetGrid::new(b, 256, 256);
        let (dx, dy) = grid.pixel_size();
        assert!(dx > 0.0 && dy > 0.0);
        // First center is half a pixel inside the top-left corner…
        let (x0, y0) = grid.pixel_center(0, 0);
        assert!((x0 - (b.min_x + dx / 2.0)).abs() < 1e-9);
        assert!((y0 - (b.max_y - dy / 2.0)).abs() < 1e-9);
        // …and the last is half a pixel inside the bottom-right corner.
        let (x1, y1) = grid.pixel_center(255, 255);
        assert!((x1 - (b.max_x - dx / 2.0)).abs() < 1e-9);
        assert!((y1 - (b.min_y + dy / 2.0)).abs() < 1e-9);
    }
}
