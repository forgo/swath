# ADR 0002 — Pure-Rust, single-binary core; build/adopt/bind boundary

**Status:** Accepted · **Date:** 2026-08-08

## Context

The differentiating core (dynamic tiler + materialization engine + orchestration + observability) needs to
be owned, controllable, performant, memory-safe, and long-lived. Government guidance (NSA/CISA, ONCD) favors
memory-safe languages; Rust also enables a single static binary (helps "out of the box"). The risk is
scope: a naive "rewrite everything in Rust" would reimplement mature tooling.

## Decision

Write the core in **Rust**, shipped as a **single static binary**. Hold a strict boundary:

- **Build:** tiler brain (window/overview selection, warp+resample kernels, pixel ops, tile API, per-tile
  decision hooks), materialization planner, process compiler + IR, catalog/ingest orchestration, trace model.
- **Adopt (Rust crates):** COG/Zarr readers (`async-geotiff`, `zarrs`), `object_store`, image codecs, `axum`,
  `geoarrow-rs`.
- **Bind (thin, isolated):** projection math via pure-Rust `proj4rs`, falling back to PROJ C-bindings for
  the exotic long tail.
- **Never reimplement:** the projection/datum catalog, GDAL format drivers, general GDAL warp. (GDAL/rio-tiler
  live only in the test suite as a correctness oracle.)

A purpose-built Rust tiler is justified by requirements, not aesthetics: no existing tiler exposes the
per-tile decision hooks the materialization engine and x-ray overlay require.

## Consequences

- Owned, safe, fast, single-binary core (R7, R10, R8). Correctness proven by perceptual-diff vs GDAL.
- Cost: front-loads the biggest engineering lift; pure-Rust warp/reprojection is nascent, so we bind PROJ
  and adopt the emerging Rust readers rather than waiting.
- Supersedes the earlier "Python core" note in CHARTER.md §8.
