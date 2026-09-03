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
  expect(doc.adoptedStyleSheets).toHaveLength(1);
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
