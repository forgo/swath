# swath-referencer

Ingest-time sidecar: turns legacy granules (NetCDF4/HDF5, GRIB2) into virtual chunk manifests via
VirtualiZarr, per ADR 0006. The manifest is the port contract — serve-time reading is pure Rust.

Prototype 0001 (`prototypes/0001-2026-08-08-referencer-bakeoff/`) is the bake-off harness that
decides, per format, when the pure-Rust generator supersedes this sidecar. `chunk_grid` here is
the first shared vocabulary: the same chunk-key math the manifest equivalence check relies on.
