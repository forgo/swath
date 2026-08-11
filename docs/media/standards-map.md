# Standards surfaces map

Hand-crafted SVG — [`standards-map.svg`](standards-map.svg) is both the editable source and
the export (canonical). Solid connectors and boxes are surfaces that exist in code with
conformance-grade test evidence; dashed are deferred, deliberately-not-claimed, or docs-only.
The split is cross-checked line-by-line against the declared conformance list and the test
files in [`standards-map.notes.md`](standards-map.notes.md).

![Standards surfaces around the hexagonal swath core (ADR 0001). Implemented, drawn solid —
inbound: OGC API Tiles 1.0 with exactly five declared conformance classes (core, tileset,
tilesets-list, dataset-tilesets, png) plus TMS 2.0 WebMercatorQuad; openEO API 1.2.0 as a
bounded profile (collections, 10-process subset, XYZ services, ADR 0010); and the
non-standard control plane (datasets, granules, Trace SSE, healthz). Implemented persistence
and formats: STAC 1.1.0 hidden persistence with datacube extension v2.2.0; Cloud-Optimized
GeoTIFF reading under the GDAL/rio-tiler oracle; virtual-reference manifest v1 for legacy
HDF5 and GRIB2 with a SHA-256 pixel oracle. Deferred or not claimed, drawn dashed: OGC API
Common (deliberately not claimed), Maps (doc over-claim, no code), Records and Processes
(phase 2), EDR and Features (phase 3), openEO auth/jobs/batch/UDPs/files (out of scope), and
GeoZarr, Icechunk, GeoParquet (docs-only).](standards-map.svg)

Honesty notes the map encodes (details and evidence in the sidecar):

- `/conformance` declares **exactly** the five OGC Tiles classes and nothing else; a test pins
  the declared set to the constant and a negative test forbids claiming OGC API - Common.
- openEO is implemented as a bounded profile validated against the pinned official 1.2.0
  schemas, but **no openEO conformance class URI is claimed** — by design (no auth endpoints).
- OGC API - Maps is paired with Tiles in ARCHITECTURE.md's phase table but has no endpoints,
  classes, or tests — it is dashed here (doc over-claim; only a `tilesets-map` link rel exists).
- STAC is deliberately hidden (R2): validated by round-trip property tests and snapshots, not
  by external schemas — solid as a persistence contract, not as an inbound surface.
