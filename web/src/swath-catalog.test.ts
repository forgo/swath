// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { SwathApi } from "./api.js";
import { GRANULES_EMPTY_GUIDANCE } from "./catalog-model.js";
import { clearThumbnailCache, defineSwathCatalog, type SwathCatalog } from "./swath-catalog.js";

const SERVER = "https://swath.test";
const PNG = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function stub(options: { granules?: unknown; result?: () => Response } = {}) {
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
