// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The token NAME set is the M10 contract (ui-system.md §8): M12 may change
// any value or add a theme, but a renamed or dropped token fails here.
// Adding a name is a deliberate contract change — extend the list in the
// same PR and say why.
import { expect, test } from "vitest";
import { BREAKPOINTS } from "./breakpoints.js";
import { readToken } from "./styles.js";
import tokensCss from "./tokens.css?inline";

const NAMES = [
  ...[
    "bg",
    "bg-raised",
    "bg-hud",
    "fg",
    "fg-muted",
    "line",
    "accent",
    "accent-bg",
    "accent-border",
    "selection-bg",
    "warn",
    "danger",
    "info",
    "udf",
    "compare",
    "decision-live",
    "decision-overview",
    "decision-cache",
    "focus",
    // Added in #286 (a deliberate contract extension): the x-ray's heat ramp.
    "heat-1",
    "heat-2",
    "heat-3",
    "heat-4",
    "heat-5",
  ].map((role) => `--swath-color-${role}`),
  "--swath-font-ui",
  "--swath-font-mono",
  ...["xs", "sm", "md", "lg"].map((size) => `--swath-text-${size}`),
  "--swath-leading-tight",
  "--swath-leading-normal",
  "--swath-tracking-wide",
  ...[0, 1, 2, 3, 4, 5, 6, 7, 8].map((step) => `--swath-space-${step}`),
  ...["sm", "md", "pill"].map((r) => `--swath-radius-${r}`),
  "--swath-border-hairline",
  "--swath-border-focus",
  "--swath-shadow-hud",
  ...[
    "rail",
    "rail-icon",
    "inspector",
    "topbar",
    "statusbar",
    "target",
    "icon",
    "icon-sm",
    "icon-lg",
  ].map((s) => `--swath-size-${s}`),
  ...["overlay", "controls", "hud", "drawer", "palette"].map((z) => `--swath-z-${z}`),
  ...["fast", "normal", "ease"].map((m) => `--swath-motion-${m}`),
].sort();

function declaredNames(): string[] {
  const root = /:root\s*\{([^}]*)\}/.exec(tokensCss)?.[1] ?? "";
  return [...new Set([...root.matchAll(/--swath-[a-z0-9-]+(?=\s*:)/g)].map((m) => m[0]))].sort();
}

test("tokens.css declares exactly the contract's names on :root", () => {
  expect(declaredNames()).toEqual(NAMES);
});

test("every token resolves to a non-empty value in the document", () => {
  for (const name of NAMES) {
    expect(readToken(name), name).not.toBe("");
  }
});

test("fixed shell sizes are the geometry ui-system.md §4.1 names", () => {
  expect(readToken("--swath-size-rail")).toBe("248px");
  expect(readToken("--swath-size-rail-icon")).toBe("56px");
  expect(readToken("--swath-size-inspector")).toBe("320px");
  expect(readToken("--swath-size-topbar")).toBe("44px");
  expect(readToken("--swath-size-statusbar")).toBe("24px");
  expect(readToken("--swath-size-target")).toBe("44px");
  expect(readToken("--swath-size-icon")).toBe("16px");
  expect(readToken("--swath-size-icon-sm")).toBe("12px");
  expect(readToken("--swath-size-icon-lg")).toBe("24px");
});

test("the breakpoints comment in tokens.css mirrors breakpoints.ts", () => {
  const mirrored = `${BREAKPOINTS.narrow} / ${BREAKPOINTS.medium} / ${BREAKPOINTS.wide}`;
  expect(tokensCss).toContain(mirrored);
});

/** WCAG relative luminance of a `#rrggbb` token value. */
function luminance(hex: string): number {
  const channel = (pair: string): number => {
    const c = Number.parseInt(pair, 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const h = hex.trim().replace("#", "");
  return (
    0.2126 * channel(h.slice(0, 2)) +
    0.7152 * channel(h.slice(2, 4)) +
    0.0722 * channel(h.slice(4, 6))
  );
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return ((hi as number) + 0.05) / ((lo as number) + 0.05);
}

test("the three decision colours read as one set against every surface", () => {
  // They are borders and tints on map overlays, so the bar is WCAG's 3:1 for
  // a non-text UI component — which --swath-color-decision-cache did not
  // clear on bg-raised before #383 (2.83).
  const surfaces = ["--swath-color-bg", "--swath-color-bg-raised"];
  for (const decision of ["live", "overview", "cache"]) {
    const colour = readToken(`--swath-color-decision-${decision}`);
    for (const surface of surfaces) {
      expect(contrast(colour, readToken(surface)), `${decision} on ${surface}`).toBeGreaterThan(3);
    }
  }
});

test("no decision colour is another decision colour, or the link colour", () => {
  // A decision badge the colour of a link cannot be read as a decision —
  // which is why #383 raised cache to blue-500 and not blue-400.
  //
  // `warn` is deliberately outside this set for now: it is byte-identical to
  // `decision-overview`, a real collision that wants a palette decision
  // rather than a drive-by change (#433). Widen this list when that lands.
  const named = ["decision-live", "decision-overview", "decision-cache", "info", "danger"];
  const values = named.map((n) => readToken(`--swath-color-${n}`).trim().toLowerCase());
  expect(new Set(values).size, `duplicated among ${named.join(", ")}`).toBe(named.length);
});
