// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The publish receipt's model (issue #395): everything derived from the
// server's two answers, nothing inferred from the graph the client sent.
import { expect, test } from "vitest";
import { publishedContent, UNMEASURED } from "./explain-model.js";
import {
  envelopeFromHeader,
  receiptFrom,
  receiptTile,
  type TilesetDocument,
  tilesetBbox,
} from "./receipt-model.js";

const ORIGIN = "http://localhost:8080";

test("the URLs are the server's links, copied rather than assembled", () => {
  const doc: TilesetDocument = {
    links: [
      { rel: "self", href: `${ORIGIN}/tilesets/xyz-fire` },
      { rel: "item", href: `${ORIGIN}/tilesets/xyz-fire/tiles/{z}/{y}/{x}`, templated: true },
    ],
  };
  const receipt = receiptFrom(doc, "xyz-fire", ORIGIN);
  expect(receipt.ogc).toBe(`${ORIGIN}/tilesets/xyz-fire`);
  expect(receipt.xyz).toBe(`${ORIGIN}/tilesets/xyz-fire/tiles/{z}/{y}/{x}`);
});

test("a service id with URL-unsafe characters is encoded, not interpolated raw", () => {
  const receipt = receiptFrom({}, "xyz fire/1", ORIGIN);
  expect(receipt.ogc).toBe(`${ORIGIN}/tilesets/xyz%20fire%2F1`);
  expect(receipt.xyz).toContain("xyz%20fire%2F1");
});

test("the window and sources are read as the server documents them", () => {
  const listed = receiptFrom(
    { "swath:window": ["2024-06-01T00:00:00Z", "2024-09-01T00:00:00Z"] },
    "x",
    ORIGIN,
  );
  expect(listed.window).toBe("2024-06-01T00:00:00Z/2024-09-01T00:00:00Z");
  // An open end stays open rather than being filled in.
  expect(receiptFrom({ "swath:window": ["2024-06-01T00:00:00Z", null] }, "x", ORIGIN).window).toBe(
    "2024-06-01T00:00:00Z/…",
  );
  // `swath:sources` is documented as a branch COUNT, not a granule list.
  // Saying "2 sources" is true; naming granules we were not given is not.
  expect(receiptFrom({ "swath:sources": 2 }, "x", ORIGIN).sources).toEqual(["2 sources"]);
  expect(receiptFrom({ "swath:sources": 1 }, "x", ORIGIN).sources).toEqual(["1 source"]);
  // A future server that lists them is read verbatim.
  expect(receiptFrom({ "swath:sources": ["a.tif", "b.tif"] }, "x", ORIGIN).sources).toEqual([
    "a.tif",
    "b.tif",
  ]);
  // Absent stays absent — the card renders an em dash for it.
  expect(receiptFrom({}, "x", ORIGIN).window).toBeUndefined();
  expect(receiptFrom({}, "x", ORIGIN).sources).toBeUndefined();
});

test("the receipt renders the tile that frames what was published", () => {
  // A small extent gets a deep tile; a global one gets a shallow tile. A
  // fixed zoom would render an ocean tile for one and a sub-pixel one for
  // the other.
  const small = receiptTile([-121.74, 39.98, -121.64, 40.06]);
  const global = receiptTile([-180, -85, 180, 85]);
  expect(small.z).toBeGreaterThan(global.z);
  expect(global.z).toBe(0);
  // No extent at all still addresses a real tile rather than throwing.
  expect(receiptTile(undefined)).toEqual({ z: 0, x: 0, y: 0 });
  // A degenerate (zero-area) extent does not produce an infinite zoom.
  expect(Number.isFinite(receiptTile([10, 10, 10, 10]).z)).toBe(true);
});

test("tilesetBbox reads the OGC shape, and refuses anything else", () => {
  expect(tilesetBbox({ boundingBox: { lowerLeft: [1, 2], upperRight: [3, 4] } })).toEqual([
    1, 2, 3, 4,
  ]);
  expect(tilesetBbox({})).toBeUndefined();
  expect(tilesetBbox({ boundingBox: { lowerLeft: [1], upperRight: [3, 4] } })).toBeUndefined();
  expect(
    tilesetBbox({ boundingBox: { lowerLeft: ["1", "2"], upperRight: [3, 4] } }),
  ).toBeUndefined();
});

test("the trace header becomes an envelope that claims only what it carries", () => {
  const header =
    '{"decision":"live","bytes_read":546497,"total_ms":327,"ingest_to_pixel_ms":106175}';
  const envelope = envelopeFromHeader(header, { z: 12, x: 848, y: 1561 }, "xyz-fire");
  expect(envelope?.tile).toBe("12/848/1561");
  expect(envelope?.trace.bytes_read).toBe(546_497);
  expect(envelope?.trace.timings.total_ms).toBe(327);

  // The header is a SUMMARY: no plan, and no per-stage breakdown. The card
  // must show em dashes there rather than a breakdown we invented.
  const content = publishedContent(envelope as NonNullable<typeof envelope>, {});
  const rows = Object.fromEntries(content.rows.map((r) => [r.label, r.value]));
  expect(rows["total"]).toBe("327 ms");
  expect(rows["read"]).toBe(UNMEASURED);
  expect(rows["warp"]).toBe(UNMEASURED);
  expect(content.candidates).toEqual([]);
});

test("a missing or malformed trace header yields no envelope at all", () => {
  const tile = { z: 1, x: 0, y: 0 };
  expect(envelopeFromHeader(null, tile, "x")).toBeUndefined();
  expect(envelopeFromHeader("", tile, "x")).toBeUndefined();
  expect(envelopeFromHeader("not json", tile, "x")).toBeUndefined();
  // Present but empty of figures: an envelope whose numbers are all dashes,
  // which is honest — the tile rendered, we just were not told what it cost.
  const bare = envelopeFromHeader("{}", tile, "x");
  expect(bare).toBeDefined();
  const rows = Object.fromEntries(
    publishedContent(bare as NonNullable<typeof bare>, {}).rows.map((r) => [r.label, r.value]),
  );
  expect(rows["bytes read"]).toBe(UNMEASURED);
  expect(rows["ingest→pixel"]).toBe(UNMEASURED);
});
