// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Dev server for the demo page (demo/index.html). The Swath API routes are
// proxied to the compose stack so the page and the tiles share an origin —
// the API doesn't serve CORS headers (a deliberate not-yet; the component
// itself takes any base URL via its `server` attribute).
import { defineConfig } from "vite";

const SWATH = "http://localhost:8080";

export default defineConfig({
  optimizeDeps: { exclude: ["maplibre-gl"] },
  server: {
    proxy: {
      "/tilesets": SWATH,
      "/tiles": SWATH,
      "/conformance": SWATH,
      "/healthz": SWATH,
      "/traces": SWATH,
    },
  },
});
