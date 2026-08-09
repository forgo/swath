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
 * `event: lagged` carries `{"missed":N}` — surfaced as a badge, because
 * the stream is best-effort telemetry and drops must be visible, not
 * silent. Keepalive comments and reconnects are `EventSource`'s problem.
 */

/** The planner's decision, as the pinned `Trace` JSON serializes it:
 * unit variants as strings, `overview` externally tagged. */
export type TraceDecision = "live" | "cache_hit" | { overview: { level: number } };

/** Per-stage wall-clock timings (`Timings`, pinned in swath-core). */
export interface TraceTimings {
  read_ms: number;
  warp_ms: number;
  pixel_ops_ms: number;
  encode_ms: number;
  total_ms: number;
}

/** One byte range read from a source (`Provenance`, pinned). */
export interface TraceProvenance {
  path: string;
  offset: number;
  length: number;
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
}

/** The `data:` payload of one `event: trace` (envelope pinned in
 * swath-api `traces`): `tile` is XYZ-ordered `"z/x/y"`. */
export interface TraceEnvelope {
  tile: string;
  layer: string;
  trace: TraceJson;
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

/** One stored trace: parsed tile address + the envelope payload. */
interface XRayEntry {
  layer: string;
  z: number;
  x: number;
  y: number;
  trace: TraceJson;
}

/** Store bound: per-tile latest-wins entries kept, least recently
 * *updated* evicted first. ~500 covers several screenfuls across a pan
 * without unbounded growth on a long-lived page. */
const DEFAULT_CAPACITY = 500;

const STYLE_ELEMENT_ID = "swath-xray-styles";

/** Decision palette: live = green (the "rendered fresh for you" color).
 * Overview (amber) and cache_hit (blue) are reserved here so the colors
 * are contractual before #36+ (overviews, cache) can produce them. */
const DECISION_COLORS: Record<"live" | "overview" | "cache_hit", { border: string; tint: string }> =
  {
    live: { border: "#16a34a", tint: "rgb(22 163 74 / 12%)" },
    overview: { border: "#d97706", tint: "rgb(217 119 6 / 12%)" },
    cache_hit: { border: "#2563eb", tint: "rgb(37 99 235 / 12%)" },
  };

const OVERLAY_CSS = `
.swath-xray { position: absolute; inset: 0; overflow: hidden; pointer-events: none; }
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
  font: 11px/1.4 ui-monospace, monospace;
  color: #fff;
  cursor: pointer;
  text-align: left;
}
.swath-xray-badge > span {
  padding: 1px 4px;
  background: rgb(0 0 0 / 65%);
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
  background: rgb(0 0 0 / 75%);
  color: #4ade80;
  font: 700 14px/1.5 ui-monospace, monospace;
}
.swath-xray-lagged {
  padding: 2px 8px;
  border-radius: 4px;
  background: rgb(153 27 27 / 90%);
  color: #fff;
  font: 12px/1.5 ui-monospace, monospace;
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
  background: rgb(15 23 42 / 95%);
  color: #e2e8f0;
  font: 12px/1.5 ui-monospace, monospace;
}
.swath-xray-inspector:focus { outline: 2px solid #4ade80; }
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
.swath-xray-inspector dt { color: #94a3b8; margin-top: 4px; }
.swath-xray-inspector dd { margin: 0; }
.swath-xray-inspector .swath-xray-provenance {
  max-height: 9em;
  overflow: auto;
  margin: 0;
  padding-left: 1em;
}
.swath-xray-inspector .swath-xray-provenance li { word-break: break-all; }
`;

function injectStyles(doc: Document): void {
  if (doc.getElementById(STYLE_ELEMENT_ID)) {
    return;
  }
  const style = doc.createElement("style");
  style.id = STYLE_ELEMENT_ID;
  style.textContent = OVERLAY_CSS;
  doc.head.append(style);
}

/** The decision's flat kind — what the badge color and `data-decision`
 * carry (`{"overview":{...}}` collapses to `"overview"`). */
export function decisionKind(decision: TraceDecision): "live" | "cache_hit" | "overview" {
  return typeof decision === "string" ? decision : "overview";
}

function decisionLabel(decision: TraceDecision): string {
  return typeof decision === "string" ? decision : `overview (level ${decision.overview.level})`;
}

/**
 * Northwest corner of Web Mercator tile `z/x/y` in lon/lat degrees (the
 * standard slippy-map inverse). The southeast corner is the northwest of
 * `z/x+1/y+1`.
 */
export function tileNorthWest(z: number, x: number, y: number): [number, number] {
  const n = 2 ** z;
  const lon = (x / n) * 360 - 180;
  const lat = (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / n))) * 180) / Math.PI;
  return [lon, lat];
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

