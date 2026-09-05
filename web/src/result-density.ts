// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Painting the density surface (#413): the counts endpoint's cells as a
 * weighted fill, so a scope with hundreds of results shows *where* they
 * are instead of a hundred overlapping outlines.
 *
 * Same shape as `GranuleFootprints` — its own source and layer, re-added
 * on every style swap — but a fill rather than a line, ramped on the
 * weight each cell carries.
 */
import type { DensityCell } from "./density-model.js";
import { DENSITY_SOURCE_ID, type FootprintMapLike } from "./granule-footprints.js";
import { readToken } from "./ui/styles.js";

interface CellFeature {
  type: "Feature";
  properties: { count: number; weight: number };
  geometry: { type: "Polygon"; coordinates: [number, number][][] };
}

export interface DensityCollection {
  type: "FeatureCollection";
  features: CellFeature[];
}

/** The cells as closed CRS84 rings, each carrying its own weight so the
 * paint ramps per feature rather than per layer. */
export function densityCollection(cells: readonly DensityCell[]): DensityCollection {
  return {
    type: "FeatureCollection",
    features: cells.map((cell) => {
      const [west, south, east, north] = cell.bbox;
      return {
        type: "Feature",
        properties: { count: cell.count, weight: cell.weight },
        geometry: {
          type: "Polygon",
          coordinates: [
            [
              [west, south],
              [east, south],
              [east, north],
              [west, north],
              [west, south],
            ],
          ],
        },
      };
    }),
  };
}

/** Keeps the density source + fill layer on the map across style
 * re-applies. An empty set is painted as empty rather than torn down, so
 * a swap back to footprints cannot race the style. */
export class ResultDensity {
  readonly #map: FootprintMapLike;
  readonly #onStyleData: () => void;
  #collection: DensityCollection = { type: "FeatureCollection", features: [] };
  #disposed = false;

  constructor(map: FootprintMapLike) {
    this.#map = map;
    this.#onStyleData = () => this.#paint();
    this.#map.on("styledata", this.#onStyleData);
  }

  get collection(): DensityCollection {
    return this.#collection;
  }

  set(cells: readonly DensityCell[]): void {
    this.#collection = densityCollection(cells);
    this.#paint();
  }

  clear(): void {
    this.set([]);
  }

  dispose(): void {
    this.#disposed = true;
    this.#map.off("styledata", this.#onStyleData);
  }

  #paint(): void {
    if (this.#disposed) {
      return;
    }
    try {
      const source = this.#map.getSource(DENSITY_SOURCE_ID) as
        | { setData?: (data: DensityCollection) => void }
        | undefined;
      if (source?.setData) {
        source.setData(this.#collection);
      } else if (!source) {
        this.#map.addSource(DENSITY_SOURCE_ID, { type: "geojson", data: this.#collection });
      }
      if (!this.#map.getLayer(DENSITY_SOURCE_ID)) {
        this.#map.addLayer({
          id: DENSITY_SOURCE_ID,
          type: "fill",
          source: DENSITY_SOURCE_ID,
          // MapLibre paint cannot read custom properties: the token is
          // resolved at paint time (ui-system.md §4.1). The ramp is on
          // the feature's own weight, so the busiest cell is the most
          // opaque and an empty cell was never drawn.
          paint: {
            "fill-color": readToken("--swath-color-accent"),
            "fill-opacity": ["interpolate", ["linear"], ["get", "weight"], 0, 0.08, 1, 0.55],
          },
        });
      }
    } catch {
      // Style mid-swap: the next styledata repaints.
    }
  }
}
