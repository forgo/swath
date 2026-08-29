# Swath — Requirements & Vision (North Star)

*The enduring reference; it changes rarely and deliberately. Everything else may evolve freely
but should always be checkable against what is written here — drift means a conscious, dated
amendment in §10 or a course correction. Read this first, forever.*

**Status:** v1.0 — 2026-08-08; amended through 2026-08-29 (§10).

---

## 1. Mission (one sentence)

**Satellite data comes in and is immediately live on a map — and anyone can derive a new
product from that live flow and publish it the same way, from one screen.**

## 2. The problem we exist to solve

The cloud-native geospatial primitives are mature, but no product fuses them into one managed,
low-latency, observable loop — standing that up means hand-wiring several projects per
deployment. Swath is that missing product layer.

## 3. North-star metric

**Ingest-to-pixel latency** — from *"a new granule arrives"* to *"a correct tile is visible on
the map."* Every subsystem is measured against it; the platform reports it continuously and
honestly.

## 4. Non-negotiable requirements

The durable commitments, numbered for reference in reviews and ADRs.

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

### R1–R10 status (annotated 2026-08-11, issue #123)

A dated snapshot, not a rewrite — the requirement texts above are unchanged; scope evolutions
are the §10 amendments. "Met" means linked evidence exists in the tree.

| Req | Status | Evidence |
|---|---|---|
| R1 | **Met** | Filedrop → catalog → serve, no per-granule steps; measured <!-- number:i2p-ms -->646 ms<!-- /number:i2p-ms -->, budget-asserted every commit (`PERFORMANCE.md` §4) |
| R2 | **Met, as amended (A1)** | Datasets/layers control plane + UI, STAC hidden (`docs/design/catalog-domain.md`) |
| R3 | **Met** | openEO graph → live tiled XYZ layer in one motion (ADR 0010) |
| R4 | **Met** | Per-tile trace; the e2e asserts on the same trace the x-ray renders (`ARCHITECTURE.md` §9) |
| R5 | **Met, as amended (A3)** | Tiles + native openEO at a documented bounded profile, conformance-tested; breadth is Phase-3 scope, not a silent gap |
| R6 | **Met** | Six ports, seven adapter crates, core free of tool types (`ARCHITECTURE.md` §6) |
| R7 | **Met, as amended (A2)** | The differentiating path is owned Rust; projections bound, GDAL/TiTiler/morecantile kept as test oracles (`ARCHITECTURE.md` §3) |
| R8 | **Met, as amended (A4)** | One command from a checkout and a no-checkout GHCR one-liner, both CI-exercised |
| R9 | **Met** | ADR 0013; `EXTENDING.md` is verified by construction |
| R10 | **Partially met** — docs breadth in flight | Oracle pixel-diffs, proptest/Hypothesis, conformance gates, goldens; no `unsafe`; performance measured and enforced. Remaining docs tracked (#118, #119) |

## 5. What we are building (pillars)

**Ground-segment ingest spine** (event-driven, absorbing legacy archives without rewrites);
the **data-scientist product loop** (a standard process graph compiled into low-latency
dynamic tiles); the **single-pane control plane** (datasets/layers over the OGC API family,
STAC hidden); the **geo-embeddings frontier** (embeddings as a first-class product type).

## 6. How we build (principles)

Hexagonal core. Standards-as-interfaces. Pure-Rust, single-binary core. Glass-box by
construction (the trace is the test oracle). Deep before wide. Priorities, in order:
**correctness → performance/memory → UX → safety → docs → standards breadth.**

## 7. Success criteria (the "are we still on vision?" checklist)

On-vision means answering *yes* to each: zero-manual-step granules with a stated
ingest-to-pixel latency (R1, R3); a non-expert never sees "STAC" (R2); one-motion publishing
(R3); the x-ray explains every tile and a test asserts the same fact (R4); standards-validated
interfaces (R5); a vanished upstream tool costs one adapter (R6); the differentiating logic is
ours, oracle-tested (R7); one command stands up the system (R8); extension without touching the
core (R9).

## 8. Non-goals

Not a general GIS/desktop tool. Not a data archive or storage vendor. Not a reimplementation of
PROJ, GDAL format drivers, or existing dynamic tilers for their own sake. Not tied to any single
agency, cloud, or dataset. Not a framework-heavy web app.

## 9. Business shape (for context, not a requirement)

Open-core: a permissive (Apache-2.0), genuinely useful, self-hostable core; a commercial layer
(managed hosting, enterprise/government features, support) on top. The core must stand on its
own merits.

## 10. Amendments log

Changes to this north star are recorded here, dated, with rationale — a conscious, visible
act.

- 2026-08-08 — v1.0 established.

- **2026-08-11 — A1: R2 scope as built.** "Never STAC" is scoped to Swath's own control plane
  and UI; the openEO surface serves STAC-based collection metadata because openEO collections
  *are* STAC — at a standards boundary, R5 governs. *Decided:* ADR 0010,
  `docs/design/catalog-domain.md`. *Sign-off:* PR #157.

- **2026-08-11 — A2: compose-to-oracle demotion.** §2's premise implied composing the existing
  serving primitives; as built, Swath owns the tiler, planner, compiler, trace, and
  orchestration in pure Rust, composes pgstac and object storage, binds proj4rs, and demotes
  the rest to correctness oracles (`tests/oracle/`; VirtualiZarr is the ingest-time conformance
  reference, ADR 0006). Owning the hot path proved out once measured
  (<!-- number:i2p-ms -->646 ms<!-- /number:i2p-ms -->;
  <!-- number:ref-ratio-approx -->~40×<!-- /number:ref-ratio-approx --> referencer), and
  validating against the ecosystem is a stronger claim than inheriting from it. *Decided:*
  ADR 0002, the oracle harness (#19), ADRs 0006/0008. *Sign-off:* PR #157.

- **2026-08-11 — A3: R5 bounded-profile honesty.** R5 is satisfied at *documented bounded
  profiles*: each surface implements an honest subset whose capabilities document advertises
  **only what exists**; full-breadth conformance is roadmap, not implied. *Decided:* ADR 0010;
  conformance tests `crates/swath-api/tests/`. *Sign-off:* PR #157.

- **2026-08-11 — A4: R8 one command, from checkout.** Defined pre-1.0 as: from a fresh
  checkout, one command stands up the system (`docker compose up` / `just demo`), and with no
  checkout, one CI-smoke-tested command runs the demo container; installers are
  graduation-tier (`docs/RELEASING.md`). *Decided:* issue #104; ENGINEERING §7. *Sign-off:*
  PR #157.

- **2026-08-29 — A5: the mission sentence, one wording.** §1 now carries the sentence the
  README opens with — wording only ("live on a map", "from one screen"); the meaning is
  v1.0's. A docs gate keeps README and §1 identical, so the mission is stated once and quoted
  everywhere else. *Decided:* issue #338. *Sign-off:* PR #358.
