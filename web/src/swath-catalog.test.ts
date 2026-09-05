// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathApi } from "./api.js";
import { GRANULES_EMPTY_GUIDANCE } from "./catalog-model.js";
import { clearThumbnailCache, defineSwathCatalog, type SwathCatalog } from "./swath-catalog.js";
import { createSwathEvent } from "./ui/events.js";

const SERVER = "https://swath.test";
const PNG = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function stub(
  options: { granules?: unknown; facets?: unknown; counts?: unknown; result?: () => Response } = {},
) {
  const requests: string[] = [];
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = String(input);
    const method = init?.method ?? "GET";
    requests.push(`${method} ${new URL(url).pathname}`);
    if (url.endsWith("/collections")) {
      return json({
        collections: [
          {
            id: "hls-s30",
            title: "HLS S30",
            "cube:dimensions": { bands: { type: "bands", values: ["b02", "b03", "b04"] } },
          },
          { id: "empty-set", title: "Nothing yet" },
        ],
      });
    }
    if (url.includes("/datasets/hls-s30/granules")) {
      return json(
        options.granules ?? {
          granules: [
            { id: "FIX.A.2026", bbox: [10.3, 45.6, 11.1, 46.4], datetime: "2026-06-01T10:00:00Z" },
            { id: "FIX.B.2026", bbox: [11.0, 45.5, 12.0, 46.2], datetime: "2026-05-24T10:12:00Z" },
          ],
        },
      );
    }
    if (url.includes("/counts")) {
      return options.counts === undefined ? json({ total: 0, buckets: [] }) : json(options.counts);
    }
    if (url.includes("/facets")) {
      return options.facets === undefined ? json({ total: 0, facets: [] }) : json(options.facets);
    }
    if (url.includes("/datasets/empty-set/granules")) {
      return json({ granules: [] });
    }
    if (url.endsWith("/result") && method === "POST") {
      return options.result
        ? options.result()
        : new Response(new Blob([PNG], { type: "image/png" }), { status: 200 });
    }
    return new Response("not scripted", { status: 404 });
  }) as typeof fetch;
  return { impl, requests };
}

beforeAll(() => {
  defineSwathCatalog();
});

afterEach(() => {
  document.body.replaceChildren();
  clearThumbnailCache();
});

async function mount(s: ReturnType<typeof stub>): Promise<SwathCatalog> {
  const catalog = document.createElement("swath-catalog");
  catalog.api = new SwathApi({ base: SERVER, fetch: s.impl });
  document.body.append(catalog);
  await catalog.updateComplete;
  return catalog;
}

const cards = (catalog: SwathCatalog) => [
  ...(catalog.shadowRoot?.querySelectorAll("swath-granule-card") ?? []),
];
const settle = () => new Promise((r) => setTimeout(r, 30));

test("lazy by contract: nothing is fetched until active; then one listing", async () => {
  const s = stub();
  const catalog = await mount(s);
  await settle();
  expect(s.requests).toEqual([]);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  expect(s.requests).toEqual(["GET /collections"]);
  expect(catalog.datasets.map((d) => d.id)).toEqual(["hls-s30", "empty-set"]);
});

test("selecting a dataset fetches its granules live, announces footprints, renders cards with engine thumbnails", async () => {
  const s = stub();
  const catalog = await mount(s);
  const announced: string[][] = [];
  catalog.addEventListener("swath-dataset-granules", (e) =>
    announced.push(e.detail.granules.map((g) => g.id)),
  );
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;
  expect(announced).toEqual([["FIX.A.2026", "FIX.B.2026"]]);
  expect(cards(catalog).map((c) => c.dataset["granule"])).toEqual(["FIX.A.2026", "FIX.B.2026"]); // newest first
  await settle();
  expect(s.requests.filter((r) => r === "POST /result")).toHaveLength(2);
  for (const c of cards(catalog)) {
    await c.updateComplete;
    expect(c.thumbnail).toMatch(/^blob:/);
  }
  await catalog.select("hls-s30"); // re-select: re-fetches (a cache would lie), thumbnails cached
  await settle();
  expect(s.requests.filter((r) => r.endsWith("/granules"))).toHaveLength(2);
  expect(s.requests.filter((r) => r === "POST /result")).toHaveLength(2);
});

