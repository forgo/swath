// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import {
  type CatalogDataset,
  type CatalogGranule,
  filterGranules,
  instantWindow,
  parseCollections,
  parseGranules,
  previewGraph,
  previewKind,
  sortGranules,
} from "./catalog-model.js";

const HLS = {
  id: "hls-s30",
  title: "HLS S30",
  "cube:dimensions": { bands: { type: "bands", values: ["b02", "b03", "b04", "b8a"] } },
};

test("parseCollections: id + title + bands from cube:dimensions; junk skipped", () => {
  expect(
    parseCollections({ collections: [HLS, { id: "" }, 3, { title: "no id" }, { id: "bare" }] }),
  ).toEqual([
    { id: "hls-s30", title: "HLS S30", bands: ["b02", "b03", "b04", "b8a"] },
    { id: "bare", title: "bare", bands: [] },
  ]);
  expect(parseCollections(null)).toEqual([]);
});

test("parseGranules keeps only granules with a usable bbox", () => {
  expect(
    parseGranules({
      granules: [
        { id: "a", bbox: [10, 45, 11, 46], datetime: "2026-06-01T10:00:00Z" },
        { id: "b", bbox: [10, 45, 11] },
        { id: "", bbox: [10, 45, 11, 46] },
        { id: "c", bbox: [1, 2, 3, 4] },
      ],
    }),
  ).toEqual([
    { id: "a", bbox: [10, 45, 11, 46], datetime: "2026-06-01T10:00:00Z" },
    { id: "c", bbox: [1, 2, 3, 4], datetime: "" },
  ]);
});

const GRANULES = [
  { id: "FIX.B.2026", bbox: [11, 45.5, 12, 46.2] as const, datetime: "2026-05-24T10:12:00Z" },
  { id: "FIX.A.2026", bbox: [10.3, 45.6, 11.1, 46.4] as const, datetime: "2026-06-01T10:00:00Z" },
  { id: "undated", bbox: [0, 0, 1, 1] as const, datetime: "" },
];

test("filterGranules: inclusive date days, intersecting view; undated granules fail date filters", () => {
  expect(filterGranules(GRANULES, { from: "2026-06-01" }).map((g) => g.id)).toEqual(["FIX.A.2026"]);
  expect(filterGranules(GRANULES, { to: "2026-05-24" }).map((g) => g.id)).toEqual(["FIX.B.2026"]);
  expect(
    filterGranules(GRANULES, { view: { west: 10, south: 45, east: 10.5, north: 47 } }).map(
      (g) => g.id,
    ),
  ).toEqual(["FIX.A.2026"]);
  expect(filterGranules(GRANULES, {})).toHaveLength(3);
});

test("sortGranules: newest / oldest by datetime then id, or by id", () => {
  expect(sortGranules(GRANULES, "newest").map((g) => g.id)).toEqual([
    "FIX.A.2026",
    "FIX.B.2026",
    "undated",
  ]);
  expect(sortGranules(GRANULES, "oldest").map((g) => g.id)).toEqual([
    "undated",
    "FIX.B.2026",
    "FIX.A.2026",
  ]);
  expect(sortGranules(GRANULES, "id").map((g) => g.id)).toEqual([
    "FIX.A.2026",
    "FIX.B.2026",
    "undated",
  ]);
});

test("previewGraph: RGB quick look, extent left to the server (footprint-framed), gray when no RGB triple", () => {
  const hls = parseCollections({ collections: [HLS] })[0] as CatalogDataset;
  const rgb = previewGraph(hls, GRANULES[1] as CatalogGranule);
  expect(Object.keys(rgb)).toEqual(["load", "scale", "save"]);
  expect((rgb["load"] as { arguments: Record<string, unknown> }).arguments).toEqual({
    id: "hls-s30",
    spatial_extent: null,
    // Pinned to GRANULES[1]'s own instant, not left open (#406).
    temporal_extent: ["2026-06-01T10:00:00.000Z", "2026-06-01T10:00:00.001Z"],
    bands: ["b04", "b03", "b02"],
  });
  expect(previewKind(hls)).toBe("rgb");
  const single = { id: "ndvi-only", title: "x", bands: ["ndvi"] };
  const gray = previewGraph(single, GRANULES[0] as CatalogGranule);
  expect(Object.keys(gray)).toEqual(["load", "gray", "scale", "save"]);
  expect(previewKind(single)).toBe("gray");
});

test("previewGraph: two granules of one dataset ask for two different instants (#406)", () => {
  const hls = parseCollections({ collections: [HLS] })[0] as CatalogDataset;
  const windowOf = (granule: CatalogGranule): unknown =>
    (previewGraph(hls, granule)["load"] as { arguments: Record<string, unknown> }).arguments[
      "temporal_extent"
    ];
  // The bug this replaces: every card rendered the dataset's LATEST
  // granule, so a grid of cards was N copies of one picture.
  expect(windowOf(GRANULES[0] as CatalogGranule)).not.toEqual(
    windowOf(GRANULES[1] as CatalogGranule),
  );
});

test("instantWindow: left-closed, right-open, one millisecond wide", () => {
  // The server compiles [start, end) to the inclusive end-1ms, so this is
  // the smallest window that contains the instant and nothing after it.
  expect(instantWindow("2026-05-24T10:12:00Z")).toEqual([
    "2026-05-24T10:12:00.000Z",
    "2026-05-24T10:12:00.001Z",
  ]);
  // Sub-second precision survives to the millisecond the server resolves at.
  expect(instantWindow("2026-05-24T10:12:00.500Z")).toEqual([
    "2026-05-24T10:12:00.500Z",
    "2026-05-24T10:12:00.501Z",
  ]);
  // A window is never fabricated: no datetime, or an unparseable one,
  // keeps the open query the provider always sent.
  expect(instantWindow("")).toBeNull();
  expect(instantWindow("not a datetime")).toBeNull();
});

test("previewGraph: an undated granule keeps the open window", () => {
  const hls = parseCollections({ collections: [HLS] })[0] as CatalogDataset;
  const undated = GRANULES.find((g) => g.datetime === "") as CatalogGranule | undefined;
  expect(undated).toBeDefined();
  const graph = previewGraph(hls, undated as CatalogGranule);
  expect(
    (graph["load"] as { arguments: Record<string, unknown> }).arguments["temporal_extent"],
  ).toBeNull();
});
