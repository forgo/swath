// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
  bandNote,
  barHeight,
  buildTimeline,
  dayBefore,
  dayOf,
  EMPTY_TIMELINE,
  rangeOf,
} from "./timeline-model.js";

const counts = (rows: [string, string, number][]) => ({
  buckets: rows.map(([start, end, count]) => ({ start, end, count })),
});

const JAN: [string, string, number] = ["2024-01-01T00:00:00Z", "2024-02-01T00:00:00Z", 124];
const FEB: [string, string, number] = ["2024-02-01T00:00:00Z", "2024-03-01T00:00:00Z", 116];

describe("buildTimeline", () => {
  it("stacks the surviving band inside the held one", () => {
    const timeline = buildTimeline(
      counts([JAN, FEB]),
      counts([["2024-01-01T00:00:00Z", "2024-02-01T00:00:00Z", 30]]),
    );
    expect(timeline.buckets).toEqual([
      { start: JAN[0], end: JAN[1], exists: 124, survives: 30 },
      { start: FEB[0], end: FEB[1], exists: 116, survives: 0 },
    ]);
    expect(timeline.peak).toBe(124);
    expect(timeline.totalExists).toBe(240);
    expect(timeline.totalSurvives).toBe(30);
    expect(timeline.unfiltered).toBe(false);
  });

  it("says the filters remove nothing when the bands agree", () => {
    const both = counts([JAN, FEB]);
    const timeline = buildTimeline(both, both);
    expect(timeline.unfiltered).toBe(true);
    expect(bandNote(timeline)).toBe("240 granules — no filters, so every one is in view.");
  });

  it("never draws a surviving band taller than the axis it sits in", () => {
    // A scope cannot add granules; if the two answers disagree, the axis
    // grows rather than the band overflowing it.
    const timeline = buildTimeline(counts([JAN]), counts([[JAN[0], JAN[1], 999]]));
    expect(timeline.buckets[0]).toEqual({
      start: JAN[0],
      end: JAN[1],
      exists: 999,
      survives: 999,
    });
    expect(timeline.peak).toBe(999);
  });

  it("ignores cell buckets and malformed rows rather than drawing them", () => {
    const timeline = buildTimeline(
      { buckets: [{ bbox: [0, 0, 1, 1], count: 5 }, "nope", { start: 3, end: 4, count: 1 }] },
      {},
    );
    expect(timeline).toEqual(EMPTY_TIMELINE);
    expect(bandNote(timeline)).toBe("No granules on this timeline yet.");
  });

  it("keeps the axis in ascending order whichever order the server used", () => {
    const timeline = buildTimeline(counts([FEB, JAN]), counts([FEB, JAN]));
    expect(timeline.buckets.map((b) => b.start)).toEqual([JAN[0], FEB[0]]);
  });
});

describe("barHeight", () => {
  it("scales against the axis peak, and an empty axis is flat", () => {
    expect(barHeight(62, 124)).toBe(0.5);
    expect(barHeight(124, 124)).toBe(1);
    expect(barHeight(5, 0)).toBe(0);
  });
});

describe("rangeOf", () => {
  const timeline = buildTimeline(counts([JAN, FEB]), counts([JAN, FEB]));

  it("reads a drag as whole days, inclusive on both ends", () => {
    expect(rangeOf(timeline, 0, 0)).toEqual({ from: "2024-01-01", to: "2024-01-31" });
    expect(rangeOf(timeline, 0, 1)).toEqual({ from: "2024-01-01", to: "2024-02-29" });
  });

  it("does not care which way the drag went", () => {
    expect(rangeOf(timeline, 1, 0)).toEqual(rangeOf(timeline, 0, 1));
  });

  it("clamps to the axis and is nothing when there is no axis", () => {
    expect(rangeOf(timeline, -5, 99)).toEqual({ from: "2024-01-01", to: "2024-02-29" });
    expect(rangeOf(EMPTY_TIMELINE, 0, 0)).toBeUndefined();
  });
});

describe("day helpers", () => {
  it("takes the date of an instant, and the day before an exclusive end", () => {
    expect(dayOf("2024-06-06T17:54:00Z")).toBe("2024-06-06");
    expect(dayBefore("2024-03-01T00:00:00Z")).toBe("2024-02-29");
    expect(dayBefore("not a date")).toBe("not a date");
  });
});

describe("bandNote", () => {
  it("counts only what the server gave", () => {
    const timeline = buildTimeline(counts([JAN]), counts([[JAN[0], JAN[1], 1]]));
    expect(bandNote(timeline)).toBe("1 of 124 granules survive the filters.");
  });
});
