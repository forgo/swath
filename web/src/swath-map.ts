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
 *   UI, like the entry page's `<swath-layer-list>` (issues #108, #282).
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
 * - `cinematic` — the zero-config landing's opening move (issue #211),
 *   read at apply time: with no `layer` given, the default becomes the
 *   first tileset with a playable time series (two or more granule
 *   dates; a bounded metadata scan) instead of the first tileset, and
 *   once that layer's domain lands the loop plays on its own — unless
 *   `prefers-reduced-motion` is set, in which case it waits with a play
 *   affordance. Hovering the map pauses the loop (moving off resumes
 *   it); any real interaction — a scrub, the play button, a drag or
 *   wheel, a layer switch — hands playback to the user for good. A
 *   top-center landing card narrates the state and carries the one-line
 *   x-ray invitation; it hides once the overlay is on. Hosts must set
 *   this ONLY for a view nobody asked for explicitly: a deep link or a
 *   restored session is never overridden (the entry page's precedence).
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
import { SwathApi } from "./api.js";
import {
  type CompareSides,
  clampSwipe,
  compareSides,
  resolveCompare,
  sideLabels,
} from "./compare-model.js";
import {
  type DatedFootprint,
  frameBounds,
  type GranuleBbox,
  parseBbox,
  unionBbox,
} from "./granule-footprints.js";
import { CompareView } from "./swath-compare.js";
import {
  type EventSourceFactory,
  type TraceEnvelope,
  TraceStream,
  type XRayChrome,
  XRayOverlay,
} from "./swath-xray.js";
import { boundDomain, parseGranuleDatetimes, TimeSlider } from "./time-slider.js";
import { centerTile } from "./tms.js";
import { SwathButton } from "./ui/button.js";
import { createSwathEvent } from "./ui/events.js";
import { SwathHudCard } from "./ui/hud-card.js";
import { adoptTokens } from "./ui/styles.js";
import { formatSwipe, parseCenter, parseNumber, parseSwipe, parseTime } from "./view-state.js";

/** One entry of the server's tilesets list, as `layers()` returns it. */
export interface SwathLayer {
  /** Layer id — the `{layerId}` path segment of the tile URL. */
  id: string;
  /** Human-readable title from the tileset metadata. */
  title: string;
}

/** Geographic bounds from tileset metadata (CRS84 lon/lat degrees). */
export interface LonLatBounds {
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
  /** The layer's frame-selection window from its tileset metadata
   * (`swath:window`, #301): bounds the slider's domain. */
  window?: [string | null, string | null];
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

/** Bound on the `cinematic` default's search for a playable layer: how
 * many tilesets (in listing order) get their metadata — and, when
 * catalog-backed, their granule listing — read before the search gives
 * up and today's first-tileset rule applies. Datasets are read once
 * each (sibling layers share a domain). Small: the landing must open in
 * seconds, and a demo catalog lists its playable series early. */
const CINEMATIC_SCAN_MAX = 8;

/** The temporal domain + footprint a granules listing yields. */
interface LayerDomain {
  frames: string[];
  /** Each granule's footprint WITH its instant, so the bounds can be
   * narrowed the same way the frames are: by the layer's compiled window,
   * and by the frame currently being viewed (#397). */
  footprints: DatedFootprint[];
}

const EMPTY_DOMAIN: LayerDomain = { frames: [], footprints: [] };

/** The OS-level "no animation, please" signal the cinematic landing
 * honors (issue #211): true means wait with a play affordance. */
function prefersReducedMotion(): boolean {
  return (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

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
/* MapLibre control skins: the library's white buttons, our type. Every
 * colour is a token (issue #285); translucent variants mix a token with
 * transparent so M12 retunes them from tokens.css alone. */
swath-map .swath-map-switcher button {
  width: auto;
  padding: 0 8px;
  font-family: var(--swath-font-ui);
  font-size: var(--swath-text-sm);
  line-height: 29px;
}
swath-map .swath-map-switcher button[aria-pressed="true"] {
  font-weight: 700;
  background: color-mix(in srgb, var(--swath-color-bg) 8%, transparent);
}
/* The map's own toggles (#293): a column top-right, in-map when bare. */
.swath-map-toggles {
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 2;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--swath-space-1);
}
swath-hud-dock > .swath-map-toggles { position: static; }
/* Floating over a live render, the toggles need a surface of their own.
 * swath-button's base is \`background: none\`, which reads only against the
 * dark palette — over pale NDVI in the compose preview column (#400) they
 * were close to invisible (#472). The same HUD surface the time card uses,
 * plus the blur that is this product's depth cue (#388). Only the surface:
 * the button keeps its own ink, so hover and pressed still read. */
.swath-map-toggles swath-button::part(base) {
  background: var(--swath-color-bg-hud);
  backdrop-filter: var(--swath-blur-hud);
}
/* The time slider (issue #182) and the landing card (issue #211) are the
 * map's own chrome, class-scoped (not \`swath-map .…\`) because a shell
 * hosts them in its HUD dock (issue #285): docked, they sit in the dock's
 * flex corners; bare, they float over the map as before. Same dark
 * telemetry card either way. */
.swath-map-time {
  position: absolute;
  left: 50%;
  bottom: 8px;
  transform: translateX(-50%);
  z-index: 2;
  display: flex;
  align-items: center;
  gap: var(--swath-space-2);
  padding: var(--swath-space-1) var(--swath-space-3);
  border-radius: var(--swath-radius-sm);
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono);
  font-size: var(--swath-text-xs);
  line-height: var(--swath-leading-normal);
}
swath-hud-dock > .swath-map-time { position: static; transform: none; }
.swath-map-time[hidden] { display: none; }
.swath-map-time-play {
  border: 0;
  margin: 0;
  padding: 1px 6px;
  border-radius: var(--swath-radius-sm);
  background: color-mix(in srgb, var(--swath-color-fg) 12%, transparent);
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.swath-map-time-play[aria-pressed="true"] {
  color: var(--swath-color-accent);
  font-weight: 700;
}
.swath-map-time input[type="range"] {
  width: 180px;
  accent-color: var(--swath-color-accent);
}
.swath-map-time-label { white-space: nowrap; }
/* The compare swipe (issue #210): the right-side map stacks by DOM
 * ORDER, not z-index — CompareView inserts it immediately after the
 * primary map container, so at z auto it paints over the primary canvas
 * and under everything appended later (the x-ray overlay root), while
 * MapLibre's control corners (z 2), the time slider (z 2), and the
 * x-ray inspector (z 3) rise above it. A positive z-index here would
 * force the overlay root to one too, which would lift badges over the
 * control buttons and swallow their clicks (the CI-found regression). */
swath-map .swath-map-compare {
  position: absolute;
  inset: 0;
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
  background: var(--swath-color-fg);
  box-shadow: 0 0 4px color-mix(in srgb, var(--swath-color-bg) 50%, transparent);
}
swath-map .swath-map-compare-handle:focus-visible {
  outline: var(--swath-border-focus);
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
  border-radius: var(--swath-radius-pill);
  background: var(--swath-color-fg);
  color: var(--swath-color-bg);
  font-family: var(--swath-font-mono);
  font-size: var(--swath-text-md);
  font-weight: 700;
  line-height: 1;
  box-shadow: 0 0 6px color-mix(in srgb, var(--swath-color-bg) 40%, transparent);
}
swath-map .swath-map-compare-label {
  position: absolute;
  top: 8px;
  padding: 2px 8px;
  border-radius: var(--swath-radius-sm);
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono);
  font-size: var(--swath-text-xs);
  line-height: var(--swath-leading-normal);
  white-space: nowrap;
}
swath-map .swath-map-compare-label[data-side="left"] { right: 10px; }
swath-map .swath-map-compare-label[data-side="right"] { left: 10px; }
/* The cinematic landing card (issue #211): top-center, the same dark
 * telemetry card as the slider — one status line, the x-ray invitation
 * as the single accent-colored action. Hidden once x-ray is on. */
.swath-map-landing {
  position: absolute;
  top: 8px;
  /* Centered by auto margins, not left:50%: an absolutely positioned
   * box at left:50% shrink-wraps to HALF the container and clips its
   * status line. Capped short of the top-right controls. */
  left: 0;
  right: 0;
  margin: 0 auto;
  width: fit-content;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: var(--swath-space-3);
  max-width: calc(100% - 260px);
  padding: 5px 6px 5px 10px;
  border-radius: var(--swath-radius-sm);
  background: var(--swath-color-bg-hud);
  color: var(--swath-color-fg);
  font-family: var(--swath-font-mono);
  font-size: var(--swath-text-xs);
  line-height: var(--swath-leading-normal);
}
swath-hud-dock > .swath-map-landing { position: static; margin: 0; }
.swath-map-landing[hidden] { display: none; }
.swath-map-landing-status { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.swath-map-landing-status[hidden] { display: none; }
.swath-map-landing button {
  flex: none;
  margin: 0;
  padding: 1px 8px;
  border: 1px solid transparent;
  border-radius: var(--swath-radius-sm);
  background: color-mix(in srgb, var(--swath-color-fg) 12%, transparent);
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.swath-map-landing button[hidden] { display: none; }
.swath-map-landing button:focus-visible { outline: var(--swath-border-focus); outline-offset: 1px; }
.swath-map-landing-play { color: var(--swath-color-accent); font-weight: 700; }
.swath-map-landing-invite {
  border-color: var(--swath-color-accent-border);
  background: var(--swath-color-accent-bg);
  color: var(--swath-color-accent);
  font-weight: 700;
}
.swath-map-landing-invite:hover { background: color-mix(in srgb, var(--swath-color-accent) 22%, transparent); }
.swath-map-landing-dismiss {
  padding: 1px 5px;
  background: none;
  color: var(--swath-color-fg-muted);
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
/** The map's own toggles (#293): x-ray, compare, zoom-to-data as
 * `<swath-button>`s. Under a shell they sit in the HUD dock's `top-right`
 * slot; a bare map floats them top-right in-map. Same classes and
 * accessible names as the MapLibre controls they replace, so every suite
 * keeps its selectors; `hidden` follows the old `update()` rules. */
class MapToggles {
  readonly xray: SwathButton;
  readonly compare: SwathButton;
  readonly zoomData: SwathButton;
  readonly container: HTMLElement;
  #compareActive = false;
  #compareAvailable = false;

  constructor(host: SwathMap) {
    SwathButton.define();
    const make = (
      className: string,
      label: string,
      text: string,
      pressed?: boolean,
    ): SwathButton => {
      const button = document.createElement("swath-button");
      button.className = className;
      button.label = label;
      button.size = "sm";
      button.textContent = text;
      if (pressed !== undefined) {
        button.setAttribute("pressed", String(pressed));
      }
      return button;
    };
    this.xray = make(
      "swath-map-xray-toggle",
      "Toggle x-ray overlay",
      "X-ray",
      host.hasAttribute("xray"),
    );
    this.xray.addEventListener("swath-toggle", (event) => {
      event.stopPropagation();
      host.toggleAttribute("xray", event.detail.pressed);
    });
    this.compare = make("swath-map-compare-toggle", "Toggle compare swipe", "compare", false);
    this.compare.addEventListener("swath-toggle", (event) => {
      event.stopPropagation();
      host.toggleCompare();
    });
    this.zoomData = make("swath-map-zoomdata", "Zoom to the layer's data", "zoom to data");
    this.zoomData.addEventListener("click", () => host.zoomToData());
    this.container = document.createElement("div");
    this.container.className = "swath-map-toggles";
    this.container.append(this.xray, this.compare, this.zoomData);
    this.updateCompare(false, false);
    this.updateZoomData(false);
  }

  updateXray(pressed: boolean): void {
    this.xray.pressed = pressed;
  }

  updateCompare(active: boolean, available: boolean): void {
    this.#compareActive = active;
    this.#compareAvailable = available;
    this.compare.pressed = active;
    this.compare.hidden = !this.#compareActive && !this.#compareAvailable;
  }

  updateZoomData(known: boolean): void {
    this.zoomData.hidden = !known;
  }

  remove(): void {
    this.container.remove();
  }
}

/** Where the cinematic landing is (issue #211): `off` — nothing to
 * narrate (no `cinematic`, no playable domain); `playing` — the loop is
 * the component's; `hover` — paused under the pointer, resumes on
 * leave; `reduced` — waiting with a play affordance (reduced motion);
 * `over` — the user took over (the invitation stays, the status goes). */
type LandingState = "off" | "playing" | "hover" | "reduced" | "over";

/** The landing card's actions, wired by the host. */
interface LandingHooks {
  /** The reduced-motion play affordance: start the loop as the user. */
  play(): void;
  /** The invitation: turn the x-ray overlay on. */
  invite(): void;
}

/** The cinematic landing card (issue #211): a one-line status of the
 * loop plus the "watch the machine work" x-ray invitation. Pure DOM,
 * like the slider — the host decides the state. Exact state rides
 * `data-state` for the tests. Hidden while there is nothing to say,
 * while the x-ray overlay is on (the invitation has been accepted),
 * and after the viewer dismisses it. */
class LandingCard {
  readonly element: HTMLElement;
  readonly #status: HTMLSpanElement;
  readonly #play: HTMLButtonElement;
  #title = "";
  #frames = 0;
  #state: LandingState = "off";
  #xray = false;
  #dismissed = false;

  constructor(doc: Document, hooks: LandingHooks) {
    this.element = doc.createElement("div");
    this.element.className = "swath-map-landing";
    this.element.setAttribute("role", "status");
    this.element.hidden = true;

    this.#status = doc.createElement("span");
    this.#status.className = "swath-map-landing-status";

    this.#play = doc.createElement("button");
    this.#play.type = "button";
    this.#play.className = "swath-map-landing-play";
    this.#play.textContent = "play the season";
    this.#play.setAttribute("aria-label", "Play the season");
    this.#play.hidden = true;
    this.#play.addEventListener("click", () => hooks.play());

    const invite = doc.createElement("button");
    invite.type = "button";
    invite.className = "swath-map-landing-invite";
    invite.textContent = "watch the machine work →";
    invite.setAttribute(
      "aria-label",
      "Watch the machine work: turn on the x-ray overlay of every tile's render decision",
    );
    invite.addEventListener("click", () => hooks.invite());

    const dismiss = doc.createElement("button");
    dismiss.type = "button";
    dismiss.className = "swath-map-landing-dismiss";
    dismiss.textContent = "×";
    dismiss.setAttribute("aria-label", "Dismiss");
    dismiss.addEventListener("click", () => {
      this.#dismissed = true;
      this.#render();
    });

    this.element.append(this.#status, this.#play, invite, dismiss);
  }

  /** What the loop is over: the layer's title and its frame count. */
  set(title: string, frames: number): void {
    this.#title = title;
    this.#frames = frames;
    this.#render();
  }

  setState(state: LandingState): void {
    this.#state = state;
    this.#render();
  }

  /** The invitation is accepted while the overlay is on: card hidden. */
  setXray(on: boolean): void {
    this.#xray = on;
    this.#render();
  }

  #render(): void {
    const state = this.#state;
    this.element.dataset["state"] = state;
    this.element.hidden = state === "off" || this.#xray || this.#dismissed;
    this.#play.hidden = state !== "reduced";
    const status = {
      off: "",
      playing: `${this.#title} — ${this.#frames} frames, looping. Hover to pause, scrub or drag to take over.`,
      hover: `${this.#title} — paused. Move off the map to resume.`,
      reduced: `${this.#title} — ${this.#frames} frames, paused for reduced motion.`,
      over: "",
    }[state];
    this.#status.textContent = status;
    this.#status.hidden = status === "";
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
      "traces",
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
  #toggles: MapToggles | undefined;
  #xray: XRayOverlay | undefined;
  #time: TimeSlider | undefined;
  #landing: LandingCard | undefined;
  /** The cinematic landing (issue #211) has started — or offered — its
   * loop for the applied layer; a re-apply of the same view (liveness
   * refresh) leaves a running loop alone. */
  #cinematicArmed = false;
  /** The user took over (scrub, play button, drag, layer switch): the
   * component never again starts, pauses, or resumes playback itself. */
  #cinematicOver = false;
  /** Reduced motion: the landing is waiting with a play affordance
   * rather than looping. */
  #cinematicReduced = false;
  /** The cinematic loop is paused under the pointer (resumes on leave). */
  #hoverPaused = false;
  /** The compare swipe (issue #210): present exactly while a coherent
   * compare is asked for via `compare-datetime` / `compare-layer`. */
  #compare: CompareView | undefined;
  /** How the viewed layer is shown (issue #282): the eye and the opacity
   * slider. Viewer state, not view state — never in the URL; reset on a
   * layer switch so a hidden layer cannot make the next one blank. */
  #layerVisible = true;
  #layerOpacity = 1;
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
  /** The layer's footprints, already bounded by its window — what
   * [`zoomToData`] narrows further to the viewed frame (#397). */
  #footprints: readonly DatedFootprint[] = [];
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

  /** Where the host wants the x-ray's display modes and analytics summary
   * (the rail under `view=xray`, issue #286). Readouts, feed and inspector
   * always go to HUD cards when a shell's dock exists; without a shell
   * every part floats in the map. Settable at any time — a live overlay
   * re-homes its chrome. */
  #xrayChrome: Pick<XRayChrome, "modes" | "analytics"> | undefined;
  /** The badge-less trace stream (#287). Connection policy: while the
   * x-ray overlay exists its stream is shared (`onEnvelope`); otherwise,
   * while the `traces` attribute asks for it (the shell's status bar is
   * on screen), this stream is open; without either, no SSE connection. */
  #traces: TraceStream | undefined;
  #cursorFrame: number | undefined;
  #pointer: { lng: number; lat: number } | undefined;

  #onEnvelope = (envelope: TraceEnvelope): void => {
    this.dispatchEvent(createSwathEvent("swath-trace", { envelope }));
  };

  /** One `swath-cursor` per animation frame at most. */
  #scheduleCursor(): void {
    if (this.#cursorFrame !== undefined) {
      return;
    }
    this.#cursorFrame = requestAnimationFrame(() => {
      this.#cursorFrame = undefined;
      const map = this.#map;
      if (!map) {
        return;
      }
      const at = this.#pointer ?? map.getCenter().wrap();
      this.dispatchEvent(
        createSwathEvent("swath-cursor", {
          lng: at.lng,
          lat: at.lat,
          zoom: map.getZoom(),
          source: this.#pointer ? "pointer" : "center",
        }),
      );
    });
  }

  #syncTraces(): void {
    const wanted = this.hasAttribute("traces") && !this.#xray;
    if (wanted && !this.#traces) {
      this.#traces = new TraceStream(
        this.xrayEventSource ?? ((url) => this.api.events(url)),
        this.#onEnvelope,
      );
    } else if (!wanted && this.#traces) {
      this.#traces.dispose();
      this.#traces = undefined;
    }
    if (this.#traces && this.#activeLayer !== "") {
      this.#traces.connect(`${this.server}/traces`);
    }
  }
  #xrayCards: HTMLElement[] = [];

  get xrayChrome(): Pick<XRayChrome, "modes" | "analytics"> | undefined {
    return this.#xrayChrome;
  }

  set xrayChrome(chrome: Pick<XRayChrome, "modes" | "analytics"> | undefined) {
    this.#xrayChrome = chrome;
    this.#xray?.setChrome(this.#buildXrayChrome());
  }

  /** HUD cards for the x-ray chrome, built once per overlay and removed
   * with it: readouts bottom-left, the trace feed bottom-right, the
   * why-view inspector on the right (ui-system.md §6). */
  #buildXrayChrome(): XRayChrome | undefined {
    const dock = this.closest("swath-shell")?.querySelector(":scope > swath-hud-dock");
    if (!dock) {
      return this.#xrayChrome ? { ...this.#xrayChrome } : undefined;
    }
    if (this.#xrayCards.length === 0) {
      SwathHudCard.define();
      const tier = this.closest("swath-shell")?.getAttribute("tier") ?? "wide";
      const card = (slot: string, label: string): SwathHudCard => {
        const el = document.createElement("swath-hud-card");
        el.slot = slot;
        el.dense = true;
        // 640–1023 (ui-system.md §6): HUD cards default collapsed.
        el.collapsible = true;
        el.collapsed = tier === "narrow";
        el.title = label;
        el.className = "swath-map-xray-card";
        el.setAttribute("aria-label", label);
        el.dataset["xray"] = slot;
        dock.append(el);
        return el;
      };
      const inspector = card("right", "Trace inspector");
      inspector.autoHide = true; // only there once a badge opens the why-view
      this.#xrayCards = [
        card("bottom-left", "X-ray readouts"),
        card("bottom-right", "Trace feed"),
        inspector,
      ];
    }
    const [readouts, feed, inspector] = this.#xrayCards;
    return {
      readouts,
      feed,
      inspector,
      modes: this.#xrayChrome?.modes ?? readouts,
      analytics: this.#xrayChrome?.analytics,
    };
  }

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

  #api: SwathApi | undefined;
  #ownApi: SwathApi | undefined;

  /** The API client (ui-system.md §4.4): injected by a host or test, else
   * built from `server` — same origin when the attribute is absent. */
  get api(): SwathApi {
    if (this.#api !== undefined) {
      return this.#api;
    }
    if (this.#ownApi === undefined || this.#ownApi.base !== this.server) {
      this.#ownApi = new SwathApi({ base: this.server });
    }
    return this.#ownApi;
  }

  set api(api: SwathApi) {
    this.#api = api;
  }

  connectedCallback(): void {
    injectStyles(this.ownerDocument);
    adoptTokens(this.ownerDocument); // the chrome's colours are tokens (#285)
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
    // Cursor readout (#287): the pointer while a mouse is over the map,
    // the centre otherwise; one event per frame.
    this.#map.on("mousemove", (event) => {
      this.#pointer = { lng: event.lngLat.wrap().lng, lat: event.lngLat.lat };
      this.#scheduleCursor();
    });
    this.#map.on("mouseout", () => {
      this.#pointer = undefined;
      this.#scheduleCursor();
    });
    this.#map.on("move", () => this.#scheduleCursor());
    this.#toggles = new MapToggles(this);
    this.#time = new TimeSlider(this.ownerDocument, {
      scrubTo: (datetime) => {
        this.setAttribute("datetime", datetime);
      },
      prefetch: (datetime) => {
        this.#prefetchFrame(datetime);
      },
      canAdvance: () => this.#map?.areTilesLoaded() ?? true,
      interact: () => {
        this.#endCinematic(false);
      },
    });
    this.#landing = new LandingCard(this.ownerDocument, {
      play: () => {
        // The reduced-motion affordance IS a user act: the loop it
        // starts is the user's (no hover-pause, and the URL follows).
        this.#endCinematic(false);
        this.#time?.play();
      },
      invite: () => {
        this.setAttribute("xray", "");
      },
    });
    this.#landing.setXray(this.hasAttribute("xray"));
    // A shell hosts the chrome in its HUD dock (issue #285): the slider at
    // bottom-center, the landing card at top-center. Without a shell (a
    // bare <swath-map>, every vitest mount) they float over the map.
    const dock = this.closest("swath-shell")?.querySelector(":scope > swath-hud-dock");
    const toggles = this.#toggles?.container;
    if (dock) {
      this.#time.element.slot = "bottom-center";
      this.#landing.element.slot = "top-center";
      dock.append(this.#time.element, this.#landing.element);
      if (toggles) {
        toggles.slot = "top-right";
        dock.append(toggles);
      }
    } else {
      this.append(this.#time.element, this.#landing.element);
      if (toggles) {
        this.append(toggles);
      }
    }
    this.#cinematicArmed = false;
    this.#hoverPaused = false;
    // Hover pauses the cinematic loop, leaving resumes it — on the host,
    // not the map container: the slider and cards float above the
    // container, and reaching for them must not count as leaving.
    this.addEventListener("pointerenter", this.#onPointerEnter);
    this.addEventListener("pointerleave", this.#onPointerLeave);
    // A user-driven move (drag, wheel, keyboard — MapLibre stamps those
    // with `originalEvent`; programmatic fits carry none) takes over.
    this.#map.on("movestart", (event) => {
      if ((event as { originalEvent?: Event }).originalEvent) {
        this.#endCinematic(true);
      }
    });
    if (this.hasAttribute("xray")) {
      this.#enableXRay();
    }
    this.#startApply();
  }

  /** The cinematic loop is the component's own right now: armed, not
   * handed over, not waiting on reduced motion. What the
   * `swath-timechange` event reports as `cinematic` — the shell's cue
   * that a frame change was nobody's doing. */
  get #cinematicPlaying(): boolean {
    return this.#cinematicArmed && !this.#cinematicOver && !this.#cinematicReduced;
  }

  readonly #onPointerEnter = (event: PointerEvent): void => {
    // Mouse only: a touch fires enter/leave around every tap.
    if (event.pointerType !== "mouse" || !this.#cinematicPlaying || !this.#time?.playing) {
      return;
    }
    this.#time.pause();
    this.#hoverPaused = true;
    this.#landing?.setState("hover");
  };

  readonly #onPointerLeave = (event: PointerEvent): void => {
    if (event.pointerType !== "mouse" || !this.#hoverPaused) {
      return;
    }
    this.#hoverPaused = false;
    if (this.#cinematicPlaying) {
      this.#time?.play();
      this.#landing?.setState("playing");
    }
  };

  /** Starts (or, under reduced motion, offers) the cinematic loop once
   * the applied layer's domain is known — the `cinematic` attribute's
   * second half. No-op without the attribute, after a takeover (the
   * invitation stays), below two frames, or when already running. */
  #armCinematic(title: string, frames: number): void {
    const landing = this.#landing;
    const time = this.#time;
    if (!landing || !time || !this.hasAttribute("cinematic")) {
      return;
    }
    if (this.#cinematicOver) {
      landing.setState("over");
      return;
    }
    if (frames < 2) {
      landing.setState("off");
      return;
    }
    if (this.#cinematicArmed) {
      return;
    }
    this.#cinematicArmed = true;
    landing.set(title, frames);
    this.#cinematicReduced = prefersReducedMotion();
    if (this.#cinematicReduced) {
      landing.setState("reduced");
      return;
    }
    landing.setState("playing");
    time.play();
  }

  /** The user took over (idempotent): the loop is theirs from here.
   * `pause` stops a cinematic loop that is still running — a drag or a
   * layer switch pauses so the user can look; the slider's own controls
   * pass false because the click that follows decides (a "pause" press
   * must not be undone into a restart). */
  #endCinematic(pause: boolean, options: { resetFrame?: boolean } = {}): void {
    if (this.#cinematicOver) {
      return;
    }
    const wasPlaying = this.#cinematicPlaying;
    this.#cinematicOver = true;
    if (!this.#cinematicArmed) {
      return;
    }
    if (pause && wasPlaying) {
      this.#time?.pause();
    }
    // A frame the loop advanced to is nobody's choice: a layer switch
    // that ends the loop drops it back to "latest" rather than carrying
    // a fire-season instant onto the next layer (where it may not even
    // resolve). A frame the user scrubbed to (a takeover before this
    // point) is theirs and stays.
    if (options.resetFrame === true && wasPlaying) {
      this.removeAttribute("datetime");
    }
    this.#landing?.setState("over");
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
    this.#traces?.dispose();
    this.#traces = undefined;
    if (this.#cursorFrame !== undefined) {
      cancelAnimationFrame(this.#cursorFrame);
      this.#cursorFrame = undefined;
    }
    this.#time?.element.remove(); // docked chrome lives outside this element (#285)
    this.#landing?.element.remove();
    this.#time = undefined;
    this.#landing = undefined;
    this.removeEventListener("pointerenter", this.#onPointerEnter);
    this.removeEventListener("pointerleave", this.#onPointerLeave);
    this.#map?.remove();
    this.#map = undefined;
    this.#switcher = undefined;
    this.#toggles?.remove();
    this.#toggles = undefined;
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
      case "traces":
        this.#syncTraces();
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
        this.#toggles?.updateXray(newValue !== null);
        this.#landing?.setXray(newValue !== null);
        break;
      default:
        break;
    }
  }

  /** Fetches the server's available layers from `/tilesets`. */
  async layers(): Promise<SwathLayer[]> {
    const response = await this.api.fetch("/tilesets", {
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
    this.#endCinematic(true, { resetFrame: true }); // a user's pick: the loop is over
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
  /** Show or hide the viewed layer's raster (both compare sides). */
  setLayerVisibility(visible: boolean): void {
    this.#layerVisible = visible;
    this.#applyPaint();
  }

  /** Opacity of the viewed layer's raster, 0–1 (both compare sides). */
  setLayerOpacity(opacity: number): void {
    this.#layerOpacity = Math.min(1, Math.max(0, opacity));
    this.#applyPaint();
  }

  get layerVisible(): boolean {
    return this.#layerVisible;
  }

  get layerOpacity(): number {
    return this.#layerOpacity;
  }

  /** Raster paint/layout onto every map that carries the swath layer; a
   * map whose style is still loading gets it on `styledata`. */
  #applyPaint(): void {
    for (const map of [this.#map, this.#compare?.map]) {
      if (!map) {
        continue;
      }
      const paint = (): void => {
        if (!map.getLayer(RASTER_LAYER_ID)) {
          return;
        }
        map.setLayoutProperty(
          RASTER_LAYER_ID,
          "visibility",
          this.#layerVisible ? "visible" : "none",
        );
        map.setPaintProperty(RASTER_LAYER_ID, "raster-opacity", this.#layerOpacity);
      };
      if (map.getLayer(RASTER_LAYER_ID)) {
        paint();
      } else {
        map.once("styledata", paint);
      }
    }
  }

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
    // The viewed frame's own footprint when there is one, else the whole
    // layer — so fit never becomes a no-op (#397).
    const frame = frameBounds(this.#frames, this.#footprints, this.getAttribute("datetime"));
    const bounds = frame
      ? { west: frame[0], south: frame[1], east: frame[2], north: frame[3] }
      : this.#dataBounds;
    if (!map || !bounds) {
      return;
    }
    map.once("moveend", () => {
      this.dispatchEvent(createSwathEvent("swath-framedata", { bounds }));
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
    this.#xray = new XRayOverlay(this, map, {
      createEventSource: this.xrayEventSource ?? ((url) => this.api.events(url)),
      chrome: this.#buildXrayChrome(),
      onEnvelope: this.#onEnvelope,
    });
    this.#syncTraces();
    this.#xray.connect(`${this.server}/traces`);
    this.#xray.setLayer(this.#activeLayer);
    // X-ray toggled on mid-compare: hand the fresh overlay the sides.
    if (this.#compareSides) {
      this.#xray.setCompare({
        fraction: clampSwipe(parseSwipe(this.getAttribute("swipe"))),
        sides: this.#compareSides,
      });
    }
    this.#xray.setFrame(this.getAttribute("datetime"));
  }

  /** Tears the x-ray overlay down (idempotent): closes its EventSource
   * and removes its DOM. */
  #disableXRay(): void {
    this.#xray?.dispose();
    this.#xray = undefined;
    for (const card of this.#xrayCards) {
      card.remove();
    }
    this.#xrayCards = [];
    this.#syncTraces();
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
   * bubbling `swath-timechange` is the page shell's URL-sync seam —
   * its `cinematic` flag marks a frame the landing loop advanced on its
   * own (issue #211), which no shareable URL should follow.
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
    this.#xray?.setFrame(datetime);
    this.dispatchEvent(
      createSwathEvent("swath-timechange", { datetime, cinematic: this.#cinematicPlaying }),
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
    const path = `/tilesets/${layer}/tiles`;
    const query = `datetime=${encodeURIComponent(datetime)}`;
    let budget = SwathMap.#PREFETCH_MAX;
    for (let y = nw.y; y <= se.y && budget > 0; y += 1) {
      for (let x = nw.x; x <= se.x && budget > 0; x += 1) {
        budget -= 1;
        // OGC path order z/row/col = z/y/x.
        this.api.fetch(`${path}/${z}/${y}/${x}?${query}`).catch(() => undefined);
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
      this.dispatchEvent(createSwathEvent("swath-error", { error }));
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
    if (epoch !== this.#epoch) {
      return;
    }
    // The default layer: the first tileset — or, for the cinematic
    // landing (issue #211), the first PLAYABLE one when any exists.
    let layerId = this.getAttribute("layer") ?? undefined;
    let picked: PlayablePick | undefined;
    if (layerId === undefined && this.hasAttribute("cinematic")) {
      picked = await this.#pickPlayable(available, epoch);
      layerId = picked?.id;
    }
    layerId ??= available[0]?.id;
    if (epoch !== this.#epoch || layerId === undefined) {
      return;
    }
    const title = available.find((layer) => layer.id === layerId)?.title ?? layerId;

    // The tileset metadata carries both the geographic bounds (the
    // zero-config fit below) and, for catalog-backed layers, the
    // granules link the time slider's domain comes from (issue #182).
    const fit = this.getAttribute("center") === null && this.getAttribute("zoom") === null;
    const metadata = picked?.metadata ?? (await this.#layerMetadata(layerId));
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
    if (layerId !== this.#activeLayer) {
      this.#layerVisible = true;
      this.#layerOpacity = 1;
    }
    map.setStyle(this.#buildStyle(this.#tileTemplate(layerId), basemap) as never);
    await applied;
    if (epoch !== this.#epoch) {
      return;
    }
    this.#applyPaint();
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
    this.#syncTraces();
    this.#scheduleCursor(); // the status bar reads the centre before any move (#287)
    this.#xray?.setLayer(layerId);
    this.#switcher?.update(available, layerId);
    // `layers` rides along (issue #108) so page chrome — the entry page's
    // layer browser — can list what exists without a second /tilesets
    // fetch that could disagree with the one this apply used.
    // Both names for one milestone (ui-system.md §4.3): hosts move to
    // `swath-layer-change`; `layerchange` is the M5 name.
    const detail = {
      layer: layerId,
      layers: available,
      visible: this.#layerVisible,
      opacity: this.#layerOpacity,
    };
    this.dispatchEvent(createSwathEvent("swath-layer-change", detail));
    this.dispatchEvent(createSwathEvent("layerchange", detail));
    this.#startLivenessProbe(layerId, epoch);
    // The data domain loads LAST, after the probe kicked off: the
    // probe's timing relative to MapLibre's first tile fetches is part
    // of the painted result (its no-store fetch renders through the
    // cache, so reordering it flips a badge between live and cache_hit),
    // while the domain only feeds the slider and the data-framing.
    // `ready` still covers it — tests may await and then inspect.
    await this.#applyDataDomain(title, metadata, epoch, picked?.domain);
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
        const response = await this.api.fetch(url, { cache: "no-store" });
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
    const response = await this.api.fetch(`/tilesets/${layerId}`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) {
      return undefined;
    }
    const body = (await response.json()) as {
      boundingBox?: { lowerLeft?: number[]; upperRight?: number[] };
      links?: OgcLink[];
      "swath:window"?: [string | null, string | null];
      "swath:sources"?: number;
    };
    const metadata: LayerMetadata = {};
    // The frames the layer can serve (ADR 0015 / ADR 0022, #301): the
    // slider never offers a granule date outside the window.
    const window = body["swath:window"];
    if (Array.isArray(window) && window.length === 2) {
      metadata.window = [
        typeof window[0] === "string" ? window[0] : null,
        typeof window[1] === "string" ? window[1] : null,
      ];
    }
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

  /** The `cinematic` default (issue #211): the first listed layer whose
   * dataset has two or more granule dates — a playable time series —
   * within the scan bound, with the metadata and domain already read
   * (the apply reuses both: no second round trip). Undefined when no
   * scanned layer is playable, or when a newer apply superseded this
   * one mid-scan. */
  async #pickPlayable(
    available: readonly SwathLayer[],
    epoch: number,
  ): Promise<PlayablePick | undefined> {
    const seen = new Set<string>();
    for (const layer of available.slice(0, CINEMATIC_SCAN_MAX)) {
      const metadata = await this.#layerMetadata(layer.id);
      if (epoch !== this.#epoch) {
        return undefined;
      }
      const dataset = metadata?.dataset;
      if (metadata === undefined || dataset === undefined || seen.has(dataset)) {
        continue;
      }
      seen.add(dataset);
      const domain = await this.#fetchDomain(dataset);
      if (epoch !== this.#epoch) {
        return undefined;
      }
      if (domain.frames.length >= 2) {
        return { id: layer.id, metadata, domain };
      }
    }
    return undefined;
  }

  /** A dataset's granule listing, reduced to what the component needs:
   * the acquisition datetimes (the slider's domain) and the footprints
   * (the data bounds). Failures yield the empty domain — both are bonus
   * affordances, never a reason the imagery fails to paint. */
  async #fetchDomain(dataset: string): Promise<LayerDomain> {
    try {
      const url = `/datasets/${encodeURIComponent(dataset)}/granules`;
      const response = await this.api.fetch(url, { headers: { accept: "application/json" } });
      if (!response.ok) {
        return EMPTY_DOMAIN;
      }
      const body: unknown = await response.json();
      return {
        frames: parseGranuleDatetimes(body),
        footprints: (
          (body as { granules?: { bbox?: unknown; datetime?: unknown }[] }).granules ?? []
        )
          .map((granule) => ({
            datetime: typeof granule.datetime === "string" ? granule.datetime : "",
            bbox: parseBbox(granule.bbox),
          }))
          .filter((f): f is DatedFootprint => f.bbox !== undefined),
      };
    } catch {
      return EMPTY_DOMAIN;
    }
  }

  /** Feeds the time slider AND the data-framing (issue #182): fetches
   * the layer's granule listing (when its metadata linked one, unless
   * the cinematic scan already did), hands the acquisition datetimes to
   * the slider, and records where the data IS — the union of granule
   * footprints, falling back to the tileset metadata bounds for static
   * layers, unknown when neither exists (the zoom-to-data control hides
   * then). Last, the cinematic landing gets its chance (issue #211). */
  async #applyDataDomain(
    title: string,
    metadata: LayerMetadata | undefined,
    epoch: number,
    prefetched: LayerDomain | undefined,
  ): Promise<void> {
    const domain =
      prefetched ??
      (metadata?.dataset === undefined ? EMPTY_DOMAIN : await this.#fetchDomain(metadata.dataset));
    if (epoch !== this.#epoch) {
      return;
    }
    // Footprints are bounded by the SAME window that bounds the frames
    // (#397). They were not, so the data bounds could include granules the
    // layer will never render — and "zoom to data" would fit an area the
    // map cannot fill.
    const inWindow = new Set(boundDomain(domain.frames, metadata?.window));
    this.#footprints = domain.footprints.filter(
      (f) => f.datetime === "" || inWindow.has(f.datetime),
    );
    const union = unionBbox(this.#footprints.map((f) => f.bbox));
    this.#dataBounds = union
      ? { west: union[0], south: union[1], east: union[2], north: union[3] }
      : metadata?.bounds;
    this.#toggles?.updateZoomData(this.#dataBounds !== undefined);
    const frames = boundDomain(domain.frames, metadata?.window);
    this.#frames = frames;
    this.#time?.setDomain(frames, this.getAttribute("datetime"));
    this.#armCinematic(title, frames.length);
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
    this.#toggles?.updateCompare(spec !== undefined, this.#frames.length >= 2);
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
      this.#applyPaint();
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
      createSwathEvent("swath-comparechange", {
        compareTime: this.getAttribute("compare-datetime"),
        compareLayer: this.getAttribute("compare-layer"),
        swipe: this.getAttribute("swipe"),
      }),
    );
  }
}

/** What the cinematic scan hands the apply: the chosen layer with its
 * metadata and domain already fetched. */
interface PlayablePick {
  id: string;
  metadata: LayerMetadata;
  domain: LayerDomain;
}

/** Registers `<swath-map>`; safe to call more than once. */
export function defineSwathMap(): void {
  if (!customElements.get(SwathMap.tagName)) {
    customElements.define(SwathMap.tagName, SwathMap);
  }
}
