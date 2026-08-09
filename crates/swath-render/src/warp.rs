// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The inverse-mapping warp kernel: nearest and bilinear resampling.

use swath_core::raster::RasterInfo;
use swath_core::reproject::CoordTransform;
use swath_core::source::{PixelBuffer, WindowData};

use crate::error::RenderError;
use crate::grid::TargetGrid;
use crate::window::{SourceExtent, source_extent};

/// Below this total support weight a bilinear result is invalid rather than
/// renormalized — GDAL's own cutoff (`GWKResample` rejects an accumulated
/// weight `< 0.000001`), kept identical so edge pixels flip valid/invalid at
/// the same points the oracle's do.
const MIN_BILINEAR_WEIGHT: f64 = 0.000_001;

/// GDAL's triangle filter for bilinear resampling (`GWKBilinear`).
fn triangle(t: f64) -> f64 {
    let a = t.abs();
    if a <= 1.0 { 1.0 - a } else { 0.0 }
}

/// The per-warp resampling geometry GDAL derives before running its kernel —
/// replicated here because matching the oracle requires matching it exactly.
///
/// GDAL's warper is **not** plain 2×2 bilinear when the warp decimates. Per
/// warp it computes X/Y scales from the destination size and the source-pixel
/// span of the transformed destination region (`GDALWarpOperation::
/// ComputeSourceWindow` + the scale setup in `GDALWarpKernel::PerformWarp`,
/// GDAL 3.12). When both scales are ≥ 0.95 it uses the familiar 4-sample
/// bilinear; when an axis decimates, the kernel becomes a **scaled triangle
/// filter** (anti-aliasing): support radius `ceil(1/scale)` and weights
/// `max(0, 1 - |d·scale|)` per axis, normalized over the contributing
/// samples. The scales can differ per axis (e.g. a tile that overhangs the
/// raster edge decimates in Y but not X), and GDAL snaps a scale to `1/n`
/// when its reciprocal is within 0.05 of an integer.
///
/// It also bounds output validity: a destination pixel whose source
/// coordinate falls outside GDAL's computed source window (which is clipped
/// to the raster) is invalid outright, with no edge renormalization.
#[derive(Debug, Clone, Copy, PartialEq)]
struct KernelShape {
    /// GDAL source-window left edge, in full-resolution pixels.
    off_x: f64,
    /// GDAL source-window top edge.
    off_y: f64,
    /// GDAL source-window width in pixels.
    size_x: f64,
    /// GDAL source-window height in pixels.
    size_y: f64,
    /// Kernel X scale (1.0 when not decimating).
    scale_x: f64,
    /// Kernel Y scale.
    scale_y: f64,
    /// Kernel X support radius in source pixels.
    radius_x: i64,
    /// Kernel Y support radius.
    radius_y: i64,
}

