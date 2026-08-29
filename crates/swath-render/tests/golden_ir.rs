// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Golden tests: the Render IR (composite, band math, rescale) plus PNG
//! encode vs the GDAL/rio-tiler oracle's `compose` renders (ADR 0002).
//!
//! Each case warps the needed HLS fixture bands through the merged warp
//! path (bilinear, the continuous-band kernel), evaluates a `RenderPlan`,
//! and perceptually diffs the RGBA tile against a committed oracle render
//! (`just render-goldens`, `compose` subcommand) under the **default**
//! `swath-testsupport` policy. The swath-edge tiles exercise the validity →
//! alpha path across multiple bands; alpha is compared like any channel.

mod common;

use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{NoUdf, NodataPolicy, Resampling, WarpedBuffer, encode_png, eval};
use swath_testsupport::RgbaImage;

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
    let tile = eval(plan, &warped, &NoUdf).expect("plan evaluates");
    let ours = RgbaImage::from_raw(tile.width, tile.height, tile.pixels.clone())
        .expect("tile buffer matches dimensions");
    swath_testsupport::pdiff::assert_matches_golden(
        golden,
        &ours,
        &common::goldens_dir().join(golden),
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
//
// These two tests are level 1 of the two-level NDVI golden scheme (issue
// #94): the GDAL/rio-tiler oracle renders grayscale, so the grayscale
// plan keeps pinning the NDVI *values* against it. Level 2
// (`ndvi_rdylgn_colormap_two_level_golden` below) pins the colormapped
// presentation on top of those oracle-validated values.

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

// --- NDVI colormapped (issue #94): the two-level golden scheme ---

/// The colormapped NDVI plan the built-in `ndvi` layer now serves:
/// same band math and rescale as [`ndvi_plan`], `RdYlGn` instead of gray.
fn ndvi_rdylgn_plan() -> RenderPlan {
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
            PixelOp::Colormap(Colormap::RdYlGn),
        ],
        OutputSpec::new(TileFormat::Png),
    )
}

/// **The two-level NDVI golden scheme.** The GDAL/rio-tiler oracle
/// (`render_reference.py compose --expression`) renders grayscale; the
/// colormap is Swath's own post-processing, so no external oracle exists
/// for the colored bytes. The honest decomposition:
///
/// 1. **Values** — the grayscale render is pinned against the GDAL
///    oracle golden (`ndvi_interior_tile_z12` above, unchanged), and this
///    test asserts *mechanically* that every colormapped pixel is exactly
///    `lut[q(gray)]` of that same grayscale render (same quantization,
///    same alpha). The colors are therefore anchored to oracle-validated
///    values, not to themselves.
/// 2. **Bytes** — the colormapped tile is pinned byte-for-byte against a
///    committed golden produced by this very pipeline (regenerate with
///    `SWATH_BLESS=1`), after proving the render + encode are
///    double-run byte-stable. This is a self-golden and says nothing an
///    oracle would; its job is to freeze the served bytes for `just e2e`
///    and catch silent drift.
#[allow(clippy::print_stdout, reason = "bless mode reports what it wrote")]
#[tokio::test]
async fn ndvi_rdylgn_colormap_two_level_golden() {
    let warped = warp_bands(&[B8A, B04], 12, 848, 1561).await;
    let gray = eval(&ndvi_plan(), &warped, &NoUdf).expect("grayscale plan evaluates");
    let colored = eval(&ndvi_rdylgn_plan(), &warped, &NoUdf).expect("colormapped plan evaluates");

    // Level 1: every colormapped pixel is the LUT row of its
    // oracle-validated gray value; alpha (validity) is untouched.
    let lut = swath_render::colormaps::lut(Colormap::RdYlGn).expect("RdYlGn has a LUT");
    assert_eq!(gray.pixels.len(), colored.pixels.len());
    for (g_px, c_px) in gray
        .pixels
        .chunks_exact(4)
        .zip(colored.pixels.chunks_exact(4))
    {
        assert_eq!(c_px[3], g_px[3], "alpha must come from validity alone");
        if g_px[3] == 0 {
            assert_eq!(c_px, [0, 0, 0, 0], "invalid stays transparent black");
        } else {
            assert_eq!(&c_px[0..3], &lut[usize::from(g_px[0])], "lut[q(gray)]");
        }
    }

    // Byte stability: double render + double encode, identical bytes.
    let again = eval(&ndvi_rdylgn_plan(), &warped, &NoUdf).expect("re-evaluates");
    assert_eq!(colored.pixels, again.pixels, "eval must be deterministic");
    let png_a = encode_png(&colored).expect("encodes");
    let png_b = encode_png(&again).expect("encodes");
    assert_eq!(png_a, png_b, "PNG encode must be deterministic");

    // Level 2: the committed self-golden pins the served bytes.
    let golden_path = common::goldens_dir().join("ndvi-rdylgn-12-848-1561.png");
    if std::env::var_os("SWATH_BLESS").is_some() {
        std::fs::write(&golden_path, &png_a).expect("golden written");
        println!("blessed {}", golden_path.display());
    }
    let committed = std::fs::read(&golden_path).expect("committed colormapped golden exists");
    assert_eq!(
        png_a, committed,
        "colormapped NDVI bytes drifted from the committed golden \
         (regenerate deliberately with SWATH_BLESS=1 and inspect the diff)"
    );
}

