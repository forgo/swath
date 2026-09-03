// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The authoring DAG's pure graph model (issue #298; docs/design/
 * authoring-dag.md §4, ADR 0022). The chain's always-valid invariant —
 * "the pipeline is never in an invalid state" — survives the shape
 * change as a typed-port graph: every edge is checked against the
 * compiler's typing before it exists, the output node is permanent, and
 * what the compiler would refuse is either unconstructible here or
 * named in plain words (the design note's B1–B15 table, pinned by the
 * vitest suite on this module).
 *
 * Pure data-in/data-out — no DOM; the canvas primitives only draw it.
 * The port type is the chain's `Stage` (authoring-model.ts): `multi`
 * (a loaded cube), `gray` (one value per pixel), `udf`, each scaled or
 * not — exactly the compiler's `CubeKind` discipline, reused rather than
 * restated. Child graphs (a reducer, a resolver) ride on the node as
 * lowered openEO literals; `lower()` prefixes their keys with the
 * owning node so a server diagnostic maps back to one canvas node.
 */

import { FORMULA_OPS, LOAD_STAGE, type Stage, transition } from "./authoring-model";

/** A port's type: the stage flowing through it. */
export type PortType = Stage;

/** One canvas node: a served process, its non-cube arguments as openEO
 * literals, and a persistent id (never renumbered — the graph key it
 * lowers to, so a diagnostic from the server names it). */
export interface DagNode {
  id: string;
  process: string;
  params: Record<string, unknown>;
}

/** A port reference: a node and one of its ports by name. */
export interface PortRef {
  node: string;
  port: string;
}

/** A cube flowing from one node's output to another's input. */
export interface Edge {
  from: PortRef;
  to: PortRef;
}

/** The whole graph. Node order is presentation order; `lower()` sorts
 * topologically. */
export interface Dag {
  nodes: DagNode[];
  edges: Edge[];
}

/** The cube ports of every process the DAG admits at top level: input
 * ports are the argument names the compiler reads a cube through
 * (`CUBE_PARAM` generalised to two inputs), the output port is `out`.
 * Arithmetic and `array_element` are absent on purpose — they live
 * inside a reducer or a resolver only (B2/B3). */
export const CUBE_PORTS: Readonly<Record<string, { inputs: readonly string[]; output: boolean }>> =
  {
    load_collection: { inputs: [], output: true },
    filter_temporal: { inputs: ["data"], output: true },
    ndvi: { inputs: ["data"], output: true },
    reduce_dimension: { inputs: ["data"], output: true },
    run_udf: { inputs: ["data"], output: true },
    linear_scale_range: { inputs: ["x"], output: true },
    merge_cubes: { inputs: ["cube1", "cube2"], output: true },
    save_result: { inputs: ["data"], output: false },
  };

/** The output port's name. */
export const OUT = "out";

/** The processes a user may add between the load heads and the output —
 * the palette. `load_collection` is a head (branchable, never inserted
 * on an edge); `save_result` is the permanent tail (B1). */
export const DAG_PROCESSES = [
  "filter_temporal",
  "ndvi",
  "reduce_dimension",
  "run_udf",
  "linear_scale_range",
  "merge_cubes",
] as const;

/** The stage a process produces from its typed inputs (in port order),
 * or `null` where the compiler would refuse — one rule per process,
 * mirroring `crates/swath-render/src/process.rs`:
 *
 * - `load_collection` produces the loaded cube;
 * - `filter_temporal` passes its cube through (frame selection changes
 *   no pixels);
 * - `ndvi` / `reduce_dimension` / `run_udf` / `linear_scale_range` are
 *   the chain's `transition` (multi + unscaled → gray/udf; unscaled →
 *   scaled);
 * - `merge_cubes` (ADR 0022) needs two gray, unscaled cubes and produces
 *   one;
 * - `save_result` accepts what it is given — its shape question (B5) is
 *   answered by `resultIssue`, in words, not by a type.
 */
