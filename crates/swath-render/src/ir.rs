// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The Render IR: the typed pixel-op pipeline the tiler executes between
//! the warp kernel and the tile encoder (ARCHITECTURE.md §5).
//!
//! A [`RenderPlan`] names its input bands, lists the [`PixelOp`]s that turn
//! warped `f64` planes into an 8-bit RGB image, and states the output
//! encoding. The plan is **data**: the process compiler ([`crate::process`]) lowers
//! openEO graphs into it, tests serialize it, and [`eval`] executes it — the
//! graph is interchange, the IR is ours. Every type here derives serde for
//! that reason, and the JSON shape is pinned by insta snapshots.
//!
//! # Pipeline model
//!
//! [`eval`] runs the ops in order over an RGB triple of `f64` planes plus a
//! validity mask. [`PixelOp::BandMath`] and [`PixelOp::Composite`]
//! *produce* planes (band math yields gray — all three channels equal;
//! composite selects three input bands); [`PixelOp::Rescale`] and
//! [`PixelOp::Colormap`] *transform* the current planes and are errors
//! before any producing op. After the last op, values are quantized to
//! `u8` with numpy's `astype(uint8)` semantics (clamp to `0..=255`, then
//! truncate toward zero) — the exact arithmetic of the rio-tiler oracle,
//! so goldens compare bit-for-bit-comparable pixels.
//!
//! # Validity semantics (the alpha channel)
//!
//! Every input band in [`RenderPlan::inputs`] is **required**: an output
//! pixel is transparent iff *any* input band is invalid there, whether or
//! not an expression references that band. (Optional inputs — e.g. a QA
//! band that masks but never contributes values — are deliberate future
//! growth; `BandInput` is `#[non_exhaustive]` for exactly that field.)
//! Band math adds two invalidity sources of its own, per pixel:
//!
//! * **division by an exact-zero denominator** — the pixel becomes invalid
//!   rather than propagating `NaN`/`inf` into the image (NDVI over a
//!   zero-reflectance denominator is *no data*, not a number); and
//! * a **non-finite result** (overflow, or a non-finite constant in the
//!   expression) — same reasoning.
//!
//! Invalid pixels encode as fully transparent black `(0, 0, 0, 0)`, the
//! same bytes the oracle produces (nodata clamps to the rescale floor and
//! the mask zeroes alpha), so perceptual diffs hold color channels under
//! invalid pixels to the same bar as valid ones.
//!
//! # Colormaps
//!
//! Real colormaps (viridis and friends) are deferred: they are pure lookup
//! tables with no architectural weight, and pulling in palette data now
//! would grow this crate without exercising any new pipeline semantics.
//! [`Colormap::Grayscale`] — the identity map — pins the op's position in
//! the IR (after rescale, before encode) so the compiler can target it
//! today and palettes can land as new variants without reshaping plans.

use serde::{Deserialize, Serialize};

use crate::warp::WarpedBuffer;

/// One named input band of a [`RenderPlan`].
///
/// The name is how [`Expr::Band`] and [`PixelOp::Composite`] refer to the
/// band; the buffer itself is passed to [`eval`] positionally (the i-th
/// `WarpedBuffer` is the i-th input). All inputs are currently required
/// (see the module docs on validity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BandInput {
    /// The name expressions and composites use to reference this band.
    pub name: String,
}

impl BandInput {
    /// A required input band named `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// The arithmetic operators [`Expr::Binary`] supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BinaryOp {
    /// `lhs + rhs`.
    Add,
    /// `lhs - rhs`.
    Sub,
    /// `lhs * rhs`.
    Mul,
    /// `lhs / rhs`; an exact-zero `rhs` invalidates the pixel (module docs).
    Div,
}

/// A tiny arithmetic AST over band references and constants — the whole
/// band-math language, by design. NDVI is the shape it must express:
/// `(nir - red) / (nir + red)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Expr {
    /// The named input band's value at this pixel.
    Band(String),
    /// A constant, the same at every pixel.
    Const(f64),
    /// A binary operation on two subexpressions.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
}

/// Expressions compose with the ordinary arithmetic operators:
/// `(nir - red) / (nir + red)` is written exactly like that.
impl std::ops::Add for Expr {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::binary(BinaryOp::Add, self, rhs)
    }
}

impl std::ops::Sub for Expr {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::binary(BinaryOp::Sub, self, rhs)
    }
}

impl std::ops::Mul for Expr {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::binary(BinaryOp::Mul, self, rhs)
    }
}

