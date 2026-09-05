// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Two modes, one config (issue #103):
//
// - `vite dev`: dev server for the demo page (demo/index.html at /demo/).
//   The Swath API routes are proxied to the stack on :8080 so the page and
//   the tiles share an origin — cross-origin dev without the proxy instead
//   uses the server's opt-in CORS (`--cors-allowed-origins`, ADR 0011).
// - `vite build`: the production bundle. Root shifts to demo/ so the demo
//   page IS the app (index.html at /, hashed assets under /assets/), built
//   into web/dist — the tree `swath serve` embeds and serves same-origin
//   (crates/swath-cli, feature `embedded-ui`). No proxy assumptions are
//   baked in: the page talks to its own origin, wherever it is served from.
import { defineConfig } from "vite";

const SWATH = "http://localhost:8080";

export default defineConfig(({ command }) => ({
  ...(command === "build"
    ? {
        root: "demo",
        build: { outDir: "../dist", emptyOutDir: true },
      }
    : {}),
  optimizeDeps: { exclude: ["maplibre-gl"] },
  server: {
    proxy: {
      "/tilesets": SWATH,
      "/tiles": SWATH,
      "/conformance": SWATH,
      "/collections": SWATH,
      "/datasets": SWATH,
      "/healthz": SWATH,
      "/traces": SWATH,
      "/processes": SWATH,
      "/result": SWATH,
      "/services": SWATH,
      "/sources": SWATH,
      "/uploads": SWATH,
      // Exactly `/` (a RegExp key): the landing/capabilities document the
      // add-data panel reads (#197). The demo page itself lives at /demo/,
      // so nothing else matches.
      "^/$": SWATH,
    },
  },
}));
