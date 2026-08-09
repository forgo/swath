# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "h5py==3.16.0",
#   "numpy==2.5.1",
# ]
# ///
"""Generates the tiny committed HDF5 fixture and its known-answer truth.

Writes three committed files:

* ``tiny.h5`` (a few KB — small enough to commit, like the HLS COG
  fixtures);
* ``tiny.expected.json`` (manifest schema v1) — derived **independently of
  the Rust generator**, straight from h5py's chunk index
  (``dataset.id.chunk_iter``) and this script's own StructMetadata
  constants: the truth the Rust known-answer test
  (``tests/known_answer.rs``) and the Python sidecar conformance test
  assert against, giving PR CI real HDF5 coverage without NASA credentials;
* ``../../../adapters/swath-source-virtual/tests/data/window_truth.json``
  — h5py-derived pixel-window ground truth (values + SHA-256 of the raw
  little-endian bytes, the same regime as the COG adapter's
  ``window_truth.py``) for the virtual-reference RasterSource (#39).

The fixture exercises the storage layouts the generator must map: a fully
written chunked+deflate+shuffle array, a ragged chunk grid (chunk shape not
dividing the array shape), a partially written chunked array (unallocated
chunks absent from the index), a contiguous array, a never-written array
(no storage at all), a fixed-length string scalar, and a nested group.

Since #39 it is additionally a **real (minimal) HDF-EOS5 file**: it carries
an ``HDFEOS INFORMATION/StructMetadata.0`` grid block (sinusoidal, the
VNP09GA projection family) and two georeferenced data fields under
``HDFEOS/GRIDS/TinyGrid/Data Fields/`` — ``nir`` (shuffle+deflate, fully
written, with fill-value pixels) and ``red`` (deflate, partially written,
so a whole chunk is unallocated). Both use a ragged 3x4 chunk grid over
the 8x7 field. That gives PR CI offline coverage of georef parsing AND of
every virtual-source read path: chunk intersection, decompression,
unshuffle, ragged edge chunks, nodata passthrough, and missing-chunk fill.

Both outputs are committed together: chunk offsets/lengths depend on the
libhdf5/zlib build, so regeneration may legitimately shift bytes — commit
the regenerated set atomically, never one side.

Run from this directory:  uv run make_tiny_fixture.py
"""

import hashlib
import json
from pathlib import Path

import h5py
import numpy as np

HERE = Path(__file__).parent
FIXTURE = HERE / "tiny.h5"
EXPECTED = HERE / "tiny.expected.json"
WINDOW_TRUTH = (
    HERE / "../../../adapters/swath-source-virtual/tests/data/window_truth.json"
).resolve()

# --- the TinyGrid HDF-EOS grid (VNP09GA's sinusoidal family, tiny dims) ---
GRID = "TinyGrid"
XDIM, YDIM = 7, 8
CHUNKS = (3, 4)  # ragged on both axes: 8/3 -> 3 rows, 7/4 -> 2 cols
SPHERE_R = 6371007.181
CELL = 926.625433055833  # the VNP09GA 1-km cell size
ULX, ULY = 16679257.795, -3335851.559  # the h33v12 upper-left corner
LRX, LRY = ULX + XDIM * CELL, ULY - YDIM * CELL
FILL = np.int16(-28672)  # the VNP09GA reflectance fill value
# Exactly the proj string swath-referencer's eos.rs emits for this metadata.
SINU = f"+proj=sinu +lon_0=0 +x_0=0 +y_0=0 +R={SPHERE_R} +units=m +no_defs"

STRUCT_METADATA = f"""GROUP=SwathStructure
END_GROUP=SwathStructure
GROUP=GridStructure
\tGROUP=GRID_1
\t\tGridName="{GRID}"
\t\tXDim={XDIM}
\t\tYDim={YDIM}
\t\tUpperLeftPointMtrs=({ULX:.6f},{ULY:.6f})
\t\tLowerRightMtrs=({LRX:.6f},{LRY:.6f})
\t\tProjection=HE5_GCTP_SNSOID
\t\tProjParams=({SPHERE_R},0,0,0,0,0,0,0,0,0,0,0,0)
\t\tSphereCode=-1
\t\tGridOrigin=HE5_HDFE_GD_UL
\tEND_GROUP=GRID_1
END_GROUP=GridStructure
END
"""


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

        # --- the HDF-EOS half (#39): a real minimal StructMetadata.0 and
        # two georeferenced sinusoidal data fields ---
        f.create_dataset(
            "HDFEOS INFORMATION/StructMetadata.0",
            data=np.bytes_(STRUCT_METADATA.encode()),
        )
        fields = f"HDFEOS/GRIDS/{GRID}/Data Fields"
        # nir: fully written, shuffle+deflate(4), a block of fill pixels.
        nir_data = rng.integers(0, 10000, size=(YDIM, XDIM), dtype=np.int16)
        nir_data[0:2, 0:3] = FILL
        nir = f.create_dataset(
            f"{fields}/nir",
            data=nir_data,
            chunks=CHUNKS,
            compression="gzip",
            compression_opts=4,
            shuffle=True,
            fillvalue=FILL,
        )
        nir.attrs.create("_FillValue", FILL)
        # red: deflate(2) only, partially written — chunk row 2 (grid rows
        # 6..8) never touched, so its two chunks stay unallocated and reads
        # over them must come back as fill.
        red = f.create_dataset(
            f"{fields}/red",
            shape=(YDIM, XDIM),
            dtype=np.int16,
            chunks=CHUNKS,
            compression="gzip",
            compression_opts=2,
            fillvalue=FILL,
        )
        red.attrs.create("_FillValue", FILL)
        red[0:6, :] = rng.integers(0, 10000, size=(6, XDIM), dtype=np.int16)


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


