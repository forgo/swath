// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * View state for the entry page (issue #108): the URL is the shareable
 * representation of what the viewer shows, localStorage remembers the
 * last session, and URL params always beat storage.
 *
 * Pure module in the ADR 0005 spirit (own state/routing, no framework):
 * parse/format/compare/persist only — no DOM, no map. The app shell
 * (demo/main.ts) owns the wiring; this module owns the semantics, so the
 * precedence rule is unit-testable without a browser history.
 *
 * Byte-stability contract: a deep-link URL is never rewritten on load.
 * The shell only writes the URL on user interaction, and even then skips
 * the write when the URL already encodes the same state
 * ([`viewStatesEqual`]) — pasted links survive byte-for-byte.
 */

/** What a view is: which layer, where, when, and whether x-ray is on. */
export interface ViewState {
  /** Layer id; absent means "the server's first layer" (zero-config). */
  layer?: string;
  /** `[lon, lat]` view center; absent means "fit the layer's bounds". */
  center?: [number, number];
  /** View zoom; absent means "fit the layer's bounds". */
  zoom?: number;
  /** The viewed frame's `datetime=` instant (RFC 3339 UTC, issue #182);
   * absent means "latest" — a layer without a time dimension. */
  time?: string;
  /** Compare swipe, date-vs-date (issue #210): the right side's frame
   * (`ct=`, same RFC 3339 grammar as `time`); the left side is `time`.
   * Mutually exclusive with `compareLayer` — an ambiguous URL carrying
   * both degrades to "no compare". */
  compareTime?: string;
  /** Compare swipe, layer-vs-layer (issue #210): the right side's layer
   * id (`cl=`); the left side is `layer`. Comparing a layer with itself
   * is dropped at parse. Mutually exclusive with `compareTime`. */
  compareLayer?: string;
  /** The swipe handle position (`swipe=`), fraction 0..1 of the map's
   * width; only meaningful (parsed, written) while a compare is active.
   * Absent means the centered default. */
  swipe?: number;
  /** Whether the x-ray overlay is enabled. */
  xray: boolean;
}

/** Where an initial state came from — pinned by the precedence tests. */
export type ViewStateSource = "url" | "storage" | "default";

/** The query params this module owns (everything else passes through). */
const OWNED_PARAMS = ["layer", "center", "zoom", "t", "ct", "cl", "swipe", "xray"] as const;

/** localStorage key; versioned so a future shape change can't misparse. */
export const STORAGE_KEY = "swath.view-state.v1";

/** Coordinate precision in the URL and storage: 5 decimals ≈ 1 m. */
const CENTER_DECIMALS = 5;
/** Zoom precision: 2 decimals is finer than any visible difference. */
const ZOOM_DECIMALS = 2;
/** Swipe precision: 2 decimals (1% of the map width) is finer than a
 * deliberate handle move. */
const SWIPE_DECIMALS = 2;

/** Equality tolerances, strictly looser than the write precision above so
 * a state round-tripped through its own URL always compares equal. */
const CENTER_EPSILON = 10 ** -(CENTER_DECIMALS - 1);
const ZOOM_EPSILON = 10 ** -(ZOOM_DECIMALS - 1);
/** Just above the swipe write precision's half-step (0.005): round trips
 * compare equal, while any deliberate handle move (≥ 1% of the width)
 * still reads as a different state. */
const SWIPE_EPSILON = 0.01;

/** Parses `"lon,lat"`; undefined when absent or malformed. */
export function parseCenter(value: string | null): [number, number] | undefined {
  if (value === null) {
    return undefined;
  }
  const parts = value.split(",").map((part) => Number(part.trim()));
  const [lon, lat] = parts;
  if (parts.length !== 2 || lon === undefined || lat === undefined) {
    return undefined;
  }
  return Number.isFinite(lon) && Number.isFinite(lat) ? [lon, lat] : undefined;
}

/** The `t` param's grammar: an RFC 3339 UTC (`Z`) instant — exactly what
 * the tile route's `datetime=` accepts as an instant (ADR 0015), and an
 * alphabet of URL-safe characters, so a validated value can be written
 * into the query string verbatim (hand-written deep-link style: no
 * percent-encoded colons) without ever smuggling a `&` or `#`. */
const TIME_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/;

/** Parses the `t` param; undefined when absent or not an RFC 3339 UTC
 * instant (malformed values degrade to "latest", never break the page). */
export function parseTime(value: string | null): string | undefined {
  if (value === null) {
    return undefined;
  }
  return TIME_PATTERN.test(value) ? value : undefined;
}

/** Parses a numeric param; undefined when absent or malformed. */
export function parseNumber(value: string | null): number | undefined {
  if (value === null || value.trim() === "") {
    return undefined;
  }
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : undefined;
}

/** Fixed-precision decimal with trailing zeros (and a bare `.`) trimmed —
 * `-106.00000` → `-106`, `39.30000` → `39.3`. */
