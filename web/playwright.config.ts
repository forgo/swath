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
//
// The docs screenshot capture (issue #112, `just screenshots`) is a fourth
// project of this same config (#349), present only when SWATH_SHOTS_DIR is
// set: `just e2e-web` never runs it, and `just screenshots` runs it alone
// (`--project screenshots`). Its geometry — a fixed viewport and DPR — is
// pinned because the shots are committed artifacts a second capture run
// must reproduce within a perceptual-diff policy
// (tests/screenshots/verify_stable.py). Capture always runs through the
// vite dev server, exactly like `just demo`.
import { defineConfig, devices } from "@playwright/test";

const binaryMode = process.env.SWATH_E2E_MODE === "binary";
process.env.SWATH_DEMO_PATH ??= binaryMode ? "/" : "/demo/";
const screenshots = process.env.SWATH_SHOTS_DIR !== undefined;

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
  // Assertions wait 15s, not Playwright's 5s (#447).
  //
  // Five tests across four files had been failing only in the concurrent
  // run — always green in isolation and on rerun — and produced two wrong
  // diagnoses (a MapLibre resize theory, #461; a compose-layout theory,
  // #463) before being measured. Measured, on a fresh stack each time:
  //
  //   1 worker                   66 passed
  //   2 workers                  66 passed
  //   6 workers (this machine)    2 failed  — layer-list, palette
  //   6 workers + 20s timeout    66 passed
  //
  // and the swath container logged no error in the failing runs. So it is
  // neither a race nor a dropped interaction: a layer switch simply takes
  // longer than 5s when N browsers and the compose stack share the CPU.
  // CI sees the same thing for a different reason — one worker, but only
  // two cores shared with swath + pgstac + MinIO.
  //
  // Raising the ceiling costs nothing while tests pass; it only lengthens
  // how long a genuinely failing assertion waits before reporting.
  expect: { timeout: 15_000 },
  projects: [
    { name: "suites", grepInvert: RESTART_TEST },
    { name: "restart", grep: RESTART_TEST, dependencies: ["suites"] },
    // Touch parity (issue #290): the canvas smoke on an emulated phone —
    // coarse pointer, touch events, a narrow viewport. Only the canvas
    // suite opts in (everything else is desktop-only until #293).
    {
      name: "mobile",
      testMatch: /(canvas|mobile)\.e2e\.ts/,
      use: {
        ...devices["Pixel 7"],
        viewport: { width: 393, height: 852 },
        hasTouch: true,
        isMobile: true,
      },
    },
    ...(screenshots
      ? [
          {
            name: "screenshots",
            testDir: "screenshots",
            testMatch: /capture\.ts/,
            // The shot sequence is stateful on purpose (each capture run
            // must replay the identical tile-request history from a cold
            // cache, so decisions and analytics counters reproduce): in
            // order, no retries — a mid-sequence retry would replay against
            // a warm cache. `just screenshots` passes `--workers 1`.
            fullyParallel: false,
            retries: 0,
            timeout: 120_000,
            use: {
              // Pinned shot geometry: rail (56px icon strip + 248px panel,
              // #398) + a 1280px-wide canvas, DPR 1.
              viewport: { width: 1584, height: 928 }, // shell: 44 top bar + 860 canvas + 24 status bar (#284)
              deviceScaleFactor: 1,
            },
          },
        ]
      : []),
  ],
  use: {
    baseURL: binaryMode ? "http://localhost:8080" : "http://localhost:5173",
    // The entry page (issue #108) spends 304px on the layer rail — a 56px
    // icon strip beside a 248px panel since #398 — which
    // used to shrink the map canvas below Playwright's 1280px default —
    // narrow enough that the x-ray suite's badge clicks could land under
    // the trace-feed overlay (seen on CI: "subtree intercepts pointer
    // events"). Widen by exactly the rail so the canvas keeps its
    // historical 1280x720 geometry in both modes.
    viewport: { width: 1584, height: 788 }, // shell: 44 top bar + 720 canvas + 24 status bar (#284)
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
