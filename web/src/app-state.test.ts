// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Mirrors view-state.test.ts: the same contract, one module over.
import { expect, test } from "vitest";
import {
  APP_STORAGE_KEY,
  appStatesEqual,
  hasAppParams,
  inspectorOpen,
  loadAppPreference,
  parseAppState,
  resolveInitialAppState,
  saveAppPreference,
  withAppState,
} from "./app-state.js";
import { withViewState } from "./view-state.js";

function memoryStorage(): Storage {
  const map = new Map<string, string>();
  return {
    get length() {
      return map.size;
    },
    clear: () => map.clear(),
    getItem: (key) => map.get(key) ?? null,
    key: (index) => [...map.keys()][index] ?? null,
    removeItem: (key) => {
      map.delete(key);
    },
    setItem: (key, value) => {
      map.set(key, value);
    },
  };
}

test("parseAppState: absent = layers; unknown degrades; sel only under author; rail read", () => {
  expect(parseAppState("")).toEqual({ view: "layers" });
  expect(parseAppState("?view=data")).toEqual({ view: "data" });
  expect(parseAppState("?view=bogus")).toEqual({ view: "layers" });
  expect(parseAppState("?view=author&sel=n1")).toEqual({ view: "author", sel: "n1" });
  expect(parseAppState("?view=data&sel=n1")).toEqual({ view: "data" });
  expect(parseAppState("?sel=n1")).toEqual({ view: "layers" });
  expect(parseAppState("?rail=collapsed")).toEqual({ view: "layers", rail: "collapsed" });
  expect(parseAppState("?rail=open")).toEqual({ view: "layers" });
});

test("withAppState: never writes the default, writes sel only under author, round-trips", () => {
  expect(withAppState("", { view: "layers" })).toBe("");
  expect(withAppState("?view=data", { view: "layers" })).toBe("");
  expect(withAppState("", { view: "data" })).toBe("?view=data");
  expect(withAppState("", { view: "author", sel: "a b" })).toBe("?view=author&sel=a%20b");
  expect(withAppState("", { view: "data", sel: "n1" })).toBe("?view=data");
  for (const search of ["", "?view=data", "?view=author&sel=n1", "?view=xray"]) {
    expect(withAppState(search, parseAppState(search))).toBe(search);
  }
});

test("rail=collapsed is honoured from a link and passed through, but never introduced", () => {
  expect(withAppState("?rail=collapsed", { view: "data", rail: "collapsed" })).toBe(
    "?view=data&rail=collapsed",
  );
  expect(withAppState("", { view: "data", rail: "collapsed" })).toBe("?view=data");
});

test("foreign params (stac, basemap) are preserved; view-state's are untouched", () => {
  expect(withAppState("?stac=https%3A%2F%2Fx%2Fitem.json&basemap=demo", { view: "data" })).toBe(
    "?view=data&stac=https%3A%2F%2Fx%2Fitem.json&basemap=demo",
  );
  expect(withAppState("?layer=ndvi&zoom=8&view=layers", { view: "author", sel: "n" })).toBe(
    "?view=author&sel=n&layer=ndvi&zoom=8",
  );
});

test("composes with withViewState on one search string (view first, then app), byte-stable", () => {
  const view = { layer: "ndvi", zoom: 8, xray: true };
  const app = { view: "author" as const, sel: "n1" };
  const a = withAppState(withViewState("?basemap=demo", view), app);
  // The bare `xray` flag survives the pass-through — not rewritten as `xray=`.
  expect(a).toBe("?view=author&sel=n1&layer=ndvi&zoom=8&xray&basemap=demo");
  expect(withAppState(a, app)).toBe(a); // idempotent: the same state rewrites the same bytes
  const b = withViewState(withAppState("?basemap=demo", app), view);
  const sorted = (search: string) => {
    const params = new URLSearchParams(search);
    params.sort();
    return params.toString();
  };
  expect(sorted(b)).toBe(sorted(a)); // the other order: same params, view-state's first
  // A second write of the same state is a no-op the caller can detect.
  expect(appStatesEqual(parseAppState(a), app)).toBe(true);
});

test("hasAppParams triggers on view only; inspectorOpen is author + selection", () => {
  expect(hasAppParams("?view=data")).toBe(true);
  expect(hasAppParams("?sel=n1")).toBe(false);
  expect(hasAppParams("?rail=collapsed")).toBe(false);
  expect(inspectorOpen({ view: "author", sel: "n1" })).toBe(true);
  expect(inspectorOpen({ view: "author" })).toBe(false);
  expect(inspectorOpen({ view: "data", sel: "n1" })).toBe(false);
});

test("precedence: a URL mode beats storage; storage restores a bare visit; default is layers", () => {
  const storage = memoryStorage();
  saveAppPreference(storage, { view: "data", rail: "collapsed" });
  expect(resolveInitialAppState("?view=author", storage)).toEqual({
    state: { view: "author", rail: "collapsed" }, // the device's rail preference still applies
    source: "url",
  });
  expect(resolveInitialAppState("", storage)).toEqual({
    state: { view: "data", rail: "collapsed" },
    source: "storage",
  });
  expect(resolveInitialAppState("", memoryStorage())).toEqual({
    state: { view: "layers" },
    source: "default",
  });
  expect(resolveInitialAppState("", undefined).source).toBe("default");
});

test("storage codec: view + rail only (never sel); corruption and junk shapes are dropped", () => {
  const storage = memoryStorage();
  saveAppPreference(storage, { view: "author" });
  expect(storage.getItem(APP_STORAGE_KEY)).toBe('{"view":"author"}');
  expect(loadAppPreference(storage)).toEqual({ view: "author" });
  for (const junk of ["not json", "42", "null", '{"view":"bogus"}', '{"rail":"collapsed"}']) {
    storage.setItem(APP_STORAGE_KEY, junk);
    expect(loadAppPreference(storage), junk).toBeUndefined();
  }
  storage.setItem(APP_STORAGE_KEY, '{"view":"xray","rail":"open"}');
  expect(loadAppPreference(storage)).toEqual({ view: "xray" });
});

test("the search scope's dates: round-trips both bounds, and either alone", () => {
  expect(parseAppState("?view=data&from=2024-01-01&to=2024-02-29")).toEqual({
    view: "data",
    from: "2024-01-01",
    to: "2024-02-29",
  });
  expect(parseAppState("?from=2024-01-01")).toEqual({ view: "layers", from: "2024-01-01" });
  expect(withAppState("", { view: "data", from: "2024-01-01", to: "2024-02-29" })).toBe(
    "?view=data&from=2024-01-01&to=2024-02-29",
  );
  expect(withAppState("", { view: "layers", to: "2024-02-29" })).toBe("?to=2024-02-29");
});

test("the search scope's dates: drops a bound that is not a date, rather than filtering by nonsense", () => {
  expect(parseAppState("?from=yesterday&to=2024-13-99").from).toBeUndefined();
  expect(parseAppState("?from=yesterday&to=2024-13-99").to).toBeUndefined();
});

test("the search scope's dates: is an artifact: changing the dates is navigation, not a camera move", () => {
  const base = { view: "data" } as const;
  expect(appStatesEqual(base, { ...base, from: "2024-01-01" })).toBe(false);
  expect(appStatesEqual({ ...base, to: "2024-02-29" }, { ...base, to: "2024-02-29" })).toBe(true);
});

test("the search scope's dates: clears the params when the dates are gone", () => {
  expect(withAppState("?view=data&from=2024-01-01&to=2024-02-29", { view: "data" })).toBe(
    "?view=data",
  );
});
