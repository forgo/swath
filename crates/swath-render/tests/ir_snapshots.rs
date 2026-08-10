// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Snapshot tests (insta): pin the Render IR's serde JSON shape and a tiny
//! evaluation's exact RGBA bytes.
//!
//! The JSON shape is a contract: the process compiler (`swath_render::process`) emits it
//! and the test suite round-trips it, so an accidental rename or enum
//! re-tagging must fail loudly here — not in a downstream consumer.

use swath_render::ir::{BandInput, Colormap, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};
use swath_render::{WarpedBuffer, eval};

/// The canonical NDVI plan: band math, rescale, and — since issue #94 —
/// the diverging `RdYlGn` colormap the built-in NDVI layer defaults to.
fn ndvi_plan() -> RenderPlan {
    RenderPlan::new(
        vec![BandInput::new("nir"), BandInput::new("red")],
        vec![
            PixelOp::BandMath(
                (Expr::band("nir") - Expr::band("red")) / (Expr::band("nir") + Expr::band("red")),
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

/// The canonical true-color plan: composite then rescale.
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

#[test]
fn ndvi_plan_json_shape() {
    insta::assert_json_snapshot!(ndvi_plan());
}

#[test]
fn truecolor_plan_json_shape() {
    insta::assert_json_snapshot!(truecolor_plan());
}

#[test]
fn plans_round_trip_through_json() {
    for plan in [ndvi_plan(), truecolor_plan()] {
        let json = serde_json::to_string(&plan).expect("serializes");
        let back: RenderPlan = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back, plan);
    }
}

/// A 4x4 NDVI evaluation with one invalid input pixel and one
/// divide-by-zero pixel: the exact RGBA bytes are pinned.
#[test]
fn tiny_eval_rgba_bytes() {
    let width = 4;
    let height = 4;
    // nir ramps 0..1500 by row, red fixed at 500 — NDVI varies by row.
    let nir_values: Vec<f64> = (0..16).map(|i| f64::from(i / 4) * 500.0).collect();
    let red_values = vec![500.0; 16];
    let mut nir = WarpedBuffer {
        width,
        height,
        values: nir_values,
        valid: vec![true; 16],
    };
    let red = WarpedBuffer {
        width,
        height,
        values: red_values,
        valid: vec![true; 16],
    };
    // Pixel 5: invalid input. Pixel 0 stays valid; its denominator is
    // 0 + 500, fine. Make pixel 3 a divide-by-zero: nir = -500.
    nir.valid[5] = false;
    nir.values[3] = -500.0;

    let tile = eval(&ndvi_plan(), &[nir, red]).expect("plan evaluates");
    insta::assert_debug_snapshot!((tile.width, tile.height, tile.pixels));
}
