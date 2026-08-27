// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The time slider against the real stack (issue #182): the Park Fire
// series (six granules, tests/e2e/drop-fire-granules.sh via stack-up)
// drives the charter-promised animation. Asserted here:
//
// - the slider's domain is exactly the granules API's acquisition
//   datetimes (the test fetches the listing itself and compares);
// - single-date layers hide the control (zero-config landing untouched);
// - scrubbing re-points tile requests with `datetime=`, `t` joins the
//   share link, and the deep link alone reproduces the frame — without
//   being rewritten (the byte-stability contract, issue #108);
// - THE signature demo: the first pass over the season renders every
//   frame live, the second pass replays from the tile cache — verified
//   through the same trace stream the overlay paints from (the test's
//   own SSE subscription, the swath-xray.e2e.ts convention), with the
//   per-frame plan-mix line in the analytics card agreeing per frame;
// - play advances frames on its own and prefetches the next frame's
//   tiles before showing them.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The Park Fire fixture footprint's viewpoint (granule bbox
 * -121.7388..-121.6475 / 39.9856..40.0559). The vite-dev pass views z13
 * display tiles; the against-binary pass dives one zoom deeper (z14) so
 * its first pass over the shared stack's cache is still cold — the same
 * one-stack convention as swath-xray.e2e.ts. */
const CENTER = "-121.6932,40.0208";
const ZOOM = process.env.SWATH_E2E_MODE === "binary" ? "13" : "12";

/** The signature-loop test's OWN zoom, one deeper than everything else
 * in this file (and than the other mode's signature pass — one stack
 * serves both): its "first pass renders live" premise needs tiles no
 * other test has touched. The scrub test above provably leaks frame
 * renders into the server cache on slow runners — its page closes with
 * `datetime=` requests still in flight, and a render that has already
 * left the proxy completes (and caches) server-side regardless — which
 * turned the signature's first pass into cache hits on CI while local
 * runs (whose queued requests die with the page) stayed live. */
const SIGNATURE_ZOOM = process.env.SWATH_E2E_MODE === "binary" ? "14" : "13";

const LAYER = "park-fire-ndvi";
const DATASET = "hls-s30-fire";

/** The trace envelope as swath-api pins it (subset these tests read). */
interface Envelope {
  tile: string;
  layer: string;
  trace: {
    decision: string | { overview: { level: number } } | { cache_hit: { key: string } };
    timings: { total_ms: number };
    temporal?: {
      granule_id: string;
      granule_datetime: string;
      requested: string | null;
      rule: string;
    } | null;
  };
}

declare global {
  interface Window {
    __timeReceived?: Envelope[];
  }
}

/** The test's own subscription, awaited until the stream is OPEN —
 * not just constructed. The distinction is load-bearing on CI: the
 * fire view's initial load saturates the browser's per-origin
 * connection budget with ~28 tile requests, and a still-queued
 * EventSource misses every envelope published before it connects (SSE
 * has no replay) — the badge/stream agreement below could then never
 * complete. Locally the tiles drain fast enough to mask it. */
async function subscribeToTraces(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        const received: Envelope[] = [];
        window.__timeReceived = received;
        const source = new EventSource("/traces");
        source.addEventListener("trace", (event) => {
          received.push(JSON.parse((event as MessageEvent<string>).data) as Envelope);
        });
        source.addEventListener("open", () => resolve(), { once: true });
      }),
  );
}

/** Waits until the zero-config bounds fit has landed and settled. */
async function waitForFittedView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document.querySelector("swath-map") as {
      map?: { loaded(): boolean; areTilesLoaded(): boolean; getZoom(): number };
    } | null;
    const map = el?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

/** The layer's temporal domain straight from the granules API, reduced
 * exactly as the slider must: ascending, de-duplicated. */
