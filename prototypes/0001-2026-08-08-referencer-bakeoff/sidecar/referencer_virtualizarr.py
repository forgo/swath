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
import os
import sys


def eprint(*a):
    print(*a, file=sys.stderr)


def hdf5_codec_string(codec):
    """Zarr codec (as VirtualiZarr's HDF parser reports it) -> the codec vocabulary shared with
    the Rust generator, which derives the same strings independently from the HDF5 filter
    pipeline. Exact agreement is contractual. Returns None for the zarr `bytes` serializer,
    which is not an HDF5 filter."""
    name = str(getattr(codec, "codec_name", None) or type(codec).__name__).lower()
    config = dict(getattr(codec, "codec_config", None) or {})
    if "bytes" in name:
        return None
    if "zlib" in name or "gzip" in name:
        return f"zlib:{config.get('level', '')}"
    if "shuffle" in name:
        return "shuffle"
    if "fletcher32" in name:
        return "fletcher32"
    if "szip" in name:
        return "szip"
    return f"hdf5:{name}"


def hdf5_arrays(path):
    """VirtualManifest arrays for an HDF5/NetCDF4 file via VirtualiZarr's HDF parser.

    We walk the parser's ManifestStore group tree directly instead of `open_virtual_dataset`
    because real HDF-EOS granules (VNP09GA) both (a) spread datasets across nested groups and
    (b) reuse phony dimension names with conflicting sizes inside one group, which the xarray
    merge in `to_virtual_dataset` refuses. Array names are the HDF5 path without the leading
    slash, groups joined by "/" — the same naming the Rust generator derives from H5Iget_name.
    """
    import virtualizarr.parsers.hdf.hdf as vzhdf
    from obspec_utils.registry import ObjectStoreRegistry
    from obstore.store import LocalStore

    # VNP09GA carries string _FillValue attrs (b"N/A") on integer QF datasets; VirtualiZarr's
    # CF fill-value encoding rejects those. Fill values are not part of the manifest contract,
    # so degrade to "no fill" instead of failing the whole granule.
    orig_encode = vzhdf.encode_cf_fill_value

    def lenient_encode(fill_value, dtype):
        try:
            return orig_encode(fill_value, dtype)
        except Exception:
            return None

    vzhdf.encode_cf_fill_value = lenient_encode

    registry = ObjectStoreRegistry({"file://": LocalStore()})
    store = vzhdf.HDFParser()(url="file://" + os.path.abspath(path), registry=registry)

    arrays = []

    def walk(group, prefix):
        for name, arr in group.arrays.items():
            md = arr.metadata
            refs = [
                {
                    # ChunkManifest keys are dotted chunk-grid positions ("" for scalars);
                    # the path is rewritten from the registry URL to the argv path so both
                    # generators name the granule identically.
                    "key": str(key),
                    "path": path,
                    "offset": int(entry["offset"]),
                    "length": int(entry["length"]),
                }
                for key, entry in sorted(arr.manifest.dict().items())
            ]
            codecs = [c for c in map(hdf5_codec_string, md.codecs) if c is not None]
            arrays.append({
                "name": f"{prefix}{name}",
                "shape": [int(x) for x in md.shape],
                "chunks": [int(x) for x in md.chunks],
                "dtype": str(md.data_type.to_native_dtype()),
                "codecs": codecs,
                "refs": refs,
            })
        for name, sub in group.groups.items():
            walk(sub, f"{prefix}{name}/")

    # ManifestStore keeps its ManifestGroup tree in `_group`; there is no public accessor for
    # the raw tree yet (only the xarray views), so this reaches into a private attribute —
    # prototype scope, revisit on VirtualiZarr upgrades.
    walk(store._group, "")
    return arrays


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
        import virtualizarr  # noqa: F401
    except Exception as e:  # noqa
        eprint(f"virtualizarr not available: {e} (see sidecar/requirements.txt)")
        sys.exit(3)

    try:
        arrays = hdf5_arrays(path)
    except Exception as e:  # noqa
        eprint(f"hdf5 scan of '{path}' failed: {e}")
        sys.exit(4)

    json.dump({"generator": "virtualizarr", "source": path, "arrays": arrays}, sys.stdout)


if __name__ == "__main__":
    main()
