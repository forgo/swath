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
//! validity mask. [`PixelOp::BandMath`], [`PixelOp::Composite`], and
//! [`PixelOp::Udf`] *produce* planes (band math yields gray — all three
//! channels equal; composite selects three input bands; a UDF maps the
//! input planes through a sandboxed module, ADR 0018); [`PixelOp::Rescale`]
//! and [`PixelOp::Colormap`] *transform* the current planes and are errors
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
//! [`PixelOp::Colormap`] sits after rescale and before encode.
//! [`Colormap::Grayscale`] is the identity map; the palette variants
//! ([`Colormap::Viridis`], [`Colormap::Magma`], [`Colormap::RdYlGn`]) look
//! each gray pixel up in a vendored 256-entry RGB byte LUT
//! ([`crate::colormaps`], matplotlib's published tables). The index is the
//! **quantized** gray value — `clamp(0.0, 255.0)`, truncate toward zero —
//! exactly the arithmetic the final quantization applies, with linear
//! interpolation deliberately off: a colormapped pixel is `lut[q(gray)]`
//! for the same `q(gray)` the grayscale render would have emitted, so
//! palette output stays mechanically derivable from the oracle-validated
//! gray values. Alpha is untouched — validity alone decides transparency.
//! Palette maps require gray planes (band math), never a composite.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::udf::{UdfError, UdfExecutor, UdfLimits, UdfStage};
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

/// Renders the expression as infix text, operands parenthesized when they
/// are themselves binary: NDVI displays as `(b8a - b04) / (b8a + b04)` —
/// exactly the `BandMath` expression convention the catalog persists,
/// which is why this impl exists (the openEO services surface records a
/// compiled graph's band math in that form, ADR 0010).
impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn operand(e: &Expr, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match e {
                Expr::Binary { .. } => write!(f, "({e})"),
                Expr::Band(_) | Expr::Const(_) => write!(f, "{e}"),
            }
        }
        match self {
            Self::Band(name) => f.write_str(name),
            Self::Const(c) => write!(f, "{c}"),
            Self::Binary { op, lhs, rhs } => {
                operand(lhs, f)?;
                let symbol = match op {
                    BinaryOp::Add => " + ",
                    BinaryOp::Sub => " - ",
                    BinaryOp::Mul => " * ",
                    BinaryOp::Div => " / ",
                };
                f.write_str(symbol)?;
                operand(rhs, f)
            }
        }
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
/// The palette variants are matplotlib's published 256-entry byte LUTs,
/// indexed by quantized gray value with interpolation off — see the
/// module docs and [`crate::colormaps`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Colormap {
    /// The identity map: gray in, gray out.
    Grayscale,
    /// Matplotlib's perceptually uniform sequential `viridis`.
    Viridis,
    /// Matplotlib's perceptually uniform sequential `magma`.
    Magma,
    /// The `ColorBrewer` diverging red–yellow–green map (`RdYlGn`) — the
    /// standard vegetation-index palette; NDVI's default.
    RdYlGn,
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
    /// Run a sandboxed WASM `run_udf` module over **all** input planes
    /// (in [`RenderPlan::inputs`] order), producing the module's declared
    /// output planes — 1 rendered as gray, 3 as RGB (ADR 0018, #201).
    ///
    /// The stage names the module by content hash; the bytes never enter
    /// the IR. Execution goes through the [`UdfExecutor`] port
    /// ([`crate::udf`]) — this crate stays wasmtime-free.
    ///
    /// The IR deliberately permits UDF ops anywhere in the sequence, any
    /// number of times, so lifting the v1 one-UDF-per-plan restriction
    /// needs no serde change; the restriction itself is enforced at
    /// plan-construction time by [`PlanSpec::Udf`](crate::plan::PlanSpec)
    /// carrying exactly one stage.
    Udf(UdfStage),
}

/// The encodings a plan can request for its output tile.
///
/// PNG is the Phase-1 format. WebP is deferred: the workspace `image` dep
/// deliberately enables only the `png` codec (every extra codec is
/// supply-chain surface the license gate must carry), and lossless WebP
/// encoding would pull in the `image-webp` crate via a new feature — a
/// variant addition here when a consumer actually needs it. Deferral
/// tracked in `docs/ROADMAP.md` (deferral inventory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TileFormat {
    /// Lossless RGBA PNG (see [`crate::encode_png`]).
    Png,
}

