// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The x-ray overlay (issue #34) — the glass-box keystone (CHARTER.md §6,
 * REQUIREMENTS.md R4): per-tile materialization decisions, timings, and
 * bytes painted over the map, fed live by the Trace SSE stream
 * (`GET /traces`, swath-api `traces` module).
 *
 * # Built-in feature, not a separate element
 *
 * `XRayOverlay` is a module `<swath-map>` owns and toggles via its `xray`
 * attribute (plus a toggle button control). A separate `<swath-xray>`
 * element wired to a map was considered and rejected for v0: the overlay
 * is meaningless without a map to paint on, and the wiring (which map?
 * lifecycle races on connect order?) buys nothing until a second consumer
 * exists. The class only depends on [`XRayMapLike`] — the two methods and
 * two event hooks it actually uses — so unit tests drive it with a fake
 * map and a promotion to a standalone element stays cheap.
 *
 * # DOM badges, not a custom WebGL layer
 *
 * ARCHITECTURE.md §13 allows either. v0 paints positioned DOM elements
 * over the map container: at most ~500 stored traces of which a screenful
 * is visible, repositioned once per frame at worst — trivially within DOM
 * budget, and it gets accessibility (real buttons, a focusable inspector
 * dialog) for free where WebGL would need a parallel a11y tree. The
 * upgrade path when badge counts or per-frame sync ever matter is a
 * MapLibre custom layer reading the same store; the store/paint split
 * here keeps that a paint-side swap.
 *
 * # The wire contract consumed
 *
 * `event: trace` carries `{"tile":"z/x/y","layer":"...","trace":{...}}`
 * (envelope pinned in swath-api; inner `Trace` pinned in swath-core).
 * Since #37 the trace also carries `plan` — the planner's chosen
 * strategy plus every weighed candidate — which v1 (issue #42) renders
 * in the inspector's why-view; `plan` is null/absent on traces that
 * never went through the planner and the section is simply absent then.
 * `event: lagged` carries `{"missed":N}` — surfaced as a badge, because
 * the stream is best-effort telemetry and drops must be visible, not
 * silent. Keepalive comments and reconnects are `EventSource`'s problem.
 */

/** The planner's decision, as the pinned `Trace` JSON serializes it:
 * `live` as a bare string; `overview` and (since #36) `cache_hit`
 * externally tagged. The bare `"cache_hit"` string is tolerated for
 * compatibility with pre-#36 emitters. */
export type TraceDecision =
  | "live"
  | "cache_hit"
  | { overview: { level: number } }
  | { cache_hit: { key: string } };

import { type CompareSides, traceSide } from "./compare-model.js";
import { formatBytes, formatKb } from "./format";
import { tileNorthWest } from "./tms.js";
import { adoptSheet, css, readToken } from "./ui/styles.js";
import { AnalyticsPanel } from "./xray-analytics.js";

/** A candidate strategy as the plan payload names it (`PlannedStrategy`,
 * pinned in swath-core `planner`): the decision vocabulary minus
 * execution details — overviews carry the decimation `factor` (not a
 * level; `0` in an inadmissible record means none was selectable), cache
 * hits no key. */
export type PlannedStrategy = "live" | "cache_hit" | { overview: { factor: number } };

/** One weighed candidate (`CandidateTrace`, pinned in swath-core):
 * recorded for every candidate, chosen or not. */
export interface PlanCandidate {
  strategy: PlannedStrategy;
  estimated_cost_bytes: number;
  admissible: boolean;
  reason: string;
}

/** The planner's reasoning (`PlanTrace`, pinned in swath-core `trace`,
 * riding the envelope since #37): the chosen strategy plus every
 * candidate in fixed evaluation order `cache_hit`, `overview`, `live`. */
export interface PlanTrace {
  chosen: PlannedStrategy;
  considered: PlanCandidate[];
}

/** Per-stage wall-clock timings (`Timings`, pinned in swath-core). */
export interface TraceTimings {
  read_ms: number;
  warp_ms: number;
  pixel_ops_ms: number;
  encode_ms: number;
  total_ms: number;
  /** The `run_udf` stage's share of `pixel_ops_ms` (ADR 0018); omitted
   * by the emitter when no UDF ran, so pre-UDF traces are unchanged. */
  udf_ms?: number;
}

/** One byte range read from a source (`Provenance`, pinned). */
export interface TraceProvenance {
  path: string;
  offset: number;
  length: number;
}

/** The temporal decision (`TemporalTrace`, pinned in swath-core; riding
 * the envelope since #223, ADR 0015): which granule backs this frame,
 * what was asked, and under which resolution rule. */
export interface TraceTemporal {
  /** The granule the frame resolved to. */
  granule_id: string;
  /** That granule's acquisition datetime — the frame's identity, and
   * what the per-frame analytics key on (issue #182). */
  granule_datetime: string;
  /** The raw `datetime=` request; null when absent (plain latest). */
  requested: string | null;
  /** The resolution rule that ran (`latest`, `latest_at_or_before`,
   * `latest_in_interval`). */
  rule: string;
  /** One record per branch of a two-source frame (ADR 0022, #301):
   * which granule each `load_collection` resolved to. Absent or empty
   * on one-source frames. */
  sources?: TraceTemporalSource[];
}

/** One branch's granule of a two-source frame. */
export interface TraceTemporalSource {
  /** The `load_collection` node id of the branch. */
  node: string;
  granule_id: string;
  granule_datetime: string;
}

/** The core `Trace` JSON (pinned in swath-core `trace` module). */
export interface TraceJson {
  decision: TraceDecision;
  source: string;
  sources: string[];
  crs_from: number;
  crs_to: number;
  bytes_read: number;
  provenance: TraceProvenance[];
  timings: TraceTimings;
  ingest_to_pixel_ms: number | null;
  /** The planner's reasoning; `null` on traces that never went through
   * the planner. Optional because pre-#37 emitters (and synthetic test
   * envelopes) omit the field entirely — both read as "no plan". */
  plan?: PlanTrace | null;
  /** The temporal decision (#223); `null` on static layers (a single
   * timeless frame). Optional for pre-#223 emitters and synthetic test
   * envelopes — both read as "no time dimension". */
  temporal?: TraceTemporal | null;
  /** The deterministic fuel a `run_udf` stage consumed (ADR 0018, #205);
   * omitted when no UDF ran. */
  udf_fuel_used?: number | null;
}

/** The `data:` payload of one `event: trace` (envelope pinned in
 * swath-api `traces`): `tile` is XYZ-ordered `"z/x/y"`. */
export interface TraceEnvelope {
  tile: string;
  layer: string;
  trace: TraceJson;
}

/** The single typed parser for `event: trace` payloads: JSON plus the
 * envelope's shape invariants (parsable `"z/x/y"` tile, string layer,
 * object trace). `undefined` on anything malformed — dropped, never
 * fatal. Every consumer of the stream's data goes through this one
 * function; the analytics panel deliberately has no parser of its own. */
export function parseTraceEnvelope(
  data: string,
): (TraceEnvelope & { z: number; x: number; y: number }) | undefined {
  let envelope: TraceEnvelope;
  try {
    envelope = JSON.parse(data) as TraceEnvelope;
  } catch {
    return undefined;
  }
  const tile = parseTile(envelope.tile);
  if (!tile || typeof envelope.layer !== "string" || typeof envelope.trace !== "object") {
    return undefined;
  }
  return { ...envelope, ...tile };
}

/** The slice of `EventSource` the overlay uses — the seam unit tests
 * stub (a fake implementing this feeds synthetic envelopes, no network). */
export interface EventSourceLike {
  addEventListener(type: string, listener: (event: MessageEvent<string>) => void): void;
  close(): void;
}

/** How the overlay opens its SSE stream; the default is the real
 * `EventSource` (which owns reconnection). */
export type EventSourceFactory = (url: string) => EventSourceLike;

/** Where a host puts the overlay's chrome (#286): containers for the
 * readouts, the display-mode group, the trace feed, the why-view
 * inspector, and (optionally) the analytics summary. Absent parts float
 * in the map as before. */
export interface XRayChrome {
  readonly readouts?: HTMLElement | undefined;
  readonly modes?: HTMLElement | undefined;
  readonly feed?: HTMLElement | undefined;
  readonly inspector?: HTMLElement | undefined;
  readonly analytics?: HTMLElement | undefined;
}

/** The slice of a MapLibre `Map` the overlay uses. Narrow on purpose:
 * unit tests supply a fake with a known `project()`, and nothing here
 * couples the overlay to MapLibre's full API. */
export interface XRayMapLike {
  /** Container-pixel position of a `[lng, lat]` coordinate. */
  project(lngLat: [number, number]): { x: number; y: number };
  getZoom(): number;
  on(type: string, listener: () => void): unknown;
  off(type: string, listener: () => void): unknown;
}

/** One stored trace: parsed tile address + the envelope payload. The
 * `side` tag exists only on entries received while a compare was active
 * (issue #210): those paint into the side-clipped badge layers, and the
 * side rides the store key so both sides of one tile coexist. */
interface XRayEntry {
  layer: string;
  z: number;
  x: number;
  y: number;
  trace: TraceJson;
  side?: "left" | "right";
}

/** The compare state the overlay paints per-side badges for (issue
 * #210): the handle fraction (the clip) plus the side identities the
 * received traces are matched against (compare-model `traceSide`). */
export interface XRayCompare {
  fraction: number;
  sides: CompareSides;
}

/** Store bound: per-tile latest-wins entries kept, least recently
 * *updated* evicted first. ~500 covers several screenfuls across a pan
 * without unbounded growth on a long-lived page. */
const DEFAULT_CAPACITY = 500;

/** Feed bound: at most this many rendered lines; older lines drop off
 * the top with a visible dropped counter. ~200 lines is a few minutes
 * of demo panning — enough scrollback to be useful, small enough that
 * the DOM never grows without bound. */
const FEED_CAPACITY = 200;

/** What the overlay paints on badges: `decision` (v0 colors), `bytes`
 * (bytes_read intensity heatmap), or `off` (no badges — readouts and
 * feed stay). */
export type XRayDisplayMode = "decision" | "bytes" | "off";

const DISPLAY_MODES: readonly XRayDisplayMode[] = ["decision", "bytes", "off"];

/** Number of non-zero intensity buckets in the bytes heatmap. */
export const BYTES_BUCKET_COUNT = 5;

/**
 * Heatmap bucket for `bytes` given the store's non-zero min/max:
 * `0` is the dedicated zero bucket — a cache hit reads no source bytes,
 * which is a different *kind* of render, not the low end of the same
 * scale, so it gets a visually distinct style instead of the lightest
 * ramp step. `1..BYTES_BUCKET_COUNT` are equal steps in **log** space.
 *
 * Log, not linear, deliberately: bytes_read spans orders of magnitude
 * (an overview read is tens of KB where the full-resolution live render
 * is MBs), so a linear scale would crush every overview into one
 * indistinct bottom bucket. The log scale is what makes the
 * overview/cache savings visibly obvious as you pan between zooms —
 * the demo point of the heatmap.
 *
 * A degenerate range (min == max, or no non-zero entries yet) maps every
 * non-zero value to the top bucket: one value is its own maximum.
 */
export function bytesBucket(bytes: number, min: number, max: number): number {
  if (bytes <= 0) {
    return 0;
  }
  if (!(max > min) || min <= 0) {
    return BYTES_BUCKET_COUNT;
  }
  const t = (Math.log(bytes) - Math.log(min)) / (Math.log(max) - Math.log(min));
  return 1 + Math.max(0, Math.min(BYTES_BUCKET_COUNT - 1, Math.floor(t * BYTES_BUCKET_COUNT)));
}

/** Sequential single-hue ramp (orange, light→dark, monotonic lightness)
 * for buckets 1..5. Border carries the hue at full strength; the tint
 * deepens with the bucket so intensity reads at a glance over imagery.
 * Badge text (ms/KB) stays white-on-dark ink — the secondary encoding
 * that keeps adjacent buckets tellable apart without color. */
/** A badge colour: the token's value (MapLibre-pixel chrome in the light
 * DOM cannot inherit custom properties from a card) and a translucent
 * tint mixed from it. Resolved at overlay construction (#286). */
function shade(token: string, tintPct: number): { border: string; tint: string } {
  const border = readToken(token);
  return { border, tint: `color-mix(in srgb, ${border} ${tintPct}%, transparent)` };
}

/** Bytes-read intensity ramp, coolest → hottest: the heat tokens. */
function bytesRamp(): readonly { border: string; tint: string }[] {
  return [
    shade("--swath-color-heat-1", 16),
    shade("--swath-color-heat-2", 20),
    shade("--swath-color-heat-3", 25),
    shade("--swath-color-heat-4", 30),
    shade("--swath-color-heat-5", 36),
  ];
}

/** The zero bucket: dashed, the cache family's hue plus a border-style
 * change, so "read nothing" is distinct from the ramp even in grayscale. */
function bytesZero(): { border: string; tint: string } {
  return shade("--swath-color-decision-cache", 8);
}

/** Decision colours: live (green), overview (amber), cache_hit (blue) —
 * the `decision-*` tokens, contractual before overviews/cache can produce
 * them. */
function decisionColors(): Record<
  "live" | "overview" | "cache_hit",
  { border: string; tint: string }
> {
  return {
    live: shade("--swath-color-decision-live", 12),
    overview: shade("--swath-color-decision-overview", 12),
    cache_hit: shade("--swath-color-decision-cache", 12),
  };
}

const OVERLAY_SHEET = css`
/* No z-index, deliberately: the root must NOT form a stacking context.
 * MapLibre's control corners sit at z-index 2, so badges (z auto) stay
 * beneath the toggles — a badge under the x-ray button must never
 * intercept its click — while the inspector (z 3) and feed (z 2) still
 * compete at the host level and rise above them. The compare swipe's
 * right map (issue #210) is kept underneath by DOM order instead. */
.swath-xray { position: absolute; inset: 0; overflow: hidden; pointer-events: none; }
/* Per-side badge layers (issue #210): clipped to their side of the
 * compare handle; empty and unclipped while no compare is active. */
.swath-xray-side { position: absolute; inset: 0; pointer-events: none; }
.swath-xray * { box-sizing: border-box; }
.swath-xray-badge {
  position: absolute;
  pointer-events: auto;
  display: flex;
  align-items: flex-start;
  margin: 0;
  padding: 0;
  border: 2px solid;
  background: none;
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.4;
  color: var(--swath-color-fg);
  cursor: pointer;
  text-align: left;
}
.swath-xray-badge > span {
  padding: 1px 4px;
  background: color-mix(in srgb, var(--swath-color-bg) 65%, transparent);
  border-radius: 0 0 4px 0;
  white-space: nowrap;
}
.swath-xray-readouts {
  position: absolute;
  left: 8px;
  bottom: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  align-items: flex-start;
  pointer-events: none;
}
.swath-xray-ingest {
  padding: 4px 10px;
  border-radius: 4px;
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-accent);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-lg); font-weight: 700; line-height: var(--swath-leading-normal);
}
.swath-xray-lagged {
  padding: 2px 8px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--swath-color-danger) 60%, var(--swath-color-bg));
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: var(--swath-leading-normal);
}
.swath-xray-lagged[hidden] { display: none; }
.swath-xray-inspector {
  position: absolute;
  z-index: 3;
  pointer-events: auto;
  max-width: 340px;
  max-height: 60%;
  overflow: auto;
  padding: 8px 10px;
  border-radius: 6px;
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-sm); line-height: var(--swath-leading-normal);
}
.swath-xray-inspector:focus { outline: var(--swath-border-focus); }
.swath-xray-inspector header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  font-weight: 700;
}
.swath-xray-inspector header button {
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
  padding: 0 4px;
}
.swath-xray-inspector dl { margin: 6px 0; }
.swath-xray-inspector dt { color: var(--swath-color-fg-muted); margin-top: 4px; }
.swath-xray-inspector dd { margin: 0; }
.swath-xray-inspector .swath-xray-provenance {
  max-height: 9em;
  overflow: auto;
  margin: 0;
  padding-left: 1em;
}
.swath-xray-inspector .swath-xray-provenance li { word-break: break-all; }
.swath-xray-plan-title {
  margin: 8px 0 2px;
  font-size: 12px;
  color: var(--swath-color-fg-muted);
}
.swath-xray-plan {
  width: 100%;
  border-collapse: collapse;
}
.swath-xray-plan th,
.swath-xray-plan td {
  text-align: left;
  vertical-align: top;
  padding: 1px 8px 1px 0;
  font-weight: 400;
}
.swath-xray-plan th { color: var(--swath-color-fg-muted); }
.swath-xray-plan tr[data-chosen="true"] td { color: var(--swath-color-accent); font-weight: 700; }
.swath-xray-plan tr[data-admissible="false"] td { color: var(--swath-color-fg-muted); }
.swath-xray-modes {
  position: absolute;
  top: 8px;
  left: 8px;
  display: flex;
  overflow: hidden;
  border-radius: 4px;
  pointer-events: auto;
}
.swath-xray-modes button {
  border: 0;
  margin: 0;
  padding: 3px 8px;
  background: color-mix(in srgb, var(--swath-color-bg) 65%, transparent);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: 1.4;
  cursor: pointer;
}
.swath-xray-modes button[aria-pressed="true"] {
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-weight: 700;
}
.swath-xray-scale {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border-radius: 4px;
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: var(--swath-leading-normal);
}
.swath-xray-scale[hidden] { display: none; }
.swath-xray-scale-swatch {
  display: inline-block;
  width: 12px;
  height: 10px;
}
.swath-xray-scale-zero {
  border: 2px dashed var(--swath-color-decision-cache);
  background: none;
}
.swath-xray-feed {
  position: absolute;
  right: 8px;
  bottom: 8px;
  z-index: 2;
  width: min(440px, 60%);
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  pointer-events: none;
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: var(--swath-leading-normal);
}
.swath-xray-feed header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--swath-color-bg-hud);
  pointer-events: auto;
}
.swath-xray-feed header button {
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
  padding: 1px 4px;
}
.swath-xray-feed-toggle[aria-expanded="true"] { color: var(--swath-color-accent); font-weight: 700; }
.swath-xray-feed-pause[aria-pressed="true"] { color: var(--swath-color-warn); font-weight: 700; }
.swath-xray-feed-dropped { color: var(--swath-color-warn); }
.swath-xray-feed-lines {
  align-self: stretch;
  margin: 2px 0 0;
  padding: 4px 6px;
  list-style: none;
  max-height: 180px;
  overflow: auto;
  background: var(--swath-color-bg-hud);
  border-radius: 4px;
  pointer-events: auto;
}
.swath-xray-feed-lines[hidden] { display: none; }
.swath-xray-feed-lines li { white-space: nowrap; }
.swath-xray-feed-lines li > button {
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
  padding: 0;
}
.swath-xray-feed-lines li > button:hover,
.swath-xray-feed-lines li > button:focus { color: var(--swath-color-accent); }
.swath-xray-feed-line-lagged { color: var(--swath-color-danger); }
.swath-xray-badge-flash { outline: var(--swath-border-focus); outline-offset: 2px; }
.swath-xray-analytics {
  display: flex;
  flex-direction: column;
  padding: 4px 10px;
  border-radius: 4px;
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono); font-size: var(--swath-text-xs); line-height: var(--swath-leading-normal);
}
.swath-xray-analytics-frame { color: var(--swath-color-fg-muted); }
.swath-xray-analytics-frame[hidden] { display: none; }
.swath-xray-analytics-udf { color: var(--swath-color-udf); }
.swath-xray-analytics-udf[hidden] { display: none; }
/* The counters are the badge legend in words, so they read the SAME decision
 * tokens the badges do (#433). They used to reach for accent/warn/info, which
 * happened to match while those values coincided — and stopped matching for
 * cache the moment the decision blue moved (#383). */
.swath-xray-analytics-live { color: var(--swath-color-decision-live); }
.swath-xray-analytics-overview { color: var(--swath-color-decision-overview); }
.swath-xray-analytics-cache { color: var(--swath-color-decision-cache); }
.swath-xray-analytics-hit { font-weight: 700; }
/* Hosted by a shell (a HUD card or the rail) instead of floating in the
 * map: the same chrome, flow layout (#286). */
.swath-xray-docked { position: static; inset: auto; max-width: none; max-height: none; margin: 0; }
.swath-xray-feed.swath-xray-docked { width: auto; min-width: max-content; align-items: stretch; }
.swath-xray-feed.swath-xray-docked header { white-space: nowrap; }
.swath-xray-inspector.swath-xray-docked { width: auto; }
`;

/** The decision's flat kind — what the badge color and `data-decision`
 * carry (`{"overview":{...}}` collapses to `"overview"`,
 * `{"cache_hit":{...}}` to `"cache_hit"`). */
export function decisionKind(decision: TraceDecision): "live" | "cache_hit" | "overview" {
  if (typeof decision === "string") return decision;
  return "cache_hit" in decision ? "cache_hit" : "overview";
}

function decisionLabel(decision: TraceDecision): string {
  if (typeof decision === "string") return decision;
  if ("cache_hit" in decision) {
    return `cache_hit (${decision.cache_hit.key.slice(0, 8)}…)`;
  }
  return `overview (level ${decision.overview.level})`;
}

/** Human label for a plan candidate strategy: `"live"`, `"cache_hit"`,
 * or `"overview (factor N)"`. Doubles as the identity the chosen-row
 * match compares on — the vocabulary has no other distinguishing field. */
function plannedLabel(strategy: PlannedStrategy): string {
  if (typeof strategy === "string") {
    return strategy;
  }
  return `overview (factor ${strategy.overview.factor})`;
}

/** `"z/x/y"` → numbers; undefined when malformed. */
function parseTile(tile: string): { z: number; x: number; y: number } | undefined {
  const parts = tile.split("/").map(Number);
  const [z, x, y] = parts;
  if (parts.length !== 3 || z === undefined || x === undefined || y === undefined) {
    return undefined;
  }
  return Number.isFinite(z) && Number.isFinite(x) && Number.isFinite(y) ? { z, x, y } : undefined;
}

/** Wall-clock `HH:MM:SS` for a feed line. */
function feedTime(date: Date): string {
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** The overlay engine: SSE client + bounded per-tile store + DOM paint.
 * `<swath-map>` owns one while its `xray` attribute is present. */
export class XRayOverlay {
  readonly #map: XRayMapLike;
  readonly #root: HTMLDivElement;
  readonly #badges: HTMLDivElement;
  readonly #badgesLeft: HTMLDivElement;
  readonly #badgesRight: HTMLDivElement;
  readonly #ingest: HTMLDivElement;
  readonly #lagged: HTMLDivElement;
  readonly #createEventSource: EventSourceFactory;
  readonly #ramp = bytesRamp();
  readonly #zero = bytesZero();
  readonly #decisions = decisionColors();
  #readouts: HTMLElement | undefined;
  #chrome: XRayChrome | undefined;
  #onEnvelope: ((envelope: TraceEnvelope) => void) | undefined;
  readonly #capacity: number;
  readonly #onMove: () => void;

  /** Latest trace per tile, keyed `"layer/z/x/y"`. `Map` iteration order
   * is insertion order and updates delete-then-set, so the first key is
   * always the least recently updated — the LRU eviction victim. */
  readonly #store = new Map<string, XRayEntry>();

  readonly #analytics: AnalyticsPanel;
  readonly #modes: HTMLDivElement;
  readonly #scale: HTMLDivElement;
  readonly #scaleRange: HTMLSpanElement;
  readonly #feed: HTMLElement;
  readonly #feedToggle: HTMLButtonElement;
  readonly #feedPause: HTMLButtonElement;
  readonly #feedDroppedBadge: HTMLSpanElement;
  readonly #feedLines: HTMLOListElement;

  #source: EventSourceLike | undefined;
  #url: string | undefined;
  #layer = "";
  #missed = 0;
  #ingestMs: number | undefined;
  #frame: number | undefined;
  #inspector: HTMLElement | undefined;
  #disposed = false;
  #compare: XRayCompare | undefined;
  #mode: XRayDisplayMode = "decision";
  #feedPaused = false;
  #feedDropped = 0;
  /** Lines received while paused — appended on resume, so pause freezes
   * the visible content (and scroll position) without losing traffic. */
  #feedPending: HTMLLIElement[] = [];

  constructor(
    host: HTMLElement,
    map: XRayMapLike,
    options: {
      createEventSource?: EventSourceFactory | undefined;
      capacity?: number;
      chrome?: XRayChrome | undefined;
      /** Every parsed envelope, before the store (#287: the host relays
       * it as `swath-trace`). */
      onEnvelope?: ((envelope: TraceEnvelope) => void) | undefined;
    } = {},
  ) {
    this.#onEnvelope = options.onEnvelope;
    this.#map = map;
    this.#createEventSource = options.createEventSource ?? ((url) => new EventSource(url));
    this.#capacity = options.capacity ?? DEFAULT_CAPACITY;
    adoptSheet(OVERLAY_SHEET, host.ownerDocument);

    this.#root = document.createElement("div");
    this.#root.className = "swath-xray";
    this.#badges = document.createElement("div");
    this.#badgesLeft = document.createElement("div");
    this.#badgesLeft.className = "swath-xray-side";
    this.#badgesLeft.dataset.side = "left";
    this.#badgesRight = document.createElement("div");
    this.#badgesRight.className = "swath-xray-side";
    this.#badgesRight.dataset.side = "right";
    const readouts = document.createElement("div");
    readouts.className = "swath-xray-readouts";
    this.#lagged = document.createElement("div");
    this.#lagged.className = "swath-xray-lagged";
    this.#lagged.setAttribute("role", "status");
    this.#lagged.hidden = true;
    this.#ingest = document.createElement("div");
    this.#ingest.className = "swath-xray-ingest";
    this.#ingest.setAttribute("role", "status");
    this.#ingest.setAttribute("aria-label", "Ingest to pixel latency");
    this.#ingest.textContent = "ingest→pixel: —";
    this.#scale = document.createElement("div");
    this.#scale.className = "swath-xray-scale";
    this.#scale.setAttribute("aria-label", "Bytes-read heatmap scale");
    this.#scale.hidden = true;
    const zeroSwatch = document.createElement("span");
    zeroSwatch.className = "swath-xray-scale-swatch swath-xray-scale-zero";
    const zeroLabel = document.createElement("span");
    zeroLabel.textContent = "0 (cache)";
    this.#scale.append(zeroSwatch, zeroLabel);
    for (const step of this.#ramp) {
      const swatch = document.createElement("span");
      swatch.className = "swath-xray-scale-swatch";
      swatch.style.backgroundColor = step.border;
      this.#scale.append(swatch);
    }
    this.#scaleRange = document.createElement("span");
    this.#scaleRange.className = "swath-xray-scale-range";
    this.#scaleRange.textContent = "—";
    this.#scale.append(this.#scaleRange);
    this.#analytics = new AnalyticsPanel(host.ownerDocument);
    readouts.append(this.#lagged, this.#scale, this.#analytics.element, this.#ingest);

    this.#modes = document.createElement("div");
    this.#modes.className = "swath-xray-modes";
    this.#modes.setAttribute("role", "group");
    this.#modes.setAttribute("aria-label", "X-ray display mode");
    for (const mode of DISPLAY_MODES) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = mode;
      button.dataset.mode = mode;
      button.setAttribute("aria-pressed", String(mode === this.#mode));
      button.addEventListener("click", () => this.setDisplayMode(mode));
      this.#modes.append(button);
    }

    this.#feed = document.createElement("section");
    this.#feed.className = "swath-xray-feed";
    this.#feed.setAttribute("aria-label", "Live trace feed");
    const feedHeader = document.createElement("header");
    this.#feedToggle = document.createElement("button");
    this.#feedToggle.type = "button";
    this.#feedToggle.className = "swath-xray-feed-toggle";
    this.#feedToggle.textContent = "trace feed";
    this.#feedToggle.setAttribute("aria-expanded", "false");
    this.#feedToggle.addEventListener("click", () => this.#toggleFeed());
    this.#feedDroppedBadge = document.createElement("span");
    this.#feedDroppedBadge.className = "swath-xray-feed-dropped";
    this.#feedDroppedBadge.hidden = true;
    this.#feedPause = document.createElement("button");
    this.#feedPause.type = "button";
    this.#feedPause.className = "swath-xray-feed-pause";
    this.#feedPause.textContent = "pause";
    this.#feedPause.setAttribute("aria-label", "Pause trace feed");
    this.#feedPause.setAttribute("aria-pressed", "false");
    this.#feedPause.hidden = true;
    this.#feedPause.addEventListener("click", () => this.#togglePause());
    feedHeader.append(this.#feedToggle, this.#feedDroppedBadge, this.#feedPause);
    this.#feedLines = document.createElement("ol");
    this.#feedLines.className = "swath-xray-feed-lines";
    this.#feedLines.setAttribute("aria-label", "Received traces, oldest first");
    this.#feedLines.hidden = true;
    this.#feed.append(feedHeader, this.#feedLines);

    this.#readouts = readouts;
    this.#root.append(
      this.#badges,
      this.#badgesLeft,
      this.#badgesRight,
      readouts,
      this.#modes,
      this.#feed,
    );
    host.append(this.#root);
    this.setChrome(options.chrome);

    this.#onMove = () => this.#schedule();
    this.#map.on("move", this.#onMove);
  }

  /** Number of stored traces (bounded by the capacity). */
  get size(): number {
    return this.#store.size;
  }

  /** Re-home the chrome (#286): each part goes into the given container
   * (a shell's HUD cards / rail section) or back into the overlay root
   * where it floats over the map. Badges never move — they are positioned
   * in map pixels. Unit tests keep driving the in-map layout. */
  setChrome(chrome: XRayChrome | undefined): void {
    this.#chrome = chrome;
    const place = (element: HTMLElement | undefined, target: HTMLElement | undefined): void => {
      if (!element) {
        return;
      }
      if (target) {
        element.classList.add("swath-xray-docked");
        target.append(element);
      } else {
        element.classList.remove("swath-xray-docked");
        this.#root.append(element);
      }
    };
    place(this.#readouts, chrome?.readouts);
    place(this.#modes, chrome?.modes);
    place(this.#feed, chrome?.feed);
    // The analytics summary rides in the readouts unless the host gives it
    // its own place (the rail under view=xray).
    if (chrome?.analytics) {
      this.#analytics.element.classList.add("swath-xray-docked");
      chrome.analytics.append(this.#analytics.element);
    } else if (this.#readouts) {
      this.#analytics.element.classList.remove("swath-xray-docked");
      this.#readouts.insertBefore(this.#analytics.element, this.#ingest);
    }
    if (this.#inspector) {
      place(this.#inspector, chrome?.inspector);
    }
  }

  /** The stored trace for `"layer/z/x/y"`, if retained. */
  traceFor(key: string): TraceJson | undefined {
    return this.#store.get(key)?.trace;
  }

  /** (Re)opens the SSE stream. Idempotent per URL: reconnecting to the
   * URL already open is a no-op, so layer switches (which re-apply the
   * map style) never churn the connection. */
  connect(url: string): void {
    if (this.#disposed || url === this.#url) {
      return;
    }
    this.#source?.close();
    this.#url = url;
    const source = this.#createEventSource(url);
    source.addEventListener("trace", (event) => this.#onTrace(event.data));
    source.addEventListener("lagged", (event) => this.#onLagged(event.data));
    this.#source = source;
  }

  /** Which layer's traces to paint (badges filter on it; the store keeps
   * every layer so switching back repaints instantly). */
  /** The frame the host is viewing (its `datetime` attribute; null =
   * latest/unpinned): the analytics card's per-frame line follows it. */
  setFrame(datetime: string | null): void {
    this.#analytics.setViewedFrame(datetime);
  }

  setLayer(layer: string): void {
    if (layer !== this.#layer) {
      this.#layer = layer;
      this.#schedule();
    }
  }

  /**
   * (De)activates per-side badge painting (issue #210). A fraction-only
   * move (handle drag) just re-clips the side layers; a change of SIDES
   * purges the side-tagged entries — a badge matched against the old
   * comparison must never survive into the new one wearing the wrong
   * side. Plain entries (received outside any compare) are untouched, so
   * ending a compare repaints the normal view instantly.
   */
  setCompare(compare: XRayCompare | undefined): void {
    const previous = this.#compare;
    this.#compare = compare;
    const sidesChanged = JSON.stringify(previous?.sides) !== JSON.stringify(compare?.sides);
    if (sidesChanged) {
      for (const [key, entry] of this.#store) {
        if (entry.side !== undefined) {
          this.#store.delete(key);
        }
      }
    }
    if (compare) {
      const right = compare.fraction * 100;
      this.#badgesLeft.style.clipPath = `inset(0 ${100 - right}% 0 0)`;
      this.#badgesRight.style.clipPath = `inset(0 0 0 ${right}%)`;
    } else {
      this.#badgesLeft.style.clipPath = "";
      this.#badgesRight.style.clipPath = "";
    }
    if (sidesChanged) {
      this.#schedule();
    }
  }

  /** The current display mode. */
  get displayMode(): XRayDisplayMode {
    return this.#mode;
  }

  /** Switches what badges encode: `decision` colors, `bytes` heatmap,
   * or `off` (no badges). The mode-control buttons mirror it. */
  setDisplayMode(mode: XRayDisplayMode): void {
    if (mode === this.#mode) {
      return;
    }
    this.#mode = mode;
    for (const button of this.#modes.querySelectorAll("button")) {
      button.setAttribute("aria-pressed", String(button.dataset.mode === mode));
    }
    this.#schedule();
  }

  /** Synchronous repaint — the deterministic seam tests call instead of
   * waiting on the rAF throttle. */
  refresh(): void {
    if (this.#frame !== undefined) {
      cancelAnimationFrame(this.#frame);
      this.#frame = undefined;
    }
    this.#paint();
  }

  /** Tears the overlay down: closes the stream, detaches map listeners,
   * removes all DOM. The instance is dead afterwards. */
  dispose(): void {
    this.#disposed = true;
    this.#source?.close();
    this.#source = undefined;
    this.#map.off("move", this.#onMove);
    if (this.#frame !== undefined) {
      cancelAnimationFrame(this.#frame);
      this.#frame = undefined;
    }
    this.#closeInspector();
    this.#root.remove();
  }

  #onTrace(data: string): void {
    const envelope = parseTraceEnvelope(data);
    if (!envelope) {
      return; // malformed data is dropped, not fatal — the stream goes on
    }
    this.#onEnvelope?.(envelope);
    const { z, x, y } = envelope;
    // While comparing (issue #210), a trace that belongs to a side is
    // keyed BY that side, so both sides of one tile coexist in the store
    // (latest-wins would otherwise flicker between them). Everything
    // else — including compare-time traces that match neither side —
    // keeps the plain key and stays out of the per-side layers.
    const side = this.#compare
      ? traceSide(this.#compare.sides, envelope.layer, envelope.trace.temporal?.requested)
      : undefined;
    const key =
      side === undefined
        ? `${envelope.layer}/${envelope.tile}`
        : `${side}:${envelope.layer}/${envelope.tile}`;
    this.#store.delete(key); // latest wins, and re-insertion refreshes LRU order
    const entry: XRayEntry = { layer: envelope.layer, z, x, y, trace: envelope.trace };
    if (side !== undefined) {
      entry.side = side;
    }
    this.#store.set(key, entry);
    while (this.#store.size > this.#capacity) {
      const oldest = this.#store.keys().next().value;
      if (oldest === undefined) {
        break;
      }
      this.#store.delete(oldest);
    }
    if (envelope.trace.ingest_to_pixel_ms !== null) {
      // The server reports elapsed-since-ingest on EVERY render of a
      // granule-backed layer, so "latest" grows without bound as you pan.
      // The metric (REQUIREMENTS.md §3) is the FIRST render after arrival —
      // keep the minimum observed, which is exactly that.
      this.#ingestMs = Math.min(
        this.#ingestMs ?? Number.POSITIVE_INFINITY,
        envelope.trace.ingest_to_pixel_ms,
      );
      this.#ingest.textContent = `ingest→pixel: ${this.#ingestMs} ms`;
    }
    // The frame key feeds the analytics card's per-frame line (issue
    // #182) — scoped to the PAINTED layer, deliberately: the line
    // narrates the animation on screen ("this frame: N live, M cached"),
    // and since #223 every catalog-backed render on the shared stream
    // carries a temporal decision, so an unscoped key would follow
    // whatever background layer traced last. The stream-wide counters
    // above stay layer-agnostic, like the feed.
    // A `run_udf` cost rides the trace only when a UDF stage ran (ADR
    // 0018, #205): the deterministic fuel and the stage's wall-clock
    // share — the analytics card's per-tile UDF line (#208).
    const fuel = envelope.trace.udf_fuel_used;
    const udfMs = envelope.trace.timings.udf_ms;
    const udf =
      typeof fuel === "number" || udfMs !== undefined
        ? { tile: key, fuel: typeof fuel === "number" ? fuel : undefined, ms: udfMs ?? 0 }
        : undefined;
    this.#analytics.record(
      envelope.trace.decision,
      envelope.trace.timings.total_ms,
      envelope.layer === this.#layer ? envelope.trace.temporal?.granule_datetime : undefined,
      udf,
    );
    this.#feedTrace(key, envelope);
    this.#schedule();
  }

  #onLagged(data: string): void {
    let missed = 0;
    try {
      missed = Number((JSON.parse(data) as { missed?: number }).missed) || 0;
    } catch {
      return;
    }
    this.#missed += missed;
    this.#lagged.textContent = `missed ${this.#missed} traces`;
    this.#lagged.hidden = this.#missed === 0;
    const marker = document.createElement("li");
    marker.className = "swath-xray-feed-line-lagged";
    marker.textContent = `— missed ${missed} traces —`;
    this.#feedAppend(marker);
  }

  // --- The live trace-feed panel (bottom drawer) ---

  #toggleFeed(): void {
    const expand = this.#feedLines.hidden;
    this.#feedLines.hidden = !expand;
    this.#feedPause.hidden = !expand;
    this.#feedToggle.setAttribute("aria-expanded", String(expand));
    if (expand) {
      this.#feedLines.scrollTop = this.#feedLines.scrollHeight;
    }
  }

  #togglePause(): void {
    this.#feedPaused = !this.#feedPaused;
    this.#feedPause.textContent = this.#feedPaused ? "resume" : "pause";
    this.#feedPause.setAttribute(
      "aria-label",
      this.#feedPaused ? "Resume trace feed" : "Pause trace feed",
    );
    this.#feedPause.setAttribute("aria-pressed", String(this.#feedPaused));
    if (!this.#feedPaused) {
      const pending = this.#feedPending;
      this.#feedPending = [];
      for (const line of pending) {
        this.#feedLines.append(line);
      }
      this.#feedTrim();
      this.#feedLines.scrollTop = this.#feedLines.scrollHeight;
    }
  }

  /** One compact console line per received trace; clicking it reopens
   * the same inspector the badge click opens (and flashes the badge if
   * the tile is currently painted). */
  #feedTrace(key: string, envelope: TraceEnvelope): void {
    const { trace } = envelope;
    const line = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.key = key;
    button.dataset.decision = decisionKind(trace.decision);
    // A UDF render's fuel rides the line (#208), so the feed narrates
    // the deterministic cost the fuel axis bounds without a click.
    const fuel = typeof trace.udf_fuel_used === "number" ? ` fuel ${trace.udf_fuel_used}` : "";
    button.textContent =
      `${feedTime(new Date())} ${envelope.layer} ${envelope.tile} ` +
      `${decisionKind(trace.decision)} ${trace.timings.total_ms}ms ${formatKb(trace.bytes_read)}KB${fuel}`;
    button.setAttribute("aria-label", `Inspect trace for tile ${key}`);
    button.addEventListener("click", () => this.#revealTrace(key));
    line.append(button);
    this.#feedAppend(line);
  }

  /** Appends through the pause gate and the drop-oldest bound. While
   * paused, lines queue (bounded the same way) so the visible content
   * and scroll position hold still; resume flushes the queue. */
  #feedAppend(line: HTMLLIElement): void {
    if (this.#feedPaused) {
      this.#feedPending.push(line);
      while (this.#feedPending.length > FEED_CAPACITY) {
        this.#feedPending.shift();
        this.#feedDropped += 1;
      }
      this.#feedDroppedUpdate();
      return;
    }
    const stick =
      this.#feedLines.scrollTop + this.#feedLines.clientHeight >= this.#feedLines.scrollHeight - 4;
    this.#feedLines.append(line);
    this.#feedTrim();
    if (stick) {
      this.#feedLines.scrollTop = this.#feedLines.scrollHeight;
    }
  }

  #feedTrim(): void {
    while (this.#feedLines.children.length > FEED_CAPACITY) {
      this.#feedLines.firstElementChild?.remove();
      this.#feedDropped += 1;
    }
    this.#feedDroppedUpdate();
  }

  #feedDroppedUpdate(): void {
    this.#feedDroppedBadge.textContent = `${this.#feedDropped} dropped`;
    this.#feedDroppedBadge.hidden = this.#feedDropped === 0;
  }

  /** Feed-line click: open the inspector for the stored trace (evicted
   * entries are gone — the line then does nothing, honestly) and flash
   * the badge when the tile is on screen. */
  #revealTrace(key: string): void {
    const entry = this.#store.get(key);
    if (!entry) {
      return;
    }
    const nw = this.#map.project(tileNorthWest(entry.z, entry.x, entry.y));
    this.#openInspector(key, entry, nw);
    const badge = this.#root.querySelector(`.swath-xray-badge[data-key="${CSS.escape(key)}"]`);
    if (badge instanceof HTMLElement) {
      badge.classList.add("swath-xray-badge-flash");
      window.setTimeout(() => badge.classList.remove("swath-xray-badge-flash"), 1500);
    }
  }

  #schedule(): void {
    if (this.#frame !== undefined || this.#disposed) {
      return;
    }
    this.#frame = requestAnimationFrame(() => {
      this.#frame = undefined;
      this.#paint();
    });
  }

  /** The tile zoom currently displayed: a 256px raster source renders
   * z = style-zoom + 1 tiles at native size (MapLibre's world is 512px
   * at zoom 0), so that is the single level worth badging. */
  #displayZoom(): number {
    return Math.round(this.#map.getZoom()) + 1;
  }

  /** Non-zero bytes_read min/max across the whole store — the heatmap
   * scale's domain (whole store, not just painted badges, so the scale
   * holds still while panning across zooms — the comparison the heatmap
   * exists to make). */
  #bytesRange(): { min: number; max: number } {
    let min = Number.POSITIVE_INFINITY;
    let max = 0;
    for (const entry of this.#store.values()) {
      const bytes = entry.trace.bytes_read;
      if (bytes > 0) {
        min = Math.min(min, bytes);
        max = Math.max(max, bytes);
      }
    }
    return max > 0 ? { min, max } : { min: 0, max: 0 };
  }

  #paint(): void {
    if (this.#disposed) {
      return;
    }
    this.#badges.replaceChildren();
    this.#badgesLeft.replaceChildren();
    this.#badgesRight.replaceChildren();
    const bytes = this.#mode === "bytes" ? this.#bytesRange() : undefined;
    this.#scale.hidden = bytes === undefined;
    if (bytes) {
      if (bytes.max > 0) {
        this.#scaleRange.textContent = `${formatBytes(bytes.min)} – ${formatBytes(bytes.max)}`;
        this.#scale.dataset.min = String(bytes.min);
        this.#scale.dataset.max = String(bytes.max);
      } else {
        this.#scaleRange.textContent = "—";
        delete this.#scale.dataset.min;
        delete this.#scale.dataset.max;
      }
    }
    if (this.#mode === "off") {
      return;
    }
    const zTile = this.#displayZoom();
    const width = this.#root.clientWidth;
    const height = this.#root.clientHeight;
    const comparing = this.#compare !== undefined;
    for (const [key, entry] of this.#store) {
      // Comparing: only side-tagged entries paint, each into its clipped
      // layer (the side already pins the entry's layer — the sides ARE
      // layer filters in layer mode, and the one shown layer in date
      // mode). Not comparing: only plain entries of the active layer.
      if (comparing ? entry.side === undefined : entry.side !== undefined) {
        continue;
      }
      if ((!comparing && entry.layer !== this.#layer) || entry.z !== zTile) {
        continue;
      }
      const nw = this.#map.project(tileNorthWest(entry.z, entry.x, entry.y));
      const se = this.#map.project(tileNorthWest(entry.z, entry.x + 1, entry.y + 1));
      if (width > 0 && height > 0 && (se.x < 0 || se.y < 0 || nw.x > width || nw.y > height)) {
        continue; // off-viewport
      }
      const badge = this.#badge(key, entry, nw, se, bytes);
      if (entry.side === undefined) {
        this.#badges.append(badge);
      } else {
        badge.dataset.side = entry.side;
        (entry.side === "left" ? this.#badgesLeft : this.#badgesRight).append(badge);
      }
    }
  }

  #badge(
    key: string,
    entry: XRayEntry,
    nw: { x: number; y: number },
    se: { x: number; y: number },
    bytesRange?: { min: number; max: number },
  ): HTMLButtonElement {
    const { trace } = entry;
    const kind = decisionKind(trace.decision);
    const badge = document.createElement("button");
    badge.type = "button";
    badge.className = "swath-xray-badge";
    badge.dataset.key = key;
    badge.dataset.decision = kind;
    badge.dataset.totalMs = String(trace.timings.total_ms);
    badge.dataset.bytes = String(trace.bytes_read);
    badge.style.left = `${nw.x}px`;
    badge.style.top = `${nw.y}px`;
    badge.style.width = `${se.x - nw.x}px`;
    badge.style.height = `${se.y - nw.y}px`;
    if (bytesRange) {
      const bucket = bytesBucket(trace.bytes_read, bytesRange.min, bytesRange.max);
      const colors = bucket === 0 ? this.#zero : (this.#ramp[bucket - 1] ?? this.#zero);
      badge.dataset.bytesBucket = String(bucket);
      badge.style.borderColor = colors.border;
      badge.style.backgroundColor = colors.tint;
      if (bucket === 0) {
        badge.style.borderStyle = "dashed";
      }
    } else {
      const colors = this.#decisions[kind];
      badge.style.borderColor = colors.border;
      badge.style.backgroundColor = colors.tint;
    }
    badge.setAttribute("aria-label", `Trace for tile ${key}: ${decisionLabel(trace.decision)}`);
    const label = document.createElement("span");
    label.textContent = `${trace.timings.total_ms} ms · ${formatKb(trace.bytes_read)} KB`;
    badge.append(label);
    badge.addEventListener("click", () => this.#openInspector(key, entry, nw));
    return badge;
  }

  #openInspector(key: string, entry: XRayEntry, at: { x: number; y: number }): void {
    this.#closeInspector();
    const { trace } = entry;
    const dialog = document.createElement("section");
    dialog.className = "swath-xray-inspector";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-label", `Trace for tile ${key}`);
    dialog.tabIndex = -1;
    dialog.style.left = `${Math.max(0, Math.min(at.x, this.#root.clientWidth - 340))}px`;
    dialog.style.top = `${Math.max(0, at.y)}px`;

    const header = document.createElement("header");
    const title = document.createElement("span");
    title.textContent = key;
    const close = document.createElement("button");
    close.type = "button";
    close.textContent = "×";
    close.setAttribute("aria-label", "Close trace inspector");
    close.addEventListener("click", () => this.#closeInspector());
    header.append(title, close);

    const facts = document.createElement("dl");
    const fact = (term: string, value: string): void => {
      const dt = document.createElement("dt");
      dt.textContent = term;
      const dd = document.createElement("dd");
      dd.textContent = value;
      facts.append(dt, dd);
    };
    fact("decision", decisionLabel(trace.decision));
    fact("bytes read", `${trace.bytes_read} (${formatKb(trace.bytes_read)} KB)`);
    const { timings } = trace;
    // The UDF stage's share of pixel time, shown only when one ran.
    const udf = timings.udf_ms === undefined ? "" : ` · udf ${timings.udf_ms}`;
    fact(
      "timings",
      `read ${timings.read_ms} · warp ${timings.warp_ms} · pixel ${timings.pixel_ops_ms}${udf} · ` +
        `encode ${timings.encode_ms} · total ${timings.total_ms} ms`,
    );
    if (typeof trace.udf_fuel_used === "number") {
      // The deterministic cost the layer budget's fuel axis bounds (#205).
      fact("udf fuel", `${trace.udf_fuel_used}`);
    }
    fact("crs", `${trace.crs_from} → ${trace.crs_to}`);
    if (trace.temporal) {
      // The frame's provenance in time (#182): which granule backs it,
      // under which resolution rule — absent entirely on static layers.
      fact(
        "frame",
        `${trace.temporal.granule_datetime} (${trace.temporal.granule_id}, ${trace.temporal.rule})`,
      );
      // A joined frame (ADR 0022, #301): one line per branch, so the
      // badge explains both granules a pixel came from.
      for (const source of trace.temporal.sources ?? []) {
        fact(`branch ${source.node}`, `${source.granule_datetime} (${source.granule_id})`);
      }
    }
    fact("sources", trace.sources.join(", "));
    if (trace.ingest_to_pixel_ms !== null) {
      fact("ingest→pixel", `${trace.ingest_to_pixel_ms} ms`);
    }

    const provenanceTitle = document.createElement("dt");
    provenanceTitle.textContent = `provenance (${trace.provenance.length} ranges)`;
    const provenanceValue = document.createElement("dd");
    const ranges = document.createElement("ul");
    ranges.className = "swath-xray-provenance";
    for (const range of trace.provenance) {
      const item = document.createElement("li");
      item.textContent = `${range.path} @${range.offset} +${range.length}`;
      ranges.append(item);
    }
    provenanceValue.append(ranges);
    facts.append(provenanceTitle, provenanceValue);

    dialog.append(header, facts);
    const plan = trace.plan;
    if (plan && Array.isArray(plan.considered)) {
      dialog.append(...this.#planSection(plan));
    }
    dialog.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        this.#closeInspector();
      }
    });
    if (this.#chrome?.inspector) {
      dialog.classList.add("swath-xray-docked");
      this.#chrome.inspector.append(dialog);
    } else {
      this.#root.append(dialog);
    }
    this.#inspector = dialog;
    dialog.focus();
  }

  /** The why-view (#42): the planner's chosen strategy plus every
   * candidate it weighed, as a real table (column headers carry the
   * semantics for AT). Absent entirely when the trace has no plan (old
   * or synthetic traces) — an empty section would imply "nothing was
   * considered", which is not what `null` means. */
  #planSection(plan: PlanTrace): [HTMLHeadingElement, HTMLTableElement] {
    const title = document.createElement("h3");
    title.className = "swath-xray-plan-title";
    title.textContent = `planner — chose ${plannedLabel(plan.chosen)}`;
    const table = document.createElement("table");
    table.className = "swath-xray-plan";
    table.setAttribute("aria-label", "Planner candidates considered");
    const head = document.createElement("thead");
    const headRow = document.createElement("tr");
    for (const column of ["strategy", "est. cost", "ok", "reason"]) {
      const th = document.createElement("th");
      th.scope = "col";
      th.textContent = column;
      headRow.append(th);
    }
    head.append(headRow);
    const body = document.createElement("tbody");
    const chosenLabel = plannedLabel(plan.chosen);
    for (const candidate of plan.considered) {
      const row = document.createElement("tr");
      const label = plannedLabel(candidate.strategy);
      const chosen = label === chosenLabel;
      row.dataset.chosen = String(chosen);
      row.dataset.admissible = String(candidate.admissible);
      const cell = (text: string): void => {
        const td = document.createElement("td");
        td.textContent = text;
        row.append(td);
      };
      // "✓ chosen" is a text marker, deliberately: the green highlight
      // must never be the only way to tell the winner apart.
      cell(chosen ? `${label} ✓ chosen` : label);
      cell(formatBytes(candidate.estimated_cost_bytes));
      cell(candidate.admissible ? "yes" : "no");
      cell(candidate.reason);
      body.append(row);
    }
    table.append(head, body);
    return [title, table];
  }

  #closeInspector(): void {
    this.#inspector?.remove();
    this.#inspector = undefined;
  }
}

/**
 * The trace stream without badges (#287): the status bar's ingest→pixel
 * cell needs envelopes whether or not the x-ray is on. Same parser, same
 * idempotent `connect(url)`; the host shares the overlay's stream while
 * one exists and opens this one otherwise.
 */
export class TraceStream {
  readonly #createEventSource: EventSourceFactory;
  readonly #onEnvelope: (envelope: TraceEnvelope) => void;
  #source: EventSourceLike | undefined;
  #url = "";

  constructor(
    createEventSource: EventSourceFactory,
    onEnvelope: (envelope: TraceEnvelope) => void,
  ) {
    this.#createEventSource = createEventSource;
    this.#onEnvelope = onEnvelope;
  }

  get url(): string {
    return this.#url;
  }

  connect(url: string): void {
    if (url === this.#url) {
      return;
    }
    this.#source?.close();
    this.#url = url;
    const source = this.#createEventSource(url);
    source.addEventListener("trace", (event) => {
      const envelope = parseTraceEnvelope(event.data);
      if (envelope) {
        this.#onEnvelope(envelope);
      }
    });
    this.#source = source;
  }

  dispose(): void {
    this.#source?.close();
    this.#source = undefined;
    this.#url = "";
  }
}
