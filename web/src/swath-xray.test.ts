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
  BYTES_BUCKET_COUNT,
  bytesBucket,
  type EventSourceLike,
  type PlanTrace,
  type TraceJson,
  type XRayMapLike,
  XRayOverlay,
} from "./swath-xray.js";
import { tileNorthWest } from "./tms.js";

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
  // The keyed shape the server emits since #36; the bare string above
  // stays tolerated.
  source?.emit(
    "trace",
    envelope("truecolor", "2/3/0", { decision: { cache_hit: { key: "0123abcd".repeat(8) } } }),
  );
  overlay.refresh();

  const kinds = new Map(badges(host).map((badge) => [badge.dataset.key, badge.dataset.decision]));
  expect(kinds.get("truecolor/2/0/0")).toBe("live");
  expect(kinds.get("truecolor/2/1/0")).toBe("overview");
  expect(kinds.get("truecolor/2/2/0")).toBe("cache_hit");
  expect(kinds.get("truecolor/2/3/0")).toBe("cache_hit");
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

// --- per-side badges under the compare swipe (issue #210) ---

/** Synthetic temporal payload: the side identity rides `requested`. */
function temporal(requested: string | null, granule = "g0"): Partial<TraceJson> {
  return {
    temporal: {
      granule_id: granule,
      granule_datetime: requested ?? "2024-09-05T19:03:00Z",
      requested,
      rule: requested === null ? "latest" : "latest_at_or_before",
    },
  };
}

const T0 = "2024-06-07T19:03:00Z";
const T1 = "2024-09-05T19:03:00Z";

/** A date-vs-date compare state over the fake map's `fire` layer. */
function dateCompare(fraction = 0.5) {
  return {
    fraction,
    sides: {
      mode: "date" as const,
      left: { layer: "fire", requested: T0 },
      right: { layer: "fire", requested: T1 },
    },
  };
}

test("compare, date mode: traces split into side-clipped badge layers by requested=", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("fire");
  overlay.setCompare(dateCompare(0.25));
  const source = factory.opened[0];

  source?.emit("trace", envelope("fire", "2/1/1", temporal(T0)));
  source?.emit("trace", envelope("fire", "2/1/1", { decision: "cache_hit", ...temporal(T1) }));
  // Neither side: another layer, and a frame no side shows — dropped.
  source?.emit("trace", envelope("other", "2/1/1", temporal(T0)));
  source?.emit("trace", envelope("fire", "2/1/1", temporal("2024-07-22T19:03:00Z")));
  overlay.refresh();

  // Both sides of ONE tile coexist (side rides the store key).
  const left = host.querySelectorAll<HTMLElement>(
    '.swath-xray-side[data-side="left"] .swath-xray-badge',
  );
  const right = host.querySelectorAll<HTMLElement>(
    '.swath-xray-side[data-side="right"] .swath-xray-badge',
  );
  expect(left.length).toBe(1);
  expect(right.length).toBe(1);
  expect(left[0]?.dataset.side).toBe("left");
  expect(left[0]?.dataset.key).toBe("left:fire/2/1/1");
  expect(left[0]?.dataset.decision).toBe("live");
  expect(right[0]?.dataset.key).toBe("right:fire/2/1/1");
  expect(right[0]?.dataset.decision).toBe("cache_hit");
  // Every painted badge lives in a side layer while comparing — the
  // plain layer (and the neither-side traces) paint nothing.
  expect(badges(host).length).toBe(2);

  // The clips split at the handle fraction.
  const leftLayer = host.querySelector<HTMLElement>('.swath-xray-side[data-side="left"]');
  const rightLayer = host.querySelector<HTMLElement>('.swath-xray-side[data-side="right"]');
  expect(leftLayer?.style.clipPath).toBe("inset(0px 75% 0px 0px)");
  expect(rightLayer?.style.clipPath).toBe("inset(0px 0px 0px 25%)");

  // A fraction-only move re-clips WITHOUT dropping the side entries.
  overlay.setCompare(dateCompare(0.6));
  expect(leftLayer?.style.clipPath).toBe("inset(0px 40% 0px 0px)");
  expect(overlay.traceFor("left:fire/2/1/1")).toBeDefined();

  // Ending the compare purges side entries and restores normal painting.
  overlay.setCompare(undefined);
  source?.emit("trace", envelope("fire", "2/1/1", temporal(T0)));
  overlay.refresh();
  expect(overlay.traceFor("left:fire/2/1/1")).toBeUndefined();
  expect(badges(host).length).toBe(1);
  expect(badges(host)[0]?.dataset.key).toBe("fire/2/1/1");
  overlay.dispose();
});

