// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import {
  edgePath,
  fitViewport,
  hitEdge,
  intersects,
  portAnchor,
  rectFrom,
  toCanvas,
  toScreen,
  union,
  zoomAround,
} from "./canvas-geometry.js";

test("toCanvas/toScreen invert each other under pan and zoom", () => {
  const view = { x: 40, y: -10, k: 2 };
  const p = { x: 123, y: 456 };
  const c = toCanvas(view, p);
  expect(toScreen(view, c)).toEqual(p);
  expect(toCanvas({ x: 0, y: 0, k: 1 }, p)).toEqual(p);
});

test("zoomAround keeps the anchor point fixed on screen and clamps", () => {
  const view = { x: 0, y: 0, k: 1 };
  const around = { x: 200, y: 100 };
  const zoomed = zoomAround(view, 2, around);
  expect(zoomed.k).toBe(2);
  expect(toScreen(zoomed, toCanvas(view, around))).toEqual(around);
  expect(zoomAround(view, 100, around).k).toBe(4);
  expect(zoomAround(view, 0.001, around).k).toBe(0.25);
});

test("fitViewport centres the bounds with padding, never above zoom 1", () => {
  const size = { width: 800, height: 600 };
  const view = fitViewport({ x: 100, y: 100, width: 400, height: 200 }, size, 24);
  expect(view.k).toBe(1);
  expect(toScreen(view, { x: 300, y: 200 })).toEqual({ x: 400, y: 300 }); // centre of bounds at centre of screen
  const wide = fitViewport({ x: 0, y: 0, width: 4000, height: 200 }, size, 0);
  expect(wide.k).toBeCloseTo(0.25);
  expect(union([])).toBeUndefined();
  expect(
    union([
      { x: 0, y: 0, width: 10, height: 10 },
      { x: 5, y: 5, width: 10, height: 10 },
    ]),
  ).toEqual({
    x: 0,
    y: 0,
    width: 15,
    height: 15,
  });
});

test("portAnchor spaces ports down the node's edge; edges are cubics; hit tests find them", () => {
  const node = { x: 0, y: 0, width: 100, height: 60 };
  expect(portAnchor(node, "input", 0, 1)).toEqual({ x: 0, y: 30 });
  expect(portAnchor(node, "output", 1, 2)).toEqual({ x: 100, y: 40 });
  const path = edgePath({ x: 0, y: 0 }, { x: 100, y: 50 });
  expect(path.startsWith("M 0 0 C 50 0, 50 50, 100 50")).toBe(true);
  expect(hitEdge({ x: 50, y: 25 }, { x: 0, y: 0 }, { x: 100, y: 50 }, 6)).toBe(true);
  expect(hitEdge({ x: 50, y: 60 }, { x: 0, y: 0 }, { x: 100, y: 50 }, 6)).toBe(false);
});

test("rectFrom normalises; intersects is inclusive at touching edges", () => {
  expect(rectFrom({ x: 10, y: 10 }, { x: 0, y: 5 })).toEqual({ x: 0, y: 5, width: 10, height: 5 });
  expect(
    intersects({ x: 0, y: 0, width: 10, height: 10 }, { x: 10, y: 10, width: 5, height: 5 }),
  ).toBe(true);
  expect(
    intersects({ x: 0, y: 0, width: 10, height: 10 }, { x: 11, y: 0, width: 5, height: 5 }),
  ).toBe(false);
});
