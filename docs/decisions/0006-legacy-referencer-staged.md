# ADR 0006 — Legacy referencer: staged Python→Rust behind one manifest port

**Status:** Accepted (pending evidence from prototype 0001) · **Date:** 2026-08-08

## Context

Legacy archives (NetCDF4/HDF5, GRIB2, HDF4/HDF-EOS) should be ingested without rewriting them, via virtual
references. Generation is the hard, format-specific part; reading is comparatively mechanical and already
viable in Rust (`zarrs` + `zarrs_icechunk` reading Icechunk virtual chunk references). Rust generation
maturity varies by format: GRIB2 is pure-Rust-ready (`gribberish`); HDF5/NetCDF4 is correct today via
`hdf5-metno` bindings (`H5Dget_chunk_info` exposes chunk byte-ranges), with a pure-pure-Rust reader as a
frontier stretch; HDF4/HDF-EOS is weakest.

## Decision

Make the **virtual manifest (Icechunk virtual chunk references) the port contract** (`IngestReferencer`).
Serve-time reading is pure Rust regardless of generator. Stage the generator:

1. Ship the **Python VirtualiZarr sidecar** first as the broad-coverage adapter (unblocks all formats).
2. Build **`referencer-rs`** behind the same port, greedily: GRIB2 (pure Rust) early; VIIRS HDF5 via
   `hdf5-metno` next; optional pure-pure-Rust HDF5 chunk-index reader validated against the bound adapter.
3. **Sunset Python per-format** as Rust reaches parity; retain it only for the exotic long tail.

Supporting both is a hedge, not debt, **because** they share the manifest contract and a conformance
(equivalence) harness. It would only be waste if two generators competed permanently on the same format.

## Consequences

- Broadest coverage on day one + a path to the pure-Rust, single-binary ideal with zero migration pain.
- Resolves ARCHITECTURE.md open question §16.8.
- The go/no-go evidence per format comes from **prototype 0001 (referencer-bakeoff)**.
