// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Runs in a REAL browser (Vitest Browser Mode + Playwright): actual Custom
// Elements, actual MapLibre GL over actual WebGL. The Swath API is stubbed
// by patching `globalThis.fetch`, so these tests pin the component's
// contract with the server — most critically the tile URL template's OGC
// ordering — without a network. The real-server proof is `just e2e-web`.
import { afterEach, beforeAll, beforeEach, expect, test, vi } from "vitest";
import { defineSwathMap, SwathMap } from "./swath-map.js";
import type { EventSourceLike } from "./swath-xray.js";
import { PLAY_INTERVAL_MS } from "./time-slider.js";

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

/** The fire fixture footprint the opt-in `temporal` stub's granules
 * carry — far from the default `BBOX`, so auto-frame is observable. */
const FIRE_BBOX = { west: -121.7388, south: 39.9856, east: -121.6475, north: 40.0559 };

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
    /** Serve tileset metadata with no bounds and no links — a layer
     * whose data footprint is unknowable (zoom-to-data must hide). */
    bare?: boolean;
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
      if (opts.bare) {
        return Promise.resolve(json({ title: "stub", dataType: "map" }));
      }
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
          granules: [...FIRE_FRAMES].reverse().map((datetime, i) => ({
            id: `g${i}`,
            datetime,
            bbox: [FIRE_BBOX.west, FIRE_BBOX.south, FIRE_BBOX.east, FIRE_BBOX.north],
            assets: {},
          })),
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
  vi.useRealTimers();
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

// --- the compare swipe (issue #210) ---

/** The compare (right-side) map's raster tile templates. */
function compareTemplates(el: SwathMap): string[] {
  const sources = el.compareMap?.getStyle().sources ?? {};
  return Object.values(sources).flatMap((source) =>
    source.type === "raster" ? (source.tiles ?? []) : [],
  );
}

test("compare-datetime builds the clipped right map on the other frame", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "ndvi", datetime: FIRE_FRAMES[0] ?? "" });
  await el.ready;
  expect(el.compareMap).toBeUndefined();
  expect(el.querySelector(".swath-map-compare")).toBeNull();

  const comparechange = new Promise<void>((resolve) => {
    el.addEventListener("swath-comparechange", () => resolve(), { once: true });
  });
  el.setAttribute("compare-datetime", FIRE_FRAMES[3] ?? "");
  await comparechange;

  // The right map exists, view-synced, on the compare frame's template.
  await vi.waitFor(() =>
    expect(compareTemplates(el)).toEqual([
      `${SERVER}/tilesets/ndvi/tiles/{z}/{y}/{x}?datetime=${encodeURIComponent(
        FIRE_FRAMES[3] ?? "",
      )}`,
    ]),
  );
  expect(el.compareMap?.getCenter().lng).toBeCloseTo(el.map?.getCenter().lng ?? 0, 6);
  expect(el.compareMap?.getZoom()).toBeCloseTo(el.map?.getZoom() ?? 0, 6);
  // The left map still shows ITS frame — untouched by the compare.
  expect(tileTemplates(el)[0]).toContain(`datetime=${encodeURIComponent(FIRE_FRAMES[0] ?? "")}`);

  // Handle + per-side chips: date mode shows the two frames.
  const handle = el.querySelector<HTMLElement>(".swath-map-compare-handle");
  expect(handle?.dataset["mode"]).toBe("date");
  expect(handle?.getAttribute("role")).toBe("slider");
  expect(el.querySelector('.swath-map-compare-label[data-side="left"]')?.textContent).toBe(
    FIRE_FRAMES[0],
  );
  expect(el.querySelector('.swath-map-compare-label[data-side="right"]')?.textContent).toBe(
    FIRE_FRAMES[3],
  );
  // Default handle position: centered, clip showing the right half.
  expect(handle?.style.left).toBe("50%");
  expect(el.querySelector<HTMLElement>(".swath-map-compare")?.style.clipPath).toBe(
    "inset(0px 0px 0px 50%)",
  );

  // Ending the compare tears the right side down entirely.
  el.removeAttribute("compare-datetime");
  expect(el.compareMap).toBeUndefined();
  expect(el.querySelector(".swath-map-compare")).toBeNull();
  expect(el.querySelector(".swath-map-compare-handle")).toBeNull();
});

