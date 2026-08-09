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


# eccodes packingType -> the codec vocabulary shared with the Rust generator, which derives the
# same strings independently from the section-5 template number. Exact agreement is contractual.
GRIB_PACKING_CODECS = {
    "grid_simple": "grib2:simple",
    "grid_complex": "grib2:complex",
    "grid_complex_spatial_differencing": "grib2:complex-spatial-diff",
    "grid_ieee": "grib2:ieee-float",
    "grid_jpeg": "grib2:jpeg2000",
    "grid_png": "grib2:png",
    "grid_ccsds": "grib2:aec",
}


def grib2_arrays(path):
    """VirtualManifest arrays for a GRIB2 file via kerchunk's scan_grib (the reference model).

    scan_grib emits one zarr group per GRIB message; the data variable's single chunk is the
    whole-message byte range. We flatten: one manifest array per message, named by cfgrib's
    variable name; repeated variables get _1, _2, ... suffixes. Coordinate arrays (lat/lon/time,
    inline base64 refs) are not byte ranges into the granule, so they are skipped.
    """
    import eccodes
    import numpy as np
    from kerchunk.grib2 import scan_grib

    with open(path, "rb") as f:
        raw = f.read()

    arrays = []
    seen = {}
    for group in scan_grib(path):
        refs = group["refs"]
        for key, val in refs.items():
            if not (isinstance(val, list) and len(val) == 3):
                continue  # inline/base64 or metadata entry, not a byte range
            name, chunk_key = key.split("/", 1)
            zarray = json.loads(refs[f"{name}/.zarray"])
            _, offset, length = val

            # Codec: derived from the message bytes with eccodes (independent of the Rust path).
            handle = eccodes.codes_new_from_message(raw[offset : offset + length])
            try:
                packing = eccodes.codes_get(handle, "packingType")
            finally:
                eccodes.codes_release(handle)
            codec = GRIB_PACKING_CODECS.get(packing, f"grib2:{packing}")

            n = seen.get(name, 0)
            seen[name] = n + 1
            arrays.append({
                "name": name if n == 0 else f"{name}_{n}",
                "shape": [int(x) for x in zarray["shape"]],
                "chunks": [int(x) for x in zarray["chunks"]],
                "dtype": str(np.dtype(zarray["dtype"])),
                "codecs": [codec],
                "refs": [{
                    "key": chunk_key,
                    "path": path,
                    "offset": int(offset),
                    "length": int(length),
                }],
            })
    return arrays


def main():
    if len(sys.argv) < 2:
        eprint("usage: referencer_virtualizarr.py <granule>")
        sys.exit(2)
    path = sys.argv[1]

    if path.lower().endswith((".grib2", ".grb2", ".grib")):
        try:
            arrays = grib2_arrays(path)
        except Exception as e:  # noqa
            eprint(f"grib2 scan of '{path}' failed: {e}")
            sys.exit(4)
        json.dump({"generator": "virtualizarr", "source": path, "arrays": arrays}, sys.stdout)
        return

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
