// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The typed event catalog (docs/design/ui-system.md §4.3) — the one home
 * of `new CustomEvent(` under web/ (scripts/check-ui-dry.mjs).
 *
 * `SwathEventMap` maps event name → detail. The global augmentation makes
 * `el.addEventListener("swath-…", (e) => e.detail)` typed on every element
 * with no cast. Every Swath event is `bubbles: true, composed: true` — one
 * that is not composed dies at the first shadow boundary — hardcoded here
 * and pinned by element.test.ts.
 *
 * Naming going forward is `swath-<subject>-<verb>`; the M5–M9 names below
 * are kept verbatim until each organism migrates (#282–#291), so hosts see
 * no behaviour change. `layerchange` → `swath-layer-change`: the map
 * dispatches both for one milestone.
 */
import type { GranuleBbox } from "../granule-footprints.js";
import type { GranuleListItem } from "../swath-dataset-panel.js";
import type { LonLatBounds, SwathLayer } from "../swath-map.js";

export interface SwathEventMap {
  /** The viewed layer changed (`<swath-map>`). */
  "swath-layer-change": { layer: string; layers: SwathLayer[]; visible: boolean; opacity: number };
  /** Alias of `swath-layer-change` for one milestone. */
  layerchange: { layer: string; layers: SwathLayer[]; visible: boolean; opacity: number };
  "swath-layer-select": { layer: string };
  /** The eye on a layer row (`<swath-layer-item>`). */
  "swath-layer-visibility": { layer: string; visible: boolean };
  /** The opacity slider on a layer row, live. */
  "swath-layer-opacity": { layer: string; opacity: number };
  /** A kebab action on a layer row; the host decides what it means. */
  "swath-layer-action": { layer: string; action: "zoom" | "compare" | "info" | "delete" };
  "swath-timechange": { datetime: string | null; cinematic: boolean };
  "swath-comparechange": {
    compareTime: string | null;
    compareLayer: string | null;
    swipe: string | null;
  };
  "swath-framedata": { bounds: LonLatBounds };
  "swath-error": { error: unknown };
  "swath-dataset-granules": { dataset: string; granules: GranuleListItem[] };
  "swath-granule-zoom": { dataset: string; id: string; bbox: GranuleBbox };
  "swath-data-added": { dataset: string; layer: string };
  "swath-service-created": { id: string };
  "swath-service-deleted": { id: string };
  /** A primitive's committed value changed by the user (`<swath-toggle>`). */
  "swath-change": { name: string; value: string | number | boolean };
  /** A pressed-state button was activated (`<swath-button pressed>`). */
  "swath-toggle": { pressed: boolean };
  /** A live (uncommitted) value while the user drags or types. */
  "swath-input": { name: string; value: string | number | boolean };
  /** A menu item was chosen (`<swath-menu>`). */
  "swath-menu-select": { id: string };
  /** A drawer, sheet or menu asks to close; the host decides. */
  "swath-drawer-close": { reason: "esc" | "scrim" | "swipe" | "select" | "outside" };
  /** The rail's mode switcher picked a mode (`<swath-rail>`). */
  "swath-mode-change": { mode: string };
  /** An interactive card was activated; `long` for a long-press / context. */
  "swath-activate": { id: string; long: boolean };
}

type SwathCustomEvents = {
  [K in keyof SwathEventMap]: CustomEvent<SwathEventMap[K]>;
};

declare global {
  interface HTMLElementEventMap extends SwathCustomEvents {}
}

export function createSwathEvent<K extends keyof SwathEventMap>(
  type: K,
  detail: SwathEventMap[K],
): CustomEvent<SwathEventMap[K]> {
  return new CustomEvent(type, { detail, bubbles: true, composed: true });
}
