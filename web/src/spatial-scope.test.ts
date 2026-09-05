// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import { parseSpatialInput, REDUCED_NOTE, scopeTag } from "./spatial-scope.js";

test("four numbers are a box, used exactly as given", () => {
  expect(parseSpatialInput("-106, 39, -105, 40")).toEqual({
    ok: true,
    bbox: [-106, 39, -105, 40],
    reduced: false,
  });
  // Whitespace-separated reads the same; a pasted box comes in both forms.
  expect(parseSpatialInput("-106 39 -105 40")).toEqual({
    ok: true,
    bbox: [-106, 39, -105, 40],
    reduced: false,
  });
});

test("a pasted polygon is reduced to its envelope, and the reduction is reported", () => {
  const polygon = {
    type: "Polygon",
    coordinates: [
      [
        [-106, 39],
        [-105, 39.5],
        [-105.5, 40],
        [-106, 39],
      ],
    ],
  };
  const parsed = parseSpatialInput(JSON.stringify(polygon));
  expect(parsed).toEqual({ ok: true, bbox: [-106, 39, -105, 40], reduced: true });
  // The note is the whole point: the reduction must be readable, not just
  // true.
  expect(REDUCED_NOTE).toBe("Using the bounding box of the shape you pasted.");
});

test("a Feature, a FeatureCollection and a GeometryCollection all reduce", () => {
  const ring = [
    [0, 0],
    [2, 3],
    [0, 0],
  ];
  const geometry = { type: "Polygon", coordinates: [ring] };
  for (const doc of [
    { type: "Feature", geometry, properties: {} },
    { type: "FeatureCollection", features: [{ type: "Feature", geometry, properties: {} }] },
    { type: "GeometryCollection", geometries: [geometry] },
  ]) {
    expect(parseSpatialInput(JSON.stringify(doc))).toEqual({
      ok: true,
      bbox: [0, 0, 2, 3],
      reduced: true,
    });
  }
});

test("elevation in a position does not become a coordinate", () => {
  const line = {
    type: "LineString",
    coordinates: [
      [1, 2, 900],
      [3, 4, 1200],
    ],
  };
  expect(parseSpatialInput(JSON.stringify(line))).toEqual({
    ok: true,
    bbox: [1, 2, 3, 4],
    reduced: true,
  });
});

test("what it cannot do, it says — never a filter that quietly does nothing", () => {
  expect(parseSpatialInput("")).toEqual({ ok: false, reason: "Nothing to search in yet." });
  expect(parseSpatialInput("denver")).toMatchObject({ ok: false });
  expect(parseSpatialInput('{"type":"Point"}')).toMatchObject({ ok: false });
  expect(parseSpatialInput("-106, 40, -105, 39")).toMatchObject({ ok: false });
  expect((parseSpatialInput("-106, 40, -105, 39") as { reason: string }).reason).toContain(
    "west, south, east, north",
  );
});

test("the tag names the mode in words, and there is no shape mode to name", () => {
  expect(scopeTag("viewport")).toBe("viewport");
  expect(scopeTag("bbox")).toBe("bbox");
});
