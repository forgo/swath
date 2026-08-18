# Sidecar: standards-map.md — solid/dashed cross-check

Source of truth for "implemented": the declared conformance list in
`crates/swath-api/src/routes.rs` (`CONFORMANCE_CLASSES`, served at `/conformance`) plus the
conformance test files. A surface is **solid** only when code and tests exist; everything
else is **dashed**, whatever the docs say.

## Solid (implemented, test evidence)

| Surface | Declaration | Test evidence |
|---|---|---|
| OGC API - Tiles 1.0 core | `routes.rs` `CONFORMANCE_CLASSES`: `.../ogcapi-tiles-1/1.0/conf/core` | `crates/swath-api/tests/conformance.rs::conformance_is_schema_valid_and_declares_exactly_what_is_implemented` (asserts declared == constant) |
| OGC API - Tiles tileset | `.../conf/tileset` | `conformance.rs::tileset_metadata_is_schema_valid_with_tms_uri_bounds_and_tile_template` |
| OGC API - Tiles tilesets-list | `.../conf/tilesets-list` | `conformance.rs::tilesets_list_is_schema_valid_with_the_required_subset_per_element` |
| OGC API - Tiles dataset-tilesets | `.../conf/dataset-tilesets` | `conformance.rs` landing-page + `/tiles` == `/tilesets` assertions |
| OGC API - Tiles png | `.../conf/png` | `crates/swath-api/tests/tiles.rs` PNG assertions |
| TMS 2.0 WebMercatorQuad | `routes.rs` TMS URI; validated against pinned OGC 17-083r4 schemas (`crates/swath-api/tests/data/ogc/`) | `conformance.rs`; `crates/swath-core/tests/tms_truth.rs` |
| openEO 1.2.0 bounded profile | `crates/swath-api/src/openeo.rs` `OPENEO_ENDPOINTS`; pinned official schemas `crates/swath-api/tests/data/openeo/` | `crates/swath-api/tests/openeo_conformance.rs` (capabilities, collections, 10-process subset byte-identical to oracle copies, service types, errors registry) |
| Control plane (datasets, granules, Trace SSE, healthz) | `crates/swath-api/src/lib.rs` | `crates/swath-api/tests/{granules,trace_stream,conformance}.rs` |
| STAC 1.1.0 hidden persistence + datacube ext v2.2.0 | `crates/swath-core/src/catalog/stac.rs` `STAC_VERSION = "1.1.0"`; `openeo.rs` datacube schema URI | `crates/swath-core/tests/catalog_roundtrip.rs` round-trip properties + snapshots; `openeo_conformance.rs` cube:dimensions assertions. Internal round-trip evidence, not external-schema validation — hidden by design (R2) |
| COG reading | `crates/adapters/swath-source-cog/` | `tests/{describe,windows,overviews}.rs` (GDAL/rio-tiler oracle regime, ADR 0002) |
| Virtual-reference manifest v1 | `crates/swath-manifest/src/lib.rs`; `crates/swath-referencer/`; `crates/adapters/swath-source-virtual/` | `manifest_schema.rs`, `known_answer.rs`, `windows.rs` (SHA-256 pixel oracle); gated real-granule equivalence harness (`just test-referencer`, `.github/workflows/referencer-conformance.yml`) |

## Dashed (deferred, not claimed, or docs-only)

| Surface | Why dashed | Evidence of absence |
|---|---|---|
| OGC API - Common Part 1 | Deliberately not claimed: no OpenAPI definition served | Negative test in `conformance.rs` ("OGC API Common Core must not be declared"); rationale in `crates/swath-api/src/lib.rs` rustdoc |
| OGC API - Maps | Doc over-claim: ARCHITECTURE.md §7 pairs it with Tiles at phase 1, but no endpoints, no classes, no tests — only the `tilesets-map` link rel in `routes.rs` | repo-wide: zero Maps conformance URIs |
| OGC API - Records | Phased (2), no code | zero hits |
| OGC API - Processes (the OGC standard) | Phased (2); the authoring port speaks openEO only (ADR 0010) | no `ogcapi-processes-1` URI anywhere |
| OGC API - EDR | Phased (3), no code | zero hits |
| OGC API - Features | Phased (3), no code | zero hits |
| openEO auth / jobs / batch / UDPs / files | Explicitly out of scope (ADR 0010); this is also why no openEO conformance class is claimed | `openeo_conformance.rs` asserts no `credentials`/`jobs`/`files` path is served |
| GeoZarr pyramids | CHARTER/ARCHITECTURE vocabulary only | zero code hits |
| Icechunk | CHARTER vocabulary only | zero code hits |
| GeoParquet | CHARTER/ARCHITECTURE vocabulary only | zero code hits |

Doc sources for the phase claims: `docs/ARCHITECTURE.md` §7 ("Inbound APIs (standards), by
phase"), `docs/decisions/0001-hexagonal-standards-as-interfaces.md`,
`docs/decisions/0010-openeo-authoring-surface.md`, `docs/CHARTER.md`. Where docs and code
disagree (Maps), the map follows the code and flags the divergence.
