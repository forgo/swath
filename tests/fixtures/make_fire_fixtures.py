# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "earthaccess==0.15.1",
#     "rasterio==1.5.1",
#     "numpy==2.5.1",
# ]
# ///
"""Regenerate the committed multi-date HLS fire-event fixtures (issue #179).

Six HLSS30 v2.0 acquisitions of MGRS tile T10TFK spanning the 2024 Park Fire
(Butte/Tehama counties, California — ignited 2024-07-24, 429,603 acres, fully
contained 2024-09-26). One fixed 256x256 pixel window inside the burn
perimeter (Cohasset ridge / Ishi Wilderness area, ~40.05N 121.74W), four
bands per date: B04 (red), B8A (narrow NIR) for NDVI, B12 (SWIR2) for NBR,
and Fmask for QA. The window's mean NDVI runs ~0.74 pre-fire, collapses to
~0.27 in the first post-burn scene, then creeps up through autumn — the
burn progression the M7 time track demos against.

Same determinism contract as make_fixtures.py (whose helpers this reuses):
granules, bands, and window are hard-coded; fixtures are written as fresh
datasets; manifest.json/SHA256SUMS are merged, never clobbered, so each
script only owns its own entries. The NDVI quicklook strip is a hand-rolled
PNG (no plotting deps, no embedded metadata) — byte-stable under the pinned
zlib-backed stdlib.

Fixtures are immutable once committed (see README.md). Rerunning this script
must reproduce them byte-for-byte; any intentional change means new files and
PR discussion.

Usage: ``uv run tests/fixtures/make_fire_fixtures.py [--download-dir data/hls-src]``
"""

from __future__ import annotations

import argparse
import struct
import zlib
from pathlib import Path

import earthaccess
import numpy as np
import rasterio
from rasterio.windows import Window

from make_fixtures import FIXTURE_DIR, write_fixture, write_integrity

SHORT_NAME = "HLSS30"
VERSION = "2.0"

# Six Sentinel-2 acquisitions of tile T10TFK (UTM zone 10N, EPSG:32610)
# bracketing the 2024 Park Fire (ignited 2024-07-24 = day 206). All are
# 0-1% cloud tile-wide and cloud/shadow/nodata-free within WINDOW
# (scouted 2026-08-12 via Fmask):
#   2024159 = Jun 07  pre-fire green        2024249 = Sep 05  burn scar
#   2024204 = Jul 22  pre-fire (T-2 days)   2024274 = Sep 30  scar, contained
#   2024229 = Aug 16  fresh burn scar       2024289 = Oct 15  early post-fire
GRANULE_URS = (
    "HLS.S30.T10TFK.2024159T184919.v2.0",
    "HLS.S30.T10TFK.2024204T184921.v2.0",
    "HLS.S30.T10TFK.2024229T184919.v2.0",
    "HLS.S30.T10TFK.2024249T184919.v2.0",
    "HLS.S30.T10TFK.2024274T185211.v2.0",
    "HLS.S30.T10TFK.2024289T185249.v2.0",
)

# Red + narrow NIR (NDVI), SWIR2 (NBR), Fmask (QA) — per acceptance criteria.
BANDS = ("B04", "B8A", "B12", "Fmask")

# Fixed pixel window into the 3660x3660 T10TFK grid: fully inside the Park
# Fire perimeter, chosen for the largest mean NDVI drop (0.73 -> 0.30)
# between the Jul 22 and Sep 30 scenes with zero cloud/shadow/nodata on all
# six dates (scouted 2026-08-12). UL corner easting 607680, northing 4434720.
WINDOW = Window(col_off=256, row_off=2176, width=256, height=256)

QUICKLOOK = "hlss30-t10tfk-fire-ndvi-quicklook.png"


def sensing_day(granule_ur: str) -> str:
    """``HLS.S30.T10TFK.2024159T184919.v2.0`` -> ``2024159``."""
    return granule_ur.split(".")[3].split("T")[0]


