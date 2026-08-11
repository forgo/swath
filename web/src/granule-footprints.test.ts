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
  GranuleFootprints,
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
