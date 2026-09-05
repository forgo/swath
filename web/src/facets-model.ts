// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * What a collection's items actually carry (#409). The server discovers
 * the keys from the granules in scope — `GET /datasets/{id}/facets` — and
 * this module turns that answer into rows the panel can render without
 * deciding anything of its own.
 *
 * The honesty rule the whole feature exists for: a facet is here only
 * because a granule has the key, so nothing rendered from these rows is a
 * control over data that does not exist. Where the server says nothing —
 * a key whose values are objects or a mix of kinds — the summary is an
 * em dash, never a guess.
 */

/** The em dash an unknown is rendered as, everywhere in this module. */
export const UNKNOWN = "—";

export type FacetKind = "number" | "string" | "boolean" | "other";

export interface FacetValue {
  /** The value as the item carried it, formatted for display. */
  label: string;
  count: number;
}

export interface Facet {
  key: string;
  kind: FacetKind;
  /** Granules in scope carrying the key at all. */
  coverage: number;
  /** Distinct values, most common first — `string` and `boolean` only. */
  values: FacetValue[];
  /** The server had more values than it returned. */
  truncated: boolean;
  min?: number;
  max?: number;
}

export interface FacetSummary {
  /** Granules the scope matched: the denominator of every coverage. */
  total: number;
  facets: Facet[];
}

const KINDS: readonly FacetKind[] = ["number", "string", "boolean", "other"];

function kindOf(raw: unknown): FacetKind {
  return KINDS.find((kind) => kind === raw) ?? "other";
}

function numberOr(raw: unknown, fallback: number): number {
  return typeof raw === "number" && Number.isFinite(raw) ? raw : fallback;
}

/** A property value as text. Strings are themselves; numbers and booleans
 * their JSON form; anything else is unknown rather than `[object
 * Object]`. */
function labelOf(raw: unknown): string {
  if (typeof raw === "string") {
    return raw;
  }
  if (typeof raw === "number" && Number.isFinite(raw)) {
    return String(raw);
  }
  if (typeof raw === "boolean") {
    return raw ? "yes" : "no";
  }
  return UNKNOWN;
}

/** `GET /datasets/{id}/facets` → the rows to render. A malformed facet is
 * dropped rather than shown wrong; a missing body is no facets, which the
 * panel renders as nothing at all. */
export function parseFacets(body: unknown): FacetSummary {
  const doc = (body as { total?: unknown; facets?: unknown[] } | null) ?? {};
  const facets: Facet[] = [];
  for (const raw of doc.facets ?? []) {
    if (typeof raw !== "object" || raw === null) {
      continue;
    }
    const item = raw as Record<string, unknown>;
    if (typeof item["key"] !== "string" || item["key"] === "") {
      continue;
    }
    const kind = kindOf(item["kind"]);
    const values: FacetValue[] = [];
    for (const value of Array.isArray(item["values"]) ? item["values"] : []) {
      if (typeof value !== "object" || value === null) {
        continue;
      }
      const entry = value as Record<string, unknown>;
      values.push({ label: labelOf(entry["value"]), count: numberOr(entry["count"], 0) });
    }
    const facet: Facet = {
      key: item["key"],
      kind,
      coverage: numberOr(item["coverage"], 0),
      values,
      truncated: item["truncated"] === true,
    };
    if (kind === "number") {
      if (typeof item["min"] === "number") {
        facet.min = item["min"];
      }
      if (typeof item["max"] === "number") {
        facet.max = item["max"];
      }
    }
    facets.push(facet);
  }
  return { total: numberOr(doc.total, 0), facets };
}

/** The facet's values in one line: a range for numbers, the commonest
 * values for strings and booleans, and `—` where the server claimed
 * nothing. Never invents a remainder — a truncated list says "and more"
 * because that is all the server told us. */
export function facetSummary(facet: Facet): string {
  if (facet.kind === "number") {
    if (facet.min === undefined || facet.max === undefined) {
      return UNKNOWN;
    }
    return facet.min === facet.max ? `${facet.min}` : `${facet.min} – ${facet.max}`;
  }
  if (facet.values.length === 0) {
    return UNKNOWN;
  }
  const shown = facet.values.slice(0, 3).map((value) => value.label);
  const rest = facet.values.length - shown.length;
  const more = facet.truncated || rest > 0 ? ", and more" : "";
  return `${shown.join(", ")}${more}`;
}

/** How much of the scope carries the key, in plain words. Full coverage
 * says so; partial coverage names both numbers, because "absent" and
 * "zero" are different answers. */
export function coverageNote(facet: Facet, total: number): string {
  if (total <= 0) {
    return UNKNOWN;
  }
  if (facet.coverage >= total) {
    return "on every granule";
  }
  return `on ${facet.coverage} of ${total}`;
}
