// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeAll, expect, test } from "vitest";
import { defineSwathTimeline, type SwathTimeline } from "./swath-timeline.js";
import { buildTimeline, TIMELINE_HINT } from "./timeline-model.js";

const counts = (rows: [string, string, number][]) => ({
  buckets: rows.map(([start, end, count]) => ({ start, end, count })),
});

const JAN: [string, string, number] = ["2024-01-01T00:00:00Z", "2024-02-01T00:00:00Z", 100];
const FEB: [string, string, number] = ["2024-02-01T00:00:00Z", "2024-03-01T00:00:00Z", 50];

beforeAll(() => {
  defineSwathTimeline();
});

afterEach(() => {
  document.body.replaceChildren();
});

async function mount(exists: unknown, survives: unknown): Promise<SwathTimeline> {
  const element = document.createElement("swath-timeline");
  document.body.append(element);
  element.timeline = buildTimeline(exists, survives);
  await element.updateComplete;
  return element;
}

const bars = (element: SwathTimeline) =>
  [...(element.shadowRoot?.querySelectorAll('[part="bucket"]') ?? [])] as HTMLElement[];

test("the control says what to do in plain words, and labels every bucket", async () => {
  const element = await mount(counts([JAN, FEB]), counts([[JAN[0], JAN[1], 40]]));
  expect(element.shadowRoot?.querySelector('[part="hint"]')?.textContent).toBe(TIMELINE_HINT);
  expect(bars(element).map((bar) => bar.getAttribute("aria-label"))).toEqual([
    "2024-01-01: 40 of 100",
    "2024-02-01: 0 of 50",
  ]);
});

test("both bands scale to the axis peak, so the gap is the filter's effect", async () => {
  const element = await mount(counts([JAN, FEB]), counts([[JAN[0], JAN[1], 50]]));
  const [january, february] = bars(element);
  expect(january?.querySelector<HTMLElement>('[part="exists"]')?.style.blockSize).toBe("100%");
  expect(january?.querySelector<HTMLElement>('[part="survives"]')?.style.blockSize).toBe("50%");
  // February holds 50 of the axis's 100 and none survive.
  expect(february?.querySelector<HTMLElement>('[part="exists"]')?.style.blockSize).toBe("50%");
  expect(february?.querySelector<HTMLElement>('[part="survives"]')?.style.blockSize).toBe("0%");
});

test("with no filters the note says so rather than drawing a confusing empty band", async () => {
  const both = counts([JAN, FEB]);
  const element = await mount(both, both);
  expect(element.shadowRoot?.querySelector('[part="note"]')?.textContent).toBe(
    "150 granules — no filters, so every one is in view.",
  );
});

test("picking a bucket emits whole-day bounds; shift extends the range", async () => {
  const element = await mount(counts([JAN, FEB]), counts([JAN, FEB]));
  const ranges: { from: string | null; to: string | null }[] = [];
  element.addEventListener("swath-dates", (event) => ranges.push(event.detail));

  bars(element)[0]?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  await element.updateComplete;
  expect(ranges).toEqual([{ from: "2024-01-01", to: "2024-01-31" }]);
  expect(bars(element)[0]?.getAttribute("aria-pressed")).toBe("true");
  expect(bars(element)[1]?.getAttribute("aria-pressed")).toBe("false");

  bars(element)[1]?.dispatchEvent(
    new PointerEvent("pointerdown", { bubbles: true, shiftKey: true }),
  );
  await element.updateComplete;
  expect(ranges[1]).toEqual({ from: "2024-01-01", to: "2024-02-29" });
  expect(bars(element).map((bar) => bar.getAttribute("aria-pressed"))).toEqual(["true", "true"]);
});

test("the axis is keyboard-reachable: Enter picks, shift-Enter extends", async () => {
  const element = await mount(counts([JAN, FEB]), counts([JAN, FEB]));
  const ranges: { from: string | null }[] = [];
  element.addEventListener("swath-dates", (event) => ranges.push(event.detail));

  bars(element)[1]?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  await element.updateComplete;
  expect(ranges).toEqual([{ from: "2024-02-01", to: "2024-02-29" }]);

  bars(element)[0]?.dispatchEvent(
    new KeyboardEvent("keydown", { key: " ", bubbles: true, shiftKey: true }),
  );
  await element.updateComplete;
  expect(ranges[1]).toEqual({ from: "2024-01-01", to: "2024-02-29" });
});

test("a fresh axis clears the drag it no longer describes", async () => {
  const element = await mount(counts([JAN, FEB]), counts([JAN, FEB]));
  bars(element)[0]?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  await element.updateComplete;
  expect(bars(element)[0]?.getAttribute("aria-pressed")).toBe("true");

  element.timeline = buildTimeline(counts([FEB]), counts([FEB]));
  await element.updateComplete;
  expect(bars(element).map((bar) => bar.getAttribute("aria-pressed"))).toEqual(["false"]);
});
