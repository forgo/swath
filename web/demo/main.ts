// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

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
import { buildCommands } from "../src/commands.js";
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
import { GranuleFootprints } from "../src/granule-footprints.js";
import { formatCrs, formatIngest, formatLonLat, formatZoomCell } from "../src/status-model.js";
import { safeLocalStorage } from "../src/storage.js";
import { defineSwathAddDataPanel, SwathAddDataPanel } from "../src/swath-add-data-panel.js";
import { defineSwathAuthoringPanel, SwathAuthoringPanel } from "../src/swath-authoring-panel.js";
import { defineSwathCatalog, SwathCatalog } from "../src/swath-catalog.js";
import { defineSwathLayerList, SwathLayerList } from "../src/swath-layer-list.js";
import { defineSwathMap, SwathMap } from "../src/swath-map.js";
import { defineSwathShell, SwathShell } from "../src/swath-shell.js";
import { SwathButton } from "../src/ui/button.js";
import { SwathCommandPalette } from "../src/ui/command-palette.js";
import { SwathDrawer } from "../src/ui/drawer.js";
import { createSwathEvent } from "../src/ui/events.js";
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
  shareUrl,
  type ViewState,
  viewStatesEqual,
  withViewState,
} from "../src/view-state.js";

const mapElement = document.querySelector("swath-map");
const panelElement = document.querySelector("swath-layer-list");
const datasetElement = document.querySelector("swath-catalog");
const addDataElement = document.querySelector("swath-add-data-panel");
const authoringElement = document.querySelector("swath-authoring-panel");
SwathButton.define();
SwathRail.define();
SwathHudDock.define();
SwathDrawer.define();
SwathCommandPalette.define();
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
defineSwathAddDataPanel();
defineSwathAuthoringPanel();

