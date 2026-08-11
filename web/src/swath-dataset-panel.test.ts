// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The dataset browser's contract (issue #110) over a counting stubbed
// fetch, in the swath-xray test pattern (real Custom Elements in a real
// browser, no network): laziness is asserted as a request COUNT — zero
// while the panel is closed, exactly one listing per open, exactly one
// granule fetch per dataset expand. The real-network proof is
// web/e2e/dataset-browser.e2e.ts.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import type { GranuleBbox } from "./granule-footprints.js";
import {
  defineSwathDatasetPanel,
  GRANULES_EMPTY_GUIDANCE,
  type GranuleListItem,
  SwathDatasetPanel,
} from "./swath-dataset-panel.js";

const SERVER = "https://swath.test";

const COLLECTIONS = {
  collections: [{ id: "hls-s30", title: "Harmonized Landsat Sentinel-2" }, { id: "empty-ds" }],
};

const GRANULES = {
  granules: [
    {
      id: "HLS.S30.T13SDD.2024152T175909",
      bbox: [-106.1, 39.2, -105.9, 39.4],
      datetime: "2024-06-01T17:59:09Z",
      assets: {},
    },
    {
      id: "HLS.S30.T13SDD.2024144T175911",
      bbox: [-106.2, 39.1, -106.0, 39.3],
      datetime: "2024-05-24T17:59:11Z",
      assets: {},
    },
  ],
  numberMatched: 2,
  numberReturned: 2,
  links: [],
};

const EMPTY_PAGE = { granules: [], numberMatched: 0, numberReturned: 0, links: [] };

function json(body: object): Response {
  return new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } });
}

/** Stubs fetch with the granule-API shape and counts every request. */
function stubApi(overrides: Record<string, () => Response> = {}): { requests: string[] } {
  const requests: string[] = [];
  vi.stubGlobal("fetch", (input: RequestInfo | URL): Promise<Response> => {
    const url = input instanceof Request ? input.url : String(input);
    const path = new URL(url).pathname;
    requests.push(path);
    const override = overrides[path];
    if (override) {
      return Promise.resolve(override());
    }
    if (path === "/collections") {
      return Promise.resolve(json(COLLECTIONS));
    }
    if (path === "/datasets/hls-s30/granules") {
      return Promise.resolve(json(GRANULES));
    }
    if (path === "/datasets/empty-ds/granules") {
      return Promise.resolve(json(EMPTY_PAGE));
    }
    return Promise.resolve(new Response("not found", { status: 404 }));
  });
  return { requests };
}

function mount(): SwathDatasetPanel {
  const panel = document.createElement("swath-dataset-panel") as SwathDatasetPanel;
  panel.setAttribute("server", SERVER);
  document.body.append(panel);
  return panel;
}

function toggle(panel: SwathDatasetPanel): HTMLButtonElement {
  const button = panel.querySelector<HTMLButtonElement>(".swath-dataset-panel-toggle");
  if (!button) {
    throw new Error("no panel toggle");
  }
  return button;
}

function datasetButton(panel: SwathDatasetPanel, id: string): HTMLButtonElement {
  const button = panel.querySelector<HTMLButtonElement>(`button[data-dataset="${id}"]`);
  if (!button) {
    throw new Error(`no dataset button for ${id}`);
  }
  return button;
}

async function openPanel(panel: SwathDatasetPanel): Promise<void> {
  toggle(panel).click();
  await panel.ready;
}

async function expandDataset(panel: SwathDatasetPanel, id: string): Promise<void> {
  datasetButton(panel, id).click();
  await panel.ready;
}

beforeAll(() => {
  defineSwathDatasetPanel();
});

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

test("registers exactly once and upgrades with an accessible role", () => {
  stubApi();
  defineSwathDatasetPanel(); // second call must be a no-op
  expect(customElements.get(SwathDatasetPanel.tagName)).toBe(SwathDatasetPanel);
  const panel = mount();
  expect(panel.getAttribute("role")).toBe("group");
  expect(panel.getAttribute("aria-label")).toBe("Datasets");
  expect(toggle(panel).getAttribute("aria-expanded")).toBe("false");
});

