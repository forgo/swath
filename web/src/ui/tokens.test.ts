// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The token NAME set is the M10 contract (ui-system.md §8): M12 may change
// any value or add a theme, but a renamed or dropped token fails here.
// Adding a name is a deliberate contract change — extend the list in the
// same PR and say why.
import { expect, test } from "vitest";
import { BREAKPOINTS } from "./breakpoints.js";
import { readToken } from "./styles.js";
import themeCss from "./theme-high-contrast.css?inline";
import tokensCss from "./tokens.css?inline";

const NAMES = [
  ...[
    "bg",
    "bg-raised",
    "bg-hud",
    "bg-input",
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
  "--swath-blur-hud",
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
  // `warn` is deliberately outside this set: it is byte-identical to
  // `decision-overview` and allowed to be, because the two never co-occur —
  // the reason is written beside both tokens in tokens.css (#433).
  const named = ["decision-live", "decision-overview", "decision-cache", "info", "danger"];
  const values = named.map((n) => readToken(`--swath-color-${n}`).trim().toLowerCase());
  expect(new Set(values).size, `duplicated among ${named.join(", ")}`).toBe(named.length);
});

test("the input well is its own role, identical to the page today", () => {
  // #387: three components used --swath-color-bg as a recessed well because
  // on this palette the accident reads correctly. Naming the role is what
  // lets a theme move one without moving the other; the values stay equal
  // here so the dark theme does not shift.
  expect(readToken("--swath-color-bg-input")).toBe(readToken("--swath-color-bg"));
});

test("elevation is translucency: the blur is a token, and the shadow stays none", () => {
  // #388. --swath-shadow-hud is deliberately `none`; the depth cue is the
  // map showing through. A component that reaches for a shadow instead is
  // reintroducing the thing this token was set to none to prevent.
  expect(readToken("--swath-shadow-hud")).toBe("none");
  expect(readToken("--swath-blur-hud")).toContain("blur(");
});

/** The declaration block of one CSS rule, normalised for comparison. */
function declarations(css: string, selector: string): string {
  const start = css.indexOf(selector);
  if (start < 0) {
    return "";
  }
  const open = css.indexOf("{", start);
  let depth = 0;
  let i = open;
  for (; i < css.length; i += 1) {
    if (css[i] === "{") depth += 1;
    if (css[i] === "}") {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  return css
    .slice(open + 1, i)
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\s+/g, " ")
    .trim();
}

test("the high-contrast theme's two entry points declare the same palette", () => {
  // CSS cannot union a media query with a selector, so the declarations
  // appear twice: once under prefers-contrast, once under the explicit
  // attribute. Identical by assertion, not by discipline (#389).
  const byPreference = declarations(themeCss, "@media (prefers-contrast: more)");
  const byAttribute = declarations(themeCss, ':root[data-theme="high-contrast"]');
  expect(byPreference).not.toBe("");
  expect(declarations(byPreference, ":root")).toBe(byAttribute);
});

test("the high-contrast theme declares every contrast-bearing token, and only real ones", () => {
  const declared = new Set([...themeCss.matchAll(/--swath-[a-z0-9-]+(?=\s*:)/g)].map((m) => m[0]));
  // No typos and no stale names: a theme token that is not in the contract
  // is a value nothing reads.
  for (const name of declared) {
    expect(NAMES, `${name} is not a contract token`).toContain(name);
  }
  // Everything whose value carries contrast must be overridden. Spacing,
  // radii, type scale, z-order and motion are deliberately inherited — a
  // theme that restated them would drift from tokens.css silently.
  const mustOverride = NAMES.filter(
    (n) =>
      n.startsWith("--swath-color-") ||
      n === "--swath-border-hairline" ||
      n === "--swath-border-focus" ||
      n === "--swath-blur-hud" ||
      n === "--swath-shadow-hud",
  );
  for (const name of mustOverride) {
    expect(declared, `${name} must be themed`).toContain(name);
  }
});

test("under high contrast, foreground and accent clear WCAG AA on every ground", () => {
  const block = declarations(themeCss, ':root[data-theme="high-contrast"]');
  const value = (name: string): string => {
    const found = new RegExp(`${name}\\s*:\\s*(#[0-9a-f]{6})`).exec(block)?.[1] ?? "";
    expect(found, `${name} must be a hex value in the theme`).not.toBe("");
    return found;
  };
  const grounds = ["--swath-color-bg", "--swath-color-bg-raised", "--swath-color-bg-input"];
  for (const ground of grounds) {
    expect(contrast(value("--swath-color-fg"), value(ground)), `fg on ${ground}`).toBeGreaterThan(
      4.5,
    );
    expect(
      contrast(value("--swath-color-fg-muted"), value(ground)),
      `fg-muted on ${ground}`,
    ).toBeGreaterThan(4.5);
    expect(
      contrast(value("--swath-color-accent"), value(ground)),
      `accent on ${ground}`,
    ).toBeGreaterThan(3);
  }
  // The well must be a well: distinguishable from both the page and a panel.
  expect(value("--swath-color-bg-input")).not.toBe(value("--swath-color-bg"));
  expect(value("--swath-color-bg-input")).not.toBe(value("--swath-color-bg-raised"));
  // And the decision set stays a set, and stays clear of the link colour.
  const decisions = ["live", "overview", "cache"].map((d) => value(`--swath-color-decision-${d}`));
  expect(new Set(decisions).size).toBe(3);
  expect(decisions).not.toContain(value("--swath-color-info"));
});