test("compare-layer builds the right map on the other layer", async () => {
  const el = mount({ server: SERVER, layer: "ndvi", "compare-layer": "truecolor" });
  await el.ready;
  await vi.waitFor(() =>
    expect(compareTemplates(el)).toEqual([`${SERVER}/tilesets/truecolor/tiles/{z}/{y}/{x}`]),
  );
  const handle = el.querySelector<HTMLElement>(".swath-map-compare-handle");
  expect(handle?.dataset["mode"]).toBe("layer");
  expect(el.querySelector('.swath-map-compare-label[data-side="left"]')?.textContent).toBe("ndvi");
  expect(el.querySelector('.swath-map-compare-label[data-side="right"]')?.textContent).toBe(
    "truecolor",
  );
  // Comparing a layer with itself is dropped — no right map, no handle.
  el.setAttribute("compare-layer", "ndvi");
  expect(el.compareMap).toBeUndefined();
  expect(el.querySelector(".swath-map-compare-handle")).toBeNull();
});

test("swipe attribute moves the handle; arrow keys move the swipe attribute", async () => {
  const el = mount({
    server: SERVER,
    layer: "ndvi",
    "compare-layer": "truecolor",
    swipe: "0.25",
  });
  await el.ready;
  const handle = el.querySelector<HTMLElement>(".swath-map-compare-handle");
  expect(handle?.style.left).toBe("25%");
  expect(handle?.getAttribute("aria-valuenow")).toBe("25");

  el.setAttribute("swipe", "0.75");
  expect(handle?.style.left).toBe("75%");
  expect(el.querySelector<HTMLElement>(".swath-map-compare")?.style.clipPath).toBe(
    "inset(0px 0px 0px 75%)",
  );

  // Keyboard: the handle is a real slider — a nudge reflects into the
  // attribute (the single source of truth), which moves the handle.
  handle?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
  expect(el.getAttribute("swipe")).toBe("0.77");
  expect(handle?.style.left).toBe("77%");
  handle?.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
  expect(el.getAttribute("swipe")).toBe("0");
});

test("the compare toggle starts before-vs-after on a time series and ends any compare", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "ndvi" });
  await el.ready;
  const button = el.querySelector<HTMLButtonElement>(".swath-map-compare-toggle button");
  expect(button?.closest<HTMLElement>(".swath-map-compare-toggle")?.hidden).toBe(false);
  expect(button?.getAttribute("aria-pressed")).toBe("false");

  // Viewing "latest": the toggle pins newest right, jumps left to oldest.
  button?.click();
  expect(el.getAttribute("datetime")).toBe(FIRE_FRAMES[0]);
  expect(el.getAttribute("compare-datetime")).toBe(FIRE_FRAMES[3]);
  expect(button?.getAttribute("aria-pressed")).toBe("true");
  expect(el.querySelector(".swath-map-compare-handle")).not.toBeNull();

  // Toggling again clears every compare attribute.
  button?.click();
  expect(el.getAttribute("compare-datetime")).toBeNull();
  expect(el.getAttribute("swipe")).toBeNull();
  expect(el.querySelector(".swath-map-compare-handle")).toBeNull();
  expect(button?.getAttribute("aria-pressed")).toBe("false");

  // A single-date layer cannot offer the default gesture: hidden button.
  await el.setLayer("truecolor");
  expect(el.querySelector<HTMLElement>(".swath-map-compare-toggle")?.hidden).toBe(true);
});

// --- finding the data: auto-frame + zoom-to-data (issue #182 follow-up) ---

/** Resolves on the next user-initiated data framing (the shell's
 * URL-sync seam doubles as the tests' completion signal). */
function framed(el: SwathMap): Promise<void> {
  return new Promise((resolve) => {
    el.addEventListener("swath-framedata", () => resolve(), { once: true });
  });
}

