// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The search field's spatial scope (#412), and its honesty.
 *
 * `GranuleQuery` is `bbox` + `datetime` — that is the entire search
 * surface. So the field states its scope with a mode tag rather than
 * leaving it to an icon, and a pasted geometry is **reduced to its
 * bounding box and says so**. That one line of copy prevents a whole
 * class of wrong results from being believed.
 *
 * When the catalog port gains an `intersects` predicate the tag gains a
 * third value and the reduction note goes away. Until then this is the
 * truth, and nothing here claims otherwise.
 */
import type { GranuleBbox } from "./granule-footprints.js";

/** What the spatial filter is, in the field's own words. `viewport`
 * follows the map; `bbox` is a box the user gave. There is no `shape`:
 * the port cannot do one. */
export type ScopeMode = "viewport" | "bbox";

export interface SpatialScope {
  mode: ScopeMode;
  /** The box actually sent to the server, CRS84 `[w, s, e, n]`. */
  bbox: GranuleBbox;
  /** True when the user gave a shape and this box is its envelope. */
  reduced: boolean;
}

/** The one line that keeps a reduction from being believed as a shape
 * search. The e2e pins it. */
export const REDUCED_NOTE = "Using the bounding box of the shape you pasted.";

/** What the tag reads for each mode — never an icon alone. */
export function scopeTag(mode: ScopeMode): string {
  return mode === "viewport" ? "viewport" : "bbox";
}

export type ScopeParse =
  | { ok: true; bbox: GranuleBbox; reduced: boolean }
  | { ok: false; reason: string };

function finite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

/** Every coordinate pair inside an arbitrarily nested GeoJSON coordinate
 * array. Positions may carry elevation; only the first two are read. */
function positions(node: unknown, out: [number, number][]): void {
  if (!Array.isArray(node)) {
    return;
  }
  if (finite(node[0]) && finite(node[1])) {
    out.push([node[0], node[1]]);
    return;
  }
  for (const child of node) {
    positions(child, out);
  }
}

function envelope(points: [number, number][]): GranuleBbox | undefined {
  if (points.length === 0) {
    return undefined;
  }
  let [west, south] = points[0] as [number, number];
  let [east, north] = points[0] as [number, number];
  for (const [lon, lat] of points) {
    west = Math.min(west, lon);
    east = Math.max(east, lon);
    south = Math.min(south, lat);
    north = Math.max(north, lat);
  }
  return [west, south, east, north];
}

/** Coordinates out of anything GeoJSON-shaped: a geometry, a Feature, a
 * FeatureCollection, or a GeometryCollection. */
function geojsonPositions(doc: unknown): [number, number][] {
  const points: [number, number][] = [];
  const visit = (node: unknown): void => {
    if (typeof node !== "object" || node === null) {
      return;
    }
    const item = node as Record<string, unknown>;
    if (item["coordinates"] !== undefined) {
      positions(item["coordinates"], points);
    }
    for (const key of ["geometry", "geometries", "features"]) {
      const child = item[key];
      if (Array.isArray(child)) {
        for (const each of child) {
          visit(each);
        }
      } else if (child !== undefined) {
        visit(child);
      }
    }
  };
  visit(doc);
  return points;
}

/**
 * A pasted spatial filter: either four numbers (a box, used as given) or
 * GeoJSON (reduced to its envelope, and `reduced` says so). Anything else
 * is refused with a reason rather than silently ignored — a filter that
 * quietly did nothing is worse than one that says it cannot.
 */
export function parseSpatialInput(raw: string): ScopeParse {
  const text = raw.trim();
  if (text === "") {
    return { ok: false, reason: "Nothing to search in yet." };
  }

  const numbers = text.split(/[\s,]+/).filter((part) => part !== "");
  if (numbers.length === 4 && numbers.every((part) => finite(Number(part)))) {
    const [west, south, east, north] = numbers.map(Number) as [number, number, number, number];
    if (south > north) {
      return {
        ok: false,
        reason: "South is above north — check the order: west, south, east, north.",
      };
    }
    return { ok: true, bbox: [west, south, east, north], reduced: false };
  }

  let doc: unknown;
  try {
    doc = JSON.parse(text);
  } catch {
    return {
      ok: false,
      reason: "Paste four numbers (west, south, east, north) or a GeoJSON shape.",
    };
  }
  const box = envelope(geojsonPositions(doc));
  if (box === undefined) {
    return { ok: false, reason: "That GeoJSON carries no coordinates to search in." };
  }
  return { ok: true, bbox: box, reduced: true };
}
