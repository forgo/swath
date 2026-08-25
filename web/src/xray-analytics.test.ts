// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The analytics math against hand-computed expectations (issue #111):
// every percentile assertion below carries the arithmetic that produced
// it, so the test IS the spec of the quantile method (linear
// interpolation between closest ranks, rank = p * (n - 1)). The DOM
// face and the overlay wiring are covered here and in swath-xray.test.ts
// respectively; the against-the-real-stream proof is
// web/e2e/swath-xray.e2e.ts.
import { expect, test } from "vitest";
import { AnalyticsPanel, quantileSorted, TraceAnalytics } from "./xray-analytics.js";

test("quantileSorted: empty sample has no quantile — not 0", () => {
  expect(quantileSorted([], 0.5)).toBeUndefined();
  expect(quantileSorted([], 0.95)).toBeUndefined();
});

test("quantileSorted: a single sample is every quantile", () => {
  expect(quantileSorted([7], 0)).toBe(7);
  expect(quantileSorted([7], 0.5)).toBe(7);
  expect(quantileSorted([7], 0.95)).toBe(7);
  expect(quantileSorted([7], 1)).toBe(7);
});

test("quantileSorted: hand-computed p50/p95, even-sized sample", () => {
  // n=4, sorted [10, 20, 30, 40].
  // p50: rank = 0.50 * 3 = 1.5 → 20 + 0.5 * (30 - 20) = 25.
  // p95: rank = 0.95 * 3 = 2.85 → 30 + 0.85 * (40 - 30) = 38.5.
  expect(quantileSorted([10, 20, 30, 40], 0.5)).toBe(25);
  expect(quantileSorted([10, 20, 30, 40], 0.95)).toBe(38.5);
});

test("quantileSorted: hand-computed p50/p95, odd-sized sample", () => {
  // n=5, sorted [1, 2, 3, 4, 5].
  // p50: rank = 0.50 * 4 = 2 (integer) → the middle element, 3.
  // p95: rank = 0.95 * 4 = 3.8 → 4 + 0.8 * (5 - 4) = 4.8.
  expect(quantileSorted([1, 2, 3, 4, 5], 0.5)).toBe(3);
  expect(quantileSorted([1, 2, 3, 4, 5], 0.95)).toBe(4.8);
  // Extremes are the order statistics themselves.
  expect(quantileSorted([1, 2, 3, 4, 5], 0)).toBe(1);
  expect(quantileSorted([1, 2, 3, 4, 5], 1)).toBe(5);
});

test("TraceAnalytics sorts internally: arrival order never matters", () => {
  const analytics = new TraceAnalytics();
  for (const totalMs of [40, 10, 30, 20]) {
    analytics.record("live", totalMs);
  }
  // Same sample as the even-sized case above, delivered shuffled.
  expect(analytics.quantile(0.5)).toBe(25);
  expect(analytics.quantile(0.95)).toBe(38.5);
});

test("the window slides: oldest sample evicted, counters keep all-time", () => {
  const analytics = new TraceAnalytics(4);
  for (const totalMs of [10, 20, 30, 40]) {
    analytics.record("live", totalMs);
  }
  expect(analytics.sampleCount).toBe(4);
  // A fifth sample evicts the 10: window is now [20, 30, 40, 100].
  // p50: rank 1.5 → 30 + 0.5 * (40 - 30) = 35.
  // p95: rank 2.85 → 40 + 0.85 * (100 - 40) = 91 (to float precision:
  // 0.95 * 3 is 2.8499999999999996 in binary, hence closeTo).
  analytics.record("live", 100);
  expect(analytics.sampleCount).toBe(4);
  expect(analytics.quantile(0.5)).toBe(35);
  expect(analytics.quantile(0.95)).toBeCloseTo(91, 10);
  // The mix is all-time: the evicted trace still counts.
  expect(analytics.total).toBe(5);
  expect(analytics.mix.live).toBe(5);
});