function formatKb(bytes: number): string {
  const kb = bytes / 1024;
  return kb >= 100 ? String(Math.round(kb)) : kb.toFixed(1);
}

/** The overlay engine: SSE client + bounded per-tile store + DOM paint.
 * `<swath-map>` owns one while its `xray` attribute is present. */
export class XRayOverlay {
  readonly #map: XRayMapLike;
  readonly #root: HTMLDivElement;
  readonly #badges: HTMLDivElement;
  readonly #ingest: HTMLDivElement;
  readonly #lagged: HTMLDivElement;
  readonly #createEventSource: EventSourceFactory;
  readonly #capacity: number;
  readonly #onMove: () => void;

  /** Latest trace per tile, keyed `"layer/z/x/y"`. `Map` iteration order
   * is insertion order and updates delete-then-set, so the first key is
   * always the least recently updated — the LRU eviction victim. */
  readonly #store = new Map<string, XRayEntry>();

  #source: EventSourceLike | undefined;
  #url: string | undefined;
  #layer = "";
  #missed = 0;
  #ingestMs: number | undefined;
  #frame: number | undefined;
  #inspector: HTMLElement | undefined;
  #disposed = false;

  constructor(
    host: HTMLElement,
    map: XRayMapLike,
    options: { createEventSource?: EventSourceFactory | undefined; capacity?: number } = {},
  ) {
    this.#map = map;
    this.#createEventSource = options.createEventSource ?? ((url) => new EventSource(url));
    this.#capacity = options.capacity ?? DEFAULT_CAPACITY;
    injectStyles(host.ownerDocument);

    this.#root = document.createElement("div");
    this.#root.className = "swath-xray";
    this.#badges = document.createElement("div");
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
    readouts.append(this.#lagged, this.#ingest);
    this.#root.append(this.#badges, readouts);
    host.append(this.#root);

    this.#onMove = () => this.#schedule();
    this.#map.on("move", this.#onMove);
  }

  /** Number of stored traces (bounded by the capacity). */
  get size(): number {
    return this.#store.size;
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
  setLayer(layer: string): void {
    if (layer !== this.#layer) {
      this.#layer = layer;
      this.#schedule();
    }
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
    let envelope: TraceEnvelope;
    try {
      envelope = JSON.parse(data) as TraceEnvelope;
    } catch {
      return; // malformed data is dropped, not fatal — the stream goes on
    }
    const tile = parseTile(envelope.tile);
    if (!tile || typeof envelope.layer !== "string" || typeof envelope.trace !== "object") {
      return;
    }
    const key = `${envelope.layer}/${envelope.tile}`;
    this.#store.delete(key); // latest wins, and re-insertion refreshes LRU order
    this.#store.set(key, { layer: envelope.layer, ...tile, trace: envelope.trace });
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

  #paint(): void {
    if (this.#disposed) {
      return;
    }
    this.#badges.replaceChildren();
    const zTile = this.#displayZoom();
    const width = this.#root.clientWidth;
    const height = this.#root.clientHeight;
    for (const [key, entry] of this.#store) {
      if (entry.layer !== this.#layer || entry.z !== zTile) {
        continue;
      }
      const nw = this.#map.project(tileNorthWest(entry.z, entry.x, entry.y));
      const se = this.#map.project(tileNorthWest(entry.z, entry.x + 1, entry.y + 1));
      if (width > 0 && height > 0 && (se.x < 0 || se.y < 0 || nw.x > width || nw.y > height)) {
        continue; // off-viewport
      }
      this.#badges.append(this.#badge(key, entry, nw, se));
    }
  }

  #badge(
    key: string,
    entry: XRayEntry,
    nw: { x: number; y: number },
    se: { x: number; y: number },
  ): HTMLButtonElement {
    const { trace } = entry;
    const kind = decisionKind(trace.decision);
    const colors = DECISION_COLORS[kind];
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
    badge.style.borderColor = colors.border;
    badge.style.backgroundColor = colors.tint;
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
    fact(
      "timings",
      `read ${timings.read_ms} · warp ${timings.warp_ms} · pixel ${timings.pixel_ops_ms} · ` +
        `encode ${timings.encode_ms} · total ${timings.total_ms} ms`,
    );
    fact("crs", `${trace.crs_from} → ${trace.crs_to}`);
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
    dialog.addEventListener("keydown", (event) => {
      if (event.key === "Escape") {
        this.#closeInspector();
      }
    });
    this.#root.append(dialog);
    this.#inspector = dialog;
    dialog.focus();
  }

  #closeInspector(): void {
    this.#inspector?.remove();
    this.#inspector = undefined;
  }
}
