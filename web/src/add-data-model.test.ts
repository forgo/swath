// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The add-data semantics (issue #197), pure and DOM-free: capability
// detection, link classification, draft derivation, plain-words issues,
// the #248 request bodies, the quick-look graph, and RFC 7807 mapping.

import { expect, test } from "vitest";
import {
  bandIssue,
  brightestIssue,
  classifyLink,
  cogDraft,
  datasetBody,
  datasetIdIssue,
  datetimeIssue,
  granuleBody,
  mapProblem,
  parseCapabilities,
  quicklookBands,
  quicklookService,
  slug,
  stacDraft,
} from "./add-data-model.js";

// --- Capabilities (#198: the endpoints array states what is mounted) ---

test("capabilities come from the endpoints array, never a guess", () => {
  const writable = {
    endpoints: [
      { path: "/datasets", methods: ["POST"] },
      { path: "/uploads/{filename}", methods: ["PUT"] },
    ],
  };
  expect(parseCapabilities(writable)).toEqual({ register: true, upload: true });

  // Read-only serving filters the write methods out (#198): /datasets
  // disappears, granule browsing keeps GET — nothing here allows writing.
  const readOnly = {
    endpoints: [
      { path: "/datasets/{dataset_id}/granules", methods: ["GET"] },
      { path: "/result", methods: ["POST"] },
    ],
  };
  expect(parseCapabilities(readOnly)).toEqual({ register: false, upload: false });

  // A plain OGC landing page (static mode) has no endpoints at all.
  expect(parseCapabilities({ links: [] })).toEqual({ register: false, upload: false });
  expect(parseCapabilities(undefined)).toEqual({ register: false, upload: false });
});

// --- Link classification and drafts ---

test("a .json link is a STAC item; everything else goes to the server", () => {
  expect(classifyLink("https://x.test/item.json")).toBe("stac");
  expect(classifyLink("https://x.test/ITEM.JSON?token=1")).toBe("stac");
  expect(classifyLink("scene-b04.tif")).toBe("cog");
  expect(classifyLink("s3://bucket/scene.tif")).toBe("cog");
});

test("a pasted raster link seeds ids from its file name", () => {
  const draft = cogDraft("https://x.test/scenes/HLS.S30 T13SDD.tif");
  expect(draft.kind).toBe("cog");
  expect(draft.href).toBe("https://x.test/scenes/HLS.S30 T13SDD.tif");
  expect(draft.datasetId).toBe("hls-s30-t13sdd");
  expect(draft.granuleId).toBe("hls-s30-t13sdd");
  expect(draft.datetime).toBe("");
  expect(slug("Weird  name!!")).toBe("weird-name");
});

const ITEM = {
  type: "Feature",
  stac_version: "1.1.0",
  id: "scene-1",
  collection: "hls-demo",
  bbox: [-105.5, 39.2, -105.4, 39.3],
  properties: { datetime: "2024-06-06T17:54:00Z" },
  assets: {
    b04: { href: "scene-1-b04.tif" },
    b8a: { href: "scene-1-b8a.tif" },
  },
};

test("a STAC item pre-fills the whole draft", () => {
  const draft = stacDraft(ITEM);
  expect(draft).not.toBeTypeOf("string");
  if (typeof draft === "string") {
    return;
  }
  expect(draft.kind).toBe("stac");
  expect(draft.datasetId).toBe("hls-demo");
  expect(draft.bands).toEqual(["b04", "b8a"]);
  expect(draft.granuleId).toBe("scene-1");
  expect(draft.datetime).toBe("2024-06-06T17:54:00Z");
});

test("a non-item document is refused in plain words", () => {
  expect(stacDraft([1, 2])).toContain("not a STAC Item");
  expect(stacDraft({ type: "Collection" })).toContain('"type": "Feature"');
  expect(stacDraft({ ...ITEM, collection: undefined })).toContain("names no collection");
  expect(stacDraft({ ...ITEM, properties: {} })).toContain("properties.datetime");
  expect(stacDraft({ ...ITEM, assets: {} })).toContain("no assets");
});

// --- Inline validation phrases ---

test("issues are plain lowercase phrases mirroring the server's rules", () => {
  expect(datasetIdIssue("")).toBe("required");
  expect(datasetIdIssue("no/slashes")).toBe("use only letters, digits, - and _");
  expect(datasetIdIssue("api-hls_2")).toBe("");
  expect(bandIssue(" ")).toBe("name the band this file carries");
  expect(datetimeIssue("2024-06-06")).toContain("RFC 3339");
  expect(datetimeIssue("2024-06-06T17:54:00Z")).toBe("");
  expect(brightestIssue("0")).toBe("a positive number");
  expect(brightestIssue("10000")).toBe("");
});

