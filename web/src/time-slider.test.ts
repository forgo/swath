// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The time slider's semantics (issue #182), pinned without a map: the
// domain parse (granules API → ascending frames), frame resolution (the
// client-side mirror of the server's latest-at-or-before rule, ADR
// 0015), the hide-below-two-frames contract that keeps the zero-config
// landing untouched, and the play loop's advance + next-frame prefetch
// (driven through fake timers). The against-the-real-stack proof is
// web/e2e/time-slider.e2e.ts.
import { afterEach, expect, test, vi } from "vitest";
import {
  boundDomain,
  frameIndexForTime,
  PLAY_INTERVAL_MS,
  parseGranuleDatetimes,
  TimeSlider,
  type TimeSliderHooks,
} from "./time-slider.js";

/** The Park Fire fixture series' acquisition datetimes (ascending). */
const FRAMES = [
  "2024-06-07T19:03:00Z",
  "2024-07-22T19:03:00Z",
  "2024-08-16T19:03:00Z",
  "2024-09-05T19:03:00Z",
];

function recordingHooks(): TimeSliderHooks & { scrubbed: string[]; prefetched: string[] } {
  const scrubbed: string[] = [];
  const prefetched: string[] = [];
  return {
    scrubbed,
    prefetched,
    scrubTo: (datetime) => scrubbed.push(datetime),
    prefetch: (datetime) => prefetched.push(datetime),
  };
}

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
});

test("parseGranuleDatetimes: newest-first API order becomes an ascending domain", () => {
  // The granules API lists newest first; the slider scrubs oldest→newest.
  const body = {
    granules: [...FRAMES]
      .reverse()
      .map((datetime, i) => ({ id: `g${i}`, datetime, bbox: [0, 0, 1, 1] })),
  };
  expect(parseGranuleDatetimes(body)).toEqual(FRAMES);
});

test("parseGranuleDatetimes: duplicates collapse, junk is skipped, never fatal", () => {
  expect(
    parseGranuleDatetimes({
      granules: [
        { datetime: FRAMES[1] },
        { datetime: FRAMES[1] }, // one instant = one frame
        { datetime: "not a date" },
        { datetime: 7 },
        {},
        { datetime: FRAMES[0] },
      ],
    }),
  ).toEqual([FRAMES[0], FRAMES[1]]);
  expect(parseGranuleDatetimes({})).toEqual([]);
  expect(parseGranuleDatetimes(null)).toEqual([]);
  expect(parseGranuleDatetimes("junk")).toEqual([]);
});

test("boundDomain keeps only the frames inside the layer's window (ADR 0015/0022)", () => {
  const frames = [
    "2024-06-07T19:03:00Z",
    "2024-07-22T19:03:00Z",
    "2024-08-16T19:03:00Z",
    "2024-10-15T19:03:00Z",
  ];
  expect(boundDomain(frames, undefined)).toEqual(frames);
  expect(boundDomain(frames, [null, null])).toEqual(frames);
  expect(boundDomain(frames, ["2024-07-01T00:00:00Z", "2024-08-31T23:59:59.999Z"])).toEqual([
    "2024-07-22T19:03:00Z",
    "2024-08-16T19:03:00Z",
  ]);
  expect(boundDomain(frames, ["2024-08-16T19:03:00Z", null])).toEqual([
    "2024-08-16T19:03:00Z",
    "2024-10-15T19:03:00Z",
  ]);
  expect(boundDomain(frames, [null, "2024-06-01T00:00:00Z"])).toEqual([]);
});

test("frameIndexForTime mirrors the server's latest-at-or-before rule", () => {
  // Exactly at an acquisition: that frame.
  expect(frameIndexForTime(FRAMES, "2024-07-22T19:03:00Z")).toBe(1);
  // Between acquisitions: the latest at-or-before.
  expect(frameIndexForTime(FRAMES, "2024-08-01T00:00:00Z")).toBe(1);
  // After the last: the last.
  expect(frameIndexForTime(FRAMES, "2030-01-01T00:00:00Z")).toBe(3);
  // Before the first (the server's honest-404 window): rests at 0.
  expect(frameIndexForTime(FRAMES, "2020-01-01T00:00:00Z")).toBe(0);
  // Absent or malformed: latest.
  expect(frameIndexForTime(FRAMES, null)).toBe(3);
  expect(frameIndexForTime(FRAMES, "yesterday")).toBe(3);
});

test("hidden below two frames — the zero-config landing page is untouched", () => {
  const slider = new TimeSlider(document, recordingHooks());
  document.body.append(slider.element);
  expect(slider.element.hidden).toBe(true);
  // One granule (the stopwatch-demo fixture layer): still nothing to scrub.
  slider.setDomain([FRAMES[0] ?? ""], null);
  expect(slider.element.hidden).toBe(true);
  // Two or more: visible, with the exact state on data attributes.
  slider.setDomain(FRAMES, null);
  expect(slider.element.hidden).toBe(false);
  expect(slider.element.dataset["frames"]).toBe("4");
  expect(slider.element.dataset["index"]).toBe("3"); // no datetime = latest
  expect(slider.element.dataset["datetime"]).toBe(FRAMES[3]);
  // Switching to a single-date layer hides it again.
  slider.setDomain([FRAMES[0] ?? ""], null);
  expect(slider.element.hidden).toBe(true);
});