impl KernelShape {
    /// Replicates GDAL's source-window and scale computation for one warp
    /// (destination `dst_w × dst_h`, transformed-boundary extent `ext`,
    /// raster `raster_w × raster_h`, `filter` = kernel radius at scale 1:
    /// 1 for bilinear, 0 for nearest). `None` when the destination maps
    /// entirely off the raster.
    #[allow(
        clippy::cast_precision_loss,
        reason = "raster dims and radii far below 2^52"
    )]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "radii are small ceil() results; window values pre-clamped"
    )]
    #[allow(
        clippy::float_cmp,
        reason = "GDAL compares its integral-valued window size to the \
                  destination size exactly; both are whole numbers here"
    )]
    fn compute(
        dst_w: u32,
        dst_h: u32,
        ext: &SourceExtent,
        raster_w: u64,
        raster_h: u64,
        filter: u32,
    ) -> Option<Self> {
        let rw = raster_w as f64;
        let rh = raster_h as f64;
        // GDAL "roundIfCloseEnough": snap near-integer bounds.
        let snap = |v: f64| {
            let r = v.round();
            if (r - v).abs() < 1e-6 { r } else { v }
        };
        let west = snap(ext.min_col);
        let east = snap(ext.max_col);
        let north = snap(ext.min_row);
        let south = snap(ext.max_row);
        if west > rw || east < 0.0 || north > rh || south < 0.0 {
            return None;
        }
        // Integer-clamped bounds (GDAL truncates after clamping to >= 0).
        let west_i = west.max(0.0).trunc();
        let north_i = north.max(0.0).trunc();
        let east_i = east.ceil().min(rw).trunc();
        let south_i = south.ceil().min(rh).trunc();
        let filter = f64::from(filter);
        // Window padding radius (uses the *unclamped* span).
        let pad = |dst: f64, span: f64| {
            let scale = (dst / span).max(1e-3);
            if scale < 0.95 {
                (filter / scale).ceil()
            } else {
                filter
            }
        };
        let pad_x = pad(f64::from(dst_w), east - west);
        let pad_y = pad(f64::from(dst_h), south - north);
        // The padded window, clipped to the raster (with GDAL's >90%-width
        // shortcut that snaps to the full axis).
        let window = |min_c: f64, max_c: f64, pad: f64, raster: f64| {
            if max_c - min_c > 0.9 * raster {
                (0.0, raster)
            } else {
                let off = (min_c - pad).clamp(0.0, raster);
                let size = (raster - off).min(max_c - off + pad).max(0.0);
                (off, size)
            }
        };
        let (off_x, size_x) = window(west_i, east_i, pad_x, rw);
        let (off_y, size_y) = window(north_i, south_i, pad_y, rh);
        // Kernel scales: destination size over the padded source-window
        // size, with GDAL's reciprocal snapping. (Calibrated against the
        // oracle: the WarpedVRT read path drives `GDALWarpKernel` with
        // `dfSrcExtraSize = 0`, so `dfScale = nDstSize / nSrcSize` — the
        // z11 golden pins this observably at scales 256/320 and 256/381.)
        let kernel_scale = |dst: f64, size: f64| {
            if size <= 0.0 || size == dst {
                return 1.0;
            }
            let mut s = dst / size;
            if s < 1.0 {
                let recip = 1.0 / s;
                let n = (recip + 0.5).trunc();
                if (recip - n).abs() < 0.05 {
                    s = 1.0 / n;
                }
            }
            s
        };
        let mut scale_x = kernel_scale(f64::from(dst_w), size_x);
        let mut scale_y = kernel_scale(f64::from(dst_h), size_y);
        // Both scales >= 0.95: GDAL's 4-sample bilinear formula, i.e. plain
        // unscaled weights with radius `filter`.
        if scale_x >= 0.95 && scale_y >= 0.95 {
            scale_x = 1.0;
            scale_y = 1.0;
        }
        let radius = |s: f64| {
            if s < 1.0 {
                (filter / s).ceil() as i64
            } else {
                filter as i64
            }
        };
        Some(Self {
            off_x,
            off_y,
            size_x,
            size_y,
            scale_x,
            scale_y,
            radius_x: radius(scale_x),
            radius_y: radius(scale_y),
        })
    }

    /// GDAL's destination-pixel rejection (`GWKCheckAndComputeSrcOffsets`):
    /// source coordinates outside the computed source window are invalid,
    /// with a `1e-10` slack on the max side.
    fn contains(&self, fcol: f64, frow: f64) -> bool {
        fcol >= self.off_x
            && frow >= self.off_y
            && fcol + 1e-10 <= self.off_x + self.size_x
            && frow + 1e-10 <= self.off_y + self.size_y
    }
}

/// How bilinear sampling treats nodata among its four support pixels.
///
/// # What GDAL (and therefore rio-tiler) does
///
/// GDAL's warper excludes invalid support pixels and **renormalizes** the
/// remaining weights (`GWKResample`: sum `w·v` over valid samples, divide
/// by the sum of their weights; only when the valid weight sum falls below
/// `1e-6` is the output pixel invalid). Nodata never
/// *contaminates* a neighbourhood — a target pixel next to a nodata edge
/// still gets a value, interpolated from the valid subset. Support pixels
/// outside the source raster are treated the same way: dropped and
/// renormalized. The default [`NodataPolicy::ExcludeRenormalize`] matches
/// this exactly; the golden tests against oracle tiles with a real
/// swath-edge would fail on the alpha channel under any other default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodataPolicy {
    /// Drop invalid support pixels and renormalize the remaining weights
    /// (GDAL's behavior; the default).
    #[default]
    ExcludeRenormalize,
    /// Any missing support pixel with nonzero weight (nodata or outside the
    /// source) invalidates the output pixel. Stricter than GDAL: use when a
    /// value interpolated from partial support must never be presented as
    /// fully supported.
    Propagate,
}

