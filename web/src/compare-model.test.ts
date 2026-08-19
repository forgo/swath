// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The compare swipe's semantics (issue #210), pinned without any DOM:
// mode resolution (exclusive, self-compare dropped), side identities,
// trace-to-side matching (the per-side badge rule), and swipe clamping.
import { expect, test } from "vitest";
import {
  clampSwipe,
  compareSides,
  DEFAULT_SWIPE,
  resolveCompare,
  sideLabels,
  traceSide,
} from "./compare-model.js";

const T0 = "2024-06-07T19:03:00Z";
const T1 = "2024-09-05T19:03:00Z";

test("resolveCompare: one coherent mode, or nothing", () => {
  expect(resolveCompare("ndvi", T1, undefined)).toEqual({ mode: "date", value: T1 });
  expect(resolveCompare("ndvi", undefined, "truecolor")).toEqual({
    mode: "layer",
    value: "truecolor",
  });
  // Both at once is ambiguous — no compare, never a guess.
  expect(resolveCompare("ndvi", T1, "truecolor")).toBeUndefined();
  // Comparing a layer with itself compares nothing.
  expect(resolveCompare("ndvi", undefined, "ndvi")).toBeUndefined();
  // Empty/absent inputs are no compare.
  expect(resolveCompare("ndvi", undefined, undefined)).toBeUndefined();
  expect(resolveCompare("ndvi", undefined, "")).toBeUndefined();
});

test("compareSides: date mode splits one layer across two frames", () => {
  const sides = compareSides({ mode: "date", value: T1 }, "park-fire-ndvi", T0);
  expect(sides).toEqual({
    mode: "date",
    left: { layer: "park-fire-ndvi", requested: T0 },
    right: { layer: "park-fire-ndvi", requested: T1 },
  });
  expect(sideLabels(sides)).toEqual({ left: T0, right: T1 });
  // No viewed frame = latest on the left, honestly labeled.
  const latest = compareSides({ mode: "date", value: T1 }, "park-fire-ndvi", null);
  expect(latest.left.requested).toBeNull();
  expect(sideLabels(latest).left).toBe("latest");
});

test("compareSides: layer mode splits two layers at one frame", () => {
  const sides = compareSides({ mode: "layer", value: "truecolor" }, "ndvi", T0);
  expect(sides).toEqual({
    mode: "layer",
    left: { layer: "ndvi", requested: T0 },
    right: { layer: "truecolor", requested: T0 },
  });
  expect(sideLabels(sides)).toEqual({ left: "ndvi", right: "truecolor" });
});

test("traceSide, date mode: the raw datetime= request is the side identity", () => {
  const sides = compareSides({ mode: "date", value: T1 }, "fire", T0);
  expect(traceSide(sides, "fire", T0)).toBe("left");
  expect(traceSide(sides, "fire", T1)).toBe("right");
  // Another layer's background render belongs to neither side.
  expect(traceSide(sides, "other", T0)).toBeUndefined();
  // A frame no side is showing (an old scrub still in flight) is dropped.
  expect(traceSide(sides, "fire", "2024-07-22T19:03:00Z")).toBeUndefined();
  expect(traceSide(sides, "fire", null)).toBeUndefined();
  // Left at "latest": a null/absent requested matches the left side.
  const latest = compareSides({ mode: "date", value: T1 }, "fire", null);
  expect(traceSide(latest, "fire", null)).toBe("left");
  expect(traceSide(latest, "fire", undefined)).toBe("left");
  expect(traceSide(latest, "fire", T1)).toBe("right");
});

test("traceSide, layer mode: the envelope's layer is the side identity", () => {
  const sides = compareSides({ mode: "layer", value: "truecolor" }, "ndvi", null);
  expect(traceSide(sides, "ndvi", null)).toBe("left");
  expect(traceSide(sides, "truecolor", null)).toBe("right");
  expect(traceSide(sides, "park-fire-ndvi", null)).toBeUndefined();
});

test("traceSide: both sides asking for one frame resolves right, deterministically", () => {
  const degenerate = compareSides({ mode: "date", value: T0 }, "fire", T0);
  expect(traceSide(degenerate, "fire", T0)).toBe("right");
});

test("clampSwipe: [0,1] passes, everything else is the centered default or clamped", () => {
  expect(clampSwipe(0.25)).toBe(0.25);
  expect(clampSwipe(0)).toBe(0);
  expect(clampSwipe(1)).toBe(1);
  expect(clampSwipe(-0.5)).toBe(0);
  expect(clampSwipe(1.5)).toBe(1);
  expect(clampSwipe(undefined)).toBe(DEFAULT_SWIPE);
  expect(clampSwipe(Number.NaN)).toBe(DEFAULT_SWIPE);
});