impl TileFormat {
    /// The IANA media type of tiles encoded in this format — what HTTP
    /// responses and cache entries (#36) record.
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
        }
    }
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
    /// A palette colormap applied to non-gray planes (a composite): a LUT
    /// maps one gray value per pixel, so only band-math output qualifies.
    #[error("colormap `{map:?}` needs gray planes: apply it after band math, not a composite")]
    ColormapNeedsGray {
        /// The palette that had no gray planes to map.
        map: Colormap,
    },
    /// A UDF stage declares an output arity v1 cannot render: 1 plane
    /// renders as gray, 3 as RGB, anything else has no image meaning
    /// (ADR 0018; other arities are a v2 reopen condition). Checked
    /// before the executor is consulted — a structural plan error.
    #[error("UDF declares {declared} output planes; v1 renders 1 (gray) or 3 (RGB)")]
    UdfOutputPlanes {
        /// The stage's declared output arity.
        declared: u32,
    },
    /// The [`UdfExecutor`] port refused or failed the stage — including
    /// [`UdfError::NotConfigured`] from the default [`crate::udf::NoUdf`]
    /// executor when a plan names a module no deployment wiring can run.
    #[error("UDF stage failed")]
    Udf(#[from] UdfError),
    /// The executor violated its contract: it returned a plane set that
    /// does not match the stage's declared arity or the tile's shape.
    /// Never the module's fault alone — the adapter (#203) must have
    /// failed to enforce the ABI (`docs/udf-abi/v1.md`) before returning.
    #[error(
        "UDF executor returned {actual_planes} planes of {got_w}x{got_h}, \
         expected {expected_planes} of {want_w}x{want_h}"
    )]
    UdfOutputShape {
        /// Planes the stage declared.
        expected_planes: u32,
        /// Planes the executor returned.
        actual_planes: usize,
        /// Returned width (of the first offending plane; the expected
        /// width when only the count is wrong).
        got_w: u32,
        /// Returned height (as above).
        got_h: u32,
        /// The tile width every plane must have.
        want_w: u32,
        /// The tile height every plane must have.
        want_h: u32,
    },
}

/// The mutable pipeline state [`eval`] threads through the ops.
struct Planes {
    r: Vec<f64>,
    g: Vec<f64>,
    b: Vec<f64>,
    /// Whether the three planes are one gray value per pixel (band-math
    /// output) — the shape a palette colormap can index by.
    gray: bool,
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

/// Folds a UDF stage's executor-returned planes into the pipeline state,
/// enforcing the ABI's host post-conditions (`docs/udf-abi/v1.md`): every
/// plane tile-shaped, output validity `ANDed` with the current mask, and
/// non-finite values the executor claims valid canonicalized to invalid.
/// The caller has already checked `stage.output_planes` is 1 or 3.
fn udf_planes(
    stage: &UdfStage,
    outputs: Vec<WarpedBuffer>,
    width: u32,
    height: u32,
    valid: &mut [bool],
) -> Result<Planes, PlanError> {
    let shape_error = |got_w, got_h, actual_planes| PlanError::UdfOutputShape {
        expected_planes: stage.output_planes,
        actual_planes,
        got_w,
        got_h,
        want_w: width,
        want_h: height,
    };
    if outputs.len() != stage.output_planes as usize {
        return Err(shape_error(width, height, outputs.len()));
    }
    for plane in &outputs {
        let own_len = plane.values.len() == valid.len() && plane.valid.len() == valid.len();
        if plane.width != width || plane.height != height || !own_len {
            return Err(shape_error(plane.width, plane.height, outputs.len()));
        }
    }
    // A UDF can invalidate pixels, never resurrect them: AND, and treat a
    // non-finite "valid" sample as invalid (ADR 0018).
    for (i, ok) in valid.iter_mut().enumerate() {
        *ok = *ok
            && outputs
                .iter()
                .all(|plane| plane.valid[i] && plane.values[i].is_finite());
    }
    let mut planes = outputs.into_iter().map(|plane| plane.values);
    let r = planes.next().unwrap_or_default();
    Ok(match planes.next() {
        // Three planes: RGB.
        Some(g) => Planes {
            r,
            g,
            b: planes.next().unwrap_or_default(),
            gray: false,
        },
        // One plane: gray, colormappable like band-math output.
        None => Planes {
            g: r.clone(),
            b: r.clone(),
            r,
            gray: true,
        },
    })
}

/// What a [`PixelOp::Udf`] stage cost (ADR 0018, #205): the executor's
/// fuel meter and the stage's wall clock. The tiler records the former
/// as `Trace::udf_fuel_used` and the latter as `Timings::udf_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UdfCost {
    /// Fuel the executor charged (`None` from an unmetered executor).
    pub fuel_used: Option<u64>,
    /// Wall clock of the stage: instantiate, copy in, run, copy out.
    pub elapsed: Duration,
}

