# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "earthaccess==0.15.1",
# ]
# ///
"""Fetches the pinned VNP09GA conformance granule (ADR 0008) via earthaccess.

Usage: uv run tests/referencer/fetch_vnp09ga.py <dest-dir>

Prints the downloaded granule's path on stdout (progress goes to stderr).
Authentication is the standard NASA Earthdata netrc entry
(machine urs.earthdata.nasa.gov); `just test-referencer` checks for it and
skips politely when absent. The granule is the exact one prototype 0001's
bake-off ran on, so the structural truths asserted by
crates/swath-referencer/tests/vnp09ga_real.rs hold.
"""

import sys
from pathlib import Path

# The bake-off granule (prototype 0001 §7 / ADR 0008).
GRANULE = "VNP09GA.A2012019.h33v12.002.2023122182434"


def main() -> None:
    if len(sys.argv) != 2:
        print("usage: fetch_vnp09ga.py <dest-dir>", file=sys.stderr)
        raise SystemExit(2)
    dest = Path(sys.argv[1])
    dest.mkdir(parents=True, exist_ok=True)
    target = dest / f"{GRANULE}.h5"
    if target.exists():
        print(f"reusing cached {target}", file=sys.stderr)
        print(target)
        return

    import earthaccess

    earthaccess.login(strategy="netrc")
    results = earthaccess.search_data(
        short_name="VNP09GA", version="002", granule_name=f"{GRANULE}*"
    )
    if not results:
        print(f"granule {GRANULE} not found in CMR", file=sys.stderr)
        raise SystemExit(1)
    files = earthaccess.download(results[:1], str(dest))
    if not files:
        print("download failed", file=sys.stderr)
        raise SystemExit(1)
    path = Path(files[0])
    print(path if path.is_absolute() else path.resolve())


if __name__ == "__main__":
    main()
