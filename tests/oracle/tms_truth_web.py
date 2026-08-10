# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "morecantile==6.2.0",
# ]
# ///
"""WebMercatorQuad ground-truth table for the web components' TMS math (issue #106).

Prints, as JSON on stdout, morecantile's WebMercatorQuad answers for a fixed
list of cases. The output is committed VERBATIM as ``web/src/tms_truth.json``
and asserted against by ``web/src/tms.test.ts`` — the TS twin of the Rust
oracle test ``crates/swath-core/tests/tms_truth.rs`` (whose table
``tests/oracle/tms_truth.py`` generates). The truth is pinned: regenerating it
means running ``just tms-truth-web`` and reviewing the diff — the committed
JSON, not morecantile-at-HEAD, is the oracle CI sees.

Two tables, matching the two functions ``web/src/tms.ts`` exports:

* ``center_tiles`` — (lon, lat, zoom) → containing tile, via ``tms.tile()``.
  Asserted EXACTLY (integer tile addresses). The point grid crosses a fixed
  zoom list with points spanning the quadrant seams, both antimeridian sides
  (lon ±180 exactly and just inside), the Web Mercator latitude edges
  (±85.0511 and poleward ±90, which morecantile clamps to the TMS bbox — the
  same answer as the TS clamp), and the repo's landmark tiles (the synthetic
  COG oracle tile 6/10/24 and the HLS fixture tile 12/848/1561).
* ``northwest_corners`` — tile (z, x, y) → upper-left lon/lat, via
  ``tms.ul()``. Asserted within 1e-9 degrees (the tolerance twin of the Rust
  test's 1e-6 m): morecantile inverts through pyproj while the TS closed form
  uses atan/sinh, so the two agree only to ~1e-13 degrees, not bit-exactly.
  Includes x = 2^z and y = 2^z (one past the last tile) because the overlay
  derives a tile's southeast corner as the northwest of (x+1, y+1).
"""

import json
import sys
import warnings

import morecantile

# Fixed case lists — never grown silently; adding a case is a reviewed diff.
ZOOMS: list[int] = [0, 1, 2, 4, 6, 8, 12, 16, 22]

POINTS: list[tuple[float, float]] = [
    (0.0, 0.0),  # quadrant seam
    (-0.0001, 0.0001),  # just NW of the seam
    (-180.0, 0.0),  # antimeridian, west edge
    (180.0, 0.0),  # antimeridian, east edge (clamps into the last column)
    (-179.9999999, 45.0),  # just east of the antimeridian
    (179.9999999, -45.0),  # just west of the antimeridian
    (0.0, 85.0511),  # north Web Mercator edge (the TS clamp constant)
    (0.0, -85.0511),  # south Web Mercator edge
    (0.0, 90.0),  # pole — clamped to the bbox, same tile as the edge
    (0.0, -90.0),
    (-180.0, 85.0511),  # NW corner of the plane
    (180.0, -85.0511),  # SE corner of the plane
    (-105.42, 39.27),  # inside the HLS fixture tile 12/848/1561 (T13SDD)
    (-120.9375, 38.8),  # inside the synthetic-COG tile 6/10/24
    (2.3522, 48.8566),  # Paris — an ordinary mid-latitude point
    (151.2093, -33.8688),  # Sydney — southern hemisphere, far east
    (-43.1729, -22.9068),  # Rio — southern hemisphere, far west
]

NW_TILES: list[tuple[int, int, int]] = [
    (0, 0, 0),
    (0, 1, 1),  # SE corner of the root, via the x+1/y+1 convention
    *((1, x, y) for x in range(3) for y in range(3)),  # all z1 corners incl. x=y=2
    (6, 10, 24),  # the synthetic-COG oracle tile
    (6, 11, 25),  # its SE corner
    (12, 848, 1561),  # the HLS fixture tile
    (12, 849, 1562),  # its SE corner
    (12, 4095, 0),  # last column, top row
    (12, 4096, 4096),  # one past both edges: lon 180, lat -85.051...
    (22, 0, 0),
    (22, 4194303, 4194304),
    (3, 8, 8),
    (5, 17, 11),
    (8, 255, 128),
    (16, 32768, 32768),
]


def main() -> None:
    tms = morecantile.tms.get("WebMercatorQuad")
    center_tiles = []
    # The poleward points (lat ±90) are deliberate: morecantile warns and
    # clamps them to the TMS bbox, which is exactly the behavior under test.
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        for zoom in ZOOMS:
            for lon, lat in POINTS:
                tile = tms.tile(lon, lat, zoom)
                center_tiles.append(
                    {"lon": lon, "lat": lat, "zoom": zoom, "z": tile.z, "x": tile.x, "y": tile.y}
                )
    northwest_corners = []
    for z, x, y in NW_TILES:
        ul = tms.ul(morecantile.Tile(x=x, y=y, z=z))
        northwest_corners.append({"z": z, "x": x, "y": y, "lon": ul.x, "lat": ul.y})
    doc = {
        "generator": "tests/oracle/tms_truth_web.py",
        "command": "just tms-truth-web",
        "morecantile": morecantile.__version__,
        "tms": "WebMercatorQuad",
        "center_tiles": center_tiles,
        "northwest_corners": northwest_corners,
    }
    json.dump(doc, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
