// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The entry experience (issue #108), against the real stack in both
// modes (vite dev at /demo/, against-binary at / — playwright.config.ts):
//
// - `/` with no params shows the layer browser and opens the cinematic
//   landing (issue #211): the Park Fire series auto-framed and looping,
//   WITHOUT rewriting the URL — hover pauses, a scrub takes over, the
//   x-ray invitation turns the overlay on, reduced motion holds with a
//   play affordance.
// - Selecting a layer updates the URL; that URL alone (incognito: a
//   fresh context with empty storage) reproduces the same view.
// - Share copies exactly that URL — byte-identical to the address bar
//   after an interaction, the explicit form of the landing before one.
// - Deep-link URLs are byte-stable: params applied, URL untouched, and
//   never animated over.
// - URL params beat localStorage — THE precedence rule.
// - localStorage restores the last layer/viewport on a paramless visit.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The app's storage key (src/view-state.ts). */
const STORAGE_KEY = "swath.view-state.v1";

/** The landing's layer: the first playable series (six Park Fire dates). */
const FIRE = "park-fire-ndvi";

const landingCard = (page: Page) => page.locator(".swath-map-landing");
const playButton = (page: Page) => page.locator(".swath-map-time-play");
const slider = (page: Page) => page.locator(".swath-map-time");
// `#swath-share` is a <swath-button> host: enabled/disabled and clicks
// belong to the native <button> in its shadow root (Playwright's CSS
// pierces it); the copy feedback (`data-state`, `data-url`) sits on the host.
const shareHost = (page: Page) => page.locator("#swath-share");
const shareButton = (page: Page) => page.locator("#swath-share button");

/** Waits for the loop to show a frame other than `from`. */
async function waitForFrameChange(page: Page, from: string | null): Promise<string> {
  await page.waitForFunction(
    (frame) =>
      document.querySelector<HTMLElement>(".swath-map-time")?.dataset["datetime"] !== frame,
    from,
    { timeout: 60_000 }, // a cold first pass renders every frame live
  );
  return (await slider(page).getAttribute("data-datetime")) ?? "";
}

/** Scrubs through the control's own range input (the user path). */
async function scrubTo(page: Page, index: number): Promise<void> {
  await page
    .locator('.swath-map-time input[type="range"]')
    .evaluate((el: HTMLInputElement, value) => {
      el.value = String(value);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }, index);
}

/** The fire series' frames straight from the granules API (ascending). */
async function granuleFrames(page: Page): Promise<string[]> {
  const response = await page.request.get("/datasets/hls-s30-fire/granules");
  expect(response.ok()).toBe(true);
  const body = (await response.json()) as { granules?: { datetime?: string }[] };
  return [...new Set((body.granules ?? []).map((granule) => granule.datetime ?? ""))].sort();
}

/** The share link the button copied: the clipboard's text (the real
 * thing) — which must also be what the button reports it copied. */
async function copiedLink(page: Page): Promise<string> {
  await shareButton(page).click();
  await expect(shareHost(page)).toHaveAttribute("data-state", "copied");
  const clipboard = await page.evaluate(() => navigator.clipboard.readText());
  expect(await shareHost(page).getAttribute("data-url")).toBe(clipboard);
  return clipboard;
}

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
  return page.locator(`swath-layer-item[data-layer="${layerId}"] [part="row"]`);
}

test("paramless / is the cinematic landing: the fire season loops, URL untouched", async ({
  page,
}) => {
  test.setTimeout(180_000); // the first pass over a cold cache renders live
  const tile = page.waitForResponse(
    (response) => response.url().includes(`/tilesets/${FIRE}/tiles/`) && response.status() === 200,
  );
  await page.goto(DEMO_PATH);

  // The layer browser lists the built-in layers; the landing's default
  // is the playable series (issue #211), not the first tileset by id.
  await expect(panelButton(page, "ndvi")).toBeVisible();
  await expect(panelButton(page, "truecolor")).toBeVisible();
  await expect(panelButton(page, FIRE)).toContainText("Park Fire NDVI");
  await expect(panelButton(page, FIRE)).toHaveAttribute("aria-pressed", "true");
  await expect(panelButton(page, "ndvi")).toHaveAttribute("aria-pressed", "false");

  // Auto-framed on the fire (~-121.7, 40.0), real tiles, and the loop
  // is playing on its own with the invitation up.
  await tile;
  await waitForFittedView(page);
  const view = await mapView(page);
  expect(view.lng).toBeGreaterThan(-122.5);
  expect(view.lng).toBeLessThan(-121);
  expect(view.lat).toBeGreaterThan(39.5);
  expect(view.lat).toBeLessThan(40.5);
  await expect(slider(page)).toBeVisible();
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "true");
  await expect(landingCard(page)).toBeVisible();
  await expect(landingCard(page)).toHaveAttribute("data-state", "playing");
  await expect(landingCard(page)).toContainText("watch the machine work");

  // Frames advance by themselves...
  const first = await slider(page).getAttribute("data-datetime");
  const second = await waitForFrameChange(page, first);
  await waitForFrameChange(page, second);
  // ...and none of that is an interaction: the bare URL stays bare, and
  // nothing was remembered as a session.
  expect(new URL(page.url()).search).toBe("");
  expect(await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY)).toBeNull();
});

