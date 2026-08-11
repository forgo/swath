# ADR 0014 — Preview: openEO `POST /result` as a preview-bounded synchronous subset

**Status:** Proposed · **Date:** 2026-08-11 · **Extends:** ADR 0010 · **Source:** issue #151,
`docs/design/authoring-ux.md` §6 (option P1, selected by the maintainer 2026-08-11)

## Context

The #151 authoring-UX design note classified every reachable bad state of the authoring panel
(§2). Its Model B canvas makes B1–B10 unconstructible client-side, but B11 — a graph that is
*valid and wrong* (swapped `nir`/`red`, a washed-out input range, a mismatched colormap) — is
unreachable by any validator: no schema, stage table, or server diagnostic can know what the
user meant to see. The only countermeasure is **seeing the draft before publishing**.

The pieces already exist. `POST /services` compiles the same graph through the #32 compiler,
and the tile path renders the result; only the combination — *render a draft graph without
persisting a service* — has no route. The design note weighed three ways to add one:

- **Zero-server bridge (rejected):** publish, fetch a tile, delete. Works today but churns the
  services list and the catalog (`swath:layers` writes on every debounce), races the layer-list
  refresh events, and makes "preview" a side-effectful lie.
- **P2 — bespoke `POST /preview` (declined):** smaller to specify, but it is exactly the
  "minimal control-plane wrapper, no discoverability" shape ADR 0010 already weighed and
  declined for the authoring surface. A second, nonstandard verb beside a standard one that
  fits is hard to justify, and no openEO client would ever find it.
- **P1 — openEO `POST /result`, preview-bounded (selected):** the standard synchronous-execute
  endpoint, admitted to the profile the same way everything else in ADR 0010 was — bounded,
  with the narrowing declared honestly.

ADR 0010 fixed the public surface by decision ("Jobs, batch processing, user-defined processes,
and file management are explicitly out of scope until demanded"), so growing it is an ADR-level
act, not a UI-side call. This ADR is that act. Demand has now appeared: B11 and the
preview-before-publish half of #151.

## Decision

Extend the bounded openEO profile (ADR 0010) with **`POST /result`, implemented as a
preview-bounded synchronous subset**:

- **Request** as in the openEO API 1.2.0 spec: `{"process": {"process_graph": …}}`. The graph
  is compiled by the **same #32 compiler path `POST /services` uses** — same narrowing, same
  typed diagnostics — so the preview pixels are exactly what publishing the same graph would
  serve. Nothing is persisted: no service, no `swath:layers` write, no catalog or event
  side effects of any kind.
- **Response**: `image/png` — **one small overview-backed render** covering the graph's
  `spatial_extent` (or the referenced collection's extent when null), *not* the spec's default
  of the full extent at native resolution. That default is unbounded work, and Swath's engine
  is a tiler; the preview render is sized so the planner's cost model
  (`max_estimated_live_bytes`, materialization-planner design §1) admits it from overviews —
  a preview is exactly the workload overviews exist for.
- **Strict budget, refusal over degradation.** A single bounded pixel budget (one small
  render), enforced through the planner's byte ceiling as a hard time/pixel proxy. When the
  estimate exceeds the budget and nothing cheaper can serve, the server **refuses** with the
  spec's `ProcessGraphComplexity` error — it never silently downgrades to a different extent
  than requested and never performs an unbounded full-resolution read.
- **No general synchronous-processing claim.** The capabilities document lists `POST /result`
  because it exists, with the preview-grade narrowing stated honestly in its description,
  exactly as the profile's other bounded entries are; general openEO synchronous-processing
  conformance is **not** claimed, just as ADR 0010 declines the general conformance class
  while auth is absent. Honesty over reach: the profile says what the endpoint does, not what
  the spec allows it to do.
- **Rate/debounce behavior stays a UI concern.** The endpoint is stateless per request; the
  authoring canvas owns its debounce, and no server-side session or throttling machinery is
  introduced by this decision.

This is the bounded-profile pattern of ADR 0010 extended, not broken: a standard endpoint,
admitted at exactly the width the engine honestly supports, discoverable by real openEO
clients, with the narrowing declared where clients look for it.

## Consequences

- **API surface**: the openEO capabilities `endpoints` list (and the `.well-known` document's
  view of it) gains `POST /result`. `/conformance` continues to list only the OGC Tiles
  classes actually met; no openEO conformance class is added — per ADR 0010 the general class
  is not claimed, and per-endpoint honesty lives in the capabilities document.
- **Error taxonomy** (all from the spec's pinned `errors.json` registry, the ADR 0010 rule):
  compile failures reuse the existing mapped diagnostics unchanged (`ProcessGraphInvalid`,
  `ProcessUnsupported`, `ProcessParameterInvalid`, band/collection errors, …) — identical
  codes for identical graphs whether submitted to `/result` or `/services`; budget refusals
  answer `ProcessGraphComplexity` (the spec's code for requests too heavy for synchronous
  processing). No new bespoke error vocabulary.
- **Runtime shape**: no new one. A preview is one bounded overview-backed render on the
  existing inline path; ADR 0012's load evidence (control plane ≤1.44 ms p99 under a
  sustained mixed live-render storm, even pinned to 2 CPUs) already covers this load class.
- **Validation** the #27 way: response and error shapes checked against the pinned openEO
  1.2.0 spec in `crates/swath-api/tests/data/openeo/`; the preview/publish equivalence
  ("same compiler path") is provable byte-for-byte against the built-in NDVI golden, the same
  instrument ADR 0010 used for the authoring loop.
- **UI**: the always-valid canvas (the #151 Model B selection) gains its debounced preview
  tile beside the narrative sentence — B11's only countermeasure — with no bespoke server
  vocabulary. Implementation is a follow-up issue, not this ADR.
- Jobs, batch processing, `POST /validation`, user-defined processes, and file management
  remain out of scope exactly as ADR 0010 left them; this ADR admits one endpoint, bounded,
  and nothing else.

## Reopen / supersede conditions

Supersede this ADR when any of the following demand appears:

- **Genuine synchronous processing** — a client needs `POST /result` semantics beyond one
  small preview render (full extent, native resolution, non-PNG formats): that is a
  jobs/batch-shaped question and must not be answered by quietly widening the preview budget.
- **The preview bound proves wrong in practice** — real drafts routinely refused
  (`ProcessGraphComplexity` on reasonable authoring-sized requests) or the inline path shows
  the ADR 0012 reopen signals under preview load (`/healthz` p99 > 50 ms, dropped SSE): the
  budget, or the inline placement, gets redecided with evidence.
- **A server-side validation surface lands** — if a standard `POST /validation` is added (the
  Model B stage-table follow-up flagged in the design note §4), the division of labor between
  it and this endpoint should be restated in that ADR.
