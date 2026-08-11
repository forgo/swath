// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The always-valid canvas's pure pipeline model (issue #151, design
// note §4): the stage table that mirrors the process compiler's cube
// discipline (crates/swath-render/src/process.rs — these tests are the
// client half of the "pinned by tests on both sides" drift mitigation),
// the insertion rule derived from it, and the formula builder's row
// model with its lowering to the reduce_dimension reducer child graph.
import { expect, test } from "vitest";
import {
  buildReducerGraph,
  canInsertAt,
  type FormulaRow,
  finalStage,
  formulaIssues,
  formulaPhrase,
  insertableAt,
  LOAD_STAGE,
  locateServerError,
  pickBand,
  transition,
} from "./authoring-model.js";

const ALL = new Set([
  "load_collection",
  "ndvi",
  "reduce_dimension",
  "array_element",
  "add",
  "subtract",
  "multiply",
  "divide",
  "linear_scale_range",
  "save_result",
]);

test("the stage table mirrors the compiler: reduce needs multi+unscaled, scale needs unscaled", () => {
  // load_collection's output: a multi-band, unscaled cube.
  expect(LOAD_STAGE).toEqual({ kind: "multi", scaled: false });
  // ndvi / a formula reduce a multi-band cube to gray…
  expect(transition(LOAD_STAGE, "ndvi")).toEqual({ kind: "gray", scaled: false });
  expect(transition(LOAD_STAGE, "reduce_dimension")).toEqual({ kind: "gray", scaled: false });
  // …and reject a gray or already-scaled cube (the compiler's
  // unscaled_multi check — B4's other half).
  expect(transition({ kind: "gray", scaled: false }, "ndvi")).toBeNull();
  expect(transition({ kind: "multi", scaled: true }, "ndvi")).toBeNull();
  // linear_scale_range marks any unscaled cube scaled, once.
  expect(transition(LOAD_STAGE, "linear_scale_range")).toEqual({ kind: "multi", scaled: true });
  expect(transition({ kind: "gray", scaled: false }, "linear_scale_range")).toEqual({
    kind: "gray",
    scaled: true,
  });
  expect(transition({ kind: "gray", scaled: true }, "linear_scale_range")).toBeNull();
  // Anything else is not a top-level pipeline step at all (B2/B3).
  for (const id of ["add", "subtract", "multiply", "divide", "array_element"]) {
    expect(transition(LOAD_STAGE, id), id).toBeNull();
  }
});

test("B4: scale-before-reduce does not type end to end, so it cannot be inserted", () => {
  // Inserting scale before an existing reduce step breaks the chain…
  expect(canInsertAt(["ndvi"], 0, "linear_scale_range")).toBe(false);
  // …while reduce-before-an-existing-scale is fine (the NDVI shape).
  expect(canInsertAt(["linear_scale_range"], 0, "ndvi")).toBe(true);
  // After a scale step nothing reduces or re-scales.
  expect(canInsertAt(["linear_scale_range"], 1, "ndvi")).toBe(false);
  expect(canInsertAt(["linear_scale_range"], 1, "linear_scale_range")).toBe(false);
  // The complete NDVI chain types, and is full: nothing fits any gap.
  expect(finalStage(["ndvi", "linear_scale_range"])).toEqual({ kind: "gray", scaled: true });
  for (const gap of [0, 1, 2]) {
    expect(insertableAt(["ndvi", "linear_scale_range"], gap, ALL)).toEqual([]);
  }
});