export function apply(process: string, inputs: readonly (Stage | null)[]): Stage | null {
  if (inputs.some((input) => input === null)) {
    return null;
  }
  const typed = inputs as readonly Stage[];
  const [first] = typed;
  switch (process) {
    case "load_collection":
      return LOAD_STAGE;
    case "filter_temporal":
      return first ?? null;
    case "merge_cubes": {
      const [a, b] = typed;
      if (a === undefined || b === undefined) {
        return null;
      }
      if (a.kind !== "gray" || b.kind !== "gray" || a.scaled || b.scaled) {
        return null;
      }
      return { kind: "gray", scaled: false };
    }
    case "save_result":
      return first ?? null;
    default:
      return first === undefined ? null : transition(first, process);
  }
}

/** The edge feeding `port` of `node`, if any. */
export function feeding(dag: Dag, node: string, port: string): Edge | undefined {
  return dag.edges.find((edge) => edge.to.node === node && edge.to.port === port);
}

/** The node by id. */
export function nodeOf(dag: Dag, id: string): DagNode | undefined {
  return dag.nodes.find((node) => node.id === id);
}

/**
 * A memoised typer over one graph: `typeOf(id)` is the stage at the
 * node's output, `null` when the node cannot type — an input port left
 * unfed, a rule refusing, or a cycle through it. Memoised per graph so
 * `edgeAllowed` can re-type the whole graph on a candidate cheaply.
 */
export function typer(dag: Dag): (id: string) => Stage | null {
  const memo = new Map<string, Stage | null>();
  const visiting = new Set<string>();
  const typeOf = (id: string): Stage | null => {
    const known = memo.get(id);
    if (known !== undefined) {
      return known;
    }
    if (visiting.has(id)) {
      return null; // a cycle: openEO graphs are DAGs
    }
    const node = nodeOf(dag, id);
    const ports = node === undefined ? undefined : CUBE_PORTS[node.process];
    if (node === undefined || ports === undefined) {
      memo.set(id, null);
      return null;
    }
    visiting.add(id);
    const inputs = ports.inputs.map((port) => {
      const edge = feeding(dag, id, port);
      return edge === undefined ? null : typeOf(edge.from.node);
    });
    visiting.delete(id);
    const stage = apply(node.process, inputs);
    memo.set(id, stage);
    return stage;
  };
  return typeOf;
}

/** Whether the whole graph types: every node has every input fed and
 * every rule accepts. */
export function wellTyped(dag: Dag): boolean {
  const typeOf = typer(dag);
  return dag.nodes.every((node) => typeOf(node.id) !== null);
}

/** The words for a stage, for reasons. */
function describe(stage: Stage | null): string {
  if (stage === null) {
    return "nothing";
  }
  const kind = { multi: "a loaded cube", gray: "a gray cube", udf: "a UDF result" }[stage.kind];
  return stage.scaled ? `${kind} (already scaled)` : kind;
}

/** The collection a node's cube traces back to (its `load_collection`'s
 * `id`), following the first cube input; `undefined` when unfed. Every
 * cube of a well-formed graph has exactly one origin unless a join sits
 * upstream — a join's branches are checked to agree (B13). */
export function collectionOf(dag: Dag, id: string): string | undefined {
  const seen = new Set<string>();
  let current: string | undefined = id;
  while (current !== undefined && !seen.has(current)) {
    seen.add(current);
    const node = nodeOf(dag, current);
    if (node === undefined) {
      return undefined;
    }
    if (node.process === "load_collection") {
      const collection = node.params["id"];
      return typeof collection === "string" ? collection : undefined;
    }
    const [port] = CUBE_PORTS[node.process]?.inputs ?? [];
    current = port === undefined ? undefined : feeding(dag, current, port)?.from.node;
  }
  return undefined;
}

/** The verdict on a candidate edge. */
export type EdgeVerdict = { ok: true } | { ok: false; reason: string };

/**
 * Whether `edge` may be added: both ports exist, the input is not
 * already fed, the graph stays acyclic, every node downstream still
 * types (B12 — the refusal names the mismatch), and a join's two
 * branches load the same collection (B13). Model B's rule: the editor
 * offers only what would compile.
 */
