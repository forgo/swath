# SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
# SPDX-License-Identifier: Apache-2.0
# pyright: basic
# ^ VirtualiZarr's manifest internals (ManifestStore/ManifestGroup, reached
#   below because no public API exposes the raw manifest tree) ship no type
#   information; strict mode would demand stubs for a third-party surface
#   this module deliberately treats as the independent reference.

"""The conformance-reference manifest generator (ADR 0006).

``swath-referencer <granule.h5>`` prints a schema-v1 ``VirtualManifest``
(see ``swath_core::manifest``) derived with **VirtualiZarr's HDF parser** —
an implementation independent of the production Rust generator. The gated
conformance harness (``just test-referencer``) runs both on a real VNP09GA
granule and asserts byte-range equivalence; a PR-CI test does the same
against the tiny committed fixture. This CLI is the promoted form of
prototype 0001's sidecar script.

Scope: HDF5/NetCDF4 only. The prototype's GRIB2 reference path rides on
kerchunk + cfgrib + eccodes (a native library pin); GRIB conformance stays
with the prototype's pinned environment until a GRIB dataset is on the
serving path.

Georeferencing is deliberately absent from this generator: georef truth is
asserted by the Rust side's known-answer tests, and the equivalence check
excludes it by contract.
"""

import json
import sys
from pathlib import Path
from typing import Any, cast

MANIFEST_VERSION = 1


def _codec_string(codec: object) -> str | None:
    """Zarr codec (as VirtualiZarr's HDF parser reports it) -> the codec
    vocabulary shared with the Rust generator, which derives the same
    strings independently from the HDF5 filter pipeline. Exact agreement is
    contractual. Returns None for the zarr ``bytes`` serializer, which is
    not an HDF5 filter."""
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


def hdf5_arrays(path: str) -> list[dict[str, Any]]:
    """Manifest arrays for an HDF5/NetCDF4 file via VirtualiZarr's HDF parser.

    Walks the parser's ManifestStore group tree directly instead of
    ``open_virtual_dataset``: real HDF-EOS granules (VNP09GA) spread
    datasets across nested groups and reuse phony dimension names with
    conflicting sizes inside one group, which the xarray merge refuses.
    Array names are the HDF5 path without the leading slash, matching the
    Rust generator's naming.
    """
    # Deferred imports (PLC0415 waived): virtualizarr's import graph is
    # heavy (xarray/zarr/obstore); the usage-line error path stays readable
    # when the environment lacks it.
    import virtualizarr.parsers.hdf.hdf as vzhdf  # noqa: PLC0415
    from obspec_utils.registry import ObjectStoreRegistry  # noqa: PLC0415
    from obstore.store import LocalStore  # noqa: PLC0415

    # VNP09GA carries string _FillValue attrs (b"N/A") on integer QF
    # datasets; VirtualiZarr's CF fill-value encoding rejects those. Fill
    # values are outside the manifest contract, so degrade to "no fill"
    # instead of failing the granule (prototype 0001 §7).
    # (cast to Any: the hook is a module-internal function with no public
    # typing surface.)
    hdf_module = cast("Any", vzhdf)
    orig_encode = hdf_module.encode_cf_fill_value

    def lenient_encode(fill_value: Any, dtype: Any) -> Any:
        try:
            return orig_encode(fill_value, dtype)
        except Exception:  # noqa: BLE001 - any encode failure degrades to no-fill
            return None

    hdf_module.encode_cf_fill_value = lenient_encode

    registry = ObjectStoreRegistry({"file://": LocalStore()})
    absolute = Path(path).resolve()
    store = vzhdf.HDFParser()(url=absolute.as_uri(), registry=registry)

    arrays: list[dict[str, Any]] = []

    def walk(group: Any, prefix: str) -> None:
        for name, arr in group.arrays.items():
            md = arr.metadata
            refs = [
                {
                    # ChunkManifest keys are dotted chunk-grid positions
                    # ("" for scalars); the path is the granule as given on
                    # argv, so both generators name it identically.
                    "key": str(key),
                    "path": path,
                    "offset": int(entry["offset"]),
                    "length": int(entry["length"]),
                }
                for key, entry in sorted(arr.manifest.dict().items())
            ]
            codecs = [c for c in map(_codec_string, md.codecs) if c is not None]
            arrays.append(
                {
                    "name": f"{prefix}{name}",
                    "shape": [int(x) for x in md.shape],
                    "chunks": [int(x) for x in md.chunks],
                    "dtype": str(md.data_type.to_native_dtype()),
                    "codecs": codecs,
                    "refs": refs,
                }
            )
        for name, sub in group.groups.items():
            walk(sub, f"{prefix}{name}/")

    # ManifestStore keeps its ManifestGroup tree in `_group`; there is no
    # public accessor for the raw tree (only xarray views), so this reaches
    # into a private attribute — revisit on VirtualiZarr upgrades.
    walk(store._group, "")
    return arrays


def main() -> None:
    """Entry point: granule path on argv, manifest JSON on stdout."""
    if len(sys.argv) != 2:  # noqa: PLR2004 - argv arity, not magic
        print("usage: swath-referencer <granule.h5>", file=sys.stderr)
        raise SystemExit(2)
    path = sys.argv[1]
    if path.lower().endswith((".grib2", ".grb2", ".grib")):
        print(
            "swath-referencer: GRIB2 conformance runs from prototype 0001's "
            "pinned environment (kerchunk/cfgrib/eccodes); this CLI is "
            "HDF5/NetCDF4 only",
            file=sys.stderr,
        )
        raise SystemExit(3)
    try:
        arrays = hdf5_arrays(path)
    except Exception as e:  # CLI boundary: report and exit
        print(f"swath-referencer: hdf5 scan of '{path}' failed: {e}", file=sys.stderr)
        raise SystemExit(4) from e
    manifest = {
        "manifest_version": MANIFEST_VERSION,
        "generator": "virtualizarr",
        "source": path,
        "arrays": arrays,
    }
    json.dump(manifest, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