test("insertion offers only served processes; the formula chip needs its whole toolkit", () => {
  expect(insertableAt([], 0, ALL)).toEqual(["ndvi", "reduce_dimension", "linear_scale_range"]);
  const without = (...ids: string[]): Set<string> =>
    new Set([...ALL].filter((id) => !ids.includes(id)));
  expect(insertableAt([], 0, without("ndvi"))).toEqual(["reduce_dimension", "linear_scale_range"]);
  // A formula lowers to reduce_dimension + array_element + arithmetic:
  // remove any leg and the chip is gone (schema honesty — the table
  // only orders processes that exist).
  expect(insertableAt([], 0, without("reduce_dimension"))).toEqual(["ndvi", "linear_scale_range"]);
  expect(insertableAt([], 0, without("array_element"))).toEqual(["ndvi", "linear_scale_range"]);
  expect(insertableAt([], 0, without("add", "subtract", "multiply", "divide"))).toEqual([
    "ndvi",
    "linear_scale_range",
  ]);
  // One surviving arithmetic op is enough.
  expect(insertableAt([], 0, without("add", "subtract", "multiply"))).toContain("reduce_dimension");
});

/** The NDVI formula as rows: r1 = b8a − b04; r2 = b8a + b04; r3 = r1 ÷ r2. */
const NDVI_ROWS: FormulaRow[] = [
  { op: "subtract", left: { kind: "band", band: "b8a" }, right: { kind: "band", band: "b04" } },
  { op: "add", left: { kind: "band", band: "b8a" }, right: { kind: "band", band: "b04" } },
  { op: "divide", left: { kind: "row", index: 0 }, right: { kind: "row", index: 1 } },
];

test("the formula flags its gaps in plain words, against the loaded-band vocabulary", () => {
  expect(formulaIssues([], ["b8a"])).toEqual(["add a line to the formula"]);
  const rows: FormulaRow[] = [
    { op: "divide", left: { kind: "band", band: "" }, right: { kind: "number", text: "x" } },
  ];
  expect(formulaIssues(rows, ["b8a"])).toEqual([
    "line 1: pick the left value",
    "line 1: the right value must be a number",
  ]);
  // A band unticked on the Load card after being used here flags (B7:
  // the vocabulary is the loaded bands, always).
  expect(formulaIssues(NDVI_ROWS, ["b8a"])).toContain("line 1: b04 is not loaded any more");
  expect(formulaIssues(NDVI_ROWS, ["b8a", "b04"])).toEqual([]);
});

test("the formula narrates as plain math", () => {
  expect(formulaPhrase(NDVI_ROWS, ["b8a", "b04"])).toBe("(b8a − b04) ÷ (b8a + b04)");
  expect(formulaPhrase(NDVI_ROWS, ["b8a"])).toBe(""); // incomplete: no phrase
});

test("B2/B3: the formula lowers to the reducer child graph — arithmetic and array_element live there and only there", () => {
  expect(buildReducerGraph(NDVI_ROWS)).toEqual({
    b1: {
      process_id: "array_element",
      arguments: { data: { from_parameter: "data" }, label: "b8a" },
    },
    b2: {
      process_id: "array_element",
      arguments: { data: { from_parameter: "data" }, label: "b04" },
    },
    r1: {
      process_id: "subtract",
      arguments: { x: { from_node: "b1" }, y: { from_node: "b2" } },
    },
    r2: { process_id: "add", arguments: { x: { from_node: "b1" }, y: { from_node: "b2" } } },
    r3: {
      process_id: "divide",
      arguments: { x: { from_node: "r1" }, y: { from_node: "r2" } },
      result: true,
    },
  });
  // Number operands become JSON numbers.
  const scaled = buildReducerGraph([
    { op: "multiply", left: { kind: "band", band: "b02" }, right: { kind: "number", text: "2" } },
  ]);
  expect(scaled["r1"]).toEqual({
    process_id: "multiply",
    arguments: { x: { from_node: "b1" }, y: 2 },
    result: true,
  });
});

test("band heuristics and server-error location carry over from #148", () => {
  expect(pickBand(["b02", "b04", "b8a"], [/nir/i, /8a$/i], "nir")).toBe("b8a");
  expect(pickBand(["x"], [/nir/i], "nir")).toBe("nir");
  expect(
    locateServerError("node `s3` (linear_scale_range): invalid argument `outputMin`: nope"),
  ).toEqual({ node: "s3", argument: "outputMin" });
  expect(locateServerError("no result node")).toEqual({});
});
