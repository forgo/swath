// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Runs in a REAL browser (Vitest Browser Mode + Playwright): actual Custom
// Elements, actual MapLibre GL over actual WebGL. The Swath API is stubbed
// by patching `globalThis.fetch`, so these tests pin the component's
// contract with the server — most critically the tile URL template's OGC
// ordering — without a network. The real-server proof is `just e2e-web`.
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vitest";
import { defineSwathMap, SwathMap } from "./swath-map.js";

const SERVER = "https://swath.test";
const OTHER_SERVER = "https://other.test";

/** 1x1 transparent PNG, so stubbed tile requests decode cleanly. */
const TINY_PNG = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
  ),
  (char) => char.codePointAt(0) ?? 0,
);

/** The e2e stack's fixture footprint (tests/e2e/swath-catalog.toml). */
const BBOX = { west: -106.1, south: 39.2, east: -105.9, north: 39.4 };

function tilesetItem(base: string, id: string, title: string): object {
  return {
    title,
    dataType: "map",
    crs: "http://www.opengis.net/def/crs/EPSG/0/3857",
    tileMatrixSetURI: "http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad",
    links: [{ href: `${base}/tilesets/${id}`, rel: "self", type: "application/json" }],
  };
}

function json(body: object): Response {
  return new Response(JSON.stringify(body), {
    headers: { "content-type": "application/json" },
  });
}

/** The Park Fire fixture series' acquisition datetimes (ascending) —
 * the temporal domain the opt-in `temporal` stub serves. */
const FIRE_FRAMES = [
  "2024-06-07T19:03:00Z",
  "2024-07-22T19:03:00Z",
  "2024-08-16T19:03:00Z",
  "2024-09-05T19:03:00Z",
];

/** A stub Swath API over `globalThis.fetch`: the tilesets list, per-layer
 * metadata (with the fixture bbox), and PNG bytes for tile requests.
 * `temporal` opts one layer into the time dimension (issue #182): its
 * metadata carries the `granules` link and the dataset's granule listing
 * answers with `FIRE_FRAMES` (newest first, like the real API). */
function stubSwathApi(
  opts: {
    tilesLiveAfterMs?: number;
    serverLiveAfterMs?: number;
    temporal?: { layer: string; dataset: string };
  } = {},
): {
  requests: string[];
} {
  const requests: string[] = [];
  const liveAt = opts.tilesLiveAfterMs === undefined ? 0 : Date.now() + opts.tilesLiveAfterMs;
  const serverLiveAt =
    opts.serverLiveAfterMs === undefined ? 0 : Date.now() + opts.serverLiveAfterMs;
  vi.stubGlobal("fetch", (input: RequestInfo | URL): Promise<Response> => {
    const url = input instanceof Request ? input.url : String(input);
    requests.push(url);
    if (Date.now() < serverLiveAt) {
      return Promise.resolve(new Response("bad gateway", { status: 502 }));
    }
    if (url.includes("/tiles/") && Date.now() < liveAt) {
      return Promise.resolve(new Response("no granule yet", { status: 404 }));
    }
    if (/\/tilesets$/.test(url)) {
      const base = new URL(url).origin;
      return Promise.resolve(
        json({
          tilesets: [
            tilesetItem(base, "truecolor", "HLS true color"),
            tilesetItem(base, "ndvi", "HLS NDVI"),
          ],
        }),
      );
    }
    if (/\/tilesets\/[a-z0-9-]+$/.test(url)) {
      const { temporal } = opts;
      const links =
        temporal !== undefined && url.endsWith(`/tilesets/${temporal.layer}`)
          ? [
              {
                href: `${new URL(url).origin}/datasets/${temporal.dataset}/granules`,
                rel: "granules",
                type: "application/json",
              },
            ]
          : [];
      return Promise.resolve(
        json({
          title: "stub",
          dataType: "map",
          boundingBox: {
            lowerLeft: [BBOX.west, BBOX.south],
            upperRight: [BBOX.east, BBOX.north],
          },
          links,
        }),
      );
    }
    if (url.includes("/granules")) {
      // Newest first, like the real listing; the component sorts.
      return Promise.resolve(
        json({
          granules: [...FIRE_FRAMES]
            .reverse()
            .map((datetime, i) => ({ id: `g${i}`, datetime, bbox: [0, 0, 1, 1], assets: {} })),
        }),
      );
    }
    if (url.includes("/tiles/")) {
      return Promise.resolve(
        new Response(TINY_PNG.slice().buffer, { headers: { "content-type": "image/png" } }),
      );
    }
    if (url.endsWith("/basemap-style.json")) {
      return Promise.resolve(
        json({
          version: 8,
          sources: {
            base: { type: "raster", tiles: ["https://basemap.example/{z}/{x}/{y}.png"] },
          },
          layers: [{ id: "base", type: "raster", source: "base" }],
        }),
      );
    }
    return Promise.reject(new Error(`unstubbed fetch: ${url}`));
  });
  return { requests };
}