def fixture_name(granule_ur: str, band: str) -> str:
    """Committed filename for one date x band fixture."""
    return f"hlss30-t10tfk-{sensing_day(granule_ur)}-{band.lower()}.tif"


def ndvi_rgb(red: np.ndarray, nir: np.ndarray) -> np.ndarray:
    """Colormap one date's NDVI to uint8 RGB (brown -> yellow -> green)."""
    r = red.astype(np.float64)
    n = nir.astype(np.float64)
    ndvi = (n - r) / np.maximum(n + r, 1.0)
    t = np.clip((ndvi - (-0.1)) / 1.0, 0.0, 1.0)  # -0.1..0.9 -> 0..1
    lo, mid, hi = (121, 74, 44), (222, 213, 116), (17, 87, 29)
    out = np.empty((*ndvi.shape, 3), dtype=np.uint8)
    for c in range(3):
        low_seg = lo[c] + (mid[c] - lo[c]) * (t / 0.5)
        high_seg = mid[c] + (hi[c] - mid[c]) * ((t - 0.5) / 0.5)
        out[..., c] = np.where(t < 0.5, low_seg, high_seg).round().astype(np.uint8)
    out[(r == -9999) | (n == -9999)] = 0
    # 2x2 block-mean downsample: keeps the quicklook strip small (~160 KB).
    half = out.reshape(out.shape[0] // 2, 2, out.shape[1] // 2, 2, 3)
    return half.mean(axis=(1, 3)).round().astype(np.uint8)


def write_png(path: Path, rgb: np.ndarray) -> None:
    """Minimal deterministic 8-bit RGB PNG writer (no ancillary chunks)."""
    height, width, _ = rgb.shape
    raw = b"".join(b"\x00" + rgb[i].tobytes() for i in range(height))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def main() -> int:
    """Download the pinned granules' bands, window them, write fixtures + quicklook."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--download-dir", default="data/hls-src")
    args = parser.parse_args()

    earthaccess.login(strategy="netrc")
    download_dir = Path(args.download_dir)
    download_dir.mkdir(parents=True, exist_ok=True)
    for granule_ur in GRANULE_URS:
        results = earthaccess.search_data(
            short_name=SHORT_NAME, version=VERSION, granule_ur=granule_ur
        )
        if len(results) != 1:
            raise SystemExit(f"expected exactly 1 granule for {granule_ur}, got {len(results)}")
        links = [
            link
            for link in results[0].data_links()
            if any(link.endswith(f".{band}.tif") for band in BANDS)
        ]
        if len(links) != len(BANDS):
            raise SystemExit(f"expected {len(BANDS)} band links, got {len(links)}: {links}")
        earthaccess.download(links, str(download_dir))

    manifest: dict[str, dict[str, object]] = {}
    panels: list[np.ndarray] = []
    for granule_ur in GRANULE_URS:
        for band in BANDS:
            name = fixture_name(granule_ur, band)
            manifest[name] = write_fixture(
                download_dir / f"{granule_ur}.{band}.tif",
                FIXTURE_DIR / name,
                band,
                window=WINDOW,
            )
            print(f"wrote {name} ({(FIXTURE_DIR / name).stat().st_size} bytes)")
        with rasterio.open(FIXTURE_DIR / fixture_name(granule_ur, "B04")) as src:
            red = src.read(1)
        with rasterio.open(FIXTURE_DIR / fixture_name(granule_ur, "B8A")) as src:
            nir = src.read(1)
        panels.append(ndvi_rgb(red, nir))

    gap = np.zeros((panels[0].shape[0], 4, 3), dtype=np.uint8)
    strip = np.concatenate([part for panel in panels for part in (panel, gap)][:-1], axis=1)
    write_png(FIXTURE_DIR / QUICKLOOK, strip)
    print(f"wrote {QUICKLOOK} ({(FIXTURE_DIR / QUICKLOOK).stat().st_size} bytes)")

    write_integrity(manifest)
    print(f"wrote manifest.json + SHA256SUMS ({len(manifest)} fixtures updated)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
