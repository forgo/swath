// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The compare swipe, layer-vs-layer (issue #210), in both e2e modes
// (vite dev proxy and SWATH_E2E_MODE=binary — playwright.config.ts):
// a `cl=` deep link splits two layers of the Colorado fixture across
// one handle, and the pasted URL survives byte-for-byte (the issue
// #108 contract).
//
// The DATE-vs-date compare tests live in time-slider.e2e.ts, not here,
// deliberately: they render Park Fire frames, and the fire layer's
// trace stream is owned by that file — its signature-loop test asserts
// per-frame analytics ("this frame: N live, 0 cached") that any
// cross-worker fire render would race (Playwright runs FILES in
// parallel workers; tests within one file are serialized). This file
// only touches the Colorado layers (ndvi/truecolor), whose suites
// (x-ray, landing) assert against the same shared stream they receive —
// parallel-safe by construction.
//
// Zoom convention (one shared stack serves every suite in both modes):
// style zoom 10, display tiles z11 — below everything the x-ray suite
// renders, so its cold-cache premises stay untouched.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The Colorado fixture viewpoint shared by ndvi and truecolor. */
const CO_CENTER = "-105.4475,39.265";
const CO_ZOOM = "10";

/** Waits until the map is up and settled on its deep-linked view. */
async function waitForSettledView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document.querySelector("swath-map") as {
      map?: { loaded(): boolean; areTilesLoaded(): boolean; getZoom(): number };
    } | null;
    const map = el?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

const handle = (page: Page) => page.locator("swath-map .swath-map-compare-handle");

test("layer-vs-layer: cl puts the second layer's tiles on the right side", async ({ page }) => {
  const deepLink = `${DEMO_PATH}?layer=ndvi&cl=truecolor&center=${CO_CENTER}&zoom=${CO_ZOOM}`;
  const ndviTile = page.waitForRequest((request) =>
    request.url().includes("/tilesets/ndvi/tiles/"),
  );
  const truecolorTile = page.waitForRequest((request) =>
    request.url().includes("/tilesets/truecolor/tiles/"),
  );
  await page.goto(deepLink);
  await ndviTile;
  await truecolorTile;
  await waitForSettledView(page);

  await expect(handle(page)).toBeVisible();
  await expect(handle(page)).toHaveAttribute("data-mode", "layer");
  await expect(page.locator('.swath-map-compare-label[data-side="left"]')).toHaveText("ndvi");
  await expect(page.locator('.swath-map-compare-label[data-side="right"]')).toHaveText("truecolor");
  // Byte-stable, layer edition.
  expect(page.url()).toBe(new URL(deepLink, page.url()).toString());

  // Moving the handle is a user interaction: `swipe` joins the share
  // link (keyboard here; the drag path is covered in time-slider's
  // date-mode test and the component unit tests).
  await handle(page).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(handle(page)).toHaveAttribute("data-fraction", "0.48");
  await expect(page).toHaveURL(/[?&]swipe=0\.48/);
});
