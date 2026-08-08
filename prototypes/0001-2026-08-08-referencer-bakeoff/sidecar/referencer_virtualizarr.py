#!/usr/bin/env python3
"""VirtualiZarr sidecar for prototype 0001.

Reads a legacy granule (NetCDF4/HDF5, GRIB2, ...) with VirtualiZarr and prints a VirtualManifest
JSON (the prototype schema shared with the Rust harness) to stdout. Errors go to stderr with a
non-zero exit code so the Rust `VirtualizarrSidecar` adapter can surface them.

NOTE: VirtualiZarr's API surface shifts between releases; the manifest-extraction below tries a couple
of access patterns and is the most likely spot to need a small tweak when you run it. That's expected —
this is a prototype whose job is to produce a manifest equivalent to the Rust generator's.
"""

import json
import sys


def eprint(*a):
    print(*a, file=sys.stderr)


def extract_refs(manifest):
    """Return [{key, path, offset, length}] from a VirtualiZarr ChunkManifest."""
    # Pattern A: manifest.dict() -> {"0.0": {"path":..., "offset":..., "length":...}, ...}
    d = None
    if hasattr(manifest, "dict"):
        try:
            d = manifest.dict()
        except Exception:
            d = None
    if isinstance(d, dict):
        entries = d.get("entries", d)  # some versions nest under "entries"
        refs = []
        for key, e in entries.items():
            if not isinstance(e, dict):
                continue
            refs.append({
                "key": str(key),
                "path": str(e.get("path", "")),
                "offset": int(e.get("offset", 0)),
                "length": int(e.get("length", 0)),
            })
        return refs

    # Pattern B: parallel numpy arrays _paths / _offsets / _lengths indexed by chunk grid.
    if all(hasattr(manifest, a) for a in ("_paths", "_offsets", "_lengths")):
        import numpy as np  # noqa
        paths = manifest._paths
        offsets = manifest._offsets
        lengths = manifest._lengths
        refs = []
        it = np.ndindex(paths.shape)
        for idx in it:
            key = ".".join(str(i) for i in idx)
            refs.append({
                "key": key,
                "path": str(paths[idx]),
                "offset": int(offsets[idx]),
                "length": int(lengths[idx]),
            })
        return refs

    raise RuntimeError(
        "could not extract chunk refs from this VirtualiZarr manifest object; "
        "inspect its API and adjust extract_refs()"
    )


def get_chunks(marr, var):
    for attr in ("chunks",):
        c = getattr(marr, attr, None)
        if c:
            return [int(x) for x in c]
    za = getattr(marr, "zarray", None)
    if za is not None and getattr(za, "chunks", None):
        return [int(x) for x in za.chunks]
    return [int(x) for x in getattr(var, "shape", [])]


def get_codecs(marr):
    za = getattr(marr, "zarray", None)
    codecs = []
    if za is not None:
        for f in (getattr(za, "filters", None) or []):
            codecs.append(str(f.get("id", f)) if isinstance(f, dict) else str(f))
        comp = getattr(za, "compressor", None)
        if comp is not None:
            codecs.append(str(comp.get("id", comp)) if isinstance(comp, dict) else str(comp))
    return codecs


def main():
    if len(sys.argv) < 2:
        eprint("usage: referencer_virtualizarr.py <granule>")
        sys.exit(2)
    path = sys.argv[1]

    try:
        from virtualizarr import open_virtual_dataset
    except Exception as e:  # noqa
        eprint(f"virtualizarr not available: {e} (see sidecar/requirements.txt)")
        sys.exit(3)

    try:
        vds = open_virtual_dataset(path)
    except Exception as e:  # noqa
        eprint(f"open_virtual_dataset('{path}') failed: {e}")
        sys.exit(4)

    arrays = []
    for name, var in vds.variables.items():
        marr = getattr(var, "data", None)
        manifest = getattr(marr, "manifest", None)
        if manifest is None:
            continue  # loaded coordinate / not a virtual ManifestArray
        try:
            refs = extract_refs(manifest)
        except Exception as e:  # noqa
            eprint(f"warning: skipping '{name}': {e}")
            continue
        arrays.append({
            "name": str(name),
            "shape": [int(x) for x in getattr(marr, "shape", var.shape)],
            "chunks": get_chunks(marr, var),
            "dtype": str(getattr(marr, "dtype", var.dtype)),
            "codecs": get_codecs(marr),
            "refs": refs,
        })

    json.dump({"generator": "virtualizarr", "source": path, "arrays": arrays}, sys.stdout)


if __name__ == "__main__":
    main()
