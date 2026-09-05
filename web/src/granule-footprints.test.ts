// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The footprint paint's contract without MapLibre: a fake map records
// sources/layers and can wipe them like `setStyle` does, so the tests
// prove the geometry (closed CRS84 rings from bboxes), the paint
// (source + line layer added once, then updated in place), the style-swap
// survival (styledata re-adds), and the zoom-to-footprint move.
import { expect, test, vi } from "vitest";
import {
  FOOTPRINT_LAYER_ID,
  FOOTPRINT_SOURCE_ID,
  type FootprintCollection,
  footprintCollection,
  frameBounds,
  type GranuleBbox,
  GranuleFootprints,
  parseBbox,
  SCOPE_SOURCE_ID,
  unionBbox,
} from "./granule-footprints.js";

/** A fake map: records sources/layers/fitBounds, replays styledata, and
 * `wipe()` simulates what `setStyle` does to sources and layers. */
class FakeMap {
  sources = new Map<string, { data: FootprintCollection; setDataCalls: number }>();
  layers = new Map<string, object>();
  fitBoundsCalls: { bounds: [[number, number], [number, number]]; options: object }[] = [];
  #listeners = new Map<string, (() => void)[]>();

  getSource(id: string): unknown {
    const entry = this.sources.get(id);
    if (!entry) {
      return undefined;
    }
    return {
      setData: (data: FootprintCollection): void => {
        entry.data = data;
        entry.setDataCalls += 1;
      },
    };
  }

  addSource(id: string, source: object): void {
    const data = (source as { data: FootprintCollection }).data;
    this.sources.set(id, { data, setDataCalls: 0 });
  }

  getLayer(id: string): unknown {
    return this.layers.get(id);
  }

  addLayer(layer: object): void {
    this.layers.set((layer as { id: string }).id, layer);
  }

  fitBounds(bounds: [[number, number], [number, number]], options: object): void {
    this.fitBoundsCalls.push({ bounds, options });
  }

  on(type: string, listener: () => void): unknown {
    const listeners = this.#listeners.get(type) ?? [];
    listeners.push(listener);
    this.#listeners.set(type, listeners);
    return this;
  }

  off(type: string, listener: () => void): unknown {
    this.#listeners.set(
      type,
      (this.#listeners.get(type) ?? []).filter((entry) => entry !== listener),
    );
    return this;
  }

  emit(type: string): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener();
    }
  }

  /** What `setStyle` does: every source and layer is gone. */
  wipe(): void {
    this.sources.clear();
    this.layers.clear();
  }
}

const GRANULES = [
  { id: "a", bbox: [-106.1, 39.2, -105.9, 39.4] as const },
  { id: "b", bbox: [10, 45, 12, 47] as const },
];

test("footprintCollection: one closed CRS84 Polygon ring per bbox", () => {
  const collection = footprintCollection(GRANULES);
  expect(collection.type).toBe("FeatureCollection");
  expect(collection.features).toHaveLength(2);
  expect(collection.features[0]?.properties.id).toBe("a");
  expect(collection.features[1]?.geometry.coordinates).toEqual([
    [
      [10, 45],
      [12, 45],
      [12, 47],
      [10, 47],
      [10, 45],
    ],
  ]);
});

test("set paints a GeoJSON source and a line layer; a second set updates in place", () => {
  const map = new FakeMap();
  const footprints = new GranuleFootprints(map);
  footprints.set(GRANULES);

  const source = map.sources.get(FOOTPRINT_SOURCE_ID);
  expect(source?.data.features).toHaveLength(2);
  const layer = map.layers.get(FOOTPRINT_LAYER_ID) as { type: string; source: string };
  expect(layer.type).toBe("line");
  expect(layer.source).toBe(FOOTPRINT_SOURCE_ID);

  footprints.set([GRANULES[1] as (typeof GRANULES)[1]]);
  expect(source?.setDataCalls).toBe(1); // updated, not re-added
  expect(source?.data.features).toHaveLength(1);
  expect(map.layers.size).toBe(1);
});

test("styledata after a style wipe re-adds source and layer with the same data", () => {
  const map = new FakeMap();
  const footprints = new GranuleFootprints(map);
  footprints.set(GRANULES);

  map.wipe(); // what setStyle (a layer switch) does
  expect(map.sources.size).toBe(0);
  map.emit("styledata");
  expect(map.sources.get(FOOTPRINT_SOURCE_ID)?.data.features).toHaveLength(2);
  expect(map.layers.has(FOOTPRINT_LAYER_ID)).toBe(true);
});

test("clear empties the collection; zoomTo fits the footprint bounds", () => {
  const map = new FakeMap();
  const footprints = new GranuleFootprints(map);
  footprints.set(GRANULES);
  footprints.clear();
  expect(map.sources.get(FOOTPRINT_SOURCE_ID)?.data.features).toHaveLength(0);

  footprints.zoomTo([10, 45, 12, 47]);
  expect(map.fitBoundsCalls).toHaveLength(1);
  expect(map.fitBoundsCalls[0]?.bounds).toEqual([
    [10, 45],
    [12, 47],
  ]);
  expect(map.fitBoundsCalls[0]?.options).toMatchObject({ duration: 0 });
});