test("hover pauses the landing loop; a scrub takes it over and the URL follows", async ({
  page,
}) => {
  test.setTimeout(180_000);
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  await expect(landingCard(page)).toHaveAttribute("data-state", "playing");

  // The pointer over the map holds the frame; moving off resumes.
  const box = await page.locator("swath-map").boundingBox();
  if (!box) {
    throw new Error("swath-map has no box");
  }
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await expect(landingCard(page)).toHaveAttribute("data-state", "hover");
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "false");
  const held = await slider(page).getAttribute("data-datetime");
  await page.waitForTimeout(2_600); // two would-be ticks
  await expect(slider(page)).toHaveAttribute("data-datetime", held ?? "");
  expect(new URL(page.url()).search).toBe("");
  await page.mouse.move(box.x - 40, box.y + box.height / 2); // into the rail
  await expect(landingCard(page)).toHaveAttribute("data-state", "playing");
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "true");

  // A scrub is the user's: the loop is over, the frame is theirs, and
  // the share link now carries the whole view — layer, view, frame.
  // To a frame OTHER than the one showing: an input at the current
  // index is a no-op for the range control (no frame change, no act).
  const showing = Number(await slider(page).getAttribute("data-index"));
  await scrubTo(page, showing === 0 ? 1 : 0);
  await expect(landingCard(page)).toHaveAttribute("data-state", "over");
  await expect(page).toHaveURL(
    /[?&]layer=park-fire-ndvi&center=-121\.[\d.]+,40\.[\d.]+&zoom=[\d.]+&t=2024-/,
  );
  // The scrub itself did not stop the running loop (the slider's own
  // controls decide); pressing pause does, and hover no longer resumes.
  await playButton(page).click();
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "false");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.move(box.x - 40, box.y + box.height / 2);
  await page.waitForTimeout(1_500);
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "false");
});