export function edgeAllowed(dag: Dag, edge: Edge): EdgeVerdict {
  const from = nodeOf(dag, edge.from.node);
  const to = nodeOf(dag, edge.to.node);
  if (from === undefined || to === undefined) {
    return { ok: false, reason: "one end of the connection is not on the canvas" };
  }
  if (edge.from.port !== OUT || CUBE_PORTS[from.process]?.output !== true) {
    return { ok: false, reason: `${from.process} has no output to connect from` };
  }
  const inputs = CUBE_PORTS[to.process]?.inputs ?? [];
  if (!inputs.includes(edge.to.port)) {
    return { ok: false, reason: `${to.process} has no input named ${edge.to.port}` };
  }
  if (from.id === to.id) {
    return { ok: false, reason: "a step cannot feed itself" };
  }
  if (feeding(dag, to.id, edge.to.port) !== undefined) {
    return { ok: false, reason: `${to.process}'s ${edge.to.port} input is already connected` };
  }
  if (reaches(dag, to.id, from.id)) {
    return { ok: false, reason: "that connection would loop the pipeline back on itself" };
  }
  const candidate: Dag = { nodes: dag.nodes, edges: [...dag.edges, edge] };
  const typeOf = typer(candidate);
  const given = typeOf(from.id);
  if (given === null) {
    return { ok: false, reason: `${from.process} is not complete yet` };
  }
  if (to.process === "merge_cubes") {
    const other = inputs.find((port) => port !== edge.to.port);
    const otherEdge = other === undefined ? undefined : feeding(dag, to.id, other);
    if (otherEdge !== undefined) {
      const a = collectionOf(candidate, edge.from.node);
      const b = collectionOf(candidate, otherEdge.from.node);
      if (a !== undefined && b !== undefined && a !== b) {
        return { ok: false, reason: "both branches must load the same collection" };
      }
    }
  }
  // Re-type the target and everything downstream of it: a null that
  // was not null before the edge is this edge's doing.
  for (const node of candidate.nodes) {
    if (node.id !== to.id && !reaches(candidate, to.id, node.id)) {
      continue;
    }
    if (typeOf(node.id) !== null) {
      continue;
    }
    if (allInputsFed(candidate, node.id)) {
      const expected = expectation(node.process);
      return {
        ok: false,
        reason: `${node.process} needs ${expected}, but this would give it ${describe(given)}`,
      };
    }
  }
  return { ok: true };
}

/** Whether every cube input of `id` is connected. */
export function allInputsFed(dag: Dag, id: string): boolean {
  const node = nodeOf(dag, id);
  const inputs = node === undefined ? [] : (CUBE_PORTS[node.process]?.inputs ?? []);
  return inputs.every((port) => feeding(dag, id, port) !== undefined);
}

/** What a process needs on its inputs, in words. */
function expectation(process: string): string {
  switch (process) {
    case "ndvi":
    case "reduce_dimension":
    case "run_udf":
      return "a loaded, unscaled cube";
    case "linear_scale_range":
      return "an unscaled cube";
    case "merge_cubes":
      return "two gray, unscaled cubes";
    default:
      return "a cube";
  }
}

/** Whether `target` is reachable from `source` along edges. */
export function reaches(dag: Dag, source: string, target: string): boolean {
  if (source === target) {
    return true;
  }
  const seen = new Set<string>();
  const stack = [source];
  while (stack.length > 0) {
    const current = stack.pop() as string;
    for (const edge of dag.edges) {
      if (edge.from.node !== current || seen.has(edge.to.node)) {
        continue;
      }
      if (edge.to.node === target) {
        return true;
      }
      seen.add(edge.to.node);
      stack.push(edge.to.node);
    }
  }
  return false;
}

/** The permanent output node (B1). */
export function outputNode(dag: Dag): DagNode | undefined {
  return dag.nodes.find((node) => node.process === "save_result");
}

/** Nodes with no path to the output — dead steps (B10): explained on the
 * canvas, omitted from the narrative, and a publish gate. The output
 * itself is never an orphan. */
export function orphans(dag: Dag): string[] {
  const out = outputNode(dag);
  if (out === undefined) {
    return dag.nodes.map((node) => node.id);
  }
  return dag.nodes
    .filter((node) => node.id !== out.id && !reaches(dag, node.id, out.id))
    .map((node) => node.id);
}

/** Which palette processes could be spliced onto `edge` (between its
 * two ends) with the graph still typing — the chain's `insertableAt`,
 * generalised to an edge. A join has two inputs and is never spliced;
 * it is branched into (`branchableInto`). */