// --- Park Fire NDVI, colormapped (issue #211): the same two-level scheme ---

/// The Park Fire fixture series' proven z13 tile (`tests/fixtures/README.md`),
/// OGC z/x/y as the render path addresses it.
const FIRE_TILE: (u8, u32, u32) = (13, 1326, 3100);

/// The two dated frames the compose e2e pins (`swath-e2e`: pre-fire and
/// fresh burn scar) and the grayscale oracle goldens their values carry.
const FIRE_DAYS: [&str; 2] = ["2024204", "2024229"];

/// **The Park Fire layer is `RdYlGn`** (the landing's opening frame, issue
/// #211), so its served frames need the two-level scheme of
/// [`ndvi_rdylgn_colormap_two_level_golden`]: per dated frame, (1) the
/// grayscale render is pinned against the rio-tiler oracle golden
/// (`fire-ndvi-13-1326-3100-<day>.png`, `just render-goldens`) and every
/// colormapped pixel is proven `lut[q(gray)]` of it; (2) the colormapped
/// bytes are frozen as a committed self-golden
/// (`fire-ndvi-rdylgn-13-1326-3100-<day>.png`, regenerate with
/// `SWATH_BLESS=1`) that `just e2e` pins the compose stack's `datetime=`
/// frames against byte-for-byte.
#[allow(clippy::print_stdout, reason = "bless mode reports what it wrote")]
#[tokio::test]
async fn fire_ndvi_rdylgn_two_level_golden_per_date() {
    let (z, x, y) = FIRE_TILE;
    let lut = swath_render::colormaps::lut(Colormap::RdYlGn).expect("RdYlGn has a LUT");
    for day in FIRE_DAYS {
        let b8a = format!("hlss30-t10tfk-{day}-b8a.tif");
        let b04 = format!("hlss30-t10tfk-{day}-b04.tif");
        let fixtures = [b8a.as_str(), b04.as_str()];
        // Level 1: values against the oracle.
        assert_matches_oracle(
            &ndvi_plan(),
            &fixtures,
            &format!("fire-ndvi-13-1326-3100-{day}.png"),
            z,
            x,
            y,
        )
        .await;
        let warped = warp_bands(&fixtures, z, x, y).await;
        let gray = eval(&ndvi_plan(), &warped, &NoUdf).expect("grayscale plan evaluates");
        let colored =
            eval(&ndvi_rdylgn_plan(), &warped, &NoUdf).expect("colormapped plan evaluates");
        assert_eq!(gray.pixels.len(), colored.pixels.len());
        for (g_px, c_px) in gray
            .pixels
            .chunks_exact(4)
            .zip(colored.pixels.chunks_exact(4))
        {
            assert_eq!(
                c_px[3], g_px[3],
                "{day}: alpha must come from validity alone"
            );
            if g_px[3] == 0 {
                assert_eq!(c_px, [0, 0, 0, 0], "{day}: invalid stays transparent black");
            } else {
                assert_eq!(
                    &c_px[0..3],
                    &lut[usize::from(g_px[0])],
                    "{day}: lut[q(gray)]"
                );
            }
        }
        // Byte stability, then level 2: the committed self-golden.
        let again = eval(&ndvi_rdylgn_plan(), &warped, &NoUdf).expect("re-evaluates");
        assert_eq!(
            colored.pixels, again.pixels,
            "{day}: eval must be deterministic"
        );
        let png_a = encode_png(&colored).expect("encodes");
        let png_b = encode_png(&again).expect("encodes");
        assert_eq!(png_a, png_b, "{day}: PNG encode must be deterministic");
        let golden_path =
            common::goldens_dir().join(format!("fire-ndvi-rdylgn-13-1326-3100-{day}.png"));
        if std::env::var_os("SWATH_BLESS").is_some() {
            std::fs::write(&golden_path, &png_a).expect("golden written");
            println!("blessed {}", golden_path.display());
        }
        let committed =
            std::fs::read(&golden_path).expect("committed colormapped fire golden exists");
        assert_eq!(
            png_a, committed,
            "{day}: colormapped fire bytes drifted from the committed golden \
             (regenerate deliberately with SWATH_BLESS=1 and inspect the diff)"
        );
    }
}
