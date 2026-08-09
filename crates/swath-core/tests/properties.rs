// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Property tests (proptest, per ENGINEERING.md §2) for the swath-core
//! domain math: tile pyramid geometry, geotransform inversion, and window
//! algebra. These run on every PR — they are the regression suite issue #21
//! asks for.

use proptest::prelude::{ProptestConfig, Strategy, prop_assert, prop_assert_eq, proptest};
use swath_core::raster::{GeoTransform, WindowRequest};
use swath_core::tile::{TileCoord, WebMercatorQuad};

/// Absolute tolerance in meters for Web Mercator edge comparisons — well
/// below a pixel at any practical zoom, well above f64 rounding noise.
const M_TOL: f64 = 1e-6;

/// Any valid tile at zoom 0..=24 (the practical `WebMercatorQuad` range; the
/// synthetic oracle and fixtures live well inside it).
fn arb_tile() -> impl Strategy<Value = TileCoord> {
    (0u8..=24).prop_flat_map(|z| {
        let max = 1u32 << z;
        (0..max, 0..max).prop_map(move |(x, y)| TileCoord::new(z, x, y).expect("in range"))
    })
}

/// An invertible geotransform with the determinant bounded away from zero
/// (|`pixel_width` * `pixel_height`| >= 0.01, |rotations product| <= 0.0025), so
/// inversion is well-conditioned and a fixed tolerance is honest.
fn arb_geotransform() -> impl Strategy<Value = GeoTransform> {
    (
        -1.0e6..1.0e6f64,
        0.1..1.0e3f64,
        -0.05..0.05f64,
        -1.0e6..1.0e6f64,
        -0.05..0.05f64,
        -1.0e3..-0.1f64,
    )
        .prop_map(
            |(origin_x, pixel_width, row_rotation, origin_y, col_rotation, pixel_height)| {
                GeoTransform {
                    origin_x,
                    pixel_width,
                    row_rotation,
                    origin_y,
                    col_rotation,
                    pixel_height,
                }
            },
        )
}

/// Windows small enough that offset+size never saturates.
fn arb_window() -> impl Strategy<Value = WindowRequest> {
    (0u64..1_000_000, 0u64..1_000_000, 0u64..10_000, 0u64..10_000).prop_map(
        |(col_off, row_off, width, height)| WindowRequest {
            col_off,
            row_off,
            width,
            height,
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every child's bounds nest within its parent's bounds.
    #[test]
    fn child_bounds_nest_within_parent(tile in arb_tile()) {
        let parent = WebMercatorQuad::xy_bounds(tile);
        if let Some(children) = tile.children() {
            for child in children {
                let b = WebMercatorQuad::xy_bounds(child);
                prop_assert!(parent.min_x - M_TOL <= b.min_x, "{child} west leaks from {tile}");
                prop_assert!(parent.min_y - M_TOL <= b.min_y, "{child} south leaks from {tile}");
                prop_assert!(b.max_x <= parent.max_x + M_TOL, "{child} east leaks from {tile}");
                prop_assert!(b.max_y <= parent.max_y + M_TOL, "{child} north leaks from {tile}");
            }
        }
    }

    /// The four children partition the parent exactly: outer edges coincide
    /// with the parent's, inner edges meet on shared midlines.
    #[test]
    fn children_partition_parent_exactly(tile in arb_tile()) {
        let Some([nw, ne, sw, se]) = tile.children() else { return Ok(()) };
        let p = WebMercatorQuad::xy_bounds(tile);
        let (bnw, bne, bsw, bse) = (
            WebMercatorQuad::xy_bounds(nw),
            WebMercatorQuad::xy_bounds(ne),
            WebMercatorQuad::xy_bounds(sw),
            WebMercatorQuad::xy_bounds(se),
        );
        // Outer edges are the parent's edges.
        prop_assert!((bnw.min_x - p.min_x).abs() <= M_TOL);
        prop_assert!((bnw.max_y - p.max_y).abs() <= M_TOL);
        prop_assert!((bse.max_x - p.max_x).abs() <= M_TOL);
        prop_assert!((bse.min_y - p.min_y).abs() <= M_TOL);
        // Inner edges meet: no gaps, no overlaps.
        prop_assert!((bnw.max_x - bne.min_x).abs() <= M_TOL);
        prop_assert!((bsw.max_x - bse.min_x).abs() <= M_TOL);
        prop_assert!((bnw.min_y - bsw.max_y).abs() <= M_TOL);
        prop_assert!((bne.min_y - bse.max_y).abs() <= M_TOL);
        // Same column/row alignment across the split.
        prop_assert!((bnw.min_x - bsw.min_x).abs() <= M_TOL);
        prop_assert!((bne.max_x - bse.max_x).abs() <= M_TOL);
    }

    /// tile -> bounds -> center point -> tile is the identity.
    #[test]
    fn xyz_bounds_round_trip(tile in arb_tile()) {
        let (cx, cy) = WebMercatorQuad::xy_bounds(tile).center();
        let back = WebMercatorQuad::tile_for_xy(cx, cy, tile.z).expect("z is valid");
        prop_assert_eq!(back, tile);
    }

    /// crs_to_pixel(pixel_to_crs(p)) == p within f64 tolerance for
    /// well-conditioned transforms.
    #[test]
    fn geotransform_forward_inverse_identity(
        gt in arb_geotransform(),
        col in -1.0e3..1.0e3f64,
        row in -1.0e3..1.0e3f64,
    ) {
        let (x, y) = gt.pixel_to_crs(col, row);
        let (col2, row2) = gt.crs_to_pixel(x, y).expect("determinant bounded away from zero");
        prop_assert!((col2 - col).abs() <= 1e-6, "col {col} -> {col2}");
        prop_assert!((row2 - row).abs() <= 1e-6, "row {row} -> {row2}");
    }

    /// Window intersection is commutative, and the intersection is contained
    /// in both operands.
    #[test]
    fn window_intersection_commutative_and_contained(
        a in arb_window(),
        b in arb_window(),
    ) {
        prop_assert_eq!(a.intersection(&b), b.intersection(&a));
        if let Some(i) = a.intersection(&b) {
            prop_assert!(!i.is_empty());
            prop_assert!(a.contains(&i));
            prop_assert!(b.contains(&i));
        }
    }

    /// A non-empty window intersected with itself is itself; and containment
    /// holds reflexively.
    #[test]
    fn window_self_intersection_is_identity(w in arb_window()) {
        prop_assert!(w.contains(&w));
        if w.is_empty() {
            prop_assert_eq!(w.intersection(&w), None);
        } else {
            prop_assert_eq!(w.intersection(&w), Some(w));
        }
    }
}
