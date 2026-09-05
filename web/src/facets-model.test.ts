// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { coverageNote, type Facet, facetSummary, parseFacets, UNKNOWN } from "./facets-model.js";

const facet = (over: Partial<Facet> = {}): Facet => ({
  key: "platform",
  kind: "string",
  coverage: 3,
  values: [],
  truncated: false,
  ...over,
});

describe("parseFacets", () => {
  it("reads the server's discovery, kinds and all", () => {
    const summary = parseFacets({
      total: 12,
      facets: [
        { key: "eo:cloud_cover", kind: "number", coverage: 12, min: 0, max: 80.5 },
        {
          key: "platform",
          kind: "string",
          coverage: 9,
          values: [
            { value: "sentinel-2a", count: 6 },
            { value: "sentinel-2b", count: 3 },
          ],
        },
      ],
    });
    expect(summary.total).toBe(12);
    expect(summary.facets.map((f) => f.key)).toEqual(["eo:cloud_cover", "platform"]);
    expect(summary.facets[0]).toMatchObject({ kind: "number", min: 0, max: 80.5 });
    expect(summary.facets[1]?.values).toEqual([
      { label: "sentinel-2a", count: 6 },
      { label: "sentinel-2b", count: 3 },
    ]);
  });

  it("is empty for a collection whose items carry nothing", () => {
    expect(parseFacets({ total: 4, facets: [] })).toEqual({ total: 4, facets: [] });
    expect(parseFacets(null).facets).toEqual([]);
  });

  it("drops a facet it cannot name, rather than rendering a blank row", () => {
    const summary = parseFacets({ total: 1, facets: [{ kind: "string" }, "nope", { key: "" }] });
    expect(summary.facets).toEqual([]);
  });

  it("keeps an unrecognised kind honest rather than guessing a control", () => {
    const [only] = parseFacets({
      total: 1,
      facets: [{ key: "proj:transform", kind: "matrix", coverage: 1 }],
    }).facets;
    expect(only?.kind).toBe("other");
    expect(only?.min).toBeUndefined();
  });

  it("never carries a range on a non-number facet", () => {
    const [only] = parseFacets({
      total: 1,
      facets: [{ key: "platform", kind: "string", coverage: 1, min: 3, max: 9 }],
    }).facets;
    expect(only?.min).toBeUndefined();
    expect(only?.max).toBeUndefined();
  });
});

describe("facetSummary", () => {
  it("reads a number facet as its range", () => {
    expect(facetSummary(facet({ kind: "number", min: 0, max: 80.5 }))).toBe("0 – 80.5");
    expect(facetSummary(facet({ kind: "number", min: 4, max: 4 }))).toBe("4");
  });

  it("is an em dash where the server claimed nothing", () => {
    expect(facetSummary(facet({ kind: "number" }))).toBe(UNKNOWN);
    expect(facetSummary(facet({ kind: "other" }))).toBe(UNKNOWN);
    expect(facetSummary(facet({ values: [] }))).toBe(UNKNOWN);
  });

  it("names the commonest values and admits the rest without counting them", () => {
    const values = ["a", "b", "c", "d"].map((label) => ({ label, count: 1 }));
    expect(facetSummary(facet({ values }))).toBe("a, b, c, and more");
    expect(facetSummary(facet({ values: values.slice(0, 2) }))).toBe("a, b");
    expect(facetSummary(facet({ values: values.slice(0, 2), truncated: true }))).toBe(
      "a, b, and more",
    );
  });

  it("renders booleans as words", () => {
    const [only] = parseFacets({
      total: 1,
      facets: [
        {
          key: "swath:reprocessed",
          kind: "boolean",
          coverage: 1,
          values: [{ value: true, count: 1 }],
        },
      ],
    }).facets;
    expect(only && facetSummary(only)).toBe("yes");
  });
});

describe("coverageNote", () => {
  it("distinguishes absent from zero", () => {
    expect(coverageNote(facet({ coverage: 12 }), 12)).toBe("on every granule");
    expect(coverageNote(facet({ coverage: 1 }), 12)).toBe("on 1 of 12");
  });

  it("claims nothing when there is no scope to measure against", () => {
    expect(coverageNote(facet(), 0)).toBe(UNKNOWN);
  });
});
