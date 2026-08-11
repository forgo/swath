// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Trace analytics (issue #111): rolling p50/p95 render latency, the
 * decision mix (live / overview / cache_hit), and the cache hit rate —
 * computed client-side from the trace stream the x-ray overlay already
 * consumes. The distribution audit's finding was that this timing
 * material rides every `event: trace` and the UI discarded it; this
 * module keeps it. Zero server involvement, by design.
 *
 * # Fed by the overlay, not by its own stream
 *
 * The panel never opens an `EventSource` and never parses SSE data:
 * [`XRayOverlay`] hands it the [`TraceJson`] its single (typed,
 * validated) parse produced, so there is exactly one stream, one parser,
 * and one reconnect story — `EventSource`'s own, which the overlay
 * already relies on. On a disconnect the panel simply holds its last
 * computed values and resumes accumulating when the stream comes back;
 * there is nothing else for it to do, and that is the graceful part.
 *
 * # The statistics, pinned
 *
 * Percentiles are over a bounded sliding window of the last
 * [`ANALYTICS_WINDOW`] traces' `timings.total_ms` (all layers — the
 * panel describes the stream, like the feed, not the painted subset).
 * A rolling window rather than all-time, deliberately: the demo story
 * is "watch p95 move as you pan into cold tiles / back over warm ones",
 * and an unbounded aggregate freezes into its own history. The quantile
 * is linear interpolation between closest ranks (the "inclusive" method:
 * rank `p * (n - 1)` over the sorted window) — chosen because it is
 * exact-value testable by hand and matches numpy's default, our oracle
 * convention elsewhere. Mix counters and hit rate are all-time totals
 * for the panel's lifetime: rates over a tiny window whipsaw, and
 * "N traces, H hits" is the honest demo number.
 */

import { decisionKind, type TraceDecision } from "./swath-xray.js";

/** Percentile window: the last this-many traces. ~200 is a few
 * screenfuls of tile renders — recent enough to move when the workload
 * changes, wide enough that p95 (the 10th-ish largest sample) is not
 * one outlier's puppet. Matches the feed's scrollback bound. */
export const ANALYTICS_WINDOW = 200;

/** Linear-interpolation quantile (inclusive method) over an ascending-
 * sorted sample: rank `p * (n - 1)`, fractional ranks interpolate
 * between the neighbouring order statistics. `undefined` on an empty
 * sample — "no data yet" must stay distinguishable from 0 ms. */
export function quantileSorted(sorted: readonly number[], p: number): number | undefined {
  if (sorted.length === 0) {
    return undefined;
  }
  const rank = p * (sorted.length - 1);
  const lo = Math.floor(rank);
  const lower = sorted[lo];
  const upper = sorted[Math.ceil(rank)];
  if (lower === undefined || upper === undefined) {
    return undefined; // unreachable for p in [0,1]; the types demand it
  }
  return lower + (rank - lo) * (upper - lower);
}

/** All-time decision-mix counters (the flat kinds `decisionKind` yields). */
export interface DecisionMix {
  live: number;
  overview: number;
  cache_hit: number;
}

/** The pure aggregation core — no DOM, so unit tests drive it with
 * synthetic traces and assert hand-computed expectations exactly. */
export class TraceAnalytics {
  readonly #window: number;
  /** Insertion-ordered `total_ms` samples, bounded to the window. */
  readonly #samples: number[] = [];
  readonly #mix: DecisionMix = { live: 0, overview: 0, cache_hit: 0 };

  constructor(windowSize: number = ANALYTICS_WINDOW) {
    this.#window = windowSize;
  }

  /** Folds one trace in: `total_ms` into the rolling window (oldest
   * sample out once full), its decision kind into the all-time mix. */
  record(decision: TraceDecision, totalMs: number): void {
    this.#samples.push(totalMs);
    if (this.#samples.length > this.#window) {
      this.#samples.shift();
    }
    this.#mix[decisionKind(decision)] += 1;
  }

  /** Samples currently in the window (≤ the window size). */
  get sampleCount(): number {
    return this.#samples.length;
  }

  /** All-time traces folded in. */
  get total(): number {
    return this.#mix.live + this.#mix.overview + this.#mix.cache_hit;
  }