test("compare, layer mode: the envelope's layer picks the side", () => {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), { createEventSource: factory.create });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("ndvi");
  overlay.setCompare({
    fraction: 0.5,
    sides: {
      mode: "layer",
      left: { layer: "ndvi", requested: null },
      right: { layer: "truecolor", requested: null },
    },
  });
  const source = factory.opened[0];
  source?.emit("trace", envelope("ndvi", "2/1/1"));
  source?.emit("trace", envelope("truecolor", "2/2/1"));
  source?.emit("trace", envelope("park-fire-ndvi", "2/3/1"));
  overlay.refresh();
  const left = host.querySelectorAll<HTMLElement>(
    '.swath-xray-side[data-side="left"] .swath-xray-badge',
  );
  const right = host.querySelectorAll<HTMLElement>(
    '.swath-xray-side[data-side="right"] .swath-xray-badge',
  );
  expect([...left].map((badge) => badge.dataset.key)).toEqual(["left:ndvi/2/1/1"]);
  expect([...right].map((badge) => badge.dataset.key)).toEqual(["right:truecolor/2/2/1"]);
  overlay.dispose();
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

/** Two animation frames: the overlay's rAF-throttled paint has landed. */
async function painted(): Promise<void> {
  for (let i = 0; i < 2; i += 1) {
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  }
}

/** The topmost element at the center of `target`'s box — the real
 * hit-test a click performs, unlike `element.click()`. */
function hitAt(target: Element): Element | null {
  const box = target.getBoundingClientRect();
  return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
}

// The stacking contract between the overlay, MapLibre's control corners,
// and the compare swipe's right map (issue #210) — pinned by hit-testing
// because CI found the regression: an overlay root with a z-index equal
// to the control corners lifted badges OVER the x-ray toggle, and a badge
// under the button swallowed its click (`subtree intercepts pointer
// events`). Both enable orders, since the compare container and the
// overlay root are inserted at different times.
for (const order of ["compare first", "x-ray first"] as const) {
  test(`controls stay clickable over badges, badges over the compare map (${order})`, async () => {
    const factory = fakeFactory();
    const el = mountMap(factory.create, order === "x-ray first");
    if (order === "compare first") {
      el.setAttribute("compare-layer", "ndvi");
    }
    await el.ready;
    if (order === "compare first") {
      el.setAttribute("xray", "");
    } else {
      el.setAttribute("compare-layer", "ndvi");
    }

    // DOM order IS the stacking order at z auto: primary container, then
    // the compare clip right after it, then the overlay root.
    const container = el.querySelector(".swath-map-container");
    const compare = el.querySelector(".swath-map-compare");
    const overlay = el.querySelector(".swath-xray");
    expect(container?.nextElementSibling).toBe(compare);
    expect(compare && overlay ? compare.compareDocumentPosition(overlay) : 0).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );

    // Paint a badge over the control corner: at style zoom 2 (a 2048px
    // world) the 320x240 view on 0,0 shows 256px z3 tiles, and tile
    // 3/4/3 spans container x 160..320, y 0..120 — the top-right corner
    // the controls live in. Tile 3/4/4 is the right half's lower
    // quadrant: over the compare map, clear of every control. Both as
    // RIGHT-side (ndvi) traces: the right badge layer is the one clipped
    // to x >= 160, i.e. the one actually painted under the corner.
    factory.opened[0]?.emit("trace", envelope("ndvi", "3/4/3"));
    factory.opened[0]?.emit("trace", envelope("ndvi", "3/4/4"));
    await painted();
    const toggle = el.querySelector<HTMLButtonElement>(".swath-map-xray-toggle button");
    if (!toggle) {
      throw new Error("no x-ray toggle");
    }
    const badge = el.querySelector<HTMLElement>('.swath-xray-badge[data-key="right:ndvi/3/4/3"]');
    expect(badge).not.toBeNull();
    // A badge really does underlie the toggle...
    const toggleBox = toggle.getBoundingClientRect();
    const badgeBox = badge?.getBoundingClientRect();
    expect(badgeBox && toggleBox.left < badgeBox.right && toggleBox.right > badgeBox.left).toBe(
      true,
    );
    // ...and still the toggle wins the hit-test (the click lands).
    const hit = hitAt(toggle);
    expect(hit).toBe(toggle);
    // A badge clear of the controls is hit over the compare map — the
    // overlay paints above the right side, not under it.
    const clear = el.querySelector<HTMLElement>('.swath-xray-badge[data-key="right:ndvi/3/4/4"]');
    // (The tile box overhangs the 320x240 host, which the overlay clips —
    // so probe the visible part: the badge's intersection with the host.)
    const host = el.getBoundingClientRect();
    const clearBox = clear?.getBoundingClientRect();
    if (!clear || !clearBox) {
      throw new Error("no clear badge");
    }
    const clearHit = document.elementFromPoint(
      (Math.max(clearBox.left, host.left) + Math.min(clearBox.right, host.right)) / 2,
      (Math.max(clearBox.top, host.top) + Math.min(clearBox.bottom, host.bottom)) / 2,
    );
    expect(clearHit?.closest(".swath-xray-badge")).toBe(clear);
  });
}