/** The raster tile templates of the component's current map style. */
function tileTemplates(el: SwathMap): string[] {
  const sources = el.map?.getStyle().sources ?? {};
  return Object.values(sources).flatMap((source) =>
    source.type === "raster" ? (source.tiles ?? []) : [],
  );
}

function mount(attributes: Record<string, string>): SwathMap {
  const el = document.createElement("swath-map") as SwathMap;
  for (const [name, value] of Object.entries(attributes)) {
    el.setAttribute(name, value);
  }
  el.style.width = "320px";
  el.style.height = "240px";
  document.body.append(el);
  return el;
}

beforeAll(() => {
  defineSwathMap();
});

beforeEach(() => {
  stubSwathApi();
});

afterEach(() => {
  for (const el of document.querySelectorAll("swath-map")) {
    el.remove(); // disposes the WebGL context (disconnectedCallback)
  }
  vi.unstubAllGlobals();
});

test("registers exactly once and upgrades", () => {
  defineSwathMap(); // second call must be a no-op, not a registry error
  expect(customElements.get(SwathMap.tagName)).toBe(SwathMap);
  const el = mount({ server: SERVER });
  expect(el).toBeInstanceOf(SwathMap);
  expect(el.map).toBeDefined();
});

test("tile template is OGC-ordered: {z}/{y}/{x}, never {z}/{x}/{y}", async () => {
  // The mirror of the API-side #27 ordering test: our path is
  // `{tileMatrix}/{tileRow}/{tileCol}` = z/y/x, so MapLibre's XYZ-named
  // template MUST put `{y}` (row) in the middle segment.
  const el = mount({ server: SERVER, layer: "truecolor" });
  await el.ready;
  expect(tileTemplates(el)).toEqual([`${SERVER}/tilesets/truecolor/tiles/{z}/{y}/{x}`]);
  expect(tileTemplates(el)[0]).not.toContain("{z}/{x}/{y}");
});

test("zero-config: picks the first layer and fits the tileset bounds", async () => {
  const el = mount({ server: SERVER }); // no layer/center/zoom
  await el.ready;
  expect(tileTemplates(el)).toEqual([`${SERVER}/tilesets/truecolor/tiles/{z}/{y}/{x}`]);
  const center = el.map?.getCenter();
  expect(center?.lng).toBeGreaterThan(BBOX.west);
  expect(center?.lng).toBeLessThan(BBOX.east);
  expect(center?.lat).toBeGreaterThan(BBOX.south);
  expect(center?.lat).toBeLessThan(BBOX.north);
  expect(el.map?.getZoom()).toBeGreaterThan(4); // a ~0.2 degree footprint is deep
});

test("explicit center/zoom win over the bounds fit", async () => {
  const el = mount({ server: SERVER, layer: "truecolor", center: "10,20", zoom: "3" });
  await el.ready;
  expect(el.map?.getCenter().lng).toBeCloseTo(10, 5);
  expect(el.map?.getCenter().lat).toBeCloseTo(20, 5);
  expect(el.map?.getZoom()).toBeCloseTo(3, 5);
});

test("layer and server attributes are reactive", async () => {
  const el = mount({ server: SERVER, layer: "truecolor" });
  await el.ready;

  el.setAttribute("layer", "ndvi");
  await el.ready;
  expect(tileTemplates(el)).toEqual([`${SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}`]);

  el.setAttribute("server", OTHER_SERVER);
  await el.ready;
  expect(tileTemplates(el)).toEqual([`${OTHER_SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}`]);
});

