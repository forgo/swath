# swath-manifest

Virtual-reference manifest v1 — the versioned, deny-unknown JSON contract
describing a legacy granule (HDF5/NetCDF4, GRIB2) as **byte-range
references into the original file**, extracted from
[Swath](https://github.com/forgo/swath) (ADR 0016).

A `VirtualManifest` lets a reader serve a legacy granule as a cloud-native
cube without rewriting it: each array carries its shape, chunk grid, numpy
dtype string, codec chain (HDF5 filter-pipeline order), optional
georeferencing (CRS as EPSG or proj string, GDAL-convention geotransform,
nodata, band semantics), and the chunk byte ranges. The normative schema
description ships with this crate as [`SPEC.md`](SPEC.md); the API is that
document made executable.

## The generator contract

Whoever generates a manifest — the pure-Rust
[`swath-referencer`](https://crates.io/crates/swath-referencer), a
VirtualiZarr/kerchunk sidecar, or your own tool — the reading side treats
them identically, so generators are interchangeable behind the contract.
`compare()` is that interchangeability made executable: same arrays, same
grids, same per-chunk byte ranges, with every mismatch reported for a
conformance run's eyes.

## Versioning

Documents carry `"manifest_version": 1`. Readers reject versions (and
unknown fields) they do not understand, loudly, at the parse boundary —
version skew is never half-parsed. The serde shape is pinned by a snapshot
test that travels with the crate.

## Dependencies

Exactly `serde` + `serde_json`. Errors are hand-implemented; nothing else
enters the tree.

## Status

Published as a `0.1.0-alpha.N` — built from a tagged commit through the
full Swath CI gate, with no API stability promised between alphas.
Licensed Apache-2.0.
