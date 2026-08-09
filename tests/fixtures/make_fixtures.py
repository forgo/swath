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
"""Regenerate the committed HLS COG fixtures (issue #20).

Downloads five per-band COGs of one real HLSS30 v2.0 granule from LP DAAC
(Earthdata credentials via ~/.netrc), windows them to a fixed 512x512 subset,
and writes proper COGs (tiled 256, deflate, one overview) plus
``manifest.json`` and ``SHA256SUMS``.

Determinism contract
--------------------
* The source granule, band list, and pixel window are hard-coded constants.
  HLS granules are immutable once published, so the input bytes are fixed.
* Each fixture is written as a FRESH dataset: only the windowed pixels, the
  windowed geotransform, the native CRS, and the source nodata value carry
  over. No source metadata tags (processing timestamps, software versions,
  per-run identifiers) are copied, and GDAL's TIFF/COG writer embeds no
  timestamps of its own, so output bytes depend only on (source pixels,
  constants below, pinned library versions above).
* ``manifest.json`` is serialized with sorted keys; ``SHA256SUMS`` lists
  files in sorted order.

Fixtures are immutable once committed (see README.md). Rerunning this script
must reproduce them byte-for-byte; any intentional change means new files and
PR discussion.

Usage: ``uv run tests/fixtures/make_fixtures.py [--download-dir data/hls-src]``
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import earthaccess
import rasterio
from rasterio.windows import Window

FIXTURE_DIR = Path(__file__).parent

# One real HLSS30 v2.0 granule (Sentinel-2 B on 2024-06-06, MGRS tile T13SDD,
# UTM zone 13N, southern Colorado). Chosen for 46% spatial coverage — the
# Sentinel-2 swath edge crosses the tile, giving REAL nodata — with 2% cloud.
GRANULE_UR = "HLS.S30.T13SDD.2024158T173909.v2.0"
SHORT_NAME = "HLSS30"
VERSION = "2.0"

# Bands: RGB + narrow NIR (the pair HLS's own NDVI product uses) + Fmask.
BANDS = ("B02", "B03", "B04", "B8A", "Fmask")

# Fixed pixel window into the 3660x3660 source grid. Chosen so ~31% of the
# window is swath-edge nodata and the rest is clear land (scouted 2026-08-08).
WINDOW = Window(col_off=1792, row_off=1536, width=512, height=512)


def fixture_name(band: str) -> str:
    """Committed filename for one band's fixture."""
    return f"hlss30-t13sdd-2024158-{band.lower()}.tif"


def write_fixture(src_path: Path, out_path: Path, band: str) -> dict[str, object]:
    """Window one source band into a fresh deterministic COG; return manifest entry."""
    with rasterio.open(src_path) as src:
        data = src.read(1, window=WINDOW)
        profile = {
            "driver": "COG",
            "dtype": src.dtypes[0],
            "count": 1,
            "width": int(WINDOW.width),
            "height": int(WINDOW.height),
            "crs": src.crs,
            "transform": src.window_transform(WINDOW),
            "nodata": src.nodata,
            "compress": "deflate",
            "blocksize": 256,
            "overview_count": 1,
            # Categorical Fmask must not blend classes across pixels.
            "overview_resampling": "nearest" if band == "Fmask" else "average",
        }
    with rasterio.open(out_path, "w", **profile) as dst:
        dst.write(data, 1)
    with rasterio.open(out_path) as chk:
        return {
            "band": band,
            "crs": str(chk.crs),
            "width": chk.width,
            "height": chk.height,
            "count": chk.count,
            "dtype": chk.dtypes[0],
            "nodata": chk.nodata,
            "transform": list(chk.transform)[:6],
        }


def main() -> int:
    """Download the pinned granule's bands, window them, write manifest + checksums."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--download-dir", default="data/hls-src")
    args = parser.parse_args()

    earthaccess.login(strategy="netrc")
    results = earthaccess.search_data(
        short_name=SHORT_NAME, version=VERSION, granule_ur=GRANULE_UR
    )
    if len(results) != 1:
        raise SystemExit(f"expected exactly 1 granule for {GRANULE_UR}, got {len(results)}")
    links = [
        link
        for link in results[0].data_links()
        if any(link.endswith(f".{band}.tif") for band in BANDS)
    ]
    if len(links) != len(BANDS):
        raise SystemExit(f"expected {len(BANDS)} band links, got {len(links)}: {links}")
    download_dir = Path(args.download_dir)
    download_dir.mkdir(parents=True, exist_ok=True)
    earthaccess.download(links, str(download_dir))

    manifest: dict[str, dict[str, object]] = {}
    for band in BANDS:
        name = fixture_name(band)
        manifest[name] = write_fixture(
            download_dir / f"{GRANULE_UR}.{band}.tif", FIXTURE_DIR / name, band
        )
        size = (FIXTURE_DIR / name).stat().st_size
        print(f"wrote {name} ({size} bytes)")

    manifest_path = FIXTURE_DIR / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")

    sums = []
    for name in sorted([*manifest, "manifest.json"]):
        digest = hashlib.sha256((FIXTURE_DIR / name).read_bytes()).hexdigest()
        sums.append(f"{digest}  {name}")
    (FIXTURE_DIR / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    print(f"wrote manifest.json + SHA256SUMS ({len(manifest)} fixtures)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