test("decision mix and hit rate: keyed and bare decisions both count", () => {
  const analytics = new TraceAnalytics();
  expect(analytics.hitRate).toBeUndefined(); // 0/0 is "no data", not 0%
  analytics.record("live", 18);
  analytics.record({ overview: { level: 2 } }, 9);
  analytics.record("cache_hit", 1); // pre-#36 bare form
  analytics.record({ cache_hit: { key: "0123abcd" } }, 2); // keyed form (#36)
  analytics.record("live", 22);
  expect(analytics.mix).toEqual({ live: 2, overview: 1, cache_hit: 2 });
  expect(analytics.total).toBe(5);
  expect(analytics.hitRate).toBe(2 / 5);
});

test("per-frame mix: traces bucket under their temporal granule_datetime (issue #182)", () => {
  const analytics = new TraceAnalytics();
  expect(analytics.latestFrame).toBeUndefined();
  // A static-layer trace (no frame) touches only the all-time mix.
  analytics.record("live", 10);
  expect(analytics.latestFrame).toBeUndefined();

  const pre = "2024-07-22T19:03:00Z";
  const post = "2024-08-16T19:03:00Z";
  analytics.record("live", 20, pre);
  analytics.record("live", 30, pre);
  analytics.record({ cache_hit: { key: "abcd1234" } }, 2, pre);
  analytics.record("live", 40, post);
  expect(analytics.latestFrame).toBe(post);
  expect(analytics.frameMix(pre)).toEqual({ live: 2, overview: 0, cache_hit: 1 });
  expect(analytics.frameMix(post)).toEqual({ live: 1, overview: 0, cache_hit: 0 });
  expect(analytics.frameMix("2030-01-01T00:00:00Z")).toBeUndefined();
  // The all-time mix still counts every trace, framed or not.
  expect(analytics.mix).toEqual({ live: 4, overview: 0, cache_hit: 1 });
});

test("panel shows the latest frame's own plan mix; hidden without temporal traces", () => {
  const panel = new AnalyticsPanel(document);
  document.body.append(panel.element);
  const { element } = panel;
  const frameLine = element.querySelector<HTMLElement>(".swath-xray-analytics-frame");
  expect(frameLine?.hidden).toBe(true);
  panel.record("live", 10); // static layer: line stays hidden
  expect(frameLine?.hidden).toBe(true);
  expect(element.dataset.frame).toBeUndefined();

  const frame = "2024-08-16T19:03:00Z";
  panel.record("live", 20, frame);
  panel.record({ cache_hit: { key: "feedbeef" } }, 2, frame);
  expect(frameLine?.hidden).toBe(false);
  expect(element.dataset.frame).toBe(frame);
  expect(element.dataset.frameLive).toBe("1");
  expect(element.dataset.frameOverview).toBe("0");
  expect(element.dataset.frameCacheHit).toBe("1");
  expect(frameLine?.textContent).toBe(`frame ${frame} · live 1 · ovr 0 · cache 1`);
  element.remove();
});