test("a user-initiated switch auto-frames data that is nowhere in view", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  // Viewing the other side of the world...
  const el = mount({ server: SERVER, layer: "truecolor", center: "10,20", zoom: "4" });
  await el.ready;
  // ...the user switches to the fire layer (setLayer = the user path:
  // the built-in switcher and the entry page's rail both go through it).
  const frame = framed(el);
  await el.setLayer("ndvi");
  await frame;
  const center = el.map?.getCenter();
  expect(center?.lng).toBeGreaterThan(FIRE_BBOX.west);
  expect(center?.lng).toBeLessThan(FIRE_BBOX.east);
  expect(center?.lat).toBeGreaterThan(FIRE_BBOX.south);
  expect(center?.lat).toBeLessThan(FIRE_BBOX.north);
});

test("deep-linked views are honored: attribute-driven applies never auto-frame", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  // The shell applies a deep link's layer/center/zoom as attributes —
  // the URL's view must win even though the fire data is off-screen.
  const el = mount({ server: SERVER, layer: "ndvi", center: "10,20", zoom: "4" });
  await el.ready;
  expect(el.map?.getCenter().lng).toBeCloseTo(10, 5);
  expect(el.map?.getCenter().lat).toBeCloseTo(20, 5);
  // A programmatic attribute write is not a user switch either.
  el.setAttribute("layer", "truecolor");
  await el.ready;
  el.setAttribute("layer", "ndvi");
  await el.ready;
  expect(el.map?.getCenter().lng).toBeCloseTo(10, 5);
  expect(el.map?.getCenter().lat).toBeCloseTo(20, 5);
});

test("no auto-frame when the data already intersects the view", async () => {
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "truecolor", center: "-121.7,40.02", zoom: "9" });
  await el.ready;
  await el.setLayer("ndvi");
  // The fire footprint is on screen: the view is not yanked.
  expect(el.map?.getCenter().lng).toBeCloseTo(-121.7, 5);
  expect(el.map?.getCenter().lat).toBeCloseTo(40.02, 5);
  expect(el.map?.getZoom()).toBeCloseTo(9, 5);
});

test("zoom to data: frames known bounds; hidden when nothing is known", async () => {
  // A static layer (no granules link): the metadata bounds are the
  // fallback footprint, so the control shows and frames them.
  const el = mount({ server: SERVER, layer: "truecolor", center: "10,20", zoom: "3" });
  await el.ready;
  const control = el.querySelector<HTMLElement>(".swath-map-zoomdata");
  expect(control?.hidden).toBe(false);
  const frame = framed(el);
  control?.querySelector("button")?.click();
  await frame;
  const center = el.map?.getCenter();
  expect(center?.lng).toBeGreaterThan(BBOX.west);
  expect(center?.lng).toBeLessThan(BBOX.east);
  expect(center?.lat).toBeGreaterThan(BBOX.south);
  expect(center?.lat).toBeLessThan(BBOX.north);
  el.remove();

  // No bounds, no granules — no dead button.
  stubSwathApi({ bare: true });
  const unknowable = mount({ server: SERVER, layer: "truecolor", center: "10,20", zoom: "3" });
  await unknowable.ready;
  expect(unknowable.querySelector<HTMLElement>(".swath-map-zoomdata")?.hidden).toBe(true);
});

// --- the cinematic landing (issue #211) ---

/** Fakes only the play loop's clock: MapLibre's own rAF/timeouts stay
 * real so the map still loads and settles under `await el.ready`. */
function fakePlayClock(): void {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
}

/** The landing card and the slider's play button of a mounted map. */
function landing(el: SwathMap): { card: HTMLElement; play: HTMLButtonElement } {
  const card = el.querySelector<HTMLElement>(".swath-map-landing");
  const play = el.querySelector<HTMLButtonElement>(".swath-map-time-play");
  if (!card || !play) {
    throw new Error("landing chrome missing");
  }
  return { card, play };
}

/** Every `swath-timechange` the map fires, as `[datetime, cinematic]`. */
function recordTimechanges(el: SwathMap): [string | null, boolean][] {
  const seen: [string | null, boolean][] = [];
  el.addEventListener("swath-timechange", (event) => {
    const detail = (event as CustomEvent<{ datetime: string | null; cinematic: boolean }>).detail;
    seen.push([detail.datetime, detail.cinematic]);
  });
  return seen;
}

