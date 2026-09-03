// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Modes in the URL (issue #283): `view=` follows the view-state rules —
// a deep link is honoured byte-for-byte, a switch writes it, storage
// restores it on a bare visit. No shell yet: today's panels show or hide.
import { expect, type Page, test } from "@playwright/test";
import { DEMO_PATH, railMode as modeButton } from "./support";

const APP_KEY = "swath.app-state.v1";

const pressed = (page: Page, mode: string) =>
  expect(modeButton(page, mode)).toHaveAttribute("aria-current", "page");

test("/?view=data lands in Data mode with the URL untouched", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?view=data`);
  await pressed(page, "data");
  await expect(page.locator("swath-layer-list")).toBeHidden();
  await expect(page.locator("swath-catalog")).toBeVisible();
  await expect(page.locator("swath-add-data-panel")).toBeVisible();
  await expect(page.locator("swath-authoring-panel")).toBeHidden();
  await page.waitForTimeout(1_500); // the landing loop must not touch a deep link
  expect(new URL(page.url()).search).toBe("?view=data");
});

test("a mode switch writes view=; back to layers removes it; storage remembers", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  await pressed(page, "layers");
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
  await pressed(page, "xray");
  await expect(page.locator("swath-map")).toHaveAttribute("xray", "");
  await expect(page.locator("swath-layer-list")).toBeVisible();
  await expect(page.locator("swath-authoring-panel")).toBeHidden();
});

test("rail collapse is a device preference: storage remembers it, a rail=collapsed link is honoured without a rewrite", async ({
  page,
}) => {
  await page.goto(DEMO_PATH);
  const rail = page.locator("swath-rail");
  await expect(rail).not.toHaveAttribute("collapsed", "");
  await rail.locator('[part="collapse"] button').click();
  await expect(rail).toHaveAttribute("collapsed", "");
  expect(await page.evaluate((key) => localStorage.getItem(key), APP_KEY)).toBe(
    '{"view":"layers","rail":"collapsed"}',
  );
  expect(new URL(page.url()).search).not.toContain("rail");
  await page.reload();
  await expect(rail).toHaveAttribute("collapsed", "");

  const fresh = await page.context().browser()?.newContext();
  const link = await fresh?.newPage();
  if (!link) {
    throw new Error("no context");
  }
  await link.goto(`${DEMO_PATH}?rail=collapsed&view=data`);
  await expect(link.locator("swath-rail")).toHaveAttribute("collapsed", "");
  await link.locator('swath-rail [part="item"][data-mode="author"]').click();
  await expect(link).toHaveURL(/[?&]view=author/);
  expect(new URL(link.url()).search).toContain("rail=collapsed");
  await fresh?.close();
});

/** Steps back until `view=` reads `mode`, bounded. Deliberately not "back
 * exactly once": a mode can legitimately push more than one entry (entering
 * `author` also writes `sel=` once a step is selected), and this test is
 * about whether history *walks the artifacts*, not about how many entries
 * each one costs. The no-extra-entries property is asserted precisely, on
 * `history.length`, by the pan test below. */
async function backTo(page: Page, mode: string | null): Promise<void> {
  for (let step = 0; step < 8; step += 1) {
    await page.goBack();
    if (new URL(page.url()).searchParams.get("view") === mode) {
      return;
    }
  }
  throw new Error(`back never reached view=${String(mode)}; stopped at ${page.url()}`);
}

test("the back button walks artifacts (#392)", async ({ page }) => {
  // An explicit deep link, so the cinematic landing loop is not playing:
  // its frame advances rewrite the current entry in place (by design), which
  // would make the URLs under test move while they are being read.
  await page.goto(`${DEMO_PATH}?layer=park-fire-ndvi`);
  await pressed(page, "layers");

  await modeButton(page, "data").click();
  await expect(page).toHaveURL(/[?&]view=data(&|$)/);
  await modeButton(page, "author").click();
  await expect(page).toHaveURL(/[?&]view=author(&|$)/);
  await modeButton(page, "xray").click();
  await expect(page).toHaveURL(/[?&]view=xray(&|$)/);

  // Back returns to the view you were just looking at, without a reload —
  // the shell is driven from the URL by `popstate`, the same path a cold
  // load takes, so the panels follow.
  await backTo(page, "author");
  await pressed(page, "author");
  await expect(page.locator("swath-authoring-panel")).toBeVisible();

  await backTo(page, "data");
  await pressed(page, "data");
  await expect(page.locator("swath-catalog")).toBeVisible();

  await backTo(page, null);
  await pressed(page, "layers");
  await expect(page.locator("swath-layer-list")).toBeVisible();

  // Forward walks back up: history is real, not a one-way trip.
  await page.goForward();
  await expect(page).toHaveURL(/[?&]view=data(&|$)/);
  await pressed(page, "data");
});

test("panning replaces rather than pushes: the camera adds no history (#392)", async ({ page }) => {
  await page.goto(`${DEMO_PATH}?layer=park-fire-ndvi`);
  await modeButton(page, "data").click();
  await expect(page).toHaveURL(/[?&]view=data(&|$)/);

  const map = page.locator("swath-map");
  await expect(map).toBeVisible();
  // Let the landing settle so an artifact resolving mid-test (the layer the
  // server picks) is not counted as a pan.
  await page.waitForTimeout(1_000);
  const before = await page.evaluate(() => history.length);

  for (const [x, y] of [
    [480, 340],
    [340, 330],
    [440, 250],
  ] as const) {
    await map.hover();
    await page.mouse.down();
    await page.mouse.move(x, y, { steps: 8 });
    await page.mouse.up();
    await page.waitForTimeout(300);
  }

  // The camera moved and the URL followed it...
  await expect(page).toHaveURL(/[?&](center|zoom)=/);
  // ...without adding a single entry. This is the whole point of the split:
  // forty pans must not bury the view you were looking at.
  expect(await page.evaluate(() => history.length)).toBe(before);
});

test("the chip row is the URL made visible; removing a chip drops its param (#393)", async ({
  page,
}) => {
  await page.goto(`${DEMO_PATH}?layer=park-fire-ndvi&xray`);
  const chips = page.locator("swath-chip-row");
  await expect(chips.locator('[part="chip"][data-chip="layer"]')).toContainText("park-fire-ndvi");
  const xray = chips.locator('[part="chip"][data-chip="xray"]');
  await expect(xray).toBeVisible();

  // Dropping the chip drops the parameter — and pushes, so back restores it.
  await xray.locator('[part="remove"]').click();
  await expect(page).not.toHaveURL(/[?&]xray(&|=|$)/);
  await expect(xray).toBeHidden();

  await page.goBack();
  await expect(page).toHaveURL(/[?&]xray(&|=|$)/);
  await expect(chips.locator('[part="chip"][data-chip="xray"]')).toBeVisible();
});
