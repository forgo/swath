// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The Sources screen (#418, ADR 0030) against a real server: what is
// watching, read off `GET /sources` and nothing else. The stack's watch
// directory is a configured source, so the screen has a real row with a
// real measured state — no fixture of its own.
import { expect, type Page, test } from "@playwright/test";
import { DEMO_PATH, demoUrl, railMode, waitForFittedView } from "./support";

const sourcesMode = (page: Page) => railMode(page, "sources");
const screen = (page: Page) => page.locator("swath-sources");
const rows = (page: Page) => screen(page).locator('[part="row"]');

test("the sources screen is lazy, then shows a measured state per source", async ({ page }) => {
  const hits: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/sources") {
      hits.push("sources");
    }
  });
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  // Lazy by contract: the screen fetches nothing until its mode opens.
  expect(hits).toEqual([]);

  await sourcesMode(page).click();
  await expect(screen(page)).toBeVisible();
  await expect.poll(() => hits.length).toBe(1);

  // The stack watches a drop directory, so there is exactly one source and
  // its state came from the server.
  await expect(rows(page)).toHaveCount(1);
  const row = rows(page).first();
  await expect(row).toContainText("filedrop");
  await expect(row).toContainText("config");
  // The state is one of the served vocabulary — never a client guess.
  await expect(row).toContainText(/watching|no reports yet|not answering/);
  // And the target's path never reaches the page.
  await expect(screen(page)).not.toContainText("/data/drop");
});

test("the mode is a shareable artifact, like every other", async ({ page }) => {
  await page.goto(demoUrl({ view: "sources" }));
  await waitForFittedView(page);
  await expect(screen(page)).toBeVisible();
  expect(new URL(page.url()).searchParams.get("view")).toBe("sources");
});
