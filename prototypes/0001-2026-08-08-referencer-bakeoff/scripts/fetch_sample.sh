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

echo "== GRIB2 sample (GFS 0.25deg subset via AWS Open Data, no auth) =="
# Byte-range three fields out of one GFS analysis file using its .idx sidecar and concatenate
# them into a small standalone multi-message GRIB2 file (each range is a complete GRIB2 message).
# Bucket: https://noaa-gfs-bdp-pds.s3.amazonaws.com/ (NOAA Open Data Dissemination, public).
GFS_KEY="gfs.20260801/00/atmos/gfs.t00z.pgrb2.0p25.f000"
GFS_URL="https://noaa-gfs-bdp-pds.s3.amazonaws.com/${GFS_KEY}"
GRIB_OUT="data/gfs_sample.grib2"
if [ -s "$GRIB_OUT" ]; then
  echo "  $GRIB_OUT already exists, skipping"
else
  idx=$(curl -fsS "${GFS_URL}.idx")
  : > "$GRIB_OUT"
  for field in ":TMP:850 mb:" ":UGRD:10 m above ground:" ":PRMSL:mean sea level:"; do
    # idx line format: msgnum:byte_offset:date:VAR:level:forecast: — end = next message's offset - 1
    start=$(printf '%s\n' "$idx" | grep -F "$field" | head -1 | cut -d: -f2)
    end=$(printf '%s\n' "$idx" | cut -d: -f2 | awk -v s="$start" '$1 > s' | sort -n | head -1)
    [ -n "$start" ] || { echo "  field '$field' not in index" >&2; exit 1; }
    curl -fsS -r "${start}-$((end - 1))" "$GFS_URL" >> "$GRIB_OUT"
    echo "  + ${field} bytes ${start}-$((end - 1))"
  done
  echo "  wrote $GRIB_OUT ($(wc -c < "$GRIB_OUT" | tr -d ' ') bytes)"
fi
echo
echo "Done. Granules (if any) are in ./data/. Next: 'just bakeoff data/<granule>'."