/// The result of [`eval_with`]: the tile plus, when the plan ran a UDF
/// stage, its cost.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Evaluation {
    /// The quantized RGBA tile.
    pub tile: RgbaTile,
    /// The UDF stage's cost; `None` when the plan has no UDF stage.
    pub udf: Option<UdfCost>,
}

/// Executes `plan`'s pixel ops over `inputs` (positionally matched to
/// `plan.inputs`), returning the quantized RGBA tile. See the module docs
/// for the pipeline model, validity semantics, and quantization.
///
/// `udf` is the [`UdfExecutor`] port a [`PixelOp::Udf`] stage runs
/// through (ADR 0018); it is consulted only when a UDF op is reached, so
/// plans without UDF stages evaluate identically under any executor —
/// pass [`crate::udf::NoUdf`] where none is wired. A UDF stage runs under
/// the default [`UdfLimits`]; [`eval_with`] takes explicit limits and
/// reports the stage's cost — the tiler's form.
///
/// # Errors
///
/// Any [`PlanError`]: mismatched input count or shapes, references to
/// undeclared bands, a transform op before any producing op (or no
/// producing op at all), a degenerate rescale range, a palette
/// colormap over non-gray planes, or a failed/unwired UDF stage.
pub fn eval(
    plan: &RenderPlan,
    inputs: &[WarpedBuffer],
    udf: &dyn UdfExecutor,
) -> Result<RgbaTile, PlanError> {
    eval_with(plan, inputs, udf, &UdfLimits::default()).map(|evaluated| evaluated.tile)
}

