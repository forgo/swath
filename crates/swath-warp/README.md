# swath-warp

GDAL-exact warp/resample kernel in pure Rust — the inverse-mapping warp
engine extracted from [Swath](https://github.com/forgo/swath), for anyone
who needs GDAL's exact resampling behavior without GDAL.

## The GDAL-equivalence contract

`swath-warp` replicates GDAL 3.12's `GDALWarpKernel` semantics for nearest
and bilinear resampling, including the parts a naive reimplementation
misses:

- **Scaled triangle filters under decimation.** GDAL's warper is not plain
  2×2 bilinear when the warp decimates: per warp it derives X/Y kernel
  scales from the destination size and the source-pixel span of the
  transformed destination region, and below scale 0.95 an axis switches to
  an anti-aliasing triangle filter with support radius `ceil(1/scale)`.
  The scales are per-axis (a target overhanging the raster edge can
  decimate in Y but not X).
- **Per-axis scale snapping.** GDAL snaps a kernel scale to `1/n` when its
  reciprocal is within 0.05 of an integer; the kernel reproduces that,
  along with the near-integer window snapping (`roundIfCloseEnough`), the
  \>90%-width full-axis shortcut, and the exact source-window padding.
- **GDAL's exact validity cutoffs.** A destination pixel is invalid when
  its source coordinate falls outside GDAL's computed source window (with
  the `1e-10` slack), when the accumulated bilinear support weight falls
  below `1e-6`, and when the source pixel *containing* the mapped point is
  nodata — GDAL's containing-pixel gate, which invalidates a bilinear
  output even when the wider support holds valid samples. Renormalization
  over valid support, the skip-the-divide `1e-5` unit-weight shortcut, and
  nearest's `floor(coord + 1e-10)` rounding are all matched.
- **Source-window computation.** `source_window` traces the densified
  target boundary (21 points per edge, the same density GDAL uses in
  `GDALSuggestedWarpOutput`) through the CRS transform, excludes
  out-of-domain points as GDAL does, and clips to the raster.

Nodata handling is GDAL's by default (`NodataPolicy::ExcludeRenormalize`:
drop invalid support, renormalize the rest); a stricter `Propagate` policy
is available when partially supported values must not be presented as
fully supported.

## The oracle method

The contract is proven, not claimed:

- **Upstream oracle goldens.** In the Swath workspace this kernel renders
  real HLS fixture bands into Web Mercator tiles that are perceptually
  diffed against tiles rendered by rio-tiler/GDAL from the same fixtures
  (tolerance 2/255, ≤0.5% bad pixels, alpha held to the same bar — the
  swath-edge nodata tiles are the real test).
- **Committed crate goldens.** `tests/golden.rs` replays captured warps —
  the recorded proj4rs transform outputs, the COG-read source windows, and
  the oracle-anchored expected outputs — and requires **bit-identical**
  results, with no projection library or raster reader in the loop. The
  cases pin the swath-edge nodata behavior under both kernels and the
  z11 decimation path (kernel scales 256/320 and 256/381).
- **Property tests.** Nodata never fabricates data, nearest never invents
  values, bilinear stays inside the valid source range, constants warp to
  constants — regardless of geometry.

## Self-contained by design

The crate has **zero dependencies** and takes trait-shaped, minimal
inputs: implement `CoordTransform` (target CRS → source CRS points) over
proj4rs, PROJ, or any projection library; describe the source with
`GeoTransform`/`SourceGrid`/`PixelWindow`; hand samples over as `f64`
(exact for every integer dtype up to 32 bits and for `f32`). Projection
math never lives here.

See the crate documentation for a complete, runnable example.

## Status

Published as a `0.1.0-alpha.N` — built from a tagged commit through the
full Swath CI gate, with no API stability promised between alphas.
Licensed Apache-2.0.
