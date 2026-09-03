// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The explain card's model (issue #394): three densities over one shape,
// every figure from the server, an unmeasured one an em dash, and the
// planner's reasons quoted rather than paraphrased.
import { expect, test } from "vitest";
import {
  conceptContent,
  explainBytes,
  explainMs,
  fixFor,
  measuredContent,
  NO_OVERVIEWS_REASON,
  publishedContent,
  UNMEASURED,
} from "./explain-model.js";
import type { PlanCandidate, TraceEnvelope } from "./swath-xray.js";

const CANDIDATES: PlanCandidate[] = [
  {
    strategy: "cache_hit",
    estimated_cost_bytes: 0,
    admissible: false,
    reason: "not estimated: cache hit short-circuits",
  },
  {
    strategy: { overview: { factor: 0 } },
    estimated_cost_bytes: 0,
    admissible: false,
    reason: NO_OVERVIEWS_REASON,
  },
  {
    strategy: "live",
    estimated_cost_bytes: 80_672,
    admissible: true,
    reason: "full-resolution read",
  },
];

function envelope(overrides: Partial<TraceEnvelope["trace"]> = {}): TraceEnvelope {
  return {
    tile: "12/848/1561",
    layer: "park-fire-ndvi",
    trace: {
      decision: "live",
      source: "a.tif",
      sources: ["a.tif"],
      crs_from: 32610,
      crs_to: 3857,
      bytes_read: 80_672,
      provenance: [],
      timings: { read_ms: 412, warp_ms: 88, pixel_ops_ms: 31, encode_ms: 14, total_ms: 545 },
      ingest_to_pixel_ms: 4737,
      plan: { chosen: "live", considered: CANDIDATES },
      ...overrides,
    },
  };
}

test("unmeasured is an em dash — never zero, never a spinner", () => {
  expect(explainBytes(null)).toBe(UNMEASURED);
  expect(explainBytes(undefined)).toBe(UNMEASURED);
  expect(explainMs(null)).toBe(UNMEASURED);
  expect(explainMs(Number.NaN)).toBe(UNMEASURED);
  // A measured zero is a measurement and says so.
  expect(explainBytes(0)).toBe("0 B");
  expect(explainMs(0)).toBe("0 ms");
});

test("measured: every row is a figure the trace carried", () => {
  const content = measuredContent(envelope());
  expect(content.density).toBe("measured");
  const rows = Object.fromEntries(content.rows.map((r) => [r.label, r.value]));
  expect(rows["decision"]).toBe("live");
  expect(rows["tile"]).toBe("12/848/1561");
  expect(rows["bytes read"]).toBe("78.8 KB");
  expect(rows["total"]).toBe("545 ms");
  expect(rows["ingest→pixel"]).toBe("4737 ms");
});

test("measured: a trace with no timings renders dashes, not zeroes", () => {
  const bare = envelope();
  const stripped = {
    ...bare,
    trace: { ...bare.trace, ingest_to_pixel_ms: null, bytes_read: Number.NaN },
  } as TraceEnvelope;
  const rows = Object.fromEntries(measuredContent(stripped).rows.map((r) => [r.label, r.value]));
  expect(rows["ingest→pixel"]).toBe(UNMEASURED);
  expect(rows["bytes read"]).toBe(UNMEASURED);
});

test("measured: the planner's candidates are quoted, in its own order", () => {
  const { candidates } = measuredContent(envelope());
  expect(candidates.map((c) => c.strategy)).toEqual(["cache_hit", "overview (factor 0)", "live"]);
  // Verbatim: the planner's words ARE the explanation.
  expect(candidates[2]?.reason).toBe("full-resolution read");
  expect(candidates[1]?.admissible).toBe(false);
});

test("the fix is offered only for the rejection an operator can act on", () => {
  expect(fixFor(CANDIDATES)).toContain("swath materialize");
  // The other two overview rejections are not materialize problems.
  const eligible: PlanCandidate[] = [
    { ...CANDIDATES[1], reason: "no overview factor eligible at this zoom" } as PlanCandidate,
  ];
  expect(fixFor(eligible)).toBeUndefined();
  const noWindow: PlanCandidate[] = [
    { ...CANDIDATES[1], reason: "no source window" } as PlanCandidate,
  ];
  expect(fixFor(noWindow)).toBeUndefined();
  // An admissible overview is not a problem at all.
  const fine: PlanCandidate[] = [
    { ...CANDIDATES[1], admissible: true, reason: NO_OVERVIEWS_REASON } as PlanCandidate,
  ];
  expect(fixFor(fine)).toBeUndefined();
  expect(fixFor([])).toBeUndefined();
});

test("concept: a definition and nothing else; an unknown term has no card", () => {
  const content = conceptContent("granule");
  expect(content?.density).toBe("concept");
  expect(content?.definition).toContain("acquisition");
  expect(content?.rows).toEqual([]);
  expect(content?.candidates).toEqual([]);
  expect(conceptContent("satellite")).toBeUndefined();
});

test("published: the receipt's fields lead, the measurements follow", () => {
  const content = publishedContent(envelope(), {
    ogc: "http://localhost:8080/tilesets/x",
    xyz: "http://localhost:8080/tiles/x/{z}/{x}/{y}.png",
    window: "2024-06-01/2024-09-01",
    sources: ["a.tif", "b.tif"],
  });
  expect(content.density).toBe("published");
  expect(content.rows[0]?.label).toBe("ogc");
  expect(content.rows.map((r) => r.label)).toContain("bytes read");
  const rows = Object.fromEntries(content.rows.map((r) => [r.label, r.value]));
  expect(rows["granules"]).toBe("a.tif, b.tif");
  // Nothing is invented for a field the server did not serve.
  const empty = publishedContent(envelope(), {});
  const emptyRows = Object.fromEntries(empty.rows.map((r) => [r.label, r.value]));
  expect(emptyRows["ogc"]).toBe(UNMEASURED);
  expect(emptyRows["granules"]).toBe(UNMEASURED);
});
