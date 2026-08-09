// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The x-ray overlay's contract without a network: synthetic SSE envelopes
// go through a fake `EventSource` (the `createEventSource` seam), badge
// geometry is checked against a fake map with a known `project()`, and
// the `<swath-map>` integration (attribute toggle, layer-switch survival,
// cleanup) runs over the same stubbed fetch as swath-map.test.ts. The
// real-stream proof is web/e2e/swath-xray.e2e.ts.
import { afterEach, beforeAll, expect, test, vi } from "vitest";
import { defineSwathMap, type SwathMap } from "./swath-map.js";
import {
  type EventSourceLike,
  type TraceJson,
  tileNorthWest,
  type XRayMapLike,
  XRayOverlay,
} from "./swath-xray.js";

const SERVER = "https://swath.test";

/** A scriptable `EventSource` stand-in: tests `emit` named events. */
class FakeEventSource implements EventSourceLike {
  readonly url: string;
  closed = false;
  readonly #listeners = new Map<string, ((event: MessageEvent<string>) => void)[]>();

  constructor(url: string) {
    this.url = url;
  }

  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void {
    const listeners = this.#listeners.get(type) ?? [];
    listeners.push(listener);
    this.#listeners.set(type, listeners);
  }

  close(): void {
    this.closed = true;
  }

  emit(type: string, data: string): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener(new MessageEvent(type, { data }));
    }
  }
}

/** Factory recording every stream it opened. */
function fakeFactory(): { opened: FakeEventSource[]; create: (url: string) => FakeEventSource } {
  const opened: FakeEventSource[] = [];
  return {
    opened,
    create: (url: string) => {
      const source = new FakeEventSource(url);
      opened.push(source);
      return source;
    },
  };
}

/** The pinned core `Trace` JSON (swath-core `trace` module), overridable. */
function makeTrace(overrides: Partial<TraceJson> = {}): TraceJson {
  return {
    decision: "live",
    source: "s3://hls/granule/B04.tif",
    sources: ["s3://hls/granule/B04.tif"],
    crs_from: 32613,
    crs_to: 3857,
    bytes_read: 131_072,
    provenance: [{ path: "granule/B04.tif", offset: 4096, length: 131_072 }],
    timings: { read_ms: 12, warp_ms: 3, pixel_ops_ms: 1, encode_ms: 2, total_ms: 18 },
    ingest_to_pixel_ms: null,
    ...overrides,
  };
}

/** One `event: trace` payload — the swath-api envelope. */
function envelope(layer: string, tile: string, overrides: Partial<TraceJson> = {}): string {
  return JSON.stringify({ tile, layer, trace: makeTrace(overrides) });
}

/** A fake map with a known, linear `project()` so expected badge
 * geometry is computable in the test. Style zoom 1 → displayed tile
 * zoom 2 (256px source offset). */
function fakeMap(zoom = 1): XRayMapLike {
  return {
    project: ([lng, lat]: [number, number]) => ({ x: (lng + 180) * 2, y: (90 - lat) * 2 }),
    getZoom: () => zoom,
    on: () => undefined,
    off: () => undefined,
  };
}

function mountHost(): HTMLDivElement {
  const host = document.createElement("div");
  host.style.cssText = "position: relative; width: 800px; height: 600px;";
  document.body.append(host);
  return host;
}

function badges(host: HTMLElement): HTMLButtonElement[] {
  return [...host.querySelectorAll<HTMLButtonElement>(".swath-xray-badge")];
}

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

test("trace events populate the store, latest trace wins per tile", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("truecolor");
  const source = factory.opened[0];
  expect(source?.url).toBe(`${SERVER}/traces`);

  source?.emit("trace", envelope("truecolor", "2/1/1"));
  source?.emit(
    "trace",
    envelope("truecolor", "2/1/1", {
      timings: { read_ms: 1, warp_ms: 1, pixel_ops_ms: 1, encode_ms: 1, total_ms: 44 },
    }),
  );
  overlay.refresh();

  expect(overlay.size).toBe(1);
  const painted = badges(host);
  expect(painted).toHaveLength(1);
  expect(painted[0]?.dataset.key).toBe("truecolor/2/1/1");
  expect(painted[0]?.dataset.totalMs).toBe("44");
  expect(painted[0]?.textContent).toContain("44 ms");
  expect(painted[0]?.textContent).toContain("128 KB");
});

test("malformed payloads are dropped, not fatal", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  const source = factory.opened[0];
  source?.emit("trace", "not json");
  source?.emit("trace", JSON.stringify({ tile: "nope", layer: "l", trace: makeTrace() }));
  overlay.refresh();
  expect(overlay.size).toBe(0);
});