test("a paint that lands mid style swap heals on the retry timer", () => {
  vi.useFakeTimers();
  try {
    const map = new FakeMap();
    // First addSource throws (what MapLibre does while a style is still
    // loading); afterwards the map behaves normally.
    let failures = 1;
    const addSource = map.addSource.bind(map);
    map.addSource = (id, source): void => {
      if (failures > 0) {
        failures -= 1;
        throw new Error("Style is not done loading");
      }
      addSource(id, source);
    };
    const footprints = new GranuleFootprints(map);
    footprints.set(GRANULES);
    expect(map.sources.size).toBe(0); // the paint failed…
    vi.advanceTimersByTime(250);
    expect(map.sources.get(FOOTPRINT_SOURCE_ID)?.data.features).toHaveLength(2); // …and healed
    expect(map.layers.has(FOOTPRINT_LAYER_ID)).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

test("dispose detaches: styledata after dispose paints nothing", () => {
  const map = new FakeMap();
  const footprints = new GranuleFootprints(map);
  footprints.set(GRANULES);
  footprints.dispose();
  map.wipe();
  map.emit("styledata");
  expect(map.sources.size).toBe(0);
  expect(map.layers.size).toBe(0);
});

test("parseBbox: the checked tuple, or undefined for junk shapes", () => {
  expect(parseBbox([-121.74, 39.99, -121.65, 40.06])).toEqual([-121.74, 39.99, -121.65, 40.06]);
  for (const junk of [undefined, null, "box", [1, 2, 3], [1, 2, 3, "four"], {}]) {
    expect(parseBbox(junk)).toBeUndefined();
  }
});

test("unionBbox: the envelope of footprints; empty stays unknown, not [0,0,0,0]", () => {
  expect(unionBbox([])).toBeUndefined();
  expect(unionBbox([[-2, -1, 3, 4]])).toEqual([-2, -1, 3, 4]);
  expect(
    unionBbox([
      [-121.74, 39.99, -121.65, 40.06],
      [-121.8, 40.0, -121.7, 40.1],
      [-121.72, 39.9, -121.6, 40.02],
    ]),
  ).toEqual([-121.8, 39.9, -121.6, 40.1]);
});

test("frameBounds: fit means the granule you are looking at, not the whole layer (#397)", () => {
  const frames = ["2024-06-07T19:03:00Z", "2024-07-22T19:03:00Z", "2024-08-16T19:03:00Z"];
  const footprints = [
    { datetime: frames[0] as string, bbox: [0, 0, 1, 1] as GranuleBbox },
    { datetime: frames[1] as string, bbox: [10, 10, 11, 11] as GranuleBbox },
    { datetime: frames[2] as string, bbox: [20, 20, 21, 21] as GranuleBbox },
  ];
  // Exactly on a frame: that frame's footprint.
  expect(frameBounds(frames, footprints, frames[1] as string)).toEqual([10, 10, 11, 11]);
  // Between frames: the latest AT OR BEFORE, the server's own rule — a
  // `datetime=` need not equal any granule's instant.
  expect(frameBounds(frames, footprints, "2024-08-01T00:00:00Z")).toEqual([10, 10, 11, 11]);
  // After every frame: the last one, which is what the map is showing.
  expect(frameBounds(frames, footprints, "2025-01-01T00:00:00Z")).toEqual([20, 20, 21, 21]);
  // Before every frame: nothing is backing the view, so the caller falls
  // back to the layer's extent rather than fitting an empty box.
  expect(frameBounds(frames, footprints, "2020-01-01T00:00:00Z")).toBeUndefined();
  // No frame viewed, nothing known, or an unparseable instant: same.
  expect(frameBounds(frames, footprints, null)).toBeUndefined();
  expect(frameBounds(frames, [], frames[0] as string)).toBeUndefined();
  expect(frameBounds(frames, footprints, "not a datetime")).toBeUndefined();
});

test("frameBounds: several granules at one instant are one footprint", () => {
  const frames = ["2024-06-07T19:03:00Z"];
  const footprints = [
    { datetime: frames[0] as string, bbox: [0, 0, 1, 1] as GranuleBbox },
    { datetime: frames[0] as string, bbox: [2, 2, 3, 3] as GranuleBbox },
  ];
  // A pass can cover an area with more than one granule; fit shows all of it.
  expect(frameBounds(frames, footprints, frames[0] as string)).toEqual([0, 0, 3, 3]);
});

test("a second painter owns its own source and layer, so the scope box does not fight the footprints (#412)", () => {
  const map = new FakeMap();
  const footprints = new GranuleFootprints(map);
  const scope = new GranuleFootprints(map, SCOPE_SOURCE_ID, true);
  footprints.set([{ id: "g1", bbox: [0, 0, 1, 1] }]);
  scope.set([{ id: "scope", bbox: [-10, -10, 10, 10] }]);

  expect([...map.sources.keys()].sort()).toEqual([FOOTPRINT_SOURCE_ID, SCOPE_SOURCE_ID].sort());
  expect(footprints.collection.features).toHaveLength(1);
  expect(scope.collection.features[0]?.geometry.coordinates[0]?.[0]).toEqual([-10, -10]);
  // The derived box is dashed: a box the user did not draw is not drawn
  // like one.
  const layer = map.layers.get(SCOPE_SOURCE_ID) as { paint?: Record<string, unknown> };
  expect(layer?.paint?.["line-dasharray"]).toEqual([2, 2]);
  expect(
    (map.layers.get(FOOTPRINT_LAYER_ID) as { paint?: Record<string, unknown> })?.paint?.[
      "line-dasharray"
    ],
  ).toBeUndefined();
});
