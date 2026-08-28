// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The phone tier (issue #293, 393×852 with touch): the tab bar, the mode
// sheet with 40/90 snaps, the dock chip, the toggles in the dock, author
// entry — the same product, both modes.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

test.skip(({ isMobile }) => !isMobile, "the phone tier runs on the mobile project");

const tab = (page: Page, mode: string) =>
  page.locator(`swath-rail [part="item"][data-mode="${mode}"]`);
const sheet = (page: Page) => page.locator("#swath-rail-drawer");

async function landed(page: Page): Promise<void> {
  await expect(page.locator("swath-shell")).toHaveAttribute("tier", "phone");
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();
}

test("landing: the tab bar sits at the bottom, the status chip in the dock, the map paints", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await landed(page);
  const rail = await page.locator("swath-rail").boundingBox();
  const map = await page.locator("swath-map").boundingBox();
  expect(rail && map && rail.y >= map.y + map.height - 1).toBe(true); // below the map
  await expect(page.locator("swath-hud-dock swath-status-bar[chip]")).toBeVisible();
  await expect(page.locator("#swath-status-ingest")).toBeVisible();
  await expect(page.locator("#swath-status-crs")).toBeHidden(); // one chip, not four
});

test("layers: the Layers tab opens the sheet; a row switches the layer; the 90% snap makes the map inert", async ({
  page,
}) => {
  await page.goto(`${DEMO_PATH}?layer=truecolor`);
  await landed(page);
  await expect(sheet(page)).toHaveAttribute("presentation", "bottom");
  await expect(sheet(page)).toHaveAttribute("open", "");
  const row = page.locator('swath-layer-item[data-layer="ndvi"] [part="row"]');
  await row.tap();
  await expect(row).toHaveAttribute("aria-pressed", "true");
  await expect(page).toHaveURL(/[?&]layer=ndvi/);
  await page.locator("#swath-rail-drawer").evaluate((el) => {
    (el as unknown as { snapIndex: number }).snapIndex = 1;
  });
  await expect(page.locator("swath-map")).toHaveAttribute("inert", "");
  await tab(page, "layers").tap(); // the active tab toggles the sheet away
  await expect(sheet(page)).not.toHaveAttribute("open", "");
  await expect(page.locator("swath-map")).not.toHaveAttribute("inert", "");
});

test("data + x-ray + author entry from the tab bar and the dock", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await landed(page);
  await tab(page, "data").tap();
  await expect(page).toHaveURL(/[?&]view=data/);
  await expect(page.locator("#swath-rail-drawer swath-catalog")).toBeVisible();
  await page.locator(".swath-map-xray-toggle button").tap();
  await expect(page.locator(".swath-xray")).toBeAttached();
  await expect(page).toHaveURL(/xray/);
  await tab(page, "author").tap();
  await expect(page.locator("#swath-author-dock")).toHaveAttribute("open", "");
  await expect(page).toHaveURL(/[?&]view=author/);
});

test("author: the pipeline is a canvas — a tap on a node's chip selects its step (#299)", async ({
  page,
}) => {
  // The panel's toggle lives in the rail sheet, which the author dock
  // covers on a phone: open the panel from Layers mode, then switch.
  await page.goto(DEMO_PATH);
  await landed(page);
  await page.locator("swath-authoring-panel .swath-authoring-toggle").tap();
  await tab(page, "author").tap();
  await expect(page.locator("#swath-author-dock")).toHaveAttribute("open", "");
  const canvas = page.locator("#swath-author-dock swath-canvas.swath-authoring-canvas");
  await expect(canvas).toBeVisible();
  await expect(canvas.locator("swath-canvas-node")).toHaveCount(2); // Load, Output
  const output = canvas.locator('.swath-authoring-chip[data-chip="s2"]');
  await output.tap();
  await expect(output).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('#swath-author-inspector [data-step="s2"]')).toBeAttached();
  await expect(page).toHaveURL(/[?&]sel=s2/);
});