export function insertableOn(
  dag: Dag,
  edge: Edge,
  served: ReadonlySet<string>,
  nextId: string,
): string[] {
  return DAG_PROCESSES.filter((process) => {
    if (!served.has(process) || process === "merge_cubes") {
      return false;
    }
    if (
      process === "reduce_dimension" &&
      (!served.has("array_element") || !FORMULA_OPS.some((op) => served.has(op)))
    ) {
      return false; // the formula builder needs its child-graph vocabulary served
    }
    const spliced = splice(dag, edge, { id: nextId, process, params: defaultParams(process) });
    return spliced !== undefined && wellTyped(spliced);
  });
}

/** `dag` with `node` spliced onto `edge`; `undefined` if `edge` is not
 * in the graph. */
export function splice(dag: Dag, edge: Edge, node: DagNode): Dag | undefined {
  const index = dag.edges.findIndex((e) => sameEdge(e, edge));
  if (index < 0) {
    return undefined;
  }
  const [port] = CUBE_PORTS[node.process]?.inputs ?? [];
  if (port === undefined) {
    return undefined;
  }
  const edges = [...dag.edges];
  edges.splice(
    index,
    1,
    { from: edge.from, to: { node: node.id, port } },
    { from: { node: node.id, port: OUT }, to: edge.to },
  );
  return { nodes: [...dag.nodes, node], edges };
}

/** Which palette processes a NEW branch from `from`'s output could feed
 * — a second consumer of a cube. Today that is the join (a gray cube
 * into a `merge_cubes` created with its resolver, B14) and a second
 * chain head's worth of single-input steps. */
export function branchableInto(dag: Dag, from: string, served: ReadonlySet<string>): string[] {
  const stage = typer(dag)(from);
  if (stage === null) {
    return [];
  }
  return DAG_PROCESSES.filter((process) => {
    if (!served.has(process)) {
      return false;
    }
    const [port] = CUBE_PORTS[process]?.inputs ?? [];
    return port !== undefined && apply(process, padInputs(process, stage)) !== null;
  });
}

/** `stage` on the first input, the other inputs assumed the same. */
function padInputs(process: string, stage: Stage): Stage[] {
  return (CUBE_PORTS[process]?.inputs ?? []).map(() => stage);
}

function sameEdge(a: Edge, b: Edge): boolean {
  return (
    a.from.node === b.from.node &&
    a.from.port === b.from.port &&
    a.to.node === b.to.node &&
    a.to.port === b.to.port
  );
}

/** The resolver child graph over the pair: `x` from `cube1`, `y` from
 * `cube2` — openEO's own binding, the shape the compiler evaluates. */
export function resolverGraph(op: "add" | "subtract" | "multiply" | "divide"): {
  process_graph: Record<string, Record<string, unknown>>;
} {
  return {
    process_graph: {
      r1: {
        process_id: op,
        arguments: { x: { from_parameter: "x" }, y: { from_parameter: "y" } },
        result: true,
      },
    },
  };
}

/** A new node's arguments: a join is created WITH its resolver (B14,
 * the spec's default would fail every pixel); the output is PNG (B9);
 * `reduce_dimension` over bands with an empty formula the builder
 * fills; the rest empty. */
export function defaultParams(process: string): Record<string, unknown> {
  switch (process) {
    case "merge_cubes":
      return { overlap_resolver: resolverGraph("subtract") };
    case "save_result":
      return { format: "png" };
    case "reduce_dimension":
      return { dimension: "bands" };
    default:
      return {};
  }
}

/** The output's shape question (B5), in words: a loaded cube reaches
 * the output with a band count that is not three, or a colormap rides
 * a composite / UDF result (B6). `undefined` when the output is fine. */
