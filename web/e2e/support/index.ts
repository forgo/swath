// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The one home for what the Playwright suites and the screenshot capture
 * share: the demo path and fixture ids, the map-settled waits, the trace
 * subscription, the rail/layer/slider locators and the authoring-panel
 * drivers (#349). Each helper existed as two to five hand-maintained
 * copies before; a copy that diverged (`subscribeToTraces` waiting for the
 * stream to open, `granuleFrames` sorting by instant) survives here as the
 * superset. */
import { expect, type Locator, type Page } from "@playwright/test";

/** `SWATH_E2E_MODE=binary` runs the suites against the embedded UI in the
 * release binary (`/` on :8080) instead of vite (`/demo/` on :5173);
 * `playwright.config.ts` derives `SWATH_DEMO_PATH` from it per mode. */
export const BINARY_MODE = process.env.SWATH_E2E_MODE === "binary";
export const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The fixture stack's fire-season layer and the dataset behind it. */
export const FIRE_LAYER = "park-fire-ndvi";
export const FIRE_DATASET = "hls-s30-fire";
/** A tile inside the HLS fixture footprint, `z/x/y`. */
export const TILE = "12/1561/848";

/** The subset of the live MapLibre instance the suites read through the
 * `swath-map` element's `map` property. */
export interface SwathMapLike {
  map?: {
    loaded(): boolean;
    areTilesLoaded(): boolean;
    getZoom(): number;
    getCenter(): { lng: number; lat: number };
    jumpTo(options: { zoom: number }): void;
    triggerRepaint(): void;
    once(event: string, listener: () => void): void;
    getCanvas(): HTMLCanvasElement;
  };
}

/** Waits for the zero-config bounds fit to land: the fitted footprint view
 * is deep, the boot view is zoom 1 — the same discriminator everywhere. */
export async function waitForFittedView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

/** Waits until the map has loaded and every visible tile has arrived. */
export async function waitForMapIdle(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded());
  });
}

/** The map's current view, read off the live MapLibre instance. */
export async function mapView(page: Page): Promise<{ lng: number; lat: number; zoom: number }> {
  return await page.evaluate(() => {
    const map = (document.querySelector("swath-map") as SwathMapLike | null)?.map;
    if (!map) {
      throw new Error("swath-map has no map instance");
    }
    const center = map.getCenter();
    return { lng: center.lng, lat: center.lat, zoom: map.getZoom() };
  });
}