test("panel renders exact values in data attributes, formatted text on top", () => {
  const panel = new AnalyticsPanel(document);
  document.body.append(panel.element);
  const { element } = panel;
  expect(element.getAttribute("aria-label")).toBe("Trace analytics");
  // Empty: percentiles and hit rate show "no data", not zeros.
  expect(element.textContent).toContain("p50 — · p95 — ms");
  expect(element.textContent).toContain("hit —");
  expect(element.dataset.p50).toBeUndefined();
  expect(element.dataset.hitRate).toBeUndefined();

  // The scripted mix: 3 live, 1 overview, 2 cache hits.
  panel.record("live", 10);
  panel.record("live", 20);
  panel.record({ overview: { level: 2 } }, 30);
  panel.record("cache_hit", 5);
  panel.record({ cache_hit: { key: "feedbeef" } }, 15);
  panel.record("live", 40);

  // Window sorted: [5, 10, 15, 20, 30, 40], n=6.
  // p50: rank 2.5 → 15 + 0.5 * (20 - 15) = 17.5.
  // p95: rank 4.75 → 30 + 0.75 * (40 - 30) = 37.5.
  expect(element.dataset.p50).toBe("17.5");
  expect(element.dataset.p95).toBe("37.5");
  expect(element.dataset.samples).toBe("6");
  expect(element.textContent).toContain("p50 17.5 · p95 37.5 ms (last 6)");
  expect(element.dataset.live).toBe("3");
  expect(element.dataset.overview).toBe("1");
  expect(element.dataset.cacheHit).toBe("2");
  expect(element.dataset.total).toBe("6");
  expect(element.dataset.hitRate).toBe(String(2 / 6));
  expect(element.textContent).toContain("live 3");
  expect(element.textContent).toContain("ovr 1");
  expect(element.textContent).toContain("cache 2");
  expect(element.textContent).toContain("hit 33.3%");
  element.remove();
});

test("a viewed frame pins the per-frame line, whatever traced last (issue #211)", () => {
  const panel = new AnalyticsPanel(document);
  document.body.append(panel.element);
  const { element } = panel;
  const mine = "2024-08-16T19:03:00Z";
  const theirs = "2024-06-07T19:03:00Z";

  // Pinned before any trace: an honest all-zero line for that frame.
  panel.setViewedFrame(mine);
  expect(element.dataset.frame).toBe(mine);
  expect(element.dataset.frameLive).toBe("0");
  // Another page's loop traces a different frame: the line holds.
  panel.record("live", 20, theirs);
  panel.record({ cache_hit: { key: "feedbeef" } }, 2, theirs);
  expect(element.dataset.frame).toBe(mine);
  panel.record("live", 30, mine);
  expect(element.dataset.frameLive).toBe("1");
  expect(element.dataset.frameCacheHit).toBe("0");
  // Unpinned: back to the last traced frame.
  panel.setViewedFrame(null);
  expect(element.dataset.frame).toBe(mine); // mine traced last
  panel.record("live", 5, theirs);
  expect(element.dataset.frame).toBe(theirs);
  element.remove();
});

test("the per-tile UDF line (#208): latest fuel and udf_ms, hidden until a UDF trace arrives", () => {
  const analytics = new TraceAnalytics();
  const panel = new AnalyticsPanel(document, analytics);
  const { element } = panel;
  document.body.append(element);
  const line = element.querySelector<HTMLElement>(".swath-xray-analytics-udf");
  panel.record("live", 10);
  expect(line?.hidden).toBe(true);
  expect(analytics.latestUdf).toBeUndefined();
  expect(analytics.udfTiles).toBe(0);

  panel.record("live", 30, undefined, { tile: "udf/12/1561/848", fuel: 3_276_800, ms: 4 });
  expect(line?.hidden).toBe(false);
  expect(line?.textContent).toBe("udf udf/12/1561/848 · fuel 3276800 · 4 ms (1 udf tile)");
  expect(element.dataset.udfTile).toBe("udf/12/1561/848");
  expect(element.dataset.udfFuel).toBe("3276800");
  expect(element.dataset.udfMs).toBe("4");
  expect(element.dataset.udfTiles).toBe("1");

  // Latest wins; a sample without fuel (udf_ms only) reads "—", never 0.
  panel.record("live", 30, undefined, { tile: "udf/12/1561/849", fuel: undefined, ms: 2 });
  expect(line?.textContent).toBe("udf udf/12/1561/849 · fuel — · 2 ms (2 udf tiles)");
  expect(element.dataset.udfFuel).toBeUndefined();
  expect(element.dataset.udfTiles).toBe("2");
  // Copies out, never the internal record.
  const latest = analytics.latestUdf;
  if (latest) {
    latest.ms = 999;
  }
  expect(analytics.latestUdf?.ms).toBe(2);
  element.remove();
});