test("lazy by contract: a closed panel issues zero requests", async () => {
  const { requests } = stubApi();
  const panel = mount();
  await panel.ready;
  expect(requests).toEqual([]); // nothing at mount

  // Opening fetches the listing exactly once; nothing granule-shaped.
  await openPanel(panel);
  expect(requests).toEqual(["/collections"]);
  expect(requests.filter((path) => path.includes("granules"))).toEqual([]);
});

test("opening lists every dataset with title and id", async () => {
  stubApi();
  const panel = mount();
  await openPanel(panel);
  expect(toggle(panel).getAttribute("aria-expanded")).toBe("true");
  const buttons = [...panel.querySelectorAll<HTMLButtonElement>("button[data-dataset]")];
  expect(buttons.map((b) => b.dataset["dataset"])).toEqual(["hls-s30", "empty-ds"]);
  expect(buttons[0]?.querySelector(".swath-dataset-panel-title")?.textContent).toBe(
    "Harmonized Landsat Sentinel-2",
  );
  expect(buttons[0]?.querySelector(".swath-dataset-panel-id")?.textContent).toBe("hls-s30");
  expect(buttons[1]?.querySelector(".swath-dataset-panel-title")?.textContent).toBe("empty-ds");
  expect(buttons.map((b) => b.getAttribute("aria-expanded"))).toEqual(["false", "false"]);
});

test("expanding fetches granules once, renders them, announces footprints", async () => {
  const { requests } = stubApi();
  const panel = mount();
  const announced: { dataset: string; granules: GranuleListItem[] }[] = [];
  panel.addEventListener("swath-dataset-granules", (event) => {
    announced.push((event as CustomEvent<{ dataset: string; granules: GranuleListItem[] }>).detail);
  });
  await openPanel(panel);
  await expandDataset(panel, "hls-s30");

  expect(requests.filter((path) => path.includes("granules"))).toEqual([
    "/datasets/hls-s30/granules",
  ]);
  expect(datasetButton(panel, "hls-s30").getAttribute("aria-expanded")).toBe("true");
  const rows = [...panel.querySelectorAll<HTMLButtonElement>("button[data-granule]")];
  expect(rows.map((row) => row.dataset["granule"])).toEqual([
    "HLS.S30.T13SDD.2024152T175909",
    "HLS.S30.T13SDD.2024144T175911",
  ]);
  expect(rows[0]?.textContent).toContain("2024-06-01T17:59:09Z");

  expect(announced).toHaveLength(1);
  expect(announced[0]?.dataset).toBe("hls-s30");
  expect(announced[0]?.granules.map((granule) => granule.bbox)).toEqual([
    [-106.1, 39.2, -105.9, 39.4],
    [-106.2, 39.1, -106.0, 39.3],
  ]);
});

test("clicking a granule announces a zoom with its bbox", async () => {
  stubApi();
  const panel = mount();
  await openPanel(panel);
  await expandDataset(panel, "hls-s30");

  const zoomed = new Promise<{ dataset: string; id: string; bbox: GranuleBbox }>((resolve) => {
    panel.addEventListener(
      "swath-granule-zoom",
      (event) => {
        resolve((event as CustomEvent<{ dataset: string; id: string; bbox: GranuleBbox }>).detail);
      },
      { once: true },
    );
  });
  panel
    .querySelector<HTMLButtonElement>('button[data-granule="HLS.S30.T13SDD.2024144T175911"]')
    ?.click();
  const detail = await zoomed;
  expect(detail.dataset).toBe("hls-s30");
  expect(detail.id).toBe("HLS.S30.T13SDD.2024144T175911");
  expect(detail.bbox).toEqual([-106.2, 39.1, -106.0, 39.3]);
});

