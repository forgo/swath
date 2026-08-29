// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/** Responsive reflow (issue #293, ui-system.md §6): the shell reports its
 * tier; this moves the pieces. Wide: as drawn. Medium/narrow: an icon
 * rail, the mode content in a right drawer (modal with a scrim on narrow).
 * Phone: a bottom tab bar, the content as a sheet (40/90 snaps), the
 * status bar folded into the dock as one chip, the map inert under a 90%
 * sheet. Tapping the active tab toggles the sheet. Lifted out of the
 * entry script (#355). */
import type { ViewMode } from "../app-state";
import type { SwathMap } from "../swath-map";
import { SwathShell } from "../swath-shell";
import { SwathDrawer } from "../ui/drawer";
import { SwathRail } from "../ui/rail";
import { SwathStatusBar } from "../ui/status-bar";

export interface ReflowDeps {
  map: SwathMap;
  rail: Element | null;
  shell: Element | null;
  /** The rail's mode content, in rail order. */
  content: () => HTMLElement[];
  currentView: () => ViewMode;
}

export function wireReflow(deps: ReflowDeps): void {
  const { map, rail, shell, content, currentView } = deps;
  const railDrawer = document.querySelector("#swath-rail-drawer");
  const railDrawerBody = document.querySelector("#swath-rail-drawer-body");
  const statusBar = document.querySelector("swath-status-bar");
  const hudDock = document.querySelector("swath-hud-dock");
  const railContent = content;
  let tier = "wide";
  const modeHasContent = (): boolean => currentView() !== "xray" || map.hasAttribute("xray");
  const applyTier = (next: string): void => {
    tier = next;
    document.body.dataset["tier"] = next;
    const compact = next !== "wide";
    if (
      railDrawer instanceof SwathDrawer &&
      railDrawerBody instanceof HTMLElement &&
      rail instanceof SwathRail
    ) {
      if (compact) {
        railDrawerBody.append(...railContent());
        railDrawer.modal = next === "narrow";
        railDrawer.open = modeHasContent();
      } else {
        rail.append(...railContent());
        railDrawer.open = false;
      }
    }
    if (
      statusBar instanceof SwathStatusBar &&
      hudDock instanceof HTMLElement &&
      shell instanceof SwathShell
    ) {
      if (next === "phone") {
        statusBar.chip = true;
        statusBar.slot = "bottom-left";
        hudDock.append(statusBar);
      } else {
        statusBar.chip = false;
        statusBar.slot = "statusbar";
        shell.append(statusBar);
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
  if (shell instanceof SwathShell) {
    shell.addEventListener("swath-change", (event) => {
      if (event.detail.name === "tier") {
        applyTier(String(event.detail.value));
      }
    });
    if (shell.tier !== undefined) {
      applyTier(shell.tier);
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
  if (rail instanceof SwathRail && railDrawer instanceof SwathDrawer) {
    // Capture phase: this must see the view *before* the rail's own click
    // handler switches modes — otherwise tapping a new tab opens the sheet
    // (mode change) and immediately toggles it closed (same-mode tap).
    rail.addEventListener(
      "click",
      (event) => {
        const item = event
          .composedPath()
          .find(
            (n): n is HTMLElement => n instanceof HTMLElement && n.dataset["mode"] !== undefined,
          );
        if (item && item.dataset["mode"] === currentView() && tier !== "wide") {
          railDrawer.open = !railDrawer.open;
          syncInert();
        }
      },
      { capture: true },
    );
    rail.addEventListener("swath-mode-change", () => {
      if (tier !== "wide" && railDrawer instanceof SwathDrawer) {
        railDrawer.open = modeHasContent();
        syncInert();
      }
    });
  }
}
