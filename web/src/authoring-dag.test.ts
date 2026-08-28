// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The DAG model pinned against the compiler (docs/design/authoring-dag.md
 * §6, B1–B15): what is unconstructible stays unconstructible, what is
 * explained is explained in words, and a linear graph lowers to exactly
 * the chain's output.
 */

import { expect, test } from "vitest";
import {
  apply,
  branchableInto,
  branchWindows,
  changeTemplate,
  type Dag,
  defaultParams,
  edgeAllowed,
  halves,
  insertableOn,
  locateDagError,
  lower,
  ndviTemplate,
  OUT,
  orphans,
  resultIssue,
  splice,
  typer,
  wellTyped,
} from "./authoring-dag";

const SERVED = new Set([
  "load_collection",
  "filter_temporal",
  "ndvi",
  "reduce_dimension",
  "linear_scale_range",
  "merge_cubes",
  "save_result",
  "add",
  "subtract",
  "multiply",
  "divide",
  "array_element",
]);

const ndvi = (): Dag => ndviTemplate("hls-s30", "b8a", "b04");
const change = (): Dag =>
  changeTemplate("hls-s30", "b8a", "b04", {
    before: ["2024-07-01T00:00:00Z", "2024-08-01T00:00:00Z"],
    after: ["2024-08-01T00:00:00Z", "2024-09-01T00:00:00Z"],
  });
const edge = (from: string, to: string, port: string) => ({
  from: { node: from, port: OUT },
  to: { node: to, port },
});

// --- Typing: the compiler's rules, one per process -------------------------

test("apply mirrors the compiler: reduce needs a loaded unscaled cube, merge needs two gray ones", () => {
  const multi = { kind: "multi", scaled: false } as const;
  const gray = { kind: "gray", scaled: false } as const;
  expect(apply("load_collection", [])).toEqual(multi);
  expect(apply("ndvi", [multi])).toEqual(gray);
  expect(apply("ndvi", [gray])).toBeNull(); // B4's cousin: nothing reduces a gray cube
  expect(apply("ndvi", [{ kind: "multi", scaled: true }])).toBeNull(); // B4: scale before reduce
  expect(apply("linear_scale_range", [gray])).toEqual({ kind: "gray", scaled: true });
  expect(apply("linear_scale_range", [{ kind: "gray", scaled: true }])).toBeNull();
  expect(apply("merge_cubes", [gray, gray])).toEqual(gray);
  expect(apply("merge_cubes", [multi, gray])).toBeNull();
  expect(apply("merge_cubes", [gray, { kind: "gray", scaled: true }])).toBeNull();
  expect(apply("merge_cubes", [gray])).toBeNull(); // one input unfed
  expect(apply("filter_temporal", [multi])).toEqual(multi);
  expect(apply("save_result", [gray])).toEqual(gray);
});

test("the templates type end to end and have no orphans", () => {
  for (const dag of [ndvi(), change()]) {
    expect(wellTyped(dag)).toBe(true);
    expect(orphans(dag)).toEqual([]);
    expect(resultIssue(dag)).toBeUndefined();
  }
  const typeOf = typer(change());
  expect(typeOf("s7")).toEqual({ kind: "gray", scaled: false });
  expect(typeOf("s8")).toEqual({ kind: "gray", scaled: true });
});

// --- Edges: refused in words (B12, B13), never silently ------------------

test("B12: a loaded cube cannot feed a gray port, and the refusal names the mismatch", () => {
  const dag = change();
  // s1 is a load head (multi); s7's cube2 is already fed by s3 — free it first.
  dag.edges = dag.edges.filter((e) => !(e.to.node === "s7" && e.to.port === "cube2"));
  const verdict = edgeAllowed(dag, edge("s1", "s7", "cube2"));
  expect(verdict.ok).toBe(false);
  if (!verdict.ok) {
    expect(verdict.reason).toContain("merge_cubes needs two gray, unscaled cubes");
    expect(verdict.reason).toContain("a loaded cube");
  }
  // The gray branch it had is accepted back.
  expect(edgeAllowed(dag, edge("s3", "s7", "cube2"))).toEqual({ ok: true });
});

