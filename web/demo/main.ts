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
import {
  defineSwathDatasetPanel,
  type GranuleListItem,
  SwathDatasetPanel,
} from "../src/swath-dataset-panel.js";
import { defineSwathLayerPanel, SwathLayerPanel } from "../src/swath-layer-panel.js";
import { defineSwathMap, type SwathLayer, SwathMap } from "../src/swath-map.js";
import {
  formatCenter,
  formatZoom,
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
if (initial.xray) {
  mapElement?.setAttribute("xray", "");
}
// `basemap` is pass-through page config, not view state: never persisted,
// preserved in rewritten URLs by `withViewState`.
const basemap = new URLSearchParams(location.search).get("basemap");
if (basemap !== null) {
  mapElement?.setAttribute("basemap", basemap);
}

defineSwathMap();
defineSwathLayerPanel();
defineSwathDatasetPanel();

if (mapElement instanceof SwathMap && panelElement instanceof SwathLayerPanel) {
  wire(mapElement, panelElement);
}

if (mapElement instanceof SwathMap && datasetElement instanceof SwathDatasetPanel) {
  wireDatasetBrowser(mapElement, datasetElement);
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
}
