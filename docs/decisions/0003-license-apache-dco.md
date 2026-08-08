# ADR 0003 — License: Apache-2.0 with DCO; defer CLA

**Status:** Accepted · **Date:** 2026-08-08

## Context

Swath is an open-core, monetizable platform targeting enterprise and government adopters. The composed
ecosystem is split: the serving lineage (TiTiler, rio-tiler, stac-fastapi, pgstac) is MIT; the cube/storage/
processing lineage (Icechunk, VirtualiZarr, openEO) is Apache-2.0. All are permissive; none copyleft, so no
dependency forces our choice. Relicensing later requires consent from all contributors, so the choice is
best made now, while it is just us.

## Decision

License the project **Apache-2.0**. Require a lightweight **DCO** (Developer Certificate of Origin,
sign-off) on contributions now. **Defer a CLA** unless/until a commercial edition makes dual-licensing
worthwhile.

Rationale: Apache-2.0's explicit **patent grant** matters for serious enterprise/government adoption and
is the norm for company-backed and foundation projects. DCO keeps contribution friction low without
assigning rights; a CLA can be introduced later if needed.

## Consequences

- Stronger adoption posture for the target users; patent protection for users and contributors.
- Requires a `NOTICE` file and DCO sign-off tooling.
- Supersedes the interim MIT `LICENSE`; the repo license file will be updated to Apache-2.0.