export function resultIssue(dag: Dag): string | undefined {
  const out = outputNode(dag);
  if (out === undefined) {
    return "the pipeline has no output";
  }
  const edge = feeding(dag, out.id, "data");
  if (edge === undefined) {
    return "nothing reaches the output yet";
  }
  const stage = typer(dag)(edge.from.node);
  if (stage === null) {
    return "the step feeding the output is not complete";
  }
  const options = out.params["options"];
  const colormap =
    typeof options === "object" && options !== null && "colormap" in options
      ? (options as { colormap?: unknown }).colormap
      : undefined;
  if (stage.kind === "multi") {
    const load = nodeOf(dag, loadHead(dag, edge.from.node) ?? "");
    const bands = Array.isArray(load?.params["bands"]) ? load.params["bands"].length : undefined;
    if (bands !== undefined && bands !== 3) {
      return `a composite needs exactly 3 bands, this loads ${bands} — reduce to one value per pixel or pick three`;
    }
  }
  if (colormap !== undefined && stage.kind !== "gray") {
    return "a colormap applies to a gray result only";
  }
  return undefined;
}

/** The `load_collection` at the head of `id`'s first-input chain. */
export function loadHead(dag: Dag, id: string): string | undefined {
  const seen = new Set<string>();
  let current: string | undefined = id;
  while (current !== undefined && !seen.has(current)) {
    seen.add(current);
    const node = nodeOf(dag, current);
    if (node?.process === "load_collection") {
      return current;
    }
    const [port] = node === undefined ? [] : (CUBE_PORTS[node.process]?.inputs ?? []);
    current = port === undefined ? undefined : feeding(dag, current, port)?.from.node;
  }
  return undefined;
}

/** Nodes in topological order (Kahn), ties by presentation order. */
export function topological(dag: Dag): DagNode[] {
  const remaining = new Map(dag.nodes.map((node) => [node.id, node]));
  const order: DagNode[] = [];
  while (remaining.size > 0) {
    const ready = [...remaining.values()].filter((node) =>
      dag.edges.every((edge) => edge.to.node !== node.id || !remaining.has(edge.from.node)),
    );
    if (ready.length === 0) {
      break; // a cycle: the rest never lowers (edgeAllowed forbids it)
    }
    for (const node of ready) {
      order.push(node);
      remaining.delete(node.id);
    }
  }
  return order;
}

/** The child-key separator: `s7.r1` is child `r1` of node `s7`. */
export const CHILD_SEP = ".";

/**
 * Lowers the graph to the openEO `process_graph` the compiler evaluates:
 * node ids become keys, edges become `from_node` references, params ride
 * verbatim, the output node is the result. A linear chain lowers to
 * exactly what the chain's `buildGraph()` produced (the byte-identical
 * NDVI check is the safety net). Child graphs (`reducer`,
 * `overlap_resolver`) keep their `from_node`s but their keys are
 * prefixed with the owning node's id, so `locateDagError` can map a
 * server diagnostic inside a child back to the canvas node.
 */
/** The graph truncated at `node`: that node, everything feeding it, and a
 * `save_result` in place of whatever came after.
 *
 * This is what "the preview follows the selected step" means (#401) — the
 * preview answers *what does this step produce*, which is a different graph
 * from the one publish sends. Building it here rather than in the panel
 * keeps the semantics testable without a DOM, and keeps one definition of
 * "everything feeding a node".
 *
 * `save` supplies the output node's id and params (the panel's own Output
 * card, so the format and any colormap are the ones the author chose).
 * Returns `undefined` when `node` is not in the graph, or when it IS the
 * output — there is nothing to truncate then, and the caller should preview
 * the whole graph.
 */
export function truncatedAt(dag: Dag, node: string, save: DagNode): Dag | undefined {
  const target = nodeOf(dag, node);
  if (target === undefined || target.process === "save_result") {
    return undefined;
  }
  // Everything that reaches the target, the target included. `reaches` is
  // the same relation the orphan check uses, so "feeds it" means one thing.
  const kept = dag.nodes.filter((candidate) => reaches(dag, candidate.id, node));
  const ids = new Set(kept.map((n) => n.id));
  const edges = dag.edges.filter((edge) => ids.has(edge.from.node) && ids.has(edge.to.node));
  const output: DagNode = { id: save.id, process: "save_result", params: { ...save.params } };
  const port = CUBE_PORTS["save_result"]?.inputs[0] ?? "data";
  return {
    nodes: [...kept, output],
    edges: [...edges, { from: { node, port: "" }, to: { node: save.id, port } }],
  };
}

