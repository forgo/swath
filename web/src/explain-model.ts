// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The explain card's pure model (issue #394): what one card says, at each of
 * its three densities, derived from a trace envelope and the glossary.
 *
 * Separated from the element for the reason every model in `web/src` is: the
 * derivation is where the honesty rules live — every figure comes from the
 * server, an unmeasured one renders an em dash, and the planner's reasons are
 * quoted rather than paraphrased — and those are worth testing without a DOM.
 */
import { formatBytes as bytes } from "./format.js";
import { define, type GlossaryEntry } from "./glossary.js";
import {
  decisionLabel,
  type PlanCandidate,
  plannedLabel,
  type TraceEnvelope,
} from "./swath-xray.js";

/** How much the card is saying. */
export type ExplainDensity = "concept" | "measured" | "published";

/** One labelled figure. `value` is already formatted, em dash included. */
export interface ExplainRow {
  label: string;
  value: string;
  /** The instrument register renders these in mono. */
  mono?: boolean;
}

/** What the card shows. */
export interface ExplainContent {
  density: ExplainDensity;
  title: string;
  /** Prose, at `concept`; absent otherwise. */
  definition?: string;
  rows: ExplainRow[];
  /** The planner's candidates, quoted verbatim from the trace. */
  candidates: { strategy: string; cost: string; admissible: boolean; reason: string }[];
  /** A fix the trace's own reasons imply, in the product's words. */
  fix?: string;
}

/** The em dash: we have not measured this. Never `0`, never a spinner. */
export const UNMEASURED = "—";

/** Bytes, or the em dash when there is no measurement. The formatting is
 * `format.ts`'s — this only adds the unmeasured case, which is the card's
 * rule rather than the formatter's. */
export function explainBytes(value: number | null | undefined): string {
  return value === null || value === undefined || !Number.isFinite(value)
    ? UNMEASURED
    : bytes(value);
}

/** Milliseconds, or the em dash. */
export function explainMs(value: number | null | undefined): string {
  return value === null || value === undefined || !Number.isFinite(value)
    ? UNMEASURED
    : `${Math.round(value)} ms`;
}

/** The `concept` density: a definition, no numbers. */
export function conceptContent(term: string): ExplainContent | undefined {
  const entry: GlossaryEntry | undefined = define(term);
  if (!entry) {
    return undefined;
  }
  return {
    density: "concept",
    title: entry.term,
    definition: entry.definition,
    rows: [],
    candidates: [],
  };
}

/** The exact reason the planner emits when a dataset has no overviews at
 * all — `crates/swath-planner/src/lib.rs`, the `None` arm of the overview
 * candidate. Matched exactly, not fuzzily: a substring match would keep
 * passing while silently never firing again if the wording changed.
 * `swath-docs-check` asserts the planner still emits this string. */
export const NO_OVERVIEWS_REASON = "source has no overviews";

/** The fix a rejected candidate implies, in the product's words.
 *
 * Nothing is invented. The planner already records, for every candidate it
 * rejected, why — this only turns the one reason the operator can act on
 * into the action. The other rejections (`no source window`, `no overview
 * factor eligible at this zoom`) are not things `swath materialize` fixes,
 * so they produce no fix line.
 */
export function fixFor(candidates: readonly PlanCandidate[]): string | undefined {
  const blocked = candidates.some((c) => !c.admissible && c.reason === NO_OVERVIEWS_REASON);
  return blocked
    ? "This dataset has no overviews, so every tile is read at full resolution. Building them with `swath materialize` makes zoomed-out views read a small image instead of a large one."
    : undefined;
}

/** The `measured` density: what the planner decided for one tile, and why. */
export function measuredContent(envelope: TraceEnvelope): ExplainContent {
  const { trace } = envelope;
  const plan = trace.plan ?? null;
  const rows: ExplainRow[] = [
    {
      label: "decision",
      value: plan ? plannedLabel(plan.chosen) : decisionLabel(trace.decision),
      mono: true,
    },
    { label: "tile", value: envelope.tile, mono: true },
    { label: "bytes read", value: explainBytes(trace.bytes_read), mono: true },
    { label: "total", value: explainMs(trace.timings?.total_ms), mono: true },
    { label: "read", value: explainMs(trace.timings?.read_ms), mono: true },
    { label: "warp", value: explainMs(trace.timings?.warp_ms), mono: true },
    { label: "pixels", value: explainMs(trace.timings?.pixel_ops_ms), mono: true },
    { label: "encode", value: explainMs(trace.timings?.encode_ms), mono: true },
    { label: "ingest→pixel", value: explainMs(trace.ingest_to_pixel_ms), mono: true },
  ];
  const candidates = (plan?.considered ?? []).map((c) => ({
    strategy: plannedLabel(c.strategy),
    cost: explainBytes(c.estimated_cost_bytes),
    admissible: c.admissible,
    // Quoted, never paraphrased: the planner's words are the explanation.
    reason: c.reason,
  }));
  const content: ExplainContent = {
    density: "measured",
    title: envelope.layer,
    rows,
    candidates,
  };
  const fix = fixFor(plan?.considered ?? []);
  if (fix !== undefined) {
    content.fix = fix;
  }
  return content;
}

/** What a publish produced, beside the first tile's measurements. */
export interface PublishReceipt {
  /** OGC API - Tiles URL, as the server serves it. */
  ogc?: string;
  /** XYZ template, as the server serves it. */
  xyz?: string;
  /** `swath:sources` — the granules the first tile read. */
  sources?: readonly string[];
  /** `swath:window` — the compiled window, verbatim. */
  window?: string;
}

/** The `published` density: the measured card, plus what was published. */
export function publishedContent(envelope: TraceEnvelope, receipt: PublishReceipt): ExplainContent {
  const content = measuredContent(envelope);
  const rows: ExplainRow[] = [
    { label: "ogc", value: receipt.ogc ?? UNMEASURED, mono: true },
    { label: "xyz", value: receipt.xyz ?? UNMEASURED, mono: true },
    { label: "window", value: receipt.window ?? UNMEASURED, mono: true },
    {
      label: "granules",
      value:
        receipt.sources && receipt.sources.length > 0 ? receipt.sources.join(", ") : UNMEASURED,
      mono: true,
    },
    ...content.rows,
  ];
  return { ...content, density: "published", rows };
}
