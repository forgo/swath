# ADR 0017 — Icechunk interop target: spec v2.1, written via the icechunk crate

**Status:** Accepted · **Date:** 2026-08-18 · **Source:** issue #191 (the ADR 0016
"spec-version target recorded as an ADR addendum" condition) · **Implements:** ADR 0016's
Icechunk interop half (M8.7)

## Context

ADR 0016 graduated the roadmap's Icechunk record into executed interop: the referencer's
virtual chunk references join the VirtualiZarr→Icechunk ecosystem instead of living only in
Swath's manifest v1. Executing that requires two decisions ADR 0016 deliberately left to
implementation: **which spec version** the interop targets, and **how the store is written**
(the icechunk Rust crate, `zarrs_icechunk`, or hand-emitted format files).

Researched 2026-08-18: icechunk (Rust and Python) is at 2.1.x, writing **spec v2.1**
(flatbuffers-in-zstd container files; v2.1 additions are optional fields, compatible with 2.0
readers). The Rust crate is Apache-2.0 with a first-class virtual-reference write API
(`Store::set_virtual_refs`, `VirtualChunkRef` = URL + byte range — exactly the manifest's
shape) and feature-gates its heavy baggage: `default-features = false` drops the AWS SDK and
all remote object-store backends. `zarrs_icechunk` is a zarrs *storage* adapter with no
virtual-ref surface (it would still drop down to the icechunk API for every ref). The spec is
implementable by hand but ~a year old and moving; owning flatbuffers/zstd emission and
ref-file atomicity to avoid a vetted Apache-2.0 dependency buys risk, not value.

## Decision

- **Spec target: Icechunk spec v2.1**, as written by icechunk 2.1.x — the pinned
  `icechunk = 2.1.2` (workspace `Cargo.toml`) is the single source of truth; Renovate moves
  it, and a bump that changes the written spec version updates this ADR's target line.
- **Write mechanism: the `icechunk` Rust crate**, `default-features = false` +
  `object-store-fs` (local repositories; a remote backend is feature-flagged in when a real
  deployment demands it). Not `zarrs_icechunk` (no virtual-ref surface), not format-level
  emission (owning a moving on-disk format to avoid a vetted dependency).
- **Placement: the unpublished `swath-icechunk` adapter crate.** The published
  `swath-referencer` stays exactly as extracted — no tokio, no icechunk — per ADR 0016's
  standalone rule; the committer consumes the published `swath-manifest` vocabulary.
- **Conformance is executable:** the credential-free tiny-fixture round trip
  (`just test-icechunk`, byte-identical through Icechunk's own container resolution in Rust
  and through icechunk-python/zarr/xarray) and the real-granule gate inside
  `just test-referencer` — the same two-tier pattern as the referencer's VirtualiZarr
  equivalence.

## Consequences

- Skips are loud: arrays with no Zarr v3 mapping (byte-string metadata blobs, GRIB2 packing)
  are reported per array, never silently dropped; an all-skip manifest refuses to commit.
- The codec vocabulary written (`numcodecs.shuffle`/`numcodecs.zlib`) is the one
  VirtualiZarr/kerchunk write, so zarr-python decodes Swath-committed stores with no Swath
  code in the loop.
- Supply chain: `icechunk` (+ `url`, already transitive) enters the workspace graph
  feature-trimmed, itemized in PR #191 per the #190 checkpoint pattern; it never enters any
  published crate's tree.

## Reopen / supersede conditions

- A spec version bump that breaks the v2.1 mapping (ADR 0016's "Icechunk spec drift"
  condition) reopens the interop decision.
- A remote-deployment requirement (S3/GCS containers) revisits the feature trim, not this
  ADR's write-mechanism decision.
