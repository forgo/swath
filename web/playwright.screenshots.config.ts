// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The docs screenshot capture (issue #112, `just screenshots`): a
// dedicated Playwright config so the capture suite never mixes into the
// e2e test set (playwright.config.ts matches *.e2e.ts only). Geometry is
// pinned here — fixed viewport and DPR — because the shots are committed
// artifacts a second capture run must reproduce within a perceptual-diff
// policy (tests/screenshots/verify_stable.py); anything geometric that
// drifted would fail that gate as a dimension mismatch.
//
// Capture always runs through the vite dev server (the OGC routes ride
// the dev proxy to the compose stack on :8080, exactly like `just demo`);
// `just screenshots` owns the stack lifecycle around it.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "screenshots",
  testMatch: /capture\.ts/,
  reporter: [["list"]],
  // The shot sequence is stateful on purpose (each capture run must
  // replay the identical tile-request history from a cold cache, so
  // decisions and analytics counters reproduce): one worker, in order,
  // no retries — a mid-sequence retry would replay against a warm cache.
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 120_000,
  use: {
    baseURL: "http://localhost:5173",
    // Pinned shot geometry: rail (248px) + a 1280px-wide canvas, DPR 1.
    viewport: { width: 1528, height: 860 },
    deviceScaleFactor: 1,
  },
  webServer: {
    command: "pnpm exec vite dev --port 5173 --strictPort",
    url: "http://localhost:5173/demo/",
    reuseExistingServer: process.env.CI === undefined,
  },
});