  /** All-time decision mix (a copy — the counters stay internal). */
  get mix(): DecisionMix {
    return { ...this.#mix };
  }

  /** `cache_hit / total`; `undefined` before any trace (0/0 is not 0%). */
  get hitRate(): number | undefined {
    const total = this.total;
    return total === 0 ? undefined : this.#mix.cache_hit / total;
  }

  /** Rolling-window quantile of `total_ms` (see [`quantileSorted`]);
   * `undefined` while the window is empty. The window is ≤200 samples,
   * so the sort-per-call is nothing. */
  quantile(p: number): number | undefined {
    return quantileSorted(
      [...this.#samples].sort((a, b) => a - b),
      p,
    );
  }
}

/** `12` / `17.5` — interpolated percentiles carry at most the one
 * decimal a half-step between integer millisecond samples produces
 * worth showing. */
function formatMs(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

/** The readouts-panel face of [`TraceAnalytics`]: one stats card in the
 * overlay's bottom-left readout column, styled like the ingest readout
 * and the heatmap legend (same dark card, same mono type; mix counts in
 * the decision palette's hue families so `live`/`overview`/`cache`
 * mean the same colors badges use). Exact values ride `data-*`
 * attributes for the tests; the text is the human formatting. */
export class AnalyticsPanel {
  readonly element: HTMLElement;
  readonly #analytics: TraceAnalytics;
  readonly #percentiles: HTMLDivElement;
  readonly #live: HTMLSpanElement;
  readonly #overview: HTMLSpanElement;
  readonly #cache: HTMLSpanElement;
  readonly #hit: HTMLSpanElement;

  constructor(doc: Document, analytics: TraceAnalytics = new TraceAnalytics()) {
    this.#analytics = analytics;
    this.element = doc.createElement("section");
    this.element.className = "swath-xray-analytics";
    this.element.setAttribute("aria-label", "Trace analytics");
    this.#percentiles = doc.createElement("div");
    const mixLine = doc.createElement("div");
    const span = (className: string): HTMLSpanElement => {
      const el = doc.createElement("span");
      el.className = className;
      return el;
    };
    this.#live = span("swath-xray-analytics-live");
    this.#overview = span("swath-xray-analytics-overview");
    this.#cache = span("swath-xray-analytics-cache");
    this.#hit = span("swath-xray-analytics-hit");
    const dot = (): Text => doc.createTextNode(" · ");
    mixLine.append(this.#live, dot(), this.#overview, dot(), this.#cache, dot(), this.#hit);
    this.element.append(this.#percentiles, mixLine);
    this.#render();
  }

  /** Folds one trace in and refreshes the card (a handful of text
   * nodes — per-trace update is well within budget, as the feed's
   * per-trace line already is). */
  record(decision: TraceDecision, totalMs: number): void {
    this.#analytics.record(decision, totalMs);
    this.#render();
  }

  #render(): void {
    const p50 = this.#analytics.quantile(0.5);
    const p95 = this.#analytics.quantile(0.95);
    const { dataset } = this.element;
    if (p50 === undefined || p95 === undefined) {
      this.#percentiles.textContent = "p50 — · p95 — ms";
      delete dataset.p50;
      delete dataset.p95;
    } else {
      this.#percentiles.textContent =
        `p50 ${formatMs(p50)} · p95 ${formatMs(p95)} ms ` + `(last ${this.#analytics.sampleCount})`;
      dataset.p50 = String(p50);
      dataset.p95 = String(p95);
    }
    dataset.samples = String(this.#analytics.sampleCount);
    const mix = this.#analytics.mix;
    this.#live.textContent = `live ${mix.live}`;
    this.#overview.textContent = `ovr ${mix.overview}`;
    this.#cache.textContent = `cache ${mix.cache_hit}`;
    dataset.live = String(mix.live);
    dataset.overview = String(mix.overview);
    dataset.cacheHit = String(mix.cache_hit);
    dataset.total = String(this.#analytics.total);
    const hitRate = this.#analytics.hitRate;
    if (hitRate === undefined) {
      this.#hit.textContent = "hit —";
      delete dataset.hitRate;
    } else {
      this.#hit.textContent = `hit ${(hitRate * 100).toFixed(1)}%`;
      dataset.hitRate = String(hitRate);
    }
  }
}
