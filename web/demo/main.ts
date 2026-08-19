// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The entry page's app shell (issue #108). Semantics live in
// src/view-state.ts; this file only wires them to the DOM:
//
// - Initial view: URL params beat localStorage beats zero-config default
//   (`resolveInitialState`). Applied as plain attributes BEFORE define()
//   so the elements upgrade with them; a bare URL with an empty storage
//   stays exactly the zero-config demo it always was.
// - The URL is the share link: user interactions (layer selection, map
//   movement, the x-ray toggle) rewrite the query via replaceState. Loads
//   and programmatic moves never do — deep links stay byte-stable, and a
//   pasted URL is never rewritten (`viewStatesEqual` guards even the
//   interaction path against no-op writes).
// - localStorage tracks every state change as "the last session", so a
//   later paramless visit resumes where this one ended.
import { type GranuleBbox, GranuleFootprints } from "../src/granule-footprints.js";
import { defineSwathAddDataPanel, SwathAddDataPanel } from "../src/swath-add-data-panel.js";
import { defineSwathAuthoringPanel, SwathAuthoringPanel } from "../src/swath-authoring-panel.js";
import {
  defineSwathDatasetPanel,
  type GranuleListItem,
  SwathDatasetPanel,
} from "../src/swath-dataset-panel.js";
import { defineSwathLayerPanel, SwathLayerPanel } from "../src/swath-layer-panel.js";
import { defineSwathMap, type SwathLayer, SwathMap } from "../src/swath-map.js";
import {
  formatCenter,
  formatSwipe,
  formatZoom,
  parseSwipe,
  parseTime,
  parseViewState,
  resolveInitialState,
  safeLocalStorage,
  saveViewState,
  type ViewState,
  viewStatesEqual,
  withViewState,
} from "../src/view-state.js";

const mapElement = document.querySelector("swath-map");
const panelElement = document.querySelector("swath-layer-panel");
const datasetElement = document.querySelector("swath-dataset-panel");
const addDataElement = document.querySelector("swath-add-data-panel");
const authoringElement = document.querySelector("swath-authoring-panel");

const storage = safeLocalStorage();
const { state: initial } = resolveInitialState(location.search, storage);

if (initial.layer !== undefined) {
  mapElement?.setAttribute("layer", initial.layer);
}
if (initial.center) {
  mapElement?.setAttribute("center", formatCenter(initial.center));
}
if (initial.zoom !== undefined) {
  mapElement?.setAttribute("zoom", formatZoom(initial.zoom));
}
if (initial.time !== undefined) {
  mapElement?.setAttribute("datetime", initial.time);
}
// Compare swipe (issue #210): the parsed state is already coherent
// (exclusive modes, validated swipe) — apply verbatim.
if (initial.compareTime !== undefined) {
  mapElement?.setAttribute("compare-datetime", initial.compareTime);
}
if (initial.compareLayer !== undefined) {
  mapElement?.setAttribute("compare-layer", initial.compareLayer);
}
if (initial.swipe !== undefined) {
  mapElement?.setAttribute("swipe", formatSwipe(initial.swipe));
}
if (initial.xray) {
  mapElement?.setAttribute("xray", "");
}
// `basemap` is pass-through page config, not view state: never persisted,
// preserved in rewritten URLs by `withViewState`.
const basemap = new URLSearchParams(location.search).get("basemap");
if (basemap !== null) {
  mapElement?.setAttribute("basemap", basemap);
}
// `stac` (issue #197) is the "Open in Swath" entry: pass-through page
// config exactly like `basemap` — not view state, never persisted, never
// rewritten on load (byte-stability), preserved by `withViewState`. The
// add-data panel opens pre-filled; registering stays a user click.
const stac = new URLSearchParams(location.search).get("stac");
if (stac !== null && stac !== "") {
  addDataElement?.setAttribute("stac", stac);
}

defineSwathMap();
defineSwathLayerPanel();
defineSwathDatasetPanel();
defineSwathAddDataPanel();
defineSwathAuthoringPanel();