test("store is LRU-bounded: least recently updated tile is evicted", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), {
    createEventSource: factory.create,
    capacity: 3,
  });
  overlay.connect(`${SERVER}/traces`);
  const source = factory.opened[0];

  source?.emit("trace", envelope("truecolor", "2/0/0"));
  source?.emit("trace", envelope("truecolor", "2/1/0"));
  source?.emit("trace", envelope("truecolor", "2/2/0"));
  // Updating the oldest refreshes its LRU position...
  source?.emit("trace", envelope("truecolor", "2/0/0"));
  // ...so the fourth distinct tile evicts 2/1/0, not 2/0/0.
  source?.emit("trace", envelope("truecolor", "2/3/0"));

  expect(overlay.size).toBe(3);
  expect(overlay.traceFor("truecolor/2/1/0")).toBeUndefined();
  expect(overlay.traceFor("truecolor/2/0/0")).toBeDefined();
  expect(overlay.traceFor("truecolor/2/3/0")).toBeDefined();
});

test("lagged events accumulate into a visible missed-traces badge", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  const source = factory.opened[0];

  const laggedBadge = host.querySelector<HTMLElement>(".swath-xray-lagged");
  expect(laggedBadge?.hidden).toBe(true);
  source?.emit("lagged", JSON.stringify({ missed: 7 }));
  expect(laggedBadge?.hidden).toBe(false);
  expect(laggedBadge?.textContent).toBe("missed 7 traces");
  source?.emit("lagged", JSON.stringify({ missed: 3 }));
  expect(laggedBadge?.textContent).toBe("missed 10 traces");
});

test("badges land where the map projects the tile bounds", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const map = fakeMap();
  const overlay = new XRayOverlay(host, map, { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("truecolor");
  factory.opened[0]?.emit("trace", envelope("truecolor", "2/1/1"));
  overlay.refresh();

  const badge = badges(host)[0];
  const nw = map.project(tileNorthWest(2, 1, 1));
  const se = map.project(tileNorthWest(2, 2, 2));
  // The style serializer rounds long floats, so compare numerically.
  expect(Number.parseFloat(badge?.style.left ?? "")).toBeCloseTo(nw.x, 3);
  expect(Number.parseFloat(badge?.style.top ?? "")).toBeCloseTo(nw.y, 3);
  expect(Number.parseFloat(badge?.style.width ?? "")).toBeCloseTo(se.x - nw.x, 3);
  expect(Number.parseFloat(badge?.style.height ?? "")).toBeCloseTo(se.y - nw.y, 3);
});

test("only the displayed tile zoom and the active layer are badged", () => {
  const host = mountHost();
  const factory = fakeFactory();
  // Style zoom 1 → a 256px raster source displays z2 tiles.
  const overlay = new XRayOverlay(host, fakeMap(1), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("truecolor");
  const source = factory.opened[0];
  source?.emit("trace", envelope("truecolor", "2/1/1")); // painted
  source?.emit("trace", envelope("truecolor", "3/1/1")); // wrong zoom
  source?.emit("trace", envelope("ndvi", "2/2/1")); // wrong layer
  overlay.refresh();

  expect(overlay.size).toBe(3); // all stored — filtering is paint-time
  expect(badges(host).map((badge) => badge.dataset.key)).toEqual(["truecolor/2/1/1"]);

  // Switching layer re-filters without touching the store.
  overlay.setLayer("ndvi");
  overlay.refresh();
  expect(badges(host).map((badge) => badge.dataset.key)).toEqual(["ndvi/2/2/1"]);
});

test("decisions color and tag the badge: live vs overview vs cache_hit", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("truecolor");
  const source = factory.opened[0];
  source?.emit("trace", envelope("truecolor", "2/0/0", { decision: "live" }));
  source?.emit("trace", envelope("truecolor", "2/1/0", { decision: { overview: { level: 2 } } }));
  source?.emit("trace", envelope("truecolor", "2/2/0", { decision: "cache_hit" }));
  overlay.refresh();

  const kinds = new Map(badges(host).map((badge) => [badge.dataset.key, badge.dataset.decision]));
  expect(kinds.get("truecolor/2/0/0")).toBe("live");
  expect(kinds.get("truecolor/2/1/0")).toBe("overview");
  expect(kinds.get("truecolor/2/2/0")).toBe("cache_hit");
  const borders = new Set(badges(host).map((badge) => badge.style.borderColor));
  expect(borders.size).toBe(3); // three decisions, three colors
});

test("clicking a badge opens the inspector with the full trace; Escape closes", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("truecolor");
  factory.opened[0]?.emit(
    "trace",
    envelope("truecolor", "2/1/1", {
      decision: { overview: { level: 2 } },
      ingest_to_pixel_ms: 297,
    }),
  );
  overlay.refresh();
  badges(host)[0]?.click();

  const inspector = host.querySelector<HTMLElement>(".swath-xray-inspector");
  expect(inspector?.getAttribute("role")).toBe("dialog");
  expect(inspector?.getAttribute("aria-label")).toBe("Trace for tile truecolor/2/1/1");
  const text = inspector?.textContent ?? "";
  expect(text).toContain("overview (level 2)");
  expect(text).toContain("131072");
  expect(text).toContain("total 18 ms");
  expect(text).toContain("granule/B04.tif @4096 +131072");
  expect(text).toContain("32613 → 3857");
  expect(text).toContain("297 ms");

  inspector?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(host.querySelector(".swath-xray-inspector")).toBeNull();
});