// --- Request bodies (#248) ---

test("the bodies match the dataset surface's contract", () => {
  const draft = cogDraft("uploads/scene-b04.tif");
  draft.datasetId = "dropped";
  draft.bands = ["b04"];
  draft.granuleId = "scene-1";
  draft.datetime = "2024-06-06T17:54:00Z";
  expect(datasetBody(draft)).toMatchObject({ id: "dropped", bands: ["b04"] });
  expect(granuleBody(draft)).toEqual({
    id: "scene-1",
    datetime: "2024-06-06T17:54:00Z",
    assets: { b04: "uploads/scene-b04.tif" },
  });

  const stac = stacDraft(ITEM);
  if (typeof stac === "string") {
    throw new Error(stac);
  }
  // The inline form: the document itself, verbatim — the server never
  // fetches URLs; this panel supplies what it read in-browser.
  expect(granuleBody(stac)).toEqual({ stac_item: ITEM });
});

// --- The quick look (through the engine, never client-side) ---

test("three or more bands compose RGB, preferring color names", () => {
  expect(quicklookBands(["b02", "b03", "b04", "b8a"])).toEqual(["b04", "b03", "b02"]);
  expect(quicklookBands(["red", "green", "blue"])).toEqual(["red", "green", "blue"]);
  expect(quicklookBands(["a", "b", "c"])).toEqual(["a", "b", "c"]);
  expect(quicklookBands(["b04"])).toEqual(["b04"]);
});

test("the service body is an xyz service whose graph loads, scales, saves", () => {
  const rgb = quicklookService("hls-demo", "HLS demo", ["b02", "b03", "b04"], 10000);
  expect(rgb["type"]).toBe("xyz");
  const graph = (rgb["process"] as { process_graph: Record<string, unknown> }).process_graph;
  expect(Object.keys(graph).sort()).toEqual(["load", "save", "scale"]);
  expect(graph["load"]).toMatchObject({
    process_id: "load_collection",
    arguments: { id: "hls-demo", bands: ["b04", "b03", "b02"] },
  });
  expect(graph["scale"]).toMatchObject({
    process_id: "linear_scale_range",
    arguments: { inputMin: 0, inputMax: 10000, outputMin: 0, outputMax: 255 },
  });

  // A single band reduces to gray first (an RGB composite needs three).
  const gray = quicklookService("d", "D", ["b04"], 10000);
  const grayGraph = (gray["process"] as { process_graph: Record<string, unknown> }).process_graph;
  expect(Object.keys(grayGraph).sort()).toEqual(["gray", "load", "save", "scale"]);
  expect(grayGraph["gray"]).toMatchObject({ process_id: "reduce_dimension" });
});

// --- RFC 7807 problems onto fields ---

test("server refusals land under the field that caused them", () => {
  const problem = (detail: string) => ({
    type: "about:blank",
    title: "Bad Request",
    status: 400,
    detail,
  });
  expect(
    mapProblem(400, problem("asset `b04` (no-such-file.tif) failed header validation: …")),
  ).toMatchObject({ field: "link" });
  expect(mapProblem(400, problem("dataset id `x y` is not URL-safe (…)"))).toEqual({
    field: "dataset",
    note: "use only letters, digits, - and _",
  });
  expect(
    mapProblem(400, problem("asset band `fmask` is not in dataset `d`'s declared bands …")),
  ).toMatchObject({ field: "band" });
  expect(mapProblem(400, problem("stac_item does not describe a granule: …"))).toMatchObject({
    field: "link",
  });
  // The inline item's collection must agree with the dataset id — that
  // refusal is about the dataset choice, not the link (review round 1).
  expect(
    mapProblem(400, problem("stac_item collection `hls-demo` does not match dataset `edited`")),
  ).toMatchObject({ field: "dataset" });
  expect(mapProblem(400, problem("`datetime`: not RFC 3339"))).toMatchObject({
    field: "datetime",
  });
  // Unknown shapes degrade to the status, never a crash.
  expect(mapProblem(502, "gateway text")).toEqual({
    field: "",
    note: "the server refused with HTTP 502",
  });
});