// --- x-ray v1 (issue #42): why-view, bytes heatmap, live trace feed ---

/** The pinned plan payload (swath-core `trace`/`planner`): an overview
 * chosen over an inadmissible cache probe and an admissible live read. */
function makePlan(overrides: Partial<PlanTrace> = {}): PlanTrace {
  return {
    chosen: { overview: { factor: 2 } },
    considered: [
      { strategy: "cache_hit", estimated_cost_bytes: 0, admissible: false, reason: "cache miss" },
      {
        strategy: { overview: { factor: 2 } },
        estimated_cost_bytes: 128_018,
        admissible: true,
        reason: "coarsest overview within the oversample threshold",
      },
      {
        strategy: "live",
        estimated_cost_bytes: 510_050,
        admissible: true,
        reason: "full-resolution read",
      },
    ],
    ...overrides,
  };
}

function mountOverlay(capacity?: number): {
  host: HTMLDivElement;
  overlay: XRayOverlay;
  source: FakeEventSource;
} {
  const host = mountHost();
  const factory = fakeFactory();
  const overlay = new XRayOverlay(host, fakeMap(), {
    createEventSource: factory.create,
    ...(capacity === undefined ? {} : { capacity }),
  });
  overlay.connect(`${SERVER}/traces`);
  overlay.setLayer("truecolor");
  const source = factory.opened[0];
  if (!source) {
    throw new Error("no stream opened");
  }
  return { host, overlay, source };
}

