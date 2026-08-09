# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "rio-tiler==9.4.2",
#     "rasterio==1.5.1",
#     "numpy==2.5.1",
# ]
# ///
"""Swath correctness oracle: GDAL/rio-tiler reference renderer (issue #19).

Per ADR 0002 GDAL lives ONLY in the test suite; this script is that boundary.
It runs via ``uv run tests/oracle/render_reference.py`` with exact pinned
wheels (rasterio wheels bundle their own GDAL/PROJ — no Docker, no system
GDAL), so the reference pipeline is fully reproducible from the pins above.

Determinism contract
--------------------
* ``synth-cog`` writes a synthetic Cloud-Optimized GeoTIFF whose pixel values
  are pure functions of pixel coordinates (gradients + checkerboard + optional
  nodata corner). No randomness, no timestamps, no environment-dependent
  values enter the raster.
* ``render`` produces an XYZ tile PNG whose bytes depend only on (COG bytes,
  z/x/y, band selection, rescale range, pinned library versions). The PNG is
  encoded by GDAL's PNG driver via rio-tiler, which embeds no timestamps or
  ancillary metadata. Every invocation renders the tile TWICE in-process and
  refuses to write unless both byte streams hash identically, so a
  nondeterministic pipeline fails loudly instead of poisoning comparisons.
* Byte-stability ACROSS runs (the issue #19 validation gate) is asserted by
  ``just oracle-verify``, which renders twice in separate processes and
  compares SHA-256 digests.

Subcommands
-----------
* ``synth-cog OUT.tif [--size 512] [--nodata-corner]``
* ``render COG z x y OUT.png [--bands 1,2,3] [--rescale MIN,MAX]
  [--resampling nearest|bilinear|cubic] [--no-overviews] [--exact-grid]``
* ``compose z x y OUT.png --input COG [--input COG ...] [--expression EXPR]
  [--rescale MIN,MAX] [--resampling ...] [--no-overviews] [--exact-grid]``

``compose`` (issue #25) renders the multi-file pipelines the Render IR
executes — RGB composites and band-math expressions across single-band
COGs (one band per file, HLS-style). Each input's band 1 is warped onto
the tile grid exactly as ``render`` warps it (same rio-tiler read path,
same flags); the stage under test is what happens *after* the warp.
Ground truth for ``--expression`` is computed in numpy over the warped
float arrays (names ``b1``..``bN``, operators ``+ - * /``) rather than
through rio-tiler's expression plumbing: rio-tiler expressions are
per-dataset (``Reader.tile(expression=...)``), and composing them across
files needs a MultiBandReader with a naming convention — dataset-layout
machinery that is beside the point here. The numpy path evaluates the
same post-warp arithmetic on the same GDAL-warped pixels, which is
precisely the semantics the IR's ``BandMath`` defines (warp first, math
second). Non-finite results (division by zero) and any input's nodata
mask the output pixel; masked pixels are written as transparent black,
matching the IR's documented encoding. ``--rescale`` clips then maps
linearly to 0..255 per channel (numpy ``astype(uint8)`` truncation),
identical to ``render``'s rescale arithmetic. Determinism is enforced
the same way as ``render``: two in-process runs must hash identically.
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.enums import Resampling
from rasterio.transform import from_bounds
from rasterio.vrt import WarpedVRT
from rio_tiler.io import Reader
from rio_tiler.models import ImageData

# WebMercator (EPSG:3857) half-world extent in metres.
WEB_MERCATOR_HALF_WORLD = 20037508.342789244

# The synthetic COG exactly covers this XYZ tile, so renders of it (and its
# children) are well-defined without any external fixture. Arbitrary but fixed.
SYNTH_TILE_Z, SYNTH_TILE_X, SYNTH_TILE_Y = 6, 10, 24

NODATA_VALUE = 0


def tile_bounds_3857(z: int, x: int, y: int) -> tuple[float, float, float, float]:
    """Return (west, south, east, north) of XYZ tile z/x/y in EPSG:3857."""
    span = 2.0 * WEB_MERCATOR_HALF_WORLD / (2.0**z)
    west = -WEB_MERCATOR_HALF_WORLD + x * span
    north = WEB_MERCATOR_HALF_WORLD - y * span
    return (west, north - span, west + span, north)


def synth_bands(size: int, nodata_corner: bool) -> np.ndarray:
    """Deterministic 3-band uint8 test pattern, values in 1..255.

    Band 1: horizontal gradient; band 2: vertical gradient; band 3: 32-px
    checkerboard. Values start at 1 so 0 stays free for nodata. With
    ``nodata_corner``, the top-left size/8 square of every band is nodata.
    """
    ramp = np.rint(np.arange(size) * 254.0 / (size - 1)).astype(np.uint8) + 1
    band1 = np.broadcast_to(ramp, (size, size)).copy()
    band2 = np.broadcast_to(ramp[:, np.newaxis], (size, size)).copy()
    yy, xx = np.mgrid[0:size, 0:size]
    band3 = np.where(((xx // 32) + (yy // 32)) % 2 == 0, 60, 200).astype(np.uint8)
    data = np.stack([band1, band2, band3])
    if nodata_corner:
        corner = size // 8
        data[:, :corner, :corner] = NODATA_VALUE
    return data


def cmd_synth_cog(args: argparse.Namespace) -> int:
    """Write the deterministic synthetic COG."""
    size: int = args.size
    data = synth_bands(size, args.nodata_corner)
    bounds = tile_bounds_3857(SYNTH_TILE_Z, SYNTH_TILE_X, SYNTH_TILE_Y)
    profile = {
        "driver": "COG",
        "dtype": "uint8",
        "count": 3,
        "width": size,
        "height": size,
        "crs": "EPSG:3857",
        "transform": from_bounds(*bounds, size, size),
        "nodata": NODATA_VALUE,
        "compress": "deflate",
        "blocksize": 256,
        "overview_resampling": "average",
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with rasterio.open(out, "w", **profile) as dst:
        dst.write(data)
    print(f"synth-cog: wrote {out} ({size}x{size}, 3 bands, nodata corner: {args.nodata_corner})")
    return 0


def render_tile_png(
    cog: str,
    z: int,
    x: int,
    y: int,
    bands: tuple[int, ...],
    rescale: tuple[float, float] | None,
    resampling: str = "nearest",
    no_overviews: bool = False,
    exact_grid: bool = False,
) -> bytes:
    """Render one XYZ tile of ``cog`` to PNG bytes via rio-tiler.

    ``resampling`` selects the GDAL warp kernel (rio-tiler's
    ``reproject_method``; default ``nearest``, rio-tiler's own default, so
    pre-existing renders are byte-identical). Added for issue #24: the Swath
    warp kernels are validated per-kernel against the oracle, so the oracle
    must be able to warp with the same kernel under test (bilinear for
    continuous bands, nearest for categorical).

    ``no_overviews`` hides the COG's embedded overviews (GTiff open option
    ``OVERVIEW_LEVEL=NONE``), forcing GDAL to warp from the full-resolution
    grid. Also issue #24: at decimating zooms GDAL otherwise resamples from
    an overview level, so a kernel-vs-kernel comparison would silently
    compare against *average-decimated* pixels. Swath's overview selection
    is a separate, planner-level decision (ARCHITECTURE.md §5) validated on
    its own; kernel goldens must be renders of the same source pixels the
    kernel under test consumed.

    ``exact_grid`` warps in a single stage directly onto the 256-px tile grid
    (a ``WarpedVRT`` whose transform IS the tile grid), instead of rio-tiler's
    read pipeline (warp to a VRT near dataset resolution, then a decimated
    ``nearest`` read). The two are byte-identical when the tile does not
    decimate the source (verified for the committed fixtures at z12/z13),
    but at decimating zooms the two-stage pipeline point-samples the warped
    grid — an artifact of the *read* path, not of GDAL's warp kernel. Kernel
    goldens for decimating zooms use this flag so the reference is GDAL's
    warp semantics (anti-aliased scaled kernel), the semantics Swath's
    kernels implement.

    Exact-grid mode also uses an (effectively) exact coordinate transformer
    (``tolerance=1e-6`` source pixels instead of GDAL's default 0.125-px
    approximation): the approximation is a throughput optimization that
    perturbs sampling coordinates, not part of the kernel semantics under
    test, and Swath transforms every pixel exactly.
    """
    open_options = {"OVERVIEW_LEVEL": "NONE"} if no_overviews else {}
    if exact_grid:
        bounds = tile_bounds_3857(z, x, y)
        with rasterio.open(cog, **open_options) as dataset:
            with WarpedVRT(
                dataset,
                crs="EPSG:3857",
                transform=from_bounds(*bounds, 256, 256),
                width=256,
                height=256,
                resampling=Resampling[resampling],
                tolerance=1e-6,
            ) as vrt:
                data = vrt.read(indexes=bands)
                mask = vrt.read_masks(bands[0])
        img = ImageData(np.ma.MaskedArray(data, mask=np.broadcast_to(mask == 0, data.shape)))
    elif no_overviews:
        with rasterio.open(cog, **open_options) as dataset:
            with Reader(None, dataset=dataset) as reader:
                img = reader.tile(x, y, z, indexes=bands, reproject_method=resampling)
    else:
        with Reader(cog) as reader:
            img = reader.tile(x, y, z, indexes=bands, reproject_method=resampling)
    if rescale is not None:
        img.rescale(in_range=(rescale,) * len(bands))
    return img.render(img_format="PNG")


def warp_band_f64(
    cog: str,
    z: int,
    x: int,
    y: int,
    resampling: str,
    no_overviews: bool,
    exact_grid: bool,
) -> tuple[np.ndarray, np.ndarray]:
    """Warp band 1 of ``cog`` onto the 256-px tile grid of z/x/y.

    Returns ``(values, valid)``: float64 pixel values and a boolean validity
    mask. The read paths mirror ``render_tile_png`` exactly (same WarpedVRT
    settings for ``--exact-grid``, same ``Reader.tile`` call otherwise) so a
    composed golden warps identically to a single-band golden.
    """
    open_options = {"OVERVIEW_LEVEL": "NONE"} if no_overviews else {}
    if exact_grid:
        bounds = tile_bounds_3857(z, x, y)
        with rasterio.open(cog, **open_options) as dataset:
            with WarpedVRT(
                dataset,
                crs="EPSG:3857",
                transform=from_bounds(*bounds, 256, 256),
                width=256,
                height=256,
                resampling=Resampling[resampling],
                tolerance=1e-6,
            ) as vrt:
                data = vrt.read(indexes=(1,))
                mask = vrt.read_masks(1)
        return data[0].astype(np.float64), mask != 0
    if no_overviews:
        with rasterio.open(cog, **open_options) as dataset:
            with Reader(None, dataset=dataset) as reader:
                img = reader.tile(x, y, z, indexes=(1,), reproject_method=resampling)
    else:
        with Reader(cog) as reader:
            img = reader.tile(x, y, z, indexes=(1,), reproject_method=resampling)
    # ImageData.array is the source of truth for validity (ImageData.mask is
    # dtype-cast and unreliable for non-uint8 data in rio-tiler 9).
    return img.data[0].astype(np.float64), ~np.ma.getmaskarray(img.array)[0]


def compose_tile_png(
    inputs: list[str],
    z: int,
    x: int,
    y: int,
    expression: str | None,
    rescale: tuple[float, float] | None,
    resampling: str,
    no_overviews: bool,
    exact_grid: bool,
) -> bytes:
    """Render a composed (multi-file) tile to PNG bytes; see module docs."""
    warped = [
        warp_band_f64(cog, z, x, y, resampling, no_overviews, exact_grid) for cog in inputs
    ]
    valid = np.logical_and.reduce([v for _, v in warped])
    if expression is not None:
        names = {f"b{i + 1}": values for i, (values, _) in enumerate(warped)}
        with np.errstate(all="ignore"):
            # Trusted test-tooling input: names b1..bN, arithmetic only.
            result = eval(expression, {"__builtins__": {}}, names)  # noqa: S307
        result = np.asarray(result, dtype=np.float64)
        valid &= np.isfinite(result)
        planes = np.stack([result] * 3)
    else:
        if len(warped) != 3:
            msg = f"compose without --expression needs exactly 3 inputs, got {len(warped)}"
            raise ValueError(msg)
        planes = np.stack([values for values, _ in warped])
    if rescale is not None:
        lo, hi = rescale
        planes = (np.clip(planes, lo, hi) - lo) / (hi - lo) * 255.0
    quantized = np.clip(planes, 0.0, 255.0).astype(np.uint8)
    quantized[:, ~valid] = 0  # masked pixels are transparent black
    img = ImageData(np.ma.MaskedArray(quantized, mask=np.broadcast_to(~valid, quantized.shape)))
    return img.render(img_format="PNG")


def cmd_compose(args: argparse.Namespace) -> int:
    """Render a composed tile, verifying in-process determinism first."""
    rescale: tuple[float, float] | None = None
    if args.rescale is not None:
        lo, hi = (float(v) for v in args.rescale.split(","))
        rescale = (lo, hi)
    def render_once() -> bytes:
        return compose_tile_png(
            args.input,
            args.z,
            args.x,
            args.y,
            args.expression,
            rescale,
            args.resampling,
            args.no_overviews,
            args.exact_grid,
        )

    first = render_once()
    second = render_once()
    digest = hashlib.sha256(first).hexdigest()
    if hashlib.sha256(second).hexdigest() != digest:
        print("compose: NONDETERMINISTIC — two in-process renders differ", file=sys.stderr)
        return 1
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(first)
    print(
        f"compose: wrote {out} (z={args.z} x={args.x} y={args.y} "
        f"inputs={len(args.input)} expression={args.expression!r} sha256={digest})"
    )
    return 0


def cmd_render(args: argparse.Namespace) -> int:
    """Render an XYZ tile, verifying in-process determinism before writing."""
    bands = tuple(int(b) for b in args.bands.split(","))
    rescale: tuple[float, float] | None = None
    if args.rescale is not None:
        lo, hi = (float(v) for v in args.rescale.split(","))
        rescale = (lo, hi)

    first = render_tile_png(
        args.cog,
        args.z,
        args.x,
        args.y,
        bands,
        rescale,
        args.resampling,
        args.no_overviews,
        args.exact_grid,
    )
    second = render_tile_png(
        args.cog,
        args.z,
        args.x,
        args.y,
        bands,
        rescale,
        args.resampling,
        args.no_overviews,
        args.exact_grid,
    )
    digest = hashlib.sha256(first).hexdigest()
    if hashlib.sha256(second).hexdigest() != digest:
        print("render: NONDETERMINISTIC — two in-process renders differ", file=sys.stderr)
        return 1

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(first)
    print(f"render: wrote {out} (z={args.z} x={args.x} y={args.y} bands={bands} sha256={digest})")
    return 0


def main(argv: list[str] | None = None) -> int:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    p_synth = sub.add_parser("synth-cog", help="write the deterministic synthetic COG")
    p_synth.add_argument("out")
    p_synth.add_argument("--size", type=int, default=512)
    p_synth.add_argument("--nodata-corner", action="store_true")
    p_synth.set_defaults(func=cmd_synth_cog)

    p_render = sub.add_parser("render", help="render an XYZ tile to PNG")
    p_render.add_argument("cog")
    p_render.add_argument("z", type=int)
    p_render.add_argument("x", type=int)
    p_render.add_argument("y", type=int)
    p_render.add_argument("out")
    p_render.add_argument("--bands", default="1,2,3")
    p_render.add_argument("--rescale", default=None, metavar="MIN,MAX")
    p_render.add_argument(
        "--resampling",
        default="nearest",
        choices=("nearest", "bilinear", "cubic"),
        help="GDAL warp kernel (rio-tiler reproject_method); default matches rio-tiler",
    )
    p_render.add_argument(
        "--no-overviews",
        action="store_true",
        help="hide COG overviews so GDAL warps from the full-resolution grid",
    )
    p_render.add_argument(
        "--exact-grid",
        action="store_true",
        help="single-stage warp directly onto the 256-px tile grid (kernel goldens)",
    )
    p_render.set_defaults(func=cmd_render)

    p_compose = sub.add_parser(
        "compose", help="render a multi-file composite/band-math tile to PNG"
    )
    p_compose.add_argument("z", type=int)
    p_compose.add_argument("x", type=int)
    p_compose.add_argument("y", type=int)
    p_compose.add_argument("out")
    p_compose.add_argument(
        "--input",
        action="append",
        required=True,
        metavar="COG",
        help="input COG, band 1 (repeat; order defines b1..bN and R,G,B)",
    )
    p_compose.add_argument(
        "--expression",
        default=None,
        help="numpy arithmetic over b1..bN (grayscale output); omit for 3-input RGB",
    )
    p_compose.add_argument("--rescale", default=None, metavar="MIN,MAX")
    p_compose.add_argument(
        "--resampling",
        default="nearest",
        choices=("nearest", "bilinear", "cubic"),
        help="GDAL warp kernel, as in render",
    )
    p_compose.add_argument("--no-overviews", action="store_true")
    p_compose.add_argument("--exact-grid", action="store_true")
    p_compose.set_defaults(func=cmd_compose)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
