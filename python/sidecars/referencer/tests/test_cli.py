# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0

"""Conformance of the sidecar CLI against the committed known-answer truth.

The tiny HDF5 fixture and its expected manifest live with the Rust
generator's known-answer suite (``crates/swath-referencer/tests/data``);
the expected byte ranges were derived straight from h5py's chunk index,
independently of BOTH generators. The Rust suite asserts the production
generator against that truth; this test asserts the VirtualiZarr sidecar
against the same truth — so PR CI transitively proves the two generators
equivalent on real HDF5 without NASA credentials (the real-VNP09GA
equivalence run is ``just test-referencer``).
"""

import json
from pathlib import Path
from typing import Any

from swath_referencer.cli import MANIFEST_VERSION, hdf5_arrays

REPO_ROOT = Path(__file__).resolve().parents[4]
DATA = REPO_ROOT / "crates" / "swath-referencer" / "tests" / "data"


def _comparable(arrays: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    """The equivalence-relevant projection of a manifest's arrays: grid,
    dtype, codecs, and per-chunk (offset, length) — ref paths and generator
    identity excluded, mirroring ``swath_core::manifest::compare``."""
    return {
        a["name"]: {
            "shape": a["shape"],
            "chunks": a["chunks"],
            "dtype": a["dtype"],
            "codecs": a["codecs"],
            "refs": {r["key"]: (r["offset"], r["length"]) for r in a["refs"]},
        }
        for a in arrays
    }


def test_sidecar_matches_the_h5py_truth_on_the_tiny_fixture() -> None:
    expected = json.loads((DATA / "tiny.expected.json").read_text())
    assert expected["manifest_version"] == MANIFEST_VERSION

    generated = hdf5_arrays(str(DATA / "tiny.h5"))
    assert _comparable(generated) == _comparable(expected["arrays"])

    # The truth's coverage is pinned: 6 arrays, 13 chunk refs (chunked +
    # ragged + partial + contiguous + unallocated + string scalar).
    expected_arrays, expected_refs = 6, 13
    assert len(expected["arrays"]) == expected_arrays
    assert sum(len(a["refs"]) for a in expected["arrays"]) == expected_refs
