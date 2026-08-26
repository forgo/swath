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
  parseTime,
  parseViewState,
  resolveInitialState,
  STORAGE_KEY,
  saveViewState,
  shareUrl,
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

test("stac (the Open-in-Swath entry, issue #197) is foreign: passed through, never a view param", () => {
  // Deep links like /?stac=<item-url> must survive URL rewrites…
  const search = "?stac=https%3A%2F%2Fdata.test%2Fitem.json";
  const query = withViewState(search, { layer: "ndvi", xray: false });
  expect(query).toBe("?layer=ndvi&stac=https%3A%2F%2Fdata.test%2Fitem.json");
  // …and must not count as view state (URL-beats-storage stays untouched).
  expect(hasViewParams(search)).toBe(false);
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

test("shareUrl writes the full explicit view onto the page URL (issue #211)", () => {
  const state: ViewState = {
    layer: "park-fire-ndvi",
    center: [-121.6932, 40.0208],
    zoom: 12.4,
    time: "2024-08-16T19:03:00Z",
    xray: true,
  };
  // A bare landing: the copied link carries what the viewer resolved to.
  expect(shareUrl("http://localhost:5173/demo/", state)).toBe(
    "http://localhost:5173/demo/?layer=park-fire-ndvi&center=-121.6932,40.0208&zoom=12.4&t=2024-08-16T19:03:00Z&xray",
  );
  // Foreign params ride along, the hash survives, and a URL that already
  // says this comes back byte-identical (the address-bar agreement).
  const deepLink =
    "http://localhost:5173/demo/?layer=park-fire-ndvi&center=-121.6932,40.0208&zoom=12.4&t=2024-08-16T19:03:00Z&xray&basemap=demo#top";
  expect(shareUrl(deepLink, state)).toBe(deepLink);
  expect(shareUrl(deepLink, parseViewState(new URL(deepLink).search))).toBe(deepLink);
  // Compare state (issue #210) rides the same writer: `ct`/`cl` + `swipe`.
  const compared: ViewState = { ...state, compareTime: "2024-06-07T19:03:00Z", swipe: 0.35 };
  const comparedLink =
    "http://localhost:5173/demo/?layer=park-fire-ndvi&center=-121.6932,40.0208&zoom=12.4&t=2024-08-16T19:03:00Z&ct=2024-06-07T19:03:00Z&swipe=0.35&xray";
  expect(shareUrl("http://localhost:5173/demo/", compared)).toBe(comparedLink);
  expect(shareUrl(comparedLink, parseViewState(new URL(comparedLink).search))).toBe(comparedLink);
});

test("hasViewParams triggers on exactly the owned params", () => {
  expect(hasViewParams("?layer=x")).toBe(true);
  expect(hasViewParams("?xray")).toBe(true);
  expect(hasViewParams("?t=2024-08-16T19:03:00Z")).toBe(true);
  expect(hasViewParams("?basemap=demo")).toBe(false);
  expect(hasViewParams("")).toBe(false);
});

test("t joins the owned params: parsed, written verbatim, round-tripped (issue #182)", () => {
  // Parse: an RFC 3339 UTC instant, exactly the tile route's grammar.
  expect(parseTime("2024-08-16T19:03:00Z")).toBe("2024-08-16T19:03:00Z");
  expect(parseTime("2024-08-16T19:03:00.500Z")).toBe("2024-08-16T19:03:00.500Z");
  // Malformed values degrade to "latest" — and can never smuggle a
  // reserved character into the query string the writer emits verbatim.
  for (const bad of [null, "", "yesterday", "2024-08-16", "2024-08-16T19:03:00+00:00", "a&b=c"]) {
    expect(parseTime(bad)).toBeUndefined();
  }
  expect(parseViewState("?layer=park-fire-ndvi&t=2024-08-16T19:03:00Z")).toEqual({
    layer: "park-fire-ndvi",
    time: "2024-08-16T19:03:00Z",
    xray: false,
  });
  expect(parseViewState("?t=nope")).toEqual({ xray: false });

  // Write: verbatim (no percent-encoded colons — the hand-written
  // deep-link style), and the round trip is the identity.
  const state: ViewState = { layer: "park-fire-ndvi", time: "2024-08-16T19:03:00Z", xray: true };
  const query = withViewState("", state);
  expect(query).toBe("?layer=park-fire-ndvi&t=2024-08-16T19:03:00Z&xray");
  expect(parseViewState(query)).toEqual(state);

  // Equality: a different (or missing) frame is a different view.
  expect(viewStatesEqual(state, { ...state })).toBe(true);
  expect(viewStatesEqual(state, { ...state, time: "2024-09-05T19:03:00Z" })).toBe(false);
  expect(viewStatesEqual(state, { layer: "park-fire-ndvi", xray: true })).toBe(false);

  // Storage: time round-trips; junk shapes are dropped, never kept.
  saveViewState(localStorage, state);
  expect(loadViewState(localStorage)).toEqual(state);
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ time: "yesterday", xray: false }));
  expect(loadViewState(localStorage)).toEqual({ xray: false });
});

