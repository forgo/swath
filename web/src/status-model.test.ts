// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import { formatCrs, formatIngest, formatLonLat, formatZoomCell } from "./status-model.js";

test("formatLonLat: 4 decimals, trailing zeros trimmed, longitude wrapped", () => {
  expect(formatLonLat(-121.69313, 40.02077)).toBe("-121.6931, 40.0208");
  expect(formatLonLat(10, 20)).toBe("10, 20");
  expect(formatLonLat(190, 0)).toBe("-170, 0");
  expect(formatLonLat(-180, -90)).toBe("-180, -90");
  expect(formatLonLat(360, 0)).toBe("0, 0");
});

test("formatZoomCell mirrors view-state's zoom precision", () => {
  expect(formatZoomCell(12.6349)).toBe("12.63");
  expect(formatZoomCell(13)).toBe("13");
});

test("formatIngest: — until the first trace, then whole milliseconds", () => {
  expect(formatIngest(undefined)).toBe("—");
  expect(formatIngest(14882.4)).toBe("14882 ms");
  expect(formatIngest(0)).toBe("0 ms");
});

test("the CRS cell names the tiling scheme and its EPSG code", () => {
  expect(formatCrs()).toBe("WebMercatorQuad · EPSG:3857");
});