test("inspector renders the planner why-view: chosen marked, inadmissible explained", () => {
  const { host, overlay, source } = mountOverlay();
  source.emit(
    "trace",
    envelope("truecolor", "2/1/1", {
      decision: { overview: { level: 2 } },
      plan: makePlan(),
    }),
  );
  overlay.refresh();
  badges(host)[0]?.click();

  const inspector = host.querySelector<HTMLElement>(".swath-xray-inspector");
  expect(inspector?.textContent).toContain("planner — chose overview (factor 2)");
  const table = inspector?.querySelector<HTMLTableElement>("table.swath-xray-plan");
  expect(table?.getAttribute("aria-label")).toBe("Planner candidates considered");
  expect([...(table?.querySelectorAll("th") ?? [])].map((th) => th.textContent)).toEqual([
    "strategy",
    "est. cost",
    "ok",
    "reason",
  ]);
  const rows = [...(table?.querySelectorAll<HTMLTableRowElement>("tbody tr") ?? [])];
  expect(rows).toHaveLength(3);
  // Fixed evaluation order: cache_hit, overview, live.
  expect(rows[0]?.textContent).toContain("cache_hit");
  expect(rows[0]?.dataset.admissible).toBe("false");
  expect(rows[0]?.textContent).toContain("no");
  expect(rows[0]?.textContent).toContain("cache miss");
  // The chosen candidate is marked in data AND text — never color alone.
  expect(rows[1]?.dataset.chosen).toBe("true");
  expect(rows[1]?.textContent).toContain("overview (factor 2) ✓ chosen");
  expect(rows[1]?.textContent).toContain("125 KB"); // human-formatted estimate
  expect(rows[2]?.dataset.chosen).toBe("false");
  expect(rows[2]?.textContent).toContain("live");
  expect(rows[2]?.textContent).toContain("498 KB");
});

test("null or absent plan: the why-view section is absent, the rest renders", () => {
  const { host, overlay, source } = mountOverlay();
  source.emit("trace", envelope("truecolor", "2/1/1", { plan: null }));
  overlay.refresh();
  badges(host)[0]?.click();
  let inspector = host.querySelector<HTMLElement>(".swath-xray-inspector");
  expect(inspector).not.toBeNull();
  expect(inspector?.querySelector(".swath-xray-plan")).toBeNull();
  expect(inspector?.textContent).not.toContain("planner");

  // Pre-#37 emitters omit the field entirely — same absence.
  source.emit("trace", envelope("truecolor", "2/1/1"));
  overlay.refresh();
  badges(host)[0]?.click();
  inspector = host.querySelector<HTMLElement>(".swath-xray-inspector");
  expect(inspector?.querySelector(".swath-xray-plan")).toBeNull();
});

test("bytesBucket: log-spaced buckets, dedicated zero bucket, degenerate range", () => {
  // Zero is its own bucket regardless of range — cache hits read nothing.
  expect(bytesBucket(0, 1024, 1_048_576)).toBe(0);
  // Extremes land in the first and last buckets.
  expect(bytesBucket(1024, 1024, 1_048_576)).toBe(1);
  expect(bytesBucket(1_048_576, 1024, 1_048_576)).toBe(BYTES_BUCKET_COUNT);
  // Log spacing: the geometric midpoint (32 KB over 1 KB..1 MB) sits in
  // the middle bucket, where a linear scale would put it in bucket 1.
  expect(bytesBucket(32_768, 1024, 1_048_576)).toBe(3);
  // Degenerate range: a single value is its own maximum.
  expect(bytesBucket(4096, 4096, 4096)).toBe(BYTES_BUCKET_COUNT);
});

test("bytes mode: badges color by bytes bucket, zero is distinct, off clears", () => {
  const { host, overlay, source } = mountOverlay();
  source.emit("trace", envelope("truecolor", "2/0/0", { bytes_read: 1024 }));
  source.emit("trace", envelope("truecolor", "2/1/0", { bytes_read: 1_048_576 }));
  source.emit(
    "trace",
    envelope("truecolor", "2/2/0", { decision: "cache_hit", bytes_read: 0, provenance: [] }),
  );
  overlay.refresh();

  // Decision mode paints no bucket data.
  expect(badges(host).every((badge) => badge.dataset.bytesBucket === undefined)).toBe(true);

  overlay.setDisplayMode("bytes");
  overlay.refresh();
  const byKey = new Map(badges(host).map((badge) => [badge.dataset.key, badge]));
  expect(byKey.get("truecolor/2/0/0")?.dataset.bytesBucket).toBe("1");
  expect(byKey.get("truecolor/2/1/0")?.dataset.bytesBucket).toBe(String(BYTES_BUCKET_COUNT));
  const zero = byKey.get("truecolor/2/2/0");
  expect(zero?.dataset.bytesBucket).toBe("0");
  expect(zero?.style.borderStyle).toBe("dashed"); // distinct in shape, not just hue
  // Min and max buckets carry different colors.
  expect(byKey.get("truecolor/2/0/0")?.style.borderColor).not.toBe(
    byKey.get("truecolor/2/1/0")?.style.borderColor,
  );

  // The mode control mirrors the mode.
  const pressed = [...host.querySelectorAll<HTMLButtonElement>(".swath-xray-modes button")].map(
    (button) => [button.dataset.mode, button.getAttribute("aria-pressed")],
  );
  expect(pressed).toEqual([
    ["decision", "false"],
    ["bytes", "true"],
    ["off", "false"],
  ]);

  overlay.setDisplayMode("off");
  overlay.refresh();
  expect(badges(host)).toHaveLength(0);

  overlay.setDisplayMode("decision");
  overlay.refresh();
  expect(badges(host)).toHaveLength(3);
  expect(badges(host).every((badge) => badge.dataset.bytesBucket === undefined)).toBe(true);
});

