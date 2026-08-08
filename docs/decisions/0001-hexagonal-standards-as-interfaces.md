# ADR 0001 — Hexagonal architecture; standards as interface contracts

**Status:** Accepted · **Date:** 2026-08-08

## Context

Swath must fuse a fast-moving ecosystem (dynamic tilers, STAC, cube formats, virtualization) into one
product without becoming hostage to any of those tools as they evolve. The stated fear is fragility:
"susceptible to bugs and breaks as the ecosystem evolves." Minimizing dependency *count* does not by itself
solve this; dependency *isolation* does.

## Decision

Adopt a **ports-and-adapters (hexagonal)** architecture. A small, portable core depends only on narrow
**port** traits and standard data types. Every external tool is an **adapter** behind a port. Crucially,
the ports are shaped like open standards — **STAC** (catalog), the **OGC API family** (Tiles/Maps/Records/
EDR/Features/Processes), and the **openEO/OGC-Processes graph** (product authoring). Standards-as-interfaces
is the anti-lock-in mechanism.

## Consequences

- Ecosystem churn is absorbed at adapters; the core never breaks (satisfies R6).
- Interop and longevity are free where we speak standards (R5). Extension happens at ports (R9).
- Cost: we must define and maintain clean interface boundaries, and some adapters add an integration seam.