export function lower(dag: Dag): Record<string, unknown> {
  const out = outputNode(dag);
  const graph: Record<string, unknown> = {};
  for (const node of topological(dag)) {
    const args: Record<string, unknown> = {};
    for (const [name, value] of Object.entries(node.params)) {
      args[name] = isChildGraph(value) ? prefixChild(node.id, value) : value;
    }
    for (const port of CUBE_PORTS[node.process]?.inputs ?? []) {
      const edge = feeding(dag, node.id, port);
      if (edge !== undefined) {
        args[port] = { from_node: edge.from.node };
      }
    }
    const lowered: Record<string, unknown> = { process_id: node.process, arguments: args };
    if (out !== undefined && node.id === out.id) {
      lowered["result"] = true;
    }
    graph[node.id] = lowered;
  }
  return graph;
}

function isChildGraph(value: unknown): value is { process_graph: Record<string, unknown> } {
  return (
    typeof value === "object" &&
    value !== null &&
    "process_graph" in value &&
    typeof (value as { process_graph: unknown }).process_graph === "object"
  );
}

function prefixChild(
  owner: string,
  child: { process_graph: Record<string, unknown> },
): { process_graph: Record<string, unknown> } {
  const graph: Record<string, unknown> = {};
  for (const [key, node] of Object.entries(child.process_graph)) {
    graph[`${owner}${CHILD_SEP}${key}`] = rewriteRefs(owner, node);
  }
  return { process_graph: graph };
}

function rewriteRefs(owner: string, value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((item) => rewriteRefs(owner, item));
  }
  if (typeof value === "object" && value !== null) {
    const record = value as Record<string, unknown>;
    if (typeof record["from_node"] === "string" && Object.keys(record).length === 1) {
      return { from_node: `${owner}${CHILD_SEP}${record["from_node"]}` };
    }
    const out: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(record)) {
      out[key] = rewriteRefs(owner, item);
    }
    return out;
  }
  return value;
}

/** Where a server diagnostic points on the canvas: the owning node (a
 * prefixed child key maps to its owner, with the child kept), and the
 * argument when named — the chain's `locateServerError`, prefix-aware. */
export function locateDagError(message: string): {
  node?: string;
  child?: string;
  argument?: string;
} {
  const located: { node?: string; child?: string; argument?: string } = {};
  const node = message.match(/node `([^`]+)`/);
  if (node?.[1] !== undefined) {
    const [owner, ...rest] = node[1].split(CHILD_SEP);
    located.node = owner ?? node[1];
    if (rest.length > 0) {
      located.child = rest.join(CHILD_SEP);
    }
  }
  const argument = message.match(/(?:missing required argument|invalid argument) `([^`]+)`/);
  if (argument?.[1] !== undefined) {
    located.argument = argument[1];
  }
  return located;
}

// --- Templates: graph literals -------------------------------------------

/** The NDVI chain as a graph: load → ndvi → linear_scale_range → save,
 * ids `s1..s4` — node for node what the chain's `buildGraph()` emits. */
export function ndviTemplate(collection: string, nir: string, red: string): Dag {
  return {
    nodes: [
      { id: "s1", process: "load_collection", params: { id: collection, bands: [nir, red] } },
      { id: "s2", process: "ndvi", params: { nir, red } },
      {
        id: "s3",
        process: "linear_scale_range",
        params: { inputMin: -1, inputMax: 1, outputMin: 0, outputMax: 255 },
      },
      { id: "s4", process: "save_result", params: { format: "png" } },
    ],
    edges: [
      { from: { node: "s1", port: OUT }, to: { node: "s2", port: "data" } },
      { from: { node: "s2", port: OUT }, to: { node: "s3", port: "x" } },
      { from: { node: "s3", port: OUT }, to: { node: "s4", port: "data" } },
    ],
  };
}

/** A left-closed window `[start, end)` as the served definitions take. */
export type Window = [string, string];

/** Two windows from a collection's temporal extent, split at its middle
 * instant: `before` = the first half, `after` = the second — the
 * change template's t₁/t₂ pre-fill. */
export function halves(extent: readonly [string, string]): { before: Window; after: Window } {
  const [start, end] = extent;
  const mid = new Date((Date.parse(start) + Date.parse(end)) / 2).toISOString();
  return { before: [start, mid], after: [mid, end] };
}

