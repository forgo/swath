// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import {
  buildDensity,
  DENSITY_THRESHOLD,
  densityNote,
  EMPTY_DENSITY,
  isDense,
} from "./density-model.js";

const cells = (rows: [number[], number][]) => ({
  buckets: rows.map(([bbox, count]) => ({ bbox, count })),
});

test("cells become weights against the busiest cell", () => {
  const density = buildDensity(
    cells([
      [[0, 0, 1, 1], 10],
      [[1, 0, 2, 1], 5],
    ]),
  );
  expect(density.peak).toBe(10);
  expect(density.cells.map((cell) => cell.weight)).toEqual([1, 0.5]);
});

test("a time bucketing is not a density, and neither is a malformed row", () => {
  expect(
    buildDensity({
      buckets: [
        { start: "2024-01-01T00:00:00Z", end: "2024-02-01T00:00:00Z", count: 9 },
        { bbox: [0, 0, 1], count: 3 },
        { bbox: [0, 0, 1, "x"], count: 3 },
        "nope",
      ],
    }),
  ).toEqual(EMPTY_DENSITY);
});

test("an empty cell is not drawn — an absent count is a zero, not a place", () => {
  expect(buildDensity(cells([[[0, 0, 1, 1], 0]])).cells).toEqual([]);
  expect(buildDensity(null)).toEqual(EMPTY_DENSITY);
});

test("the threshold is where outlines stop informing, and it is a named number", () => {
  expect(DENSITY_THRESHOLD).toBe(40);
  expect(isDense(DENSITY_THRESHOLD)).toBe(false);
  expect(isDense(DENSITY_THRESHOLD + 1)).toBe(true);
});

test("the note says what the map is doing, and admits an empty surface", () => {
  const density = buildDensity(cells([[[0, 0, 1, 1], 3]]));
  expect(densityNote(density, 99)).toBe(
    "99 results — the map shows where they are, not each outline.",
  );
  expect(densityNote(EMPTY_DENSITY, 99)).toBe("99 results — the map has no density to draw yet.");
});
