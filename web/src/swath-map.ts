// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * `<swath-map>` — the Swath viewer: MapLibre GL over a Swath OGC API -
 * Tiles surface (ADR 0005, issue #33).
 *
 * Plain Custom Element, light DOM, `observedAttributes` reactivity, no
 * framework. MapLibre GL is the single production dependency; its CSS is
 * injected into the document once by the component, so consumers need
 * zero setup beyond defining the element.
 *
 * Attributes (all observed and reactive):
 * - `server` — base URL of a Swath API (default: same origin).
 * - `layer`  — initial layer id (default: first entry of `/tilesets`).
 * - `center` — `"lon,lat"` initial view center.
 * - `zoom`   — initial zoom.
 * - `switcher` — `"off"` omits the built-in layer-switcher control
 *   (read at connect time only): for hosts that provide their own layer
 *   UI, like the entry page's `<swath-layer-panel>` (issue #108).
 * - `xray`   — presence toggles the x-ray overlay (issue #34): per-tile
 *   decisions/timings from the `/traces` SSE stream, painted by the
 *   built-in [`XRayOverlay`] module (see swath-xray.ts for the design
 *   rationale). A toggle button control mirrors the attribute.
 * - `datetime` — the viewed frame (issue #182, ADR 0015): an RFC 3339
 *   UTC instant appended to tile requests as `datetime=`; changing it
 *   re-points the raster source in place (no style rebuild, no bounds
 *   refit). The built-in [`TimeSlider`] control mirrors it: its domain
 *   comes from the granules listing the layer's tileset metadata links
 *   (`rel: granules`, catalog-backed layers only), and it stays hidden
 *   for layers with fewer than two acquisition dates.
 * - `compare-datetime` / `compare-layer` — the compare swipe (issue
 *   #210): a second, view-synced map clipped to the right of a drag
 *   handle, comparing the viewed frame/layer against another `datetime=`
 *   instant (date-vs-date) or another layer (layer-vs-layer). Mutually
 *   exclusive; both at once (or comparing a layer with itself) degrades
 *   to no compare. A `compare` toggle control offers the one-button
 *   before-vs-after default on layers with a time series.
 * - `swipe` — the handle position, fraction 0..1 (default 0.5); dragging
 *   the handle reflects back into the attribute, the source of truth.
 *
 * When neither `center` nor `zoom` is given, the view fits the layer's
 * geographic bounds from `/tilesets/{id}` metadata — a bare
 * `<swath-map>` against a live server is a working demo.
 *
 * Finding the data (issue #182 follow-up): a `zoom to data` control
 * frames the active layer's data footprint (union of its granule
 * footprints, else the metadata bounds; hidden when unknown), and a
 * USER-initiated layer switch ([`setLayer`]) whose target's data is
 * nowhere in the current viewport auto-frames it. Deep-linked views are
 * never stomped: attribute-driven applies skip the auto-frame.
 *
 * Tile addressing (the #27 mirror): the Swath API serves OGC order
 * `/tiles/{tileMatrix}/{tileRow}/{tileCol}` = z/y/x, while MapLibre
 * raster templates are XYZ-named — so the template's middle segment must
 * be `{y}` (row): `/tiles/{z}/{y}/{x}`.
 */

import { type IControl, Map as MapLibreMap, type RasterTileSource } from "maplibre-gl";
import maplibreCss from "maplibre-gl/dist/maplibre-gl.css?inline";
import {
  type CompareSides,
  clampSwipe,
  compareSides,
  resolveCompare,
  sideLabels,
} from "./compare-model.js";
import { type GranuleBbox, parseBbox, unionBbox } from "./granule-footprints.js";
import { CompareView } from "./swath-compare.js";
import { type EventSourceFactory, XRayOverlay } from "./swath-xray.js";
import { parseGranuleDatetimes, TimeSlider } from "./time-slider.js";
import { centerTile } from "./tms.js";
import { formatSwipe, parseCenter, parseNumber, parseSwipe, parseTime } from "./view-state.js";

/** One entry of the server's tilesets list, as `layers()` returns it. */
export interface SwathLayer {
  /** Layer id — the `{layerId}` path segment of the tile URL. */
  id: string;
  /** Human-readable title from the tileset metadata. */
  title: string;
}

/** Geographic bounds from tileset metadata (CRS84 lon/lat degrees). */
interface LonLatBounds {
  west: number;
  south: number;
  east: number;
  north: number;
}

/** Subset of an OGC link object the component reads. */
interface OgcLink {
  href?: string;
  rel?: string;
}

/** What the component reads off `/tilesets/{id}` metadata: the data
 * bounds (zero-config fit) and, for catalog-backed layers, the backing
 * dataset id from the granules link (issue #182). The id, not the raw
 * href, deliberately: metadata links are absolute against the server's
 * `base-url`, while the component must fetch through its own `server`
 * origin (the vite dev proxy in dev — CORS stays opt-in and off), the
 * same reason `layerIdFromSelfLink` parses ids instead of following
 * hrefs. */
interface LayerMetadata {
  bounds?: LonLatBounds;
  dataset?: string;
}

/** Subset of a tilesets-list item the component reads. */
interface OgcTileSetItem {
  title?: string;
  links?: OgcLink[];
}

const SOURCE_ID = "swath";
const RASTER_LAYER_ID = "swath";

/**
 * The `basemap="demo"` shorthand: MapLibre's own demo world tiles — a light
 * vector world map hosted by maplibre.org for exactly this kind of demo use.
 * Any other non-empty `basemap` value is treated as a style-JSON URL. Without
 * a basemap, tiles outside the layer footprint are transparent over the page
 * background — geographically honest, but disorienting; the basemap gives the
 * imagery a world for context. Fetched once per URL and cached; a fetch
 * failure falls back to the bare style (the imagery must never be hostage to
 * a third-party host, so e2e/CI runs keep basemap off).
 */
const DEMO_BASEMAP_URL = "https://demotiles.maplibre.org/style.json";

/** Tile-404 auto-retry pacing: one style re-apply per interval while tiles
 * are missing, up to the cap (~3 minutes of patience for the demo's
 * "viewer open before the granule exists" flow, then quiet). */
const RETRY_INTERVAL_MS = 3000;
const MAX_TILE_RETRIES = 60;

const basemapCache = new Map<string, Promise<Record<string, unknown> | undefined>>();

function fetchBasemapStyle(url: string): Promise<Record<string, unknown> | undefined> {
  let cached = basemapCache.get(url);
  if (!cached) {
    cached = fetch(url)
      .then((r) => (r.ok ? (r.json() as Promise<Record<string, unknown>>) : undefined))
      .catch(() => undefined);
    basemapCache.set(url, cached);
  }
  return cached;
}
const STYLE_ELEMENT_ID = "swath-map-styles";

/** Component chrome on top of MapLibre's stylesheet: a block host with a
 * usable default height (consumer CSS overrides both), plus the minimal
 * layer-switcher skin. */
const COMPONENT_CSS = `
swath-map { display: block; position: relative; }
/* Zero-specificity default height: the injected sheet lands AFTER consumer
 * styles in <head>, so at normal specificity this fallback would BEAT an
 * equally-specific consumer rule (it silently squashed the full-viewport
 * demo to a 400px strip). :where() keeps it losing to any consumer CSS. */
:where(swath-map) { height: 400px; }
swath-map .swath-map-container { width: 100%; height: 100%; }
swath-map .swath-map-switcher button {
  width: auto;
  padding: 0 8px;
  font: 12px/29px system-ui, sans-serif;
}
swath-map .swath-map-switcher button[aria-pressed="true"] {
  font-weight: 700;
  background: rgb(0 0 0 / 8%);
}
swath-map .swath-map-xray-toggle button {
  width: auto;
  padding: 0 8px;
  font: 12px/29px system-ui, sans-serif;
}
swath-map .swath-map-xray-toggle button[aria-pressed="true"] {
  font-weight: 700;
  background: rgb(22 163 74 / 15%);
}
swath-map .swath-map-zoomdata button {
  width: auto;
  padding: 0 8px;
  font: 12px/29px system-ui, sans-serif;
}
swath-map .swath-map-zoomdata[hidden] { display: none; }
/* The time slider (issue #182): bottom-center, between the x-ray
 * readouts (bottom-left) and the trace feed (bottom-right), styled like
 * the overlay's dark-telemetry cards. */
swath-map .swath-map-time {
  position: absolute;
  left: 50%;
  bottom: 8px;
  transform: translateX(-50%);
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px;
  border-radius: 4px;
  background: rgb(0 0 0 / 75%);
  color: #e2e8f0;
  font: 11px/1.5 ui-monospace, monospace;
}
swath-map .swath-map-time[hidden] { display: none; }
swath-map .swath-map-time-play {
  border: 0;
  margin: 0;
  padding: 1px 6px;
  border-radius: 3px;
  background: rgb(255 255 255 / 12%);
  color: inherit;
  font: inherit;
  cursor: pointer;
}
swath-map .swath-map-time-play[aria-pressed="true"] {
  color: #4ade80;
  font-weight: 700;
}
swath-map .swath-map-time input[type="range"] {
  width: 180px;
  accent-color: #4ade80;
}
swath-map .swath-map-time-label { white-space: nowrap; }
/* The compare swipe (issue #210): the right-side map rides z-index 1 —
 * below MapLibre's control corners (z 2), the time slider (z 2), and
 * the x-ray overlay (z 2), so every control stays reachable over it. */
swath-map .swath-map-compare {
  position: absolute;
  inset: 0;
  z-index: 1;
  overflow: hidden;
  pointer-events: none;
}
swath-map .swath-map-compare-map { position: absolute; inset: 0; }
/* A real 12px hit area centered on the divider (the negative margin
 * re-centers it on the left percentage) — draggable and focusable. */
swath-map .swath-map-compare-handle {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 12px;
  margin-left: -6px;
  z-index: 3;
  pointer-events: auto;
  cursor: ew-resize;
  touch-action: none;
}
swath-map .swath-map-compare-handle::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 5px;
  width: 2px;
  background: #fff;
  box-shadow: 0 0 4px rgb(0 0 0 / 50%);
}
swath-map .swath-map-compare-handle:focus-visible {
  outline: 2px solid #4ade80;
  outline-offset: 2px;
}
swath-map .swath-map-compare-grip {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 30px;
  height: 30px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: #fff;
  color: #0f172a;
  font: 700 13px/1 ui-monospace, monospace;
  box-shadow: 0 0 6px rgb(0 0 0 / 40%);
}
swath-map .swath-map-compare-label {
  position: absolute;
  top: 8px;
  padding: 2px 8px;
  border-radius: 4px;
  background: rgb(0 0 0 / 75%);
  color: #e2e8f0;
  font: 11px/1.5 ui-monospace, monospace;
  white-space: nowrap;
}
swath-map .swath-map-compare-label[data-side="left"] { right: 10px; }
swath-map .swath-map-compare-label[data-side="right"] { left: 10px; }
swath-map .swath-map-compare-toggle button {
  width: auto;
  padding: 0 8px;
  font: 12px/29px system-ui, sans-serif;
}
swath-map .swath-map-compare-toggle button[aria-pressed="true"] {
  font-weight: 700;
  background: rgb(37 99 235 / 15%);
}
swath-map .swath-map-compare-toggle[hidden] { display: none; }
`;

/** Injects MapLibre's CSS + the component chrome once per document. */
function injectStyles(doc: Document): void {
  if (doc.getElementById(STYLE_ELEMENT_ID)) {
    return;
  }
  const style = doc.createElement("style");
  style.id = STYLE_ELEMENT_ID;
  style.textContent = `${maplibreCss}\n${COMPONENT_CSS}`;
  doc.head.append(style);
}

/** The layer id is the last path segment of a tileset's `self` link (the
 * OGC list item carries no bare id field). */
function layerIdFromSelfLink(item: OgcTileSetItem): string | undefined {
  const self = item.links?.find((link) => link.rel === "self")?.href;
  return self?.split("/").filter(Boolean).pop();
}

/** The built-in layer switcher: a MapLibre control of real `<button>`s
 * (accessible: `aria-pressed` marks the active layer). Deliberately
 * minimal — the control-plane UI is later work. */
class LayerSwitcherControl implements IControl {
  readonly #host: SwathMap;
  #container: HTMLElement | undefined;
  #layers: readonly SwathLayer[] = [];
  #active = "";

  constructor(host: SwathMap) {
    this.#host = host;
  }

  onAdd(): HTMLElement {
    const container = document.createElement("div");
    container.className = "maplibregl-ctrl maplibregl-ctrl-group swath-map-switcher";
    container.setAttribute("role", "group");
    container.setAttribute("aria-label", "Layers");
    this.#container = container;
    this.#render();
    return container;
  }

  onRemove(): void {
    this.#container?.remove();
    this.#container = undefined;
  }

  update(layers: readonly SwathLayer[], active: string): void {
    this.#layers = layers;
    this.#active = active;
    this.#render();
  }

  #render(): void {
    const container = this.#container;
    if (!container) {
      return;
    }
    container.replaceChildren();
    for (const layer of this.#layers) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = layer.title;
      button.setAttribute("aria-pressed", String(layer.id === this.#active));
      button.addEventListener("click", () => {
        void this.#host.setLayer(layer.id);
      });
      container.append(button);
    }
  }
}

/** The x-ray toggle: one accessible button whose `aria-pressed` mirrors
 * the host's `xray` attribute — the attribute is the single source of
 * truth, the button just flips it. */
class XRayToggleControl implements IControl {
  readonly #host: SwathMap;
  #button: HTMLButtonElement | undefined;

  constructor(host: SwathMap) {
    this.#host = host;
  }

  onAdd(): HTMLElement {
    const container = document.createElement("div");
    container.className = "maplibregl-ctrl maplibregl-ctrl-group swath-map-xray-toggle";
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "X-ray";
    button.setAttribute("aria-label", "Toggle x-ray overlay");
    button.addEventListener("click", () => {
      this.#host.toggleAttribute("xray");
    });
    this.#button = button;
    container.append(button);
    this.update(this.#host.hasAttribute("xray"));
    return container;
  }

  onRemove(): void {
    this.#button?.parentElement?.remove();
    this.#button = undefined;
  }

  update(pressed: boolean): void {
    this.#button?.setAttribute("aria-pressed", String(pressed));
  }
}

/** The compare toggle (issue #210): one button that starts/ends the
 * before-vs-after gesture on the current layer's time series. Hidden
 * while the layer has fewer than two frames AND no compare is active (a
 * dead button would promise what it cannot do — but an active
 * layer-vs-layer compare must always be dismissable). The host's
 * attributes are the single source of truth; the button just asks. */
class CompareToggleControl implements IControl {
  readonly #host: SwathMap;
  #container: HTMLElement | undefined;
  #button: HTMLButtonElement | undefined;
  #active = false;
  #available = false;

  constructor(host: SwathMap) {
    this.#host = host;
  }

  onAdd(): HTMLElement {
    const container = document.createElement("div");
    container.className = "maplibregl-ctrl maplibregl-ctrl-group swath-map-compare-toggle";
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "compare";
    button.setAttribute("aria-label", "Toggle compare swipe");
    button.addEventListener("click", () => {
      this.#host.toggleCompare();
    });
    container.append(button);
    this.#container = container;
    this.#button = button;
    this.#render();
    return container;
  }

  onRemove(): void {
    this.#container?.remove();
    this.#container = undefined;
    this.#button = undefined;
  }

  update(active: boolean, available: boolean): void {
    this.#active = active;
    this.#available = available;
    this.#render();
  }

  #render(): void {
    if (this.#container) {
      this.#container.hidden = !this.#active && !this.#available;
    }
    this.#button?.setAttribute("aria-pressed", String(this.#active));
  }
}

/** The "zoom to data" control (issue #182 follow-up): one button that
 * frames the current layer's data footprint — the recovery affordance
 * for "I picked a layer and see nothing; where on Earth is it?". Hidden
 * whenever the layer's footprint is unknown (a dead button would
 * promise what it cannot do). */
class ZoomToDataControl implements IControl {
  readonly #host: SwathMap;
  #container: HTMLElement | undefined;
  #known = false;

  constructor(host: SwathMap) {
    this.#host = host;
  }

  onAdd(): HTMLElement {
    const container = document.createElement("div");
    container.className = "maplibregl-ctrl maplibregl-ctrl-group swath-map-zoomdata";
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "zoom to data";
    button.setAttribute("aria-label", "Zoom to the layer's data");
    button.addEventListener("click", () => {
      this.#host.zoomToData();
    });
    container.append(button);
    container.hidden = !this.#known;
    this.#container = container;
    return container;
  }

  onRemove(): void {
    this.#container?.remove();
    this.#container = undefined;
  }

  /** Shows the button exactly while a footprint is known. */
  update(known: boolean): void {
    this.#known = known;
    if (this.#container) {
      this.#container.hidden = !known;
    }
  }
}

export class SwathMap extends HTMLElement {
  static readonly tagName = "swath-map";

  static get observedAttributes(): readonly string[] {
    return [
      "server",
      "layer",
      "center",
      "zoom",
      "datetime",
      "xray",
      "basemap",
      "compare-datetime",
      "compare-layer",
      "swipe",
    ];
  }

  #map: MapLibreMap | undefined;
  #retriesLeft = MAX_TILE_RETRIES;
  #retryTimer: number | undefined;
  // Bumped when a liveness probe turns 404→200 and appended to the tile
  // template (`?v=n`): MapLibre treats raster-tile 404s as "empty, done" (no
  // error event, no refetch — ever), and its style diff no-ops an unchanged
  // source, so recovery needs BOTH a poll and a genuinely different source.
  #retrySeq = 0;
  #probedEmpty = false;
  #switcher: LayerSwitcherControl | undefined;
  #xrayToggle: XRayToggleControl | undefined;
  #xray: XRayOverlay | undefined;
  #time: TimeSlider | undefined;
  #zoomData: ZoomToDataControl | undefined;
  /** The compare swipe (issue #210): present exactly while a coherent
   * compare is asked for via `compare-datetime` / `compare-layer`. */
  #compare: CompareView | undefined;
  #compareToggle: CompareToggleControl | undefined;
  /** The sides of the active compare — what the x-ray overlay matches
   * traces against, and the guard that keeps swipe-only updates from
   * touching the right map's source. */
  #compareSides: CompareSides | undefined;
  /** The right side's last-applied tile template (skip identical
   * re-points: `setTiles` refetches even for an identical template). */
  #compareTemplate = "";
  /** The basemap style object the last apply merged under the imagery —
   * reused for the compare map so both sides share one world. */
  #appliedBasemap: Record<string, unknown> | undefined;
  /** The active layer's temporal domain (ascending) — feeds the compare
   * toggle's availability and its before-vs-after default. */
  #frames: readonly string[] = [];
  /** Where the active layer's data IS (issue #182 follow-up): the union
   * of its granule footprints (catalog-backed), else the tileset
   * metadata bounds (static layers), else unknown. What `zoomToData`
   * frames and the auto-frame-on-switch checks against. */
  #dataBounds: LonLatBounds | undefined;
  /** Set by [`setLayer`] — a USER-initiated switch: the one path that
   * may auto-frame the new layer's data. Attribute-driven applies (deep
   * links, programmatic sets) never auto-frame — a shared URL's view is
   * honored exactly (the issue #108 precedence contract). */
  #pendingAutoFrame = false;
  /** The layer id the last successful apply painted — what the x-ray
   * overlay filters its badges on. */
  #activeLayer = "";
  /** Monotonic token: only the latest async apply may touch the map. */
  #epoch = 0;
  #ready: Promise<void> = Promise.resolve();

  /** Test seam: how the x-ray overlay opens its SSE stream. Leave unset
   * for the real `EventSource`; unit tests assign a factory producing a
   * scriptable fake BEFORE setting the `xray` attribute. */
  xrayEventSource: EventSourceFactory | undefined;

  /**
   * The underlying MapLibre `Map` instance (undefined until connected).
   *
   * Escape hatch, explicitly UNSTABLE: reaching through it couples the
   * consumer to MapLibre's API and this component's internal style
   * (source/layer ids may change without notice).
   */
  get map(): MapLibreMap | undefined {
    return this.#map;
  }

  /** Settles when the last attribute-driven update has been applied —
   * `await el.ready` before inspecting the map. Rejects on fetch/apply
   * failure (also reported via a bubbling `swath-error` event). */
  get ready(): Promise<void> {
    return this.#ready;
  }

  /** Base URL of the Swath API (no trailing slash); same origin when the
   * `server` attribute is absent. */
  get server(): string {
    return (this.getAttribute("server") ?? "").replace(/\/+$/, "");
  }

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    const container = document.createElement("div");
    container.className = "swath-map-container";
    this.replaceChildren(container);

    const center = parseCenter(this.getAttribute("center"));
    const zoom = parseNumber(this.getAttribute("zoom"));
    this.#map = new MapLibreMap({
      container,
      style: { version: 8, sources: {}, layers: [] },
      center: center ?? [0, 0],
      zoom: zoom ?? 1,
    });
    if (this.getAttribute("switcher") !== "off") {
      this.#switcher = new LayerSwitcherControl(this);
      this.#map.addControl(this.#switcher, "top-right");
    }
    this.#xrayToggle = new XRayToggleControl(this);
    this.#map.addControl(this.#xrayToggle, "top-right");
    this.#compareToggle = new CompareToggleControl(this);
    this.#map.addControl(this.#compareToggle, "top-right");
    this.#zoomData = new ZoomToDataControl(this);
    this.#map.addControl(this.#zoomData, "top-right");
    this.#time = new TimeSlider(this.ownerDocument, {
      scrubTo: (datetime) => {
        this.setAttribute("datetime", datetime);
      },
      prefetch: (datetime) => {
        this.#prefetchFrame(datetime);
      },
    });
    this.append(this.#time.element);
    if (this.hasAttribute("xray")) {
      this.#enableXRay();
    }
    this.#startApply();
  }

  /** Re-applies the current layer (style + sources) — refetches tiles. */
  refresh(): void {
    this.#startApply();
  }

  disconnectedCallback(): void {
    this.#epoch += 1;
    if (this.#retryTimer !== undefined) {
      window.clearTimeout(this.#retryTimer);
      this.#retryTimer = undefined;
    }
    this.#disableXRay();
    this.#compare?.dispose();
    this.#compare = undefined;
    this.#compareSides = undefined;
    this.#compareTemplate = "";
    this.#time?.dispose();
    this.#time = undefined;
    this.#map?.remove();
    this.#map = undefined;
    this.#switcher = undefined;
    this.#xrayToggle = undefined;
    this.#compareToggle = undefined;
    this.#zoomData = undefined;
    this.replaceChildren();
  }

  attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void {
    if (!this.#map || oldValue === newValue) {
      return;
    }
    switch (name) {
      case "server":
      case "layer":
      case "basemap":
        this.#startApply();
        break;
      case "center": {
        const center = parseCenter(newValue);
        if (center) {
          this.#map.jumpTo({ center });
        }
        break;
      }
      case "zoom": {
        const zoom = parseNumber(newValue);
        if (zoom !== undefined) {
          this.#map.jumpTo({ zoom });
        }
        break;
      }
      case "datetime":
        this.#repointTime(newValue);
        break;
      case "compare-datetime":
      case "compare-layer":
        this.#applyCompare();
        this.#dispatchCompareChange();
        break;
      case "swipe": {
        // The drag fast path: re-clip only. Never re-point the right
        // source here — `setTiles` refetches even an identical template,
        // and a drag emits many of these per second.
        const fraction = clampSwipe(parseSwipe(newValue));
        this.#compare?.setFraction(fraction);
        if (this.#compareSides) {
          this.#xray?.setCompare({ fraction, sides: this.#compareSides });
        }
        this.#dispatchCompareChange();
        break;
      }
      case "xray":
        if (newValue === null) {
          this.#disableXRay();
        } else {
          this.#enableXRay();
        }
        this.#xrayToggle?.update(newValue !== null);
        break;
      default:
        break;
    }
  }

  /** Fetches the server's available layers from `/tilesets`. */
  async layers(): Promise<SwathLayer[]> {
    const response = await fetch(`${this.server}/tilesets`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) {
      throw new Error(`GET ${this.server}/tilesets failed: ${response.status}`);
    }
    const body = (await response.json()) as { tilesets?: OgcTileSetItem[] };
    const layers: SwathLayer[] = [];
    for (const item of body.tilesets ?? []) {
      const id = layerIdFromSelfLink(item);
      if (id !== undefined) {
        layers.push({ id, title: item.title ?? id });
      }
    }
    return layers;
  }

  /** Switches the displayed layer (reflects the `layer` attribute) and
   * settles once the map style has been updated. This is the
   * USER-INITIATED path (the built-in switcher, the entry page's layer
   * rail, an authored service landing): if the new layer's data
   * footprint is nowhere in the current viewport, the apply frames it —
   * a ~10 km fire window must never be an invisible needle on a world
   * map. Deep links and programmatic attribute writes go through the
   * attribute directly and never auto-frame (a shared URL's view wins,
   * the issue #108 precedence contract). */
  async setLayer(id: string): Promise<void> {
    this.#pendingAutoFrame = true;
    this.setAttribute("layer", id);
    await this.#ready;
  }

  /**
   * The compare toggle's action (issue #210). Active → clears every
   * compare attribute (whatever the mode). Inactive → starts the
   * before-vs-after gesture on the current layer's time series: the
   * right side pins the NEWEST frame, and the left side keeps the
   * currently viewed frame unless that IS the newest (or "latest"), in
   * which case it jumps to the oldest — two sides showing one frame
   * would compare nothing. No-op on layers without at least two frames
   * (the control is hidden then); layer-vs-layer has no one-button
   * default and is entered via the `compare-layer` attribute (deep
   * links, host pages).
   */
  toggleCompare(): void {
    if (
      this.getAttribute("compare-datetime") !== null ||
      this.getAttribute("compare-layer") !== null
    ) {
      this.removeAttribute("compare-datetime");
      this.removeAttribute("compare-layer");
      this.removeAttribute("swipe");
      return;
    }
    const oldest = this.#frames[0];
    const newest = this.#frames[this.#frames.length - 1];
    if (oldest === undefined || newest === undefined || oldest === newest) {
      return;
    }
    const viewed = this.getAttribute("datetime");
    if (viewed === null || viewed === newest) {
      this.setAttribute("datetime", oldest);
    }
    this.setAttribute("compare-datetime", newest);
  }

  /**
   * The compare swipe's right-side MapLibre map (undefined while no
   * compare is active). Same UNSTABLE escape hatch as [`map`].
   */
  get compareMap(): MapLibreMap | undefined {
    return this.#compare?.map;
  }

  /** Frames the active layer's data footprint (the `zoom to data`
   * control's action — the always-available recovery affordance). No-op
   * while the footprint is unknown; the control is hidden then. */
  zoomToData(): void {
    this.#frameData();
  }

  /** Jumps the view to the active layer's data bounds and announces the
   * user-initiated move (`swath-framedata`, the page shell's URL-sync
   * seam — a framed view must be shareable). `duration: 0` follows the
   * footprint zoom's precedent: a jump reads clearer than an animation
   * across the world, and it is deterministic for tests. */
  #frameData(): void {
    const map = this.#map;
    const bounds = this.#dataBounds;
    if (!map || !bounds) {
      return;
    }
    map.once("moveend", () => {
      this.dispatchEvent(new CustomEvent("swath-framedata", { detail: { bounds }, bubbles: true }));
    });
    map.fitBounds(
      [
        [bounds.west, bounds.south],
        [bounds.east, bounds.north],
      ],
      { duration: 0, padding: 48 },
    );
  }

  /** Whether any part of `bounds` is inside the current viewport — the
   * "is the data already visible?" test that keeps auto-frame from
   * yanking a view that can see the data. */
  #viewIntersects(bounds: LonLatBounds): boolean {
    const map = this.#map;
    if (!map) {
      return true; // no view to disturb
    }
    const view = map.getBounds();
    return (
      bounds.west <= view.getEast() &&
      bounds.east >= view.getWest() &&
      bounds.south <= view.getNorth() &&
      bounds.north >= view.getSouth()
    );
  }

  /** Brings the x-ray overlay up (idempotent). The overlay's DOM lives
   * on this host element, not inside MapLibre's container, and its map
   * hooks are map-level events — both survive `setStyle`, so layer
   * switches never disturb it. */
  #enableXRay(): void {
    const map = this.#map;
    if (this.#xray || !map) {
      return;
    }
    this.#xray = new XRayOverlay(this, map, { createEventSource: this.xrayEventSource });
    this.#xray.connect(`${this.server}/traces`);
    this.#xray.setLayer(this.#activeLayer);
    // X-ray toggled on mid-compare: hand the fresh overlay the sides.
    if (this.#compareSides) {
      this.#xray.setCompare({
        fraction: clampSwipe(parseSwipe(this.getAttribute("swipe"))),
        sides: this.#compareSides,
      });
    }
  }

  /** Tears the x-ray overlay down (idempotent): closes its EventSource
   * and removes its DOM. */
  #disableXRay(): void {
    this.#xray?.dispose();
    this.#xray = undefined;
  }

  /** OGC `{tileMatrix}/{tileRow}/{tileCol}` is z/y/x, so MapLibre's
   * XYZ-named template must carry `{y}` (row) in the middle segment. */
  #tileTemplate(layerId: string): string {
    return this.#tileTemplateFor(layerId, this.#viewedDatetime());
  }

  /** The tile template for an arbitrary layer/frame pair — what the
   * compare swipe's right side is built from (issue #210). The query
   * carries the frame (`datetime=`, ADR 0015) and the liveness-probe
   * recovery version (`v=`, which must make the source template
   * genuinely different — see `#retrySeq`). */
  #tileTemplateFor(layerId: string, datetime: string | null): string {
    const parts: string[] = [];
    if (datetime !== null && datetime !== "") {
      parts.push(`datetime=${encodeURIComponent(datetime)}`);
    }
    if (this.#retrySeq > 0) {
      parts.push(`v=${this.#retrySeq}`);
    }
    const query = parts.length === 0 ? "" : `?${parts.join("&")}`;
    return `${this.server}/tilesets/${layerId}/tiles/{z}/{y}/{x}${query}`;
  }

  /** The `datetime` attribute as the tile query sees it (empty = absent). */
  #viewedDatetime(): string | null {
    const datetime = this.getAttribute("datetime");
    return datetime === "" ? null : datetime;
  }

  /**
   * The `datetime` fast path (issue #182): scrubbing re-points the
   * raster source in place — `setTiles` refetches at the new frame with
   * no style rebuild, no bounds refit, no `/tilesets` round trip. Before
   * the first apply lands there is no source yet and the pending apply
   * reads the attribute itself. The slider mirrors the attribute; the
   * bubbling `swath-timechange` is the page shell's URL-sync seam.
   */
  #repointTime(datetime: string | null): void {
    const map = this.#map;
    if (this.#activeLayer !== "" && map) {
      const repoint = (): void => {
        const source = map.getSource(SOURCE_ID) as RasterTileSource | undefined;
        // The template re-reads the datetime attribute, so a deferred
        // re-point always applies the LATEST frame, never a stale one.
        source?.setTiles([this.#tileTemplate(this.#activeLayer)]);
      };
      if (map.getSource(SOURCE_ID)) {
        repoint();
      } else {
        // Style mid-apply (a scrub can land inside setStyle's diff
        // window, where the source is momentarily unreachable): apply
        // the frame as soon as the style lands instead of silently
        // dropping it — the stuck-forever failure mode the screenshot
        // suite exposed.
        map.once("styledata", repoint);
      }
    }
    this.#time?.setActive(datetime);
    // A scrub moves the compare's LEFT side (and, in layer mode, the
    // right side's frame too): refresh sides/labels/template. Date-mode
    // right templates are unchanged and the identical-template guard
    // keeps the right source untouched then.
    this.#applyCompare();
    this.dispatchEvent(
      new CustomEvent("swath-timechange", { detail: { datetime }, bubbles: true }),
    );
  }

  /** Bound on play-mode prefetch: at most this many tiles per frame (a
   * 1528px viewport shows ~24 tiles; more means the view is mid-zoom
   * and warming it all buys nothing). */
  static readonly #PREFETCH_MAX = 32;

  /** Warms one frame (play mode): fetches the viewport's visible tile
   * URLs at the displayed tile zoom with the frame's `datetime=`, so
   * the server's write-through cache holds them before the frame
   * displays. Fire-and-forget; failures are the next request's problem. */
  #prefetchFrame(datetime: string): void {
    const map = this.#map;
    const layer = this.#activeLayer;
    if (!map || layer === "") {
      return;
    }
    // A 256px raster source displays z = style-zoom + 1 tiles (the same
    // arithmetic the x-ray overlay badges by).
    const z = Math.round(map.getZoom()) + 1;
    const bounds = map.getBounds();
    const nw = centerTile(bounds.getWest(), bounds.getNorth(), z);
    const se = centerTile(bounds.getEast(), bounds.getSouth(), z);
    const path = `${this.server}/tilesets/${layer}/tiles`;
    const query = `datetime=${encodeURIComponent(datetime)}`;
    let budget = SwathMap.#PREFETCH_MAX;
    for (let y = nw.y; y <= se.y && budget > 0; y += 1) {
      for (let x = nw.x; x <= se.x && budget > 0; x += 1) {
        budget -= 1;
        // OGC path order z/row/col = z/y/x.
        fetch(`${path}/${z}/${y}/${x}?${query}`).catch(() => undefined);
      }
    }
  }

  /** Kicks off an async (re)apply of server+layer; `ready` tracks it. */
  #startApply(): void {
    const ready = this.#applyLayer();
    this.#ready = ready;
    ready.catch((error: unknown) => {
      // Namespaced (not `error`): a bubbling `error` event would reach
      // `window` and read as an unhandled page error to host tooling.
      this.dispatchEvent(new CustomEvent("swath-error", { detail: { error }, bubbles: true }));
      // Self-healing applies: a viewer opened while the server is still
      // starting (the demo prints its URL during the docker build) sees
      // /tilesets fail — without a retry the component would stay blank
      // forever, basemap included. Same bounded budget as the tile probe.
      if (this.#retriesLeft > 0 && this.#retryTimer === undefined && this.isConnected) {
        this.#retriesLeft -= 1;
        this.#retryTimer = window.setTimeout(() => {
          this.#retryTimer = undefined;
          this.#startApply();
        }, RETRY_INTERVAL_MS);
      }
    });
  }

  async #applyLayer(): Promise<void> {
    const map = this.#map;
    if (!map) {
      return;
    }
    const epoch = ++this.#epoch;
    const available = await this.layers();
    const requested = this.getAttribute("layer");
    const layerId = requested ?? available[0]?.id;
    if (epoch !== this.#epoch || layerId === undefined) {
      return;
    }

    // The tileset metadata carries both the geographic bounds (the
    // zero-config fit below) and, for catalog-backed layers, the
    // granules link the time slider's domain comes from (issue #182).
    const fit = this.getAttribute("center") === null && this.getAttribute("zoom") === null;
    const metadata = await this.#layerMetadata(layerId);
    const bounds = fit ? metadata?.bounds : undefined;

    // Optional basemap under the imagery: fetch (cached) and merge our raster
    // source/layer ON TOP of its sources/layers. Failure → bare style.
    const basemapAttr = this.getAttribute("basemap");
    const basemapUrl =
      basemapAttr === null || basemapAttr === ""
        ? undefined
        : basemapAttr === "demo"
          ? DEMO_BASEMAP_URL
          : basemapAttr;
    const basemap = basemapUrl ? await fetchBasemapStyle(basemapUrl) : undefined;
    if (epoch !== this.#epoch || !this.#map) {
      return;
    }

    this.#appliedBasemap = basemap;
    const applied = new Promise<void>((resolve) => {
      map.once("styledata", () => resolve());
    });
    map.setStyle(this.#buildStyle(this.#tileTemplate(layerId), basemap) as never);
    await applied;
    if (epoch !== this.#epoch) {
      return;
    }
    if (bounds) {
      map.fitBounds(
        [
          [bounds.west, bounds.south],
          [bounds.east, bounds.north],
        ],
        { duration: 0, padding: 16 },
      );
    }
    this.#activeLayer = layerId;
    // `connect` is idempotent per URL: this only reconnects after a
    // `server` change, while layer switches just re-filter the badges.
    this.#xray?.connect(`${this.server}/traces`);
    this.#xray?.setLayer(layerId);
    this.#switcher?.update(available, layerId);
    // `layers` rides along (issue #108) so page chrome — the entry page's
    // layer browser — can list what exists without a second /tilesets
    // fetch that could disagree with the one this apply used.
    this.dispatchEvent(
      new CustomEvent("layerchange", {
        detail: { layer: layerId, layers: available },
        bubbles: true,
      }),
    );
    this.#startLivenessProbe(layerId, epoch);
    // The data domain loads LAST, after the probe kicked off: the
    // probe's timing relative to MapLibre's first tile fetches is part
    // of the painted result (its no-store fetch renders through the
    // cache, so reordering it flips a badge between live and cache_hit),
    // while the domain only feeds the slider and the data-framing.
    // `ready` still covers it — tests may await and then inspect.
    await this.#applyDataDomain(metadata, epoch);
    if (epoch !== this.#epoch) {
      return;
    }
    // The compare swipe (issue #210) follows the applied layer/server/
    // basemap — a full restyle, since any of those may have changed.
    this.#applyCompare({ restyle: true });
    // Auto-frame (issue #182 follow-up): a USER-initiated switch to a
    // layer whose data is nowhere in view jumps to the data — consumed
    // exactly once per `setLayer`, and skipped entirely when the data
    // already intersects the viewport (never yank a view that can see
    // it) or when the footprint is unknown.
    const autoFrame = this.#pendingAutoFrame;
    this.#pendingAutoFrame = false;
    if (autoFrame && this.#dataBounds && !this.#viewIntersects(this.#dataBounds)) {
      this.#frameData();
    }
  }

  /** Self-healing tiles for the "viewer open before the data exists" flow
   * (the stopwatch demo): probe the view's center tile; while it 404s,
   * re-probe each interval (bounded); on the first 200, bump the source
   * version and re-apply so MapLibre — which never refetches a failed tile —
   * actually paints the now-live imagery. A layer that is already live on
   * the first probe ends the loop immediately. */
  #startLivenessProbe(layerId: string, epoch: number): void {
    if (this.#retryTimer !== undefined) {
      window.clearTimeout(this.#retryTimer);
      this.#retryTimer = undefined;
    }
    const probe = async (): Promise<void> => {
      const map = this.#map;
      if (epoch !== this.#epoch || !map || this.#retriesLeft <= 0) {
        return;
      }
      let live = false;
      try {
        const { z, x, y } = centerTile(map.getCenter().lng, map.getCenter().lat, map.getZoom());
        const url = this.#tileTemplate(layerId)
          .replace("{z}", String(z))
          .replace("{y}", String(y))
          .replace("{x}", String(x));
        const response = await fetch(url, { cache: "no-store" });
        if (response.ok) {
          live = true;
        } else if (response.status !== 404) {
          return; // real errors are not "not yet" — stop probing
        }
      } catch {
        return; // network-level failure: stop; a reload is a fresh start
      }
      if (epoch !== this.#epoch) {
        return;
      }
      if (live) {
        if (this.#probedEmpty) {
          this.#probedEmpty = false;
          this.#retrySeq += 1;
          this.refresh();
        }
        return; // live on first probe: nothing to heal
      }
      this.#probedEmpty = true;
      this.#retriesLeft -= 1;
      this.#retryTimer = window.setTimeout(() => {
        this.#retryTimer = undefined;
        void probe();
      }, RETRY_INTERVAL_MS);
    };
    void probe();
  }

  /** Geographic bounds + granules link from `/tilesets/{id}` metadata;
   * undefined when the layer is not resolvable yet (e.g. empty catalog:
   * an honest 404) — the apply then proceeds without a fit or a slider. */
  async #layerMetadata(layerId: string): Promise<LayerMetadata | undefined> {
    const response = await fetch(`${this.server}/tilesets/${layerId}`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) {
      return undefined;
    }
    const body = (await response.json()) as {
      boundingBox?: { lowerLeft?: number[]; upperRight?: number[] };
      links?: OgcLink[];
    };
    const metadata: LayerMetadata = {};
    const lower = body.boundingBox?.lowerLeft;
    const upper = body.boundingBox?.upperRight;
    const [west, south] = lower ?? [];
    const [east, north] = upper ?? [];
    if (west !== undefined && south !== undefined && east !== undefined && north !== undefined) {
      metadata.bounds = { west, south, east, north };
    }
    // `/datasets/{id}/granules` — the dataset id is the second-to-last
    // path segment of the granules link.
    const granules = body.links?.find((link) => link.rel === "granules")?.href;
    const dataset = granules?.split("/").filter(Boolean).at(-2);
    if (dataset !== undefined) {
      metadata.dataset = dataset;
    }
    return metadata;
  }

  /** Feeds the time slider AND the data-framing (issue #182): fetches
   * the layer's granule listing (when its metadata linked one), hands
   * the acquisition datetimes to the slider, and records where the data
   * IS — the union of granule footprints, falling back to the tileset
   * metadata bounds for static layers, unknown when neither exists (the
   * zoom-to-data control hides then). Failures are tolerated: both are
   * bonus affordances, never a reason the imagery fails to paint. */
  async #applyDataDomain(metadata: LayerMetadata | undefined, epoch: number): Promise<void> {
    let frames: string[] = [];
    let footprints: GranuleBbox[] = [];
    if (metadata?.dataset !== undefined) {
      try {
        const url = `${this.server}/datasets/${encodeURIComponent(metadata.dataset)}/granules`;
        const response = await fetch(url, { headers: { accept: "application/json" } });
        if (response.ok) {
          const body: unknown = await response.json();
          frames = parseGranuleDatetimes(body);
          footprints = ((body as { granules?: { bbox?: unknown }[] }).granules ?? [])
            .map((granule) => parseBbox(granule.bbox))
            .filter((bbox): bbox is GranuleBbox => bbox !== undefined);
        }
      } catch {
        frames = [];
        footprints = [];
      }
    }
    if (epoch !== this.#epoch) {
      return;
    }
    const union = unionBbox(footprints);
    this.#dataBounds = union
      ? { west: union[0], south: union[1], east: union[2], north: union[3] }
      : metadata?.bounds;
    this.#zoomData?.update(this.#dataBounds !== undefined);
    this.#frames = frames;
    this.#time?.setDomain(frames, this.getAttribute("datetime"));
  }

  /** The map style for one side: the swath raster (from `template`) on
   * top of the shared basemap, or bare when there is none. */
  #buildStyle(template: string, basemap: Record<string, unknown> | undefined): object {
    const swathSource = { type: "raster", tiles: [template], tileSize: 256 };
    const swathLayer = { id: RASTER_LAYER_ID, type: "raster", source: SOURCE_ID };
    return basemap
      ? {
          ...basemap,
          sources: {
            ...(basemap["sources"] as Record<string, unknown>),
            [SOURCE_ID]: swathSource,
          },
          layers: [...(basemap["layers"] as unknown[]), swathLayer],
        }
      : {
          version: 8,
          sources: { [SOURCE_ID]: swathSource },
          layers: [swathLayer],
        };
  }

  /**
   * The compare swipe's reconciler (issue #210): reads the compare
   * attributes, and either tears the swipe down (nothing coherent asked
   * for) or brings the right side in line — creating the clipped second
   * map on first activation, re-styling it after a layer/server/basemap
   * apply (`restyle`), or just re-pointing its source when only the
   * right frame/layer changed. Also keeps the toggle control and the
   * x-ray overlay's per-side matching in sync.
   */
  #applyCompare(options: { restyle?: boolean } = {}): void {
    const map = this.#map;
    const spec =
      map && this.#activeLayer !== ""
        ? resolveCompare(
            this.#activeLayer,
            parseTime(this.getAttribute("compare-datetime")),
            this.getAttribute("compare-layer") ?? undefined,
          )
        : undefined;
    this.#compareToggle?.update(spec !== undefined, this.#frames.length >= 2);
    if (!spec || !map) {
      this.#compare?.dispose();
      this.#compare = undefined;
      this.#compareSides = undefined;
      this.#compareTemplate = "";
      this.#xray?.setCompare(undefined);
      return;
    }
    const sides = compareSides(spec, this.#activeLayer, this.#viewedDatetime());
    const fraction = clampSwipe(parseSwipe(this.getAttribute("swipe")));
    const template = this.#tileTemplateFor(sides.right.layer, sides.right.requested);
    let view = this.#compare;
    let restyle = options.restyle === true;
    if (!view) {
      view = new CompareView(this, map, {
        onSwipe: (moved) => {
          this.setAttribute("swipe", formatSwipe(moved));
        },
      });
      this.#compare = view;
      restyle = true;
    }
    if (restyle || view.map === undefined) {
      view.setStyle(this.#buildStyle(template, this.#appliedBasemap));
    } else if (template !== this.#compareTemplate) {
      view.repoint(SOURCE_ID, template);
    }
    this.#compareTemplate = template;
    const labels = sideLabels(sides);
    view.setLabels(sides.mode, labels.left, labels.right);
    view.setFraction(fraction);
    this.#compareSides = sides;
    this.#xray?.setCompare({ fraction, sides });
  }

  #dispatchCompareChange(): void {
    this.dispatchEvent(
      new CustomEvent("swath-comparechange", {
        detail: {
          compareTime: this.getAttribute("compare-datetime"),
          compareLayer: this.getAttribute("compare-layer"),
          swipe: this.getAttribute("swipe"),
        },
        bubbles: true,
      }),
    );
  }
}

/** Registers `<swath-map>`; safe to call more than once. */
export function defineSwathMap(): void {
  if (!customElements.get(SwathMap.tagName)) {
    customElements.define(SwathMap.tagName, SwathMap);
  }
}