test("B13: two collections into one join are refused before the server sees them", () => {
  const dag = change();
  const other = dag.nodes.find((n) => n.id === "s4");
  if (other) {
    other.params = { ...other.params, id: "sentinel-2" };
  }
  dag.edges = dag.edges.filter((e) => !(e.to.node === "s7" && e.to.port === "cube1"));
  const verdict = edgeAllowed(dag, edge("s6", "s7", "cube1"));
  expect(verdict).toEqual({ ok: false, reason: "both branches must load the same collection" });
});

test("an input already connected, a loop, and a missing port are each refused in words", () => {
  const dag = ndvi();
  expect(edgeAllowed(dag, edge("s1", "s2", "data"))).toMatchObject({ ok: false });
  const fed = edgeAllowed(dag, edge("s3", "s2", "data"));
  expect(fed.ok).toBe(false);
  if (!fed.ok) {
    expect(fed.reason).toContain("already connected");
  }
  const loop = edgeAllowed(
    { nodes: dag.nodes, edges: dag.edges.filter((e) => e.to.node !== "s2") },
    edge("s3", "s2", "data"),
  );
  expect(loop.ok).toBe(false);
  if (!loop.ok) {
    expect(loop.reason).toContain("loop");
  }
  expect(edgeAllowed(dag, edge("s1", "s3", "data"))).toMatchObject({
    ok: false,
    reason: "linear_scale_range has no input named data",
  });
  expect(edgeAllowed(dag, edge("s4", "s2", "data"))).toMatchObject({ ok: false });
});

// --- The palette: only what would compile (B2/B3/B4) ------------------------

test("B2/B3: arithmetic and array_element are never offered at top level", () => {
  const dag = ndvi();
  for (const process of insertableOn(dag, edge("s1", "s2", "data"), SERVED, "s5")) {
    expect(["add", "subtract", "multiply", "divide", "array_element"]).not.toContain(process);
  }
  expect(branchableInto(dag, "s2", SERVED)).not.toContain("subtract");
});

test("B4: a scale is never offered before the reduce, and nothing reduces or scales after it", () => {
  const dag = ndvi();
  // On load→ndvi: a scale would hand ndvi a scaled cube (B4), a UDF
  // stage a UDF result (one run_udf per graph, feeding only the scale
  // and the output) — neither is offered.
  const withUdf = new Set([...SERVED, "run_udf"]);
  expect(insertableOn(dag, edge("s1", "s2", "data"), withUdf, "s5")).toEqual(["filter_temporal"]);
  // Where the loaded cube goes straight to the output, the UDF stage is.
  const direct: Dag = {
    nodes: [
      { id: "s1", process: "load_collection", params: { id: "hls-s30", bands: ["b8a", "b04"] } },
      { id: "s2", process: "save_result", params: { format: "png" } },
    ],
    edges: [edge("s1", "s2", "data")],
  };
  expect(insertableOn(direct, edge("s1", "s2", "data"), withUdf, "s3")).toEqual([
    "filter_temporal",
    "ndvi",
    "reduce_dimension",
    "run_udf",
    "linear_scale_range",
  ]);
  // Between ndvi and the existing scale: a second scale would leave the
  // existing one nothing to scale.
  expect(insertableOn(dag, edge("s2", "s3", "x"), SERVED, "s5")).toEqual(["filter_temporal"]);
  // After the scale, no step may reduce or scale again.
  expect(insertableOn(dag, edge("s3", "s4", "data"), SERVED, "s5")).toEqual(["filter_temporal"]);
});

