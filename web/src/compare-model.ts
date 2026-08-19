// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The compare swipe's semantics (issue #210), DOM-free in the
 * add-data-model/authoring-model tradition: which of the two modes a
 * pair of raw attribute values resolves to, what each side of the
 * handle shows, and which side a received trace belongs to. The DOM
 * machinery (the second map, the handle, per-side badges) lives in
 * swath-compare.ts / swath-xray.ts; everything here is unit-testable
 * with no browser at all.
 *
 * Two modes, deliberately exclusive (the issue's scope — no lens/spy):
 *
 * - **date-vs-date**: one layer, two `datetime=` frames (ADR 0015). The
 *   left side is the map's own `datetime` attribute (the time slider
 *   keeps scrubbing it); the right side is the compare instant.
 * - **layer-vs-layer**: two layers at the same instant (the map's
 *   `datetime` attribute rides both sides' tile requests).
 *
 * Side identity for the x-ray overlay comes from the trace stream
 * itself: every temporal trace carries `requested` — the raw
 * `datetime=` of the request (#223) — so date-mode sides are told apart
 * by the exact instant each side asked for, and layer-mode sides by the
 * envelope's layer id. No heuristics, the same one-source-of-truth rule
 * the overlay is built on.
 */

/** Which flavor of comparison is active. */
export type CompareMode = "date" | "layer";

/** A resolved comparison: the mode plus the right side's identity (the
 * left side is always the map's own layer/datetime). */
export interface CompareSpec {
  mode: CompareMode;
  /** `date`: the right frame's RFC 3339 instant; `layer`: the right
   * layer id. */
  value: string;
}

/**
 * Resolves the raw compare inputs to a spec, or undefined when nothing
 * coherent is asked for. Malformed states degrade to "no compare",
 * never break the page (the view-state module's own rule):
 * both-at-once is ambiguous and dropped; comparing a layer with itself
 * is meaningless and dropped.
 */
export function resolveCompare(
  activeLayer: string,
  compareTime: string | undefined,
  compareLayer: string | undefined,
): CompareSpec | undefined {
  const layer = compareLayer === "" ? undefined : compareLayer;
  if (compareTime !== undefined && layer !== undefined) {
    return undefined; // ambiguous — one mode at a time
  }
  if (compareTime !== undefined) {
    return { mode: "date", value: compareTime };
  }
  if (layer !== undefined && layer !== activeLayer) {
    return { mode: "layer", value: layer };
  }
  return undefined;
}

/** One side of the handle: which layer its tiles come from and the raw
 * `datetime=` its requests carry (null = latest, no param). */
export interface CompareSide {
  layer: string;
  requested: string | null;
}

/** Both sides, resolved against the map's current layer and frame. */
export interface CompareSides {
  mode: CompareMode;
  left: CompareSide;
  right: CompareSide;
}

/** What each side shows given the spec and the map's own state. */
export function compareSides(
  spec: CompareSpec,
  activeLayer: string,
  datetime: string | null,
): CompareSides {
  const left: CompareSide = { layer: activeLayer, requested: datetime };
  if (spec.mode === "date") {
    return { mode: "date", left, right: { layer: activeLayer, requested: spec.value } };
  }
  return { mode: "layer", left, right: { layer: spec.value, requested: datetime } };
}

/**
 * Which side a received trace belongs to; undefined when it belongs to
 * neither (another layer's background render, or a frame no side is
 * showing — dropped from the per-side badges, never guessed). The right
 * side is checked first so the degenerate "both sides ask for the same
 * frame" state stays deterministic.
 */
export function traceSide(
  sides: CompareSides,
  layer: string,
  requested: string | null | undefined,
): "left" | "right" | undefined {
  if (sides.mode === "layer") {
    if (layer === sides.right.layer) {
      return "right";
    }
    return layer === sides.left.layer ? "left" : undefined;
  }
  if (layer !== sides.left.layer) {
    return undefined;
  }
  const raw = requested ?? null;
  if (raw === sides.right.requested) {
    return "right";
  }
  return raw === sides.left.requested ? "left" : undefined;
}

/** What the per-side chips read: the frames in date mode, the layer ids
 * in layer mode. An absent left frame is honestly "latest". */
export function sideLabels(sides: CompareSides): { left: string; right: string } {
  if (sides.mode === "date") {
    return { left: sides.left.requested ?? "latest", right: sides.right.requested ?? "latest" };
  }
  return { left: sides.left.layer, right: sides.right.layer };
}

/** Where the handle rests until a deep link or a drag says otherwise. */
export const DEFAULT_SWIPE = 0.5;

/** A usable handle position: finite, clamped to [0, 1]; anything else
 * (absent, malformed) is the centered default. */
export function clampSwipe(value: number | undefined): number {
  if (value === undefined || !Number.isFinite(value)) {
    return DEFAULT_SWIPE;
  }
  return Math.min(1, Math.max(0, value));
}
