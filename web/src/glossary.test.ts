// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The glossary (issue #396): one definition per term, in the explaining
// register. The citation half — that each entry's source document exists and
// still uses the term — is checked by `swath-docs-check`, which can read the
// repo; this pins the shape and the register.
import { expect, test } from "vitest";
import { define, GLOSSARY, glossaryTerms } from "./glossary.js";

test("lookup is case- and space-insensitive, and unknown terms are undefined", () => {
  expect(define("granule")?.term).toBe("granule");
  expect(define("  Granule ")?.term).toBe("granule");
  expect(define("GRANULE")?.term).toBe("granule");
  // Not every word is a term. The affordance must not appear on prose.
  expect(define("satellite")).toBeUndefined();
  expect(define("")).toBeUndefined();
});

test("every definition is prose, not a spec fragment", () => {
  for (const entry of GLOSSARY) {
    // design-language.md §1: the explaining register is complete sentences.
    expect(entry.definition.endsWith("."), `${entry.term} must end its sentence`).toBe(true);
    expect(entry.definition[0], `${entry.term} starts sentence-cased`).toBe(
      entry.definition[0]?.toUpperCase(),
    );
    // §3: a definition carries no figures — those belong to `measured`.
    expect(entry.definition, `${entry.term} must not quote a number`).not.toMatch(/\d/);
    // Long enough to explain, short enough to read in a card.
    expect(entry.definition.length, `${entry.term} is too terse`).toBeGreaterThan(40);
    expect(entry.definition.length, `${entry.term} is too long for a card`).toBeLessThan(320);
  }
});

test("terms are lowercase, unique, and each cites a document", () => {
  const terms = glossaryTerms();
  expect(new Set(terms).size).toBe(terms.length);
  for (const entry of GLOSSARY) {
    expect(entry.term).toBe(entry.term.toLowerCase());
    expect(entry.source).toMatch(/^docs\/.+\.md$/);
  }
});

test("no definition leans on a term the glossary does not also offer", () => {
  // The register's rule: no jargon the reader has not already been given.
  // `overview` may say "materializing" precisely because `materialize` is
  // also defined here. Inflections are mapped rather than stemmed — a
  // stemmer turns "materializing" into "materializ" and passes nothing.
  const INFLECTIONS: Record<string, string> = {
    granule: "granule",
    granules: "granule",
    tile: "tile",
    tiles: "tile",
    cube: "cube",
    cubes: "cube",
    overview: "overview",
    overviews: "overview",
    materialize: "materialize",
    materializing: "materialize",
    decision: "decision",
    refusal: "refusal",
    window: "window",
  };
  const offered = new Set(glossaryTerms());
  for (const entry of GLOSSARY) {
    const words = entry.definition.toLowerCase().match(/[a-z]+/g) ?? [];
    for (const word of words) {
      const term = INFLECTIONS[word];
      if (term === undefined || term === entry.term) {
        continue;
      }
      expect(
        offered.has(term),
        `${entry.term}'s definition uses "${word}", so "${term}" must also be defined`,
      ).toBe(true);
    }
  }
});
