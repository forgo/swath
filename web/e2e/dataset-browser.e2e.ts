// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Data mode's catalog (issue #288, the dataset browser of #110 rebuilt on
// cards): lazy by contract, footprints on the map, zoom-to-granule, the
// ingest guidance, and thumbnails that are previews the ENGINE rendered
// (`POST /result`) — never a client-side decode (ADR 0019).
import { expect, type Page, test } from "@playwright/test";
import { DEMO_PATH, demoUrl, railMode, waitForFittedView } from "./support";

const dataMode = (page: Page) => railMode(page, "data");

const FOOTPRINTS_ID = "swath-granule-footprints";

const FIXTURE_GRANULES = {
  granules: [
    {
      id: "FIX.A.2026",
      bbox: [10.3, 45.6, 11.1, 46.4],
      datetime: "2026-06-01T10:00:00Z",
      assets: {},
    },
    {
      id: "FIX.B.2026",
      bbox: [11.0, 45.5, 12.0, 46.2],
      datetime: "2026-05-24T10:12:00Z",
      assets: {},
    },
  ],
  numberMatched: 2,
  numberReturned: 2,
  links: [],
};
const EMPTY_PAGE = { granules: [], numberMatched: 0, numberReturned: 0, links: [] };

const datasetSelect = (page: Page) => page.locator('swath-catalog [part="dataset"] select');
const card = (page: Page, id: string) =>
  page.locator(`swath-catalog swath-granule-card[data-granule="${id}"]`);
const cards = (page: Page) => page.locator("swath-catalog swath-granule-card");

function browsePath(url: string): string | undefined {
  const { pathname } = new URL(url);
  if (pathname.endsWith("/collections") || /\/datasets\/[^/]+\/granules$/.test(pathname)) {
    return pathname;
  }
  return undefined;
}

test("lazy by contract: zero browse requests until Data mode is entered; one listing, live granules", async ({
  page,
}) => {
  const hits: string[] = [];
  page.on("request", (request) => {
    const path = browsePath(request.url());
    if (path !== undefined) {
      hits.push(path);
    }
  });
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  // The map's own temporal domain reads are the only granule requests.
  expect(hits.filter((path) => path.endsWith("/collections"))).toEqual([]);
  expect(hits).toEqual(["/datasets/hls-s30/granules", "/datasets/hls-s30-fire/granules"]);
  await dataMode(page).click();
  await expect(datasetSelect(page)).toBeVisible();
  await expect.poll(() => hits.filter((path) => path.endsWith("/collections")).length).toBe(1);
  expect(hits.filter((path) => path.includes("granules"))).toHaveLength(2);
  await datasetSelect(page).selectOption("hls-s30");
  await expect(cards(page).first()).toBeVisible();
  expect(hits.filter((path) => path.includes("granules"))).toEqual([
    "/datasets/hls-s30/granules",
    "/datasets/hls-s30-fire/granules",
    "/datasets/hls-s30/granules",
  ]);
});

test("cards carry engine-rendered thumbnails (POST /result), never a client decode", async ({
  page,
}) => {
  const previews: string[] = [];
  page.on("request", (request) => {
    if (request.url().endsWith("/result") && request.method() === "POST") {
      previews.push(request.postData() ?? "");
    }
  });
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30");
  const first = cards(page).first();
  await expect(first).toBeVisible();
  await expect(first.locator('img[part="media"]')).toBeVisible({ timeout: 60_000 });
  expect(previews.length).toBeGreaterThan(0);
  expect(previews[0]).toContain('"load_collection"');
  expect(previews[0]).toContain('"spatial_extent"');
  const src = await first.locator('img[part="media"]').getAttribute("src");
  expect(src).toMatch(/^blob:/); // the engine's PNG, as an object URL
});

test("choosing a dataset renders footprint outlines as a MapLibre layer", async ({ page }) => {
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30");
  await expect(cards(page).first()).toBeVisible();
  await page.waitForFunction((id) => {
    const el = document.querySelector("swath-map") as {
      map?: {
        getLayer(id: string): { type?: string } | undefined;
        getSource(id: string): { serialize(): { data?: unknown } } | undefined;
      };
    } | null;
    const map = el?.map;
    if (!map?.getLayer(id) || map.getLayer(id)?.type !== "line") {
      return false;
    }
    const data = map.getSource(id)?.serialize().data as
      | { features?: { geometry?: { type?: string } }[] }
      | undefined;
    const features = data?.features ?? [];
    return features.length > 0 && features[0]?.geometry?.type === "Polygon";
  }, FOOTPRINTS_ID);
});

test("activating a card zooms the map to its footprint (fixtures); filters narrow the count", async ({
  page,
}) => {
  await page.route("**/datasets/hls-s30/granules*", (route) =>
    route.fulfill({ json: FIXTURE_GRANULES }),
  );
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30");
  await expect(card(page, "FIX.A.2026")).toBeVisible();
  await expect(page.locator('swath-catalog [part="count"]')).toHaveText("2 of 2 granules");
  await card(page, "FIX.A.2026").locator('swath-card [part="base"]').click();
  await page.waitForFunction(() => {
    const el = document.querySelector("swath-map") as {
      map?: {
        getCenter(): { lng: number; lat: number };
        getBounds(): { contains(point: [number, number]): boolean };
      };
    } | null;
    const map = el?.map;
    if (!map) {
      return false;
    }
    const center = map.getCenter();
    const bounds = map.getBounds();
    return (
      Math.abs(center.lng - 10.7) < 0.05 &&
      Math.abs(center.lat - 46.0) < 0.05 &&
      bounds.contains([10.3, 45.6]) &&
      bounds.contains([11.1, 46.4])
    );
  });
  // Date range: from June keeps only FIX.A; "in current view" (the map is
  // now on FIX.A) drops FIX.B too.
  await page.locator('swath-catalog swath-field[name="from"] input').fill("2026-06-01");
  await page.locator('swath-catalog swath-field[name="from"] input').dispatchEvent("change");
  await expect(page.locator('swath-catalog [part="count"]')).toHaveText("1 of 2 granules");
  await expect(card(page, "FIX.B.2026")).toHaveCount(0);
});

test("a dataset with no granules shows the ingest guidance (fixtures)", async ({ page }) => {
  await page.route("**/datasets/hls-s30/granules*", (route) => route.fulfill({ json: EMPTY_PAGE }));
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30");
  const empty = page.locator('swath-catalog [part="empty"]');
  await expect(empty).toBeVisible();
  await expect(empty).toContainText("No granules ingested yet");
  await expect(empty).toContainText("swath ingest");
  await expect(cards(page)).toHaveCount(0);
});
