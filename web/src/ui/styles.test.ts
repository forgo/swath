// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import { expect, test } from "vitest";
import { adoptTokens, css, readToken, tokens } from "./styles.js";

test("tokens are adopted once per document, however often asked", () => {
  adoptTokens(document);
  adoptTokens(document);
  adoptTokens();
  expect(document.adoptedStyleSheets.filter((sheet) => sheet === tokens)).toHaveLength(1);
});

test("a windowless document gets the tokens as one <style>", () => {
  const doc = document.implementation.createHTMLDocument("probe");
  adoptTokens(doc);
  adoptTokens(doc);
  // No window → no realm to construct a sheet in; a <style> carries the tokens.
  expect(doc.adoptedStyleSheets).toHaveLength(0);
  expect(doc.head.querySelectorAll("style")).toHaveLength(1);
  expect(doc.head.querySelector("style")?.textContent).toContain("--swath-color-bg:");
  // The one <style> carries the theme too, so a windowless document is not
  // silently stuck on the default palette (#389).
  expect(doc.head.querySelector("style")?.textContent).toContain("prefers-contrast");
});

test("an iframe's document adopts a sheet constructed in its own realm", () => {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const doc = frame.contentDocument;
  if (!doc) {
    throw new Error("iframe has no document");
  }
  adoptTokens(doc);
  adoptTokens(doc);
  // Three sheets: the faces (#379), the tokens, then the high-contrast
  // overrides that narrow them (#389). All built in the frame's own realm.
  expect(doc.adoptedStyleSheets).toHaveLength(3);
  expect(doc.adoptedStyleSheets[0]).not.toBe(tokens);
  expect(getComputedStyle(doc.documentElement).getPropertyValue("--swath-size-icon").trim()).toBe(
    "16px",
  );
  frame.remove();
});

test("css`` caches by text so a sheet is one object across call sites", () => {
  const a = css`:host { color: var(--swath-color-fg); }`;
  const b = css`:host { color: var(--swath-color-fg); }`;
  const c = css`:host { color: var(--swath-color-fg-muted); }`;
  expect(a).toBe(b);
  expect(a).not.toBe(c);
  expect(a.cssRules).toHaveLength(1);
});

test("readToken bridges a token to code that can't read custom properties", () => {
  expect(readToken("--swath-size-icon")).toBe("16px");
  expect(readToken("--swath-color-accent")).toMatch(/^#[0-9a-f]{6}$/i);
  expect(readToken("--swath-nope")).toBe("");
});

test("every SwathElement gets tabular figures and the product's selection colour", async () => {
  // Numbers that change while you watch them must not re-lay-out (#382).
  // Asserted on real primitives rather than a probe: these are the two
  // shapes that carry the fastest-changing readouts in the product.
  const { SwathIcon } = await import("./icon.js");
  const { SwathSlider } = await import("./slider.js");
  SwathIcon.define();
  SwathSlider.define();
  const slider = document.createElement("swath-slider");
  document.body.append(slider);
  await (slider as unknown as { updateComplete: Promise<void> }).updateComplete;
  expect(getComputedStyle(slider).fontVariantNumeric).toBe("tabular-nums");

  // The selection colour is a token, not a literal — a theme must be able
  // to move it (the high-contrast theme, #389).
  expect(readToken("--swath-color-selection-bg")).not.toBe("");
});

test("the high-contrast overrides are adopted immediately after the tokens", async () => {
  const { highContrast } = await import("./styles.js");
  adoptTokens(document);
  const sheets = document.adoptedStyleSheets;
  const tokensAt = sheets.indexOf(tokens);
  const themeAt = sheets.indexOf(highContrast);
  expect(tokensAt).toBeGreaterThanOrEqual(0);
  // Order is the whole mechanism: the theme narrows values tokens.css set,
  // so a sheet adopted before it would win (#389).
  expect(themeAt).toBe(tokensAt + 1);
});

test("the self-hosted faces are adopted ahead of the tokens that name them", async () => {
  const { faces, highContrast } = await import("./styles.js");
  adoptTokens(document);
  const sheets = document.adoptedStyleSheets;
  const facesAt = sheets.indexOf(faces);
  expect(facesAt).toBeGreaterThanOrEqual(0);
  expect(sheets.indexOf(tokens)).toBe(facesAt + 1);
  expect(sheets.indexOf(highContrast)).toBe(facesAt + 2);
});

test("both faces are declared, cover the arrow, and keep a real fallback", async () => {
  const { facesCss } = await import("./fonts.js");
  // Three @font-face rules: one variable UI face, two mono weights (#379).
  expect(facesCss.match(/@font-face/g)).toHaveLength(3);
  // Every src is a bundled asset URL, never a remote host — ADR 0021 §4
  // objects to the CDN, and this is the line that keeps it true.
  for (const src of facesCss.matchAll(/url\(([^)]*)\)/g)) {
    expect(src[1], "no font may be fetched from another origin").not.toMatch(/^https?:/);
  }
  // U+2192 is NOT in Google's `latin` range; the subset adds it because the
  // status bar writes `ingest→pixel` in mono. Without it that one glyph
  // falls back to a system face.
  expect(facesCss).toContain("U+2191-2193");
  // The platform stacks stay behind the new faces, so an unsubsetted glyph
  // still renders.
  expect(readToken("--swath-font-ui")).toContain("system-ui");
  expect(readToken("--swath-font-mono")).toContain("monospace");
  expect(readToken("--swath-font-ui").startsWith('"Space Grotesk"')).toBe(true);
  expect(readToken("--swath-font-mono").startsWith('"Space Mono"')).toBe(true);
});
