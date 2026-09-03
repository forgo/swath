// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The glossary (issue #396) — the explaining register, in one place.
 *
 * Every term the interface offers to define, defined once. The explain card's
 * `concept` density reads from here; nothing else in the UI writes a
 * definition of its own, so a term cannot mean two things in two panels.
 *
 * Each entry names the document it comes from. `swath-docs-check`'s glossary
 * gate asserts that document exists and still contains the term, so a
 * definition cannot outlive the prose it was drawn from — the same
 * closed-both-ways discipline the budgets and route tables use.
 *
 * The register is fixed by `docs/design/design-language.md` §1: complete
 * sentences, no jargon the reader has not already been given, and no term
 * defined in terms of another term on this list without that one also being
 * offered.
 */

/** One defined term. */
export interface GlossaryEntry {
  /** The word as it appears in the interface, lowercase. */
  term: string;
  /** One or two sentences, in the explaining register. No numbers. */
  definition: string;
  /** Repo-relative document this is drawn from (the gate checks it). */
  source: string;
}

export const GLOSSARY: readonly GlossaryEntry[] = [
  {
    term: "granule",
    definition:
      "One acquisition: the imagery a satellite captured over one area at one moment. A dataset is a collection of granules, and the map shows one of them at a time.",
    source: "docs/CHARTER.md",
  },
  {
    term: "tile",
    definition:
      "One square image the map asks for, at one zoom level. A view of the map is made of many tiles, and each one is rendered on demand.",
    source: "docs/ARCHITECTURE.md",
  },
  {
    term: "decision",
    definition:
      "How a tile was produced: read from the source imagery, read from a pre-built overview, or served from the cache. Every tile records its own, and the x-ray shows them.",
    source: "docs/CHARTER.md",
  },
  {
    term: "overview",
    definition:
      "A smaller, pre-built copy of the imagery, used when the map is zoomed out far enough that full resolution would be wasted. Building them is called materializing.",
    source: "docs/PERFORMANCE.md",
  },
  {
    term: "materialize",
    definition:
      "To build the overviews for a dataset ahead of time, so zoomed-out views read a small image instead of a large one.",
    source: "docs/CONFIG.md",
  },
  {
    term: "window",
    definition:
      "The range of dates a layer is allowed to draw from. The map shows the newest granule at or before the end of the window.",
    source: "docs/ENDPOINTS.md",
  },
  {
    term: "cube",
    definition:
      "The imagery a step in a pipeline works on: some bands, over some area, at some moment. Steps take cubes and return cubes.",
    source: "docs/ARCHITECTURE.md",
  },
  {
    term: "refusal",
    definition:
      "An answer that says no, and why. When a request would cost more than the operator allows, Swath refuses in plain words instead of returning something degraded.",
    source: "docs/ENDPOINTS.md",
  },
];

/** The definition of `term`, or `undefined` when it is not offered. */
export function define(term: string): GlossaryEntry | undefined {
  const wanted = term.trim().toLowerCase();
  return GLOSSARY.find((entry) => entry.term === wanted);
}

/** Every term, for the affordance that marks them up. */
export function glossaryTerms(): readonly string[] {
  return GLOSSARY.map((entry) => entry.term);
}