if (mapElement instanceof SwathMap && panelElement instanceof SwathLayerPanel) {
  wire(mapElement, panelElement);
}
if (mapElement instanceof SwathMap && authoringElement instanceof SwathAuthoringPanel) {
  wireAuthoring(mapElement, authoringElement);
}

if (mapElement instanceof SwathMap && datasetElement instanceof SwathDatasetPanel) {
  wireDatasetBrowser(mapElement, datasetElement);
}

// The add-data panel (issue #197) registers through the API and publishes
// a quick-look service; the shell only routes the outcome to the map —
// the switch refetches /tilesets, so the rail lists the new layer at once.
if (mapElement instanceof SwathMap && addDataElement instanceof SwathAddDataPanel) {
  const map = mapElement;
  addDataElement.addEventListener("swath-data-added", (event) => {
    const layer = (event as CustomEvent<{ dataset: string; layer: string }>).detail.layer;
    if (layer !== "") {
      // Failures surface via the map's own `swath-error` + retry loop.
      map.setLayer(layer).catch(() => undefined);
    }
  });
}

/** Routes the dataset browser (issue #110) to the map: announced granules
 * become the footprint layer, a granule click becomes a bounds fit. The
 * panel fetches lazily on its own; nothing here runs until it announces. */
function wireDatasetBrowser(map: SwathMap, panel: SwathDatasetPanel): void {
  let footprints: GranuleFootprints | undefined;
  const paint = (): GranuleFootprints | undefined => {
    const inner = map.map;
    if (!inner) {
      return undefined;
    }
    footprints ??= new GranuleFootprints(inner);
    return footprints;
  };
  panel.addEventListener("swath-dataset-granules", (event) => {
    const detail = (event as CustomEvent<{ dataset: string; granules: GranuleListItem[] }>).detail;
    paint()?.set(detail.granules);
  });
  panel.addEventListener("swath-granule-zoom", (event) => {
    const detail = (event as CustomEvent<{ bbox: GranuleBbox }>).detail;
    paint()?.zoomTo(detail.bbox);
  });
}