test("scrubbing the range input reports the chosen frame, once", () => {
  const hooks = recordingHooks();
  const slider = new TimeSlider(document, hooks);
  document.body.append(slider.element);
  slider.setDomain(FRAMES, "2024-08-16T19:03:00Z");
  expect(slider.element.dataset["index"]).toBe("2");

  const range = slider.element.querySelector<HTMLInputElement>('input[type="range"]');
  if (!range) {
    throw new Error("slider has no range input");
  }
  range.value = "0";
  range.dispatchEvent(new Event("input"));
  expect(hooks.scrubbed).toEqual([FRAMES[0]]);
  expect(slider.element.dataset["datetime"]).toBe(FRAMES[0]);
  // Re-dispatch at the same value: no duplicate scrub.
  range.dispatchEvent(new Event("input"));
  expect(hooks.scrubbed).toEqual([FRAMES[0]]);
  // setActive mirrors the host attribute without scrubbing back.
  slider.setActive("2024-09-05T19:03:00Z");
  expect(slider.element.dataset["index"]).toBe("3");
  expect(hooks.scrubbed).toEqual([FRAMES[0]]);
});

test("play advances a frame per tick (wrapping) and prefetches the frame after next", () => {
  vi.useFakeTimers();
  const hooks = recordingHooks();
  const slider = new TimeSlider(document, hooks);
  document.body.append(slider.element);
  slider.setDomain(FRAMES, FRAMES[0] ?? null); // thumb at frame 0

  const play = slider.element.querySelector<HTMLButtonElement>(".swath-map-time-play");
  play?.click();
  expect(slider.playing).toBe(true);
  expect(play?.getAttribute("aria-pressed")).toBe("true");
  // Pressing play immediately warms the frame the first tick will show.
  expect(hooks.prefetched).toEqual([FRAMES[1]]);

  vi.advanceTimersByTime(PLAY_INTERVAL_MS);
  expect(hooks.scrubbed).toEqual([FRAMES[1]]);
  expect(hooks.prefetched).toEqual([FRAMES[1], FRAMES[2]]);

  // Full wrap: 1 → 2 → 3 → 0.
  vi.advanceTimersByTime(3 * PLAY_INTERVAL_MS);
  expect(hooks.scrubbed).toEqual([FRAMES[1], FRAMES[2], FRAMES[3], FRAMES[0]]);
  expect(hooks.prefetched.at(-1)).toBe(FRAMES[1]);

  play?.click(); // pause
  expect(slider.playing).toBe(false);
  expect(play?.getAttribute("aria-pressed")).toBe("false");
  vi.advanceTimersByTime(5 * PLAY_INTERVAL_MS);
  expect(hooks.scrubbed).toHaveLength(4); // no ticks while paused

  slider.dispose();
  expect(slider.element.isConnected).toBe(false);
});

test("a vetoed tick holds the frame; user acts announce themselves first (issue #211)", () => {
  vi.useFakeTimers();
  const acts: string[] = [];
  let painted = false;
  const hooks: TimeSliderHooks & { scrubbed: string[] } = {
    ...recordingHooks(),
    canAdvance: () => painted,
    interact: () => acts.push("interact"),
  };
  const scrubbed = hooks.scrubbed;
  hooks.scrubTo = (datetime) => {
    scrubbed.push(datetime);
    acts.push(`scrub:${datetime}`);
  };
  const slider = new TimeSlider(document, hooks);
  document.body.append(slider.element);
  slider.setDomain(FRAMES, FRAMES[0] ?? null);

  // Programmatic play (the cinematic landing's path): no user act.
  slider.play();
  expect(acts).toEqual([]);
  // The frame is still painting: ticks pass without advancing...
  vi.advanceTimersByTime(3 * PLAY_INTERVAL_MS);
  expect(scrubbed).toEqual([]);
  expect(slider.element.dataset["index"]).toBe("0");
  // ...and the first tick after it lands advances exactly one frame.
  painted = true;
  vi.advanceTimersByTime(PLAY_INTERVAL_MS);
  expect(scrubbed).toEqual([FRAMES[1]]);

  // A scrub: `interact` BEFORE the scrub, so the host can attribute
  // the frame change to the user; the play button likewise.
  const range = slider.element.querySelector<HTMLInputElement>('input[type="range"]');
  if (!range) {
    throw new Error("no range input");
  }
  range.value = "3";
  range.dispatchEvent(new Event("input"));
  expect(acts).toEqual([`scrub:${FRAMES[1]}`, "interact", `scrub:${FRAMES[3]}`]);
  slider.element.querySelector<HTMLButtonElement>(".swath-map-time-play")?.click();
  expect(acts.at(-1)).toBe("interact");
  expect(slider.playing).toBe(false); // the click after interact still pauses
  slider.dispose();
});
