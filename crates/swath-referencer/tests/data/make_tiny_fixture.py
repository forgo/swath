# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "h5py==3.16.0",
#   "numpy==2.5.1",
# ]
# ///
"""Generates the tiny committed HDF5 fixture and its known-answer manifest.

Writes ``tiny.h5`` (a few KB — small enough to commit, like the HLS COG
fixtures) and ``tiny.expected.json`` (manifest schema v1). The expected
manifest is derived **independently of the Rust generator**, straight from
h5py's chunk index (``dataset.id.chunk_iter``) — it is the truth the Rust
known-answer test (``tests/known_answer.rs``) and the Python sidecar
conformance test assert against, giving PR CI real HDF5 coverage without
NASA credentials.

The fixture exercises the storage layouts the generator must map: a fully
written chunked+deflate+shuffle array, a ragged chunk grid (chunk shape not
dividing the array shape), a partially written chunked array (unallocated
chunks absent from the index), a contiguous array, a never-written array
(no storage at all), a fixed-length string scalar, and a nested group.

Both outputs are committed together: chunk offsets/lengths depend on the
libhdf5/zlib build, so regeneration may legitimately shift bytes — commit
the regenerated pair atomically, never one side.

Run from this directory:  uv run make_tiny_fixture.py
"""

import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).parent
FIXTURE = HERE / "tiny.h5"
EXPECTED = HERE / "tiny.expected.json"


def build_fixture() -> None:
    rng = np.random.default_rng(seed=40)  # issue number; fully deterministic
    with h5py.File(FIXTURE, "w", libver="earliest") as f:
        # Fully written, chunked, deflate(4)+shuffle, ragged grid: 8x6 in
        # 4x3 chunks -> 2x2 grid, exact fit; and 5x7 in 4x3 -> 2x3 ragged.
        f.create_dataset(
            "grid/reflectance",
            data=rng.integers(-2000, 12000, size=(8, 6), dtype=np.int16),
            chunks=(4, 3),
            compression="gzip",
            compression_opts=4,
            shuffle=True,
        )
        ragged = f.create_dataset(
            "grid/ragged",
            shape=(5, 7),
            dtype=np.uint8,
            chunks=(4, 3),
            compression="gzip",
            compression_opts=1,
        )
        ragged[...] = rng.integers(0, 255, size=(5, 7), dtype=np.uint8)
        # Partially written: only the first of two 4x6 chunks is allocated.
        partial = f.create_dataset(
            "grid/partial", shape=(8, 6), dtype=np.float32, chunks=(4, 6)
        )
        partial[0:4, :] = np.float32(1.5)
        # Contiguous, and never-written (no storage allocated).
        f.create_dataset(
            "aux/contiguous", data=rng.random(size=(4, 4)).astype(np.float64)
        )
        f.create_dataset("aux/unallocated", shape=(3, 3), dtype=np.int32)
        # Fixed-length string scalar (the StructMetadata.0 shape).
        f.create_dataset("meta", data=np.bytes_(b"tiny swath fixture"))


def codec_strings(ds: h5py.Dataset) -> list[str]:
    codecs: list[str] = []
    # h5py exposes the pipeline in filter order via the DCPL.
    plist = ds.id.get_create_plist()
    for i in range(plist.get_nfilters()):
        code, _flags, values, _name = plist.get_filter(i)
        if code == h5py.h5z.FILTER_SHUFFLE:
            codecs.append("shuffle")
        elif code == h5py.h5z.FILTER_DEFLATE:
            codecs.append(f"zlib:{values[0]}")
        elif code == h5py.h5z.FILTER_FLETCHER32:
            codecs.append("fletcher32")
        else:  # pragma: no cover - fixture uses only the above
            codecs.append(f"hdf5:filter{code}")
    return codecs


def array_entry(name: str, ds: h5py.Dataset, source: str) -> dict:
    shape = [int(s) for s in ds.shape]
    if ds.chunks is None:
        chunks = shape
        offset = ds.id.get_offset()
        refs = (
            []
            if offset is None
            else [
                {
                    "key": ".".join(["0"] * len(shape)),
                    "path": source,
                    "offset": int(offset),
                    "length": int(ds.id.get_storage_size()),
                }
            ]
        )
    else:
        chunks = [int(c) for c in ds.chunks]
        refs = []

        def visit(info) -> None:
            key = ".".join(
                str(elem // dim) for elem, dim in zip(info.chunk_offset, chunks)
            )
            refs.append(
                {
                    "key": key,
                    "path": source,
                    "offset": int(info.byte_offset),
                    "length": int(info.size),
                }
            )

        ds.id.chunk_iter(visit)
    return {
        "name": name,
        "shape": shape,
        "chunks": chunks,
        "dtype": str(ds.dtype),
        "codecs": codec_strings(ds),
        "refs": refs,
    }


def build_expected() -> None:
    source = FIXTURE.name
    arrays: list[dict] = []
    with h5py.File(FIXTURE, "r") as f:

        def walk(group: h5py.Group, prefix: str) -> None:
            # Datasets first, then subgroups — the traversal both manifest
            # generators use (prototype 0001 §7).
            members = [(name, group[name]) for name in group]
            for name, obj in members:
                if isinstance(obj, h5py.Dataset):
                    arrays.append(array_entry(f"{prefix}{name}", obj, source))
            for name, obj in members:
                if isinstance(obj, h5py.Group):
                    walk(obj, f"{prefix}{name}/")

        walk(f, "")
    manifest = {
        "manifest_version": 1,
        "generator": "h5py-truth",
        "source": source,
        "arrays": arrays,
    }
    EXPECTED.write_text(json.dumps(manifest, indent=2) + "\n")


def main() -> None:
    build_fixture()
    build_expected()
    total = sum(
        len(a["refs"]) for a in json.loads(EXPECTED.read_text())["arrays"]
    )
    print(f"wrote {FIXTURE.name} ({FIXTURE.stat().st_size} bytes), "  # noqa: T201
          f"{EXPECTED.name} ({total} chunk refs)")


if __name__ == "__main__":
    main()