function trimmed(value: number, decimals: number): string {
  return value
    .toFixed(decimals)
    .replace(/(\.\d*?)0+$/, "$1")
    .replace(/\.$/, "");
}

/** Canonical `"lon,lat"` for URLs, attributes, and storage. */
export function formatCenter(center: [number, number]): string {
  return `${trimmed(center[0], CENTER_DECIMALS)},${trimmed(center[1], CENTER_DECIMALS)}`;
}

/** Canonical zoom string. */
export function formatZoom(zoom: number): string {
  return trimmed(zoom, ZOOM_DECIMALS);
}

/** Canonical swipe-fraction string for URLs, attributes, and storage. */
export function formatSwipe(swipe: number): string {
  return trimmed(swipe, SWIPE_DECIMALS);
}

/** Parses a swipe fraction: a finite number in [0, 1]; undefined when
 * absent or out of range (out-of-range degrades to the default handle
 * position, never a half-off-screen handle). */
export function parseSwipe(value: string | null): number | undefined {
  const parsed = parseNumber(value);
  if (parsed === undefined || parsed < 0 || parsed > 1) {
    return undefined;
  }
  return parsed;
}

/** True when the query string carries any view param this module owns —
 * the trigger for "URL beats storage". */
export function hasViewParams(search: string): boolean {
  const params = new URLSearchParams(search);
  return OWNED_PARAMS.some((name) => params.has(name));
}

/** The view state a query string encodes (`?layer=…&center=…&zoom=…&xray`). */
export function parseViewState(search: string): ViewState {
  const params = new URLSearchParams(search);
  const state: ViewState = { xray: params.has("xray") };
  const layer = params.get("layer");
  if (layer !== null && layer !== "") {
    state.layer = layer;
  }
  const center = parseCenter(params.get("center"));
  if (center) {
    state.center = center;
  }
  const zoom = parseNumber(params.get("zoom"));
  if (zoom !== undefined) {
    state.zoom = zoom;
  }
  const time = parseTime(params.get("t"));
  if (time !== undefined) {
    state.time = time;
  }
  // Compare (issue #210): the two modes are exclusive, so a URL carrying
  // both `ct` and `cl` is ambiguous and degrades to "no compare" — as
  // does comparing a layer with itself. `swipe` rides along only when a
  // compare is actually active (a stray handle position means nothing).
  const compareTime = parseTime(params.get("ct"));
  const compareLayerRaw = params.get("cl");
  const compareLayer =
    compareLayerRaw === null || compareLayerRaw === "" ? undefined : compareLayerRaw;
  if (compareTime !== undefined && compareLayer === undefined) {
    state.compareTime = compareTime;
  } else if (
    compareLayer !== undefined &&
    compareTime === undefined &&
    compareLayer !== state.layer
  ) {
    state.compareLayer = compareLayer;
  }
  if (state.compareTime !== undefined || state.compareLayer !== undefined) {
    const swipe = parseSwipe(params.get("swipe"));
    if (swipe !== undefined) {
      state.swipe = swipe;
    }
  }
  return state;
}

/**
 * The canonical query string for `state`, preserving every param this
 * module does not own (e.g. `basemap`) exactly where the current search
 * put it. Returns `""` for a default state with no foreign params — so a
 * bare `/` stays a bare `/`. The `xray` flag is written valueless
 * (`&xray`), matching the hand-written deep-link style.
 */
export function withViewState(search: string, state: ViewState): string {
  const foreign = new URLSearchParams(search);
  for (const name of OWNED_PARAMS) {
    foreign.delete(name);
  }
  const parts: string[] = [];
  if (state.layer !== undefined) {
    parts.push(`layer=${encodeURIComponent(state.layer)}`);
  }
  if (state.center) {
    parts.push(`center=${formatCenter(state.center)}`);
  }
  if (state.zoom !== undefined) {
    parts.push(`zoom=${formatZoom(state.zoom)}`);
  }
  if (state.time !== undefined) {
    // Written verbatim: the value has already passed TIME_PATTERN (only
    // validated times enter a ViewState), whose alphabet is URL-safe —
    // so the deep link keeps the human-readable `t=2024-08-16T19:03:00Z`.
    parts.push(`t=${state.time}`);
  }
  // Compare (issue #210): `ct` verbatim for the same TIME_PATTERN
  // reason as `t`; `swipe` only ever rides an active compare.
  if (state.compareTime !== undefined) {
    parts.push(`ct=${state.compareTime}`);
  } else if (state.compareLayer !== undefined) {
    parts.push(`cl=${encodeURIComponent(state.compareLayer)}`);
  }
  if (
    state.swipe !== undefined &&
    (state.compareTime !== undefined || state.compareLayer !== undefined)
  ) {
    parts.push(`swipe=${formatSwipe(state.swipe)}`);
  }
  if (state.xray) {
    parts.push("xray");
  }
  const rest = foreign.toString();
  if (rest !== "") {
    parts.push(rest);
  }
  return parts.length === 0 ? "" : `?${parts.join("&")}`;
}

