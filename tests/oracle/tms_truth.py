# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "morecantile==6.2.0",
# ]
# ///
"""WebMercatorQuad ground-truth table for swath-core's TMS math (issue #21).

Prints, as JSON on stdout, morecantile's WebMercatorQuad bounds for a fixed
list of tiles. The output is committed VERBATIM as
``crates/swath-core/tests/data/tms_truth.json`` and asserted against by the
Rust integration test ``crates/swath-core/tests/tms_truth.rs`` (tolerance
1e-6 m). The truth is pinned: regenerating it means rerunning this script
(``uv run tests/oracle/tms_truth.py > crates/swath-core/tests/data/tms_truth.json``)
and reviewing the diff — the committed JSON, not morecantile-at-HEAD, is the
oracle CI sees.

Tile list (fixed, never grown silently):
* 0/0/0            — the root: the full Web Mercator plane
* 1/{0,1}/{0,1}    — all four z1 tiles: quadrant edges and shared midlines
* 6/10/24          — the synthetic-COG oracle tile (tests/oracle, issue #19)
* 12/848/1561      — the tile over the committed HLS fixture window
                     (tests/fixtures/README.md, T13SDD)
"""

import json
import sys

import morecantile

TILES: list[tuple[int, int, int]] = [
    (0, 0, 0),
    (1, 0, 0),
    (1, 1, 0),
    (1, 0, 1),
    (1, 1, 1),
    (6, 10, 24),
    (12, 848, 1561),
]


def main() -> None:
    tms = morecantile.tms.get("WebMercatorQuad")
    rows = []
    for z, x, y in TILES:
        tile = morecantile.Tile(x=x, y=y, z=z)
        xy = tms.xy_bounds(tile)
        ll = tms.bounds(tile)
        rows.append(
            {
                "z": z,
                "x": x,
                "y": y,
                "xy_bounds": {
                    "min_x": xy.left,
                    "min_y": xy.bottom,
                    "max_x": xy.right,
                    "max_y": xy.top,
                },
                "lonlat_bounds": {
                    "west": ll.left,
                    "south": ll.bottom,
                    "east": ll.right,
                    "north": ll.top,
                },
            }
        )
    doc = {
        "generator": "tests/oracle/tms_truth.py",
        "morecantile": morecantile.__version__,
        "tms": "WebMercatorQuad",
        "tiles": rows,
    }
    json.dump(doc, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
