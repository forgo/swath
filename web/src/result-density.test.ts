// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The density paint's contract without MapLibre: a fake map records the
// source and layer, so the tests prove the geometry (closed CRS84 rings
// from cells), the per-feature weight the ramp reads, and the style-swap
// survival that every paint in this app owes.
import { expect, test } from "vitest";
import type { DensityCell } from "./density-model.js";
import { DENSITY_SOURCE_ID } from "./granule-footprints.js";
import { densityCollection, ResultDensity } from "./result-density.js";

class FakeMap {
  sources = new Map<string, { data: unknown; setDataCalls: number }>();
  layers = new Map<string, object>();
  #listeners: (() => void)[] = [];

  getSource(id: string): unknown {
    const entry = this.sources.get(id);
    return entry === undefined
      ? undefined
      : {
          setData: (data: unknown) => {
            entry.data = data;
            entry.setDataCalls += 1;
          },
        };
  }
  addSource(id: string, source: { data?: unknown }): void {
    this.sources.set(id, { data: source.data, setDataCalls: 0 });
  }
  getLayer(id: string): unknown {
    return this.layers.get(id);
  }
  addLayer(layer: { id: string }): void {
    this.layers.set(layer.id, layer);
  }
  fitBounds(): void {}
  on(_type: string, listener: () => void): unknown {
    this.#listeners.push(listener);
    return this;
  }
  off(): unknown {
    return this;
  }
  /** What `setStyle` does: sources and layers are gone. */
  wipe(): void {
    this.sources.clear();
    this.layers.clear();
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

const cells: DensityCell[] = [
  { bbox: [0, 0, 1, 1], count: 10, weight: 1 },
  { bbox: [1, 0, 2, 1], count: 5, weight: 0.5 },
];

test("cells become closed CRS84 rings carrying their own weight", () => {
  const collection = densityCollection(cells);
  expect(collection.features).toHaveLength(2);
  const ring = collection.features[0]?.geometry.coordinates[0];
  expect(ring?.[0]).toEqual([0, 0]);
  expect(ring?.at(-1)).toEqual(ring?.[0]);
  // The ramp reads the feature, not the layer: one paint, many weights.
  expect(collection.features.map((f) => f.properties.weight)).toEqual([1, 0.5]);
});

test("the surface is one source and one fill layer, updated in place", () => {
  const map = new FakeMap();
  const density = new ResultDensity(map);
  density.set(cells);
  expect(map.sources.has(DENSITY_SOURCE_ID)).toBe(true);
  expect((map.layers.get(DENSITY_SOURCE_ID) as { type?: string })?.type).toBe("fill");

  density.set(cells.slice(0, 1));
  expect(map.sources.size).toBe(1);
  expect(map.sources.get(DENSITY_SOURCE_ID)?.setDataCalls).toBe(1);
  expect(density.collection.features).toHaveLength(1);
});

test("a style swap re-adds the surface rather than losing it", () => {
  const map = new FakeMap();
  const density = new ResultDensity(map);
  density.set(cells);
  map.wipe();
  expect(map.sources.has(DENSITY_SOURCE_ID)).toBe(true);
  expect(map.layers.has(DENSITY_SOURCE_ID)).toBe(true);
});

test("clearing paints nothing rather than tearing the layer down mid-style", () => {
  const map = new FakeMap();
  const density = new ResultDensity(map);
  density.set(cells);
  density.clear();
  expect(density.collection.features).toEqual([]);
  expect(map.layers.has(DENSITY_SOURCE_ID)).toBe(true);
});