test("heatmap legend shows the store's non-zero min/max and hides off bytes mode", () => {
  const { host, overlay, source } = mountOverlay();
  const legend = host.querySelector<HTMLElement>(".swath-xray-scale");
  expect(legend?.hidden).toBe(true);

  overlay.setDisplayMode("bytes");
  overlay.refresh();
  expect(legend?.hidden).toBe(false);
  expect(legend?.textContent).toContain("—"); // empty store: no range yet

  source.emit("trace", envelope("truecolor", "2/0/0", { bytes_read: 2048 }));
  source.emit("trace", envelope("truecolor", "2/1/0", { bytes_read: 3_145_728 }));
  source.emit("trace", envelope("truecolor", "2/2/0", { decision: "cache_hit", bytes_read: 0 }));
  overlay.refresh();
  expect(legend?.dataset.min).toBe("2048"); // zero never enters the range
  expect(legend?.dataset.max).toBe("3145728");
  expect(legend?.textContent).toContain("2.0 KB – 3.0 MB");
  expect(legend?.textContent).toContain("0 (cache)");

  overlay.setDisplayMode("decision");
  overlay.refresh();
  expect(legend?.hidden).toBe(true);
});

test("feed lines mirror received traces; clicking one opens the inspector", () => {
  const { host, overlay, source } = mountOverlay();
  const toggle = host.querySelector<HTMLButtonElement>(".swath-xray-feed-toggle");
  const lines = host.querySelector<HTMLOListElement>(".swath-xray-feed-lines");
  expect(toggle?.getAttribute("aria-expanded")).toBe("false");
  expect(lines?.hidden).toBe(true); // collapsed by default: never blocks the map

  source.emit("trace", envelope("truecolor", "2/1/1", { plan: makePlan() }));
  source.emit("trace", envelope("ndvi", "3/2/1", { decision: "cache_hit", bytes_read: 0 }));
  toggle?.click();
  expect(toggle?.getAttribute("aria-expanded")).toBe("true");
  expect(lines?.hidden).toBe(false);

  const items = [...(lines?.querySelectorAll("li > button") ?? [])] as HTMLButtonElement[];
  expect(items).toHaveLength(2); // every layer's traffic, not just the active layer
  expect(items[0]?.dataset.key).toBe("truecolor/2/1/1");
  expect(items[0]?.textContent).toContain("truecolor 2/1/1 live 18ms 128 KB".replace(" KB", "KB"));
  expect(items[1]?.dataset.key).toBe("ndvi/3/2/1");
  expect(items[1]?.textContent).toContain("cache_hit");

  overlay.refresh();
  items[0]?.click();
  const inspector = host.querySelector<HTMLElement>(".swath-xray-inspector");
  expect(inspector?.getAttribute("aria-label")).toBe("Trace for tile truecolor/2/1/1");
  expect(inspector?.textContent).toContain("planner — chose overview (factor 2)");
});

