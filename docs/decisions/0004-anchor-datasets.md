# ADR 0004 — Anchor datasets: HLS (clean), VIIRS (legacy-primary), MODIS (stretch)

**Status:** Accepted · **Date:** 2026-08-08

## Context

We need concrete first datasets that (a) exercise the clean cloud-native path and (b) prove seamless legacy
ingest, ideally with apples-to-apples derived products. NASA is actively running a MODIS→VIIRS transition
(Terra/Aqua drifting toward decommissioning; VIIRS is the operational continuation and the recommended NDVI
source).

## Decision

- **Clean path:** **HLS** (Harmonized Landsat Sentinel-2) — already COG, in CMR, with a published
  titiler-cmr tiling guide and official RGB + NDVI products. Day-one benchmark.
- **Legacy-primary:** **VIIRS** VNP09 surface reflectance → NDVI (NetCDF4/HDF5), virtualized in place. It
  mirrors HLS's derived products exactly (same product, different source/format), is operationally live,
  and NetCDF virtualizes reliably (lower risk).
- **Stretch:** **MODIS** HDF-EOS — the frozen 25-year archive — attempted *after* the mechanism is proven on
  the lower-risk NetCDF path.

## Consequences

- The Phase-2 legacy proof is "identical product, two very different sources/formats" (HLS NDVI vs VIIRS NDVI).
- Aligns with the NESDIS/weather lineage and the operational transition happening now.
- MODIS's gnarlier HDF4/HDF-EOS is deliberately deferred, reducing early risk.