test("center/zoom attributes are reactive", async () => {
  const el = mount({ server: SERVER, layer: "truecolor", center: "0,0", zoom: "2" });
  await el.ready;
  el.setAttribute("center", "-100,40");
  el.setAttribute("zoom", "5");
  expect(el.map?.getCenter().lng).toBeCloseTo(-100, 5);
  expect(el.map?.getCenter().lat).toBeCloseTo(40, 5);
  expect(el.map?.getZoom()).toBeCloseTo(5, 5);
});

test("layers() returns the server's tilesets list", async () => {
  const el = mount({ server: SERVER, layer: "truecolor" });
  await el.ready;
  expect(await el.layers()).toEqual([
    { id: "truecolor", title: "HLS true color" },
    { id: "ndvi", title: "HLS NDVI" },
  ]);
});

test("setLayer() swaps the source, reflects the attribute, fires layerchange", async () => {
  const el = mount({ server: SERVER, layer: "truecolor" });
  await el.ready;
  const layerchange = new Promise<string>((resolve) => {
    el.addEventListener(
      "layerchange",
      (event) => resolve((event as CustomEvent<{ layer: string }>).detail.layer),
      { once: true },
    );
  });
  await el.setLayer("ndvi");
  expect(el.getAttribute("layer")).toBe("ndvi");
  expect(tileTemplates(el)).toEqual([`${SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}`]);
  expect(await layerchange).toBe("ndvi");
});

test("built-in switcher renders accessible buttons with aria-pressed", async () => {
  const el = mount({ server: SERVER, layer: "truecolor" });
  await el.ready;
  const buttons = [...el.querySelectorAll<HTMLButtonElement>(".swath-map-switcher button")];
  expect(buttons.map((button) => button.textContent)).toEqual(["HLS true color", "HLS NDVI"]);
  expect(buttons.map((button) => button.getAttribute("aria-pressed"))).toEqual(["true", "false"]);

  buttons[1]?.click();
  await el.ready;
  const pressed = [...el.querySelectorAll<HTMLButtonElement>(".swath-map-switcher button")].map(
    (button) => button.getAttribute("aria-pressed"),
  );
  expect(pressed).toEqual(["false", "true"]);
});

test('switcher="off" omits the built-in control for hosts with their own layer UI', async () => {
  const el = mount({ server: SERVER, layer: "truecolor", switcher: "off" });
  await el.ready;
  expect(el.querySelector(".swath-map-switcher")).toBeNull();
  // The x-ray toggle is unaffected by the switcher opt-out.
  expect(el.querySelector(".swath-map-xray-toggle")).not.toBeNull();
});

test("basemap style merges with the swath raster layer painted on top", async () => {
  // The demo-page fix (post-#35): without a basemap, everything outside the
  // fixture footprint is blank void; with one, the imagery gets a world for
  // context. The merge must keep our raster LAST (on top) and untouched.
  stubSwathApi();
  const el = mount({
    server: SERVER,
    layer: "truecolor",
    basemap: `${SERVER}/basemap-style.json`,
  });
  await el.ready;
  const style = el.map?.getStyle();
  expect(style?.layers.map((l) => l.id)).toEqual(["base", "swath"]);
  expect(Object.keys(style?.sources ?? {})).toEqual(expect.arrayContaining(["base", "swath"]));
  expect(tileTemplates(el)).toContain(`${SERVER}/tilesets/truecolor/tiles/{z}/{y}/{x}`);
});

test("basemap fetch failure degrades to the bare style, never blocks imagery", async () => {
  stubSwathApi();
  const el = mount({
    server: SERVER,
    layer: "truecolor",
    basemap: `${SERVER}/no-such-style.json`,
  });
  await el.ready;
  expect(el.map?.getStyle().layers.map((l) => l.id)).toEqual(["swath"]);
});

test("tiles that 404 are retried automatically until data appears", {
  timeout: 15_000,
}, async () => {
  // The stopwatch-demo flow: viewer open BEFORE the granule exists. The
  // component must recover on its own once tiles go live — no reload, no
  // map-nudging (MapLibre never refetches a tile it saw fail).
  const { requests } = stubSwathApi({ tilesLiveAfterMs: 4_000 });
  const el = mount({ server: SERVER, layer: "truecolor" });
  await el.ready;
  // Every tile request 404s for the first 4s (viewer opened pre-drop), then
  // the layer goes live. The component's liveness probe must notice and
  // re-apply with a bumped source version (`?v=n`) so MapLibre — which never
  // refetches a failed tile — paints the imagery with no user action.
  await vi.waitFor(
    () => expect(requests.some((u) => u.includes("/tiles/") && u.includes("?v="))).toBe(true),
    { timeout: 12_000, interval: 250 },
  );
  el.remove();
});