test("the invitation turns x-ray on, keeps the loop going, and joins the link", async ({
  page,
}) => {
  test.setTimeout(180_000);
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  await expect(landingCard(page)).toHaveAttribute("data-state", "playing");

  await landingCard(page)
    .getByRole("button", { name: /watch the machine work/i })
    .click();
  await expect(page.locator("swath-map .swath-xray")).toBeAttached();
  await expect(page.getByRole("button", { name: "Toggle x-ray overlay" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(landingCard(page)).toBeHidden(); // invitation accepted
  // The click left the pointer over the map (hover-paused); off it, the
  // loop is still the landing's and resumes under the overlay.
  await page.mouse.move(10, 300);
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "true");
  await expect(page).toHaveURL(/xray/);
});

test.describe("reduced motion", () => {
  test.use({ contextOptions: { reducedMotion: "reduce" } });

  test("the landing holds on its latest frame with a play affordance", async ({ page }) => {
    test.setTimeout(120_000);
    await page.goto(DEMO_PATH);
    await waitForFittedView(page);
    await expect(panelButton(page, FIRE)).toHaveAttribute("aria-pressed", "true");
    await expect(landingCard(page)).toHaveAttribute("data-state", "reduced");
    await expect(landingCard(page)).toContainText("watch the machine work");
    await expect(playButton(page)).toHaveAttribute("aria-pressed", "false");
    const affordance = landingCard(page).getByRole("button", { name: "Play the season" });
    await expect(affordance).toBeVisible();
    const held = await slider(page).getAttribute("data-datetime");
    await page.waitForTimeout(2_600);
    await expect(slider(page)).toHaveAttribute("data-datetime", held ?? "");
    expect(new URL(page.url()).search).toBe("");

    // The affordance starts the loop AS THE USER: frames advance and the
    // URL follows them.
    await affordance.click();
    await expect(playButton(page)).toHaveAttribute("aria-pressed", "true");
    await expect(landingCard(page)).toHaveAttribute("data-state", "over");
    await waitForFrameChange(page, held);
    await expect(page).toHaveURL(/[?&]t=2024-/);
  });
});

test.describe("share", () => {
  test.use({ permissions: ["clipboard-read", "clipboard-write"] });

  test("Share copies the explicit landing link; that link is a still, byte-stable view", async ({
    page,
    browser,
  }) => {
    test.setTimeout(180_000);
    await page.goto(DEMO_PATH);
    await expect(shareButton(page)).toBeDisabled(); // nothing to share yet
    await waitForFittedView(page);
    await expect(landingCard(page)).toHaveAttribute("data-state", "playing");
    await expect(shareButton(page)).toBeEnabled();

    // Copied mid-loop: the link pins whichever frame was showing on the
    // auto-framed view — a real frame of the series, or "latest" (no
    // `t`) when the loop has not advanced off its opening frame yet.
    const frames = await granuleFrames(page);
    const view = await mapView(page);
    const link = await copiedLink(page);
    const url = new URL(link);
    expect(url.origin + url.pathname).toBe(new URL(DEMO_PATH, page.url()).toString());
    expect(url.searchParams.get("layer")).toBe(FIRE);
    const frame = url.searchParams.get("t") ?? frames.at(-1) ?? "";
    expect(frames).toContain(frame);
    expect(url.searchParams.has("xray")).toBe(false);
    expect(Number(url.searchParams.get("zoom"))).toBeCloseTo(view.zoom, 1);
    // Sharing is not an interaction: the page's own URL stayed bare and
    // the loop is still the landing's.
    expect(new URL(page.url()).search).toBe("");
    await expect(landingCard(page)).toHaveAttribute("data-state", "playing");

    // Incognito: the link reproduces the frame and view exactly — and
    // being explicit state, it is NOT the cinematic landing: no loop, no
    // card, URL untouched.
    const incognito = await browser.newContext();
    try {
      const copy = await incognito.newPage();
      await copy.goto(link);
      await expect(panelButton(copy, FIRE)).toHaveAttribute("aria-pressed", "true");
      await expect(slider(copy)).toHaveAttribute("data-datetime", frame);
      await expect(playButton(copy)).toHaveAttribute("aria-pressed", "false");
      await expect(landingCard(copy)).toBeHidden();
      const copied = await mapView(copy);
      expect(copied.lng).toBeCloseTo(view.lng, 3);
      expect(copied.lat).toBeCloseTo(view.lat, 3);
      await copy.waitForTimeout(2_600);
      await expect(slider(copy)).toHaveAttribute("data-datetime", frame);
      expect(copy.url()).toBe(link);
    } finally {
      await incognito.close();
    }
  });

  test("after an interaction, Share and the address bar agree byte-for-byte", async ({ page }) => {
    await page.goto(DEMO_PATH);
    await waitForFittedView(page);
    await panelButton(page, "truecolor").click();
    await expect(page).toHaveURL(/\?layer=truecolor&center=[-\d.,]+&zoom=[\d.]+$/);
    await page.getByRole("button", { name: "Toggle x-ray overlay" }).click();
    await expect(page).toHaveURL(/&xray$/);
    const link = await copiedLink(page);
    expect(link).toBe(page.url());
    // Copying twice yields the same bytes.
    expect(await copiedLink(page)).toBe(link);

    // Compare state (issue #210) rides the same link: the fire series'
    // one-button before-vs-after compare puts `ct` in the URL (`swipe`
    // only rides an explicit handle move), and Share still agrees with
    // the address bar byte-for-byte.
    await panelButton(page, FIRE).click();
    await expect(page).toHaveURL(/layer=park-fire-ndvi/);
    await page.getByRole("button", { name: "Toggle compare swipe" }).click();
    await expect(page).toHaveURL(/[?&]ct=2024-/);
    expect(await copiedLink(page)).toBe(page.url());
  });
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

  // Byte-stable: the pasted URL survives the load byte-for-byte — and
  // explicit state is never the cinematic landing (no card, no loop).
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();
  expect(page.url()).toBe(new URL(deepLink, page.url()).toString());
  await expect(landingCard(page)).toBeHidden();
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "false");

  // The same holds for a deep link INTO the playable layer: the frame
  // and view are honored, nothing plays on its own.
  const fireLink = `${DEMO_PATH}?layer=${FIRE}&center=-121.6932,40.0208&zoom=12&t=2024-06-07T19:03:00Z`;
  await page.goto(fireLink);
  await expect(slider(page)).toHaveAttribute("data-datetime", "2024-06-07T19:03:00Z");
  await expect(landingCard(page)).toBeHidden();
  await page.waitForTimeout(2_600);
  await expect(playButton(page)).toHaveAttribute("aria-pressed", "false");
  await expect(slider(page)).toHaveAttribute("data-datetime", "2024-06-07T19:03:00Z");
  expect(page.url()).toBe(new URL(fireLink, page.url()).toString());
});

test("localStorage restores the last layer and viewport on a paramless visit", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);

  // A session: switch layers (persisted as it happens).
  await panelButton(page, "truecolor").click();
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  await expect(page).toHaveURL(/layer=truecolor/);
  const view = await mapView(page);

  // A later paramless visit resumes exactly there — a restored session
  // is explicit state, not the cinematic landing.
  await page.goto(DEMO_PATH);
  await expect(panelButton(page, "truecolor")).toHaveAttribute("aria-pressed", "true");
  const restored = await mapView(page);
  expect(restored.lng).toBeCloseTo(view.lng, 4);
  expect(restored.lat).toBeCloseTo(view.lat, 4);
  expect(restored.zoom).toBeCloseTo(view.zoom, 1);
  await expect(landingCard(page)).toBeHidden();
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
