# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
#
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "icechunk==2.1.2",
#     "xarray>=2025.1",
#     "zarr>=3.1",
#     "h5py>=3.12",
#     "numpy>=2",
# ]
# ///
"""The icechunk-python/xarray half of the #191 conformance gate (ADR 0017).

Usage: uv run tests/referencer/icechunk_check.py <repo-dir> <granule.h5>

Opens the Icechunk repository the Rust committer wrote — through
icechunk-python's own virtual-chunk authorization, zarr-python's codec
decode, and xarray's dataset view (the exact stack an external consumer
uses; none of Swath's Rust in the loop) — and asserts every committed
array's pixel values equal the HDF5 source's, exactly.

Exit code 0 on full equality; non-zero with a per-array report otherwise.
"""

import sys
from pathlib import Path

import h5py
import icechunk
import numpy as np
import xarray as xr
import zarr


def walk_arrays(group: zarr.Group, prefix: str = "") -> list[str]:
    """Every array path under `group`, depth-first."""
    found: list[str] = []
    for name, member in sorted(group.members()):
        path = f"{prefix}{name}"
        if isinstance(member, zarr.Group):
            found.extend(walk_arrays(member, f"{path}/"))
        else:
            found.append(path)
    return found


def main() -> int:
    if len(sys.argv) != 3:  # noqa: PLR2004 - argv arity, not magic
        print(__doc__, file=sys.stderr)
        return 2

    repo_dir = Path(sys.argv[1]).resolve()
    granule = Path(sys.argv[2]).resolve()
    container_prefix = granule.parent.as_uri() + "/"

    storage = icechunk.local_filesystem_storage(str(repo_dir))
    repo = icechunk.Repository.open(
        storage,
        authorize_virtual_chunk_access={
            container_prefix: icechunk.credentials.LocalFileSystemAccess,
        },
    )
    session = repo.readonly_session(branch="main")
    root = zarr.open_group(session.store, mode="r")
    arrays = walk_arrays(root)
    if not arrays:
        print("FAIL: the Icechunk store holds no arrays", file=sys.stderr)
        return 1

    failures: list[str] = []
    compared = 0
    with h5py.File(granule, "r") as h5:
        for path in arrays:
            zarr_values = root[path][...]
            h5_values = h5[path][...]
            if np.array_equal(zarr_values, h5_values):
                compared += 1
            else:
                failures.append(path)

        # xarray opens every leaf group holding arrays — the consumer view.
        groups = sorted({path.rsplit("/", 1)[0] for path in arrays if "/" in path})
        for group in groups:
            ds = xr.open_zarr(
                session.store,
                group=group,
                consolidated=False,
                mask_and_scale=False,
            )
            if not ds.data_vars:
                failures.append(f"{group} (xarray sees no variables)")

    if failures:
        print(f"FAIL: {len(failures)} mismatch(es): {failures}", file=sys.stderr)
        return 1
    print(
        f"icechunk conformance PASS: {compared} array(s) byte-equal to the HDF5 "
        f"source; xarray opened {len(groups)} group(s)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