/** Semantic equality within the write precision — the "don't rewrite a
 * URL that already says this" guard behind byte-stable deep links. */
export function viewStatesEqual(a: ViewState, b: ViewState): boolean {
  if (a.layer !== b.layer || a.xray !== b.xray || a.time !== b.time) {
    return false;
  }
  if (a.compareTime !== b.compareTime || a.compareLayer !== b.compareLayer) {
    return false;
  }
  if ((a.swipe === undefined) !== (b.swipe === undefined)) {
    return false;
  }
  if (
    a.swipe !== undefined &&
    b.swipe !== undefined &&
    Math.abs(a.swipe - b.swipe) > SWIPE_EPSILON
  ) {
    return false;
  }
  if ((a.center === undefined) !== (b.center === undefined)) {
    return false;
  }
  if (a.center && b.center) {
    if (
      Math.abs(a.center[0] - b.center[0]) > CENTER_EPSILON ||
      Math.abs(a.center[1] - b.center[1]) > CENTER_EPSILON
    ) {
      return false;
    }
  }
  if ((a.zoom === undefined) !== (b.zoom === undefined)) {
    return false;
  }
  if (a.zoom !== undefined && b.zoom !== undefined && Math.abs(a.zoom - b.zoom) > ZOOM_EPSILON) {
    return false;
  }
  return true;
}

/**
 * The initial view for a page load — THE precedence rule (issue #108):
 * any owned URL param present → the URL alone wins (storage ignored, no
 * merging: a shared link must show the same view everywhere); otherwise
 * the stored last session; otherwise the zero-config default.
 */
export function resolveInitialState(
  search: string,
  storage: Storage | undefined,
): { state: ViewState; source: ViewStateSource } {
  if (hasViewParams(search)) {
    return { state: parseViewState(search), source: "url" };
  }
  const stored = storage ? loadViewState(storage) : undefined;
  if (stored) {
    return { state: stored, source: "storage" };
  }
  return { state: { xray: false }, source: "default" };
}

/** Persists `state` as the last session; storage failures (quota, private
 * mode) are deliberately silent — persistence is a nicety, never a fault. */
export function saveViewState(storage: Storage, state: ViewState): void {
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Best-effort by design.
  }
}

/** The stored last session, or undefined when absent or malformed (a
 * corrupt entry must never break the page — it just loses the restore). */
export function loadViewState(storage: Storage): ViewState | undefined {
  let raw: string | null;
  try {
    raw = storage.getItem(STORAGE_KEY);
  } catch {
    return undefined;
  }
  if (raw === null) {
    return undefined;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return undefined;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return undefined;
  }
  const record = parsed as Record<string, unknown>;
  const state: ViewState = { xray: record["xray"] === true };
  if (typeof record["layer"] === "string" && record["layer"] !== "") {
    state.layer = record["layer"];
  }
  const center = record["center"];
  if (
    Array.isArray(center) &&
    center.length === 2 &&
    typeof center[0] === "number" &&
    typeof center[1] === "number" &&
    Number.isFinite(center[0]) &&
    Number.isFinite(center[1])
  ) {
    state.center = [center[0], center[1]];
  }
  if (typeof record["zoom"] === "number" && Number.isFinite(record["zoom"])) {
    state.zoom = record["zoom"];
  }
  if (typeof record["time"] === "string") {
    const time = parseTime(record["time"]);
    if (time !== undefined) {
      state.time = time;
    }
  }
  // Compare (issue #210): the same exclusivity and validation as the URL
  // path — a corrupt or ambiguous stored compare just loses the restore.
  const compareTime =
    typeof record["compareTime"] === "string" ? parseTime(record["compareTime"]) : undefined;
  const compareLayer =
    typeof record["compareLayer"] === "string" && record["compareLayer"] !== ""
      ? record["compareLayer"]
      : undefined;
  if (compareTime !== undefined && compareLayer === undefined) {
    state.compareTime = compareTime;
  } else if (
    compareLayer !== undefined &&
    compareTime === undefined &&
    compareLayer !== state.layer
  ) {
    state.compareLayer = compareLayer;
  }
  if (
    (state.compareTime !== undefined || state.compareLayer !== undefined) &&
    typeof record["swipe"] === "number" &&
    Number.isFinite(record["swipe"]) &&
    record["swipe"] >= 0 &&
    record["swipe"] <= 1
  ) {
    state.swipe = record["swipe"];
  }
  return state;
}

/** `window.localStorage`, or undefined where touching it throws (storage
 * disabled): the whole persistence feature then degrades to no-op. */
export function safeLocalStorage(): Storage | undefined {
  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}