/** The loop only advances over a painted frame (`canAdvance`), so wait
 * for the map's tiles before expecting a tick to land. */
async function waitForTiles(el: SwathMap): Promise<void> {
  await vi.waitFor(() => {
    expect(el.map?.areTilesLoaded()).toBe(true);
  });
}

function scrub(el: SwathMap, index: number): void {
  const range = el.querySelector<HTMLInputElement>('.swath-map-time input[type="range"]');
  if (!range) {
    throw new Error("no slider range input");
  }
  range.value = String(index);
  range.dispatchEvent(new Event("input"));
}

test("cinematic: the playable layer beats the first tileset, and the loop plays itself", async () => {
  fakePlayClock();
  const { requests } = stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, cinematic: "" });
  const changes = recordTimechanges(el);
  await el.ready;

  // The listing says truecolor first; the landing picked the series —
  // and the apply reused the scan's granules read (one, not two).
  expect(tileTemplates(el)[0]).toContain("/tilesets/ndvi/tiles/");
  expect(requests.filter((url) => url.includes("/granules"))).toHaveLength(1);
  const { card, play } = landing(el);
  expect(card.hidden).toBe(false);
  expect(card.dataset["state"]).toBe("playing");
  expect(card.textContent).toContain("HLS NDVI — 4 frames");
  expect(play.getAttribute("aria-pressed")).toBe("true");

  // A tick advances the frame (latest wraps to oldest) and the event
  // says it was the landing's own doing — the shell leaves the URL alone.
  await waitForTiles(el);
  vi.advanceTimersByTime(PLAY_INTERVAL_MS);
  expect(el.getAttribute("datetime")).toBe(FIRE_FRAMES[0]);
  expect(changes.at(-1)).toEqual([FIRE_FRAMES[0], true]);

  // Hover pauses (no ticks land), leaving resumes.
  el.dispatchEvent(new PointerEvent("pointerenter", { pointerType: "mouse" }));
  expect(play.getAttribute("aria-pressed")).toBe("false");
  expect(card.dataset["state"]).toBe("hover");
  vi.advanceTimersByTime(3 * PLAY_INTERVAL_MS);
  expect(el.getAttribute("datetime")).toBe(FIRE_FRAMES[0]);
  el.dispatchEvent(new PointerEvent("pointerleave", { pointerType: "mouse" }));
  expect(play.getAttribute("aria-pressed")).toBe("true");
  expect(card.dataset["state"]).toBe("playing");

  // A scrub takes over: the frame change is the user's, the loop is
  // theirs (still running — the slider's own controls decide), and a
  // later hover no longer touches it.
  scrub(el, 2);
  expect(card.dataset["state"]).toBe("over");
  expect(changes.at(-1)).toEqual([FIRE_FRAMES[2], false]);
  expect(play.getAttribute("aria-pressed")).toBe("true");
  el.dispatchEvent(new PointerEvent("pointerenter", { pointerType: "mouse" }));
  expect(play.getAttribute("aria-pressed")).toBe("true");
  await waitForTiles(el);
  vi.advanceTimersByTime(PLAY_INTERVAL_MS);
  expect(changes.at(-1)).toEqual([FIRE_FRAMES[3], false]);
});

test("cinematic: reduced motion waits with a play affordance, and that affordance hands over", async () => {
  fakePlayClock();
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("prefers-reduced-motion"),
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false,
  }));
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, cinematic: "" });
  const changes = recordTimechanges(el);
  await el.ready;

  const { card, play } = landing(el);
  expect(card.dataset["state"]).toBe("reduced");
  expect(play.getAttribute("aria-pressed")).toBe("false");
  const affordance = el.querySelector<HTMLButtonElement>(".swath-map-landing-play");
  expect(affordance?.hidden).toBe(false);
  await waitForTiles(el);
  vi.advanceTimersByTime(3 * PLAY_INTERVAL_MS);
  expect(changes).toEqual([]); // nothing moved on its own

  affordance?.click();
  expect(play.getAttribute("aria-pressed")).toBe("true");
  expect(card.dataset["state"]).toBe("over");
  vi.advanceTimersByTime(PLAY_INTERVAL_MS);
  expect(changes.at(-1)).toEqual([FIRE_FRAMES[0], false]); // the user's loop
});

