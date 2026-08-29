// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** The command palette (issue #292): built from live state each time it
 * opens — layers, the other modes, the map toggles, share, and in Data
 * mode a jump to any listed granule. ⌘K / Ctrl-K anywhere, or the top
 * bar's button; Esc restores focus to where it was. Lifted out of the
 * entry script (#355). */
import type { ViewMode } from "../app-state";
import { buildCommands } from "../commands";
import { SwathCatalog } from "../swath-catalog";
import type { SwathLayerList } from "../swath-layer-list";
import type { SwathMap } from "../swath-map";
import { SwathCommandPalette } from "../ui/command-palette";
import { createSwathEvent } from "../ui/events";

export interface PaletteDeps {
  map: SwathMap;
  panel: SwathLayerList;
  catalog: Element | null;
  share: HTMLElement | null;
  currentLayer: () => string | undefined;
  currentView: () => ViewMode;
  setMode: (mode: ViewMode) => void;
}

export function wirePalette(deps: PaletteDeps): void {
  const { map, panel, catalog, share, currentLayer, currentView, setMode } = deps;
  const palette = document.querySelector("swath-command-palette");
  if (palette instanceof SwathCommandPalette) {
    const openPalette = (): void => {
      palette.commands = buildCommands({
        layers: panel.layers,
        activeLayer: currentLayer(),
        mode: currentView(),
        xray: map.hasAttribute("xray"),
        compareAvailable: map.querySelector(".swath-map-compare-toggle:not([hidden])") !== null,
        granules:
          catalog instanceof SwathCatalog && catalog.selected !== ""
            ? catalog.granules.map((granule) => ({
                dataset: catalog.selected,
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
        share: () => share?.click(),
        zoomToGranule: (_dataset, granule) => {
          if (catalog instanceof SwathCatalog) {
            catalog.dispatchEvent(
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
