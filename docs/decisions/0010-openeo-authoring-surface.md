# ADR 0010 — Authoring surface: native openEO API, bounded profile

**Status:** Accepted · **Date:** 2026-08-09 · **Resolves:** CHARTER.md §13 "openEO surface"

## Context

CHARTER §13 left the derived-product authoring surface open: adopt an existing openEO backend /
Processes engine vs. implement a minimal Processes subset first, with a bias toward composing. The
process compiler (#32) already lowers standard openEO process-graph JSON to the Render IR, the
catalog (#30) persists serving definitions as `swath:layers`, and catalog-backed serving (#31)
renders them live — the wedge needs only a standards-shaped front door: submit a graph, register
the derived layer, serve tiles (REQUIREMENTS.md R3, one motion).

Options weighed: (a) a minimal control-plane wrapper (Swath-shaped POST, no discoverability);
(b) adopting an existing openEO backend implementation (a second engine beside the tiler,
violating the pure-core shape of ADR 0002); (c) implementing the openEO API natively, bounded to
what actually exists.

## Decision

Implement the **openEO API (1.2.0) natively, at a bounded profile** — chosen by the maintainer
over the minimal-wrapper lean: real openEO clients should be able to discover Swath, read its
collections and processes, and publish a process graph as a live tiled layer. The profile is
exactly:

- `GET /.well-known/openeo` and `GET /` capabilities: `api_version` 1.2.0, an `endpoints` list of
  **only what exists**, no billing. The capabilities document doubles as the OGC API landing page
  (one root, both vocabularies).
- `GET /collections` and `GET /collections/{id}`: openEO collection metadata derived from catalog
  Datasets via the #30 STAC converters (openEO collections *are* STAC-based; STAC stays hidden
  from Swath's own control plane per R2, but openEO clients speak STAC — that is the standard).
- `GET /processes`: the #32 compiler subset, served as the pinned official openeo-processes 1.2.0
  definitions with Swath's parameter narrowing noted honestly in the descriptions.
- `GET /service_types` and secondary-service CRUD (`POST/GET /services`, `GET/DELETE
  /services/{id}`) with a single service type, **`xyz`**: POST validates the graph through the #32
  compiler against the referenced collection's bands, persists the derived layer on the Dataset
  (`swath:layers`), and answers 201 with the service's tile URL — the OGC tiles endpoint. openEO
  graph in, live XYZ out. `PATCH /services/{id}` is deliberately omitted (delete + re-create
  covers v0).
- Compiler diagnostics map onto the standardized openEO error format (`code`/`message`, codes from
  the spec's `errors.json`).
- **No auth** (openEO conformance requires the authentication endpoints; their absence is declared
  honestly — the general openEO conformance class is *not* claimed). Jobs, batch processing,
  user-defined processes, and file management are explicitly out of scope until demanded.

## Consequences

- R3 becomes real for external tooling: any openEO client can author against Swath within the
  profile; the authoring loop (POST a graph → tile from the returned URL) is proven end-to-end
  against the built-in NDVI golden, byte-for-byte — same compiler, same serve path.
- The surface is validated the #27 way: response schemas from the pinned openEO API 1.2.0 spec
  (`crates/swath-api/tests/data/openeo/`), error codes pinned against the spec registry.
- Honesty over reach: `/conformance` continues to list only the OGC Tiles classes actually met;
  the openEO capabilities document lists only implemented endpoints. Growing toward fuller openEO
  conformance (auth first, per the Phase-3 charter) extends this profile rather than replacing it.
- Adopting a third-party openEO backend is off the table while the pure-Rust single-binary shape
  (ADR 0002) holds — the compiler *is* the engine; this ADR records that composition happens at
  the standards boundary, not by embedding someone else's runtime.
