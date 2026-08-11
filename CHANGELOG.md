# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Until the first plain-semver release, versions are `0.1.0-alpha.N`
pre-releases: built, tested, and checksummed, with no semver or stability
commitment (see docs/RELEASING.md).

## [0.1.0-alpha.1] - 2026-08-11

### Features

- Cargo workspace skeleton with inherited lints
- Justfile task contract (#44)
- Cargo-deny supply-chain gate configuration (#45)
- CI pipeline — setup-rust composite action + ci-ok gated workflow (#46)
- Zizmor workflow linting in CI and prek pre-commit hooks (#47)
- Nightly RustSec advisory scanning with issue filing (#49)
- Coverage job uploading to Codecov (#51)
- Conventional-commit PR title lint (#52)
- Main-branch ruleset config (deferred enforcement) (#53)
- Project hygiene files (SECURITY, CONTRIBUTING, CODEOWNERS, issue forms, CoC) (#54)
- Renovate configuration with cooldown and grouped updates (#55)
- REUSE 3.3 / SPDX compliance with CI gate (#56)
- Docker compose skeleton (pgstac + MinIO) with e2e smoke gate (#57)
- Web scaffold — pnpm, Biome, strict TS 7, Vitest browser mode (#60)
- Python scaffold — uv workspace, ruff, pyright strict, hypothesis (#62)
- GRIB2 pure-Rust referencer via gribberish (prototype 0001) (#63)
- HDF5/NetCDF4 pure-Rust referencer via hdf5-metno (prototype 0001) (#64)
- Conclude referencer bake-off — Rust-primary for GRIB2 and HDF5 (#65)
- GDAL oracle harness and perceptual-diff testkit (#66)
- Deterministic HLS COG test fixtures (#68)
- Swath-core domain types — TMS math, raster vocabulary, trace model (#69)
- RasterSource port + COG adapter over object_store (#70)
- Reproject port + proj4rs adapter (#71)
- Swath-render warp and resample kernels (#72)
- Render IR pixel ops and PNG tile encoding (#73)
- Render_tile orchestration with full Trace emission (#74)
- OGC API - Tiles endpoint (swath-api) (#75)
- Trace SSE stream endpoint (#76)
- Swath serve binary + compose e2e (#77)
- Enable public-repo security surfaces (Scorecard, CodeQL, dependency-review, zizmor SARIF) (#78)
- Catalog domain model + pgstac adapter (#79)
- Ingest orchestrator, filedrop events, ingest-to-pixel timer (#80)
- OpenEO process-graph compiler to Render IR (#81)
- Swath-map viewer component (MapLibre over OGC tiles) (#82)
- X-ray overlay v0 — per-tile decisions, timings, ingest-to-pixel readout (#83)
- North-star stopwatch demo and tightened ingest-to-pixel regression (#84)
- TileCache port, object_store adapter, write-through serving (#86)
- Production Rust referencer behind IngestReferencer port (#87)
- Serve existing COG overviews (Overview strategy) (#88)
- Virtual-reference RasterSource — serve legacy cubes from original bytes (#89)
- Cost-aware materialization planner (#90)
- X-ray v1 — planner why-view, bytes heatmap, live trace feed (#92)
- OpenEO API authoring surface (capabilities, collections, processes, XYZ services) (#93)
- Swath-testsupport dev-crate — shared truth tables, temp dirs, gated-skip (#127)
- Colormap engine — viridis, magma, RdYlGn; NDVI colormapped by default (#128)
- Production web build served from the binary + opt-in CORS (#134)
- Legacy-hdf5 cargo feature — default-on, fast opt-out for dev loops (#136)
- Granule browsing API — GET /datasets/{id}/granules (#140)
- *(web)* Extract shared TMS math into tms.ts, oracled by a morecantile truth table (#141)
- Release pipeline — alpha prereleases via release-plz + cargo-dist (#143)
- *(web)* Landing page, layer browser, and localStorage view state (#142)

### Bug fixes

- Run all CI jobs on push to main; filter paths on PRs only (#48)
- Cover docker-compose.yml in REUSE annotations (#58)
- Scope CI tool installs and lock source-build fallbacks (#61)
- Authenticate CI tool installs and scope tools per job (#67)
- Demo shows the imagery — true footprint, basemap, full-height map (#85)

### Refactoring

- Single RenderPlan constructor with PlanKind round-trip property (#130)

### Documentation

- ADR 0009 — fenced sinusoidal math exception (#91)
- ADR 0012 — render stays inline on the async runtime (resolves 16.7) (#135)
- PERFORMANCE.md with re-measured, regenerable baselines (#145)

### Testing

- Swath-cli coverage — serve, ingest, config errors, clap tree (#129)
- Typed e2e harness — bash assertions become named Rust tests (#131)
- Render-stage criterion benches and just bench (#132)
- Just load — concurrency harness with committed baselines (#133)

### Miscellaneous

- Add pull request template (#43)
- Check in AI working agreement and merge-blocking hook (#59)
- Hygiene sweep — gated-skip unification, core docs claim, machete, deny decision (#137)
- Publish GHCR image on main with smoke-tested one-liner (#138)

### Other

- North star, architecture, ADRs 0001-0007, engineering standards
