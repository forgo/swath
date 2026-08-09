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

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