async function granuleFrames(page: Page): Promise<string[]> {
  const response = await page.request.get(`/datasets/${DATASET}/granules`);
  expect(response.ok()).toBe(true);
  const body = (await response.json()) as { granules?: { datetime?: string }[] };
  const datetimes = (body.granules ?? [])
    .map((granule) => granule.datetime)
    .filter((value): value is string => typeof value === "string");
  return [...new Set(datetimes)].sort((a, b) => Date.parse(a) - Date.parse(b));
}

const slider = (page: Page) => page.locator("swath-map .swath-map-time");

/** Scrubs to frame `index` through the control's own range input (the
 * user path: set + input event, exactly what dragging emits). */
async function scrubTo(page: Page, index: number): Promise<void> {
  await page
    .locator('swath-map .swath-map-time input[type="range"]')
    .evaluate((el: HTMLInputElement, value) => {
      el.value = String(value);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }, index);
}

/**
 * Polls until every painted badge of the fire layer at the displayed
 * zoom (a) belongs to `frame` per the latest trace the TEST received
 * for that tile and (b) shows `kind` — i.e. the overlay and the stream
 * agree, and the whole viewport is on the expected side of the cache.
 * Returns the number of badges that settled.
 */
async function expectFrameBadges(page: Page, frame: string, kind: string): Promise<number> {
  const dump = async (): Promise<string> =>
    await page.evaluate(
      ({ frame, kind, layer }) => {
        const received = window.__timeReceived ?? [];
        const latest = new Map<string, Envelope>();
        for (const envelope of received) {
          latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
        }
        const badges = [...document.querySelectorAll<HTMLElement>(".swath-xray-badge")]
          .filter((badge) => (badge.dataset.key ?? "").startsWith(`${layer}/`))
          .map((badge) => {
            const key = badge.dataset.key ?? "";
            const envelope = latest.get(key);
            return `${key}=${badge.dataset.decision}|rx:${
              envelope
                ? `${typeof envelope.trace.decision === "string" ? envelope.trace.decision : JSON.stringify(envelope.trace.decision)}@${envelope.trace.temporal?.granule_datetime ?? "-"}`
                : "MISSING"
            }`;
          });
        const analytics = document.querySelector<HTMLElement>(".swath-xray-analytics");
        return JSON.stringify(
          {
            want: { frame, kind },
            received: received.length,
            receivedForFrame: received.filter(
              (e) => e.layer === layer && e.trace.temporal?.granule_datetime === frame,
            ).length,
            analyticsFrame: analytics?.dataset.frame,
            slider: document.querySelector<HTMLElement>(".swath-map-time")?.dataset,
            badges,
          },
          null,
          1,
        );
      },
      { frame, kind, layer: LAYER },
    );
  const handle = await page
    .waitForFunction(
      ({ frame, kind, layer }) => {
        const received = window.__timeReceived ?? [];
        const latest = new Map<string, Envelope>();
        for (const envelope of received) {
          latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
        }
        const badges = [...document.querySelectorAll<HTMLElement>(".swath-xray-badge")].filter(
          (badge) => (badge.dataset.key ?? "").startsWith(`${layer}/`),
        );
        if (badges.length === 0) {
          return null;
        }
        for (const badge of badges) {
          const envelope = latest.get(badge.dataset.key ?? "");
          if (!envelope || envelope.trace.temporal?.granule_datetime !== frame) {
            return null; // this tile's latest trace is not the frame yet
          }
          const { decision } = envelope.trace;
          const flat =
            typeof decision === "string"
              ? decision
              : "cache_hit" in decision
                ? "cache_hit"
                : "overview";
          if (flat !== kind || badge.dataset.decision !== kind) {
            return null;
          }
        }
        return badges.length;
      },
      { frame, kind, layer: LAYER },
      // Generous per-frame budget: a frame is a viewport of live NDVI
      // renders, and CI's 2-core runner shares them with the browser and
      // the dev server (observed >30 s there; seconds locally).
      { timeout: 120_000 },
    )
    .catch(async (error: unknown) => {
      // The stall's anatomy rides the failure itself: which badge
      // disagrees, what the stream last said about it, and what the
      // overlay shows.
      throw new Error(`expectFrameBadges stalled (${String(error)}): ${await dump()}`);
    });
  return (await handle.jsonValue()) as number;
}

