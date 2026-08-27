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
    "warn",
    "danger",
    "info",
    "udf",
    "compare",
    "decision-live",
    "decision-overview",
    "decision-cache",
    "focus",
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
  ...["rail", "rail-icon", "inspector", "topbar", "statusbar", "target", "icon"].map(
    (s) => `--swath-size-${s}`,
  ),
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
});

test("the breakpoints comment in tokens.css mirrors breakpoints.ts", () => {
  const mirrored = `${BREAKPOINTS.narrow} / ${BREAKPOINTS.medium} / ${BREAKPOINTS.wide}`;
  expect(tokensCss).toContain(mirrored);
});
