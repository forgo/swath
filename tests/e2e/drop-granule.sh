#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# THE DROP (the manual step count for everything after it: zero) — the
# single source of truth for the filedrop convention: band COGs land
# first, the manifest is staged under an ignored dotfile name and renamed
# into place last. Extracted from stack-up.sh (issue #98) so the bash
# bring-up path (`just e2e-web`, `just demo`) and the typed harness
# (`swath-e2e`, which asserts the pre-drop 404 first) share one drop.
set -euo pipefail

dir=target/e2e
granule=hlss30-t13sdd-2024158
cp tests/fixtures/$granule-*.tif "$dir/store/"
printf '%s\n' \
  '{' \
  '  "dataset": "hls-s30",' \
  "  \"granule\": \"$granule\"," \
  '  "bbox": [-105.5370, 39.1954, -105.3581, 39.3345],' \
  '  "datetime": "2024-06-06T17:54:00Z",' \
  '  "assets": {' \
  "    \"b02\": \"$granule-b02.tif\"," \
  "    \"b03\": \"$granule-b03.tif\"," \
  "    \"b04\": \"$granule-b04.tif\"," \
  "    \"b8a\": \"$granule-b8a.tif\"," \
  "    \"fmask\": \"$granule-fmask.tif\"" \
  '  }' \
  '}' > "$dir/store/drop/.$granule.json"
mv "$dir/store/drop/.$granule.json" "$dir/store/drop/$granule.json"
echo "swath: granule dropped at $(date -u '+%H:%M:%S') UTC"