/// Which resampling kernel a warp uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Resampling {
    /// Nearest neighbour — value-preserving; the only correct choice for
    /// categorical bands (e.g. HLS Fmask), where interpolation would invent
    /// class codes.
    Nearest,
    /// Bilinear interpolation over the 2×2 support — for continuous bands
    /// (reflectance, radiance), with the given nodata policy.
    Bilinear(NodataPolicy),
}

/// The result of a warp: one value and one validity flag per target pixel,
/// row-major over the target grid.
///
/// # Buffer design (feeds the pixel-ops and encode stages)
///
/// Values are **`f64`**, not the source dtype: every supported source dtype
/// (`u8`…`i32`, `f32`) is exactly representable in `f64`, bilinear results
/// are intrinsically fractional, and the next pipeline stage (band math,
/// rescale, colormap — issues #25/#26) computes in floating point anyway.
/// Quantizing back to the source dtype here would bake rounding error into
/// the pipeline before band math runs. The validity mask is a separate
/// `Vec<bool>` rather than a nodata sentinel so downstream stages never
/// have to guess whether a value is data — invalid pixels hold `0.0` and
/// must be interpreted through [`valid`](Self::valid) (the encode stage
/// turns it into the alpha channel).
#[derive(Debug, Clone, PartialEq)]
pub struct WarpedBuffer {
    /// Grid width in pixels.
    pub width: u32,
    /// Grid height in pixels.
    pub height: u32,
    /// Sample values, row-major; `0.0` where invalid.
    pub values: Vec<f64>,
    /// Per-pixel validity: `true` where [`values`](Self::values) holds data.
    pub valid: Vec<bool>,
}

impl WarpedBuffer {
    fn empty_invalid(width: u32, height: u32) -> Self {
        let len = width as usize * height as usize;
        Self {
            width,
            height,
            values: vec![0.0; len],
            valid: vec![false; len],
        }
    }

    /// Number of valid pixels.
    #[must_use]
    pub fn valid_count(&self) -> usize {
        self.valid.iter().filter(|v| **v).count()
    }
}