test("a refused preview shows the server's words on the card, never a client decode", async () => {
  const s = stub({
    result: () =>
      json({ code: "BudgetExceeded", message: "preview exceeds the pixel budget" }, 400),
  });
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;
  await settle();
  const first = cards(catalog)[0];
  await first?.updateComplete;
  expect(first?.note).toBe("no preview — preview exceeds the pixel budget");
  expect(first?.shadowRoot?.querySelector('[part="note"]')?.textContent).toContain("pixel budget");
});

test("zoom on activation; empty guidance; count + sort + filters", async () => {
  const s = stub();
  const catalog = await mount(s);
  const zooms: string[] = [];
  catalog.addEventListener("swath-granule-zoom", (e) =>
    zooms.push(`${e.detail.dataset}:${e.detail.id}`),
  );
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;
  const first = cards(catalog)[0];
  first?.shadowRoot
    ?.querySelector("swath-card")
    ?.shadowRoot?.querySelector<HTMLElement>('[part="base"]')
    ?.click();
  expect(zooms).toEqual(["hls-s30:FIX.A.2026"]);
  expect(catalog.shadowRoot?.querySelector('[part="count"]')?.textContent).toBe("2 of 2 granules");
  catalog.viewBounds = { west: 11.5, south: 45, east: 13, north: 47 }; // only FIX.B intersects
  const toggle = catalog.shadowRoot?.querySelector("swath-toggle");
  toggle?.shadowRoot?.querySelector<HTMLButtonElement>('[part="control"]')?.click();
  await catalog.updateComplete;
  expect(cards(catalog).map((c) => c.dataset["granule"])).toEqual(["FIX.B.2026"]);
  expect(catalog.shadowRoot?.querySelector('[part="count"]')?.textContent).toBe("1 of 2 granules");
  await catalog.select("empty-set");
  await catalog.updateComplete;
  expect(catalog.shadowRoot?.querySelector('[part="empty"]')?.textContent).toBe(
    GRANULES_EMPTY_GUIDANCE,
  );
});

test("facets are the collection's own: a key is offered only because a granule carries it", async () => {
  const s = stub({
    facets: {
      total: 2,
      facets: [
        { key: "eo:cloud_cover", kind: "number", coverage: 2, min: 0, max: 42 },
        {
          key: "platform",
          kind: "string",
          coverage: 1,
          values: [{ value: "sentinel-2a", count: 1 }],
        },
      ],
    },
  });
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;

  const block = catalog.shadowRoot?.querySelector('[part="facets"]');
  expect(block?.textContent).toContain("eo:cloud_cover");
  expect(block?.textContent).toContain("0 – 42");
  // Coverage below the total keeps "absent" distinguishable from "zero".
  expect(block?.textContent).toContain("on 1 of 2");
  expect(block?.textContent).toContain("on every granule");
});

test("a collection whose items carry nothing renders no facet block at all", async () => {
  const s = stub();
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;
  expect(catalog.shadowRoot?.querySelector('[part="facets"]')).toBeNull();
});

test("a facets failure costs the facets, never the granule list", async () => {
  const s = stub({ facets: null });
  const impl = (async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> =>
    String(input).includes("/facets")
      ? new Response("nope", { status: 500 })
      : s.impl(input, init)) as typeof fetch;
  const catalog = await mount({ impl, requests: s.requests });
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;
  expect(cards(catalog)).toHaveLength(2);
  expect(catalog.shadowRoot?.querySelector('[part="facets"]')).toBeNull();
});

const MONTHS = {
  total: 150,
  buckets: [
    { start: "2024-01-01T00:00:00Z", end: "2024-02-01T00:00:00Z", count: 100 },
    { start: "2024-02-01T00:00:00Z", end: "2024-03-01T00:00:00Z", count: 50 },
  ],
};

test("the timeline's bands both come from the counts endpoint, not from the page", async () => {
  const s = stub({ counts: MONTHS });
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;

  // Two granules were fetched; the bands say 150, because that is what the
  // server counted.
  const timeline = catalog.shadowRoot?.querySelector("swath-timeline");
  expect(timeline).not.toBeNull();
  await timeline?.updateComplete;
  expect(timeline?.shadowRoot?.querySelector('[part="note"]')?.textContent).toContain("150");
  expect(s.requests.filter((r) => r.endsWith("/counts"))).toHaveLength(1);
});

