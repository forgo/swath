// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Modes in the URL (issue #283): `view=` follows the view-state rules —
// a deep link is honoured byte-for-byte, a switch writes it, storage
// restores it on a bare visit. No shell yet: today's panels show or hide.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";
const APP_KEY = "swath.app-state.v1";

const modeButton = (page: Page, mode: string) =>
  page.locator(`#swath-modes [data-mode="${mode}"] button`);

test("/?view=data lands in Data mode with the URL untouched", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?view=data`);
  await expect(modeButton(page, "data")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("swath-layer-list")).toBeHidden();
  await expect(page.locator("swath-dataset-panel")).toBeVisible();
  await expect(page.locator("swath-add-data-panel")).toBeVisible();
  await expect(page.locator("swath-authoring-panel")).toBeHidden();
  await page.waitForTimeout(1_500); // the landing loop must not touch a deep link
  expect(new URL(page.url()).search).toBe("?view=data");
});

test("a mode switch writes view=; back to layers removes it; storage remembers", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await expect(modeButton(page, "layers")).toHaveAttribute("aria-pressed", "true");
  await modeButton(page, "author").click();
  await expect(page).toHaveURL(/[?&]view=author(&|$)/);
  await expect(page.locator("swath-authoring-panel")).toBeVisible();
  await expect(page.locator("swath-layer-list")).toBeHidden();
  expect(await page.evaluate((key) => localStorage.getItem(key), APP_KEY)).toBe(
    '{"view":"author"}',
  );
  await modeButton(page, "layers").click();
  await expect(page).not.toHaveURL(/[?&]view=/);
  await expect(page.locator("swath-layer-list")).toBeVisible();
});

test("a bare visit restores the last mode from storage; the x-ray mode turns the overlay on", async ({
  page,
}) => {
  await page.addInitScript(
    ([key, value]) => {
      window.localStorage.setItem(String(key), String(value));
    },
    [APP_KEY, JSON.stringify({ view: "xray" })],
  );
  await page.goto(DEMO_PATH);
  await expect(modeButton(page, "xray")).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("swath-map")).toHaveAttribute("xray", "");
  await expect(page.locator("swath-layer-list")).toBeVisible();
  await expect(page.locator("swath-authoring-panel")).toBeHidden();
});
