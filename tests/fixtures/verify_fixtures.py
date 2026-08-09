# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "rasterio==1.5.1",
# ]
# ///
"""Offline sanity loader for the committed HLS fixtures (issue #20).

Opens every fixture listed in ``manifest.json`` with GDAL's network layer
hard-disabled (``CPL_CURL_ENABLE=NO`` — any attempted remote read fails
loudly) and asserts CRS, shape, dtype, nodata, band count, and geotransform
match the manifest. Checksum verification is the other half of
``just fixtures-verify``.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

os.environ["CPL_CURL_ENABLE"] = "NO"  # any network read fails loudly

import rasterio  # noqa: E402

FIXTURE_DIR = Path(__file__).parent


def main() -> int:
    """Assert every fixture matches its manifest entry; return 0 iff all pass."""
    manifest: dict[str, dict[str, object]] = json.loads(
        (FIXTURE_DIR / "manifest.json").read_text()
    )
    failures = 0
    for name, expect in sorted(manifest.items()):
        with rasterio.open(FIXTURE_DIR / name) as src:
            got = {
                "band": expect["band"],
                "crs": str(src.crs),
                "width": src.width,
                "height": src.height,
                "count": src.count,
                "dtype": src.dtypes[0],
                "nodata": src.nodata,
                "transform": list(src.transform)[:6],
            }
        if got != expect:
            failures += 1
            print(f"FAIL {name}: expected {expect}, got {got}", file=sys.stderr)
        else:
            print(f"ok {name}: {got['crs']} {got['width']}x{got['height']} "
                  f"{got['dtype']} nodata={got['nodata']}")
    if failures:
        print(f"fixtures-verify: {failures} fixture(s) failed", file=sys.stderr)
        return 1
    print(f"fixtures-verify: all {len(manifest)} fixtures match the manifest")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
