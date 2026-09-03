// SPDX-FileCopyrightText: 2026 Elliott Richerson <elliott.richerson@gmail.com>
// SPDX-License-Identifier: Apache-2.0

/**
 * Stylesheet plumbing for the UI system (docs/design/ui-system.md §4.1).
 *
 * `tokens.css` is adopted ONCE per document (custom properties inherit
 * through shadow boundaries); `base.css` and each element's own `css`
 * sheets are adopted into shadow roots by `SwathElement`. Constructed
 * `CSSStyleSheet`s are cached by text, so a sheet declared once at module
 * scope is shared by every instance — no per-instance `<style>` nodes.
 */
import baseCss from "./base.css?inline";
import themeCss from "./theme-high-contrast.css?inline";
import tokensCss from "./tokens.css?inline";

const sheets = new Map<string, CSSStyleSheet>();

function sheet(text: string): CSSStyleSheet {
  let cached = sheets.get(text);
  if (!cached) {
    cached = new CSSStyleSheet();
    cached.replaceSync(text);
    sheets.set(text, cached);
  }
  return cached;
}

/** Tagged template → a cached, constructed `CSSStyleSheet`. Interpolations
 * are for token references and numbers, never raw values (the DRY gate
 * polices literals regardless of where they hide). */
export function css(
  strings: TemplateStringsArray,
  ...values: readonly (string | number)[]
): CSSStyleSheet {
  return sheet(String.raw({ raw: strings }, ...values));
}

/** The token sheet — one `:root` block; adopted at document level. */
export const tokens: CSSStyleSheet = sheet(tokensCss);

/** The high-contrast overrides; adopted immediately after `tokens`, whose
 * values it narrows. Always adopted — it is inert until the viewer asks for
 * more contrast, or the app sets `data-theme="high-contrast"`. */
export const highContrast: CSSStyleSheet = sheet(themeCss);

/** The shared shadow-root reset; adopted into every `SwathElement`. */
export const base: CSSStyleSheet = sheet(baseCss);

const adopted = new WeakSet<Document>();

/** Adopt the token sheet into `doc` exactly once (idempotent per document).
 * A constructed sheet belongs to one realm, so a foreign document (an
 * iframe, a popup) gets its own copy built there; a document with no window
 * (`createHTMLDocument`) gets a `<style>` instead. */
export function adoptTokens(doc: Document = document): void {
  if (adopted.has(doc)) {
    return;
  }
  adopted.add(doc);
  if (doc === document) {
    doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, tokens, highContrast];
    return;
  }
  const Realm = doc.defaultView?.CSSStyleSheet;
  if (Realm) {
    const copy = new Realm();
    copy.replaceSync(tokensCss);
    const theme = new Realm();
    theme.replaceSync(themeCss);
    doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, copy, theme];
    return;
  }
  const style = doc.createElement("style");
  style.textContent = `${tokensCss}\n${themeCss}`;
  doc.head.append(style);
}

const adoptedSheets = new WeakMap<Document, Set<CSSStyleSheet>>();

/** Adopt `sheet` at document level exactly once per document — for
 * light-DOM chrome that must live in the page's cascade (the x-ray's
 * badges are positioned in map pixels inside `<swath-map>`). Constructed
 * in the main realm; foreign documents are not supported here. */
export function adoptSheet(sheet: CSSStyleSheet, doc: Document = document): void {
  let set = adoptedSheets.get(doc);
  if (!set) {
    set = new Set();
    adoptedSheets.set(doc, set);
  }
  if (set.has(sheet)) {
    return;
  }
  set.add(sheet);
  adoptTokens(doc);
  doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, sheet];
}

/** Read a token's computed value where custom properties can't be consumed
 * directly — MapLibre paint properties, canvas drawing. `name` is the full
 * property name (`--swath-color-accent`). Empty string when undeclared. */
export function readToken(name: string, doc: Document = document): string {
  adoptTokens(doc);
  return getComputedStyle(doc.documentElement).getPropertyValue(name).trim();
}
