#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

# Image smoke test shared by publish-image.yml (main-branch `latest`) and
# release-image.yml (versioned `v*` tags) — issue #116 extracted it from
# publish-image.yml unchanged. Runs the README install one-liner against a
# locally built, NOT-yet-pushed image (`-d --name` added only so CI can
# tail logs and tear down) and asserts the three things a fresh user sees:
# liveness, a real rendered tile, and the embedded UI (#103).
#
# Usage: smoke-image.sh <image-ref>
set -euo pipefail

image="${1:?usage: smoke-image.sh <image-ref>}"

docker run -d --name swath-smoke -p 8080:8080 "$image" serve --fixtures
trap 'docker logs swath-smoke; docker rm -f swath-smoke' EXIT

# 1. /healthz answers 200 within 30s.
ok=""
for _ in $(seq 1 60); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' http://localhost:8080/healthz)" = "200" ]; then
    ok=1; break
  fi
  sleep 0.5
done
[ -n "$ok" ] || { echo "::error::/healthz never answered 200"; exit 1; }
echo "healthz: 200"

# 2. A truecolor tile over the fixture footprint: 200, image/png,
#    non-empty body that really is a PNG.
code=$(curl -s -D headers.txt -o tile.png -w '%{http_code}' \
  http://localhost:8080/tilesets/truecolor/tiles/12/1561/848)
[ "$code" = "200" ] || { echo "::error::tile returned $code"; exit 1; }
grep -i '^content-type: image/png' headers.txt \
  || { echo "::error::tile content-type is not image/png"; cat headers.txt; exit 1; }
[ -s tile.png ] || { echo "::error::tile body is empty"; exit 1; }
[ "$(head -c 8 tile.png | xxd -p)" = "89504e470d0a1a0a" ] \
  || { echo "::error::tile body is not a PNG"; exit 1; }
echo "tile: 200 image/png ($(wc -c < tile.png) bytes)"

# 3. The embedded UI (#103): a browser-shaped GET / serves the
#    viewer's index.html.
code=$(curl -s -H 'Accept: text/html' -D ui-headers.txt -o ui.html \
  -w '%{http_code}' http://localhost:8080/)
[ "$code" = "200" ] || { echo "::error::UI returned $code"; exit 1; }
grep -i '^content-type: text/html' ui-headers.txt \
  || { echo "::error::UI content-type is not text/html"; cat ui-headers.txt; exit 1; }
grep -q 'swath-map' ui.html \
  || { echo "::error::UI index.html does not mount <swath-map>"; exit 1; }
echo "ui: 200 text/html ($(wc -c < ui.html) bytes)"
