// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Golden tests: the Swath warp kernels vs the GDAL/rio-tiler oracle
//! (ADR 0002 — GDAL lives only in the test suite, as the correctness bar).
//!
//! Each case renders one 256-px XYZ tile of a committed HLS fixture band
//! through the full pipeline (`swath-source-cog` read → `proj4rs`
//! 3857→UTM transform → `swath-render` warp → oracle-identical grayscale
//! encode) and perceptually diffs it against a committed reference tile
//! rendered by rio-tiler/GDAL from the same fixture. All comparisons must
//! pass the **default** `swath-testsupport` policy (tolerance 2/255, ≤0.5% bad
//! pixels); alpha is compared like any channel, so the validity mask is
//! held to the same bar as the pixel values (the swath-edge tiles are the
//! real nodata test).
//!
//! The goldens in `tests/data/` are regenerated with
//! `just render-goldens` (rio-tiler pinned via
//! `tests/oracle/render_reference.py`; bilinear warp for the continuous
//! B04 band rescaled 0..3000, nearest for the categorical Fmask band).

mod common;

use swath_core::tile::TileCoord;
use swath_render::{NodataPolicy, Resampling};

/// One golden case: fixture band, tile, kernel, oracle-matching rescale.
struct Case {
    fixture: &'static str,
    golden: &'static str,
    tile: TileCoord,
    resampling: Resampling,
    rescale: Option<(f64, f64)>,
}

const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const FMASK: &str = "hlss30-t13sdd-2024158-fmask.tif";

fn tile(z: u8, x: u32, y: u32) -> TileCoord {
    TileCoord::new(z, x, y).expect("valid tile")
}

fn b04_case(golden: &'static str, z: u8, x: u32, y: u32) -> Case {
    Case {
        fixture: B04,
        golden,
        tile: tile(z, x, y),
        resampling: Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
        rescale: Some((0.0, 3000.0)),
    }
}

fn fmask_case(golden: &'static str, z: u8, x: u32, y: u32) -> Case {
    Case {
        fixture: FMASK,
        golden,
        tile: tile(z, x, y),
        resampling: Resampling::Nearest,
        rescale: None,
    }
}

async fn assert_matches_oracle(case: Case) {
    let (warped, nodata, elapsed) =
        common::render_warped(case.fixture, case.tile, case.resampling).await;
    let ours = common::encode_like_oracle(&warped, nodata, case.rescale);
    swath_testsupport::pdiff::assert_matches_golden(
        &format!("{} (warp {elapsed:?})", case.golden),
        &ours,
        &common::goldens_dir().join(case.golden),
    );
}

// --- B04 (continuous reflectance, bilinear, rescale 0..3000) ---

#[tokio::test]
async fn b04_interior_tile_z12() {
    assert_matches_oracle(b04_case("b04-12-848-1561.png", 12, 848, 1561)).await;
}

#[tokio::test]
async fn b04_swath_edge_nodata_tile_z12() {
    assert_matches_oracle(b04_case("b04-12-848-1562.png", 12, 848, 1562)).await;
}

#[tokio::test]
async fn b04_child_tile_z13() {
    assert_matches_oracle(b04_case("b04-13-1697-3122.png", 13, 1697, 3122)).await;
}

#[tokio::test]
async fn b04_parent_tile_z11_downsamples() {
    assert_matches_oracle(b04_case("b04-11-424-780.png", 11, 424, 780)).await;
}

// --- Fmask (categorical QA, nearest, no rescale) ---

#[tokio::test]
async fn fmask_interior_tile_z12() {
    assert_matches_oracle(fmask_case("fmask-12-848-1561.png", 12, 848, 1561)).await;
}

#[tokio::test]
async fn fmask_swath_edge_nodata_tile_z12() {
    assert_matches_oracle(fmask_case("fmask-12-848-1562.png", 12, 848, 1562)).await;
}

#[tokio::test]
async fn fmask_child_tile_z13() {
    assert_matches_oracle(fmask_case("fmask-13-1697-3122.png", 13, 1697, 3122)).await;
}

#[tokio::test]
async fn fmask_parent_tile_z11_downsamples() {
    assert_matches_oracle(fmask_case("fmask-11-424-780.png", 11, 424, 780)).await;
}
