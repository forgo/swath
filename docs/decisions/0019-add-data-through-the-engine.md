# ADR 0019 — Add-data goes through the engine; client-side COG rendering rejected

**Status:** Accepted · **Date:** 2026-08-18 · **Refs:** issue #197 (the add-data panel), #196
(dataset-creation API), #198 (read-only slice), ADR 0005 (frontend), ADR 0010 (openEO
authoring), ADR 0014 (preview)

## Context

The add-data panel is the dataset-creation API's UI face: paste a COG or STAC-Item link (or
drop a file in local mode) and see it on the map. The tempting shortcut is to render the
pasted COG **client-side** — geotiff.js/WebGL readers can paint a pasted URL onto MapLibre in
seconds, with no registration round-trip at all, and several mature viewers do exactly that.

But Swath's whole claim (REQUIREMENTS.md north star; the glass-box principle) is that a pixel
on screen is a pixel the *engine* served: planned by the planner, traced end to end, cached by
content, budgeted, and identical to what any OGC/openEO client would fetch from the same
server. A client-rendered preview is a second, untraced render path — its pixels bypass the
planner and the x-ray, can disagree with what serving will later produce (different
resampling, different nodata handling), and teach the user a flow ("see it without adding
it") the product then cannot honor from any other client.

## Decision

**Everything the panel shows goes through the engine.** The flow is registration, not
rendering:

- Paste → the panel fetches a `.json` link in-browser (the server never fetches URLs — the
  #196 SSRF fence) and registers through `POST /datasets` + `POST /datasets/{id}/granules`,
  where asset headers are validated by the same source stack tiles read.
- The visual result is a **quick-look openEO service** (`POST /services` — register-then-
  author, deliberately reusing ADR 0010's surface rather than a parallel vocabulary): RGB
  from three bands, gray from one, scaled to a user-stated brightest value. Its tiles serve
  and trace like every other layer.
- Local-mode file drop is `PUT /uploads/{filename}` into the serving object store (mounted
  only for writable catalog serving over a local store root), then the same registration.
- The panel exists exactly where the capabilities document (`GET /`, #198-filtered) advertises
  `POST /datasets`; the drop zone exactly where it advertises the upload route. Never probed,
  never hardcoded.

**Client-side COG rendering is rejected** — recorded here so the shortcut is not re-litigated
per feature. This includes "just a preview while registering": preview needs go through the
engine too (ADR 0014's `POST /result`).

## Consequences

- Seeing pasted data requires a writable server; the read-only demo shows a plain "viewing
  only" note instead of the form. That is the honest trade: no untraced pixels, ever.
- No geotiff/WebGL decode dependency enters the web bundle (ADR 0005's no-framework budget
  holds), and the e2e can assert the paste flow end to end against the real serve path (tile
  bytes + `X-Swath-Trace`).
- A pasted remote COG must be readable by the *server's* source stack; a URL only the browser
  could reach fails header validation with an RFC 7807 problem the panel maps onto the link
  field — a truthful refusal, not a client-side workaround.

## Reopen / supersede conditions

- An offline/field-kit requirement (no server writable anywhere) would need a client render
  path; that is a new product claim and a new ADR, not a widening of this one.
