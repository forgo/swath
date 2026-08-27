// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * App state in the URL (docs/design/ui-system.md §4.5, issue #283) — the
 * mode and its selection, beside `view-state.ts` and in its shape
 * (parse / format / equal / resolve / save / load). The URL-is-truth rule
 * (ADR 0011) extended, not forked:
 *
 * - `view=` is the mode: `layers | data | author | xray`. Absent = `layers`
 *   and NEVER written when default, so a bare `/` stays bare. Unknown
 *   values degrade to `layers`.
 * - `sel=<node-id>` is meaningful only under `view=author`; parsed
 *   otherwise → dropped.
 * - `rail=collapsed` is honoured when present in a URL (a deep link is
 *   never rewritten) but never written by interaction: collapse is a
 *   device preference, persisted in storage only.
 * - Writes compose `withViewState` then `withAppState` on one search
 *   string; both pass foreign params (`stac=`, `basemap=`) through.
 *
 * The x-ray OVERLAY (view-state's bare `xray` flag) and the analytics MODE
 * (`view=xray`) are distinct; the host turns the overlay on when entering
 * the mode and leaves it alone on leaving.
 */

export const VIEW_MODES = ["layers", "data", "author", "xray"] as const;
export type ViewMode = (typeof VIEW_MODES)[number];

export interface AppState {
  view: ViewMode;
  /** The selected authoring node; only under `author`. */
  sel?: string;
  /** Read from a deep link only; never written. */
  rail?: "collapsed";
}

/** What storage remembers between visits: the mode and the rail preference. */
export interface AppPreference {
  view: ViewMode;
  rail?: "collapsed";
}

export type AppStateSource = "url" | "storage" | "default";

export const APP_STORAGE_KEY = "swath.app-state.v1";

/** Params this module writes. `rail` is read but never written, so it is
 * foreign to the writer and survives untouched. */
const WRITTEN_PARAMS = ["view", "sel"] as const;

export function isViewMode(value: unknown): value is ViewMode {
  return typeof value === "string" && (VIEW_MODES as readonly string[]).includes(value);
}

export function hasAppParams(search: string): boolean {
  return new URLSearchParams(search).has("view");
}

export function parseAppState(search: string): AppState {
  const params = new URLSearchParams(search);
  const raw = params.get("view");
  const view: ViewMode = isViewMode(raw) ? raw : "layers";
  const state: AppState = { view };
  const sel = params.get("sel");
  if (view === "author" && sel !== null && sel !== "") {
    state.sel = sel;
  }
  if (params.get("rail") === "collapsed") {
    state.rail = "collapsed";
  }
  return state;
}

/** The search string with this module's params rewritten canonically:
 * `view` only when not the default, `sel` only under `author`, foreign
 * params (and `rail`) preserved in their order. */
export function withAppState(search: string, state: AppState): string {
  // Foreign params are copied byte-for-byte (not through URLSearchParams,
  // which would rewrite view-state's bare `xray` flag as `xray=`).
  const foreign = search
    .replace(/^\?/, "")
    .split("&")
    .filter((pair) => {
      if (pair === "") {
        return false;
      }
      const key = decodeURIComponent(pair.split("=")[0] ?? "");
      return !(WRITTEN_PARAMS as readonly string[]).includes(key);
    });
  const parts: string[] = [];
  if (state.view !== "layers") {
    parts.push(`view=${state.view}`);
  }
  if (state.view === "author" && state.sel !== undefined && state.sel !== "") {
    parts.push(`sel=${encodeURIComponent(state.sel)}`);
  }
  parts.push(...foreign);
  return parts.length === 0 ? "" : `?${parts.join("&")}`;
}

export function appStatesEqual(a: AppState, b: AppState): boolean {
  return a.view === b.view && a.sel === b.sel && a.rail === b.rail;
}

/** Derived, never serialised: the inspector column is open under `author`
 * with a selection. */
export function inspectorOpen(state: AppState): boolean {
  return state.view === "author" && state.sel !== undefined && state.sel !== "";
}

export function resolveInitialAppState(
  search: string,
  storage: Storage | undefined,
): { state: AppState; source: AppStateSource } {
  const stored = storage ? loadAppPreference(storage) : undefined;
  if (hasAppParams(search)) {
    const state = parseAppState(search);
    // The rail preference is the device's even under a deep link that
    // says nothing about it.
    if (state.rail === undefined && stored?.rail !== undefined) {
      state.rail = stored.rail;
    }
    return { state, source: "url" };
  }
  if (stored) {
    const state: AppState = { view: stored.view };
    if (stored.rail !== undefined) {
      state.rail = stored.rail;
    }
    return { state, source: "storage" };
  }
  return { state: { view: "layers" }, source: "default" };
}

export function saveAppPreference(storage: Storage, preference: AppPreference): void {
  const record: AppPreference = { view: preference.view };
  if (preference.rail !== undefined) {
    record.rail = preference.rail;
  }
  try {
    storage.setItem(APP_STORAGE_KEY, JSON.stringify(record));
  } catch {
    // Quota or privacy mode: the URL still carries the truth.
  }
}

export function loadAppPreference(storage: Storage): AppPreference | undefined {
  let raw: string | null;
  try {
    raw = storage.getItem(APP_STORAGE_KEY);
  } catch {
    return undefined;
  }
  if (raw === null) {
    return undefined;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return undefined;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return undefined;
  }
  const record = parsed as { view?: unknown; rail?: unknown };
  if (!isViewMode(record.view)) {
    return undefined;
  }
  const preference: AppPreference = { view: record.view };
  if (record.rail === "collapsed") {
    preference.rail = "collapsed";
  }
  return preference;
}