/// [`eval`] under explicit [`UdfLimits`], answering the UDF stage's
/// [`UdfCost`] beside the tile — the form the tiler uses to bound a
/// layer's module by its `Budget::max_udf_fuel_per_tile` and to put the
/// cost on the Trace.
///
/// # Errors
///
/// Exactly [`eval`]'s.
#[allow(
    clippy::too_many_lines,
    reason = "one match arm per PixelOp; splitting the loop would hide the pipeline"
)]
pub fn eval_with(
    plan: &RenderPlan,
    inputs: &[WarpedBuffer],
    udf: &dyn UdfExecutor,
    limits: &UdfLimits,
) -> Result<Evaluation, PlanError> {
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
    let mut udf_cost: Option<UdfCost> = None;

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
                    gray: true,
                });
            }
            PixelOp::Composite { r, g, b } => {
                let (ri, gi, bi) = (index_of(r)?, index_of(g)?, index_of(b)?);
                planes = Some(Planes {
                    r: inputs[ri].values.clone(),
                    g: inputs[gi].values.clone(),
                    b: inputs[bi].values.clone(),
                    gray: false,
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
            PixelOp::Udf(stage) => {
                // Arity first — a structural plan error, independent of
                // (and checked before) any executor.
                if !matches!(stage.output_planes, 1 | 3) {
                    return Err(PlanError::UdfOutputPlanes {
                        declared: stage.output_planes,
                    });
                }
                // The stage's cost is measured around the executor
                // alone (the ABI's whole round trip, not the host-side
                // post-conditions below) — `Timings::udf_ms` is the UDF's
                // share of `pixel_ops_ms`, and the fuel is the
                // deterministic number the budget bounds.
                let started = Instant::now();
                let output = udf.run(stage, inputs, limits)?;
                udf_cost = Some(UdfCost {
                    fuel_used: output.fuel_used,
                    elapsed: started.elapsed(),
                });
                planes = Some(udf_planes(stage, output.planes, width, height, &mut valid)?);
            }
            PixelOp::Colormap(map) => {
                let planes = planes
                    .as_mut()
                    .ok_or(PlanError::NothingToTransform { op: "Colormap" })?;
                // Grayscale is the identity; palettes look each quantized
                // gray value up in their vendored 256-entry LUT — no
                // interpolation (module docs on colormaps).
                if let Some(lut) = crate::colormaps::lut(*map) {
                    if !planes.gray {
                        return Err(PlanError::ColormapNeedsGray { map: *map });
                    }
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "clamped into 0..=255 before the cast"
                    )]
                    let q = |v: f64| v.clamp(0.0, 255.0) as u8;
                    for i in 0..planes.r.len() {
                        let [r, g, b] = lut[usize::from(q(planes.r[i]))];
                        planes.r[i] = f64::from(r);
                        planes.g[i] = f64::from(g);
                        planes.b[i] = f64::from(b);
                    }
                    planes.gray = false;
                }
            }
        }
    }

    let planes = planes.ok_or(PlanError::NothingToTransform { op: "end of plan" })?;
    Ok(Evaluation {
        tile: planes.quantize(width, height, &valid),
        udf: udf_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        BandInput, Colormap, Expr, OutputSpec, PixelOp, PlanError, RenderPlan, TileFormat, eval,
        eval_with,
    };
    use crate::udf::{NoUdf, UdfError, UdfExecutor, UdfLimits, UdfOutput, UdfStage};
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
        let tile = eval(&ndvi_plan(), &[nir, red], &NoUdf).unwrap();
        // Second pixel: NDVI -0.5 -> 63.75 -> 63.
        assert_eq!(tile.pixels, [191, 191, 191, 255, 63, 63, 63, 255]);
    }

    #[test]
    fn divide_by_zero_is_invalid_not_nan() {
        // nir = red = 0: denominator is exactly zero -> transparent black.
        let nir = buffer(1, 1, vec![0.0]);
        let red = buffer(1, 1, vec![0.0]);
        let tile = eval(&ndvi_plan(), &[nir, red], &NoUdf).unwrap();
        assert_eq!(tile.pixels, [0, 0, 0, 0]);
    }

    #[test]
    fn non_finite_results_are_invalid() {
        let plan = RenderPlan::new(
            vec![BandInput::new("b")],
            vec![PixelOp::BandMath(Expr::band("b") * Expr::Const(f64::MAX))],
            png_output(),
        );
        let tile = eval(&plan, &[buffer(1, 1, vec![f64::MAX])], &NoUdf).unwrap();
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
        let tile = eval(&plan, &[used, unused], &NoUdf).unwrap();
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
        let tile = eval(&plan, &[r, g, b], &NoUdf).unwrap();
        assert_eq!(tile.pixels, [255, 127, 0, 255]);
    }

    #[test]
    fn plan_errors_are_reported() {
        let plan = ndvi_plan();
        assert_eq!(
            eval(&plan, &[buffer(1, 1, vec![1.0])], &NoUdf),
            Err(PlanError::InputCount {
                expected: 2,
                actual: 1
            })
        );
        assert_eq!(
            eval(
                &plan,
                &[buffer(1, 1, vec![1.0]), buffer(2, 1, vec![1.0, 2.0])],
                &NoUdf
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
            eval(&unknown, &[buffer(1, 1, vec![1.0])], &NoUdf),
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
            eval(&bare_rescale, &[buffer(1, 1, vec![1.0])], &NoUdf),
            Err(PlanError::NothingToTransform { op: "Rescale" })
        );

        let empty = RenderPlan::new(vec![BandInput::new("a")], vec![], png_output());
        assert_eq!(
            eval(&empty, &[buffer(1, 1, vec![1.0])], &NoUdf),
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
            eval(&degenerate, &[buffer(1, 1, vec![1.0])], &NoUdf),
            Err(PlanError::DegenerateRescale { min: 1.0, max: 1.0 })
        );
    }

    /// An executor handing back canned output planes — the port's test
    /// double (the real adapter is #203's wasmtime crate). Charges the
    /// whole fuel limit it was given, so the limit's plumbing is visible.
    struct FakeUdf(Vec<WarpedBuffer>);

    impl UdfExecutor for FakeUdf {
        fn run(
            &self,
            _stage: &UdfStage,
            _inputs: &[WarpedBuffer],
            limits: &UdfLimits,
        ) -> Result<UdfOutput, UdfError> {
            Ok(UdfOutput::new(self.0.clone()).with_fuel_used(limits.max_fuel))
        }
    }

    /// An executor that must never be consulted.
    struct PanicUdf;

    impl UdfExecutor for PanicUdf {
        fn run(
            &self,
            _stage: &UdfStage,
            _inputs: &[WarpedBuffer],
            _limits: &UdfLimits,
        ) -> Result<UdfOutput, UdfError> {
            panic!("the executor was consulted by a path that must not reach it")
        }
    }

    /// `eval_with` hands the caller's limits to the executor and reports
    /// the stage's cost beside the tile (#205); a plan without a UDF
    /// stage reports no cost at all.
    #[test]
    fn eval_with_threads_limits_and_reports_the_udf_cost() {
        let inputs = [buffer(1, 1, vec![1.0]), buffer(1, 1, vec![2.0])];
        let executor = FakeUdf(vec![buffer(1, 1, vec![7.0])]);
        let evaluated =
            eval_with(&udf_plan(1), &inputs, &executor, &UdfLimits::new(4_321)).unwrap();
        let cost = evaluated.udf.expect("a UDF stage ran");
        assert_eq!(cost.fuel_used, Some(4_321));
        assert_eq!(evaluated.tile.pixels, [7, 7, 7, 255]);
        // The default limits are the budget default (100 M).
        assert_eq!(
            UdfLimits::default().max_fuel,
            swath_core::planner::DEFAULT_MAX_UDF_FUEL_PER_TILE
        );

        let nir = buffer(1, 1, vec![3000.0]);
        let red = buffer(1, 1, vec![1000.0]);
        let evaluated = eval_with(&ndvi_plan(), &[nir, red], &PanicUdf, &UdfLimits::default())
            .expect("plan evaluates without the executor");
        assert_eq!(evaluated.udf, None);
    }

    /// A sha256-hex-shaped module identity for tests.
    const HASH: &str = "cafe0000000000000000000000000000000000000000000000000000000000ff";

    /// A plan running one UDF stage over two input bands.
    fn udf_plan(output_planes: u32) -> RenderPlan {
        RenderPlan::new(
            vec![BandInput::new("nir"), BandInput::new("red")],
            vec![PixelOp::Udf(UdfStage::new(
                HASH,
                output_planes,
                serde_json::Value::Null,
            ))],
            png_output(),
        )
    }

    /// The `NoUdf` refusal, pinned: a UDF plan under the default executor
    /// is exactly `PlanError::Udf(UdfError::NotConfigured)` naming the
    /// module (issue #201's acceptance test).
    #[test]
    fn no_udf_executor_refuses_udf_plans() {
        let inputs = [buffer(1, 1, vec![1.0]), buffer(1, 1, vec![2.0])];
        assert_eq!(
            eval(&udf_plan(1), &inputs, &NoUdf),
            Err(PlanError::Udf(UdfError::NotConfigured {
                code_hash: HASH.to_owned()
            }))
        );
    }

    /// Plans without UDF stages never touch the executor — the property
    /// that makes `NoUdf` a safe default everywhere.
    #[test]
    fn plans_without_udf_stages_never_touch_the_executor() {
        let nir = buffer(1, 1, vec![3000.0]);
        let red = buffer(1, 1, vec![1000.0]);
        eval(&ndvi_plan(), &[nir, red], &PanicUdf).expect("plan evaluates without the executor");
    }

    #[test]
    fn one_plane_udf_produces_gray() {
        let out = buffer(2, 1, vec![0.0, 300.0]);
        let executor = FakeUdf(vec![out]);
        let inputs = [buffer(2, 1, vec![1.0, 1.0]), buffer(2, 1, vec![2.0, 2.0])];
        let tile = eval(&udf_plan(1), &inputs, &executor).unwrap();
        // Gray planes quantize like band math: 300 clamps to 255.
        assert_eq!(tile.pixels, [0, 0, 0, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn three_plane_udf_produces_rgb_and_transforms_apply_after() {
        let (r, g, b) = (
            buffer(1, 1, vec![4.0]),
            buffer(1, 1, vec![2.0]),
            buffer(1, 1, vec![0.0]),
        );
        let executor = FakeUdf(vec![r, g, b]);
        let mut plan = udf_plan(3);
        // A transform after the producing UDF op: the ordinary order rule.
        plan.ops.push(PixelOp::Rescale { min: 0.0, max: 4.0 });
        let inputs = [buffer(1, 1, vec![1.0]), buffer(1, 1, vec![2.0])];
        let tile = eval(&plan, &inputs, &executor).unwrap();
        assert_eq!(tile.pixels, [255, 127, 0, 255]);
    }

    /// The ABI's host post-conditions (docs/udf-abi/v1.md), enforced by
    /// `eval` itself: a UDF can invalidate pixels but never resurrect
    /// them, and a non-finite value it claims valid becomes invalid.
    #[test]
    fn udf_validity_is_anded_and_non_finite_is_invalid() {
        // Pixel 0: invalid input the executor claims valid — stays invalid.
        // Pixel 1: valid input, executor returns NaN as "valid" — invalid.
        // Pixel 2: valid throughout.
        let mut nir = buffer(3, 1, vec![1.0, 1.0, 1.0]);
        nir.valid[0] = false;
        let red = buffer(3, 1, vec![2.0, 2.0, 2.0]);
        let out = buffer(3, 1, vec![9.0, f64::NAN, 9.0]);
        let tile = eval(&udf_plan(1), &[nir, red], &FakeUdf(vec![out])).unwrap();
        assert_eq!(
            tile.pixels,
            [0, 0, 0, 0, 0, 0, 0, 0, 9, 9, 9, 255],
            "resurrected and non-finite pixels are transparent"
        );
    }

    /// An unsupported output arity is a structural plan error, decided
    /// before any executor runs (`PanicUdf` proves the order).
    #[test]
    fn udf_output_arity_is_checked_before_the_executor() {
        let inputs = [buffer(1, 1, vec![1.0]), buffer(1, 1, vec![2.0])];
        assert_eq!(
            eval(&udf_plan(2), &inputs, &PanicUdf),
            Err(PlanError::UdfOutputPlanes { declared: 2 })
        );
    }

    /// An executor violating its contract — wrong plane count or shape —
    /// errors loudly instead of rendering garbage.
    #[test]
    fn udf_executor_shape_violations_are_loud() {
        let inputs = [buffer(1, 1, vec![1.0]), buffer(1, 1, vec![2.0])];
        assert_eq!(
            eval(&udf_plan(1), &inputs, &FakeUdf(Vec::new())),
            Err(PlanError::UdfOutputShape {
                expected_planes: 1,
                actual_planes: 0,
                got_w: 1,
                got_h: 1,
                want_w: 1,
                want_h: 1
            })
        );
        assert_eq!(
            eval(
                &udf_plan(1),
                &inputs,
                &FakeUdf(vec![buffer(2, 1, vec![0.0, 0.0])])
            ),
            Err(PlanError::UdfOutputShape {
                expected_planes: 1,
                actual_planes: 1,
                got_w: 2,
                got_h: 1,
                want_w: 1,
                want_h: 1
            })
        );
    }

    /// A UDF is a *producing* op: a transform before it still errors
    /// (`PanicUdf` proves the order rule fires first).
    #[test]
    fn transform_before_udf_still_has_nothing_to_transform() {
        let mut plan = udf_plan(1);
        plan.ops.insert(0, PixelOp::Rescale { min: 0.0, max: 1.0 });
        let inputs = [buffer(1, 1, vec![1.0]), buffer(1, 1, vec![2.0])];
        assert_eq!(
            eval(&plan, &inputs, &PanicUdf),
            Err(PlanError::NothingToTransform { op: "Rescale" })
        );
    }

    #[test]
    fn zero_sized_inputs_evaluate_to_an_empty_tile() {
        let plan = RenderPlan::new(
            vec![BandInput::new("a")],
            vec![PixelOp::BandMath(Expr::band("a"))],
            png_output(),
        );
        let tile = eval(&plan, &[buffer(0, 0, vec![])], &NoUdf).unwrap();
        assert_eq!((tile.width, tile.height), (0, 0));
        assert!(tile.pixels.is_empty());
    }
}
