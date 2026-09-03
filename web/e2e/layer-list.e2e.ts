// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The layer list (issue #282): switching, the eye and the opacity slider
// act on the viewed layer's raster in the real map; kebab delete lives in
// authoring.e2e.ts (it needs a published service).
import { expect, type Page, test } from "@playwright/test";
import { DEMO_PATH, demoUrl, FIRE_LAYER as FIRE, layerRow as row } from "./support";

/** The raster layer's paint on the primary map. */
async function paint(page: Page): Promise<{ visibility: string; opacity: number }> {
  return page.evaluate(() => {
    const el = document.querySelector("swath-map") as {
      map?: {
        getLayoutProperty(id: string, name: string): string | undefined;
        getPaintProperty(id: string, name: string): number | undefined;
      };
    } | null;
    return {
      visibility: el?.map?.getLayoutProperty("swath", "visibility") ?? "visible",
      opacity: el?.map?.getPaintProperty("swath", "raster-opacity") ?? 1,
    };
  });
}

test("switching layers through the list updates the URL and the pressed row", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await expect(row(page, FIRE)).toHaveAttribute("aria-pressed", "true");
  await row(page, "truecolor").click();
  await expect(row(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  await expect(row(page, FIRE)).toHaveAttribute("aria-pressed", "false");
  await expect(page).toHaveURL(/[?&]layer=truecolor/);
});

test("the eye hides the viewed raster and shows it again; the URL never learns", async ({
  page,
}) => {
  await page.goto(demoUrl({ layer: "truecolor" }));
  await expect(row(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  const eye = page.locator('swath-layer-item[data-layer="truecolor"] [part="eye"] button');
  await expect(eye).toHaveAttribute("aria-pressed", "true");
  await eye.click();
  await expect(eye).toHaveAttribute("aria-pressed", "false");
  await expect.poll(async () => (await paint(page)).visibility).toBe("none");
  await eye.click();
  await expect.poll(async () => (await paint(page)).visibility).toBe("visible");
  expect(new URL(page.url()).search).toBe("?layer=truecolor");
});

test("the opacity slider on the active row drives raster-opacity", async ({ page }) => {
  await page.goto(demoUrl({ layer: "truecolor" }));
  const range = page.locator('swath-layer-item[data-layer="truecolor"] [part="opacity"] input');
  await expect(range).toBeVisible();
  await range.focus();
  for (let i = 0; i < 6; i += 1) {
    await page.keyboard.press("ArrowLeft"); // 6 × 0.05
  }
  await expect.poll(async () => (await paint(page)).opacity).toBeCloseTo(0.7, 2);
  // A non-active row keeps its slider folded.
  await expect(
    page.locator(`swath-layer-item[data-layer="${FIRE}"] [part="opacity"]`),
  ).toBeHidden();
});