if (mapElement instanceof SwathMap && panelElement instanceof SwathLayerList) {
  wire(mapElement, panelElement);
}
if (mapElement instanceof SwathMap && authoringElement instanceof SwathAuthoringPanel) {
  wireAuthoring(mapElement, authoringElement);
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
  const syncUrl = (): void => {
    if (!interacted) {
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
    history.replaceState(null, "", `${location.pathname}${next}${location.hash}`);
  };

  // Modes over the rail's content (#283/#284): `layers` is the full rail
  // as it always was; the others narrow it. Entering `xray` turns the
  // overlay on (a user act); leaving leaves it. The rail's collapse is a
  // device preference: storage only, never the URL (honoured from a
  // `rail=collapsed` link without rewriting it).
  const MODE_TITLES = { layers: "Layers", data: "Data", author: "Author", xray: "X-ray" } as const;
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
  applyMode(appState.view);
  if (railElement instanceof SwathRail) {
    railElement.items = [
      { id: "layers", label: "Layers", icon: "layers" },
      { id: "data", label: "Data", icon: "data" },
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
    syncUrl();
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

  // Responsive reflow (issue #293, ui-system.md §6): the shell reports its
  // tier; the host moves the pieces. Wide: as drawn. Medium/narrow: an
  // icon rail, the mode content in a right drawer (modal with a scrim on
  // narrow). Phone: a bottom tab bar, the content as a sheet (40/90 snaps),
  // the status bar folded into the dock as one chip, the map inert under
  // a 90% sheet. Tapping the active tab toggles the sheet.
  const railDrawer = document.querySelector("#swath-rail-drawer");
  const railDrawerBody = document.querySelector("#swath-rail-drawer-body");
  const statusBar = document.querySelector("swath-status-bar");
  const hudDock = document.querySelector("swath-hud-dock");
  const railContent = (): HTMLElement[] =>
    [panel, datasetElement, addDataElement, authoringElement, xrayRail].filter(
      (node): node is HTMLElement => node instanceof HTMLElement,
    );
  let tier = "wide";
  const modeHasContent = (): boolean => appState.view !== "xray" || map.hasAttribute("xray");
  const applyTier = (next: string): void => {
    tier = next;
    document.body.dataset["tier"] = next;
    const compact = next !== "wide";
    if (
      railDrawer instanceof SwathDrawer &&
      railDrawerBody instanceof HTMLElement &&
      railElement instanceof SwathRail
    ) {
      if (compact) {
        railDrawerBody.append(...railContent());
        railDrawer.modal = next === "narrow";
        railDrawer.open = modeHasContent();
      } else {
        railElement.append(...railContent());
        railDrawer.open = false;
      }
    }
    if (
      statusBar instanceof SwathStatusBar &&
      hudDock instanceof HTMLElement &&
      shellElement instanceof SwathShell
    ) {
      if (next === "phone") {
        statusBar.chip = true;
        statusBar.slot = "bottom-left";
        hudDock.append(statusBar);
      } else {
        statusBar.chip = false;
        statusBar.slot = "statusbar";
        shellElement.append(statusBar);
      }
    }
    syncInert();
  };
  const syncInert = (): void => {
    const sheetFull =
      railDrawer instanceof SwathDrawer &&
      railDrawer.open &&
      railDrawer.getAttribute("presentation") === "bottom" &&
      railDrawer.snapIndex === 1;
    const modalOpen = railDrawer instanceof SwathDrawer && railDrawer.open && railDrawer.modal;
    map.inert = sheetFull || modalOpen;
  };
  if (shellElement instanceof SwathShell) {
    shellElement.addEventListener("swath-change", (event) => {
      if (event.detail.name === "tier") {
        applyTier(String(event.detail.value));
      }
    });
    if (shellElement.tier !== undefined) {
      applyTier(shellElement.tier);
    }
  }
  if (railDrawer instanceof SwathDrawer) {
    railDrawer.addEventListener("swath-drawer-close", () => {
      railDrawer.open = false;
      syncInert();
    });
    railDrawer.addEventListener("swath-change", (event) => {
      if (event.detail.name === "snap") {
        syncInert();
      }
    });
  }
  if (railElement instanceof SwathRail && railDrawer instanceof SwathDrawer) {
    // Capture phase: this must see the view *before* the rail's own click
    // handler switches modes — otherwise tapping a new tab opens the sheet
    // (mode change) and immediately toggles it closed (same-mode tap).
    railElement.addEventListener(
      "click",
      (event) => {
        const item = event
          .composedPath()
          .find(
            (n): n is HTMLElement => n instanceof HTMLElement && n.dataset["mode"] !== undefined,
          );
        if (item && item.dataset["mode"] === appState.view && tier !== "wide") {
          railDrawer.open = !railDrawer.open;
          syncInert();
        }
      },
      { capture: true },
    );
    railElement.addEventListener("swath-mode-change", () => {
      if (tier !== "wide" && railDrawer instanceof SwathDrawer) {
        railDrawer.open = modeHasContent();
        syncInert();
      }
    });
  }

  // The command palette (issue #292): built from live state each time it
  // opens — layers, the other modes, the map toggles, share, and in Data
  // mode a jump to any listed granule. ⌘K / Ctrl-K anywhere, or the top
  // bar's button; Esc restores focus to where it was.
  const palette = document.querySelector("swath-command-palette");
  if (palette instanceof SwathCommandPalette) {
    const openPalette = (): void => {
      palette.commands = buildCommands({
        layers: panel.layers,
        activeLayer: appliedLayer,
        mode: appState.view,
        xray: map.hasAttribute("xray"),
        compareAvailable: map.querySelector(".swath-map-compare-toggle:not([hidden])") !== null,
        granules:
          datasetElement instanceof SwathCatalog && datasetElement.selected !== ""
            ? datasetElement.granules.map((granule) => ({
                dataset: datasetElement.selected,
                granule,
              }))
            : undefined,
        setLayer: (id) => {
          map.setLayer(id).catch(() => undefined);
        },
        setMode,
        toggleXray: () => {
          if (map.hasAttribute("xray")) {
            map.removeAttribute("xray");
          } else {
            map.setAttribute("xray", "");
          }
        },
        toggleCompare: () => map.toggleCompare(),
        zoomToData: () => map.zoomToData(),
        share: () => shareElement?.click(),
        zoomToGranule: (_dataset, granule) => {
          if (datasetElement instanceof SwathCatalog) {
            datasetElement.dispatchEvent(
              createSwathEvent("swath-granule-zoom", {
                dataset: _dataset,
                id: granule.id,
                bbox: granule.bbox,
              }),
            );
          }
        },
      });
      palette.show();
    };
    document.querySelector("#swath-search")?.addEventListener("click", openPalette);
    window.addEventListener("keydown", (event) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        if (palette.open) {
          palette.close();
        } else {
          openPalette();
        }
      }
    });
  }
}

/** How long the Share button reads "copied" before reverting. */
const SHARE_FEEDBACK_MS = 1600;

/** The Share button (issue #211): copies the canonical deep link of the
 * current view — the same URL the address bar shows after an
 * interaction, written in full even on a bare landing. Clipboard
 * failure (no secure context, permission denied) falls back to a
 * prompt holding the link, so the URL is never unreachable. */
function wireShare(button: SwathButton, snapshot: () => ViewState): void {
  const idle = (button.textContent ?? "").trim();
  let revert: number | undefined;
  const feedback = (state: "copied" | "failed"): void => {
    button.dataset["state"] = state;
    button.textContent = state === "copied" ? "copied" : "copy failed";
    window.clearTimeout(revert);
    revert = window.setTimeout(() => {
      delete button.dataset["state"];
      button.textContent = idle;
    }, SHARE_FEEDBACK_MS);
  };
  button.addEventListener("click", () => {
    const url = shareUrl(location.href, snapshot());
    button.dataset["url"] = url; // what was copied, inspectable (tests, tooling)
    navigator.clipboard
      .writeText(url)
      .then(() => feedback("copied"))
      .catch(() => {
        feedback("failed");
        window.prompt("Copy this link", url);
      });
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
    const id = event.detail.id;
    map.setLayer(id).catch(() => undefined);
  });
  authoring.addEventListener("swath-service-deleted", (event) => {
    onServiceDeleted(map, event.detail.id);
  });
}

/** A service is gone (deleted from the authoring panel or a layer row's
 * kebab): the viewed layer falls back to the server default, any other
 * just refreshes the list. */
function onServiceDeleted(map: SwathMap, id: string): void {
  if (map.getAttribute("layer") === id) {
    map.removeAttribute("layer"); // re-applies with the server default
  } else {
    map.refresh();
  }
}
