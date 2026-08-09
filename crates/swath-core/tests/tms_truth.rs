// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Known-answer tests: `WebMercatorQuad` bounds vs morecantile ground truth.
//!
//! `data/tms_truth.json` is generated ONCE by `tests/oracle/tms_truth.py`
//! (morecantile pinned there) and committed; the truth is pinned — CI asserts
//! against the committed JSON, not against morecantile-at-HEAD. Regenerating
//! requires rerunning the script and reviewing the diff.
//!
//! Tolerances: 1e-6 m on Web Mercator bounds (the issue #21 bar; morecantile
//! itself carries ~2e-8 m float noise on the zero midlines), and 1e-9 degrees
//! on geographic bounds.

use swath_core::tile::{TileCoord, WebMercatorQuad};

/// One row of the committed truth table.
#[derive(serde::Deserialize)]
struct TruthTile {
    z: u8,
    x: u32,
    y: u32,
    xy_bounds: XyBounds,
    lonlat_bounds: LlBounds,
}

#[derive(serde::Deserialize)]
struct XyBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

#[derive(serde::Deserialize)]
struct LlBounds {
    west: f64,
    south: f64,
    east: f64,
    north: f64,
}

#[derive(serde::Deserialize)]
struct TruthTable {
    tms: String,
    tiles: Vec<TruthTile>,
}

fn truth() -> TruthTable {
    let raw = include_str!("data/tms_truth.json");
    serde_json::from_str(raw).expect("committed truth table parses")
}

#[track_caller]
fn assert_close(actual: f64, expected: f64, tol: f64, what: &str, tile: TileCoord) {
    assert!(
        (actual - expected).abs() <= tol,
        "{tile}: {what} = {actual}, morecantile says {expected} (tol {tol})"
    );
}

#[test]
fn truth_table_covers_the_expected_tiles() {
    let table = truth();
    assert_eq!(table.tms, "WebMercatorQuad");
    // Root, all four z1 quadrants, the synthetic oracle tile, the HLS fixture tile.
    assert_eq!(table.tiles.len(), 7);
}

#[test]
fn xy_bounds_match_morecantile_within_1e6_m() {
    for t in truth().tiles {
        let tile = TileCoord::new(t.z, t.x, t.y).expect("truth tiles are valid");
        let b = WebMercatorQuad::xy_bounds(tile);
        assert_close(b.min_x, t.xy_bounds.min_x, 1e-6, "min_x", tile);
        assert_close(b.min_y, t.xy_bounds.min_y, 1e-6, "min_y", tile);
        assert_close(b.max_x, t.xy_bounds.max_x, 1e-6, "max_x", tile);
        assert_close(b.max_y, t.xy_bounds.max_y, 1e-6, "max_y", tile);
    }
}

#[test]
fn lonlat_bounds_match_morecantile_within_1e9_deg() {
    for t in truth().tiles {
        let tile = TileCoord::new(t.z, t.x, t.y).expect("truth tiles are valid");
        let b = WebMercatorQuad::lonlat_bounds(tile);
        assert_close(b.west, t.lonlat_bounds.west, 1e-9, "west", tile);
        assert_close(b.south, t.lonlat_bounds.south, 1e-9, "south", tile);
        assert_close(b.east, t.lonlat_bounds.east, 1e-9, "east", tile);
        assert_close(b.north, t.lonlat_bounds.north, 1e-9, "north", tile);
    }
}