/// Division by an exact-zero denominator invalidates the pixel (module docs).
impl std::ops::Div for Expr {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        Self::binary(BinaryOp::Div, self, rhs)
    }
}

impl Expr {
    /// The named band's value.
    pub fn band(name: impl Into<String>) -> Self {
        Self::Band(name.into())
    }

    fn binary(op: BinaryOp, lhs: Self, rhs: Self) -> Self {
        Self::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    /// Evaluates the expression at one pixel; `None` on division by an
    /// exact-zero denominator.
    fn eval_at(&self, resolve: &impl Fn(&str) -> f64) -> Option<f64> {
        match self {
            Self::Band(name) => Some(resolve(name)),
            Self::Const(c) => Some(*c),
            Self::Binary { op, lhs, rhs } => {
                let l = lhs.eval_at(resolve)?;
                let r = rhs.eval_at(resolve)?;
                match op {
                    BinaryOp::Add => Some(l + r),
                    BinaryOp::Sub => Some(l - r),
                    BinaryOp::Mul => Some(l * r),
                    BinaryOp::Div => {
                        if r == 0.0 {
                            None
                        } else {
                            Some(l / r)
                        }
                    }
                }
            }
        }
    }

    /// Every band name the expression references, for validation.
    fn band_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Band(name) => out.push(name),
            Self::Const(_) => {}
            Self::Binary { lhs, rhs, .. } => {
                lhs.band_refs(out);
                rhs.band_refs(out);
            }
        }
    }
}

/// A named colormap applied to gray planes.
///
/// Only the identity map exists today; real palettes (viridis etc.) are a
/// deferred variant addition — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Colormap {
    /// The identity map: gray in, gray out.
    Grayscale,
}

/// One step of the pixel pipeline. See the module docs for the
/// producing-vs-transforming distinction and evaluation order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PixelOp {
    /// Evaluate an arithmetic expression per pixel, producing gray planes
    /// (all three channels carry the result).
    BandMath(Expr),
    /// Linearly map `min..=max` to `0..=255`, clamping outside values to
    /// the ends. Applied to all three planes.
    Rescale {
        /// Value mapped to 0 (values below clamp to 0).
        min: f64,
        /// Value mapped to 255 (values above clamp to 255).
        max: f64,
    },
    /// Select three named input bands as the R, G, B planes.
    Composite {
        /// Band for the red channel.
        r: String,
        /// Band for the green channel.
        g: String,
        /// Band for the blue channel.
        b: String,
    },
    /// Apply a named colormap to the current planes.
    Colormap(Colormap),
}

/// The encodings a plan can request for its output tile.
///
/// PNG is the Phase-1 format. WebP is deferred: the workspace `image` dep
/// deliberately enables only the `png` codec (every extra codec is
/// supply-chain surface the license gate must carry), and lossless WebP
/// encoding would pull in the `image-webp` crate via a new feature — a
/// variant addition here when a consumer actually needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TileFormat {
    /// Lossless RGBA PNG (see [`crate::encode_png`]).
    Png,
}

/// What the pipeline emits after the pixel ops run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputSpec {
    /// The encoded tile format.
    pub format: TileFormat,
}

impl OutputSpec {
    /// An output spec for `format`.
    #[must_use]
    pub fn new(format: TileFormat) -> Self {
        Self { format }
    }
}

/// A complete render plan: named inputs, the op pipeline, the output spec.
///
/// This is the executable IR the process compiler ([`crate::process`]) lowers openEO
/// graphs into and [`eval`] runs. It is serde round-trippable; the JSON
/// shape is pinned by snapshot tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RenderPlan {
    /// The named input bands, positionally matched to [`eval`]'s buffers.
    pub inputs: Vec<BandInput>,
    /// The pixel ops, applied in order.
    pub ops: Vec<PixelOp>,
    /// The requested output encoding.
    pub output: OutputSpec,
}

impl RenderPlan {
    /// A plan over `inputs` running `ops` and emitting `output`.
    #[must_use]
    pub fn new(inputs: Vec<BandInput>, ops: Vec<PixelOp>, output: OutputSpec) -> Self {
        Self {
            inputs,
            ops,
            output,
        }
    }
}

/// An evaluated tile: 8-bit RGBA, row-major, straight (non-premultiplied)
/// alpha. Invalid pixels are `(0, 0, 0, 0)` (module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaTile {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, RGBA interleaved, row-major.
    pub pixels: Vec<u8>,
}

