// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * The guided import's pure model (#420).
 *
 * # One detecting input
 *
 * A person importing data knows what they *have* — a link someone sent
 * them, a file a colleague exported — not which of our methods it maps
 * to. So there is one input: paste a URL or drop a file, and the flow
 * works out the method. Asking someone to choose a method before they
 * know the vocabulary is asking them to guess.
 *
 * When detection fails it **says what it tried** and offers the explicit
 * choice, which is the only honest fallback: a flow that silently picked
 * the wrong method would be worse than one that asked.
 *
 * # Every step is nameable
 *
 * The steps are a named, ordered list and the current one rides in the
 * URL (M14's chips), so a half-finished import is a link — resumable,
 * shareable, and survivable across a reload.
 */

/** What kind of thing the input turned out to be. */
export type ImportMethod =
  | "stac-catalog"
  | "stac-collection"
  | "stac-item"
  | "stac-item-collection";

/** The named steps, in order. The current one is the URL's `step`. */
export const IMPORT_STEPS = ["source", "review", "confirm"] as const;
export type ImportStep = (typeof IMPORT_STEPS)[number];

/** What each step is called on screen — the chip's value, too. */
export const STEP_LABELS: Record<ImportStep, string> = {
  source: "Where from",
  review: "What we found",
  confirm: "Add it",
};

/** The `step` in a URL, or the first step when there is none or it is
 * not one of ours. A pasted link naming a step nobody has is a link to
 * the beginning, not an error page. */
export function stepFrom(raw: string | null | undefined): ImportStep {
  return IMPORT_STEPS.find((step) => step === raw) ?? "source";
}

/** The step after `step`, or `undefined` at the end. */
export function nextStep(step: ImportStep): ImportStep | undefined {
  return IMPORT_STEPS[IMPORT_STEPS.indexOf(step) + 1];
}

export interface Detected {
  ok: true;
  method: ImportMethod;
  /** What the flow will call it, from the document or the URL. */
  title: string;
  /** The URL it came from, when it came from one. */
  url?: string;
  /** The parsed document, when the input was one. */
  document?: unknown;
}

export interface Undetected {
  ok: false;
  /** Everything the flow tried, in the order it tried it — so the
   * message can say what was ruled out rather than "invalid input". */
  tried: string[];
  /** The one-line reason, in the person's terms. */
  reason: string;
}

export type Detection = Detected | Undetected;

/** What a STAC `type` maps to. Unknown types are not guessed. */
const STAC_TYPES: Record<string, ImportMethod> = {
  Catalog: "stac-catalog",
  Collection: "stac-collection",
  Feature: "stac-item",
  FeatureCollection: "stac-item-collection",
};

/** The method's name in the person's words, for the explicit choice. */
export const METHOD_LABELS: Record<ImportMethod, string> = {
  "stac-catalog": "a STAC catalog",
  "stac-collection": "a STAC collection",
  "stac-item": "a single STAC item",
  "stac-item-collection": "a set of STAC items",
};

function titleOf(document: Record<string, unknown>, fallback: string): string {
  for (const key of ["title", "id"]) {
    const value = document[key];
    if (typeof value === "string" && value !== "") {
      return value;
    }
  }
  return fallback;
}

/**
 * What the pasted or dropped `input` is.
 *
 * Order matters and is reported: JSON is tried first because a document
 * names its own type, then a URL, whose *path* is only ever a hint. A URL
 * is reported as a catalog — the flow fetches it and re-detects from what
 * comes back, rather than deciding from the path alone.
 */
export function detect(input: string): Detection {
  const text = input.trim();
  const tried: string[] = [];
  if (text === "") {
    return { ok: false, tried, reason: "Paste a link or drop a file to start." };
  }

  tried.push("a STAC document");
  if (text.startsWith("{")) {
    let document: unknown;
    try {
      document = JSON.parse(text);
    } catch {
      return {
        ok: false,
        tried,
        reason: "That looks like JSON but does not parse.",
      };
    }
    const doc = (document ?? {}) as Record<string, unknown>;
    const method = typeof doc["type"] === "string" ? STAC_TYPES[doc["type"]] : undefined;
    if (method !== undefined) {
      return { ok: true, method, title: titleOf(doc, method), document };
    }
    return {
      ok: false,
      tried,
      reason:
        typeof doc["type"] === "string"
          ? `That JSON says it is a "${doc["type"]}", which is not something we import.`
          : "That JSON has no `type`, so we cannot tell what it is.",
    };
  }

  tried.push("a link to a STAC endpoint");
  if (/^https?:\/\/\S+$/i.test(text)) {
    // The path is a hint and nothing more: what the endpoint actually is
    // comes from fetching it, which is the next step.
    return { ok: true, method: "stac-catalog", title: hostOf(text) ?? text, url: text };
  }

  return {
    ok: false,
    tried,
    reason: "That is neither a STAC document nor an http(s) link.",
  };
}

/** The host of a URL, for a human-readable title. */
export function hostOf(url: string): string | undefined {
  const authority = url.split("://")[1]?.split(/[/?#]/)[0];
  const host = authority?.split("@").pop()?.split(":")[0];
  return host !== undefined && host !== "" ? host : undefined;
}

/** What the flow says when detection failed: the reason, then what it
 * ruled out, then the offer of the explicit choice. Never "invalid
 * input" — that tells a person nothing they can act on. */
export function undetectedNote(detection: Undetected): string {
  if (detection.tried.length === 0) {
    return detection.reason;
  }
  return `${detection.reason} We tried ${detection.tried.join(" and ")}. Choose the method yourself below.`;
}
