#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# THE FIRE DROP (ADR 0015 / issue #180): the six-date Park Fire series
# (tests/fixtures/README.md) into the watched drop directory, same
# convention as drop-granule.sh — band COGs land first, each manifest is
# staged under an ignored dotfile name and renamed into place last.
# Acquisition datetimes are the dates' nominal T10TFK overpass time; the
# swath-e2e harness asserts `datetime=` frame selection, cache identity,
# and the derived temporal extent (2024-06-07..2024-10-15) against them.
set -euo pipefail

dir=target/e2e
bbox='[-121.7388, 39.9856, -121.6475, 40.0559]'

drop() { # day datetime
  local granule="hlss30-t10tfk-$1"
  cp tests/fixtures/$granule-*.tif "$dir/store/"
  printf '%s\n' \
    '{' \
    '  "dataset": "hls-s30-fire",' \
    "  \"granule\": \"$granule\"," \
    "  \"bbox\": $bbox," \
    "  \"datetime\": \"$2\"," \
    '  "assets": {' \
    "    \"b04\": \"$granule-b04.tif\"," \
    "    \"b12\": \"$granule-b12.tif\"," \
    "    \"b8a\": \"$granule-b8a.tif\"," \
    "    \"fmask\": \"$granule-fmask.tif\"" \
    '  }' \
    '}' > "$dir/store/drop/.$granule.json"
  mv "$dir/store/drop/.$granule.json" "$dir/store/drop/$granule.json"
}

drop 2024159 "2024-06-07T19:03:00Z"
drop 2024204 "2024-07-22T19:03:00Z"
drop 2024229 "2024-08-16T19:03:00Z"
drop 2024249 "2024-09-05T19:03:00Z"
drop 2024274 "2024-09-30T19:03:00Z"
drop 2024289 "2024-10-15T19:03:00Z"
echo "swath: park fire series dropped at $(date -u '+%H:%M:%S') UTC"
