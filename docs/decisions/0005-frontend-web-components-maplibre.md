# ADR 0005 — Frontend: Web Components + MapLibre GL; no framework, no deck.gl

**Status:** Accepted · **Date:** 2026-08-08

## Context

Priorities are bundle size, runtime performance, and longevity (frontend frameworks are a leading source of
"it broke as the ecosystem moved"). The core visuals are raster/derived-product tiles, the x-ray overlay,
and moderate vector (GeoParquet). MapLibre GL alone covers raster tiles, data-driven vector styling,
heatmaps, basic 3D, and — critically — **custom WebGL layers** (enough to build the x-ray overlay natively).

## Decision

Build the frontend as **vanilla Web Components / Custom Elements** in TypeScript with a tiny in-house
reactive layer. **MapLibre GL** is the single necessary dependency (BSD, framework-agnostic WebGL renderer).
**No React. No deck.gl** for now. The x-ray overlay is a MapLibre custom WebGL/Canvas layer fed by the Trace
SSE stream.

deck.gl is deferred: it earns its weight only for GPU-scale vector/point/3D (e.g. dense embeddings scatter),
which is later and optional, and would be added as an isolated visualization module behind a boundary.

## Consequences

- Minimal bundle, strong performance, framework-churn resilience (R10, R6).
- Cost: we build our own reactivity/state/routing and accept lower dev velocity for it.
- Supersedes the earlier React/deck.gl note in CHARTER.md §8.