/// Why a [`RenderPlan`] could not be evaluated. These are *plan* errors —
/// malformed pipelines or mismatched inputs — never per-pixel data
/// conditions, which land in the validity mask instead.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    /// The number of buffers does not match the plan's declared inputs.
    #[error("plan declares {expected} input bands, got {actual} buffers")]
    InputCount {
        /// Bands the plan declares.
        expected: usize,
        /// Buffers actually passed.
        actual: usize,
    },
    /// Two input buffers disagree on dimensions (or a buffer's vectors
    /// disagree with its own stated dimensions).
    #[error("input band `{name}` is {got_w}x{got_h}, expected {want_w}x{want_h}")]
    ShapeMismatch {
        /// The offending band's name.
        name: String,
        /// Its width.
        got_w: u32,
        /// Its height.
        got_h: u32,
        /// Expected width (from the first input).
        want_w: u32,
        /// Expected height.
        want_h: u32,
    },
    /// An expression or composite references a band the plan never declared.
    #[error("op references undeclared band `{name}`")]
    UnknownBand {
        /// The undeclared name.
        name: String,
    },
    /// A transforming op ([`PixelOp::Rescale`] / [`PixelOp::Colormap`]) ran
    /// before any producing op, or the plan has no producing op at all.
    #[error("no pixels produced yet: `{op}` needs a BandMath or Composite before it")]
    NothingToTransform {
        /// The op that had nothing to act on.
        op: &'static str,
    },
    /// A rescale with an empty or inverted range (`min >= max`).
    #[error("rescale range is degenerate: min {min} >= max {max}")]
    DegenerateRescale {
        /// The rescale minimum.
        min: f64,
        /// The rescale maximum.
        max: f64,
    },
}

/// The mutable pipeline state [`eval`] threads through the ops.
struct Planes {
    r: Vec<f64>,
    g: Vec<f64>,
    b: Vec<f64>,
}

/// Evaluates a band-math expression over every still-valid pixel, marking
/// pixels invalid on division by zero or a non-finite result. Band names
/// must already be validated against the plan.
fn band_math(
    expr: &Expr,
    inputs: &[WarpedBuffer],
    index_of: &impl Fn(&str) -> Result<usize, PlanError>,
    valid: &mut [bool],
) -> Vec<f64> {
    let mut gray = vec![0.0; valid.len()];
    for (i, (out, ok)) in gray.iter_mut().zip(valid.iter_mut()).enumerate() {
        if !*ok {
            continue;
        }
        let resolve = |name: &str| {
            // Names were validated by the caller; a miss is unreachable.
            let band = index_of(name).unwrap_or_default();
            inputs[band].values[i]
        };
        match expr.eval_at(&resolve) {
            Some(v) if v.is_finite() => *out = v,
            // Division by zero or a non-finite result: no data.
            _ => *ok = false,
        }
    }
    gray
}

impl Planes {
    /// Quantizes the planes to RGBA bytes: numpy `astype(uint8)` semantics
    /// (clamp into `0..=255`, truncate toward zero), invalid pixels as
    /// transparent black.
    fn quantize(&self, width: u32, height: u32, valid: &[bool]) -> RgbaTile {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped into 0..=255 before the cast"
        )]
        let q = |v: f64| v.clamp(0.0, 255.0) as u8;
        let mut pixels = Vec::with_capacity(valid.len() * 4);
        for (i, ok) in valid.iter().enumerate() {
            if *ok {
                pixels.extend_from_slice(&[q(self.r[i]), q(self.g[i]), q(self.b[i]), 255]);
            } else {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        RgbaTile {
            width,
            height,
            pixels,
        }
    }
}

