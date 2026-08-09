# swath-referencer

The **conformance reference** for legacy-granule virtual referencing (ADR 0006, concluded by
prototype 0001): `swath-referencer <granule.h5>` emits a schema-v1 `VirtualManifest` via
VirtualiZarr's HDF parser — an implementation independent of the production pure-Rust generator
(`crates/swath-referencer`). Production ingest never calls this package; its jobs are:

- the gated equivalence harness (`just test-referencer`): both generators on a real VNP09GA
  granule, byte-range equivalence asserted;
- the PR-CI conformance test (`tests/test_cli.py`): the sidecar against the tiny committed
  fixture's h5py-derived truth — the same truth the Rust known-answer suite asserts;
- the long-tail fallback for containers the Rust path deliberately rejects (exotic dtypes,
  nonzero per-chunk filter masks).

`chunk_grid`/`chunk_keys` remain the shared chunk-key vocabulary the manifest contract rides on.
Scope: HDF5/NetCDF4. GRIB2 conformance stays with prototype 0001's pinned kerchunk/cfgrib/eccodes
environment until a GRIB dataset is on the serving path.