def georef_entry(name: str) -> dict | None:
    """The georef truth for TinyGrid data fields, derived from this
    script's OWN StructMetadata constants (independent of the Rust
    parser): corners re-parsed from the 6-decimal text exactly as a
    reader sees them, cell size = (corner span) / dims."""
    if not name.startswith(f"HDFEOS/GRIDS/{GRID}/"):
        return None
    ulx, uly = round(ULX, 6), round(ULY, 6)
    lrx, lry = round(LRX, 6), round(LRY, 6)
    return {
        "crs": {"proj4": SINU},
        "transform": {
            "origin_x": ulx,
            "pixel_width": (lrx - ulx) / XDIM,
            "row_rotation": 0.0,
            "origin_y": uly,
            "col_rotation": 0.0,
            "pixel_height": (lry - uly) / YDIM,
        },
        "nodata": float(FILL),
        "band": name.rsplit("/", 1)[-1],
    }


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
    entry = {
        "name": name,
        "shape": shape,
        "chunks": chunks,
        "dtype": str(ds.dtype),
        "codecs": codec_strings(ds),
        "refs": refs,
    }
    georef = georef_entry(name)
    if georef is not None and shape == [YDIM, XDIM]:
        entry["georef"] = georef
    return entry


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


# (name, dataset, col_off, row_off, width, height) — requested, pre-clip.
# interior crosses both ragged chunk seams; oob_clipped extends past the
# grid; red/missing_chunks covers the unallocated chunk row (fill).
WINDOWS: list[tuple[str, str, int, int, int, int]] = [
    ("full", "nir", 0, 0, XDIM, YDIM),
    ("interior", "nir", 2, 1, 4, 5),
    ("one_pixel", "nir", 3, 4, 1, 1),
    ("oob_clipped", "nir", 5, 6, 4, 4),
    ("nodata_block", "nir", 0, 0, 4, 3),
    ("full", "red", 0, 0, XDIM, YDIM),
    ("missing_chunks", "red", 0, 5, XDIM, 3),
]


def build_window_truth() -> None:
    cases = []
    fields = f"HDFEOS/GRIDS/{GRID}/Data Fields"
    with h5py.File(FIXTURE, "r") as f:
        for wname, band, col, row, w, h in WINDOWS:
            ds = f[f"{fields}/{band}"]
            height, width = ds.shape
            c0, r0 = max(col, 0), max(row, 0)
            c1, r1 = min(col + w, width), min(row + h, height)
            cw, ch = max(c1 - c0, 0), max(r1 - r0, 0)
            data = ds[r0 : r0 + ch, c0 : c0 + cw]
            assert data.shape == (ch, cw)
            le = np.ascontiguousarray(data).astype(data.dtype.newbyteorder("<"))
            flat = data.ravel()
            mask = flat == FILL
            cases.append(
                {
                    "array": f"{fields}/{band}",
                    "window_name": wname,
                    "requested": {
                        "col_off": col,
                        "row_off": row,
                        "width": w,
                        "height": h,
                    },
                    "clipped": {
                        "col_off": c0,
                        "row_off": r0,
                        "width": cw,
                        "height": ch,
                    },
                    "dtype": str(ds.dtype),
                    "nodata": float(FILL),
                    "nodata_count": int(mask.sum()),
                    "valid_sum": int(flat[~mask].astype(np.int64).sum()),
                    "first8": [int(v) for v in flat[:8]],
                    "last8": [int(v) for v in flat[-8:]],
                    "sha256_le": hashlib.sha256(le.tobytes()).hexdigest(),
                }
            )
    # The red/missing_chunks case must actually exercise unallocated-chunk
    # fill: its last row lives entirely in the never-written chunk row.
    missing = next(
        c for c in cases if c["window_name"] == "missing_chunks"
    )
    assert missing["nodata_count"] >= 2 * XDIM, "missing-chunk rows must be fill"
    WINDOW_TRUTH.parent.mkdir(parents=True, exist_ok=True)
    WINDOW_TRUTH.write_text(json.dumps({"cases": cases}, indent=2) + "\n")


def main() -> None:
    build_fixture()
    build_expected()
    build_window_truth()
    total = sum(
        len(a["refs"]) for a in json.loads(EXPECTED.read_text())["arrays"]
    )
    print(f"wrote {FIXTURE.name} ({FIXTURE.stat().st_size} bytes), "  # noqa: T201
          f"{EXPECTED.name} ({total} chunk refs), "
          f"{WINDOW_TRUTH.name} ({len(WINDOWS)} windows)")


if __name__ == "__main__":
    main()