test("a dataset with no granules renders the ingest guidance", async () => {
  stubApi();
  const panel = mount();
  await openPanel(panel);
  await expandDataset(panel, "empty-ds");
  const empty = panel.querySelector(".swath-dataset-panel-empty");
  expect(empty?.textContent).toBe(GRANULES_EMPTY_GUIDANCE);
  expect(empty?.textContent).toContain("swath ingest"); // points at the ingest command
  expect(panel.querySelectorAll("button[data-granule]")).toHaveLength(0);
});

test("collapsing a dataset (or the panel) announces empty footprints", async () => {
  stubApi();
  const panel = mount();
  const announced: { dataset: string; granules: GranuleListItem[] }[] = [];
  panel.addEventListener("swath-dataset-granules", (event) => {
    announced.push((event as CustomEvent<{ dataset: string; granules: GranuleListItem[] }>).detail);
  });
  await openPanel(panel);
  await expandDataset(panel, "hls-s30");
  expect(announced).toHaveLength(1);

  // Collapse the dataset: footprints must clear.
  await expandDataset(panel, "hls-s30");
  expect(announced).toHaveLength(2);
  expect(announced[1]).toEqual({ dataset: "", granules: [] });
  expect(datasetButton(panel, "hls-s30").getAttribute("aria-expanded")).toBe("false");

  // Expand again, then close the whole panel: same clearing contract.
  await expandDataset(panel, "hls-s30");
  toggle(panel).click();
  expect(announced).toHaveLength(4);
  expect(announced[3]).toEqual({ dataset: "", granules: [] });
  expect(toggle(panel).getAttribute("aria-expanded")).toBe("false");
});

test("re-expanding re-fetches: granules arrive live, a cache would lie", async () => {
  const { requests } = stubApi();
  const panel = mount();
  await openPanel(panel);
  await expandDataset(panel, "hls-s30");
  await expandDataset(panel, "hls-s30"); // collapse
  await expandDataset(panel, "hls-s30"); // expand again
  expect(requests.filter((path) => path.includes("granules"))).toHaveLength(2);
});

test("listing failure renders an error state, not a blank panel", async () => {
  stubApi({ "/collections": () => new Response("boom", { status: 500 }) });
  const panel = mount();
  await openPanel(panel);
  const error = panel.querySelector(".swath-dataset-panel-error");
  expect(error?.textContent).toContain("Dataset list unavailable");
  expect(error?.textContent).toContain("500");
});

test("granule failure renders an error state under the dataset", async () => {
  stubApi({ "/datasets/hls-s30/granules": () => new Response("boom", { status: 500 }) });
  const panel = mount();
  await openPanel(panel);
  await expandDataset(panel, "hls-s30");
  const error = panel.querySelector(".swath-dataset-panel-error");
  expect(error?.textContent).toContain("Granules unavailable");
  expect(panel.querySelectorAll("button[data-granule]")).toHaveLength(0);
});

test("granules with malformed bboxes are skipped, not painted wrong", async () => {
  stubApi({
    "/datasets/hls-s30/granules": () =>
      json({
        granules: [
          { id: "good", bbox: [1, 2, 3, 4], datetime: "2024-06-01T00:00:00Z" },
          { id: "short", bbox: [1, 2, 3], datetime: "2024-06-01T00:00:00Z" },
          { id: "strings", bbox: ["a", "b", "c", "d"], datetime: "2024-06-01T00:00:00Z" },
        ],
        numberMatched: 3,
        numberReturned: 3,
        links: [],
      }),
  });
  const panel = mount();
  const announced: GranuleListItem[][] = [];
  panel.addEventListener("swath-dataset-granules", (event) => {
    announced.push((event as CustomEvent<{ granules: GranuleListItem[] }>).detail.granules);
  });
  await openPanel(panel);
  await expandDataset(panel, "hls-s30");
  expect(announced[0]?.map((granule) => granule.id)).toEqual(["good"]);
});
