// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Granule footprints as a MapLibre layer (issue #110): the map-side half
 * of the dataset browser. `<swath-dataset-panel>` owns the fetching and
 * the list UI; this module owns the paint — a GeoJSON source of bbox
 * polygons plus a line layer of outlines — and the zoom-to-footprint
 * move. The page shell (demo/main.ts) wires the two through events, the
 * same seam pattern as the layer panel.
 *
 * Style-swap survival: `<swath-map>` re-applies its whole style on every
 * layer switch (`setStyle` wipes sources and layers), so the module
 * listens for `styledata` and re-adds its source and layer whenever they
 * are missing. Adding into a style mid-swap can throw; failures are
 * swallowed because the next `styledata` repaints anyway.
 *
 * Like [`XRayMapLike`](./swath-xray.ts), the [`FootprintMapLike`] slice
 * keeps the dependency narrow: unit tests drive a fake map, and the real
 * MapLibre `Map` satisfies it structurally.
 */

/** A WGS84 footprint, CRS84 order: `[west, south, east, north]` — the
 * `bbox` field of the granules API (`GET /datasets/{id}/granules`). */
export type GranuleBbox = readonly [number, number, number, number];

/** The one granule slice the footprint paint needs. */
export interface FootprintGranule {
  id: string;
  bbox: GranuleBbox;
}

/** GeoJSON id of both the footprint source and its line layer. */
export const FOOTPRINT_SOURCE_ID = "swath-granule-footprints";
/** Layer id — same as the source id; one layer per source. */
export const FOOTPRINT_LAYER_ID = "swath-granule-footprints";

/** The slice of a MapLibre `Map` the footprint paint uses. */
export interface FootprintMapLike {
  /** The source if present; a GeoJSON source exposes `setData`. */
  getSource(id: string): unknown;
  addSource(id: string, source: object): void;
  getLayer(id: string): unknown;
  addLayer(layer: object): void;
  fitBounds(bounds: [[number, number], [number, number]], options: object): void;
  on(type: string, listener: () => void): unknown;
  off(type: string, listener: () => void): unknown;
}

/** A GeoJSON Feature of one footprint (Polygon ring from the bbox). */
interface FootprintFeature {
  type: "Feature";
  properties: { id: string };
  geometry: { type: "Polygon"; coordinates: [number, number][][] };
}

/** The FeatureCollection the source carries. */
export interface FootprintCollection {
  type: "FeatureCollection";
  features: FootprintFeature[];
}

/** Footprint bboxes as a GeoJSON FeatureCollection of closed Polygon
 * rings (counter-clockwise, first point repeated last per RFC 7946). */
export function footprintCollection(granules: readonly FootprintGranule[]): FootprintCollection {
  return {
    type: "FeatureCollection",
    features: granules.map((granule) => {
      const [west, south, east, north] = granule.bbox;
      return {
        type: "Feature",
        properties: { id: granule.id },
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

/** Keeps a footprint source + outline layer on the map, across style
 * re-applies, and zooms to a footprint on request. */
/** Retry pacing when a paint lands mid style swap (see `#paint`). */
const PAINT_RETRY_MS = 200;

export class GranuleFootprints {
  readonly #map: FootprintMapLike;
  readonly #onStyleData: () => void;
  #collection: FootprintCollection = { type: "FeatureCollection", features: [] };
  #retryTimer: number | undefined;
  #disposed = false;

  constructor(map: FootprintMapLike) {
    this.#map = map;
    // Every style (re)apply wipes sources/layers; re-add on styledata.
    this.#onStyleData = () => this.#paint();
    this.#map.on("styledata", this.#onStyleData);
  }

  /** The currently painted collection (test seam). */
  get collection(): FootprintCollection {
    return this.#collection;
  }

  /** Replaces the painted footprints. */
  set(granules: readonly FootprintGranule[]): void {
    this.#collection = footprintCollection(granules);
    this.#paint();
  }

  /** Removes every footprint (an empty collection stays cheap to keep —
   * no source/layer teardown to race the style swap). */
  clear(): void {
    this.set([]);
  }

  /** Fits the view to one footprint. `duration: 0` — deterministic for
   * tests, and a jump reads clearer than an animation across the world. */
  zoomTo(bbox: GranuleBbox): void {
    const [west, south, east, north] = bbox;
    this.#map.fitBounds(
      [
        [west, south],
        [east, north],
      ],
      { padding: 48, duration: 0 },
    );
  }

  /** Detaches the styledata listener; the instance is dead afterwards.
   * Painted footprints simply vanish with the next style re-apply. */
  dispose(): void {
    this.#disposed = true;
    this.#map.off("styledata", this.#onStyleData);
    if (this.#retryTimer !== undefined) {
      window.clearTimeout(this.#retryTimer);
      this.#retryTimer = undefined;
    }
  }

  #paint(): void {
    if (this.#disposed) {
      return;
    }
    try {
      const source = this.#map.getSource(FOOTPRINT_SOURCE_ID) as
        | { setData?: (data: FootprintCollection) => void }
        | undefined;
      if (source?.setData) {
        source.setData(this.#collection);
      } else if (!source) {
        this.#map.addSource(FOOTPRINT_SOURCE_ID, { type: "geojson", data: this.#collection });
      }
      if (!this.#map.getLayer(FOOTPRINT_LAYER_ID)) {
        this.#map.addLayer({
          id: FOOTPRINT_LAYER_ID,
          type: "line",
          source: FOOTPRINT_SOURCE_ID,
          paint: { "line-color": "#4ade80", "line-width": 2, "line-opacity": 0.9 },
        });
      }
    } catch {
      // Style mid-swap: the next styledata usually repaints, but nothing
      // guarantees another style event ever arrives — retry on a short
      // timer too, so a single unlucky paint can't lose the footprints.
      if (this.#retryTimer === undefined) {
        this.#retryTimer = window.setTimeout(() => {
          this.#retryTimer = undefined;
          this.#paint();
        }, PAINT_RETRY_MS);
      }
    }
  }
}
