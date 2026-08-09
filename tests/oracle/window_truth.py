# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "rasterio==1.5.1",
#     "numpy==2.5.1",
# ]
# ///
"""Ground truth for RasterSource windowed reads (issue #22, ADR 0002).

Per ADR 0002 GDAL lives ONLY in the test suite. This script reads a fixed
set of pixel windows from every committed HLS fixture (tests/fixtures/) with
rasterio/GDAL and emits a JSON truth table the swath-source-cog integration
tests compare against EXACTLY (SHA-256 of the raw little-endian pixel bytes
— not perceptually).

The window list exercises the read paths that matter:

* ``full``        — the whole 512x512 grid (all four 256px internal tiles);
* ``interior``    — a 128x128 window crossing both internal tile seams;
* ``nodata_edge`` — a 128x128 window straddling the real Sentinel-2 swath
  edge (the script *asserts* it contains both nodata and valid pixels);
* ``one_pixel``   — a single pixel (must touch exactly one tile);
* ``oob_clipped`` — a request extending past the grid edge, expected to be
  clipped to the intersection with the raster.

Overview truth (issue #38): the same idea for the overview IFD every fixture
carries (decimation 2). Requests are specified in FULL-RESOLUTION pixel
coordinates plus a level factor — exactly the ``RasterSource`` port's
coordinate-space contract — and this script replicates the port's rounding
contract (cover: ``floor(off / ratio)`` .. ``ceil(end / ratio)``, exact
per-axis ratio ``full_dim / overview_dim``, then clip) to derive the
overview-grid window it reads. Pixels come from an EXPLICIT overview open
(``rasterio.open(..., overview_level=0)``), so the hashed bytes are the
overview IFD's stored samples — the same bytes async-tiff decodes — never a
decimated read of full resolution. The overview grid's dimensions and
transform are recorded so the adapter's reported grid is pinned too.

Output is deterministic: values are pure functions of the immutable fixture
bytes and the pinned rasterio/numpy versions. Regenerate (and re-review) with:

    uv run tests/oracle/window_truth.py

which rewrites crates/adapters/swath-source-cog/tests/data/window_truth.json
and .../overview_truth.json.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

import numpy as np
import rasterio
from rasterio.windows import Window

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "tests" / "fixtures"
DATA = REPO / "crates" / "adapters" / "swath-source-cog" / "tests" / "data"
OUT = DATA / "window_truth.json"
OUT_OVERVIEW = DATA / "overview_truth.json"

# (name, col_off, row_off, width, height) — requested, before clipping.
WINDOWS: list[tuple[str, int, int, int, int]] = [
    ("full", 0, 0, 512, 512),
    ("interior", 192, 192, 128, 128),
    ("nodata_edge", 128, 0, 128, 128),
    ("one_pixel", 256, 256, 1, 1),
    ("oob_clipped", 448, 480, 128, 128),
]

FILES = sorted(p.name for p in FIXTURES.glob("*.tif"))

# Overview truth requests, in FULL-RESOLUTION coordinates (name, col_off,
# row_off, width, height). Chosen to exercise the rounding contract: exact
# tile alignment, odd offsets/sizes that force floor/ceil to differ, a
# single pixel, the swath nodata edge, and an out-of-bounds clip.
OVERVIEW_WINDOWS: list[tuple[str, int, int, int, int]] = [
    ("full", 0, 0, 512, 512),
    ("interior_odd", 191, 193, 130, 127),
    ("nodata_edge", 128, 0, 128, 128),
    ("one_pixel", 256, 256, 1, 1),
    ("oob_clipped", 448, 480, 128, 128),
]


def clip(col: int, row: int, w: int, h: int, width: int, height: int) -> tuple[int, int, int, int]:
    """Intersect a requested window with the raster grid."""
    c0, r0 = max(col, 0), max(row, 0)
    c1, r1 = min(col + w, width), min(row + h, height)
    return c0, r0, max(c1 - c0, 0), max(r1 - r0, 0)


def cover(off: int, size: int, ratio: float, grid_dim: int) -> tuple[int, int]:
    """The port's rounding contract: the smallest grid window covering a
    full-resolution span — floor the start, ceil the end, then clip."""
    import math

    lo = max(math.floor(off / ratio), 0)
    hi = math.ceil((off + size) / ratio)
    lo, hi = min(lo, grid_dim), min(hi, grid_dim)
    return lo, max(hi - lo, 0)


def overview_cases() -> list[dict[str, object]]:
    cases: list[dict[str, object]] = []
    for name in FILES:
        with rasterio.open(FIXTURES / name) as full:
            factors = full.overviews(1)
            assert factors == [2], f"{name}: expected exactly one x2 overview, got {factors}"
            full_w, full_h = full.width, full.height
        with rasterio.open(FIXTURES / name, overview_level=0) as ov:
            rx, ry = full_w / ov.width, full_h / ov.height
            nodata = ov.nodata
            for wname, col, row, w, h in OVERVIEW_WINDOWS:
                cc, cw = cover(col, w, rx, ov.width)
                cr, ch = cover(row, h, ry, ov.height)
                data = ov.read(1, window=Window(cc, cr, cw, ch))
                assert data.shape == (ch, cw)
                le = np.ascontiguousarray(data).astype(data.dtype.newbyteorder("<"))
                mask = data == np.array(nodata, dtype=data.dtype)
                if wname == "nodata_edge":
                    assert mask.any() and not mask.all(), (
                        f"{name}/{wname}: overview window does not straddle the nodata edge"
                    )
                flat = data.ravel()
                cases.append(
                    {
                        "file": name,
                        "band": 0,
                        "factor": 2,
                        "window_name": wname,
                        "requested": {"col_off": col, "row_off": row, "width": w, "height": h},
                        "clipped": {"col_off": cc, "row_off": cr, "width": cw, "height": ch},
                        "grid": {
                            "width": ov.width,
                            "height": ov.height,
                            "transform": list(ov.transform)[:6],
                        },
                        "dtype": str(data.dtype),
                        "nodata": nodata,
                        "nodata_count": int(mask.sum()),
                        "valid_sum": int(flat[~mask.ravel()].astype(np.int64).sum()),
                        "first8": [int(v) for v in flat[:8]],
                        "last8": [int(v) for v in flat[-8:]],
                        "sha256_le": hashlib.sha256(le.tobytes()).hexdigest(),
                    }
                )
    return cases


def main() -> None:
    cases = []
    for name in FILES:
        with rasterio.open(FIXTURES / name) as ds:
            assert ds.count == 1, f"{name}: expected single-band fixture"
            nodata = ds.nodata
            for wname, col, row, w, h in WINDOWS:
                cc, cr, cw, ch = clip(col, row, w, h, ds.width, ds.height)
                data = ds.read(1, window=Window(cc, cr, cw, ch))
                assert data.shape == (ch, cw)
                le = np.ascontiguousarray(data).astype(data.dtype.newbyteorder("<"))
                raw = le.tobytes()
                mask = data == np.array(nodata, dtype=data.dtype)
                if wname == "nodata_edge":
                    assert mask.any() and not mask.all(), (
                        f"{name}/{wname}: window does not straddle the nodata edge"
                    )
                flat = data.ravel()
                cases.append(
                    {
                        "file": name,
                        "band": 0,
                        "window_name": wname,
                        "requested": {"col_off": col, "row_off": row, "width": w, "height": h},
                        "clipped": {"col_off": cc, "row_off": cr, "width": cw, "height": ch},
                        "dtype": str(data.dtype),
                        "nodata": nodata,
                        "nodata_count": int(mask.sum()),
                        "valid_sum": int(flat[~mask.ravel()].astype(np.int64).sum()),
                        "first8": [int(v) for v in flat[:8]],
                        "last8": [int(v) for v in flat[-8:]],
                        "sha256_le": hashlib.sha256(raw).hexdigest(),
                    }
                )
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"cases": cases}, indent=2, sort_keys=True) + "\n")
    print(f"wrote {len(cases)} cases to {OUT.relative_to(REPO)}", file=sys.stderr)
    ov_cases = overview_cases()
    OUT_OVERVIEW.write_text(json.dumps({"cases": ov_cases}, indent=2, sort_keys=True) + "\n")
    print(
        f"wrote {len(ov_cases)} overview cases to {OUT_OVERVIEW.relative_to(REPO)}",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
