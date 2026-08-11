# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "numpy==2.5.1",
# ]
# ///
"""Ground truth for the docs/EXTENDING.md toy source adapter (issue #125).

The oracle pattern (ADR 0002), applied to `swath-source-inmem`: this script
builds the documented demo raster with numpy — independently of the Rust
adapter — and emits the JSON truth table its integration tests compare
against EXACTLY (SHA-256 of the raw little-endian pixel bytes).

The demo raster is the pure function documented in the adapter's module
docs: a 6x4 uint8 grid, ``v(row, col) = row * 16 + col * 3`` with nodata
sentinel 255 planted at (1, 2) and (2, 4).

The window list exercises the read paths the port contract requires:

* ``full``        — the whole 6x4 grid;
* ``interior``    — a 3x2 window overlapping one planted sentinel;
* ``one_pixel``   — a single pixel;
* ``oob_clipped`` — a request extending past the grid edge, expected to be
  clipped to the intersection with the raster.

Output is deterministic. Regenerate (and re-review) with:

    uv run tests/oracle/inmem_truth.py

which rewrites crates/adapters/swath-source-inmem/tests/data/window_truth.json.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

import numpy as np

NODATA = 255
ASSET = "inmem:demo"

WINDOWS = {
    "full": (0, 0, 6, 4),
    "interior": (2, 1, 3, 2),
    "one_pixel": (3, 2, 1, 1),
    "oob_clipped": (4, 2, 10, 10),
}


def demo_raster() -> np.ndarray:
    rows, cols = np.mgrid[0:4, 0:6]
    grid = (rows * 16 + cols * 3).astype(np.uint8)
    grid[1, 2] = NODATA
    grid[2, 4] = NODATA
    return grid


def case(name: str, requested: tuple[int, int, int, int], grid: np.ndarray) -> dict:
    col_off, row_off, width, height = requested
    height_px, width_px = grid.shape
    col1 = min(col_off + width, width_px)
    row1 = min(row_off + height, height_px)
    window = grid[row_off:row1, col_off:col1]
    samples = window.astype("<u1").ravel()
    valid = samples[samples != NODATA]
    flat = [int(s) for s in samples]
    return {
        "asset": ASSET,
        "window_name": name,
        "requested": {"col_off": col_off, "row_off": row_off, "width": width, "height": height},
        "clipped": {
            "col_off": col_off,
            "row_off": row_off,
            "width": col1 - col_off,
            "height": row1 - row_off,
        },
        "dtype": str(samples.dtype),
        "nodata": float(NODATA),
        "nodata_count": int((samples == NODATA).sum()),
        "valid_sum": int(valid.sum()),
        "first8": flat[:8],
        "last8": flat[-8:],
        "sha256_le": hashlib.sha256(samples.tobytes()).hexdigest(),
    }


def main() -> None:
    grid = demo_raster()
    cases = [case(name, req, grid) for name, req in WINDOWS.items()]
    out = Path("crates/adapters/swath-source-inmem/tests/data/window_truth.json")
    out.write_text(json.dumps({"cases": cases}, indent=2) + "\n")
    print(f"wrote {out} ({len(cases)} cases)")


if __name__ == "__main__":
    main()
