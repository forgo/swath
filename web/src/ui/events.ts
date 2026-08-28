// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import type { CatalogGranule } from "../catalog-model.js";
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
import type { LonLatBounds, SwathLayer } from "../swath-map.js";
import type { TraceEnvelope } from "../swath-xray.js";

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
  /** Where the map's cursor is (rAF-throttled): the pointer while a mouse
   * is over the map, the map centre otherwise (touch, or the mouse left). */
  "swath-cursor": { lng: number; lat: number; zoom: number; source: "pointer" | "center" };
  /** One trace envelope off the SSE stream — with or without badges. */
  "swath-trace": { envelope: TraceEnvelope };
  "swath-error": { error: unknown };
  "swath-dataset-granules": { dataset: string; granules: CatalogGranule[] };
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
  // --- The DAG canvas (issue #290): interaction only, no graph semantics ---
  /** The viewport changed (pan, zoom, fit). */
  "swath-canvas-change": { x: number; y: number; k: number };
  /** The selection changed (click, marquee, keyboard). */
  "swath-canvas-select": { nodes: string[]; edges: string[] };
  /** A node was dragged to a new canvas position (on release). */
  "swath-node-move": { id: string; x: number; y: number };
  /** A node was activated (Enter / double-click / tap). */
  "swath-node-activate": { id: string };
  /** The user asked to delete the selection (Delete / context). */
  "swath-delete-request": { nodes: string[]; edges: string[] };
  /** A connection gesture began at a port. */
  "swath-port-connect-start": { node: string; port: string; side: "input" | "output" };
  /** A connection gesture ended: the consumer decides whether it is allowed. */
  "swath-port-connect-end": {
    from: { node: string; port: string; side: "input" | "output" };
    to: { node: string; port: string; side: "input" | "output" } | null;
  };
  /** A port was tapped / Enter-ed: the canvas arms or completes a connection. */
  "swath-port-tap": { node: string; port: string; side: "input" | "output" };
  /** Long-press / right-click on the canvas or a node: a context request. */
  "swath-canvas-context": { node: string | null; x: number; y: number };
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