/// Widens a buffer to `f64` samples (exact for every supported variant),
/// or `None` for a variant these kernels do not know yet.
fn widen(pixels: &PixelBuffer) -> Option<Vec<f64>> {
    match pixels {
        PixelBuffer::UInt8(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Int16(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::UInt16(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Int32(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Float32(v) => Some(v.iter().copied().map(f64::from).collect()),
        PixelBuffer::Float64(v) => Some(v.clone()),
        _ => None,
    }
}

/// Whether `v` is the nodata sentinel (NaN-aware).
#[allow(
    clippy::float_cmp,
    reason = "nodata is an exact sentinel (GDAL semantics), never a range"
)]
fn is_nodata(v: f64, nodata: Option<f64>) -> bool {
    nodata.is_some_and(|nd| v == nd || (nd.is_nan() && v.is_nan()))
}

/// Warps `source` into `target` by inverse mapping: each target pixel
/// center is mapped through `transform` (**target CRS → source CRS**) and
/// `source.grid.transform` (the geotransform of the grid the window was
/// read from — `source.window` places the buffer within that grid) and
/// sampled with `resampling`. `source.grid` also supplies the raster
/// dimensions, from which the GDAL-equivalent resampling geometry
/// ([`KernelShape`]) is derived.
///
/// The grid comes from the [`WindowData`] itself (never passed separately),
/// so overview reads warp correctly by construction: an overview window
/// carries the overview grid, and every coordinate here — window offsets,
/// kernel window, raster bounds — is in that grid's pixel space (#38).
/// GDAL behaves identically when warping from an overview: the same warp
/// machinery runs against the overview dataset's grid.
///
/// Target pixels that map outside the source window, onto nodata (per the
/// kernel's policy), or outside the transform's domain come back invalid —
/// never fabricated. Transforms are batched one target row at a time
/// through [`CoordTransform::transform_slice`] so adapters' bulk paths are
/// used; a row whose batch fails falls back to per-point transforms with
/// failing pixels marked invalid.
///
/// The source window must cover the resampling support: bilinear needs the
/// tile's source extent plus 1 pixel at scale ≥ 1, growing to
/// `ceil(1/scale) + 1` when the warp decimates (pass that as the `margin`
/// of [`crate::source_window`]).
///
/// # Errors
///
/// * [`RenderError::SourceShape`] — buffer length disagrees with its window.
/// * [`RenderError::NonInvertibleTransform`] — singular geotransform.
/// * [`RenderError::UnsupportedDtype`] — pixel-buffer variant unknown to
///   these kernels.
#[allow(
    clippy::too_many_lines,
    reason = "one pass over the target grid; splitting the row loop apart \
              would only scatter tightly coupled state"
)]
pub fn warp(
    source: &WindowData,
    transform: &dyn CoordTransform,
    target: &TargetGrid,
    resampling: Resampling,
) -> Result<WarpedBuffer, RenderError> {
    let info: &RasterInfo = &source.grid;
    let win = source.window;
    let expected = win.width * win.height;
    let actual = source.pixels.len() as u64;
    if expected != actual {
        return Err(RenderError::SourceShape { expected, actual });
    }
    let src_transform = &info.transform;
    let det = src_transform.determinant();
    if det == 0.0 {
        return Err(RenderError::NonInvertibleTransform { determinant: det });
    }
    let samples = widen(&source.pixels).ok_or(RenderError::UnsupportedDtype {
        dtype: source.pixels.dtype(),
    })?;
    let nodata = source.nodata;

    let mut out = WarpedBuffer::empty_invalid(target.width(), target.height());
    if expected == 0 || out.values.is_empty() {
        return Ok(out);
    }

    let filter = match resampling {
        Resampling::Nearest => 0,
        Resampling::Bilinear(_) => 1,
    };
    let Some(extent) = source_extent(target, info, transform)? else {
        return Ok(out); // whole tile outside the transform's domain
    };
    let Some(shape) = KernelShape::compute(
        target.width(),
        target.height(),
        &extent,
        info.width,
        info.height,
        filter,
    ) else {
        return Ok(out); // whole tile off the raster
    };

    #[allow(
        clippy::cast_precision_loss,
        reason = "window dims far below 2^52; used as coordinate bounds"
    )]
    let (src_w, src_h) = (win.width as f64, win.height as f64);
    let width = target.width() as usize;

    // Scratch buffers reused across rows.
    let mut centers: Vec<(f64, f64)> = vec![(0.0, 0.0); width];
    let mut points: Vec<(f64, f64)> = vec![(0.0, 0.0); width];
    let mut point_ok: Vec<bool> = vec![true; width];

    for row in 0..target.height() {
        for (col, c) in centers.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation, reason = "col < width: u32")]
            let col = col as u32;
            *c = target.pixel_center(col, row);
        }
        points.copy_from_slice(&centers);
        if transform.transform_slice(&mut points).is_ok() {
            point_ok.fill(true);
        } else {
            // Batch failed (contents now unspecified): redo per point,
            // marking out-of-domain pixels invalid instead of failing the row.
            for ((p, c), ok) in points.iter_mut().zip(&centers).zip(point_ok.iter_mut()) {
                match transform.transform(c.0, c.1) {
                    Ok(t) => {
                        *p = t;
                        *ok = true;
                    }
                    Err(_) => *ok = false,
                }
            }
        }

        let row_base = row as usize * width;
        for (col, (&(sx, sy), &ok)) in points.iter().zip(point_ok.iter()).enumerate() {
            if !ok {
                continue;
            }
            // Continuous source pixel coordinates in the full-res grid.
            let Ok((fcol, frow)) = src_transform.crs_to_pixel(sx, sy) else {
                continue; // determinant checked above; unreachable
            };
            if !shape.contains(fcol, frow) {
                continue; // outside GDAL's source window: invalid
            }
            #[allow(clippy::cast_precision_loss, reason = "window offsets far below 2^52")]
            let (lc, lr) = (fcol - win.col_off as f64, frow - win.row_off as f64);
            // GDAL's containing-pixel gate (`GWKRealCaseThread`): if the
            // source pixel *containing* the mapped point is nodata, the
            // destination pixel is invalid — even for bilinear, whose wider
            // support might have offered valid samples ("this currently
            // ignores the multi-pixel input of bilinear" in GDAL's own
            // words). Matching it is required to match the oracle's alpha.
            if nodata.is_some() {
                let (gc, gr) = ((lc + 1e-10).floor(), (lr + 1e-10).floor());
                if gc >= 0.0 && gr >= 0.0 && gc < src_w && gr < src_h {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "bounds-checked against the window dimensions above"
                    )]
                    let idx = gr as usize * src_w as usize + gc as usize;
                    if is_nodata(samples[idx], nodata) {
                        continue;
                    }
                }
            }
            let (value, valid) = match resampling {
                Resampling::Nearest => sample_nearest(&samples, nodata, lc, lr, src_w, src_h),
                Resampling::Bilinear(policy) => {
                    sample_bilinear(&samples, nodata, lc, lr, src_w, src_h, &shape, policy)
                }
            };
            if valid {
                out.values[row_base + col] = value;
                out.valid[row_base + col] = true;
            }
        }
    }
    Ok(out)
}