test("compare params (issue #210): ct/cl/swipe parse, write verbatim, round-trip", () => {
  // Date-vs-date: `ct` shares `t`'s grammar and verbatim writing.
  const dates: ViewState = {
    layer: "park-fire-ndvi",
    time: "2024-06-07T19:03:00Z",
    compareTime: "2024-09-05T19:03:00Z",
    swipe: 0.35,
    xray: false,
  };
  const datesQuery = withViewState("", dates);
  expect(datesQuery).toBe(
    "?layer=park-fire-ndvi&t=2024-06-07T19:03:00Z&ct=2024-09-05T19:03:00Z&swipe=0.35",
  );
  expect(parseViewState(datesQuery)).toEqual(dates);

  // Layer-vs-layer: `cl` is the right side's layer id.
  const layers: ViewState = { layer: "ndvi", compareLayer: "truecolor", xray: false };
  const layersQuery = withViewState("", layers);
  expect(layersQuery).toBe("?layer=ndvi&cl=truecolor");
  expect(parseViewState(layersQuery)).toEqual(layers);

  // All compare params are owned: URL-beats-storage triggers on each.
  expect(hasViewParams("?ct=2024-09-05T19:03:00Z")).toBe(true);
  expect(hasViewParams("?cl=truecolor")).toBe(true);
  expect(hasViewParams("?swipe=0.5")).toBe(true);
});

test("compare params degrade coherently: ambiguity, self-compare, junk", () => {
  // Both modes at once is ambiguous — no compare, never a guess.
  expect(parseViewState("?layer=ndvi&ct=2024-09-05T19:03:00Z&cl=truecolor")).toEqual({
    layer: "ndvi",
    xray: false,
  });
  // Comparing a layer with itself compares nothing.
  expect(parseViewState("?layer=ndvi&cl=ndvi")).toEqual({ layer: "ndvi", xray: false });
  // A malformed ct degrades to no compare (and drops the stray swipe).
  expect(parseViewState("?layer=ndvi&ct=yesterday&swipe=0.4")).toEqual({
    layer: "ndvi",
    xray: false,
  });
  // A swipe without any compare means nothing.
  expect(parseViewState("?layer=ndvi&swipe=0.4")).toEqual({ layer: "ndvi", xray: false });
  // An out-of-range swipe degrades to the default (absent).
  expect(parseViewState("?cl=truecolor&swipe=7")).toEqual({
    compareLayer: "truecolor",
    xray: false,
  });
  // The writer never emits swipe for a compare-less state either.
  expect(withViewState("", { layer: "ndvi", swipe: 0.4, xray: false })).toBe("?layer=ndvi");
});

test("compare state: equality, storage round-trip, corrupt shapes dropped", () => {
  const state: ViewState = {
    layer: "park-fire-ndvi",
    compareTime: "2024-09-05T19:03:00Z",
    swipe: 0.5,
    xray: false,
  };
  // Round trips through the URL compare equal; real changes do not.
  expect(viewStatesEqual(state, parseViewState(withViewState("", state)))).toBe(true);
  expect(viewStatesEqual(state, { ...state, compareTime: "2024-06-07T19:03:00Z" })).toBe(false);
  expect(viewStatesEqual(state, { layer: "park-fire-ndvi", xray: false })).toBe(false);
  expect(viewStatesEqual(state, { ...state, swipe: 0.7 })).toBe(false);
  // Swipe tolerates its own write precision, not a deliberate move.
  expect(viewStatesEqual(state, { ...state, swipe: 0.504 })).toBe(true);
  const byLayer: ViewState = { layer: "ndvi", compareLayer: "truecolor", xray: false };
  expect(viewStatesEqual(byLayer, { ...byLayer })).toBe(true);
  expect(viewStatesEqual(byLayer, { ...byLayer, compareLayer: "other" })).toBe(false);

  // Storage: compare state round-trips…
  saveViewState(localStorage, state);
  expect(loadViewState(localStorage)).toEqual(state);
  // …ambiguous or junk stored shapes are dropped, never kept.
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({ compareTime: "2024-09-05T19:03:00Z", compareLayer: "x", xray: false }),
  );
  expect(loadViewState(localStorage)).toEqual({ xray: false });
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({ layer: "ndvi", compareLayer: "ndvi", swipe: 2, xray: false }),
  );
  expect(loadViewState(localStorage)).toEqual({ layer: "ndvi", xray: false });
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify({ compareTime: "junk", swipe: 0.4, xray: false }),
  );
  expect(loadViewState(localStorage)).toEqual({ xray: false });
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