function wire(map: SwathMap, panel: SwathLayerPanel): void {
  /** The layer the last successful apply painted (undefined until the
   * first `layerchange`) — distinguishes a layer CHANGE, which updates
   * the URL, from the initial apply, which must not touch it. */
  let appliedLayer: string | undefined;

  const snapshot = (): ViewState => {
    const state: ViewState = { xray: map.hasAttribute("xray") };
    if (appliedLayer !== undefined) {
      state.layer = appliedLayer;
    }
    // The viewed frame (issue #182): the map's `datetime` attribute is
    // the source of truth; re-validated through the same parser the URL
    // uses so only well-formed instants ever enter a ViewState.
    const time = parseTime(map.getAttribute("datetime"));
    if (time !== undefined) {
      state.time = time;
    }
    // Compare swipe (issue #210): same attribute-is-truth rule, with the
    // same exclusivity the URL parser enforces — only coherent compare
    // states ever enter a ViewState (and `swipe` only rides a compare).
    const compareTime = parseTime(map.getAttribute("compare-datetime"));
    const compareLayer = map.getAttribute("compare-layer");
    if (compareTime !== undefined && compareLayer === null) {
      state.compareTime = compareTime;
    } else if (
      compareLayer !== null &&
      compareLayer !== "" &&
      compareTime === undefined &&
      compareLayer !== state.layer
    ) {
      state.compareLayer = compareLayer;
    }
    if (state.compareTime !== undefined || state.compareLayer !== undefined) {
      const swipe = parseSwipe(map.getAttribute("swipe"));
      if (swipe !== undefined) {
        state.swipe = swipe;
      }
    }
    const inner = map.map;
    if (inner) {
      const center = inner.getCenter().wrap();
      state.center = [center.lng, center.lat];
      state.zoom = inner.getZoom();
    }
    return state;
  };

  const persist = (): void => {
    if (storage) {
      saveViewState(storage, snapshot());
    }
  };

  const syncUrl = (): void => {
    const state = snapshot();
    if (viewStatesEqual(parseViewState(location.search), state)) {
      return; // the URL already says this — leave its bytes alone
    }
    history.replaceState(
      null,
      "",
      `${location.pathname}${withViewState(location.search, state)}${location.hash}`,
    );
  };

  map.addEventListener("layerchange", (event) => {
    const detail = (event as CustomEvent<{ layer: string; layers: SwathLayer[] }>).detail;
    panel.update(detail.layers, detail.layer);
    const changed = appliedLayer !== undefined && appliedLayer !== detail.layer;
    appliedLayer = detail.layer;
    persist();
    if (changed) {
      syncUrl();
    }
  });

  panel.addEventListener("swath-layer-select", (event) => {
    const layer = (event as CustomEvent<{ layer: string }>).detail.layer;
    // Failures surface via the map's own `swath-error` + retry loop.
    map.setLayer(layer).catch(() => undefined);
  });

  // User-driven movement (drag, wheel, keyboard — MapLibre stamps those
  // with `originalEvent`) updates the share link; programmatic moves
  // (bounds fits, attribute jumps) only update the remembered session.
  map.map?.on("moveend", (event) => {
    persist();
    if ((event as { originalEvent?: Event }).originalEvent) {
      syncUrl();
    }
  });

  // The x-ray toggle flips the host attribute; mirror it into URL and
  // storage. The observer attaches after the initial attributes landed,
  // so a deep-linked `?xray` does not count as an interaction.
  new MutationObserver(() => {
    persist();
    syncUrl();
  }).observe(map, { attributes: true, attributeFilter: ["xray"] });

  // Time scrub/play (issue #182): the map announces every frame change
  // (slider scrub, play tick, or a programmatic attribute set) — mirror
  // it into URL and storage so deep links carry time. A deep-linked `t`
  // is applied as the attribute BEFORE define(), and upgrade-time
  // attribute callbacks run before the map exists (the component
  // ignores them) — no event fires on load, so pasted links stay
  // byte-stable; `syncUrl` additionally skips the write whenever the
  // URL already encodes the same state.
  map.addEventListener("swath-timechange", () => {
    persist();
    syncUrl();
  });

  // Compare swipe (issue #210): the map announces every compare-state
  // change (toggle, handle drag, or a programmatic attribute set) — the
  // same mirror-into-URL-and-storage seam as time. Deep-linked compare
  // attributes are applied BEFORE define(), so no event fires on load
  // and pasted links stay byte-stable.
  map.addEventListener("swath-comparechange", () => {
    persist();
    syncUrl();
  });

  // Data framing (issue #182 follow-up): both the auto-frame after a
  // user-initiated layer switch and the `zoom to data` control announce
  // their move — user-driven view changes, so the share link follows
  // (programmatic moves otherwise deliberately never rewrite the URL).
  map.addEventListener("swath-framedata", () => {
    persist();
    syncUrl();
  });
}

// The authoring panel (issue #109) is a pure openEO client; the shell
// only routes its outcomes to the map. A created service becomes the
// viewed layer (the switch refetches /tilesets, so the layer browser
// lists it immediately — no reload); a deleted one falls back to the
// server's default layer when it was the viewed one, else just refreshes
// the layer list.
function wireAuthoring(map: SwathMap, authoring: SwathAuthoringPanel): void {
  authoring.addEventListener("swath-service-created", (event) => {
    const id = (event as CustomEvent<{ id: string }>).detail.id;
    map.setLayer(id).catch(() => undefined);
  });
  authoring.addEventListener("swath-service-deleted", (event) => {
    const id = (event as CustomEvent<{ id: string }>).detail.id;
    if (map.getAttribute("layer") === id) {
      map.removeAttribute("layer"); // re-applies with the server default
    } else {
      map.refresh();
    }
  });
}