test("a gray output can branch into a join; a loaded cube cannot", () => {
  const dag = ndvi();
  expect(branchableInto(dag, "s2", SERVED)).toContain("merge_cubes");
  expect(branchableInto(dag, "s1", SERVED)).not.toContain("merge_cubes");
});

// --- B1, B10, B14: permanence, orphans, resolver ----------------------------

test("B10: a step with no path to the output is an orphan — explained and gated, never silently dropped", () => {
  const dag = ndvi();
  dag.nodes.push({ id: "s5", process: "ndvi", params: { nir: "b8a", red: "b04" } });
  dag.edges.push(edge("s1", "s5", "data"));
  expect(orphans(dag)).toEqual(["s5"]);
  expect(wellTyped(dag)).toBe(true); // it types; it just goes nowhere
  expect(Object.keys(lower(dag))).toContain("s5"); // lowering is honest about it too
});

test("B14: a join is created with its resolver; B9: the output is PNG", () => {
  expect(defaultParams("merge_cubes")).toEqual({
    overlap_resolver: {
      process_graph: {
        r1: {
          process_id: "subtract",
          arguments: { x: { from_parameter: "x" }, y: { from_parameter: "y" } },
          result: true,
        },
      },
    },
  });
  expect(defaultParams("save_result")).toEqual({ format: "png" });
  const lowered = lower(change()) as Record<string, { arguments: Record<string, unknown> }>;
  expect(lowered["s9"]?.arguments["format"]).toBe("png");
});

// --- B5/B6: the output's shape, in words -----------------------------------

test("B5/B6: a two-band loaded cube at the output, or a colormap on a composite, is explained", () => {
  const dag: Dag = {
    nodes: [
      { id: "s1", process: "load_collection", params: { id: "hls-s30", bands: ["b8a", "b04"] } },
      { id: "s2", process: "save_result", params: { format: "png" } },
    ],
    edges: [edge("s1", "s2", "data")],
  };
  expect(resultIssue(dag)).toContain("exactly 3 bands");
  const rgb: Dag = {
    nodes: [
      {
        id: "s1",
        process: "load_collection",
        params: { id: "hls-s30", bands: ["b04", "b03", "b02"] },
      },
      {
        id: "s2",
        process: "save_result",
        params: { format: "png", options: { colormap: "viridis" } },
      },
    ],
    edges: [edge("s1", "s2", "data")],
  };
  expect(resultIssue(rgb)).toBe("a colormap applies to a gray result only");
});

// --- Lowering: the chain's output, node for node ----------------------------

test("the linear NDVI DAG lowers to exactly the chain's buildGraph output", () => {
  expect(lower(ndvi())).toEqual({
    s1: {
      process_id: "load_collection",
      arguments: { id: "hls-s30", bands: ["b8a", "b04"] },
    },
    s2: {
      process_id: "ndvi",
      arguments: { nir: "b8a", red: "b04", data: { from_node: "s1" } },
    },
    s3: {
      process_id: "linear_scale_range",
      arguments: {
        inputMin: -1,
        inputMax: 1,
        outputMin: 0,
        outputMax: 255,
        x: { from_node: "s2" },
      },
    },
    s4: {
      process_id: "save_result",
      arguments: { format: "png", data: { from_node: "s3" } },
      result: true,
    },
  });
});

test("the change template lowers to the compiler's change-detection shape with a prefixed resolver", () => {
  const lowered = lower(change()) as Record<string, { arguments: Record<string, unknown> }>;
  expect(Object.keys(lowered)).toEqual(["s1", "s4", "s2", "s5", "s3", "s6", "s7", "s8", "s9"]);
  expect(lowered["s7"]?.arguments).toEqual({
    overlap_resolver: {
      process_graph: {
        "s7.r1": {
          process_id: "subtract",
          arguments: { x: { from_parameter: "x" }, y: { from_parameter: "y" } },
          result: true,
        },
      },
    },
    cube1: { from_node: "s6" },
    cube2: { from_node: "s3" },
  });
  expect(lowered["s2"]?.arguments).toEqual({
    extent: ["2024-07-01T00:00:00Z", "2024-08-01T00:00:00Z"],
    data: { from_node: "s1" },
  });
});

