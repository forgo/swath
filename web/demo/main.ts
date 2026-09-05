// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { onServiceDeleted, wireAuthoring } from "../src/app/authoring.js";
import { wirePalette } from "../src/app/palette.js";
import { wireReflow } from "../src/app/reflow.js";
import { wireShare } from "../src/app/share.js";
import {
  type AppState,
  appStatesEqual,
  inspectorOpen,
  isViewMode,
  parseAppState,
  resolveInitialAppState,
  saveAppPreference,
  type ViewMode,
  withAppState,
} from "../src/app-state.js";
// The entry page's app shell (issue #108). Semantics live in
// src/view-state.ts; this file only wires them to the DOM:
//
// - Initial view: URL params beat localStorage beats zero-config default
//   (`resolveInitialState`). Applied as plain attributes BEFORE define()
//   so the elements upgrade with them; a bare URL with an empty storage
//   stays exactly the zero-config demo it always was.
// - The URL is the share link: user interactions (layer selection, map
//   movement, the x-ray toggle, a scrub) rewrite the query via
//   replaceState. Loads and programmatic moves never do — deep links stay
//   byte-stable, and a pasted URL is never rewritten (`viewStatesEqual`
//   guards even the interaction path against no-op writes). The Share
//   button (issue #211) copies that same canonical URL — written in full,
//   so it works from a bare landing too.
// - localStorage tracks every state change FROM THE FIRST INTERACTION ON
//   as "the last session", so a later paramless visit resumes where this
//   one ended. Before the first interaction there is no session to
//   remember: a visit that only watched the landing loop leaves nothing
//   behind, so the next paramless visit is the landing again.
// - The cinematic landing (issue #211): only the zero-config default
//   (no URL params, no stored session) gets the `cinematic` attribute —
//   the fire-season loop auto-plays there and nowhere else. Its own
//   frame advances are not interactions (`swath-timechange` flags them
//   `cinematic`): the bare URL stays bare until the user takes over.
import {
  type GranuleBbox,
  GranuleFootprints,
  HOVER_SOURCE_ID,
  SCOPE_SOURCE_ID,
} from "../src/granule-footprints.js";
import { ResultDensity } from "../src/result-density.js";
import { formatCrs, formatIngest, formatLonLat, formatZoomCell } from "../src/status-model.js";
import { safeLocalStorage } from "../src/storage.js";
import { defineSwathAddDataPanel, SwathAddDataPanel } from "../src/swath-add-data-panel.js";
import { defineSwathAuthoringPanel, SwathAuthoringPanel } from "../src/swath-authoring-panel.js";
import { defineSwathCatalog, SwathCatalog } from "../src/swath-catalog.js";
import { defineSwathImport, SwathImport } from "../src/swath-import.js";
import { defineSwathLayerList, SwathLayerList } from "../src/swath-layer-list.js";
import { defineSwathMap, SwathMap } from "../src/swath-map.js";
import { defineSwathShell, SwathShell } from "../src/swath-shell.js";
import { defineSwathSources, SwathSources } from "../src/swath-sources.js";
import { SwathButton } from "../src/ui/button.js";
import { type Chip, SwathChipRow } from "../src/ui/chip-row.js";
import { SwathCommandPalette } from "../src/ui/command-palette.js";
import { SwathDrawer } from "../src/ui/drawer.js";
import { SwathExplainCard } from "../src/ui/explain-card.js";
import { SwathHudDock } from "../src/ui/hud-dock.js";
import { SwathRail } from "../src/ui/rail.js";
import { SwathStatusBar, SwathStatusCell } from "../src/ui/status-bar.js";
import {
  formatCenter,
  formatSwipe,
  formatZoom,
  parseSwipe,
  parseTime,
  parseViewState,
  resolveInitialState,
  saveViewState,
  type ViewState,
  viewArtifactsEqual,
  viewStatesEqual,
  withViewState,
} from "../src/view-state.js";

