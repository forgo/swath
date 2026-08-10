# Swath web

Vanilla Web Components + MapLibre GL (ADR 0005), no framework. `<swath-map>` is the viewer;
`demo/index.html` *is* the app: the entry page (issue #108) pairs the map with
`<swath-layer-panel>` (the layer browser) and `demo/main.ts` wires the view-state semantics
from `src/view-state.ts` — the URL is the shareable representation of layer/center/zoom/x-ray,
localStorage restores the last session on a paramless visit, URL params beat storage, and
deep-link URLs are never rewritten on load. The production bundle ships inside the `swath`
binary (ADR 0011).

## Dev workflow (vite dev + proxy) — unchanged

```sh
just setup-web            # deps + Playwright chromium
docker compose up         # or: cargo run -p swath-cli -- serve --fixtures --bind 0.0.0.0:8080
cd web && pnpm exec vite dev
```

The page lives at `http://localhost:5173/demo/`. Vite proxies the API routes (`/tilesets`,
`/tiles`, `/conformance`, `/healthz`, `/traces`) to `:8080`, so page and tiles share an origin
and no CORS is involved. To develop cross-origin instead (no proxy), start the server with
`--cors-allowed-origins http://localhost:5173` (or `*`) — CORS is opt-in and off by default
(ADR 0011).

## Production build

```sh
pnpm run build            # vite build: demo/ -> web/dist (hashed assets)
just build-full           # the bundle + the release binary embedding it
```

`vite build` shifts the root to `demo/`, so `index.html` lands at `/` with hashed assets under
`/assets/`. The binary (cargo feature `embedded-ui`, default on) embeds `web/dist` at compile
time: `swath serve --fixtures` then serves the UI at `/` — browsers get `index.html`, API
clients keep the JSON landing page, and API routes always win over asset paths. The page talks
to its own origin; nothing about the dev proxy is baked in.

## Tests

```sh
just lint-web             # biome + tsc
just test-web             # vitest browser mode (real chromium)
just e2e-web              # full stack + Playwright, BOTH modes:
                          #   1. vite dev + proxy (page at /demo/)
                          #   2. SWATH_E2E_MODE=binary — the same specs against
                          #      the binary-served embedded UI (page at /)
```

The Playwright mode switch lives in `playwright.config.ts`; specs read `SWATH_DEMO_PATH` for
the page path, so both modes run the identical suites.