/// Nearest-neighbour sample at continuous window-local `(lc, lr)`.
///
/// The pixel index is `floor(coord + 1e-10)` — GDAL's rounding
/// (`GWKCheckAndComputeSrcOffsets`), where the epsilon keeps centers that
/// land exactly on a pixel boundary from flipping on floating-point noise.
fn sample_nearest(
    samples: &[f64],
    nodata: Option<f64>,
    lc: f64,
    lr: f64,
    src_w: f64,
    src_h: f64,
) -> (f64, bool) {
    let ic = (lc + 1e-10).floor();
    let ir = (lr + 1e-10).floor();
    if ic < 0.0 || ir < 0.0 || ic >= src_w || ir >= src_h {
        return (0.0, false);
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bounds-checked against the window dimensions above"
    )]
    let idx = ir as usize * src_w as usize + ic as usize;
    let v = samples[idx];
    if is_nodata(v, nodata) {
        (0.0, false)
    } else {
        (v, true)
    }
}

/// Bilinear sample around window-local `(lc, lr)` (pixel centers at
/// integer + 0.5) — GDAL's `GWKResample`: a per-axis triangle filter,
/// scaled (anti-aliased) on any axis the warp decimates, accumulating
/// valid support and renormalizing by the accumulated weight.
///
/// At scale 1 the loop reduces exactly to the familiar 2×2 bilinear
/// (`GWKBilinearResample4Sample`): the radius is 1 and weights beyond the
/// 2×2 support evaluate to zero.
#[allow(clippy::too_many_arguments, reason = "internal kernel plumbing")]
fn sample_bilinear(
    samples: &[f64],
    nodata: Option<f64>,
    lc: f64,
    lr: f64,
    src_w: f64,
    src_h: f64,
    shape: &KernelShape,
    policy: NodataPolicy,
) -> (f64, bool) {
    // Coordinates relative to pixel centers (GDAL: dfSrc - 0.5).
    let u = lc - 0.5;
    let v = lr - 0.5;
    let i0 = u.floor();
    let j0 = v.floor();
    let dx = u - i0;
    let dy = v - j0;

    let mut acc = 0.0;
    let mut weight_sum = 0.0;
    let mut missing_support = false;

    for j in -shape.radius_y..=shape.radius_y {
        #[allow(clippy::cast_precision_loss, reason = "radii are small")]
        let jf = j as f64;
        let cj = j0 + jf;
        let wy = if shape.scale_y < 1.0 {
            triangle((jf - dy) * shape.scale_y)
        } else {
            triangle(jf - dy)
        };
        if wy <= 0.0 {
            continue;
        }
        for i in -shape.radius_x..=shape.radius_x {
            #[allow(clippy::cast_precision_loss, reason = "radii are small")]
            let ifl = i as f64;
            let ci = i0 + ifl;
            let wx = if shape.scale_x < 1.0 {
                triangle((ifl - dx) * shape.scale_x)
            } else {
                triangle(ifl - dx)
            };
            if wx <= 0.0 {
                continue;
            }
            let w = wx * wy;
            if ci < 0.0 || cj < 0.0 || ci >= src_w || cj >= src_h {
                missing_support = true;
                continue;
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "bounds-checked against the window dimensions above"
            )]
            let idx = cj as usize * src_w as usize + ci as usize;
            let s = samples[idx];
            if is_nodata(s, nodata) {
                missing_support = true;
                continue;
            }
            acc += w * s;
            weight_sum += w;
        }
    }

    if weight_sum < MIN_BILINEAR_WEIGHT {
        return (0.0, false);
    }
    if policy == NodataPolicy::Propagate && missing_support {
        return (0.0, false);
    }
    // GDAL skips the divide when the accumulated weight is within 1e-5 of
    // one, so full-support unit-scale results are a plain lerp, bit for bit.
    if (weight_sum - 1.0).abs() <= 0.000_01 {
        (acc, true)
    } else {
        (acc / weight_sum, true)
    }
}

