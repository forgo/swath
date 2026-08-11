// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The authoring panel's pure pipeline model (issue #151, Model B —
 * always-valid canvas; docs/design/authoring-ux.md §4, selected in §8).
 *
 * The canvas invariant — "the pipeline is never in an invalid state" —
 * needs the client to know which pipeline shapes the server's process
 * compiler admits. This module is that knowledge, kept deliberately
 * TINY and PINNED BY TESTS ON BOTH SIDES (the design note's drift
 * mitigation): a stage table mirroring the compiler's `Value`/`Cube`
 * discipline (`crates/swath-render/src/process.rs`), the insertion rule
 * derived from it, and the formula builder's row model with its lowering
 * to a `reduce_dimension` reducer child graph.
 *
 * Everything here is pure data-in/data-out — no DOM — so the vitest
 * suite can pin each reachable-bad-state (§2's B1–B11) against it
 * directly.
 */

/** One parameter of a served process definition (openEO 1.2.0). */
export interface ProcessParameter {
  name: string;
  description?: string;
  optional?: boolean;
  default?: unknown;
  schema?: unknown;
}

/** The slice of a `GET /processes` entry the panel reads. */
export interface ProcessDefinition {
  id: string;
  summary?: string;
  parameters?: ProcessParameter[];
}

/**
 * The value flowing between top-level pipeline steps, mirroring the
 * compiler's `CubeKind` plus its scaled/unscaled discipline: a cube is
 * multi-band until a reduce step (NDVI or a formula) collapses it to
 * gray, and `linear_scale_range` marks it scaled — after which nothing
 * may reduce it (the compiler's "apply linear_scale_range after
 * reducing") and nothing may scale it again.
 */
export interface Stage {
  kind: "multi" | "gray";
  scaled: boolean;
}

/** The stage every pipeline starts in: `load_collection`'s output. */
export const LOAD_STAGE: Stage = { kind: "multi", scaled: false };

/**
 * The process ids that may appear BETWEEN the permanent Load head and
 * the permanent Output tail. Everything else the server serves —
 * arithmetic, `array_element` — is admitted only inside a reducer's
 * child graph (the compiler rejects it at top level), so the palette
 * never offers it there: the formula builder owns that context.
 */
export const MIDDLE_PROCESSES = ["ndvi", "reduce_dimension", "linear_scale_range"] as const;
export type MiddleProcess = (typeof MIDDLE_PROCESSES)[number];

/**
 * The compiler's per-process cube input, pinned: which argument each
 * pipeline process reads its cube through. The linear canvas wires it
 * to the previous step — no source selects, no dangling steps (B10).
 */
export const CUBE_PARAM: Record<string, string> = {
  ndvi: "data",
  reduce_dimension: "data",
  linear_scale_range: "x",
  save_result: "data",
};

/**
 * One stage transition, exactly the compiler's typing:
 *
 * - `ndvi` / `reduce_dimension` need a multi-band, UNSCALED cube
 *   (`unscaled_multi`) and produce gray;
 * - `linear_scale_range` needs any unscaled cube and marks it scaled
 *   (chained scaling is rejected server-side);
 *
 * `null` = the compiler would reject this step here.
 */
export function transition(stage: Stage, processId: string): Stage | null {
  switch (processId) {
    case "ndvi":
    case "reduce_dimension":
      if (stage.kind !== "multi" || stage.scaled) {
        return null;
      }
      return { kind: "gray", scaled: false };
    case "linear_scale_range":
      if (stage.scaled) {
        return null;
      }
      return { kind: stage.kind, scaled: true };
    default:
      return null;
  }
}

/** The stage after running the whole middle chain from the load head;
 * `null` if the chain does not type (unconstructible via the UI — the
 * insertion rule below never offers such a chain). */
export function finalStage(middle: readonly string[]): Stage | null {
  let stage: Stage | null = LOAD_STAGE;
  for (const id of middle) {
    stage = transition(stage, id);
    if (stage === null) {
      return null;
    }
  }
  return stage;
}

/** Would the middle chain still type end-to-end with `processId`
 * inserted at `gap` (0 = right after the load head)? The insertion rule:
 * a step can only be added where the WHOLE pipeline stays valid, so
 * scale-before-reduce (B4) and reduce-after-scale are unconstructible,
 * not merely flagged. */
export function canInsertAt(middle: readonly string[], gap: number, processId: string): boolean {
  const chain = [...middle.slice(0, gap), processId, ...middle.slice(gap)];
  return finalStage(chain) !== null;
}

/** The formula builder's requirements beyond `reduce_dimension` itself:
 * band operands lower to `array_element`, and a formula with no
 * arithmetic is no formula. */
export const FORMULA_OPS = ["add", "subtract", "multiply", "divide"] as const;
export type FormulaOp = (typeof FORMULA_OPS)[number];

/** Which of the middle processes the served definitions actually admit
 * at `gap`. The palette is this list — `divide` and friends can never
 * appear (B2/B3), and nothing is offered where it would not compile. */
export function insertableAt(
  middle: readonly string[],
  gap: number,
  served: ReadonlySet<string>,
): MiddleProcess[] {
  return MIDDLE_PROCESSES.filter((id) => {
    if (!served.has(id)) {
      return false;
    }
    if (
      id === "reduce_dimension" &&
      (!served.has("array_element") || !FORMULA_OPS.some((op) => served.has(op)))
    ) {
      return false;
    }
    return canInsertAt(middle, gap, id);
  });
}

// --- The formula builder's row model -----------------------------------

/** A formula operand: a band of the loaded cube, a plain number, or an
 * EARLIER line's result — selects and a number input, never free text,
 * so `UnknownBand` is unconstructible from here (B7). */
export type Operand =
  | { kind: "band"; band: string }
  | { kind: "number"; text: string }
  | { kind: "row"; index: number };

/** One line of the formula: `left op right`. The last line is the
 * formula's result. */
export interface FormulaRow {
  op: FormulaOp;
  left: Operand;
  right: Operand;
}

/** An unset operand, for freshly added lines. */
export const EMPTY_OPERAND: Operand = { kind: "band", band: "" };

/** What still blocks the formula, in plain words — the card's
 * self-explanation and the submit gate's per-formula count. `bands` is
 * the loaded-band vocabulary operands must come from (a band unchecked
 * on the Load step after being used here flags, not publishes). */
export function formulaIssues(rows: readonly FormulaRow[], bands: readonly string[]): string[] {
  if (rows.length === 0) {
    return ["add a line to the formula"];
  }
  const issues: string[] = [];
  rows.forEach((row, index) => {
    for (const [side, operand] of [
      ["left", row.left],
      ["right", row.right],
    ] as const) {
      switch (operand.kind) {
        case "band":
          if (operand.band === "") {
            issues.push(`line ${index + 1}: pick the ${side} value`);
          } else if (!bands.includes(operand.band)) {
            issues.push(`line ${index + 1}: ${operand.band} is not loaded any more`);
          }
          break;
        case "number":
          if (operand.text.trim() === "" || Number.isNaN(Number(operand.text))) {
            issues.push(`line ${index + 1}: the ${side} value must be a number`);
          }
          break;
        case "row":
          if (operand.index >= index || operand.index < 0) {
            issues.push(`line ${index + 1}: a line can only use lines above it`);
          }
          break;
      }
    }
  });
  return issues;
}

const OP_SYMBOL: Record<FormulaOp, string> = {
  add: "+",
  subtract: "−",
  multiply: "×",
  divide: "÷",
};

/** The formula in plain math ("(b8a − b04) ÷ (b8a + b04)"), for the
 * narrative — `""` while the formula is incomplete. */
export function formulaPhrase(rows: readonly FormulaRow[], bands: readonly string[]): string {
  if (formulaIssues(rows, bands).length > 0) {
    return "";
  }
  const phrase = (index: number): string => {
    const row = rows[index];
    if (row === undefined) {
      return "?";
    }
    const operand = (o: Operand): string => {
      switch (o.kind) {
        case "band":
          return o.band;
        case "number":
          return o.text.trim();
        case "row":
          return `(${phrase(o.index)})`;
      }
    };
    return `${operand(row.left)} ${OP_SYMBOL[row.op]} ${operand(row.right)}`;
  };
  return phrase(rows.length - 1);
}

/**
 * Lowers the formula to the standard openEO reducer child graph the
 * compiler evaluates: one `array_element` node per distinct band (over
 * `from_parameter: "data"`, the reducer's band array — the only place
 * the compiler admits it, B3), one arithmetic node per line, the last
 * line as the child graph's result.
 */
export function buildReducerGraph(
  rows: readonly FormulaRow[],
): Record<string, Record<string, unknown>> {
  const graph: Record<string, Record<string, unknown>> = {};
  const bandKeys = new Map<string, string>();
  const bandKey = (band: string): string => {
    let key = bandKeys.get(band);
    if (key === undefined) {
      key = `b${bandKeys.size + 1}`;
      bandKeys.set(band, key);
      graph[key] = {
        process_id: "array_element",
        arguments: { data: { from_parameter: "data" }, label: band },
      };
    }
    return key;
  };
  const argOf = (operand: Operand): unknown => {
    switch (operand.kind) {
      case "band":
        return { from_node: bandKey(operand.band) };
      case "number":
        return Number(operand.text);
      case "row":
        return { from_node: `r${operand.index + 1}` };
    }
  };
  rows.forEach((row, index) => {
    const node: Record<string, unknown> = {
      process_id: row.op,
      arguments: { x: argOf(row.left), y: argOf(row.right) },
    };
    if (index === rows.length - 1) {
      node["result"] = true;
    }
    graph[`r${index + 1}`] = node;
  });
  return graph;
}

// --- Shared vocabulary helpers ------------------------------------------

/** The first band matching any pattern, else the fallback — the NDVI
 * template's (and the NDVI card prefill's) nir/red picker over a
 * collection's band vocabulary. */
export function pickBand(bands: readonly string[], patterns: RegExp[], fallback: string): string {
  for (const pattern of patterns) {
    const match = bands.find((band) => pattern.test(band));
    if (match !== undefined) {
      return match;
    }
  }
  return fallback;
}

/** Where a server diagnostic points: the compiler names nodes as
 * ``node `key`(...)`` and arguments as ``argument `name```. Model B
 * makes these nearly unreachable; the mapping stays as the safety net
 * (the design note's "back to mapped server errors — the current,
 * survivable behavior"). */
export function locateServerError(message: string): { node?: string; argument?: string } {
  const located: { node?: string; argument?: string } = {};
  const node = message.match(/node `([^`]+)`/);
  if (node?.[1] !== undefined) {
    located.node = node[1];
  }
  const argument = message.match(/(?:missing required argument|invalid argument) `([^`]+)`/);
  if (argument?.[1] !== undefined) {
    located.argument = argument[1];
  }
  return located;
}