/**
 * The first DAG template — change detection (ADR 0022): two frame-
 * selected NDVI branches of one collection joined by a `subtract`
 * resolver, scaled −1..1, RdYlGn. `before` and `after` are the branch
 * windows (`halves()` of the collection extent by default); each
 * branch is `load → filter_temporal → ndvi` so the frame is a step a
 * user can see and move.
 */
export function changeTemplate(
  collection: string,
  nir: string,
  red: string,
  windows: { before: Window; after: Window },
): Dag {
  const load = (id: string): DagNode => ({
    id,
    process: "load_collection",
    params: { id: collection, bands: [nir, red] },
  });
  const filter = (id: string, extent: Window): DagNode => ({
    id,
    process: "filter_temporal",
    params: { extent },
  });
  const ndvi = (id: string): DagNode => ({ id, process: "ndvi", params: { nir, red } });
  const edge = (from: string, to: string, port: string): Edge => ({
    from: { node: from, port: OUT },
    to: { node: to, port },
  });
  return {
    nodes: [
      load("s1"),
      filter("s2", windows.before),
      ndvi("s3"),
      load("s4"),
      filter("s5", windows.after),
      ndvi("s6"),
      { id: "s7", process: "merge_cubes", params: defaultParams("merge_cubes") },
      {
        id: "s8",
        process: "linear_scale_range",
        params: { inputMin: -1, inputMax: 1, outputMin: 0, outputMax: 255 },
      },
      {
        id: "s9",
        process: "save_result",
        params: { format: "png", options: { colormap: "rdylgn" } },
      },
    ],
    edges: [
      edge("s1", "s2", "data"),
      edge("s2", "s3", "data"),
      edge("s4", "s5", "data"),
      edge("s5", "s6", "data"),
      edge("s6", "s7", "cube1"), // after − before: cube1 is the later frame
      edge("s3", "s7", "cube2"),
      edge("s7", "s8", "x"),
      edge("s8", "s9", "data"),
    ],
  };
}

/** The frame windows the graph implies, one per `load_collection` head
 * (its `temporal_extent` intersected with every `filter_temporal` on
 * its branch — the compiler's `sources`); `[null, null]` when a branch
 * says nothing about time. The time slider's domain for a two-source
 * layer is their hull; a `datetime=` that leaves either branch without
 * a granule is the tile route's 404 (B15, in words). */
export function branchWindows(
  dag: Dag,
): { node: string; window: [string | null, string | null] }[] {
  return dag.nodes
    .filter((node) => node.process === "load_collection")
    .map((head) => {
      let window: [string | null, string | null] = [null, null];
      const extent = head.params["temporal_extent"];
      if (Array.isArray(extent) && extent.length === 2) {
        window = [extent[0] ?? null, extent[1] ?? null];
      }
      // Walk downstream along the chain of single-input steps.
      let current = head.id;
      const seen = new Set<string>();
      while (!seen.has(current)) {
        seen.add(current);
        const next = dag.edges.find((edge) => edge.from.node === current);
        const node = next === undefined ? undefined : nodeOf(dag, next.to.node);
        if (node === undefined) {
          break;
        }
        if (node.process === "filter_temporal") {
          const filter = node.params["extent"];
          if (Array.isArray(filter) && filter.length === 2) {
            window = intersect(window, [filter[0] ?? null, filter[1] ?? null]);
          }
        }
        if ((CUBE_PORTS[node.process]?.inputs.length ?? 0) !== 1) {
          break; // a join or the output: this branch ends here
        }
        current = node.id;
      }
      return { node: head.id, window };
    });
}

function intersect(
  a: [string | null, string | null],
  b: [string | null, string | null],
): [string | null, string | null] {
  const later = (x: string | null, y: string | null): string | null =>
    x === null ? y : y === null ? x : Date.parse(x) >= Date.parse(y) ? x : y;
  const earlier = (x: string | null, y: string | null): string | null =>
    x === null ? y : y === null ? x : Date.parse(x) <= Date.parse(y) ? x : y;
  return [later(a[0], b[0]), earlier(a[1], b[1])];
}
