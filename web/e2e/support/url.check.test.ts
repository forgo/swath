// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The builder names the same states the literals did (issue #399). Compared
// by PARSING both, not by string equality: the app writes its params in its
// own order, and the point of the migration is that the tests stop asserting
// a hand-typed spelling.
import { expect, test } from "vitest";
import { parseAppState } from "../../src/app-state.js";
import { parseViewState } from "../../src/view-state.js";
import { demoSearch } from "./url.js";

test("the builder and the literal it replaces name the same state", () => {
  const cases: [string, Parameters<typeof demoSearch>[0]][] = [
    ["?view=data", { view: "data" }],
    ["?layer=truecolor", { layer: "truecolor" }],
    [
      "?layer=ndvi&center=-121.6931,40.0208&zoom=12",
      {
        layer: "ndvi",
        center: [-121.6931, 40.0208],
        zoom: 12,
      },
    ],
    [
      "?xray&view=xray&layer=truecolor&center=-121.6931,40.0208&zoom=13",
      {
        xray: true,
        view: "xray",
        layer: "truecolor",
        center: [-121.6931, 40.0208],
        zoom: 13,
      },
    ],
    ["?layer=park-fire-ndvi&xray", { layer: "park-fire-ndvi", xray: true }],
  ];
  for (const [literal, link] of cases) {
    const built = demoSearch(link);
    expect(parseViewState(built), literal).toEqual(parseViewState(literal));
    expect(parseAppState(built), literal).toEqual(parseAppState(literal));
  }
});

test("the default landing is bare — no params written for defaults", () => {
  expect(demoSearch()).toBe("");
  // `layers` is the default mode and is never written, so a link that only
  // names it is still bare.
  expect(demoSearch({ view: "layers" })).toBe("");
});

test("passthrough params survive, and the app's own params are not duplicated", () => {
  const url = demoSearch({ layer: "ndvi", passthrough: { basemap: "none" } });
  expect(url).toContain("basemap=none");
  expect(url.match(/layer=/g)).toHaveLength(1);
});