test("feed is bounded: oldest lines drop with a visible counter", () => {
  const { host, source } = mountOverlay();
  for (let i = 0; i < 205; i += 1) {
    source.emit("trace", envelope("truecolor", `8/${i}/0`));
  }
  const lines = host.querySelector<HTMLOListElement>(".swath-xray-feed-lines");
  expect(lines?.children).toHaveLength(200);
  const first = lines?.querySelector<HTMLButtonElement>("li > button");
  expect(first?.dataset.key).toBe("truecolor/8/5/0"); // 0..4 dropped oldest-first
  const dropped = host.querySelector<HTMLElement>(".swath-xray-feed-dropped");
  expect(dropped?.hidden).toBe(false);
  expect(dropped?.textContent).toBe("5 dropped");
});

test("feed pause freezes content; resume flushes what arrived meanwhile", () => {
  const { host, source } = mountOverlay();
  host.querySelector<HTMLButtonElement>(".swath-xray-feed-toggle")?.click();
  const lines = host.querySelector<HTMLOListElement>(".swath-xray-feed-lines");
  const pause = host.querySelector<HTMLButtonElement>(".swath-xray-feed-pause");
  expect(pause?.hidden).toBe(false);

  source.emit("trace", envelope("truecolor", "2/0/0"));
  pause?.click();
  expect(pause?.getAttribute("aria-pressed")).toBe("true");
  expect(pause?.textContent).toBe("resume");
  source.emit("trace", envelope("truecolor", "2/1/0"));
  source.emit("trace", envelope("truecolor", "2/2/0"));
  expect(lines?.children).toHaveLength(1); // held: nothing appended while paused

  pause?.click();
  expect(pause?.getAttribute("aria-pressed")).toBe("false");
  expect(lines?.children).toHaveLength(3); // the paused traffic arrives in order
  const keys = [...(lines?.querySelectorAll("li > button") ?? [])].map(
    (button) => (button as HTMLButtonElement).dataset.key,
  );
  expect(keys).toEqual(["truecolor/2/0/0", "truecolor/2/1/0", "truecolor/2/2/0"]);
});

test("lagged events surface as inline marker lines in the feed", () => {
  const { host, source } = mountOverlay();
  source.emit("trace", envelope("truecolor", "2/0/0"));
  source.emit("lagged", JSON.stringify({ missed: 4 }));
  source.emit("trace", envelope("truecolor", "2/1/0"));
  const lines = host.querySelector<HTMLOListElement>(".swath-xray-feed-lines");
  expect(lines?.children).toHaveLength(3);
  const marker = lines?.children[1];
  expect(marker?.className).toBe("swath-xray-feed-line-lagged");
  expect(marker?.textContent).toBe("— missed 4 traces —");
});

// --- trace analytics (issue #111): the panel over the overlay's stream ---