const mapElement = document.querySelector("swath-map");
const panelElement = document.querySelector("swath-layer-list");
const datasetElement = document.querySelector("swath-catalog");
const sourcesElement = document.querySelector("swath-sources");
const importElement = document.querySelector("swath-import");
const addDataElement = document.querySelector("swath-add-data-panel");
const authoringElement = document.querySelector("swath-authoring-panel");
SwathButton.define();
SwathRail.define();
SwathHudDock.define();
SwathDrawer.define();
SwathCommandPalette.define();
SwathChipRow.define();
SwathExplainCard.define();
SwathStatusBar.define();
SwathStatusCell.define();
defineSwathShell();
const shellElement = document.querySelector("swath-shell");
const railElement = document.querySelector("swath-rail");
const shareElement = document.querySelector<SwathButton>("#swath-share");

const storage = safeLocalStorage();
const { state: initial, source } = resolveInitialState(location.search, storage);
// The mode (issue #283): `view=` beats storage beats `layers`, exactly the
// view-state precedence, resolved on the same search string.
const { state: initialApp } = resolveInitialAppState(location.search, storage);

// Nobody asked for a view: open on the season loop (when the server has
// one). A deep link or a restored session is explicit state — honored
// exactly, never animated over (the issue #108/#227 precedence).
if (source === "default") {
  mapElement?.setAttribute("cinematic", "");
}
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
// A `view=xray` link opens with the overlay on; applied here with the other
// initial attributes (before any observer), so the link's bytes stay its own.
if (initialApp.view === "xray") {
  mapElement?.setAttribute("xray", "");
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
defineSwathLayerList();
defineSwathCatalog();
defineSwathSources();
defineSwathImport();
defineSwathAddDataPanel();
defineSwathAuthoringPanel();

if (mapElement instanceof SwathMap && panelElement instanceof SwathLayerList) {
  wire(mapElement, panelElement);
}
if (mapElement instanceof SwathMap && authoringElement instanceof SwathAuthoringPanel) {
  wireAuthoring(
    mapElement,
    authoringElement,
    document.querySelector<SwathExplainCard>("#swath-explain"),
  );
}

if (mapElement instanceof SwathMap && datasetElement instanceof SwathCatalog) {
  wireDatasetBrowser(mapElement, datasetElement);
}

// The add-data panel (issue #197) registers through the API and publishes
// a quick-look service; the shell only routes the outcome to the map —
// the switch refetches /tilesets, so the rail lists the new layer at once.
if (mapElement instanceof SwathMap && addDataElement instanceof SwathAddDataPanel) {
  const map = mapElement;
  addDataElement.addEventListener("swath-data-added", (event) => {
    const layer = event.detail.layer;
    if (layer !== "") {
      // Failures surface via the map's own `swath-error` + retry loop.
      map.setLayer(layer).catch(() => undefined);
    }
  });
}

/** `state` with the search scope's dates set, dropping the keys that have
 * no value — the shape `exactOptionalPropertyTypes` asks for, and the one
 * `withAppState` writes from. */
function withStep(state: AppState, step: string | undefined): AppState {
  const next: AppState = { view: state.view };
  for (const [key, value] of [
    ["sel", state.sel],
    ["rail", state.rail],
    ["from", state.from],
    ["to", state.to],
    ["step", step],
  ] as const) {
    if (value !== undefined) {
      Object.assign(next, { [key]: value });
    }
  }
  return next;
}

function withDates(state: AppState, from: string | undefined, to: string | undefined): AppState {
  const next: AppState = { view: state.view };
  if (state.sel !== undefined) {
    next.sel = state.sel;
  }
  if (state.rail !== undefined) {
    next.rail = state.rail;
  }
  if (from !== undefined) {
    next.from = from;
  }
  if (to !== undefined) {
    next.to = to;
  }
  return next;
}

/** Routes the dataset browser (issue #110) to the map: announced granules
 * become the footprint layer, a granule click becomes a bounds fit. The
 * panel fetches lazily on its own; nothing here runs until it announces. */
function wireDatasetBrowser(map: SwathMap, panel: SwathCatalog): void {
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
    const detail = event.detail;
    paint()?.set(detail.granules);
  });
  // The search scope's box (#412), drawn so a reduced shape is seen and
  // not only read. Its own source, dashed: a box the user did not draw is
  // not drawn like one.
  let scope: GranuleFootprints | undefined;
  panel.addEventListener("swath-scope", (event) => {
    const inner = map.map;
    if (!inner) {
      return;
    }
    scope ??= new GranuleFootprints(inner, SCOPE_SOURCE_ID, true);
    const bbox = event.detail.bbox;
    scope.set(
      bbox === null || bbox.length !== 4
        ? []
        : [{ id: "scope", bbox: bbox as unknown as GranuleBbox }],
    );
  });
  // The hovered result's own outline (#413): its own source, so hovering
  // never repaints the footprint set underneath it.
  let hover: GranuleFootprints | undefined;
  panel.addEventListener("swath-granule-hover", (event) => {
    const inner = map.map;
    if (!inner) {
      return;
    }
    hover ??= new GranuleFootprints(inner, HOVER_SOURCE_ID);
    const bbox = event.detail.bbox;
    hover.set(
      bbox === null || bbox.length !== 4
        ? []
        : [{ id: event.detail.id, bbox: bbox as unknown as GranuleBbox }],
    );
  });
  // Where the results are, when there are too many outlines to read.
  let density: ResultDensity | undefined;
  panel.addEventListener("swath-dataset-density", (event) => {
    const inner = map.map;
    if (!inner) {
      return;
    }
    density ??= new ResultDensity(inner);
    density.set(
      event.detail.cells.map((cell) => ({
        bbox: cell.bbox as unknown as GranuleBbox,
        count: cell.count,
        weight: cell.weight,
      })),
    );
  });
  panel.addEventListener("swath-granule-zoom", (event) => {
    const detail = event.detail;
    paint()?.zoomTo(detail.bbox);
  });
  // The "in current view" filter follows the map (issue #288).
  const follow = (): void => {
    const inner = map.map;
    if (!inner) {
      return;
    }
    const b = inner.getBounds();
    panel.viewBounds = {
      west: b.getWest(),
      south: b.getSouth(),
      east: b.getEast(),
      north: b.getNorth(),
    };
  };
  map.map?.on("moveend", follow);
  map.addEventListener("swath-layer-change", follow);
}

