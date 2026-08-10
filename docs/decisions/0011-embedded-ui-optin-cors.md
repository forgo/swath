# ADR 0011 — UI ships inside the binary; CORS is opt-in, default off

**Status:** Accepted · **Date:** 2026-08-10 · **Refs:** issue #103 (M4), ADR 0005 (frontend), ADR 0002 (single binary)

## Context

Through M3 the web viewer existed only as a vite-dev-served demo page proxying the OGC routes to
the compose stack, and the API served no CORS headers anywhere (the distribution audit found the
only written trace of that choice was a justfile comment). Shipping M4's container raised both
questions at once: how does the UI reach users of the single deployable, and what is the
cross-origin story when it doesn't?

Options for shipping the UI: (a) a separate static-file deployment (a second artifact and origin,
against ADR 0002's single-binary story, and it forces CORS on by default); (b) a `--ui-dir`
runtime path with the image copying the bundle (two things to keep in sync at deploy time);
(c) compile-time embedding of the production bundle.

## Decision

**Embed the production web bundle in the binary at compile time**, behind a cargo feature
`embedded-ui` that is **on by default** (like the statically bundled libhdf5): `swath serve`
alone serves the UI. Mechanics, chosen so the standards surface is untouched:

- `pnpm build` (vite) produces the hashed bundle in `web/dist`; `just build-full` (and the
  Dockerfile's Node stage) runs it before the cargo build. swath-cli's build script stages
  `web/dist` into `OUT_DIR` for `include_dir!`, so a dist-less checkout still compiles — the
  binary then honestly serves no UI.
- Browsers (an `Accept` listing `text/html`) get `index.html` at `GET /`; API clients keep the
  OGC/openEO JSON landing page byte-identical. Assets serve from the router **fallback**, so API
  routes structurally outrank any bundle file; there is no SPA rewrite (unknown paths stay plain
  404). A route-table test pins the priority and the bundle/route disjointness.

**CORS is opt-in and off by default.** One origin serves both UI and tiles in the shipped
configuration, and the dev workflow proxies through vite — no cross-origin request exists, so
none is advertised. Deployments that do serve a browser frontend from another origin opt in with
an explicit allowlist — `cors-allowed-origins` in the config file, `--cors-allowed-origins`, or
`SWATH_CORS_ALLOWED_ORIGINS`; the single value `*` allows any origin (cross-origin dev). The
layer (tower-http) echoes only listed origins, mirrors requested methods/headers, and never
enables credentials; with no origins configured, no layer exists and responses are
byte-identical to before.

## Consequences

- One artifact demos the whole product: `swath serve --fixtures` is layers + UI at one origin;
  the container needs no frontend sidecar. The binary grows by roughly the bundle size (~1 MB —
  maplibre dominates).
- The Playwright viewer suites run twice (`just e2e-web`): vite-dev mode and against-binary
  mode, so the embedded path is regression-tested with the same assertions as dev.
- Disabling the UI is a build decision (`--no-default-features`), not a runtime flag — matching
  the hdf5 precedent and keeping the serve surface free of a knob nobody tunes per-deploy.
- CORS stays a deliberate, documented operator action; the swath-api crate docs carry the
  decision beside the code (`swath_api::cors`).