test("a server that is still starting is retried until it comes up", {
  timeout: 15_000,
}, async () => {
  // The demo prints its URL during the docker build: the page loads while
  // every request 502s. The component must keep retrying the whole apply
  // (layer resolution AND basemap) instead of going permanently blank.
  stubSwathApi({ serverLiveAfterMs: 4_000 });
  const el = mount({ server: SERVER, layer: "truecolor" });
  el.ready.catch(() => undefined); // first apply is EXPECTED to fail
  await vi.waitFor(
    () => expect(tileTemplates(el)).toContain(`${SERVER}/tilesets/truecolor/tiles/{z}/{y}/{x}`),
    { timeout: 12_000, interval: 250 },
  );
  el.remove();
});

test("a failing server rejects ready and fires a swath-error event", async () => {
  vi.stubGlobal("fetch", (): Promise<Response> => {
    return Promise.resolve(new Response("down", { status: 503 }));
  });
  const errored = new Promise<boolean>((resolve) => {
    document.body.addEventListener("swath-error", () => resolve(true), { once: true });
  });
  const el = mount({ server: SERVER, layer: "truecolor" });
  await expect(el.ready).rejects.toThrow(/503/);
  expect(await errored).toBe(true);
});

// --- the time slider + datetime attribute (issue #182) ---

test("time slider: visible with the granules domain on a temporal layer, hidden otherwise", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "ndvi" });
  await el.ready;
  const slider = el.querySelector<HTMLElement>(".swath-map-time");
  expect(slider?.hidden).toBe(false);
  expect(slider?.dataset["frames"]).toBe("4");
  // No datetime attribute = latest: the thumb rests on the last frame.
  expect(slider?.dataset["index"]).toBe("3");
  expect(slider?.dataset["datetime"]).toBe(FIRE_FRAMES[3]);

  // Switching to a layer without a granules link hides the control —
  // the zero-config landing (single-date fixture layers) is untouched.
  await el.setLayer("truecolor");
  expect(el.querySelector<HTMLElement>(".swath-map-time")?.hidden).toBe(true);
});

test("datetime attribute re-points the raster source with datetime= and fires swath-timechange", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "ndvi" });
  await el.ready;
  expect(tileTemplates(el)).toEqual([`${SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}`]);

  const frame = FIRE_FRAMES[1] ?? "";
  const timechange = new Promise<string | null>((resolve) => {
    el.addEventListener(
      "swath-timechange",
      (event) => resolve((event as CustomEvent<{ datetime: string | null }>).detail.datetime),
      { once: true },
    );
  });
  el.setAttribute("datetime", frame);
  expect(await timechange).toBe(frame);
  // The source was re-pointed in place — same style, new frame.
  expect(tileTemplates(el)).toEqual([
    `${SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}?datetime=${encodeURIComponent(frame)}`,
  ]);
  // The slider thumb mirrors the attribute.
  expect(el.querySelector<HTMLElement>(".swath-map-time")?.dataset["index"]).toBe("1");
});

test("a deep-linked datetime is baked into the first applied template", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const frame = FIRE_FRAMES[2] ?? "";
  const el = mount({ server: SERVER, layer: "ndvi", datetime: frame });
  await el.ready;
  expect(tileTemplates(el)).toEqual([
    `${SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}?datetime=${encodeURIComponent(frame)}`,
  ]);
  expect(el.querySelector<HTMLElement>(".swath-map-time")?.dataset["index"]).toBe("2");
});

test("scrubbing the built-in slider reflects the datetime attribute", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "ndvi" });
  await el.ready;
  const range = el.querySelector<HTMLInputElement>('.swath-map-time input[type="range"]');
  if (!range) {
    throw new Error("no slider range input");
  }
  range.value = "0";
  range.dispatchEvent(new Event("input"));
  expect(el.getAttribute("datetime")).toBe(FIRE_FRAMES[0]);
  expect(tileTemplates(el)[0]).toContain(`datetime=${encodeURIComponent(FIRE_FRAMES[0] ?? "")}`);
});
