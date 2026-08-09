// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Property tests for the Render IR evaluator: the invariants no plan is
//! allowed to break, regardless of pixel values or expression shape.
//!
//! * rescale output always lands in `0..=255` and is monotonic in the input;
//! * band math over all-valid inputs never invalidates a pixel unless a
//!   division by zero (or non-finite result) occurs — expressions built
//!   from `+ - *` over bounded values keep every pixel valid;
//! * alpha is 0 exactly where any input is invalid (all inputs required,
//!   referenced by the ops or not);
//! * composite channels are independent — changing one band's values
//!   never changes the other two channels.

use proptest::prelude::*;
use swath_render::WarpedBuffer;
use swath_render::ir::{BandInput, Expr, OutputSpec, PixelOp, RenderPlan, TileFormat};

fn buffer(values: Vec<f64>, valid: Vec<bool>) -> WarpedBuffer {
    #[allow(clippy::cast_possible_truncation, reason = "test sizes are tiny")]
    let width = values.len() as u32;
    WarpedBuffer {
        width,
        height: 1,
        values,
        valid,
    }
}

fn all_valid(values: Vec<f64>) -> WarpedBuffer {
    let valid = vec![true; values.len()];
    buffer(values, valid)
}

fn png_output() -> OutputSpec {
    OutputSpec::new(TileFormat::Png)
}

/// Alpha bytes of an RGBA buffer.
fn alphas(pixels: &[u8]) -> Vec<u8> {
    pixels.iter().skip(3).step_by(4).copied().collect()
}

/// One RGB channel (0 = R, 1 = G, 2 = B) of an RGBA buffer.
fn channel(pixels: &[u8], idx: usize) -> Vec<u8> {
    pixels.iter().skip(idx).step_by(4).copied().collect()
}

/// Expressions over bands `a`/`b` and bounded constants, `+ - *` only —
/// the division-free fragment that must never invalidate a valid pixel.
fn div_free_expr() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        Just(Expr::band("a")),
        Just(Expr::band("b")),
        (-100.0..100.0_f64).prop_map(Expr::Const),
    ];
    leaf.prop_recursive(3, 24, 2, |inner| {
        (inner.clone(), inner, 0..3_u8).prop_map(|(lhs, rhs, op)| match op {
            0 => lhs + rhs,
            1 => lhs - rhs,
            _ => lhs * rhs,
        })
    })
}

proptest! {
    /// Rescale output is always in range (alpha 255, channels arbitrary but
    /// quantized) and monotonic: sorted inputs produce sorted outputs.
    #[test]
    fn rescale_clamps_and_is_monotonic(
        mut values in proptest::collection::vec(-1e6..1e6_f64, 1..64),
        (min, max) in (-1e5..1e5_f64, 1e-3..1e5_f64).prop_map(|(lo, span)| (lo, lo + span)),
    ) {
        values.sort_by(f64::total_cmp);
        let plan = RenderPlan::new(
            vec![BandInput::new("a")],
            vec![
                PixelOp::BandMath(Expr::band("a")),
                PixelOp::Rescale { min, max },
            ],
            png_output(),
        );
        let tile = eval_ok(&plan, &[all_valid(values)]);
        let gray = channel(&tile.pixels, 0);
        // u8 is in range by construction; monotonicity is the real claim.
        let mut sorted = gray.clone();
        sorted.sort_unstable();
        prop_assert_eq!(&gray, &sorted);
        prop_assert!(alphas(&tile.pixels).iter().all(|&a| a == 255));
    }

    /// Division-free band math over all-valid, bounded inputs keeps every
    /// pixel valid: no fabricated invalidity.
    #[test]
    fn div_free_band_math_never_invalidates(
        expr in div_free_expr(),
        a in proptest::collection::vec(-100.0..100.0_f64, 8),
        b in proptest::collection::vec(-100.0..100.0_f64, 8),
    ) {
        let plan = RenderPlan::new(
            vec![BandInput::new("a"), BandInput::new("b")],
            vec![PixelOp::BandMath(expr)],
            png_output(),
        );
        let tile = eval_ok(&plan, &[all_valid(a), all_valid(b)]);
        prop_assert!(alphas(&tile.pixels).iter().all(|&alpha| alpha == 255));
    }

    /// Division by an exact-zero denominator invalidates exactly the pixels
    /// where the denominator is zero.
    #[test]
    fn divide_by_zero_invalidates_exactly_those_pixels(
        denoms in proptest::collection::vec(prop_oneof![Just(0.0), -100.0..100.0_f64], 1..32),
    ) {
        let plan = RenderPlan::new(
            vec![BandInput::new("a"), BandInput::new("b")],
            vec![PixelOp::BandMath(Expr::band("a") / Expr::band("b"))],
            png_output(),
        );
        let numerator = all_valid(vec![1.0; denoms.len()]);
        let tile = eval_ok(&plan, &[numerator, all_valid(denoms.clone())]);
        let expect: Vec<u8> = denoms.iter().map(|&d| if d == 0.0 { 0 } else { 255 }).collect();
        prop_assert_eq!(alphas(&tile.pixels), expect);
    }

    /// Alpha is 0 exactly where any input is invalid — including inputs no
    /// op references (all inputs are required).
    #[test]
    fn alpha_is_zero_exactly_where_any_input_invalid(
        valid_a in proptest::collection::vec(any::<bool>(), 16),
        valid_b in proptest::collection::vec(any::<bool>(), 16),
    ) {
        let plan = RenderPlan::new(
            vec![BandInput::new("a"), BandInput::new("unreferenced")],
            vec![PixelOp::BandMath(Expr::band("a"))],
            png_output(),
        );
        let a = buffer(vec![1.0; 16], valid_a.clone());
        let b = buffer(vec![1.0; 16], valid_b.clone());
        let tile = eval_ok(&plan, &[a, b]);
        let expect: Vec<u8> = valid_a
            .iter()
            .zip(&valid_b)
            .map(|(&va, &vb)| if va && vb { 255 } else { 0 })
            .collect();
        prop_assert_eq!(alphas(&tile.pixels), expect);
    }

    /// Composite channels are independent: perturbing the green band's
    /// values leaves the red and blue channels byte-identical.
    #[test]
    fn composite_channels_are_independent(
        r in proptest::collection::vec(0.0..4000.0_f64, 8),
        g in proptest::collection::vec(0.0..4000.0_f64, 8),
        g2 in proptest::collection::vec(0.0..4000.0_f64, 8),
        b in proptest::collection::vec(0.0..4000.0_f64, 8),
    ) {
        let plan = RenderPlan::new(
            vec![BandInput::new("r"), BandInput::new("g"), BandInput::new("b")],
            vec![
                PixelOp::Composite { r: "r".into(), g: "g".into(), b: "b".into() },
                PixelOp::Rescale { min: 0.0, max: 3000.0 },
            ],
            png_output(),
        );
        let one = eval_ok(&plan, &[all_valid(r.clone()), all_valid(g), all_valid(b.clone())]);
        let two = eval_ok(&plan, &[all_valid(r), all_valid(g2), all_valid(b)]);
        prop_assert_eq!(channel(&one.pixels, 0), channel(&two.pixels, 0));
        prop_assert_eq!(channel(&one.pixels, 2), channel(&two.pixels, 2));
        prop_assert_eq!(alphas(&one.pixels), alphas(&two.pixels));
    }
}

fn eval_ok(plan: &RenderPlan, inputs: &[WarpedBuffer]) -> swath_render::RgbaTile {
    swath_render::eval(plan, inputs).expect("plan evaluates")
}
