// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Golden-pixel tests for the colormap engine (issue #94): every palette
//! variant, exercised through [`eval`], asserting **exact RGBA** at five
//! sample stops against values pinned from the reference palette —
//! matplotlib 3.10.3's published 256-entry byte LUTs, vendored under
//! `src/colormaps/` (the pinned-oracle pattern). The literals below were
//! read off matplotlib directly (the regeneration recipe is in
//! `src/colormaps/README.md`), so they hold the committed JSON *and* the
//! engine's indexing to the reference at once.
//!
//! Also pinned here: the quantized-index semantics (clamp to `0..=255`,
//! truncate toward zero, **no interpolation** — the exact arithmetic of
//! the final gray quantization), alpha still coming from validity alone,
//! and the palette-needs-gray plan error.

use swath_render::ir::{
    BandInput, Colormap, Expr, OutputSpec, PixelOp, PlanError, RenderPlan, TileFormat,
};
use swath_render::{NoUdf, WarpedBuffer, eval};

/// The five sample stops every variant is pinned at.
const STOPS: [f64; 5] = [0.0, 64.0, 128.0, 192.0, 255.0];

/// A plan that feeds band `v` straight into `map`: the input values are
/// already in the 0..=255 index domain, so the expected pixels are the
/// LUT rows themselves.
fn colormap_plan(map: Colormap) -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("v")],
        vec![PixelOp::BandMath(Expr::band("v")), PixelOp::Colormap(map)],
        OutputSpec::new(TileFormat::Png),
    )
}

fn buffer(values: &[f64]) -> WarpedBuffer {
    WarpedBuffer {
        width: u32::try_from(values.len()).expect("test buffer fits"),
        height: 1,
        values: values.to_vec(),
        valid: vec![true; values.len()],
    }
}

/// Renders `values` through `map` and returns the RGBA bytes.
fn render(map: Colormap, values: &[f64]) -> Vec<u8> {
    eval(&colormap_plan(map), &[buffer(values)], &NoUdf)
        .expect("plan evaluates")
        .pixels
}

/// Asserts the five stops render to exactly `expected` (RGB, alpha 255).
fn assert_stops(map: Colormap, expected: [[u8; 3]; 5]) {
    let pixels = render(map, &STOPS);
    for (i, [r, g, b]) in expected.into_iter().enumerate() {
        let at = i * 4;
        assert_eq!(
            &pixels[at..at + 4],
            &[r, g, b, 255],
            "{map:?} at stop {}",
            STOPS[i]
        );
    }
}

// --- Exact RGBA at 5 stops per variant, pinned from matplotlib 3.10.3 ---

#[test]
fn viridis_matches_the_matplotlib_lut_at_five_stops() {
    assert_stops(
        Colormap::Viridis,
        [
            [68, 1, 84],    // 0
            [58, 82, 139],  // 64
            [32, 144, 140], // 128
            [94, 201, 97],  // 192
            [253, 231, 36], // 255
        ],
    );
}

#[test]
fn magma_matches_the_matplotlib_lut_at_five_stops() {
    assert_stops(
        Colormap::Magma,
        [
            [0, 0, 3],       // 0
            [80, 18, 123],   // 64
            [182, 54, 121],  // 128
            [251, 136, 97],  // 192
            [251, 252, 191], // 255
        ],
    );
}

#[test]
fn rdylgn_matches_the_matplotlib_lut_at_five_stops() {
    assert_stops(
        Colormap::RdYlGn,
        [
            [165, 0, 38],    // 0: strong red (NDVI -1: barren)
            [248, 142, 82],  // 64
            [254, 254, 189], // 128: yellow midpoint (NDVI ~0)
            [132, 202, 102], // 192
            [0, 104, 55],    // 255: strong green (NDVI +1: dense canopy)
        ],
    );
}

#[test]
fn grayscale_stays_the_identity_map() {
    let pixels = render(Colormap::Grayscale, &STOPS);
    for (i, stop) in STOPS.iter().enumerate() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "stops are exact u8 values"
        )]
        let v = *stop as u8;
        assert_eq!(&pixels[i * 4..i * 4 + 4], &[v, v, v, 255]);
    }
}

// --- Indexing semantics: quantize, don't interpolate --------------------

/// The LUT index is the quantized gray value — `clamp(0.0, 255.0)`, then
/// truncate toward zero — so a fractional gray hits the same entry its
/// grayscale render would have quantized to, and out-of-range values pin
/// to the ends. No interpolation between entries.
#[test]
fn palette_indexing_quantizes_like_the_grayscale_path() {
    let pixels = render(Colormap::Viridis, &[64.0, 64.49, 64.99, -3.0, 300.0]);
    let entry_64 = [58, 82, 139, 255];
    assert_eq!(&pixels[0..4], &entry_64, "exact index");
    assert_eq!(&pixels[4..8], &entry_64, "truncates, never rounds up");
    assert_eq!(&pixels[8..12], &entry_64, "no interpolation toward 65");
    assert_eq!(&pixels[12..16], &[68, 1, 84, 255], "clamps to entry 0");
    assert_eq!(&pixels[16..20], &[253, 231, 36, 255], "clamps to entry 255");
}

// --- Validity and plan errors -------------------------------------------

/// Alpha comes from validity alone: an invalid pixel stays transparent
/// black even where the palette maps index 0 to a saturated color.
#[test]
fn invalid_pixels_stay_transparent_black_under_a_palette() {
    let mut input = buffer(&[128.0, 128.0]);
    input.valid[1] = false;
    let tile = eval(&colormap_plan(Colormap::RdYlGn), &[input], &NoUdf).expect("plan evaluates");
    assert_eq!(&tile.pixels[0..4], &[254, 254, 189, 255]);
    assert_eq!(&tile.pixels[4..8], &[0, 0, 0, 0]);
}

/// A palette needs gray planes: applying one to a composite is a plan
/// error (grayscale, the identity, remains allowed anywhere).
#[test]
fn palette_over_a_composite_is_a_plan_error() {
    let plan = RenderPlan::new(
        vec![
            BandInput::new("r"),
            BandInput::new("g"),
            BandInput::new("b"),
        ],
        vec![
            PixelOp::Composite {
                r: "r".into(),
                g: "g".into(),
                b: "b".into(),
            },
            PixelOp::Colormap(Colormap::Viridis),
        ],
        OutputSpec::new(TileFormat::Png),
    );
    let bands = [buffer(&[1.0]), buffer(&[2.0]), buffer(&[3.0])];
    assert_eq!(
        eval(&plan, &bands, &NoUdf),
        Err(PlanError::ColormapNeedsGray {
            map: Colormap::Viridis
        })
    );
}
