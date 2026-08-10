// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The morecantile oracle for the frontend's TMS math (issue #106) — the TS
// twin of crates/swath-core/tests/tms_truth.rs. tms_truth.json is generated
// ONCE by tests/oracle/tms_truth.py's sibling tests/oracle/tms_truth_web.py
// (`just tms-truth-web`) and committed; this suite asserts against the
// committed table, so CI fails on any drift between tms.ts and the pinned
// truth. Clamp behavior outside the truth table's domain (poleward
// latitudes, out-of-range and fractional zooms) is pinned by unit tests
// below.
import { expect, test } from "vitest";
import { centerTile, tileNorthWest } from "./tms.js";
import truth from "./tms_truth.json";

/** Corner tolerance in degrees (~0.1 mm) — the twin of the Rust test's
 * 1e-6 m: morecantile inverts Web Mercator through pyproj while tms.ts uses
 * the closed atan/sinh form, so agreement is ~1e-13°, not bit-exact. */
const CORNER_EPSILON_DEG = 1e-9;

test("truth table provenance and size are as committed", () => {
  expect(truth.tms).toBe("WebMercatorQuad");
  expect(truth.morecantile).toBe("6.2.0");
  expect(truth.generator).toBe("tests/oracle/tms_truth_web.py");
  expect(truth.center_tiles.length).toBeGreaterThanOrEqual(50);
});

test("centerTile matches the committed morecantile truth exactly", () => {
  for (const c of truth.center_tiles) {
    expect(
      centerTile(c.lon, c.lat, c.zoom),
      `centerTile(${c.lon}, ${c.lat}, ${c.zoom})`,
    ).toStrictEqual({ z: c.z, x: c.x, y: c.y });
  }
});

test("tileNorthWest matches the committed morecantile truth", () => {
  for (const c of truth.northwest_corners) {
    const [lon, lat] = tileNorthWest(c.z, c.x, c.y);
    expect(Math.abs(lon - c.lon), `lon of ${c.z}/${c.x}/${c.y}`).toBeLessThanOrEqual(
      CORNER_EPSILON_DEG,
    );
    expect(Math.abs(lat - c.lat), `lat of ${c.z}/${c.x}/${c.y}`).toBeLessThanOrEqual(
      CORNER_EPSILON_DEG,
    );
  }
});

test("centerTile clamps zoom to [0, 22] and rounds fractional zooms", () => {
  expect(centerTile(0, 0, -3)).toStrictEqual(centerTile(0, 0, 0));
  expect(centerTile(0, 0, 30)).toStrictEqual(centerTile(0, 0, 22));
  expect(centerTile(2.3522, 48.8566, 11.6)).toStrictEqual(centerTile(2.3522, 48.8566, 12));
  expect(centerTile(2.3522, 48.8566, 11.4)).toStrictEqual(centerTile(2.3522, 48.8566, 11));
});

test("centerTile clamps poleward latitudes and out-of-range longitudes", () => {
  expect(centerTile(0, 90, 5)).toStrictEqual(centerTile(0, 85.0511, 5));
  expect(centerTile(0, -90, 5)).toStrictEqual(centerTile(0, -85.0511, 5));
  expect(centerTile(200, 0, 5)).toStrictEqual(centerTile(180, 0, 5));
  expect(centerTile(-200, 0, 5)).toStrictEqual(centerTile(-180, 0, 5));
});
