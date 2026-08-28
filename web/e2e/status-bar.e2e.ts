// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The status bar (issue #287): the glass-box number is always on screen —
// ingest→pixel reads a real value after the first traced tile with the
// x-ray OFF; the cursor cell follows the mouse and copies on click.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

const cell = (page: Page, id: string) => page.locator(`#swath-status-${id} [part="value"]`);

test("ingest→pixel reads a number after the first traced tile, x-ray off; CRS names the scheme", async ({
  page,
}) => {
  await page.goto(`${DEMO_PATH}?layer=truecolor&center=-106.0,39.3&zoom=13`);
  await expect(page.locator("swath-map .swath-xray")).toHaveCount(0); // overlay off
  await expect(cell(page, "crs")).toHaveText("WebMercatorQuad · EPSG:3857");
  await expect(cell(page, "ingest")).toHaveText(/^\d+ ms$/, { timeout: 60_000 });
});

test("the cursor cell follows the mouse and copies on click", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(`${DEMO_PATH}?layer=truecolor&center=-106.0,39.3&zoom=13`);
  const map = page.locator("swath-map");
  const box = await map.boundingBox();
  if (!box) {
    throw new Error("no map box");
  }
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(cell(page, "lonlat")).toHaveText(/^-?\d+(\.\d+)?, -?\d+(\.\d+)?$/);
  const centre = await cell(page, "lonlat").textContent();
  await page.mouse.move(box.x + box.width * 0.25, box.y + box.height / 2);
  await expect(cell(page, "lonlat")).not.toHaveText(centre ?? "");
  await expect(cell(page, "zoom")).toHaveText("13");
  // Clicking the cell moves the pointer OFF the map, so the readout is the
  // centre again by the time it copies — what is copied is what it showed.
  await page.locator("#swath-status-lonlat").click();
  const copied = await page.locator("#swath-status-lonlat").getAttribute("data-copied");
  expect(copied).toMatch(/^-?\d+(\.\d+)?, -?\d+(\.\d+)?$/);
  await expect(cell(page, "lonlat")).toHaveText(copied ?? "");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(copied);
});
