#!/usr/bin/env bash
# Fetch sample granules for the bake-off. Run from the prototype directory.
#   VIIRS VNP09 (HDF5)  -> the legacy-primary target (needs a free NASA Earthdata Login)
#   GRIB2 sample        -> the pure-Rust-friendly weather format
set -euo pipefail
mkdir -p data

echo "== VIIRS VNP09 (HDF5) via NASA earthaccess =="
python3 - <<'PY' || echo "  (skipped: install 'earthaccess' and log in, then re-run)"
import sys
try:
    import earthaccess
except ImportError:
    sys.exit("earthaccess not installed (pip install earthaccess)")
earthaccess.login(persist=True)  # prompts for / reuses your NASA Earthdata Login
results = earthaccess.search_data(short_name="VNP09", count=1)
if not results:
    sys.exit("no VNP09 granules found for the query")
earthaccess.download(results, "data")
print("  downloaded a VNP09 granule into data/")
PY

echo "== GRIB2 sample =="
echo "  Download a small GRIB2 file (e.g. a GFS or HRRR subset from NOAA NOMADS) into ./data/,"
echo "  then run: just rust-gen data/<file>.grib2  and  just vz-gen data/<file>.grib2"
echo
echo "Done. Granules (if any) are in ./data/. Next: 'just bakeoff data/<granule>'."
