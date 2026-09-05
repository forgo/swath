// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The timeline's pure model (#411): two bands over one time axis — what
 * the collection holds, and what survives the current filters. The gap
 * between them is the filter's effect, made visible.
 *
 * Both bands come from the counts endpoint (#410). Nothing here infers a
 * count from a fetched page: a bucket the server did not mention is a
 * zero, and a bucket nobody asked about is not drawn.
 */

/** One bucket of a counts answer, as this module needs it. */
export interface TimelineBucket {
  /** Bucket start, inclusive (RFC 3339 UTC). */
  start: string;
  /** Bucket end, exclusive. */
  end: string;
  /** Granules the collection holds in this bucket. */
  exists: number;
  /** Granules that survive the current filters. Never above `exists`. */
  survives: number;
}

export interface Timeline {
  buckets: TimelineBucket[];
  /** The largest `exists` on the axis — the height both bands scale to,
   * so the second band is read against the first and not against itself. */
  peak: number;
  /** Every granule the collection holds across the axis. */
  totalExists: number;
  /** Every granule that survives. */
  totalSurvives: number;
  /** True when the filters remove nothing: the bands are identical, and
   * the control says so rather than drawing a confusing empty band. */
  unfiltered: boolean;
}

export const EMPTY_TIMELINE: Timeline = {
  buckets: [],
  peak: 0,
  totalExists: 0,
  totalSurvives: 0,
  unfiltered: true,
};

/** The plain-words instruction on the control. No jargon, no unlabelled
 * brush — the e2e pins this string. */
export const TIMELINE_HINT = "Drag to narrow the dates";

interface CountsBucket {
  start?: unknown;
  end?: unknown;
  count?: unknown;
}

function readBuckets(body: unknown): Map<string, { end: string; count: number }> {
  const raw = (body as { buckets?: CountsBucket[] } | null)?.buckets ?? [];
  const out = new Map<string, { end: string; count: number }>();
  for (const bucket of raw) {
    if (typeof bucket?.start !== "string" || typeof bucket.end !== "string") {
      continue; // a cell bucket, or a malformed one: not a time axis
    }
    const count = typeof bucket.count === "number" && bucket.count >= 0 ? bucket.count : 0;
    out.set(bucket.start, { end: bucket.end, count });
  }
  return out;
}

/**
 * The two bands, from the unscoped counts (`exists`) and the scoped ones
 * (`survives`). The axis is the unscoped answer's buckets — the scope can
 * only ever remove granules, so a bucket the scoped answer mentions and
 * the unscoped one does not is a disagreement, and the larger of the two
 * is taken rather than drawing a band taller than its own axis.
 */
export function buildTimeline(existsBody: unknown, survivesBody: unknown): Timeline {
  const exists = readBuckets(existsBody);
  const survives = readBuckets(survivesBody);
  const starts = [...new Set([...exists.keys(), ...survives.keys()])].sort();
  const buckets: TimelineBucket[] = [];
  let peak = 0;
  let totalExists = 0;
  let totalSurvives = 0;
  for (const start of starts) {
    const held = exists.get(start);
    const kept = survives.get(start);
    const existsCount = Math.max(held?.count ?? 0, kept?.count ?? 0);
    const survivesCount = Math.min(kept?.count ?? 0, existsCount);
    const end = held?.end ?? kept?.end ?? start;
    buckets.push({ start, end, exists: existsCount, survives: survivesCount });
    peak = Math.max(peak, existsCount);
    totalExists += existsCount;
    totalSurvives += survivesCount;
  }
  return {
    buckets,
    peak,
    totalExists,
    totalSurvives,
    unfiltered: totalSurvives === totalExists,
  };
}

/** The bar height of `count`, as a fraction of the axis peak. Zero peak
 * is zero height — never a division that renders NaN. */
export function barHeight(count: number, peak: number): number {
  return peak > 0 ? Math.max(0, Math.min(1, count / peak)) : 0;
}

/** The inclusive date range covering buckets `a`..`b` (either order), as
 * the `from`/`to` a granule query takes: whole days, because the control
 * narrows dates and a person reads dates. */
export function rangeOf(
  timeline: Timeline,
  a: number,
  b: number,
): { from: string; to: string } | undefined {
  const last = timeline.buckets.length - 1;
  if (last < 0) {
    return undefined;
  }
  const lo = Math.max(0, Math.min(a, b));
  const hi = Math.min(last, Math.max(a, b));
  const first = timeline.buckets[lo];
  const final = timeline.buckets[hi];
  if (first === undefined || final === undefined) {
    return undefined;
  }
  // The bucket end is exclusive; the last day a person means is the day
  // before it, so a one-month bucket reads as the 1st to the 31st.
  return { from: dayOf(first.start), to: dayBefore(final.end) };
}

/** The `YYYY-MM-DD` of an RFC 3339 instant. */
export function dayOf(instant: string): string {
  return instant.slice(0, 10);
}

/** The day before an exclusive bucket end, `YYYY-MM-DD`. */
export function dayBefore(instant: string): string {
  const at = Date.parse(instant);
  if (Number.isNaN(at)) {
    return dayOf(instant);
  }
  return new Date(at - 86_400_000).toISOString().slice(0, 10);
}

/** What the control says about the two bands, in plain words. Never a
 * count the server did not give: an empty axis says it is empty. */
export function bandNote(timeline: Timeline): string {
  if (timeline.buckets.length === 0) {
    return "No granules on this timeline yet.";
  }
  const held = `${timeline.totalExists} granule${timeline.totalExists === 1 ? "" : "s"}`;
  if (timeline.unfiltered) {
    return `${held} — no filters, so every one is in view.`;
  }
  return `${timeline.totalSurvives} of ${held} survive the filters.`;
}