/// Executes `plan`'s pixel ops over `inputs` (positionally matched to
/// `plan.inputs`), returning the quantized RGBA tile. See the module docs
/// for the pipeline model, validity semantics, and quantization.
///
/// # Errors
///
/// Any [`PlanError`]: mismatched input count or shapes, references to
/// undeclared bands, a transform op before any producing op (or no
/// producing op at all), or a degenerate rescale range.
pub fn eval(plan: &RenderPlan, inputs: &[WarpedBuffer]) -> Result<RgbaTile, PlanError> {
    if plan.inputs.len() != inputs.len() {
        return Err(PlanError::InputCount {
            expected: plan.inputs.len(),
            actual: inputs.len(),
        });
    }
    let (width, height) = inputs
        .first()
        .map_or((0, 0), |first| (first.width, first.height));
    let len = width as usize * height as usize;
    for (band, buf) in plan.inputs.iter().zip(inputs) {
        let own_len = buf.values.len() == len && buf.valid.len() == len;
        if buf.width != width || buf.height != height || !own_len {
            return Err(PlanError::ShapeMismatch {
                name: band.name.clone(),
                got_w: buf.width,
                got_h: buf.height,
                want_w: width,
                want_h: height,
            });
        }
    }
    let index_of = |name: &str| -> Result<usize, PlanError> {
        plan.inputs
            .iter()
            .position(|band| band.name == name)
            .ok_or_else(|| PlanError::UnknownBand {
                name: name.to_owned(),
            })
    };

    // All inputs are required: a pixel starts invalid iff any input band is
    // invalid there. Band math may invalidate further; nothing ever
    // re-validates.
    let mut valid: Vec<bool> = (0..len)
        .map(|i| inputs.iter().all(|b| b.valid[i]))
        .collect();
    let mut planes: Option<Planes> = None;

    for op in &plan.ops {
        match op {
            PixelOp::BandMath(expr) => {
                let mut refs = Vec::new();
                expr.band_refs(&mut refs);
                for name in refs {
                    index_of(name)?;
                }
                let gray = band_math(expr, inputs, &index_of, &mut valid);
                planes = Some(Planes {
                    r: gray.clone(),
                    g: gray.clone(),
                    b: gray,
                });
            }
            PixelOp::Composite { r, g, b } => {
                let (ri, gi, bi) = (index_of(r)?, index_of(g)?, index_of(b)?);
                planes = Some(Planes {
                    r: inputs[ri].values.clone(),
                    g: inputs[gi].values.clone(),
                    b: inputs[bi].values.clone(),
                });
            }
            PixelOp::Rescale { min, max } => {
                // `partial_cmp`: NaN bounds must also be rejected.
                if min.partial_cmp(max) != Some(std::cmp::Ordering::Less) {
                    return Err(PlanError::DegenerateRescale {
                        min: *min,
                        max: *max,
                    });
                }
                let planes = planes
                    .as_mut()
                    .ok_or(PlanError::NothingToTransform { op: "Rescale" })?;
                let span = max - min;
                for plane in [&mut planes.r, &mut planes.g, &mut planes.b] {
                    for v in plane.iter_mut() {
                        *v = (v.clamp(*min, *max) - min) / span * 255.0;
                    }
                }
            }
            PixelOp::Colormap(map) => {
                if planes.is_none() {
                    return Err(PlanError::NothingToTransform { op: "Colormap" });
                }
                match map {
                    Colormap::Grayscale => {} // identity
                }
            }
        }
    }

    let planes = planes.ok_or(PlanError::NothingToTransform { op: "end of plan" })?;
    Ok(planes.quantize(width, height, &valid))
}

#[cfg(test)]
mod tests {
    use super::{
        BandInput, Colormap, Expr, OutputSpec, PixelOp, PlanError, RenderPlan, TileFormat, eval,
    };
    use crate::warp::WarpedBuffer;

    fn buffer(width: u32, height: u32, values: Vec<f64>) -> WarpedBuffer {
        let valid = vec![true; values.len()];
        WarpedBuffer {
            width,
            height,
            values,
            valid,
        }
    }

    fn png_output() -> OutputSpec {
        OutputSpec::new(TileFormat::Png)
    }

    /// The NDVI plan the compiler will emit: `(nir - red) / (nir + red)`,
    /// rescaled -1..1, grayscale.
    fn ndvi_plan() -> RenderPlan {
        RenderPlan::new(
            vec![BandInput::new("nir"), BandInput::new("red")],
            vec![
                PixelOp::BandMath(
                    (Expr::band("nir") - Expr::band("red"))
                        / (Expr::band("nir") + Expr::band("red")),
                ),
                PixelOp::Rescale {
                    min: -1.0,
                    max: 1.0,
                },
                PixelOp::Colormap(Colormap::Grayscale),
            ],
            png_output(),
        )
    }

    #[test]
    fn ndvi_evaluates_and_rescales() {
        // nir = 3000, red = 1000 -> NDVI 0.5 -> (0.5 + 1) / 2 * 255 = 191.25
        // -> truncates to 191.
        let nir = buffer(2, 1, vec![3000.0, 1000.0]);
        let red = buffer(2, 1, vec![1000.0, 3000.0]);
        let tile = eval(&ndvi_plan(), &[nir, red]).unwrap();
        // Second pixel: NDVI -0.5 -> 63.75 -> 63.
        assert_eq!(tile.pixels, [191, 191, 191, 255, 63, 63, 63, 255]);
    }

