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
* ``render COG z x y OUT.png [--bands 1,2,3] [--rescale MIN,MAX]``
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.transform import from_bounds
from rio_tiler.io import Reader

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
    cog: str, z: int, x: int, y: int, bands: tuple[int, ...], rescale: tuple[float, float] | None
) -> bytes:
    """Render one XYZ tile of ``cog`` to PNG bytes via rio-tiler."""
    with Reader(cog) as reader:
        img = reader.tile(x, y, z, indexes=bands)
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

    first = render_tile_png(args.cog, args.z, args.x, args.y, bands, rescale)
    second = render_tile_png(args.cog, args.z, args.x, args.y, bands, rescale)
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
    p_render.set_defaults(func=cmd_render)

    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
