// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The dataset browser (issue #110) in real chromium, in both modes
// (playwright.config.ts):
//
// - Laziness rides the REAL stack and is asserted as a network COUNT:
//   zero /collections and granule requests until the panel opens, one
//   listing per open, one granule fetch per dataset expand — and the
//   expanded granules paint a real MapLibre footprint layer.
// - Zoom-to-footprint and the empty state run over routed FIXTURES
//   (page.route), so the asserted geometry and the asserted guidance
//   string are deterministic regardless of what the stack has ingested.
import { expect, type Page, test } from "@playwright/test";

const DEMO_PATH = process.env.SWATH_DEMO_PATH ?? "/demo/";

/** The footprint source/layer id (src/granule-footprints.ts). */
const FOOTPRINTS_ID = "swath-granule-footprints";

/** Two granules over the Alps — far from the demo dataset's Colorado
 * footprint, so the zoom assertion cannot pass by accident. */
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

/** Same fitted-view discriminator as the landing suite. */
async function waitForFittedView(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const el = document.querySelector("swath-map") as {
      map?: { loaded(): boolean; areTilesLoaded(): boolean; getZoom(): number };
    } | null;
    const map = el?.map;
    return Boolean(map?.loaded() && map.areTilesLoaded() && map.getZoom() > 5);
  });
}

function panelToggle(page: Page) {
  return page.locator("swath-dataset-panel .swath-dataset-panel-toggle");
}

function datasetButton(page: Page, id: string) {
  return page.locator(`swath-dataset-panel button[data-dataset="${id}"]`);
}

/** Is a request one the dataset browser owns? (Counted for laziness.) */
function browsePath(url: string): string | undefined {
  const { pathname } = new URL(url);
  if (pathname.endsWith("/collections") || /\/datasets\/[^/]+\/granules$/.test(pathname)) {
    return pathname;
  }
  return undefined;
}

test("granule fetch is lazy: zero browse requests until the panel opens", async ({ page }) => {
  const hits: string[] = [];
  page.on("request", (request) => {
    const path = browsePath(request.url());
    if (path !== undefined) {
      hits.push(path);
    }
  });

  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  // The page is fully up — map, tiles, layer panel — and the closed
  // dataset browser has added NOTHING to the request log. The one
  // granules request that IS here belongs to the map, not the panel:
  // since issue #182 every catalog-backed layer apply reads its
  // dataset's granule listing once — the time slider's domain (which
  // for this single-date layer resolves to "hidden").
  await expect(panelToggle(page)).toBeVisible();
  expect(hits.filter((path) => path.endsWith("/collections"))).toEqual([]);
  expect(hits).toEqual(["/datasets/hls-s30/granules"]);

  // Opening fetches the dataset listing exactly once, still no further
  // granules requests — the panel stays lazy.
  await panelToggle(page).click();
  await expect(datasetButton(page, "hls-s30")).toBeVisible();
  expect(hits.filter((path) => path.endsWith("/collections"))).toHaveLength(1);
  expect(hits.filter((path) => path.includes("granules"))).toHaveLength(1);

  // Expanding the dataset fetches its granules exactly once more.
  await datasetButton(page, "hls-s30").click();
  await expect(page.locator("swath-dataset-panel button[data-granule]").first()).toBeVisible();
  expect(hits.filter((path) => path.includes("granules"))).toEqual([
    "/datasets/hls-s30/granules",
    "/datasets/hls-s30/granules",
  ]);
});

test("expanding renders footprint outlines as a MapLibre layer", async ({ page }) => {
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  await panelToggle(page).click();
  await datasetButton(page, "hls-s30").click();
  await expect(page.locator("swath-dataset-panel button[data-granule]").first()).toBeVisible();

  // A real line layer over a GeoJSON source carrying the granule's
  // footprint polygon. `serialize()` reads the data the source was given —
  // deterministic where querySourceFeatures depends on tile/render state.
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

test("clicking a granule zooms the map to its footprint (fixtures)", async ({ page }) => {
  await page.route("**/datasets/hls-s30/granules*", (route) =>
    route.fulfill({ json: FIXTURE_GRANULES }),
  );
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  await panelToggle(page).click();
  await datasetButton(page, "hls-s30").click();
  await expect(page.locator('button[data-granule="FIX.A.2026"]')).toBeVisible();

  await page.locator('button[data-granule="FIX.A.2026"]').click();
  // The view fits the fixture bbox: centered on it, both corners inside.
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
});

test("a dataset with no granules shows the ingest guidance (fixtures)", async ({ page }) => {
  await page.route("**/datasets/hls-s30/granules*", (route) => route.fulfill({ json: EMPTY_PAGE }));
  await page.goto(DEMO_PATH);
  await waitForFittedView(page);
  await panelToggle(page).click();
  await datasetButton(page, "hls-s30").click();

  const empty = page.locator("swath-dataset-panel .swath-dataset-panel-empty");
  await expect(empty).toBeVisible();
  await expect(empty).toContainText("No granules ingested yet");
  await expect(empty).toContainText("swath ingest"); // points at the ingest command
  await expect(page.locator("swath-dataset-panel button[data-granule]")).toHaveCount(0);
});
