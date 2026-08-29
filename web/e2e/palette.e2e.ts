// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The command palette (issue #292): open with ⌘K / Ctrl-K or the top bar's
// button, type, Enter — the layer switches and `layer=` joins the URL.
import { expect, type Page, test } from "@playwright/test";
import { DEMO_PATH, layerRow as row } from "./support";

const palette = (page: Page) => page.locator("swath-command-palette");
test("Ctrl-K opens; 'ndvi' + Enter switches the layer and writes layer=", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=truecolor`);
  await expect(row(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("ControlOrMeta+k");
  await expect(palette(page)).toHaveAttribute("open", "");
  await page.keyboard.type("ndvi");
  await expect(palette(page).locator('[part="item"]').first()).toHaveAttribute(
    "data-command",
    "layer:ndvi",
  );
  await page.keyboard.press("Enter");
  await expect(palette(page)).not.toHaveAttribute("open", "");
  await expect(row(page, "ndvi")).toHaveAttribute("aria-pressed", "true");
  await expect(page).toHaveURL(/[?&]layer=ndvi(&|$)/);
});

test("the top-bar button opens it; Esc closes and returns focus to the button", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  const button = page.locator("#swath-search button");
  await button.click();
  await expect(palette(page)).toHaveAttribute("open", "");
  await expect(palette(page).locator('[part="input"]')).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(palette(page)).not.toHaveAttribute("open", "");
  await expect(button).toBeFocused();
});