/** The next 200 tile response for `layerId`. */
export function tileResponse(
  page: Page,
  layerId: string,
): Promise<{ url: string; contentType: string }> {
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

/** Navigates and waits for the first 200 tile of `layer`, then the fit. */
export async function gotoAndWaitForTiles(page: Page, url: string, layer: string): Promise<void> {
  const tile = page.waitForResponse(
    (r) => r.url().includes(`/tilesets/${layer}/tiles/`) && r.status() === 200,
  );
  await page.goto(url);
  await tile;
  await waitForFittedView(page);
}

// --- rail, layers, time, compare ---

/** A rail mode button (`layers`, `data`, `author`, `xray`, …). */
export const railMode = (page: Page, mode: string): Locator =>
  page.locator(`swath-rail [part="item"][data-mode="${mode}"]`);

/** A layer's row in the layer list (the click target). */
export const layerRow = (page: Page, layerId: string): Locator =>
  page.locator(`swath-layer-item[data-layer="${layerId}"] [part="row"]`);

/** The time-slider control and its play button. */
export const slider = (page: Page): Locator => page.locator(".swath-map-time");
export const playButton = (page: Page): Locator => page.locator(".swath-map-time-play");

/** The compare swipe's draggable handle. */
export const compareHandle = (page: Page): Locator =>
  page.locator("swath-map .swath-map-compare-handle");

/** Scrubs through the control's own range input (the user path). */
export async function scrubTo(page: Page, index: number): Promise<void> {
  await page
    .locator('.swath-map-time input[type="range"]')
    .evaluate((el: HTMLInputElement, value) => {
      el.value = String(value);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    }, index);
}

/** A dataset's frames straight from the granules API, ascending by
 * instant. */
export async function granuleFrames(page: Page, dataset = FIRE_DATASET): Promise<string[]> {
  const response = await page.request.get(`/datasets/${dataset}/granules`);
  expect(response.ok()).toBe(true);
  const body = (await response.json()) as { granules?: { datetime?: string }[] };
  return [
    ...new Set(
      (body.granules ?? [])
        .map((granule) => granule.datetime)
        .filter((value): value is string => typeof value === "string"),
    ),
  ].sort((a, b) => Date.parse(a) - Date.parse(b));
}

// --- the trace stream ---

/** The plan payload as swath-core pins it (the subset the assertions read). */
export interface ReceivedPlan {
  chosen: string | { overview: { factor: number } };
  considered: {
    strategy: string | { overview: { factor: number } };
    estimated_cost_bytes: number;
    admissible: boolean;
    reason: string;
  }[];
}

/** One `GET /traces` envelope — the union of the fields the suites read. */
export interface Envelope {
  tile: string;
  layer: string;
  trace: {
    decision: string | { overview: { level: number } } | { cache_hit: { key: string } };
    bytes_read: number;
    timings: { total_ms: number };
    ingest_to_pixel_ms: number | null;
    /** The planner's reasoning (#37); null on unplanned traces. */
    plan?: ReceivedPlan | null;
    /** Frame selection (ADR 0015); absent on layers without a time axis. */
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
    /** Every envelope the test's own subscription received. */
    __received?: Envelope[];
    /** Quiescence probe state (see the x-ray suite). */
    __quietLen?: number;
    __quietAt?: number;
  }
}

/** The test's own subscription — opened in the page so it shares the proxy
 * path (and origin) with the overlay's stream, and resolved only once the
 * stream is *open*, not merely constructed. That distinction is
 * load-bearing on CI: a fire view's initial load saturates the browser's
 * per-origin connection budget with ~28 tile requests, and a still-queued
 * EventSource misses every envelope published before it connects (SSE has
 * no replay). Locally the tiles drain fast enough to mask it. */
export async function subscribeToTraces(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        const received: Envelope[] = [];
        window.__received = received;
        const source = new EventSource("/traces");
        source.addEventListener("trace", (event) => {
          received.push(JSON.parse((event as MessageEvent<string>).data) as Envelope);
        });
        source.addEventListener("open", () => resolve(), { once: true });
      }),
  );
}

/** Latest received envelope per `"layer/z/x/y"` key — the same latest-wins
 * reduction the overlay's store performs. */
export function latestByKey(received: Envelope[]): Map<string, Envelope> {
  const latest = new Map<string, Envelope>();
  for (const envelope of received) {
    latest.set(`${envelope.layer}/${envelope.tile}`, envelope);
  }
  return latest;
}

// --- the authoring panel ---

/** Selects step `key`'s chip in the strip if it is not already pressed. */
export async function ensureStep(page: Page, key: string): Promise<void> {
  const chip = page.locator(`.swath-authoring-chip[data-chip="${key}"]`);
  if ((await chip.count()) > 0 && (await chip.getAttribute("aria-pressed")) !== "true") {
    await chip.click();
  }
}

/** A field of the authoring form by id (`s1-id`, `s4-inputMin`, …).
 * Selecting the step's chip is a side effect of addressing the field: a
 * locator proxy that switches steps before the first action on it. */
export function fieldById(page: Page, id: string): Locator {
  const key = /^(s\d+)-/.exec(id)?.[1];
  const locator = page.locator(`#swath-authoring-${id}`);
  if (key === undefined) {
    return locator;
  }
  return new Proxy(locator, {
    get(target, prop, receiver) {
      const value = Reflect.get(target, prop, receiver);
      if (typeof value !== "function") {
        return value;
      }
      return async (...args: unknown[]) => {
        await ensureStep(page, key);
        return (value as (...a: unknown[]) => unknown).apply(target, args);
      };
    },
  });
}

/** The stage-typed insert chip for `processId` at `gap` (0 = right after
 * the Load card). */
export function chip(page: Page, gap: number, processId: string): Locator {
  return page.locator(
    `.swath-authoring-insert[data-gap="${gap}"] button[data-process="${processId}"]`,
  );
}

/** The panel is collapsed and lazy (fetches nothing until opened): every
 * flow starts by entering author mode and toggling it open. The permanent
 * Load card (s1) rendering means the canvas is ready. */
export async function openAuthoringPanel(page: Page): Promise<void> {
  await railMode(page, "author").click();
  await page.locator("swath-authoring-panel .swath-authoring-toggle").click();
  await ensureStep(page, "s1");
  await expect(page.locator('[data-step="s1"]')).toBeVisible();
}

export const submitButton = (page: Page): Locator => page.locator(".swath-authoring-submit");