test("slider domain is the granules API's datetimes; single-date layers hide it", async ({
  page,
}) => {
  const frames = await granuleFrames(page);
  expect(frames.length).toBeGreaterThanOrEqual(2); // the six-date series

  await page.goto(`${DEMO_PATH}?layer=${LAYER}&center=${CENTER}&zoom=${ZOOM}`);
  await waitForFittedView(page);
  await expect(slider(page)).toBeVisible();
  await expect(slider(page)).toHaveAttribute("data-frames", String(frames.length));
  // No `t` in the URL = latest: the thumb rests on the newest frame.
  await expect(slider(page)).toHaveAttribute("data-index", String(frames.length - 1));
  await expect(slider(page)).toHaveAttribute("data-datetime", frames[frames.length - 1] ?? "");

  // The single-granule fixture layer has one date — nothing to scrub,
  // control hidden: the zero-config landing page is visually untouched.
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=-105.4475,39.2650&zoom=12`);
  await waitForFittedView(page);
  await expect(slider(page)).toBeHidden();
});

test("scrubbing re-points tiles with datetime=; t joins the deep link, byte-stably", async ({
  page,
  browser,
}) => {
  const frames = await granuleFrames(page);
  const target = frames[1];
  if (target === undefined) {
    throw new Error("fire series has fewer than two frames");
  }

  await page.goto(`${DEMO_PATH}?layer=${LAYER}&center=${CENTER}&zoom=${ZOOM}`);
  await waitForFittedView(page);
  // Scrubbing needs the domain: wait for the slider to have its frames
  // (the granules fetch rides the layer apply, after the fitted view).
  await expect(slider(page)).toHaveAttribute("data-frames", String(frames.length));

  // Scrub to the second-oldest frame: the raster source refetches with
  // that exact acquisition instant as `datetime=`.
  const framed = page.waitForRequest(
    (request) =>
      request.url().includes(`/tilesets/${LAYER}/tiles/`) &&
      request.url().includes(`datetime=${encodeURIComponent(target)}`),
  );
  await scrubTo(page, 1);
  await framed;
  await expect(slider(page)).toHaveAttribute("data-datetime", target);

  // The scrub was a user interaction: `t` joins the share link,
  // human-readable (colons verbatim, the hand-written deep-link style).
  await expect(page).toHaveURL(new RegExp(`[?&]t=${target.replaceAll(":", "\\:")}`));
  const shareUrl = page.url();

  // Incognito: the deep link alone reproduces the frame — and survives
  // byte-for-byte (the issue #108 contract, extended to time).
  const incognito = await browser.newContext();
  try {
    const copy = await incognito.newPage();
    const tile = copy.waitForRequest(
      (request) =>
        request.url().includes(`/tilesets/${LAYER}/tiles/`) &&
        request.url().includes(`datetime=${encodeURIComponent(target)}`),
    );
    await copy.goto(shareUrl);
    await tile;
    await expect(slider(copy)).toHaveAttribute("data-datetime", target);
    expect(copy.url()).toBe(shareUrl);
  } finally {
    await incognito.close();
  }
});

test("the signature loop: first pass renders live, second pass replays from cache", async ({
  page,
}) => {
  // Deliberately generous: the first pass live-renders every frame's
  // visible tiles (six frames of real NDVI math), and CI's 2-core
  // runner needs real headroom per frame.
  test.setTimeout(600_000);
  const frames = await granuleFrames(page);

  await page.goto(`${DEMO_PATH}?xray&layer=${LAYER}&center=${CENTER}&zoom=${SIGNATURE_ZOOM}`);
  await expect(page.locator("swath-map canvas.maplibregl-canvas")).toBeVisible();
  await subscribeToTraces(page);
  await waitForFittedView(page);
  await expect(slider(page)).toHaveAttribute("data-frames", String(frames.length));

  const analytics = page.locator(".swath-xray-analytics");

  // The page load itself rendered the NEWEST frame (no `t` = latest) —
  // before the test's subscription opened, so the first pass covers the
  // frames whose renders this stream provably sees end to end: every
  // frame but the newest (which rejoins in the cached second pass).
  const firstPass = frames.slice(0, -1);

  // --- First pass, oldest → newest: every frame's badges say `live`
  // (this pass's tiles have never been rendered on this stack at this
  // zoom), and the analytics card's per-frame line narrates that frame.
  for (const [index, frame] of firstPass.entries()) {
    await scrubTo(page, index);
    const settled = await expectFrameBadges(page, frame, "live");
    expect(settled).toBeGreaterThan(0);
    await expect(analytics).toHaveAttribute("data-frame", frame);
    const live = Number(await analytics.getAttribute("data-frame-live"));
    expect(live).toBeGreaterThanOrEqual(settled);
    // No `data-frame-cache-hit == 0` here: the per-frame line reduces
    // the server-wide /traces stream, and since issue #211 sibling
    // workers' pages loop this very series on landing — their cache
    // hits for this frame land on this page's card. The viewport-scoped
    // badge check above is the isolated "first pass renders live" proof.
  }

  // --- Second pass over the whole season (the newest frame included:
  // its load-time render warmed the cache the same way): every request
  // resolves to the granule already cached under it — cache hits across
  // the board, the glass box showing WHY the loop is now smooth.
  for (const [index, frame] of frames.entries()) {
    await scrubTo(page, index);
    const settled = await expectFrameBadges(page, frame, "cache_hit");
    expect(settled).toBeGreaterThan(0);
    await expect(analytics).toHaveAttribute("data-frame", frame);
    const cached = Number(await analytics.getAttribute("data-frame-cache-hit"));
    expect(cached).toBeGreaterThanOrEqual(settled);
  }

  // Every temporal trace this test received belongs to the fire layer's
  // frames — the stream's account and the granules API agree.
  const received = await page.evaluate(() => window.__timeReceived ?? []);
  const frameSet = new Set(frames);
  for (const envelope of received) {
    if (envelope.layer === LAYER && envelope.trace.temporal) {
      expect(frameSet.has(envelope.trace.temporal.granule_datetime)).toBe(true);
    }
  }
});

test("play advances frames and prefetches the next frame before showing it", async ({ page }) => {
  test.setTimeout(120_000);
  const frames = await granuleFrames(page);
  const first = frames[0];
  const second = frames[1];
  if (first === undefined || second === undefined) {
    throw new Error("fire series has fewer than two frames");
  }

  // Start on the OLDEST frame so the play order is deterministic.
  await page.goto(`${DEMO_PATH}?layer=${LAYER}&center=${CENTER}&zoom=${ZOOM}&t=${first}`);
  await waitForFittedView(page);
  await expect(slider(page)).toHaveAttribute("data-datetime", first);

  // Prefetch leads display: pressing play must issue tile requests for
  // the SECOND frame while the slider still shows the first (the next
  // tick's tiles are warm before they are asked for on screen).
  const prefetched = page.waitForRequest(
    (request) =>
      request.url().includes(`/tilesets/${LAYER}/tiles/`) &&
      request.url().includes(`datetime=${encodeURIComponent(second)}`),
    { timeout: 10_000 },
  );
  const play = page.locator(".swath-map-time-play");
  await play.click();
  await expect(play).toHaveAttribute("aria-pressed", "true");
  await prefetched;
  // Read immediately after the prefetch request fired: the advance is a
  // full play-interval away, so the displayed frame is still the first.
  await expect(slider(page)).toHaveAttribute("data-datetime", first);

  // Then the loop advances on its own: each observed frame is a real
  // frame, in ascending season order, and the URL tracks it (`t=`).
  await page.waitForFunction(
    (from) => document.querySelector<HTMLElement>(".swath-map-time")?.dataset["datetime"] !== from,
    first,
    { timeout: 15_000 },
  );
  const advanced = await slider(page).getAttribute("data-datetime");
  expect(frames).toContain(advanced ?? "");
  expect(Date.parse(advanced ?? "")).toBeGreaterThan(Date.parse(first));
  await expect(page).toHaveURL(/[?&]t=2024-/);
  // And keeps going (a second tick lands on a further frame).
  await page.waitForFunction(
    (from) => document.querySelector<HTMLElement>(".swath-map-time")?.dataset["datetime"] !== from,
    advanced,
    { timeout: 15_000 },
  );

  await play.click(); // pause
  await expect(play).toHaveAttribute("aria-pressed", "false");
  const at = await slider(page).getAttribute("data-datetime");
  await page.waitForTimeout(2_500); // two would-be ticks
  await expect(slider(page)).toHaveAttribute("data-datetime", at ?? "");
});

// --- finding the data (issue #182 follow-up): the ~10 km fire window
// must be reachable without knowing where on Earth to look.

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

const zoomToData = (page: Page) => page.getByRole("button", { name: "Zoom to the layer's data" });

test("switching to the off-screen fire layer auto-frames it; deep links are honored", async ({
  page,
}) => {
  // Start on the Colorado fixture view (an explicit, URL-carried view).
  await page.goto(`${DEMO_PATH}?layer=ndvi&center=-105.4475,39.2650&zoom=11`);
  await waitForFittedView(page);

  // The user picks the fire layer in the rail: its footprint is nowhere
  // near Colorado, so the view auto-frames the data.
  await page.locator(`swath-layer-item[data-layer="${LAYER}"] [part="row"]`).click();
  await expect.poll(async () => (await mapView(page)).lng, { timeout: 30_000 }).toBeLessThan(-121);
  const framedView = await mapView(page);
  expect(framedView.lng).toBeGreaterThan(-122.5);
  expect(framedView.lat).toBeGreaterThan(39.5);
  expect(framedView.lat).toBeLessThan(40.5);
  // The framed view is user-driven, so the share link follows it.
  await expect(page).toHaveURL(/layer=park-fire-ndvi/);
  await expect.poll(() => new URL(page.url()).searchParams.get("center") ?? "").toMatch(/^-121\./);
  // The recovery affordance is on screen for the current layer.
  await expect(zoomToData(page)).toBeVisible();

  // A deep link with an explicit view is HONORED — no auto-frame stomp,
  // and the pasted URL survives byte-for-byte (the issue #108 contract).
  const deepLink = `${DEMO_PATH}?layer=${LAYER}&center=-105.4475,39.265&zoom=11`;
  await page.goto(deepLink);
  // The apply has fully settled once the slider carries its domain.
  await expect(slider(page)).toHaveAttribute("data-frames", /\d+/);
  const held = await mapView(page);
  expect(held.lng).toBeCloseTo(-105.4475, 3);
  expect(held.lat).toBeCloseTo(39.265, 3);
  expect(page.url()).toBe(new URL(deepLink, page.url()).toString());

  // From that data-less view, "zoom to data" recovers — and the share
  // link follows the user-driven move.
  await zoomToData(page).click();
  await expect.poll(async () => (await mapView(page)).lng).toBeLessThan(-121);
  await expect.poll(() => new URL(page.url()).searchParams.get("center") ?? "").toMatch(/^-121\./);
});

// --- the compare swipe, date-vs-date (issue #210) ---
//
// These tests live HERE, not in compare.e2e.ts, deliberately: they
// render Park Fire frames, and this file owns the fire layer's trace
// traffic — the signature-loop test above asserts per-frame analytics
// ("this frame: N live, 0 cached") that any cross-worker fire render
// would race (Playwright parallelizes FILES; tests within one file are
// serialized). They also run LAST, so their renders can never warm a
// tile the earlier cold-cache premises depend on. Layer-vs-layer
// coverage (Colorado fixtures only) stays in compare.e2e.ts.

/** The compare tests' own style zoom: display tiles z12 (vite) / z13
 * (binary) — never the signature loop's z14/z15. */
const COMPARE_ZOOM = process.env.SWATH_E2E_MODE === "binary" ? "12" : "11";

const compareHandle = (page: Page) => page.locator("swath-map .swath-map-compare-handle");

test("date-vs-date: t and ct split one layer across the handle, byte-stably", async ({
  page,
  browser,
}) => {
  const frames = await granuleFrames(page);
  const left = frames[0];
  const right = frames[frames.length - 1];
  if (left === undefined || right === undefined || left === right) {
    throw new Error("fire series has fewer than two frames");
  }

  const deepLink =
    `${DEMO_PATH}?layer=${LAYER}&center=${CENTER}&zoom=${COMPARE_ZOOM}` +
    `&t=${left}&ct=${right}&swipe=0.5`;
  // Both sides fetch their OWN frame: the primary map the left `t`, the
  // clipped right map the compare `ct`.
  const leftTile = page.waitForRequest(
    (request) =>
      request.url().includes(`/tilesets/${LAYER}/tiles/`) &&
      request.url().includes(`datetime=${encodeURIComponent(left)}`),
  );
  const rightTile = page.waitForRequest(
    (request) =>
      request.url().includes(`/tilesets/${LAYER}/tiles/`) &&
      request.url().includes(`datetime=${encodeURIComponent(right)}`),
  );
  await page.goto(deepLink);
  await leftTile;
  await rightTile;
  await waitForFittedView(page);

  // One handle, date mode, per-side chips naming the two frames.
  await expect(compareHandle(page)).toBeVisible();
  await expect(compareHandle(page)).toHaveAttribute("data-mode", "date");
  await expect(page.locator('.swath-map-compare-label[data-side="left"]')).toHaveText(left);
  await expect(page.locator('.swath-map-compare-label[data-side="right"]')).toHaveText(right);

  // The pasted deep link was never rewritten (the issue #108 contract).
  expect(page.url()).toBe(new URL(deepLink, page.url()).toString());

  // Incognito: the link alone reproduces the comparison, byte-stably.
  const incognito = await browser.newContext();
  try {
    const copy = await incognito.newPage();
    const copyRight = copy.waitForRequest(
      (request) =>
        request.url().includes(`/tilesets/${LAYER}/tiles/`) &&
        request.url().includes(`datetime=${encodeURIComponent(right)}`),
    );
    await copy.goto(page.url());
    await copyRight;
    await expect(copy.locator("swath-map .swath-map-compare-handle")).toBeVisible();
    expect(copy.url()).toBe(page.url());
  } finally {
    await incognito.close();
  }

  // Moving the handle is a user interaction: `swipe` follows in the
  // share link. Keyboard first (the handle is a real slider)…
  await compareHandle(page).focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect(compareHandle(page)).toHaveAttribute("data-fraction", "0.54");
  await expect(page).toHaveURL(/[?&]swipe=0\.54/);
  // …then a real drag on the grip.
  const grip = page.locator(".swath-map-compare-grip");
  const box = await grip.boundingBox();
  const map = await page.locator("swath-map").boundingBox();
  if (!box || !map) {
    throw new Error("no grip/map geometry");
  }
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(map.x + map.width * 0.25, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  const fraction = Number(await compareHandle(page).getAttribute("data-fraction"));
  expect(fraction).toBeGreaterThan(0.2);
  expect(fraction).toBeLessThan(0.3);
  await expect(page).toHaveURL(/[?&]swipe=0\.2\d?/);
});

test("per-side x-ray badges: the trace stream's requested= splits the sides", async ({ page }) => {
  // Live NDVI renders on a shared 2-core CI runner: generous budget.
  test.setTimeout(240_000);
  const frames = await granuleFrames(page);
  const left = frames[0];
  const right = frames[frames.length - 1];
  if (left === undefined || right === undefined || left === right) {
    throw new Error("fire series has fewer than two frames");
  }

  // Load WITHOUT the compare, then start it: the overlay's SSE stream
  // opens at connect (before any fire tile is fetched), so every render
  // both sides trigger below is provably published while it listens —
  // the same fresh-renders-after-load convention as swath-xray.e2e.ts.
  await page.goto(`${DEMO_PATH}?xray&layer=${LAYER}&center=${CENTER}&zoom=${COMPARE_ZOOM}`);
  await waitForFittedView(page);
  await page.evaluate(
    ({ left, right }) => {
      const el = document.querySelector("swath-map");
      el?.setAttribute("datetime", left);
      el?.setAttribute("compare-datetime", right);
    },
    { left, right },
  );
  await expect(compareHandle(page)).toBeVisible();

  // Badges arrive per side as each side's renders trace in; every badge
  // in a side layer carries that side's tag and the fire layer's key.
  const leftBadges = page.locator('.swath-xray-side[data-side="left"] .swath-xray-badge');
  const rightBadges = page.locator('.swath-xray-side[data-side="right"] .swath-xray-badge');
  await expect(leftBadges.first()).toBeVisible({ timeout: 180_000 });
  await expect(rightBadges.first()).toBeVisible({ timeout: 180_000 });
  await expect(leftBadges.first()).toHaveAttribute("data-key", new RegExp(`^left:${LAYER}/`));
  await expect(rightBadges.first()).toHaveAttribute("data-key", new RegExp(`^right:${LAYER}/`));
});

test("the compare control starts before-vs-after and dismisses it", async ({ page }) => {
  const frames = await granuleFrames(page);
  const oldest = frames[0];
  const newest = frames[frames.length - 1];
  if (oldest === undefined || newest === undefined) {
    throw new Error("fire series has fewer than two frames");
  }

  await page.goto(`${DEMO_PATH}?layer=${LAYER}&center=${CENTER}&zoom=${COMPARE_ZOOM}`);
  await waitForFittedView(page);
  const button = page.getByRole("button", { name: "Toggle compare swipe" });
  // The control appears once the layer's time series (>= 2 frames) is
  // known — the same domain the slider shows.
  await expect(button).toBeVisible();
  await expect(button).toHaveAttribute("aria-pressed", "false");

  // Viewing "latest": one click pins newest right, jumps left to oldest,
  // and the share link carries the whole comparison.
  await button.click();
  await expect(compareHandle(page)).toBeVisible();
  await expect(compareHandle(page)).toHaveAttribute("data-mode", "date");
  await expect(page.locator('.swath-map-compare-label[data-side="left"]')).toHaveText(oldest);
  await expect(page.locator('.swath-map-compare-label[data-side="right"]')).toHaveText(newest);
  await expect(button).toHaveAttribute("aria-pressed", "true");
  await expect(page).toHaveURL(new RegExp(`[?&]t=${oldest.replaceAll(":", "\\:")}`));
  await expect(page).toHaveURL(new RegExp(`[?&]ct=${newest.replaceAll(":", "\\:")}`));

  // A second click ends the compare and clears it from the share link.
  await button.click();
  await expect(compareHandle(page)).toHaveCount(0);
  await expect(button).toHaveAttribute("aria-pressed", "false");
  await expect(page).not.toHaveURL(/[?&]ct=/);
  await expect(page).not.toHaveURL(/[?&]swipe=/);
});
