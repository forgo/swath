// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The viewer e2e (issue #33): drives demo/index.html in real chromium
// against the real compose stack. Playwright manages the vite dev server;
// `just e2e-web` manages the stack (stack-up + granule drop) around it.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  testMatch: /.*\.e2e\.ts/,
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:5173",
  },
  webServer: {
    command: "pnpm exec vite dev --port 5173 --strictPort",
    url: "http://localhost:5173/demo/",
    reuseExistingServer: process.env.CI === undefined,
  },
});
