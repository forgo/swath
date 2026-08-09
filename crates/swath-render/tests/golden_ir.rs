// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Golden tests: the Render IR (composite, band math, rescale) plus PNG
//! encode vs the GDAL/rio-tiler oracle's `compose` renders (ADR 0002).
//!
//! Each case warps the needed HLS fixture bands through the merged warp
//! path (bilinear, the continuous-band kernel), evaluates a `RenderPlan`,
//! and perceptually diffs the RGBA tile against a committed oracle render
//! (`just render-goldens`, `compose` subcommand) under the **default**
//! `swath-testkit` policy. The swath-edge tiles exercise the validity →
//! alpha path across multiple bands; alpha is compared like any channel.

#[allow(
    dead_code,
    reason = "shared with golden.rs; not every helper is used here"
)]
mod common;

use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NodataPolicy, Resampling, WarpedBuffer, encode_png, eval};
use swath_testkit::{DiffPolicy, RgbaImage, diff, load_png};

const B02: &str = "hlss30-t13sdd-2024158-b02.tif";
const B03: &str = "hlss30-t13sdd-2024158-b03.tif";
const B04: &str = "hlss30-t13sdd-2024158-b04.tif";
const B8A: &str = "hlss30-t13sdd-2024158-b8a.tif";

/// The reflectance-band kernel, as in the single-band goldens.
const BILINEAR: Resampling = Resampling::Bilinear(NodataPolicy::ExcludeRenormalize);

/// The true-color plan: BGR+red fixtures composited RGB, rescaled 0..3000.
fn truecolor_plan() -> RenderPlan {
    RenderPlan::new(
        vec![
            BandInput::new("b04"),
            BandInput::new("b03"),
            BandInput::new("b02"),
        ],
        vec![
            PixelOp::Composite {
                r: "b04".into(),
                g: "b03".into(),
                b: "b02".into(),
            },
            PixelOp::Rescale {
                min: 0.0,
                max: 3000.0,
            },
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

/// The NDVI plan: `(nir - red) / (nir + red)`, rescaled -1..1, grayscale.
fn ndvi_plan() -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("b8a"), BandInput::new("b04")],
        vec![
            PixelOp::BandMath(
                (Expr::band("b8a") - Expr::band("b04")) / (Expr::band("b8a") + Expr::band("b04")),
            ),
            PixelOp::Rescale {
                min: -1.0,
                max: 1.0,
            },
            PixelOp::Colormap(Colormap::Grayscale),
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

async fn warp_bands(fixtures: &[&str], z: u8, x: u32, y: u32) -> Vec<WarpedBuffer> {
    let tile = swath_core::tile::TileCoord::new(z, x, y).expect("valid tile");
    let mut warped = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        let (buffer, _, _) = common::render_warped(fixture, tile, BILINEAR).await;
        warped.push(buffer);
    }
    warped
}

#[allow(clippy::print_stdout, reason = "diff metrics are the test's report")]
async fn assert_matches_oracle(
    plan: &RenderPlan,
    fixtures: &[&str],
    golden: &str,
    z: u8,
    x: u32,
    y: u32,
) {
    let warped = warp_bands(fixtures, z, x, y).await;
    let tile = eval(plan, &warped).expect("plan evaluates");
    let ours = RgbaImage::from_raw(tile.width, tile.height, tile.pixels.clone())
        .expect("tile buffer matches dimensions");
    let reference = load_png(&common::goldens_dir().join(golden)).expect("golden loads");

    let report = diff(&ours, &reference).expect("dimensions match");
    let policy = DiffPolicy::default();
    let bad_pct = report.pct_pixels_exceeding_tolerance(policy.per_channel_tolerance) * 100.0;
    println!(
        "{golden}: max |diff| {max}, mean |diff| {mean:.4}, bad pixels {bad_pct:.4}%",
        max = report.max_abs_channel_diff,
        mean = report.mean_abs_diff,
    );
    assert!(
        report.passes(&policy),
        "{golden}: fails default policy — max |diff| {}, {bad_pct:.4}% pixels over tolerance {}",
        report.max_abs_channel_diff,
        policy.per_channel_tolerance,
    );

    // The encoded tile is deterministic and lossless: double-encode
    // byte-identically, and decode back to exactly the evaluated pixels.
    let png_a = encode_png(&tile).expect("encodes");
    let png_b = encode_png(&tile).expect("encodes");
    assert_eq!(png_a, png_b, "{golden}: PNG encode must be deterministic");
    let decoded = image::load_from_memory(&png_a)
        .expect("decodes")
        .into_rgba8();
    assert_eq!(
        decoded.into_raw(),
        tile.pixels,
        "{golden}: PNG must round-trip losslessly"
    );
}

// --- True-color composite (b04/b03/b02 -> RGB, rescale 0..3000) ---

#[tokio::test]
async fn truecolor_interior_tile_z12() {
    assert_matches_oracle(
        &truecolor_plan(),
        &[B04, B03, B02],
        "truecolor-12-848-1561.png",
        12,
        848,
        1561,
    )
    .await;
}

#[tokio::test]
async fn truecolor_swath_edge_nodata_tile_z12() {
    assert_matches_oracle(
        &truecolor_plan(),
        &[B04, B03, B02],
        "truecolor-12-848-1562.png",
        12,
        848,
        1562,
    )
    .await;
}

// --- NDVI band math (b8a/b04, rescale -1..1, grayscale) ---

#[tokio::test]
async fn ndvi_interior_tile_z12() {
    assert_matches_oracle(
        &ndvi_plan(),
        &[B8A, B04],
        "ndvi-12-848-1561.png",
        12,
        848,
        1561,
    )
    .await;
}

#[tokio::test]
async fn ndvi_swath_edge_nodata_tile_z12() {
    assert_matches_oracle(
        &ndvi_plan(),
        &[B8A, B04],
        "ndvi-12-848-1562.png",
        12,
        848,
        1562,
    )
    .await;
}
