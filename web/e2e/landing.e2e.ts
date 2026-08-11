// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The entry experience (issue #108), against the real stack in both
// modes (vite dev at /demo/, against-binary at / — playwright.config.ts):
//
// - `/` with no params shows the layer browser and loads a sensible
//   default, WITHOUT rewriting the URL.
// - Selecting a layer updates the URL; that URL alone (incognito: a
//   fresh context with empty storage) reproduces the same view.
// - Deep-link URLs are byte-stable: params applied, URL untouched.
// - URL params beat localStorage — THE precedence rule.
// - localStorage restores the last layer/viewport on a paramless visit.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The app's storage key (src/view-state.ts). */
const STORAGE_KEY = "swath.view-state.v1";

/** Waits for the zero-config bounds fit to land (same discriminator as
 * the x-ray suite: the fitted footprint view is deep, the boot view is
 * zoom 1). */
async function waitForFittedView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document.querySelector("swath-map") as {
      map?: { loaded(): boolean; areTilesLoaded(): boolean; getZoom(): number };
    } | null;
    const map = el?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

/** The map's current view, read off the live MapLibre instance. */
async function mapView(page: Page): Promise<{ lng: number; lat: number; zoom: number }> {
  return await page.evaluate(() => {
    const el = document.querySelector("swath-map") as {
      map?: { getCenter(): { lng: number; lat: number }; getZoom(): number };
    } | null;
    const map = el?.map;
    if (!map) {
      throw new Error("swath-map has no map instance");
    }
    const center = map.getCenter();
    return { lng: center.lng, lat: center.lat, zoom: map.getZoom() };
  });
}

function panelButton(page: Page, layerId: string) {
  return page.locator(`swath-layer-panel button[data-layer="${layerId}"]`);
}

test("paramless / shows the layer browser, loads a default, URL untouched", async ({ page }) => {
  const tile = page.waitForResponse(
    (response) => response.url().includes("/tilesets/ndvi/tiles/") && response.status() === 200,
  );
  await page.goto(DEMO_PATH);

  // The layer browser lists built-in layers with title and id; the
  // default (first tileset, id order: ndvi) is marked as viewed.
  await expect(panelButton(page, "ndvi")).toBeVisible();
  await expect(panelButton(page, "truecolor")).toBeVisible();
  await expect(panelButton(page, "ndvi")).toContainText("HLS NDVI");
  await expect(panelButton(page, "ndvi")).toHaveAttribute("aria-pressed", "true");
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "false");

  // A sensible default actually loads: real ndvi tiles, fitted view.
  await tile;
  await waitForFittedView(page);

  // No interaction happened, so the bare URL stays bare — byte-stable.
  expect(new URL(page.url()).search).toBe("");
});

test("selecting a layer updates the URL; the URL alone reproduces the view", async ({
  page,
  browser,
}) => {
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);

  await panelButton(page, "truecolor").click();
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  await expect(panelButton(page, "ndvi")).toHaveAttribute("aria-pressed", "false");
  await expect(page).toHaveURL(/\?layer=truecolor&center=[-\d.,]+&zoom=[\d.]+$/);

  const shareUrl = page.url();
  const view = await mapView(page);

  // "Incognito": a brand-new context — empty localStorage, no session.
  const incognito = await browser.newContext();
  try {
    const copy = await incognito.newPage();
    const tile = copy.waitForResponse(
      (response) =>
        response.url().includes("/tilesets/truecolor/tiles/") && response.status() === 200,
    );
    await copy.goto(shareUrl);
    await expect(panelButton(copy, "truecolor")).toHaveAttribute("aria-pressed", "true");
    await tile;
    const copied = await mapView(copy);
    expect(copied.lng).toBeCloseTo(view.lng, 4);
    expect(copied.lat).toBeCloseTo(view.lat, 4);
    expect(copied.zoom).toBeCloseTo(view.zoom, 1);
    // And the share link itself was not rewritten by the visit.
    expect(copy.url()).toBe(shareUrl);
  } finally {
    await incognito.close();
  }
});

test("URL params beat storage, and the deep link stays byte-stable", async ({ page }) => {
  // A stored last session pointing somewhere else entirely.
  await page.addInitScript(
    ([key, value]) => {
      window.localStorage.setItem(String(key), String(value));
    },
    [STORAGE_KEY, JSON.stringify({ layer: "truecolor", center: [8.5, 47.4], zoom: 6, xray: true })],
  );

  const deepLink = `${DEMO_PATH}?layer=ndvi&center=-106.05,39.35&zoom=12`;
  await page.goto(deepLink);

  // The URL wins on every field: layer, viewport, and x-ray (off — the
  // stored true must not leak into a shared link's view).
  await expect(panelButton(page, "ndvi")).toHaveAttribute("aria-pressed", "true");
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "false");
  const view = await mapView(page);
  expect(view.lng).toBeCloseTo(-106.05, 4);
  expect(view.lat).toBeCloseTo(39.35, 4);
  expect(view.zoom).toBeCloseTo(12, 2);
  await expect(page.getByRole("button", { name: "Toggle x-ray overlay" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );

  // Byte-stable: the pasted URL survives the load byte-for-byte.
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();
  expect(page.url()).toBe(new URL(deepLink, page.url()).toString());
});

test("localStorage restores the last layer and viewport on a paramless visit", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);

  // A session: switch layers (persisted as it happens).
  await panelButton(page, "truecolor").click();
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  await expect(page).toHaveURL(/layer=truecolor/);
  const view = await mapView(page);

  // A later paramless visit resumes exactly there.
  await page.goto(DEMO_PATH);
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  const restored = await mapView(page);
  expect(restored.lng).toBeCloseTo(view.lng, 4);
  expect(restored.lat).toBeCloseTo(view.lat, 4);
  expect(restored.zoom).toBeCloseTo(view.zoom, 1);
  // Restoring is not an interaction: the bare URL stays bare.
  expect(new URL(page.url()).search).toBe("");
});

test("the x-ray toggle joins the share link from the entry page", async ({ page, browser }) => {
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);

  await page.getByRole("button", { name: "Toggle x-ray overlay" }).click();
  await expect(page).toHaveURL(/xray/);
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();

  const shareUrl = page.url();
  const incognito = await browser.newContext();
  try {
    const copy = await incognito.newPage();
    await copy.goto(shareUrl);
    await expect(copy.getByRole("button", { name: "Toggle x-ray overlay" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(copy.locator("swath-map .swath-xray")).toBeAttached();
    expect(copy.url()).toBe(shareUrl);
  } finally {
    await incognito.close();
  }
});
