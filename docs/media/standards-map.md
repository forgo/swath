# Standards surfaces map

Solid lines are surfaces that exist in code with conformance-grade test evidence; dashed lines
are deferred, deliberately-not-claimed, or docs-only surfaces. The split is cross-checked
line-by-line against the declared conformance list and the test files in
[`standards-map.notes.md`](standards-map.notes.md).

```mermaid
flowchart LR
    subgraph inbound["Inbound API surfaces"]
        TILES["OGC API - Tiles 1.0<br/>5 conformance classes declared:<br/>core, tileset, tilesets-list,<br/>dataset-tilesets, png<br/>+ TMS 2.0 WebMercatorQuad"]
        OPENEO["openEO API 1.2.0<br/>bounded profile, ADR 0010:<br/>collections, processes 10-op subset,<br/>service_types, XYZ services"]
        CTRL["Control plane, non-standard:<br/>datasets, granules, Trace SSE, healthz"]
        COMMON["OGC API - Common Part 1<br/>deliberately NOT claimed:<br/>no OpenAPI definition served"]
        MAPS["OGC API - Maps"]
        RECORDS["OGC API - Records, phase 2"]
        PROC["OGC API - Processes, phase 2<br/>today only via the openEO vocabulary"]
        EDR["OGC API - EDR, phase 3"]
        FEAT["OGC API - Features, phase 3"]
        OEDEF["openEO auth, jobs, batch,<br/>UDPs, files — out of scope,<br/>absence test-enforced"]
    end

    CORE{{"swath core<br/>hexagonal ports, ADR 0001"}}

    subgraph outbound["Persistence and formats behind the ports"]
        STAC["STAC 1.1.0, hidden persistence<br/>+ datacube extension v2.2.0"]
        COG["COG reading"]
        VREF["Virtual-reference manifest v1<br/>legacy HDF5, NetCDF, GRIB2"]
        GEOZARR["GeoZarr overview pyramids"]
        ICE["Icechunk"]
        GPQ["GeoParquet"]
    end

    TILES --> CORE
    OPENEO --> CORE
    CTRL --> CORE
    COMMON -.-> CORE
    MAPS -.-> CORE
    RECORDS -.-> CORE
    PROC -.-> CORE
    EDR -.-> CORE
    FEAT -.-> CORE
    OEDEF -.-> CORE

    CORE --> STAC
    CORE --> COG
    CORE --> VREF
    CORE -.-> GEOZARR
    CORE -.-> ICE
    CORE -.-> GPQ
```

Honesty notes the map encodes (details and evidence in the sidecar):

- `/conformance` declares **exactly** the five OGC Tiles classes and nothing else; a test pins
  the declared set to the constant and a negative test forbids claiming OGC API - Common.
- openEO is implemented as a bounded profile validated against the pinned official 1.2.0
  schemas, but **no openEO conformance class URI is claimed** — by design (no auth endpoints).
- OGC API - Maps is paired with Tiles in ARCHITECTURE.md's phase table but has no endpoints,
  classes, or tests — it is dashed here (doc over-claim; only a `tilesets-map` link rel exists).
- STAC is deliberately hidden (R2): validated by round-trip property tests and snapshots, not
  by external schemas — solid as a persistence contract, not as an inbound surface.