    #[test]
    fn divide_by_zero_is_invalid_not_nan() {
        // nir = red = 0: denominator is exactly zero -> transparent black.
        let nir = buffer(1, 1, vec![0.0]);
        let red = buffer(1, 1, vec![0.0]);
        let tile = eval(&ndvi_plan(), &[nir, red]).unwrap();
        assert_eq!(tile.pixels, [0, 0, 0, 0]);
    }

    #[test]
    fn non_finite_results_are_invalid() {
        let plan = RenderPlan::new(
            vec![BandInput::new("b")],
            vec![PixelOp::BandMath(Expr::band("b") * Expr::Const(f64::MAX))],
            png_output(),
        );
        let tile = eval(&plan, &[buffer(1, 1, vec![f64::MAX])]).unwrap();
        assert_eq!(tile.pixels, [0, 0, 0, 0]);
    }

    #[test]
    fn any_invalid_input_makes_the_pixel_transparent() {
        // Even a band the ops never reference: all inputs are required.
        let used = buffer(1, 1, vec![100.0]);
        let mut unused = buffer(1, 1, vec![100.0]);
        unused.valid[0] = false;
        let plan = RenderPlan::new(
            vec![BandInput::new("used"), BandInput::new("unused")],
            vec![PixelOp::BandMath(Expr::band("used"))],
            png_output(),
        );
        let tile = eval(&plan, &[used, unused]).unwrap();
        assert_eq!(tile.pixels, [0, 0, 0, 0]);
    }

    #[test]
    fn composite_selects_planes_and_rescale_applies_to_all() {
        let r = buffer(1, 1, vec![3000.0]);
        let g = buffer(1, 1, vec![1500.0]);
        let b = buffer(1, 1, vec![-50.0]); // clamps to the floor
        let plan = RenderPlan::new(
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
            png_output(),
        );
        let tile = eval(&plan, &[r, g, b]).unwrap();
        assert_eq!(tile.pixels, [255, 127, 0, 255]);
    }

    #[test]
    fn plan_errors_are_reported() {
        let plan = ndvi_plan();
        assert_eq!(
            eval(&plan, &[buffer(1, 1, vec![1.0])]),
            Err(PlanError::InputCount {
                expected: 2,
                actual: 1
            })
        );
        assert_eq!(
            eval(
                &plan,
                &[buffer(1, 1, vec![1.0]), buffer(2, 1, vec![1.0, 2.0])]
            ),
            Err(PlanError::ShapeMismatch {
                name: "red".into(),
                got_w: 2,
                got_h: 1,
                want_w: 1,
                want_h: 1
            })
        );

        let unknown = RenderPlan::new(
            vec![BandInput::new("a")],
            vec![PixelOp::BandMath(Expr::band("nope"))],
            png_output(),
        );
        assert_eq!(
            eval(&unknown, &[buffer(1, 1, vec![1.0])]),
            Err(PlanError::UnknownBand {
                name: "nope".into()
            })
        );

        let bare_rescale = RenderPlan::new(
            vec![BandInput::new("a")],
            vec![PixelOp::Rescale { min: 0.0, max: 1.0 }],
            png_output(),
        );
        assert_eq!(
            eval(&bare_rescale, &[buffer(1, 1, vec![1.0])]),
            Err(PlanError::NothingToTransform { op: "Rescale" })
        );

        let empty = RenderPlan::new(vec![BandInput::new("a")], vec![], png_output());
        assert_eq!(
            eval(&empty, &[buffer(1, 1, vec![1.0])]),
            Err(PlanError::NothingToTransform { op: "end of plan" })
        );

        let degenerate = RenderPlan::new(
            vec![BandInput::new("a")],
            vec![
                PixelOp::BandMath(Expr::band("a")),
                PixelOp::Rescale { min: 1.0, max: 1.0 },
            ],
            png_output(),
        );
        assert_eq!(
            eval(&degenerate, &[buffer(1, 1, vec![1.0])]),
            Err(PlanError::DegenerateRescale { min: 1.0, max: 1.0 })
        );
    }

    #[test]
    fn zero_sized_inputs_evaluate_to_an_empty_tile() {
        let plan = RenderPlan::new(
            vec![BandInput::new("a")],
            vec![PixelOp::BandMath(Expr::band("a"))],
            png_output(),
        );
        let tile = eval(&plan, &[buffer(0, 0, vec![])]).unwrap();
        assert_eq!((tile.width, tile.height), (0, 0));
        assert!(tile.pixels.is_empty());
    }
}
