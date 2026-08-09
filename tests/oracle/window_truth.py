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

Output is deterministic: values are pure functions of the immutable fixture
bytes and the pinned rasterio/numpy versions. Regenerate (and re-review) with:

    uv run tests/oracle/window_truth.py

which rewrites crates/adapters/swath-source-cog/tests/data/window_truth.json.
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
OUT = (
    REPO
    / "crates"
    / "adapters"
    / "swath-source-cog"
    / "tests"
    / "data"
    / "window_truth.json"
)

# (name, col_off, row_off, width, height) — requested, before clipping.
WINDOWS: list[tuple[str, int, int, int, int]] = [
    ("full", 0, 0, 512, 512),
    ("interior", 192, 192, 128, 128),
    ("nodata_edge", 128, 0, 128, 128),
    ("one_pixel", 256, 256, 1, 1),
    ("oob_clipped", 448, 480, 128, 128),
]

FILES = sorted(p.name for p in FIXTURES.glob("*.tif"))


def clip(col: int, row: int, w: int, h: int, width: int, height: int) -> tuple[int, int, int, int]:
    """Intersect a requested window with the raster grid."""
    c0, r0 = max(col, 0), max(row, 0)
    c1, r1 = min(col + w, width), min(row + h, height)
    return c0, r0, max(c1 - c0, 0), max(r1 - r0, 0)


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


if __name__ == "__main__":
    main()
