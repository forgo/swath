// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The view-state semantics (issue #108), pinned without any DOM wiring:
// URL parse/format round trips, THE precedence rule (params beat storage
// beats default), the byte-stability equality guard, and a storage codec
// that shrugs off corruption. Runs in the same real browser as the rest
// of the suite, so `localStorage` here is the genuine article.
import { afterEach, expect, test } from "vitest";
import {
  formatCenter,
  formatZoom,
  hasViewParams,
  loadViewState,
  parseViewState,
  resolveInitialState,
  STORAGE_KEY,
  saveViewState,
  type ViewState,
  viewStatesEqual,
  withViewState,
} from "./view-state.js";

afterEach(() => {
  localStorage.clear();
});

test("parseViewState reads layer, center, zoom, and the bare xray flag", () => {
  expect(parseViewState("?layer=ndvi&center=-106.1,39.3&zoom=12.5&xray")).toEqual({
    layer: "ndvi",
    center: [-106.1, 39.3],
    zoom: 12.5,
    xray: true,
  });
  expect(parseViewState("")).toEqual({ xray: false });
  // Malformed values degrade field-by-field, never throw.
  expect(parseViewState("?center=oops&zoom=NaN&layer=")).toEqual({ xray: false });
});

test("withViewState writes the canonical query and round-trips", () => {
  const state: ViewState = { layer: "ndvi", center: [-106.1, 39.3], zoom: 12.5, xray: true };
  const query = withViewState("", state);
  expect(query).toBe("?layer=ndvi&center=-106.1,39.3&zoom=12.5&xray");
  expect(parseViewState(query)).toEqual(state);
  // A default state with no foreign params is a bare URL again.
  expect(withViewState("?layer=old&xray", { xray: false })).toBe("");
});

test("withViewState preserves params it does not own (basemap)", () => {
  const query = withViewState("?basemap=demo&layer=old", { layer: "ndvi", xray: false });
  expect(query).toBe("?layer=ndvi&basemap=demo");
});

test("format trims trailing zeros at fixed precision", () => {
  expect(formatCenter([-106.000004, 39.30001])).toBe("-106,39.30001");
  expect(formatZoom(12)).toBe("12");
  expect(formatZoom(12.3456)).toBe("12.35");
});

test("viewStatesEqual tolerates round-trip precision, not real movement", () => {
  const state: ViewState = { layer: "ndvi", center: [-106.123456, 39.3], zoom: 12.34, xray: false };
  const roundTripped = parseViewState(withViewState("", state));
  expect(viewStatesEqual(state, roundTripped)).toBe(true);
  expect(viewStatesEqual(state, { ...state, center: [-106.2, 39.3] })).toBe(false);
  expect(viewStatesEqual(state, { ...state, zoom: 13 })).toBe(false);
  expect(viewStatesEqual(state, { ...state, layer: "truecolor" })).toBe(false);
  expect(viewStatesEqual(state, { ...state, xray: true })).toBe(false);
  // Presence differences are differences (a URL without a center does
  // not equal a state with one).
  expect(viewStatesEqual({ xray: false }, { xray: false, zoom: 3 })).toBe(false);
});

test("precedence: any URL view param beats storage entirely — no merging", () => {
  saveViewState(localStorage, { layer: "stored", center: [1, 2], zoom: 9, xray: true });
  const { state, source } = resolveInitialState("?layer=ndvi", localStorage);
  expect(source).toBe("url");
  // The stored center/zoom/xray must NOT leak into a shared link's view.
  expect(state).toEqual({ layer: "ndvi", xray: false });
});

test("precedence: storage restores the last session on a paramless visit", () => {
  saveViewState(localStorage, { layer: "stored", center: [1, 2], zoom: 9, xray: false });
  const { state, source } = resolveInitialState("", localStorage);
  expect(source).toBe("storage");
  expect(state).toEqual({ layer: "stored", center: [1, 2], zoom: 9, xray: false });
});

test("precedence: empty storage falls through to the zero-config default", () => {
  const { state, source } = resolveInitialState("", localStorage);
  expect(source).toBe("default");
  expect(state).toEqual({ xray: false });
  // And with no storage at all (disabled), same default.
  expect(resolveInitialState("", undefined).source).toBe("default");
});

test("hasViewParams triggers on exactly the owned params", () => {
  expect(hasViewParams("?layer=x")).toBe(true);
  expect(hasViewParams("?xray")).toBe(true);
  expect(hasViewParams("?basemap=demo")).toBe(false);
  expect(hasViewParams("")).toBe(false);
});

test("storage codec survives corruption and junk shapes", () => {
  localStorage.setItem(STORAGE_KEY, "not json {");
  expect(loadViewState(localStorage)).toBeUndefined();
  localStorage.setItem(STORAGE_KEY, JSON.stringify(["nope"]));
  // Arrays are objects: field validation still yields a safe default.
  expect(loadViewState(localStorage)).toEqual({ xray: false });
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({ layer: 7, center: [1, "two"], zoom: "high", xray: "yes" }),
  );
  expect(loadViewState(localStorage)).toEqual({ xray: false });
  localStorage.removeItem(STORAGE_KEY);
  expect(loadViewState(localStorage)).toBeUndefined();
});
