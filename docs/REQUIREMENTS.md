# Swath — Requirements & Vision (North Star)

*The enduring reference. This document changes rarely and deliberately. Everything else — the charter,
the architecture, ADRs, prototypes, code — may evolve freely, but it should always be checkable against
what is written here. If we ever drift from this, either we consciously amend this document (with a dated
entry in §10) or we correct course. Read this first, forever.*

**Status:** v1.0 — 2026-08-08.

---

## 1. Mission (one sentence)

**Satellite data comes in, and is immediately available on a map — from a single pane of glass — and
anyone can derive a new product from the live data flow and publish it the same way.**

## 2. The problem we exist to solve

The cloud-native geospatial primitives (dynamic tilers, STAC, cube formats, virtualization) are mature,
but no product fuses them into one managed, low-latency, observable loop. Standing up "data in → live on
a map, with a place to build and publish new products" today means hand-wiring several projects per
deployment. Swath is that missing product layer.

## 3. North-star metric

**Ingest-to-pixel latency** — the time from *"a new granule arrives"* to *"a correct tile is visible on
the map."* Every subsystem is measured against it; the platform reports it continuously and honestly.

## 4. Non-negotiable requirements

These are the durable commitments. Numbered so we can reference them in reviews and ADRs.

- **R1 — Immediacy.** New data becomes visible on a map with no manual per-granule work. The happy path is
  automatic: arrive → catalog → serve.
- **R2 — Single pane of glass.** Users manage *datasets* and *layers*, never STAC/Zarr/tiler internals.
  The plumbing is invisible when it works.
- **R3 — Derive & publish.** A data scientist can define a new product from the live flow and host it,
  tiled and cataloged, the same way built-in products are served — with low latency.
- **R4 — Glass-box.** Nothing the system does is a black box to those who want to look. Every render can be
  explained (live vs. overview vs. cache, bytes read, chunks touched, timings), and that explanation is
  the same data our tests assert against.
- **R5 — Standards as the contract.** External interfaces are open standards (STAC, the OGC API family,
  the openEO/Processes graph). Interop and longevity come from speaking standards, not private APIs.
- **R6 — Resilient to the ecosystem.** The core cannot be broken by churn in any external tool. Tools live
  behind narrow, swappable interfaces; the core depends only on standards and its own logic.
- **R7 — Own what matters.** The differentiating core (the tiler, the materialization engine, the
  orchestration, the observability) is code we own, control, test, and can make world-class. We adopt or
  bind for the rest, and never reimplement what would mean rewriting the world (projections, format drivers).
- **R8 — Out of the box.** A newcomer runs one command and gets a working, sane-defaulted system. Ambition
  never shows up as fiddliness for the operator.
- **R9 — Extensible without forking.** Third parties extend the system at its interfaces (a new source, a
  new product, a new backend) without editing the core.
- **R10 — Correct, safe, documented.** Correctness is the first priority; memory-safety is structural (Rust
  core); every interface and decision is well-documented. Performance and memory efficiency are first-class,
  not afterthoughts.

## 5. What we are building (pillars)

1. **Ground-segment ingest spine** — event-driven ingest that also absorbs legacy file archives without
   rewriting them.
2. **Data-scientist product loop** — author a product as a standard process graph; the materialization
   engine compiles it into low-latency dynamic tiles and decides live-vs-overview-vs-cache.
3. **Single-pane control plane** — datasets/layers over the OGC API family; STAC hidden.
4. **Frontier: geo-embeddings** — embeddings as a first-class product type enabling semantic/similarity
   search over the same catalog.

## 6. How we build (principles)

Hexagonal core (ports & adapters). Standards-as-interfaces. Pure-Rust, single-binary core. Glass-box by
construction (the trace is the test oracle). Go deep on a vertical before going wide. Intuitive by default,
extensible at the edges. Priorities, in order: **correctness → performance/memory → UX → safety → docs →
standards breadth.**

## 7. Success criteria (the "are we still on vision?" checklist)

At any point in the future, we are on-vision if we can answer *yes* to these:

- Can a new granule appear on the map with zero manual per-granule steps, and can we state its
  ingest-to-pixel latency? (R1, R3)
- Can a non-expert operate the platform without ever seeing the word "STAC"? (R2)
- Can a data scientist publish a new derived product as a hosted, tiled layer in one motion? (R3)
- Can any developer open the x-ray view and see *why* a given tile was served the way it was — and does a
  test assert that same fact? (R4)
- Do all external interfaces validate against their OGC/STAC/openEO standard? (R5)
- If a key upstream tool changed or vanished tomorrow, would the fix be one adapter, not a core rewrite? (R6)
- Is the differentiating logic ours, tested against oracles, and are we free of "rewrote the world"
  reimplementations? (R7)
- Does `one command` still stand up a working system? (R8)
- Can someone add a source/product/backend without touching our core? (R9)

## 8. Non-goals

Not a general GIS/desktop tool. Not a data archive or storage vendor. Not a reimplementation of PROJ, GDAL
format drivers, or existing dynamic tilers for their own sake. Not tied to any single agency, cloud, or
dataset. Not a framework-heavy web app.

## 9. Business shape (for context, not a requirement)

Open-core: a permissive (Apache-2.0), genuinely useful, self-hostable core; a commercial layer (managed
hosting, enterprise/government features, support) on top. The core must stand on its own merits.

## 10. Amendments log

Changes to this north star are recorded here, dated, with rationale. If this log grows quickly, we are
either learning fast or drifting — and either way it should be a conscious, visible act.

- 2026-08-08 — v1.0 established.
