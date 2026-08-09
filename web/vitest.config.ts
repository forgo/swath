// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Browser Mode is non-negotiable for this codebase: components wrap MapLibre GL
// (WebGL) and real Custom Elements — jsdom/happy-dom can't represent either
// (ENGINEERING.md §3).
import { playwright } from "@vitest/browser-playwright";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // MapLibre spawns its worker from a sibling module the dep optimizer
  // doesn't crawl; excluding it avoids a missing-file warning per run.
  optimizeDeps: { exclude: ["maplibre-gl"] },
  test: {
    browser: {
      enabled: true,
      provider: playwright(),
      headless: true,
      screenshotFailures: false,
      instances: [{ browser: "chromium" }],
    },
  },
});