test("ingest-to-pixel readout tracks the latest non-null value", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  const source = factory.opened[0];
  const readout = host.querySelector<HTMLElement>(".swath-xray-ingest");

  expect(readout?.textContent).toBe("ingest→pixel: —");
  source?.emit("trace", envelope("truecolor", "2/0/0", { ingest_to_pixel_ms: 297 }));
  expect(readout?.textContent).toBe("ingest→pixel: 297 ms");
  source?.emit("trace", envelope("truecolor", "2/1/0", { ingest_to_pixel_ms: null }));
  expect(readout?.textContent).toBe("ingest→pixel: 297 ms"); // null never regresses it
  source?.emit("trace", envelope("truecolor", "2/2/0", { ingest_to_pixel_ms: 150 }));
  expect(readout?.textContent).toBe("ingest→pixel: 150 ms");
});

// --- <swath-map> integration: the `xray` attribute over stubbed fetch ---

function json(body: object): Response {
  return new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } });
}

/** Minimal Swath API stub: enough for `<swath-map>` with explicit
 * center/zoom (tilesets list; tile requests answer 404, MapLibre copes). */
function stubSwathApi(): void {
  vi.stubGlobal("fetch", (input: RequestInfo | URL): Promise<Response> => {
    const url = input instanceof Request ? input.url : String(input);
    if (/\/tilesets$/.test(url)) {
      const base = new URL(url).origin;
      return Promise.resolve(
        json({
          tilesets: [
            {
              title: "HLS true color",
              links: [{ href: `${base}/tilesets/truecolor`, rel: "self" }],
            },
            { title: "HLS NDVI", links: [{ href: `${base}/tilesets/ndvi`, rel: "self" }] },
          ],
        }),
      );
    }
    return Promise.resolve(new Response("not found", { status: 404 }));
  });
}

function mountMap(factory: (url: string) => FakeEventSource, xray: boolean): SwathMap {
  stubSwathApi();
  const el = document.createElement("swath-map") as SwathMap;
  el.xrayEventSource = factory; // the seam, assigned before any enable
  el.setAttribute("server", SERVER);
  el.setAttribute("layer", "truecolor");
  el.setAttribute("center", "0,0");
  el.setAttribute("zoom", "2");
  if (xray) {
    el.setAttribute("xray", "");
  }
  el.style.cssText = "width: 320px; height: 240px;";
  document.body.append(el);
  return el;
}

beforeAll(() => {
  defineSwathMap();
});

test("the xray attribute opens the stream; removing it closes and cleans up", async () => {
  const factory = fakeFactory();
  const el = mountMap(factory.create, false);
  await el.ready;
  expect(factory.opened).toHaveLength(0);
  expect(el.querySelector(".swath-xray")).toBeNull();

  el.setAttribute("xray", "");
  expect(factory.opened).toHaveLength(1);
  expect(factory.opened[0]?.url).toBe(`${SERVER}/traces`);
  expect(el.querySelector(".swath-xray")).not.toBeNull();

  el.removeAttribute("xray");
  expect(factory.opened[0]?.closed).toBe(true);
  expect(el.querySelector(".swath-xray")).toBeNull();
});

test("xray at mount connects once and survives a layer switch", async () => {
  const factory = fakeFactory();
  const el = mountMap(factory.create, true);
  await el.ready;
  expect(factory.opened).toHaveLength(1);
  expect(el.querySelector(".swath-xray")).not.toBeNull();

  await el.setLayer("ndvi");
  // The overlay outlives the style swap and the stream never churns.
  expect(el.querySelector(".swath-xray")).not.toBeNull();
  expect(factory.opened).toHaveLength(1);
  expect(factory.opened[0]?.closed).toBe(false);
});

test("disconnecting the element closes the stream", async () => {
  const factory = fakeFactory();
  const el = mountMap(factory.create, true);
  await el.ready;
  el.remove();
  expect(factory.opened[0]?.closed).toBe(true);
});

test("the toggle control mirrors and flips the xray attribute", async () => {
  const factory = fakeFactory();
  const el = mountMap(factory.create, false);
  await el.ready;
  const button = el.querySelector<HTMLButtonElement>(".swath-map-xray-toggle button");
  expect(button?.getAttribute("aria-pressed")).toBe("false");

  button?.click();
  expect(el.hasAttribute("xray")).toBe(true);
  expect(button?.getAttribute("aria-pressed")).toBe("true");
  expect(el.querySelector(".swath-xray")).not.toBeNull();

  button?.click();
  expect(el.hasAttribute("xray")).toBe(false);
  expect(button?.getAttribute("aria-pressed")).toBe("false");
  expect(el.querySelector(".swath-xray")).toBeNull();
  expect(factory.opened[0]?.closed).toBe(true);
});