function wire(map: SwathMap, panel: SwathLayerList): void {
  /** The layer the last successful apply painted (undefined until the
   * first `layerchange`) — distinguishes a layer CHANGE, which updates
   * the URL, from the initial apply, which must not touch it. */
  let appliedLayer: string | undefined;
  /** Has the user done anything yet? Until then nothing is written —
   * not the URL (byte-stable loads) and not storage (no session to
   * remember): the cinematic landing's own frame advances and the
   * programmatic fits around them are nobody's doing. */
  let interacted = false;

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
    if (storage && interacted) {
      saveViewState(storage, snapshot());
    }
  };

  // View params and app params compose on ONE search string (ui-system.md
  // §4.5): view-state first, then app-state; each passes the other's
  // params through byte-for-byte, and a write that changes nothing is
  // skipped for both.
  let appState: AppState = initialApp;
  /** The artifact half of the last state written to the URL (#392). */
  let lastArtifact: ViewState | undefined;

  // The chip row IS the URL, rendered (#393): one chip per thing the view is
  // of. It is derived from the same snapshot the URL is written from, so the
  // two can never disagree — there is no second source of truth to drift.
  const chipRow = document.querySelector<SwathChipRow>("#swath-chips");
  const renderChips = (): void => {
    if (!chipRow) {
      return;
    }
    const state = snapshot();
    const chips: Chip[] = [];
    if (state.layer !== undefined) {
      // Not removable: with no layer there is no view to show.
      chips.push({ id: "layer", label: "layer", value: state.layer });
    }
    if (state.time !== undefined) {
      chips.push({ id: "time", label: "date", value: state.time, removable: true });
    }
    if (state.compareLayer !== undefined) {
      chips.push({ id: "compare", label: "vs", value: state.compareLayer, removable: true });
    } else if (state.compareTime !== undefined) {
      chips.push({ id: "compare", label: "vs", value: state.compareTime, removable: true });
    }
    if (appState.step !== undefined) {
      // The import's own chip: the step is what a resumed link points at.
      chips.push({ id: "step", label: "import", value: appState.step, removable: true });
    }
    if (appState.from !== undefined || appState.to !== undefined) {
      // One chip for the scope, reading the way a person says it (#411).
      chips.push({
        id: "dates",
        label: "dates",
        value: `${appState.from ?? "any"} to ${appState.to ?? "any"}`,
        removable: true,
      });
    }
    if (state.xray) {
      chips.push({ id: "xray", label: "x-ray", value: "on", removable: true });
    }
    chipRow.chips = chips;
  };
  chipRow?.addEventListener("swath-chip-remove", (event) => {
    // Removing a chip is a state change like any other: it goes through the
    // same write path, so it pushes history and the back button restores it.
    switch (event.detail.chip) {
      case "time":
        map.removeAttribute("datetime");
        break;
      case "compare":
        map.removeAttribute("compare-layer");
        map.removeAttribute("compare-datetime");
        map.removeAttribute("swipe");
        break;
      case "dates":
        appState = withDates(appState, undefined, undefined);
        if (datasetElement instanceof SwathCatalog) {
          datasetElement.dates = { from: undefined, to: undefined };
        }
        break;
      case "xray":
        map.removeAttribute("xray");
        break;
      default:
        return;
    }
    interact();
    syncUrl();
    renderChips();
  });
  /** `transient: true` forces a replace even when the artifact changed — for
   * state the app is driving rather than the person: the cinematic loop's
   * frame advances (#392). A loop that pushed a frame per second would fill
   * history with a slideshow and make `back` useless. */
  const syncUrl = (options: { transient?: boolean } = {}): void => {
    if (!interacted || restoring) {
      return;
    }
    const state = snapshot();
    const current = location.search;
    if (
      viewStatesEqual(parseViewState(current), state) &&
      appStatesEqual(parseAppState(current), appState)
    ) {
      return; // the URL already says this — leave its bytes alone
    }
    const next = withAppState(withViewState(current, state), appState);
    if (next === current) {
      return;
    }
    // Artifacts push, the camera replaces (#392, amending ADR 0021 decision
    // 3 at its own recorded reopen condition). A layer, a frame, a compare
    // pairing or the x-ray is something a person navigated *to*, so `back`
    // should return to it; a pan is not, and forty of them must not bury the
    // view you were looking at before.
    // Compared against the last state WE wrote, not against the URL. On the
    // first interaction the URL gains fields that were implicit all along —
    // the layer the server picked, the frame it opened on — and a pan that
    // merely makes them explicit is not navigation. `lastArtifact` is seeded
    // the moment the map reports its initial layer, so every comparison
    // after that is resolved-against-resolved.
    const previous = lastArtifact ?? parseViewState(current);
    const artifactChanged =
      options.transient !== true &&
      (!viewArtifactsEqual(previous, state) || !appStatesEqual(parseAppState(current), appState));
    lastArtifact = state;
    const url = `${location.pathname}${next}${location.hash}`;
    if (artifactChanged) {
      history.pushState(null, "", url);
    } else {
      history.replaceState(null, "", url);
    }
    renderChips();
  };

  // `popstate` drives the shell from the URL, which is the same thing a cold
  // load does — a restored state is indistinguishable from a pasted one.
  // Guarded so the applied attributes do not immediately write history back.
  let restoring = false;
  const applyViewState = (state: ViewState): void => {
    const attr = (name: string, value: string | undefined): void => {
      if (value === undefined) {
        map.removeAttribute(name);
      } else {
        map.setAttribute(name, value);
      }
    };
    attr("layer", state.layer);
    attr("datetime", state.time);
    attr("compare-datetime", state.compareTime);
    attr("compare-layer", state.compareLayer);
    attr("swipe", state.swipe === undefined ? undefined : formatSwipe(state.swipe));
    attr("center", state.center === undefined ? undefined : formatCenter(state.center));
    attr("zoom", state.zoom === undefined ? undefined : formatZoom(state.zoom));
    attr("xray", state.xray ? "" : undefined);
  };
  window.addEventListener("popstate", () => {
    restoring = true;
    try {
      const search = location.search;
      const restored = parseViewState(search);
      applyViewState(restored);
      lastArtifact = restored;
      const next = parseAppState(search);
      appState = next;
      if (datasetElement instanceof SwathCatalog) {
        datasetElement.dates = { from: next.from, to: next.to };
      }
      if (importElement instanceof SwathImport) {
        importElement.step = next.step ?? "source";
      }
      applyMode(next.view);
      if (next.view === "xray") {
        map.setAttribute("xray", "");
      }
    } finally {
      restoring = false;
    }
    renderChips();
  });

  // Modes over the rail's content (#283/#284): `layers` is the full rail
  // as it always was; the others narrow it. Entering `xray` turns the
  // overlay on (a user act); leaving leaves it. The rail's collapse is a
  // device preference: storage only, never the URL (honoured from a
  // `rail=collapsed` link without rewriting it).
  const MODE_TITLES = {
    layers: "Layers",
    data: "Data",
    sources: "Sources",
    author: "Author",
    xray: "X-ray",
  } as const;
  const authorDock = document.querySelector("#swath-author-dock");
  const authorStrip = document.querySelector("#swath-author-strip");
  const authorInspector = document.querySelector("#swath-author-inspector");
  if (
    authoringElement instanceof SwathAuthoringPanel &&
    authorStrip instanceof HTMLElement &&
    authorInspector instanceof HTMLElement
  ) {
    authoringElement.regions = { strip: authorStrip, inspector: authorInspector };
    if (appState.sel !== undefined) {
      authoringElement.sel = appState.sel;
    }
    authoringElement.addEventListener("swath-author-select", (event) => {
      if (appState.view !== "author") {
        return;
      }
      appState = { ...appState, sel: event.detail.sel };
      if (shellElement instanceof SwathShell) {
        shellElement.inspector = inspectorOpen(appState);
      }
      interact();
      syncUrl();
    });
  }
  // The search scope's dates (#411): the timeline's drag and the panel's
  // date fields both announce here, and the URL's date chip is the one
  // visible form. A restore sets `dates` instead, which does not announce.
  if (datasetElement instanceof SwathCatalog) {
    const catalog = datasetElement;
    catalog.dates = { from: appState.from, to: appState.to };
    catalog.addEventListener("swath-dates", (event) => {
      const { from, to } = event.detail;
      appState = withDates(appState, from ?? undefined, to ?? undefined);
      interact();
      syncUrl();
      renderChips();
    });
  }

  // The guided import's step (#420): named, in the URL, so a
  // half-finished import is a link someone can come back to.
  if (importElement instanceof SwathImport) {
    const flow = importElement;
    if (appState.step !== undefined) {
      flow.step = appState.step;
    }
    flow.addEventListener("swath-import-step", (event) => {
      appState = withStep(appState, event.detail.step);
      interact();
      syncUrl();
      renderChips();
    });
  }

  const xrayRail = document.querySelector("#swath-xray-rail");
  // The x-ray's display modes + analytics summary live in the rail under
  // view=xray (issue #286); the map re-homes a live overlay on assignment.
  const xrayModes = document.querySelector("#swath-xray-modes");
  const xrayAnalytics = document.querySelector("#swath-xray-analytics");
  if (xrayModes instanceof HTMLElement && xrayAnalytics instanceof HTMLElement) {
    map.xrayChrome = { modes: xrayModes, analytics: xrayAnalytics };
  }
  const modeTitle = document.querySelector("#swath-mode-title");
  const applyMode = (mode: ViewMode): void => {
    document.body.dataset["view"] = mode;
    if (shellElement instanceof SwathShell) {
      shellElement.view = mode;
    }
    if (railElement instanceof SwathRail) {
      railElement.mode = mode;
    }
    if (modeTitle) {
      modeTitle.textContent = MODE_TITLES[mode];
    }
    const show = {
      layers: mode === "layers" || mode === "xray",
      sources: mode === "sources",
      data: mode === "layers" || mode === "data",
      author: mode === "layers" || mode === "author",
    };
    panel.hidden = !show.layers;
    if (datasetElement instanceof SwathCatalog) {
      datasetElement.hidden = !show.data;
      if (mode === "data") {
        datasetElement.active = true; // lazy by contract: the first entry fetches
      }
    }
    if (addDataElement instanceof HTMLElement) {
      addDataElement.hidden = !show.data;
    }
    if (sourcesElement instanceof SwathSources) {
      sourcesElement.hidden = !show.sources;
      if (mode === "sources") {
        sourcesElement.active = true; // lazy by contract, as the catalog is
      }
    }
    if (importElement instanceof SwathImport) {
      importElement.hidden = !show.sources;
      if (mode === "sources") {
        importElement.active = true;
      }
    }
    if (authoringElement instanceof HTMLElement) {
      authoringElement.hidden = !show.author;
    }
    if (xrayRail instanceof HTMLElement) {
      xrayRail.hidden = mode !== "xray";
    }
    // Author mode (issue #291): the strip drawer opens over the map and the
    // inspector column shows once a step is selected.
    if (authorDock instanceof SwathDrawer) {
      authorDock.open = mode === "author";
      // Composing inverts the slot relationship (#400): the dock stops being
      // a strip over the map and fills the region beside the preview column.
      authorDock.size = mode === "author" ? "100%" : "38%";
    }
    if (shellElement instanceof SwathShell) {
      shellElement.compose = mode === "author";
    }
    // Entering author mode with no `sel=`: the panel's own selection (its
    // first step) becomes the state, so the inspector opens on it.
    if (
      mode === "author" &&
      appState.sel === undefined &&
      authoringElement instanceof SwathAuthoringPanel
    ) {
      const sel = authoringElement.sel;
      if (sel !== undefined && sel !== "") {
        appState = { ...appState, sel };
      }
    }
    if (shellElement instanceof SwathShell) {
      shellElement.inspector = inspectorOpen(appState);
    }
  };
  const savePreference = (): void => {
    if (storage) {
      saveAppPreference(storage, appState);
    }
  };
  const setMode = (mode: ViewMode): void => {
    if (appState.view === mode) {
      return;
    }
    const next: AppState = { view: mode };
    if (appState.rail !== undefined) {
      next.rail = appState.rail;
    }
    appState = next;
    applyMode(mode);
    if (mode === "xray") {
      map.setAttribute("xray", "");
    }
    interact();
    savePreference();
    syncUrl();
  };
  // The standing "new layer" control (#398): available from every mode, so
  // authoring is never something you have to find your way back to.
  const newLayer = document.querySelector<SwathButton>("#swath-new-layer");
  newLayer?.addEventListener("click", () => {
    setMode("author");
  });

  applyMode(appState.view);
  if (railElement instanceof SwathRail) {
    railElement.items = [
      { id: "layers", label: "Layers", icon: "layers" },
      { id: "data", label: "Data", icon: "data" },
      { id: "sources", label: "Sources", icon: "sources" },
      { id: "author", label: "Author", icon: "author" },
      { id: "xray", label: "X-ray", icon: "xray" },
    ];
    railElement.collapsed = appState.rail === "collapsed";
    railElement.addEventListener("swath-mode-change", (event) => {
      const mode = event.detail.mode;
      if (isViewMode(mode)) {
        setMode(mode);
      }
    });
    railElement.addEventListener("swath-toggle", (event) => {
      const next: AppState = { view: appState.view };
      if (event.detail.pressed) {
        next.rail = "collapsed";
      }
      appState = next;
      savePreference();
      syncUrl(); // a no-op by construction: `rail` is never written
    });
  }

  // The status bar (#284, #287): lat/lon of the cursor (copy on click),
  // zoom, the tiling CRS, and ingest→pixel — the glass-box number, fed by
  // `swath-trace` with or without the x-ray on (`traces` attribute).
  const lonlat = document.querySelector("#swath-status-lonlat");
  const zoomCell = document.querySelector("#swath-status-zoom");
  const crsCell = document.querySelector("#swath-status-crs");
  const ingestCell = document.querySelector("#swath-status-ingest");
  if (crsCell instanceof SwathStatusCell) {
    crsCell.value = formatCrs();
  }
  let ingestMs: number | undefined;
  if (ingestCell instanceof SwathStatusCell) {
    ingestCell.value = formatIngest(ingestMs);
  }
  map.addEventListener("swath-cursor", (event) => {
    if (lonlat instanceof SwathStatusCell) {
      lonlat.value = formatLonLat(event.detail.lng, event.detail.lat);
      lonlat.dataset["source"] = event.detail.source;
    }
    if (zoomCell instanceof SwathStatusCell) {
      zoomCell.value = formatZoomCell(event.detail.zoom);
    }
  });
  map.addEventListener("swath-trace", (event) => {
    const ms = event.detail.envelope.trace.ingest_to_pixel_ms;
    if (ms === null) {
      return;
    }
    ingestMs = Math.min(ingestMs ?? Number.POSITIVE_INFINITY, ms);
    if (ingestCell instanceof SwathStatusCell) {
      ingestCell.value = formatIngest(ingestMs);
    }
  });
  if (lonlat instanceof SwathStatusCell) {
    lonlat.addEventListener("click", () => {
      const text = lonlat.value ?? "";
      if (text === "") {
        return;
      }
      navigator.clipboard
        .writeText(text)
        .then(() => {
          lonlat.dataset["copied"] = text;
          if (shellElement instanceof SwathShell) {
            shellElement.announce(`Copied ${text}`);
          }
        })
        .catch(() => undefined);
    });
  }

  const interact = (): void => {
    interacted = true;
  };

  map.addEventListener("swath-layer-change", (event) => {
    const detail = event.detail;
    panel.update(detail.layers, detail.layer, { visible: detail.visible, opacity: detail.opacity });
    // Authored services get a delete action (#282); the list is best-effort.
    map.api
      .json<{ services?: { id?: unknown }[] }>("/services")
      .then((body) => {
        panel.services = (body.services ?? []).flatMap((s) =>
          typeof s.id === "string" ? [s.id] : [],
        );
      })
      .catch(() => undefined);
    // A layer change after the initial apply is always user-driven on
    // this page (the rail, an authored or added layer, a deletion).
    const changed = appliedLayer !== undefined && appliedLayer !== detail.layer;
    appliedLayer = detail.layer;
    if (!changed) {
      // The INITIAL apply is where the implicit becomes known — the layer
      // the server picked, the frame it opened on. Seed the artifact
      // baseline from it (overwriting any earlier one: a mode switch can
      // land before the map has reported a layer), so the first pan is a
      // pan and not "the URL learned the layer's name" (#392).
      lastArtifact = snapshot();
    }
    renderChips();
    if (changed) {
      interact();
    }
    persist();
    if (changed) {
      syncUrl();
    }
    if (shareElement) {
      shareElement.disabled = false;
    }
  });

  panel.addEventListener("swath-layer-select", (event) => {
    const layer = event.detail.layer;
    // Failures surface via the map's own `swath-error` + retry loop.
    map.setLayer(layer).catch(() => undefined);
  });
  // Eye and opacity act on the viewed layer only (the #282 scope fence).
  panel.addEventListener("swath-layer-visibility", (event) => {
    if (event.detail.layer === appliedLayer) {
      map.setLayerVisibility(event.detail.visible);
    }
  });
  panel.addEventListener("swath-layer-opacity", (event) => {
    if (event.detail.layer === appliedLayer) {
      map.setLayerOpacity(event.detail.opacity);
    }
  });
  panel.addEventListener("swath-layer-action", (event) => {
    const { layer, action } = event.detail;
    switch (action) {
      case "zoom":
        if (layer === appliedLayer) {
          map.zoomToData();
        } else {
          map
            .setLayer(layer)
            .then(() => map.zoomToData())
            .catch(() => undefined);
        }
        break;
      case "compare":
        if (layer === appliedLayer) {
          map.toggleCompare();
        } else {
          map.setAttribute("compare-layer", layer);
        }
        break;
      case "delete":
        map.api
          .fetch(`/services/${encodeURIComponent(layer)}`, { method: "DELETE" })
          .then((response) => {
            if (response.ok) {
              onServiceDeleted(map, layer);
            }
          })
          .catch(() => undefined);
        break;
      default:
        break; // "info" expands inside the row
    }
  });

  // User-driven movement (drag, wheel, keyboard — MapLibre stamps those
  // with `originalEvent`) updates the share link; programmatic moves
  // (bounds fits, attribute jumps) only update the remembered session.
  map.map?.on("moveend", (event) => {
    if ((event as { originalEvent?: Event }).originalEvent) {
      interact();
    }
    persist();
    if ((event as { originalEvent?: Event }).originalEvent) {
      syncUrl();
    }
  });

  // The x-ray toggle flips the host attribute; mirror it into URL and
  // storage. The observer attaches after the initial attributes landed,
  // so a deep-linked `?xray` does not count as an interaction — the
  // control, and the landing card's invitation, do.
  new MutationObserver(() => {
    interact();
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
  // URL already encodes the same state. Frames the cinematic landing
  // advanced on its own (issue #211) are not the user's: no write.
  map.addEventListener("swath-timechange", (event) => {
    if (!event.detail.cinematic) {
      interact();
    }
    persist();
    // A cinematic frame advance is the app playing, not the person
    // navigating: it updates the URL in place (#392).
    syncUrl({ transient: event.detail.cinematic });
  });

  // Compare swipe (issue #210): the map announces every compare-state
  // change (toggle, handle drag, or a programmatic attribute set) — the
  // same mirror-into-URL-and-storage seam as time. Deep-linked compare
  // attributes are applied BEFORE define(), so no event fires on load
  // and pasted links stay byte-stable. Every announced change is the
  // user's (the toggle, the handle) — the share link carries compare too.
  map.addEventListener("swath-comparechange", () => {
    interact();
    persist();
    syncUrl();
  });

  // Data framing (issue #182 follow-up): both the auto-frame after a
  // user-initiated layer switch and the `zoom to data` control announce
  // their move — user-driven view changes, so the share link follows
  // (programmatic moves otherwise deliberately never rewrite the URL).
  map.addEventListener("swath-framedata", () => {
    interact();
    persist();
    syncUrl();
  });

  if (shareElement) {
    wireShare(shareElement, snapshot);
  }

  wireReflow({
    map,
    rail: railElement,
    shell: shellElement,
    content: () =>
      [panel, datasetElement, addDataElement, authoringElement, xrayRail].filter(
        (node): node is HTMLElement => node instanceof HTMLElement,
      ),
    currentView: () => appState.view,
  });

  wirePalette({
    map,
    panel,
    catalog: datasetElement,
    share: shareElement,
    currentLayer: () => appliedLayer,
    currentView: () => appState.view,
    setMode,
  });
}