#[cfg(test)]
mod tests {
    use swath_core::crs::Crs;
    use swath_core::raster::{DType, GeoTransform, RasterInfo, WindowRequest};
    use swath_core::reproject::{CoordTransform, ReprojectError};
    use swath_core::source::{PixelBuffer, WindowData};
    use swath_core::tile::MercatorBounds;

    use super::{NodataPolicy, Resampling, WarpedBuffer, warp};
    use crate::error::RenderError;
    use crate::grid::TargetGrid;

    struct Identity;

    impl CoordTransform for Identity {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            Ok((x, y))
        }
    }

    /// Rejects x < 0 to exercise the per-point fallback path.
    struct RejectWest;

    impl CoordTransform for RejectWest {
        fn transform(&self, x: f64, y: f64) -> Result<(f64, f64), ReprojectError> {
            if x < 0.0 {
                Err(ReprojectError::OutOfDomain { x, y })
            } else {
                Ok((x, y))
            }
        }
    }

    /// A window at full-res offset (0, 0): w×h pixels, 1 m grid, origin (0, 0)
    /// with y growing downward from 0 (`north_up` with `origin_y` = 0).
    fn window(w: u64, h: u64, pixels: Vec<i16>, nodata: Option<f64>) -> WindowData {
        WindowData::new(
            WindowRequest {
                col_off: 0,
                row_off: 0,
                width: w,
                height: h,
            },
            info(w, h),
            PixelBuffer::Int16(pixels),
            nodata,
            vec![],
        )
    }

    fn gt() -> GeoTransform {
        GeoTransform::north_up(0.0, 0.0, 1.0, -1.0)
    }

    /// Raster metadata matching [`window`]/[`gt`]: a `w × h` raster whose
    /// window is the whole raster.
    fn info(w: u64, h: u64) -> RasterInfo {
        RasterInfo {
            crs: Crs::WEB_MERCATOR,
            width: w,
            height: h,
            transform: gt(),
            band_count: 1,
            dtype: DType::Int16,
            nodata: None,
            overview_levels: vec![],
        }
    }

    /// A target grid aligned 1:1 with source pixels (identity warp).
    fn aligned_grid(w: u32, h: u32) -> TargetGrid {
        TargetGrid::new(
            MercatorBounds {
                min_x: 0.0,
                min_y: -f64::from(h),
                max_x: f64::from(w),
                max_y: 0.0,
            },
            w,
            h,
        )
    }

    #[test]
    fn identity_nearest_reproduces_the_source() {
        let src = window(4, 4, (0..16).collect(), None);
        let out = warp(&src, &Identity, &aligned_grid(4, 4), Resampling::Nearest).unwrap();
        assert_eq!(out.valid_count(), 16);
        let got: Vec<f64> = out.values.clone();
        assert_eq!(got, (0..16).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn identity_bilinear_at_pixel_centers_reproduces_the_source() {
        let src = window(4, 4, (0..16).collect(), None);
        let out = warp(
            &src,
            &Identity,
            &aligned_grid(4, 4),
            Resampling::Bilinear(NodataPolicy::default()),
        )
        .unwrap();
        assert_eq!(out.valid_count(), 16);
        assert_eq!(out.values, (0..16).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn bilinear_interpolates_between_centers() {
        // 2×1 source [10, 30]; a 2× upsampled grid puts target centers at
        // source-local x = 0.25, 0.75, 1.25, 1.75 → u = -0.25, 0.25, 0.75, 1.25.
        let src = window(2, 1, vec![10, 30], None);
        let target = TargetGrid::new(
            MercatorBounds {
                min_x: 0.0,
                min_y: -1.0,
                max_x: 2.0,
                max_y: 0.0,
            },
            4,
            1,
        );
        let out = warp(
            &src,
            &Identity,
            &target,
            Resampling::Bilinear(NodataPolicy::default()),
        )
        .unwrap();
        // Edge pixels renormalize to their single in-bounds neighbour.
        assert_eq!(out.values, vec![10.0, 15.0, 25.0, 30.0]);
        assert_eq!(out.valid_count(), 4);
    }

    #[test]
    fn nearest_rejects_nodata_and_out_of_window() {
        let src = window(2, 2, vec![7, -9999, 7, 7], Some(-9999.0));
        let out = warp(&src, &Identity, &aligned_grid(2, 2), Resampling::Nearest).unwrap();
        assert_eq!(out.valid, vec![true, false, true, true]);
        assert!(out.values[1].abs() < f64::EPSILON);
    }

    #[test]
    fn bilinear_renormalizes_around_nodata_but_propagate_invalidates() {
        // 2×2 with one nodata corner; sample at (0.9, 0.9) — the containing
        // pixel (0, 0) is valid (so GDAL's containing-pixel gate passes) but
        // the 2×2 support includes the nodata corner with weight 0.16.
        let src = window(2, 2, vec![100, 100, 100, -9999], Some(-9999.0));
        let center = TargetGrid::new(
            MercatorBounds {
                min_x: 0.4,
                min_y: -1.4,
                max_x: 1.4,
                max_y: -0.4,
            },
            1,
            1,
        );
        let out = warp(
            &src,
            &Identity,
            &center,
            Resampling::Bilinear(NodataPolicy::ExcludeRenormalize),
        )
        .unwrap();
        assert_eq!(out.valid, vec![true]);
        assert!((out.values[0] - 100.0).abs() < 1e-12);

        let out = warp(
            &src,
            &Identity,
            &center,
            Resampling::Bilinear(NodataPolicy::Propagate),
        )
        .unwrap();
        assert_eq!(out.valid, vec![false]);
    }

    #[test]
    fn containing_pixel_nodata_gates_bilinear_output() {
        // GDAL invalidates the destination pixel when the source pixel
        // *containing* the mapped point is nodata, even though the bilinear
        // support holds three valid samples. Sample dead-center of the
        // nodata pixel (1.5, 1.5): support weights would renormalize to the
        // three valid neighbours, but the gate wins.
        let src = window(2, 2, vec![100, 100, 100, -9999], Some(-9999.0));
        let over_nodata = TargetGrid::new(
            MercatorBounds {
                min_x: 1.0,
                min_y: -2.0,
                max_x: 2.0,
                max_y: -1.0,
            },
            1,
            1,
        );
        for policy in [NodataPolicy::ExcludeRenormalize, NodataPolicy::Propagate] {
            let out = warp(&src, &Identity, &over_nodata, Resampling::Bilinear(policy)).unwrap();
            assert_eq!(out.valid, vec![false], "policy {policy:?}");
        }
    }

    #[test]
    fn all_nodata_support_is_invalid_under_both_policies() {
        let src = window(2, 2, vec![-9999; 4], Some(-9999.0));
        for policy in [NodataPolicy::ExcludeRenormalize, NodataPolicy::Propagate] {
            let out = warp(
                &src,
                &Identity,
                &aligned_grid(2, 2),
                Resampling::Bilinear(policy),
            )
            .unwrap();
            assert_eq!(out.valid_count(), 0, "policy {policy:?}");
        }
    }

    #[test]
    fn out_of_domain_pixels_are_invalid_not_errors() {
        // Grid straddling x = 0; RejectWest fails the batch, and the
        // per-point fallback marks the western half invalid.
        let src = window(4, 1, vec![1, 2, 3, 4], None);
        let target = TargetGrid::new(
            MercatorBounds {
                min_x: -2.0,
                min_y: -1.0,
                max_x: 2.0,
                max_y: 0.0,
            },
            4,
            1,
        );
        let out = warp(&src, &RejectWest, &target, Resampling::Nearest).unwrap();
        assert_eq!(out.valid, vec![false, false, true, true]);
    }

    #[test]
    fn shape_mismatch_and_singular_transform_are_errors() {
        let src = window(4, 4, vec![0; 15], None);
        let err = warp(&src, &Identity, &aligned_grid(4, 4), Resampling::Nearest)
            .expect_err("shape mismatch");
        assert_eq!(
            err,
            RenderError::SourceShape {
                expected: 16,
                actual: 15
            }
        );

        let mut src = window(2, 2, vec![0; 4], None);
        src.grid.transform = GeoTransform::north_up(0.0, 0.0, 1.0, 0.0);
        assert!(matches!(
            warp(&src, &Identity, &aligned_grid(2, 2), Resampling::Nearest),
            Err(RenderError::NonInvertibleTransform { .. })
        ));
    }

    #[test]
    fn empty_source_window_yields_all_invalid() {
        let src = window(0, 0, vec![], Some(-9999.0));
        let out = warp(&src, &Identity, &aligned_grid(2, 2), Resampling::Nearest).unwrap();
        assert_eq!(out.valid_count(), 0);
        assert_eq!(out, WarpedBuffer::empty_invalid(2, 2));
    }
}