test("dragging the timeline narrows the dates and re-asks for the surviving band", async () => {
  const s = stub({ counts: MONTHS });
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;

  const dates: { from: string | null; to: string | null }[] = [];
  catalog.addEventListener("swath-dates", (event) => dates.push(event.detail));
  const timeline = catalog.shadowRoot?.querySelector("swath-timeline");
  await timeline?.updateComplete;
  const bar = timeline?.shadowRoot?.querySelector('[part="bucket"]');
  bar?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  await settle();
  await catalog.updateComplete;

  expect(dates).toEqual([{ from: "2024-01-01", to: "2024-01-31" }]);
  // The second band is asked for with the dates in force — a scoped count,
  // never a client-side subtraction.
  const scoped = s.requests.filter((r) => r.endsWith("/counts"));
  expect(scoped.length).toBeGreaterThan(1);
});

test("a collection with no counts renders no timeline at all", async () => {
  const s = stub();
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await catalog.select("hls-s30");
  await catalog.updateComplete;
  expect(catalog.shadowRoot?.querySelector("swath-timeline")).toBeNull();
});

const AREA = (catalog: SwathCatalog) =>
  catalog.shadowRoot?.querySelector<HTMLElement & { value: string }>('swath-field[name="area"]') ??
  undefined;

const scopeLine = (catalog: SwathCatalog) =>
  catalog.shadowRoot?.querySelector('[part="scope"]')?.textContent ?? "";

async function typeArea(catalog: SwathCatalog, text: string): Promise<void> {
  AREA(catalog)?.dispatchEvent(createSwathEvent("swath-change", { name: "area", value: text }));
  await catalog.updateComplete;
}

test("no spatial filter, no tag — and the viewport toggle names itself when it is the filter", async () => {
  const s = stub();
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  expect(catalog.shadowRoot?.querySelector('[part="scope"]')).toBeNull();

  const toggle = catalog.shadowRoot?.querySelector('swath-toggle[name="in-view"]');
  toggle?.dispatchEvent(createSwathEvent("swath-change", { name: "in-view", value: true }));
  await catalog.updateComplete;
  expect(scopeLine(catalog)).toContain("viewport");
});

test("a pasted box is used as given; a pasted shape is reduced and says so", async () => {
  const s = stub();
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();

  const scopes: { mode: string | null; bbox: number[] | null }[] = [];
  catalog.addEventListener("swath-scope", (event) => scopes.push(event.detail));

  await typeArea(catalog, "-106, 39, -105, 40");
  expect(scopeLine(catalog)).toContain("bbox");
  expect(scopeLine(catalog)).toContain("Searching the box you gave.");
  expect(scopes.at(-1)).toEqual({ mode: "bbox", bbox: [-106, 39, -105, 40] });

  await typeArea(
    catalog,
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
  expect(scopeLine(catalog)).toContain("Using the bounding box of the shape you pasted.");
  // The box the server actually gets is the envelope — announced, so the
  // map can draw it.
  expect(scopes.at(-1)).toEqual({ mode: "bbox", bbox: [-106, 39, -105, 40] });
});

test("an unusable paste is an error, never a filter that quietly does nothing", async () => {
  const s = stub();
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();

  const scopes: { bbox: number[] | null }[] = [];
  catalog.addEventListener("swath-scope", (event) => scopes.push(event.detail));
  await typeArea(catalog, "denver");
  expect(catalog.shadowRoot?.querySelector('[part="scope-error"]')?.textContent).toContain(
    "west, south, east, north",
  );
  expect(scopes.at(-1)?.bbox).toBeNull();
  // And no tag, because no spatial filter is in force.
  expect(catalog.shadowRoot?.querySelector('[part="scope-tag"]')).toBeNull();
});

test("clearing the area drops the filter and the tag with it", async () => {
  const s = stub();
  const catalog = await mount(s);
  catalog.active = true;
  await catalog.updateComplete;
  await settle();
  await typeArea(catalog, "-106, 39, -105, 40");
  expect(catalog.shadowRoot?.querySelector('[part="scope-tag"]')).not.toBeNull();
  await typeArea(catalog, "");
  expect(catalog.shadowRoot?.querySelector('[part="scope"]')).toBeNull();
});
