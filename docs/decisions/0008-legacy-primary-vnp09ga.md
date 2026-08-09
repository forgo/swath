# ADR 0008 — Legacy-primary dataset: VNP09GA (VNP09 swath is HDF4)

**Status:** Accepted · **Date:** 2026-08-08 · **Amends:** ADR 0004 (legacy-primary detail)

## Context

ADR 0004 named **VIIRS VNP09** surface reflectance as the legacy-primary dataset, described as
"NetCDF4/HDF5" that "virtualizes reliably." During prototype 0001's validation (issue #17) we
downloaded a real VNP09 collection-002 granule and found it is actually **HDF4** (MODIS-heritage
container; magic bytes `0e 03 13 01`) — unreadable by libhdf5, h5py, and therefore by *both* the
Python VirtualiZarr path and the Rust path. This is an ecosystem gap, not a Rust-maturity gap,
and it is exactly the HDF4/HDF-EOS territory ADR 0004 deliberately deferred as a MODIS stretch.

## Decision

The legacy-primary dataset is **VNP09GA** (VIIRS daily gridded surface reflectance): the same
instrument and science lineage, distributed as genuine **HDF5** with netCDF-4-style nested
groups, on which the full bake-off ran successfully (67 arrays, 1,551 chunk refs, byte-identical
manifests; deflate-8 compression exercised). Swath VNP09 joins MODIS in the **HDF4 stretch
bucket** — revisit only if/when an HDF4 referencing path earns its place.

## Consequences

- The Phase-2 legacy proof remains "identical product, two very different sources" (HLS NDVI vs
  VIIRS NDVI) — VNP09GA carries the surface-reflectance bands NDVI needs.
- Prototype 0001, `scripts/fetch_sample.sh`, and the production referencer target VNP09GA.
- ADR 0004 stands otherwise (HLS clean-path, MODIS stretch); this ADR narrows one detail with
  evidence. Assumption-check lesson recorded: format claims about specific NASA collections are
  verified by downloading a granule, not by product documentation.
