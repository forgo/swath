// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import type { ViewMode } from "../../src/app-state.js";
/**
 * Deep links for the e2e suite (issue #399), built by the app's own writers.
 *
 * Roughly thirty-five literal query strings were spelled out across the
 * specs and the screenshot capture. That put the URL grammar in two places —
 * `view-state.ts`/`app-state.ts` for the app, and thirty-five hand-typed
 * copies for the tests — so a change to the grammar was a thirty-five-line
 * diff, and the tests could keep passing against a spelling the app no
 * longer wrote.
 *
 * This builds them through `withViewState`/`withAppState`, the same
 * functions the app writes URLs with. Tests now assert against the grammar
 * the product actually has, and a change to it is one line here.
 */
import { withAppState } from "../../src/app-state.js";
import { formatCenter, formatZoom, withViewState } from "../../src/view-state.js";

/** What a deep link can name. Everything is optional: a bare `demoUrl()` is
 * the zero-config landing, which is a state worth naming too. */
export interface DemoLink {
  layer?: string;
  /** `[lon, lat]`, formatted by the app's own writer. */
  center?: readonly [number, number];
  zoom?: number;
  /** The viewed frame's instant (`t=`). */
  time?: string;
  /** The x-ray OVERLAY flag, distinct from `view: "xray"` (the mode). */
  xray?: boolean;
  compareTime?: string;
  compareLayer?: string;
  swipe?: number;
  /** The mode (`view=`); `layers` is the default and is never written. */
  view?: ViewMode;
  /** The selected authoring node (`sel=`), meaningful only under `author`. */
  sel?: string;
  /** Params the app passes through but does not own (`stac=`, `basemap=`). */
  passthrough?: Readonly<Record<string, string>>;
}

/** The search string naming `link`'s state, in the app's own spelling.
 *
 * Separate from `demoUrl` so it can be unit-tested: this module must not
 * import the Playwright-bearing support barrel, which vitest cannot load. */
export function demoSearch(link: DemoLink = {}): string {
  const foreign = new URLSearchParams(link.passthrough ?? {}).toString();
  let search = withViewState(foreign === "" ? "" : `?${foreign}`, {
    xray: link.xray === true,
    ...(link.layer === undefined ? {} : { layer: link.layer }),
    ...(link.center === undefined ? {} : { center: [link.center[0], link.center[1]] }),
    ...(link.zoom === undefined ? {} : { zoom: link.zoom }),
    ...(link.time === undefined ? {} : { time: link.time }),
    ...(link.compareTime === undefined ? {} : { compareTime: link.compareTime }),
    ...(link.compareLayer === undefined ? {} : { compareLayer: link.compareLayer }),
    ...(link.swipe === undefined ? {} : { swipe: link.swipe }),
  });
  search = withAppState(search, {
    view: link.view ?? "layers",
    ...(link.sel === undefined ? {} : { sel: link.sel }),
  });
  return search;
}

/** `formatCenter`/`formatZoom` re-exported so a spec that needs the app's
 * number formatting for an assertion does not reimplement it. */
export { formatCenter, formatZoom };
