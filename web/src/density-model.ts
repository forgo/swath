// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The density surface (#413). Above a threshold, N overlapping outlines
 * are noise rather than information, so the map stops drawing footprints
 * and draws where the results *are* — from the counts endpoint's cell
 * bucketing (#410), never from the page the panel happens to hold.
 */
import type { GranuleBbox } from "./granule-footprints.js";

/**
 * Where footprints stop informing and start obscuring.
 *
 * The reason, not a taste: the fixture footprints are ~0.2° across and a
 * data-mode map at the default zoom shows roughly 20° of longitude, so a
 * hundred outlines put about five edges through every degree of the view
 * — past the point where any one of them can be traced. Forty is where a
 * reader can still follow a single outline to its corners, so forty is
 * where the surface takes over.
 */
export const DENSITY_THRESHOLD = 40;

/** The lattice the density asks for, in CRS84 degrees. One degree is
 * about two fixture footprints wide: fine enough to show structure,
 * coarse enough that the answer stays under the counts route's bucket
 * cap for any scope a person can pan to. */
export const DENSITY_CELL_DEGREES = 1;

/** Previews asked for without waiting to be scrolled to: about a
 * screenful of cards in the two-column grid. Leading with pictures is the
 * point — the first screen should be full before anyone scrolls — and
 * everything past it waits until it is nearly in view. */
export const EAGER_PREVIEWS = 12;

export interface DensityCell {
  bbox: GranuleBbox;
  count: number;
  /** `count` as a fraction of the busiest cell, 0..1 — what the paint
   * ramps on. */
  weight: number;
}

export interface Density {
  cells: DensityCell[];
  /** The busiest cell's count; 0 when there are no cells. */
  peak: number;
}

export const EMPTY_DENSITY: Density = { cells: [], peak: 0 };

/** A cell counts answer as a density surface. Rows that are not cells
 * (a time bucketing, a malformed row) are skipped rather than drawn at
 * an invented place. */
export function buildDensity(body: unknown): Density {
  const raw = (body as { buckets?: unknown[] } | null)?.buckets ?? [];
  const cells: { bbox: GranuleBbox; count: number }[] = [];
  let peak = 0;
  for (const row of raw) {
    if (typeof row !== "object" || row === null) {
      continue;
    }
    const bucket = row as { bbox?: unknown; count?: unknown };
    const bbox = bucket.bbox;
    if (!Array.isArray(bbox) || bbox.length !== 4 || !bbox.every(Number.isFinite)) {
      continue;
    }
    const count = typeof bucket.count === "number" && bucket.count > 0 ? bucket.count : 0;
    if (count === 0) {
      continue;
    }
    cells.push({ bbox: bbox as unknown as GranuleBbox, count });
    peak = Math.max(peak, count);
  }
  return {
    peak,
    cells: cells.map((cell) => ({ ...cell, weight: peak > 0 ? cell.count / peak : 0 })),
  };
}

/** Whether `count` results are past the point where outlines inform. */
export function isDense(count: number): boolean {
  return count > DENSITY_THRESHOLD;
}

/** What the map is showing, in words the panel can say out loud. */
export function densityNote(density: Density, count: number): string {
  if (density.cells.length === 0) {
    return `${count} results — the map has no density to draw yet.`;
  }
  return `${count} results — the map shows where they are, not each outline.`;
}
