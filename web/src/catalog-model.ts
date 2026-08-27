// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The catalog mode's pure model (issue #288): what `/collections` and
 * `/datasets/{id}/granules` say, the bounded per-granule preview graph
 * (ADR 0014's budget, ADR 0019's "every pixel is the engine's"), and the
 * client-side filters/sorts over the granule list the server already
 * returned — no STAC vocabulary, no search surface (R2, the scope fence).
 */
import { quicklookBands } from "./add-data-model.js";
import type { GranuleBbox } from "./granule-footprints.js";
import { parseBbox } from "./granule-footprints.js";
import type { LonLatBounds } from "./swath-map.js";

export interface CatalogDataset {
  id: string;
  title: string;
  /** Band names from `cube:dimensions` (empty when undeclared). */
  bands: string[];
}

export interface CatalogGranule {
  id: string;
  bbox: GranuleBbox;
  /** ISO datetime, "" when the granule carries none. */
  datetime: string;
}

export type GranuleSort = "newest" | "oldest" | "id";

export interface GranuleFilter {
  /** Inclusive ISO date (YYYY-MM-DD) bounds; either may be absent. */
  from?: string | undefined;
  to?: string | undefined;
  /** Keep only granules whose bbox intersects these bounds. */
  view?: LonLatBounds | undefined;
}

/** The linear stretch input maximum for reflectance-scaled bands; the
 * add-data flow's default (10000 = HLS surface reflectance × 10⁴). */
export const PREVIEW_BRIGHTEST = 10000;

/** Same guidance the dataset panel gave (e2e pins the wording). */
export const GRANULES_EMPTY_GUIDANCE =
  "No granules ingested yet. Drop a granule into the server's watched ingest " +
  "directory to register it (legacy formats: `swath ingest reference <granule>` " +
  "writes the manifest), then re-open this dataset.";

export function parseCollections(body: unknown): CatalogDataset[] {
  const docs = (body as { collections?: unknown[] } | null)?.collections ?? [];
  const out: CatalogDataset[] = [];
  for (const raw of docs) {
    if (typeof raw !== "object" || raw === null) {
      continue;
    }
    const doc = raw as Record<string, unknown>;
    if (typeof doc["id"] !== "string" || doc["id"] === "") {
      continue;
    }
    const cube = doc["cube:dimensions"] as
      | Record<string, { type?: string; values?: unknown[] }>
      | undefined;
    const values = Object.values(cube ?? {}).find((d) => d?.type === "bands")?.values ?? [];
    out.push({
      id: doc["id"],
      title: typeof doc["title"] === "string" && doc["title"] !== "" ? doc["title"] : doc["id"],
      bands: values.filter((band): band is string => typeof band === "string"),
    });
  }
  return out;
}

/** Granules with a usable footprint; malformed bboxes are skipped, not
 * painted wrong (the dataset panel's rule, kept). */
export function parseGranules(body: unknown): CatalogGranule[] {
  const items = (body as { granules?: unknown[] } | null)?.granules ?? [];
  const out: CatalogGranule[] = [];
  for (const raw of items) {
    if (typeof raw !== "object" || raw === null) {
      continue;
    }
    const item = raw as { id?: unknown; bbox?: unknown; datetime?: unknown };
    const bbox = parseBbox(item.bbox);
    if (typeof item.id === "string" && item.id !== "" && bbox) {
      out.push({
        id: item.id,
        bbox,
        datetime: typeof item.datetime === "string" ? item.datetime : "",
      });
    }
  }
  return out;
}

function intersects(bbox: GranuleBbox, view: LonLatBounds): boolean {
  const [west, south, east, north] = bbox;
  return !(east < view.west || west > view.east || north < view.south || south > view.north);
}

export function filterGranules(
  granules: readonly CatalogGranule[],
  filter: GranuleFilter,
): CatalogGranule[] {
  return granules.filter((granule) => {
    const day = granule.datetime.slice(0, 10);
    if (filter.from && (day === "" || day < filter.from)) {
      return false;
    }
    if (filter.to && (day === "" || day > filter.to)) {
      return false;
    }
    if (filter.view && !intersects(granule.bbox, filter.view)) {
      return false;
    }
    return true;
  });
}

export function sortGranules(
  granules: readonly CatalogGranule[],
  sort: GranuleSort,
): CatalogGranule[] {
  const list = [...granules];
  switch (sort) {
    case "newest":
      return list.sort((a, b) => b.datetime.localeCompare(a.datetime) || a.id.localeCompare(b.id));
    case "oldest":
      return list.sort((a, b) => a.datetime.localeCompare(b.datetime) || a.id.localeCompare(b.id));
    default:
      return list.sort((a, b) => a.id.localeCompare(b.id));
  }
}

/** The bounded preview graph for one granule: the dataset's quick look
 * (RGB when red/green/blue bands are declared, else the first band in
 * gray) over the granule's footprint, saved as PNG. The server frames
 * the named extent whole under ADR 0014's containing-tile rule and its
 * pixel budget — a refusal is the server's to make, and comes back as a
 * plain-words note. NOTE: previews render the dataset's *latest* granule
 * (the graph's `temporal_extent` is accepted-and-ignored until ADR 0015's
 * graph half, #181), so the picture is the current frame over this
 * footprint, not necessarily this granule's own pixels. */
export function previewGraph(
  dataset: CatalogDataset,
  granule: CatalogGranule,
): Record<string, unknown> {
  const picked = quicklookBands(dataset.bands);
  const [west, south, east, north] = granule.bbox;
  const load = {
    process_id: "load_collection",
    arguments: {
      id: dataset.id,
      spatial_extent: { west, south, east, north },
      temporal_extent: null,
      bands: picked.length === 0 ? null : picked,
    },
  };
  const scale = (from: string): Record<string, unknown> => ({
    process_id: "linear_scale_range",
    arguments: {
      x: { from_node: from },
      inputMin: 0,
      inputMax: PREVIEW_BRIGHTEST,
      outputMin: 0,
      outputMax: 255,
    },
  });
  const save = {
    process_id: "save_result",
    arguments: { data: { from_node: "scale" }, format: "png" },
    result: true,
  };
  if (picked.length === 3) {
    return { load, scale: scale("load"), save };
  }
  const gray = {
    process_id: "reduce_dimension",
    arguments: {
      data: { from_node: "load" },
      dimension: "bands",
      reducer: {
        process_graph: {
          pick: {
            process_id: "array_element",
            arguments: { data: { from_parameter: "data" }, label: picked[0] ?? 0 },
            result: true,
          },
        },
      },
    },
  };
  return { load, gray, scale: scale("gray"), save };
}

/** Preview kind for the card's meta line. */
export function previewKind(dataset: CatalogDataset): "rgb" | "gray" {
  return quicklookBands(dataset.bands).length === 3 ? "rgb" : "gray";
}
