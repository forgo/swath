# Swath — Requirements & Vision (North Star)

*The enduring reference. This document changes rarely and deliberately. Everything else — the charter,
the architecture, ADRs, prototypes, code — may evolve freely, but it should always be checkable against
what is written here. If we ever drift from this, either we consciously amend this document (with a dated
entry in §10) or we correct course. Read this first, forever.*

**Status:** v1.0 — 2026-08-08; amended through 2026-08-11 (§10).

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

### R1–R10 status (annotated 2026-08-11, issue #123)

A dated snapshot, not a rewrite: the requirement texts above are unchanged; scope evolutions are the
§10 amendments (A1–A4). "Met" means evidence exists in the tree and is linked.

| Req | Status | Evidence |
|---|---|---|
| R1 | **Met** | Filedrop → catalog → serve with no per-granule steps; measured 646 ms ingest-to-pixel, budget-asserted every commit (`PERFORMANCE.md` §4, `crates/swath-e2e`) |
| R2 | **Met, as amended (A1)** | Datasets/layers control plane + UI, STAC hidden (`docs/design/catalog-domain.md`); the openEO boundary speaks STAC because the standard does (A1, ADR 0010) |
| R3 | **Met** | openEO graph → live tiled XYZ layer in one motion (ADR 0010; authoring panel e2e `web/e2e/authoring.e2e.ts`) |
| R4 | **Met** | Per-tile trace: decision, bytes read, chunk/byte-range provenance, timings, planner candidates; the e2e asserts on the same trace the x-ray renders (`ARCHITECTURE.md` §9, `crates/swath-e2e`) |
| R5 | **Met, as amended (A3)** | OGC API - Tiles + native openEO at a documented bounded profile, conformance-tested (`crates/swath-api/tests/conformance.rs`, `openeo_conformance.rs`); breadth (EDR, Features) is Phase-3 scope, not a silent gap (`CHARTER.md` §10) |
| R6 | **Met** | Six ports, seven adapter crates, core free of tool types (`ARCHITECTURE.md` §6; reconciliation #152) |
| R7 | **Met, as amended (A2)** | Tiler, planner, compiler, trace, orchestration are owned Rust; projections bound (`proj4rs`), GDAL/TiTiler/morecantile kept as test oracles (`ARCHITECTURE.md` §3) |
| R8 | **Met, as amended (A4)** | One command from a checkout (`docker compose up` / `just demo`) and a no-checkout GHCR demo one-liner, both CI-exercised (A4) |
| R9 | **Met** | ADR 0013; `EXTENDING.md` is verified by construction (every path exercised by a shipped adapter or test) |
| R10 | **Partially met** — docs breadth in flight | Correctness: oracle pixel-diffs, proptest/Hypothesis, conformance gates, goldens (`justfile`, CI). Safety: no `unsafe` in the workspace. Performance: measured and enforced (`PERFORMANCE.md`). Remaining: operator guide / quickstart / endpoint reference are tracked (#118, #119) — until they land, "every interface well-documented" is not fully true |

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

- **2026-08-11 — A1: R2 scope as built (openEO clients see STAC; Swath's own pane never does).**
  *What changed:* R2's "never STAC/Zarr/tiler internals" is scoped to Swath's own control plane and
  UI. The openEO authoring surface serves STAC-based collection metadata to openEO clients, because
  openEO collections *are* STAC — that is what the standard specifies.
  *Why:* at a standards boundary, R5 governs: hiding STAC from a client whose standard is built on
  STAC would break interop, the very thing R5 exists to protect. Operators and the single pane still
  never see STAC (R2 intact where it was aimed).
  *Where decided:* ADR 0010 (2026-08-09), `docs/design/catalog-domain.md`.
  *Maintainer sign-off:* recorded in PR #157.

- **2026-08-11 — A2: compose-to-oracle demotion (the serving path is owned, the ecosystem became
  our test oracle).**
  *What changed:* §2's premise implied Swath would compose the existing serving primitives (TiTiler /
  rio-tiler, `xpublish-tiles`, stac-fastapi, morecantile) into the loop. As built, Swath owns the
  tiler, planner, compiler, trace, and orchestration in pure Rust; it composes **pgstac** and
  **object storage**, binds **proj4rs**, and demotes the rest to correctness oracles that the test
  suite pixel-diffs and truth-tables against (`tests/oracle/`); VirtualiZarr is the ingest-time
  conformance reference (ADR 0006).
  *Why:* R6/R7 pushed the other way once measured: owning the hot path made the core resilient,
  testable, and fast (646 ms ingest-to-pixel; ~40× referencer, `PERFORMANCE.md`), and validating
  against the ecosystem is a stronger correctness claim than inheriting from it.
  *Where decided:* ADR 0002 (2026-08-08), the oracle harness (#19), ADRs 0006/0008; charter §8
  reconciled in this change.
  *Maintainer sign-off:* recorded in PR #157.

- **2026-08-11 — A3: R5 bounded-profile honesty (standards implemented to a truthful, advertised
  subset).**
  *What changed:* R5 is satisfied at *documented bounded profiles*: each external surface implements
  its standard to an honest subset whose capabilities document advertises **only what exists** —
  openEO API 1.2.0 with capabilities/collections/processes and `xyz` secondary services (jobs,
  batch, billing, auth deferred until demanded), OGC API - Tiles for serving. Full-breadth
  conformance is roadmap, not implied.
  *Why:* claiming full conformance while shipping a subset is the kind of silent aspiration this
  project forbids; a truthful capabilities document is itself the standards-native way to bound
  scope. Real openEO clients discover and use exactly what is there.
  *Where decided:* ADR 0010 (2026-08-09); conformance tests `crates/swath-api/tests/`.
  *Maintainer sign-off:* recorded in PR #157.

- **2026-08-11 — A4: R8 one command, from checkout (and a no-checkout demo one-liner).**
  *What changed:* R8's "a newcomer runs one command" is defined pre-1.0 as: from a fresh checkout,
  one command stands up the working system (`docker compose up` for the stack; `just demo` for the
  guided demo), and with no checkout at all, one CI-smoke-tested command runs the demo container
  (`docker run -p 8080:8080 ghcr.io/forgo/swath serve --fixtures`). Installers and
  platform-package distribution are graduation-tier (`docs/RELEASING.md`).
  *Why:* "one command" needed an honest operational definition while the project is pre-release;
  the GHCR one-liner is verified by the same smoke test as every published image, so the promise is
  enforced, not aspirational.
  *Where decided:* issue #104 (GHCR image + CI-tested one-liner, 2026-08-10); ENGINEERING §7
  amendment (release tier); `docs/RELEASING.md`.
  *Maintainer sign-off:* recorded in PR #157.
