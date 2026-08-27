// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The typed event seam (docs/design/ui-system.md §4.3) — the one home of
 * `new CustomEvent(` under web/ (scripts/check-ui-dry.mjs).
 *
 * `SwathEventMap` is the catalog: event name → detail type. Issue #279 fills
 * it; until then it is empty and each element (or test) augments it with
 * `declare module "./events.js" { interface SwathEventMap { … } }`. The
 * global augmentation below makes `addEventListener("swath-…", e => …)`
 * typed on every element without casts.
 *
 * Every Swath event is `bubbles: true, composed: true` — one that is not
 * composed dies at the first shadow boundary — so it is hardcoded here and
 * pinned by element.test.ts, not left to each caller.
 */
// biome-ignore lint/suspicious/noEmptyInterface: the catalog is filled by augmentation (#279)
export interface SwathEventMap {}

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
