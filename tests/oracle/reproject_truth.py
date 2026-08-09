# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "pyproj==3.7.2",
# ]
# ///
"""Ground truth for the Reproject port (issue #23, ADR 0002).

Per ADR 0002, real PROJ lives ONLY in the test suite as a correctness
oracle. This script transforms a fixed point catalogue with pyproj (which
bundles PROJ) and emits a JSON truth table the swath-reproject-proj4rs
integration tests — and any future Reproject adapter (the PROJ C-binding
one) — compare against within documented per-pair tolerances.

The point catalogue pins the paths that matter for the HLS walking
skeleton, plus the classic edge cases:

* ``hls_fixture``     — corners + center of the committed HLS fixture
                        window (EPSG:32613; tests/fixtures/README.md),
                        against 3857 and 4326;
* ``tile_12_848_1561``— WebMercatorQuad bounds corners + center of the
                        z12 tile covering the fixture, against 32613;
* ``synth_tile_z6``   — bounds corners + center of the z6/10/24 synthetic
                        oracle tile (tests/oracle/render_reference.py),
                        against 4326;
* ``utm_south``       — southern-hemisphere UTM (EPSG:32755) points,
                        against 4326 and 3857;
* ``high_lat``        — the ±85.051129° Web Mercator edge and other
                        high-latitude points, 4326 against 3857;
* ``vnp09ga_sinu``    — corners + center of the VNP09GA h33v12 1-km grid
                        (MODIS-heritage spherical sinusoidal, **no EPSG
                        code** — named by its proj string, issue #39),
                        against 4326 and 3857. The h33v12 tile straddles
                        the antimeridian in longitude, so these points pin
                        PROJ's wrapping behavior too.

A case names each CRS side by ``from_epsg``/``to_epsg`` (a code) or
``from_proj4``/``to_proj4`` (a proj string) — the proj4 spelling was added
in #39 for sinusoidal; EPSG-only cases are byte-identical to the pre-#39
table.

Every (point set, CRS pair) appears in BOTH directions: the forward case
transforms the native points; the inverse case feeds the forward outputs
back through PROJ's inverse, so both legs are pinned on the same locus.

Output is deterministic: pure function of the constants below and the
pinned pyproj/PROJ versions (recorded in the provenance block).
Regenerate (and re-review) with:

    uv run tests/oracle/reproject_truth.py

which rewrites
crates/adapters/swath-reproject-proj4rs/tests/data/reproject_truth.json.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import pyproj
from pyproj import Transformer

REPO = Path(__file__).resolve().parents[2]
OUT = (
    REPO
    / "crates"
    / "adapters"
    / "swath-reproject-proj4rs"
    / "tests"
    / "data"
    / "reproject_truth.json"
)

# Web Mercator half-extent: pi * WGS84 semi-major axis (matches
# swath_core::tile::WEB_MERCATOR_EXTENT).
EXTENT = math.pi * 6378137.0
# Latitude of the Web Mercator square cutoff: atan(sinh(pi)).
MERC_LAT_LIMIT = math.degrees(math.atan(math.sinh(math.pi)))


def tile_points(z: int, x: int, y: int) -> list[tuple[float, float]]:
    """EPSG:3857 bounds corners + center of a WebMercatorQuad tile."""
    span = 2.0 * EXTENT / (1 << z)
    xmin = -EXTENT + x * span
    xmax = xmin + span
    ymax = EXTENT - y * span
    ymin = ymax - span
    return _corners_and_center(xmin, ymin, xmax, ymax)


def _corners_and_center(
    xmin: float, ymin: float, xmax: float, ymax: float
) -> list[tuple[float, float]]:
    return [
        (xmin, ymin),
        (xmin, ymax),
        (xmax, ymin),
        (xmax, ymax),
        ((xmin + xmax) / 2.0, (ymin + ymax) / 2.0),
    ]


# The VNP09GA sinusoidal CRS, exactly as swath-referencer's StructMetadata
# parser emits it (crates/swath-referencer/src/eos.rs).
SINU = "+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R=6371007.181 +units=m +no_defs"

# VNP09GA h33v12 1-km grid corners (StructMetadata.0 of the bake-off
# granule): UpperLeftPointMtrs / LowerRightMtrs.
VNP_UL = (16679257.795, -3335851.559)
VNP_LR = (17791208.314667, -4447802.078667)

# (set name, native CRS (EPSG int or proj string), points, partner CRSs).
POINT_SETS: list[tuple[str, int | str, list[tuple[float, float]], list[int | str]]] = [
    # HLS fixture window bounds, EPSG:32613 (tests/fixtures/README.md):
    # easting 453720-469080, northing 4338600-4353960.
    (
        "hls_fixture",
        32613,
        _corners_and_center(453720.0, 4338600.0, 469080.0, 4353960.0),
        [3857, 4326],
    ),
    # The z12 WebMercatorQuad tile covering the fixture window.
    ("tile_12_848_1561", 3857, tile_points(12, 848, 1561), [32613]),
    # The synthetic oracle tile (just oracle-verify renders z6/10/24).
    ("synth_tile_z6", 3857, tile_points(6, 10, 24), [4326]),
    # Southern-hemisphere UTM zone 55S (144°E-150°E): Melbourne and a
    # zone-edge point.
    (
        "utm_south",
        32755,
        None,  # filled below from geographic seeds
        [4326, 3857],
    ),
    # High latitudes: the exact ±Web Mercator cutoff, near-antimeridian.
    (
        "high_lat",
        4326,
        [
            (0.0, MERC_LAT_LIMIT),
            (179.999, 85.0),
            (-179.999, -MERC_LAT_LIMIT),
            (12.5, -85.0),
        ],
        [3857],
    ),
    # VNP09GA 1-km sinusoidal grid corners + center (issue #39).
    (
        "vnp09ga_sinu",
        SINU,
        _corners_and_center(VNP_UL[0], VNP_LR[1], VNP_LR[0], VNP_UL[1]),
        [4326, 3857],
    ),
]


def _crs_name(crs: int | str) -> str:
    return f"EPSG:{crs}" if isinstance(crs, int) else crs


def _crs_slug(crs: int | str) -> str:
    return str(crs) if isinstance(crs, int) else "sinu"


def _crs_fields(prefix: str, crs: int | str) -> dict:
    key = f"{prefix}_epsg" if isinstance(crs, int) else f"{prefix}_proj4"
    return {key: crs}

# utm_south native points: project geographic seeds once so the catalogue
# stores round-trippable native (easting, northing) values.
_UTM_SOUTH_SEEDS = [(144.9631, -37.8136), (149.9, -10.0)]
_to_32755 = Transformer.from_crs("EPSG:4326", "EPSG:32755", always_xy=True)
POINT_SETS[3] = (
    "utm_south",
    32755,
    [_to_32755.transform(lon, lat) for lon, lat in _UTM_SOUTH_SEEDS],
    [4326, 3857],
)


def main() -> int:
    cases = []
    for name, home, points, partners in POINT_SETS:
        assert points is not None
        for partner in partners:
            fwd = Transformer.from_crs(_crs_name(home), _crs_name(partner), always_xy=True)
            inv = Transformer.from_crs(_crs_name(partner), _crs_name(home), always_xy=True)
            out = [fwd.transform(x, y) for x, y in points]
            back = [inv.transform(x, y) for x, y in out]
            for p in out + back:
                assert all(math.isfinite(v) for v in p), (name, home, partner, p)
            cases.append(
                {
                    "name": f"{name}_{_crs_slug(home)}_to_{_crs_slug(partner)}",
                    **_crs_fields("from", home),
                    **_crs_fields("to", partner),
                    "input": [list(p) for p in points],
                    "expected": [list(p) for p in out],
                }
            )
            cases.append(
                {
                    "name": f"{name}_{_crs_slug(partner)}_to_{_crs_slug(home)}",
                    **_crs_fields("from", partner),
                    **_crs_fields("to", home),
                    "input": [list(p) for p in out],
                    "expected": [list(p) for p in back],
                }
            )

    doc = {
        "provenance": {
            "generator": "tests/oracle/reproject_truth.py",
            "pyproj": pyproj.__version__,
            "proj": pyproj.proj_version_str,
        },
        "cases": cases,
    }
    OUT.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    print(f"wrote {OUT.relative_to(REPO)}: {len(cases)} cases")
    return 0


if __name__ == "__main__":
    sys.exit(main())
