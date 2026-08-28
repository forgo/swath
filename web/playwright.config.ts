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
import { defineConfig, devices } from "@playwright/test";

const binaryMode = process.env.SWATH_E2E_MODE === "binary";
process.env.SWATH_DEMO_PATH ??= binaryMode ? "/" : "/demo/";

/** The one test that restarts the server under the stack (the x-ray
 * suite's kill-and-resume). A whole-stack outage cannot share the stack
 * with sibling workers: an in-flight tile request dies with the server
 * and MapLibre never refetches a failed tile, so any concurrent test
 * mid-render (the time slider's signature loop, the cinematic landing's
 * loops since issue #211) stalls on it — and its own "counters hold
 * through the outage" baseline catches whatever the siblings traced in
 * the second before the kill. It runs alone, after everything else. */
const RESTART_TEST = /kill-and-resume/;

export default defineConfig({
  testDir: "e2e",
  testMatch: /.*\.e2e\.ts/,
  reporter: [["list"]],
  projects: [
    { name: "suites", grepInvert: RESTART_TEST },
    { name: "restart", grep: RESTART_TEST, dependencies: ["suites"] },
    // Touch parity (issue #290): the canvas smoke on an emulated phone —
    // coarse pointer, touch events, a narrow viewport. Only the canvas
    // suite opts in (everything else is desktop-only until #293).
    {
      name: "mobile",
      testMatch: /canvas\.e2e\.ts/,
      use: { ...devices["Pixel 7"] },
    },
  ],
  use: {
    baseURL: binaryMode ? "http://localhost:8080" : "http://localhost:5173",
    // The entry page (issue #108) spends 248px on the layer rail, which
    // used to shrink the map canvas below Playwright's 1280px default —
    // narrow enough that the x-ray suite's badge clicks could land under
    // the trace-feed overlay (seen on CI: "subtree intercepts pointer
    // events"). Widen by exactly the rail so the canvas keeps its
    // historical 1280x720 geometry in both modes.
    viewport: { width: 1528, height: 788 }, // shell: 44 top bar + 720 canvas + 24 status bar (#284)
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
