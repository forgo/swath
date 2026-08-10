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
 * - `xray`   — presence toggles the x-ray overlay (issue #34): per-tile
 *   decisions/timings from the `/traces` SSE stream, painted by the
 *   built-in [`XRayOverlay`] module (see swath-xray.ts for the design
 *   rationale). A toggle button control mirrors the attribute.
 *
 * When neither `center` nor `zoom` is given, the view fits the layer's
 * geographic bounds from `/tilesets/{id}` metadata — a bare
 * `<swath-map>` against a live server is a working demo.
 *
 * Tile addressing (the #27 mirror): the Swath API serves OGC order
 * `/tiles/{tileMatrix}/{tileRow}/{tileCol}` = z/y/x, while MapLibre
 * raster templates are XYZ-named — so the template's middle segment must
 * be `{y}` (row): `/tiles/{z}/{y}/{x}`.
 */

import { type IControl, Map as MapLibreMap } from "maplibre-gl";
import maplibreCss from "maplibre-gl/dist/maplibre-gl.css?inline";
import { type EventSourceFactory, XRayOverlay } from "./swath-xray.js";
import { centerTile } from "./tms.js";

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

/** Parses a `"lon,lat"` attribute; undefined when absent or malformed. */
function parseCenter(value: string | null): [number, number] | undefined {
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

/** Parses a numeric attribute; undefined when absent or malformed. */
function parseNumber(value: string | null): number | undefined {
  if (value === null) {
    return undefined;
  }
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : undefined;
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

export class SwathMap extends HTMLElement {
  static readonly tagName = "swath-map";

  static get observedAttributes(): readonly string[] {
    return ["server", "layer", "center", "zoom", "xray", "basemap"];
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
    this.#switcher = new LayerSwitcherControl(this);
    this.#map.addControl(this.#switcher, "top-right");
    this.#xrayToggle = new XRayToggleControl(this);
    this.#map.addControl(this.#xrayToggle, "top-right");
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
    this.#map?.remove();
    this.#map = undefined;
    this.#switcher = undefined;
    this.#xrayToggle = undefined;
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
   * settles once the map style has been updated. */
  async setLayer(id: string): Promise<void> {
    this.setAttribute("layer", id);
    await this.#ready;
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
    return `${this.server}/tilesets/${layerId}/tiles/{z}/{y}/{x}`;
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

    // Zero-config view: no explicit center/zoom means "fit the layer's
    // data" — read its geographic bounds off the tileset metadata.
    const fit = this.getAttribute("center") === null && this.getAttribute("zoom") === null;
    const bounds = fit ? await this.#layerBounds(layerId) : undefined;

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

    const retrySuffix = this.#retrySeq > 0 ? `?v=${this.#retrySeq}` : "";
    const swathSource = {
      type: "raster",
      tiles: [`${this.#tileTemplate(layerId)}${retrySuffix}`],
      tileSize: 256,
    };
    const swathLayer = { id: RASTER_LAYER_ID, type: "raster", source: SOURCE_ID };
    const applied = new Promise<void>((resolve) => {
      map.once("styledata", () => resolve());
    });
    map.setStyle(
      basemap
        ? ({
            ...basemap,
            sources: {
              ...(basemap["sources"] as Record<string, unknown>),
              [SOURCE_ID]: swathSource,
            },
            layers: [...(basemap["layers"] as unknown[]), swathLayer],
          } as never)
        : {
            version: 8,
            sources: { [SOURCE_ID]: swathSource as never },
            layers: [swathLayer as never],
          },
    );
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
    this.dispatchEvent(
      new CustomEvent("layerchange", { detail: { layer: layerId }, bubbles: true }),
    );
    this.#startLivenessProbe(layerId, epoch);
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

  /** Geographic bounds from `/tilesets/{id}` metadata; undefined when the
   * layer is not resolvable yet (e.g. empty catalog: an honest 404). */
  async #layerBounds(layerId: string): Promise<LonLatBounds | undefined> {
    const response = await fetch(`${this.server}/tilesets/${layerId}`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) {
      return undefined;
    }
    const body = (await response.json()) as {
      boundingBox?: { lowerLeft?: number[]; upperRight?: number[] };
    };
    const lower = body.boundingBox?.lowerLeft;
    const upper = body.boundingBox?.upperRight;
    const [west, south] = lower ?? [];
    const [east, north] = upper ?? [];
    if (west === undefined || south === undefined || east === undefined || north === undefined) {
      return undefined;
    }
    return { west, south, east, north };
  }
}

/** Registers `<swath-map>`; safe to call more than once. */
export function defineSwathMap(): void {
  if (!customElements.get(SwathMap.tagName)) {
    customElements.define(SwathMap.tagName, SwathMap);
  }
}
