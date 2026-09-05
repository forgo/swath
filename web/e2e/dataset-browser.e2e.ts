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

// --- The timeline (#411) ---

const timeline = (page: Page) => page.locator("swath-catalog swath-timeline");
const buckets = (page: Page) => timeline(page).locator('[part="bucket"]');

test("the timeline's bands come from the counts endpoint, and a drag becomes a shareable date chip", async ({
  page,
}) => {
  const counts: string[] = [];
  page.on("request", (request) => {
    const { pathname, search } = new URL(request.url());
    if (pathname.endsWith("/counts")) {
      counts.push(`${pathname}${search}`);
    }
  });
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30-fire");

  // The bands are the server's counts, not the fetched page.
  await expect(timeline(page)).toBeVisible();
  await expect.poll(() => counts.length).toBeGreaterThan(0);
  expect(counts[0]).toContain("step=month");
  await expect(timeline(page).locator('[part="hint"]')).toHaveText("Drag to narrow the dates");
  // With nothing filtered the control says so rather than drawing an
  // empty second band.
  await expect(timeline(page).locator('[part="note"]')).toContainText("no filters");

  // Picking a bucket narrows the dates, and the narrowing is in the URL.
  const first = buckets(page).first();
  await expect(first).toBeVisible();
  const label = await first.getAttribute("aria-label");
  const day = (label ?? "").slice(0, 10);
  await first.click();
  await expect.poll(() => new URL(page.url()).searchParams.get("from")).toBe(day);
  await expect(page.locator('swath-chip-row [data-chip="dates"]')).toBeVisible();
  // The second band is a scoped count, never a client-side subtraction.
  await expect
    .poll(() => counts.filter((path) => path.includes("datetime=")).length)
    .toBeGreaterThan(0);

  // And it is navigation: back returns to the unfiltered scope.
  await page.goBack();
  await expect.poll(() => new URL(page.url()).searchParams.get("from")).toBeNull();
});

// --- The scope tag (#412) ---

const areaField = (page: Page) => page.locator('swath-catalog swath-field[name="area"] input');
const scopeLine = (page: Page) => page.locator('swath-catalog [part="scope"]');

test("the search field states its scope, and a pasted shape is reduced to its box and says so", async ({
  page,
}) => {
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30");

  // No spatial filter, no tag — the tag is never a decoration.
  await expect(scopeLine(page)).toHaveCount(0);

  await areaField(page).fill("-106, 39, -105, 40");
  await areaField(page).blur();
  await expect(scopeLine(page)).toContainText("bbox");
  await expect(scopeLine(page)).toContainText("Searching the box you gave.");

  // A pasted polygon: the tag stays `bbox` — the port has no intersects —
  // and the copy says which box is being used.
  await areaField(page).fill(
    JSON.stringify({
      type: "Polygon",
      coordinates: [
        [
          [-106, 39],
          [-105, 39.5],
          [-105.5, 40],
          [-106, 39],
        ],
      ],
    }),
  );
  await areaField(page).blur();
  await expect(scopeLine(page)).toContainText("Using the bounding box of the shape you pasted.");

  // And the derived box is on the map, so the reduction is seen.
  await expect
    .poll(async () =>
      page.evaluate((id) => {
        const map = document.querySelector("swath-map") as {
          map?: { getLayer(i: string): unknown };
        };
        return map.map?.getLayer(id) !== undefined;
      }, "swath-search-scope"),
    )
    .toBe(true);
});

// --- Results: hover draws, focus draws too (#413) ---

test("hovering a result draws its footprint, and the keyboard reaches the same thing", async ({
  page,
}) => {
  await page.goto(demoUrl({ view: "data" }));
  await waitForFittedView(page);
  await datasetSelect(page).selectOption("hls-s30");
  const first = cards(page).first();
  await expect(first).toBeVisible();

  const painted = async (): Promise<number> =>
    page.evaluate((id) => {
      const element = document.querySelector("swath-map") as {
        map?: { getSource(i: string): { serialize(): { data?: unknown } } | undefined };
      } | null;
      const data = element?.map?.getSource(id)?.serialize().data as
        | { features?: unknown[] }
        | undefined;
      return data?.features?.length ?? 0;
    }, "swath-granule-hover");

  await first.hover();
  await expect.poll(painted).toBe(1);
  // Leaving clears it rather than leaving a stale outline behind.
  await page.mouse.move(5, 5);
  await expect.poll(painted).toBe(0);

  // The focus ring draws the same footprint a pointer does: the card's
  // own focusable surface, which is what Tab reaches.
  await first.locator("swath-card").focus();
  await expect.poll(painted).toBe(1);
});
