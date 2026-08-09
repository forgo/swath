// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The real-stack proof (issue #33): the demo page in real chromium, tiles
// from the real swath service (compose stack, granule already dropped by
// tests/e2e/stack-up.sh via `just e2e-web`). This is where the OGC z/y/x
// template ordering meets reality: a wrong template would still produce
// tile *requests*, but off-footprint ones — no 200s, blank canvas.
import { expect, type Page, test } from "@playwright/test";

/** Minimal structural view of the element for in-page evaluation. */
interface SwathMapLike {
  map?: {
    loaded(): boolean;
    areTilesLoaded(): boolean;
    triggerRepaint(): void;
    once(event: string, listener: () => void): unknown;
    getCanvas(): HTMLCanvasElement;
  };
}

async function waitForMapIdle(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    return Boolean(map?.loaded() && map?.areTilesLoaded());
  });
}

/** Reads the WebGL canvas back during a render frame (the drawing buffer
 * is not preserved, so pixels are only readable mid-frame) and reduces it
 * to blank-detection stats. */
async function canvasStats(page: Page): Promise<{ distinctColors: number; width: number }> {
  return await page.evaluate(async () => {
    const el = document.querySelector("swath-map") as SwathMapLike | null;
    const map = el?.map;
    if (!map) {
      throw new Error("swath-map has no map instance");
    }
    return await new Promise<{ distinctColors: number; width: number }>((resolve, reject) => {
      map.once("render", () => {
        try {
          const source = map.getCanvas();
          const copy = document.createElement("canvas");
          copy.width = source.width;
          copy.height = source.height;
          const context = copy.getContext("2d");
          if (!context) {
            throw new Error("no 2d context");
          }
          context.drawImage(source, 0, 0);
          const { data } = context.getImageData(0, 0, copy.width, copy.height);
          const colors = new Set<number>();
          for (let i = 0; i < data.length; i += 4) {
            const r = data[i] ?? 0;
            const g = data[i + 1] ?? 0;
            const b = data[i + 2] ?? 0;
            colors.add((r << 16) | (g << 8) | b);
          }
          resolve({ distinctColors: colors.size, width: copy.width });
        } catch (error) {
          reject(error instanceof Error ? error : new Error(String(error)));
        }
      });
      map.triggerRepaint();
    });
  });
}

function tileResponse(page: Page, layerId: string): Promise<{ url: string; contentType: string }> {
  return page
    .waitForResponse(
      (response) =>
        response.url().includes(`/tilesets/${layerId}/tiles/`) && response.status() === 200,
    )
    .then((response) => ({
      url: response.url(),
      contentType: response.headers()["content-type"] ?? "",
    }));
}

test("map loads, fetches real tiles, renders pixels, and switches layers", async ({ page }) => {
  // A tile request must reach the swath service and come back 200 PNG.
  // Zero-config demo: no `layer` attribute, so the first tileset wins —
  // the API lists tilesets in layer-id order, so that is `ndvi`.
  const initialTile = tileResponse(page, "ndvi");

  await page.goto("/demo/");
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();

  const tile = await initialTile;
  expect(tile.contentType).toBe("image/png");
  // OGC ordering on the wire: the requested path is z/row/col inside the
  // fixture footprint (z12: row 1561..1562, col 848).
  expect(tile.url).toMatch(/\/tilesets\/ndvi\/tiles\/\d+\/\d+\/\d+$/);

  // The canvas actually shows imagery: not blank, not a flat wash.
  await waitForMapIdle(page);
  const stats = await canvasStats(page);
  expect(stats.width).toBeGreaterThan(0);
  expect(stats.distinctColors).toBeGreaterThan(16);
  await page.locator("swath-map").screenshot({ path: "test-results/swath-map-ndvi.png" });

  // The built-in switcher: real buttons, aria-pressed reflects state, and
  // switching re-points the raster source — new requests hit the other
  // tileset and succeed against the same granule.
  const truecolorButton = page.getByRole("button", { name: "HLS true color" });
  const ndviButton = page.getByRole("button", { name: "HLS NDVI" });
  await expect(ndviButton).toHaveAttribute("aria-pressed", "true");
  await expect(truecolorButton).toHaveAttribute("aria-pressed", "false");

  const truecolorTile = tileResponse(page, "truecolor");
  await truecolorButton.click();
  expect((await truecolorTile).contentType).toBe("image/png");
  await expect(truecolorButton).toHaveAttribute("aria-pressed", "true");
  await expect(ndviButton).toHaveAttribute("aria-pressed", "false");

  await waitForMapIdle(page);
  const truecolorStats = await canvasStats(page);
  expect(truecolorStats.distinctColors).toBeGreaterThan(16);
  await page.locator("swath-map").screenshot({ path: "test-results/swath-map-truecolor.png" });

  // And back to ndvi: the switch is symmetric, and tile requests follow.
  const ndviAgain = tileResponse(page, "ndvi");
  await ndviButton.click();
  expect((await ndviAgain).contentType).toBe("image/png");
  await expect(ndviButton).toHaveAttribute("aria-pressed", "true");
});