test("a scripted mix over the mocked stream produces the expected analytics", () => {
  const { host, source } = mountOverlay();
  const panel = host.querySelector<HTMLElement>(".swath-xray-analytics");
  expect(panel?.textContent).toContain("p50 — · p95 — ms"); // no data ≠ zero

  // The scripted request mix: 3 live, 1 overview, 2 cache hits — across
  // layers and zooms, because the panel describes the stream (like the
  // feed), not the painted subset.
  const timings = (totalMs: number): Partial<TraceJson> => ({
    timings: { read_ms: 1, warp_ms: 1, pixel_ops_ms: 1, encode_ms: 1, total_ms: totalMs },
  });
  source.emit("trace", envelope("truecolor", "2/0/0", { decision: "live", ...timings(10) }));
  source.emit("trace", envelope("truecolor", "2/1/0", { decision: "live", ...timings(20) }));
  source.emit(
    "trace",
    envelope("ndvi", "3/1/1", { decision: { overview: { level: 2 } }, ...timings(30) }),
  );
  source.emit("trace", envelope("truecolor", "2/2/0", { decision: "cache_hit", ...timings(5) }));
  source.emit(
    "trace",
    envelope("truecolor", "2/0/0", {
      decision: { cache_hit: { key: "feedbeef" } },
      ...timings(15),
    }),
  );
  source.emit("trace", envelope("truecolor", "2/3/0", { decision: "live", ...timings(40) }));

  // Window sorted [5, 10, 15, 20, 30, 40]: p50 = 15 + 0.5 * 5 = 17.5,
  // p95 = 30 + 0.75 * 10 = 37.5 (rank = p * (n - 1), interpolated).
  expect(panel?.dataset.p50).toBe("17.5");
  expect(panel?.dataset.p95).toBe("37.5");
  expect(panel?.dataset.live).toBe("3");
  expect(panel?.dataset.overview).toBe("1");
  expect(panel?.dataset.cacheHit).toBe("2");
  expect(panel?.dataset.total).toBe("6");
  expect(panel?.dataset.hitRate).toBe(String(2 / 6));
  expect(panel?.textContent).toContain("live 3");
  expect(panel?.textContent).toContain("ovr 1");
  expect(panel?.textContent).toContain("cache 2");
  expect(panel?.textContent).toContain("hit 33.3%");
  // A repeated tile (2/0/0 above) still counts twice: analytics fold the
  // stream, unlike the latest-wins badge store.
  expect(panel?.dataset.samples).toBe("6");
});

test("malformed payloads never reach the analytics", () => {
  const { host, source } = mountOverlay();
  const panel = host.querySelector<HTMLElement>(".swath-xray-analytics");
  source.emit("trace", "not json");
  source.emit("trace", JSON.stringify({ tile: "nope", layer: "l", trace: makeTrace() }));
  expect(panel?.dataset.total).toBe("0");
  expect(panel?.textContent).toContain("p50 — · p95 — ms");
});

test("temporal traces reach the per-frame analytics line and the inspector (issue #182)", () => {
  const { host, overlay, source } = mountOverlay();
  const temporal = {
    granule_id: "hlss30-t10tfk-2024229",
    granule_datetime: "2024-08-16T19:03:00Z",
    requested: "2024-08-20T00:00:00Z",
    rule: "latest_at_or_before",
  };
  source.emit("trace", envelope("truecolor", "2/1/1", { temporal }));
  source.emit(
    "trace",
    envelope("truecolor", "2/2/1", {
      decision: { cache_hit: { key: "abcd1234" } },
      temporal,
    }),
  );

  // The analytics card grew the per-frame plan-mix line, keyed on the
  // temporal decision's granule_datetime.
  const panel = host.querySelector<HTMLElement>(".swath-xray-analytics");
  expect(panel?.dataset.frame).toBe(temporal.granule_datetime);
  expect(panel?.dataset.frameLive).toBe("1");
  expect(panel?.dataset.frameCacheHit).toBe("1");
  expect(panel?.textContent).toContain(
    `frame ${temporal.granule_datetime} · live 1 · ovr 0 · cache 1`,
  );

  // The inspector names the frame: granule datetime, id, and rule.
  overlay.refresh();
  badges(host)[0]?.click();
  const inspector = host.querySelector<HTMLElement>(".swath-xray-inspector");
  expect(inspector?.textContent).toContain(
    "2024-08-16T19:03:00Z (hlss30-t10tfk-2024229, latest_at_or_before)",
  );

  // A static-layer trace (no temporal) leaves the frame line untouched.
  source.emit("trace", envelope("truecolor", "2/3/1"));
  expect(panel?.dataset.frame).toBe(temporal.granule_datetime);
  expect(panel?.dataset.frameLive).toBe("1");

  // And so does another LAYER's temporal trace: the frame line narrates
  // the painted layer's animation, not the whole stream (which counts
  // in the layer-agnostic totals above, like the feed).
  source.emit(
    "trace",
    envelope("other-layer", "2/3/1", {
      temporal: { ...temporal, granule_datetime: "2030-01-01T00:00:00Z" },
    }),
  );
  expect(panel?.dataset.frame).toBe(temporal.granule_datetime);
  expect(panel?.dataset.total).toBe("4");
});