test("cinematic: a drag pauses and hands over; the invitation turns x-ray on and retires the card", async () => {
  fakePlayClock();
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, cinematic: "" });
  const sources: string[] = [];
  el.xrayEventSource = (url): EventSourceLike => {
    sources.push(url);
    return { addEventListener: () => undefined, close: () => undefined };
  };
  await el.ready;
  const { card, play } = landing(el);
  expect(play.getAttribute("aria-pressed")).toBe("true");

  // A user-driven move (MapLibre stamps drags/wheels with originalEvent).
  el.map?.fire("movestart", { originalEvent: new MouseEvent("mousedown") });
  expect(play.getAttribute("aria-pressed")).toBe("false");
  expect(card.dataset["state"]).toBe("over");
  expect(card.hidden).toBe(false); // the invitation outlives the loop
  await waitForTiles(el);
  vi.advanceTimersByTime(3 * PLAY_INTERVAL_MS);
  expect(play.getAttribute("aria-pressed")).toBe("false");

  // "watch the machine work": the overlay comes up, the card steps aside.
  el.querySelector<HTMLButtonElement>(".swath-map-landing-invite")?.click();
  expect(el.hasAttribute("xray")).toBe(true);
  expect(sources).toEqual([`${SERVER}/traces`]);
  expect(card.hidden).toBe(true);
  el.removeAttribute("xray");
  expect(card.hidden).toBe(false);
});

test("cinematic: with no playable layer, today's landing — first tileset, nothing plays, no card", async () => {
  const el = mount({ server: SERVER, cinematic: "" });
  await el.ready;
  expect(tileTemplates(el)[0]).toContain("/tilesets/truecolor/tiles/");
  const { card, play } = landing(el);
  expect(card.hidden).toBe(true);
  expect(card.dataset["state"]).toBe("off");
  expect(play.getAttribute("aria-pressed")).toBe("false");
});

test("without the cinematic attribute a playable layer never plays itself (deep links)", async () => {
  fakePlayClock();
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, layer: "ndvi", center: "-121.7,40.02", zoom: "12" });
  const changes = recordTimechanges(el);
  await el.ready;
  const { card, play } = landing(el);
  expect(card.hidden).toBe(true);
  expect(play.getAttribute("aria-pressed")).toBe("false");
  await waitForTiles(el);
  vi.advanceTimersByTime(3 * PLAY_INTERVAL_MS);
  expect(changes).toEqual([]);
});

test("cinematic: a layer switch drops the loop's frame back to latest; a scrubbed frame stays", async () => {
  fakePlayClock();
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const el = mount({ server: SERVER, cinematic: "" });
  await el.ready;
  await waitForTiles(el);
  vi.advanceTimersByTime(PLAY_INTERVAL_MS);
  expect(el.getAttribute("datetime")).toBe(FIRE_FRAMES[0]); // the loop's doing
  // The user picks another layer: nobody chose that frame, so the new
  // layer applies at "latest" — no `datetime=` carried over.
  await el.setLayer("truecolor");
  expect(el.getAttribute("datetime")).toBeNull();
  expect(tileTemplates(el)).toEqual([`${SERVER}/tilesets/truecolor/tiles/{z}/{y}/{x}`]);
  el.remove();

  // Whereas a frame the user scrubbed to is theirs: it survives the switch.
  stubSwathApi({ temporal: { layer: "ndvi", dataset: "hls-s30-fire" } });
  const scrubbed = mount({ server: SERVER, cinematic: "" });
  await scrubbed.ready;
  scrub(scrubbed, 2);
  expect(scrubbed.getAttribute("datetime")).toBe(FIRE_FRAMES[2]);
  await scrubbed.setLayer("truecolor");
  expect(scrubbed.getAttribute("datetime")).toBe(FIRE_FRAMES[2]);
});
