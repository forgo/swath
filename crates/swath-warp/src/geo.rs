// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The affine pixel↔CRS mapping of a source grid.

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
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// `(col, row)` — the inverse of [`Self::pixel_to_crs`]. `None` when the
    /// linear part is singular (determinant exactly zero, e.g. a zero-sized
    /// pixel).
    #[must_use]
    pub fn crs_to_pixel(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let det = self.determinant();
        if det == 0.0 {
            return None;
        }
        let dx = x - self.origin_x;
        let dy = y - self.origin_y;
        Some((
            self.pixel_height.mul_add(dx, -(self.row_rotation * dy)) / det,
            self.pixel_width.mul_add(dy, -(self.col_rotation * dx)) / det,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::GeoTransform;

    #[test]
    fn transform_round_trips_a_corner() {
        // The HLS-shaped case: EPSG:32613, 30 m pixels, top-left (453720, 4353960).
        let gt = GeoTransform::north_up(453_720.0, 4_353_960.0, 30.0, -30.0);
        assert_eq!(gt.pixel_to_crs(0.0, 0.0), (453_720.0, 4_353_960.0));
        assert_eq!(gt.pixel_to_crs(512.0, 512.0), (469_080.0, 4_338_600.0));
        let (col, row) = gt.crs_to_pixel(469_080.0, 4_338_600.0).unwrap();
        assert!((col - 512.0).abs() < 1e-9);
        assert!((row - 512.0).abs() < 1e-9);
    }

    #[test]
    fn singular_transform_refuses_inversion() {
        let gt = GeoTransform::north_up(0.0, 0.0, 30.0, 0.0);
        assert_eq!(gt.crs_to_pixel(1.0, 1.0), None);
    }
}
