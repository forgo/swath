// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The viewer e2e (issue #33): drives the demo page in real chromium
// against the real compose stack, in one of two modes (issue #103):
//
// - default (vite dev): Playwright manages the vite dev server, the page
//   lives at /demo/, and the OGC routes ride the dev proxy to :8080.
//   `just e2e-web` manages the stack (stack-up + granule drop) around it.
// - SWATH_E2E_MODE=binary (against-binary): no vite at all — the SAME
//   suites drive the production bundle the swath binary embeds and
//   serves itself on :8080 (feature `embedded-ui`), where the page lives
//   at /. Same origin as the API, so no proxy and no CORS involved.
//
// Specs read SWATH_DEMO_PATH for the page path; it defaults per mode here
// (the config module is evaluated in every worker, so specs see it too).
import { defineConfig } from "@playwright/test";

const binaryMode = process.env.SWATH_E2E_MODE === "binary";
process.env.SWATH_DEMO_PATH ??= binaryMode ? "/" : "/demo/";

export default defineConfig({
  testDir: "e2e",
  testMatch: /.*\.e2e\.ts/,
  reporter: [["list"]],
  use: {
    baseURL: binaryMode ? "http://localhost:8080" : "http://localhost:5173",
  },
  ...(binaryMode
    ? {}
    : {
        webServer: {
          command: "pnpm exec vite dev --port 5173 --strictPort",
          url: "http://localhost:5173/demo/",
          reuseExistingServer: process.env.CI === undefined,
        },
      }),
});
