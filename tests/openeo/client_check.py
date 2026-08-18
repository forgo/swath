# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "openeo==0.51.0",
# ]
# ///
"""openeo-python-client compatibility check (#195, ADR 0010/0014).

Usage: uv run tests/openeo/client_check.py [base-url]   (default
http://localhost:8080 — the compose stack; `just e2e` runs this after the
Rust harness).

The STANDARD openEO Python client — no bespoke SDK, the recorded
anti-goal — drives Swath's bounded profile end to end exactly as the
notebook recipe (docs/RECIPES.md) shows:

1. `openeo.connect` (version discovery + capabilities),
2. collections and processes listings,
3. a process graph built with the client's own datacube API
   (`load_collection` → band math NDVI → `save_result`),
4. `download()` → `POST /result` → PNG bytes.

Every incompatibility found here is either fixed (cheap) or documented as
a profile narrowing in RECIPES.md; this script failing is the drift alarm.
"""

import sys

import openeo
from openeo.rest.datacube import THIS


def main() -> int:
    base = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8080"

    # 1. Connect: hits /.well-known/openeo, then the capabilities root.
    connection = openeo.connect(base)
    capabilities = connection.capabilities()
    api_version = capabilities.api_version()
    assert api_version.startswith("1."), f"unexpected api_version {api_version}"

    # 2. Discovery: the fixture dataset and the bounded process set.
    collection_ids = {c["id"] for c in connection.list_collections()}
    assert "hls-s30" in collection_ids, f"hls-s30 not listed: {collection_ids}"
    process_ids = {p["id"] for p in connection.list_processes()}
    for required in (
        "load_collection",
        "ndvi",
        "reduce_dimension",
        "array_element",
        "linear_scale_range",
        "save_result",
    ):
        assert required in process_ids, f"{required} missing from /processes"
    # The client can also describe the collection (STAC-based document).
    described = connection.describe_collection("hls-s30")
    assert described["id"] == "hls-s30"

    # 3. Build NDVI with the client's own datacube API. Band names are the
    # collection's DECLARED values (the client validates against
    # cube:dimensions), and ndvi takes explicit nir/red targets — the
    # profile does not persist a common-name alias vocabulary (RECIPES.md
    # documents this narrowing; same form the Rust e2e harness posts).
    cube = connection.load_collection("hls-s30", bands=["b04", "b8a"])
    cube = cube.ndvi(nir="b8a", red="b04")
    # PROFILE FINDING (feeds ADR 0010's reopen conditions): the client's
    # `.linear_scale_range()` sugar wraps the process in `apply`, which the
    # bounded subset does not include — the stock client's explicit
    # `.process()` form emits the profile's top-level node instead.
    cube = cube.process(
        "linear_scale_range",
        x=THIS,
        inputMin=-1,
        inputMax=1,
        outputMin=0,
        outputMax=255,
    )
    cube = cube.save_result(format="png")

    # 4. The notebook loop's final cell: download() POSTs /result and
    # returns the preview PNG bytes (ADR 0014's bounded synchronous form).
    # The format was fixed by save_result; download() takes no override.
    png = cube.download()
    assert png[:4] == b"\x89PNG", f"not a PNG: {png[:16]!r}"

    print(
        f"openeo-python-client PASS: openeo {openeo.client_version()} against "
        f"{base} — connect/discover/build/download, {len(png)} PNG bytes"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