test("a reducer child graph keeps its from_node wiring under the owner's prefix", () => {
  const dag: Dag = {
    nodes: [
      { id: "s1", process: "load_collection", params: { id: "hls-s30", bands: ["b8a", "b04"] } },
      {
        id: "s2",
        process: "reduce_dimension",
        params: {
          dimension: "bands",
          reducer: {
            process_graph: {
              b1: {
                process_id: "array_element",
                arguments: { data: { from_parameter: "data" }, label: "b8a" },
              },
              r1: {
                process_id: "subtract",
                arguments: { x: { from_node: "b1" }, y: 1 },
                result: true,
              },
            },
          },
        },
      },
      { id: "s3", process: "save_result", params: { format: "png" } },
    ],
    edges: [edge("s1", "s2", "data"), edge("s2", "s3", "data")],
  };
  const lowered = lower(dag) as Record<string, { arguments: Record<string, unknown> }>;
  expect(lowered["s2"]?.arguments["reducer"]).toEqual({
    process_graph: {
      "s2.b1": {
        process_id: "array_element",
        arguments: { data: { from_parameter: "data" }, label: "b8a" },
      },
      "s2.r1": {
        process_id: "subtract",
        arguments: { x: { from_node: "s2.b1" }, y: 1 },
        result: true,
      },
    },
  });
});

test("locateDagError maps prefixed child keys back to the owning node", () => {
  expect(locateDagError("node `s7.r1` (subtract): invalid argument `y`: …")).toEqual({
    node: "s7",
    child: "r1",
    argument: "y",
  });
  expect(locateDagError("node `s3` (linear_scale_range): invalid argument `inputMin`: …")).toEqual({
    node: "s3",
    argument: "inputMin",
  });
  expect(locateDagError("no result node")).toEqual({});
});

// --- Splicing keeps ids persistent ------------------------------------------

test("splice inserts on an edge without renumbering anything", () => {
  const dag = ndvi();
  const spliced = splice(dag, edge("s1", "s2", "data"), {
    id: "s5",
    process: "filter_temporal",
    params: { extent: ["2024-06-01T00:00:00Z", null] },
  });
  expect(spliced).toBeDefined();
  if (spliced) {
    expect(spliced.nodes.map((n) => n.id)).toEqual(["s1", "s2", "s3", "s4", "s5"]);
    expect(wellTyped(spliced)).toBe(true);
    const lowered = lower(spliced) as Record<string, { arguments: Record<string, unknown> }>;
    expect(lowered["s2"]?.arguments["data"]).toEqual({ from_node: "s5" });
    expect(lowered["s5"]?.arguments["data"]).toEqual({ from_node: "s1" });
  }
  expect(
    splice(dag, edge("s1", "s3", "x"), { id: "s9", process: "ndvi", params: {} }),
  ).toBeUndefined();
});

// --- B15: the frame windows, per branch -----------------------------------

test("B15: the change template carries one window per branch, and halves() splits the extent", () => {
  expect(halves(["2024-06-01T00:00:00Z", "2024-10-01T00:00:00Z"])).toEqual({
    before: ["2024-06-01T00:00:00Z", "2024-08-01T00:00:00.000Z"],
    after: ["2024-08-01T00:00:00.000Z", "2024-10-01T00:00:00Z"],
  });
  expect(branchWindows(change())).toEqual([
    { node: "s1", window: ["2024-07-01T00:00:00Z", "2024-08-01T00:00:00Z"] },
    { node: "s4", window: ["2024-08-01T00:00:00Z", "2024-09-01T00:00:00Z"] },
  ]);
  expect(branchWindows(ndvi())).toEqual([{ node: "s1", window: [null, null] }]);
});
